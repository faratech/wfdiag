//! The audit trail: every state-changing action the app ran, and every
//! automation decision, as one JSON line each. Read-only diagnostics never
//! appear here; this is the record a user (or a support thread) reads to
//! learn exactly what `WFDiag` changed and whether it worked.

use crate::event::SafeFixOrigin;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use wfdiag_native_issues::Timestamp;

/// What an entry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditKind {
    /// A remediation run reached a terminal state (`detail.run` is the
    /// run summary, per action).
    RunFinished,
    /// The automation layer decided what to run and what to defer.
    SafeFixesPlanned,
    /// The automation layer finished a session.
    SafeFixesFinished,
    /// A finished run was checked against fresh evidence.
    Verified,
}

impl AuditKind {
    /// The wire id.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::RunFinished => "run_finished",
            Self::SafeFixesPlanned => "safe_fixes_planned",
            Self::SafeFixesFinished => "safe_fixes_finished",
            Self::Verified => "verified",
        }
    }
}

/// One line of the trail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    /// When it happened.
    pub at: Timestamp,
    /// What happened.
    pub kind: AuditKind,
    /// The automation origin, or `None` for an action the user approved
    /// through the review dialog.
    pub origin: Option<SafeFixOrigin>,
    /// Kind-specific facts.
    pub detail: Value,
}

impl AuditEntry {
    /// The JSON object written for this entry.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "at": self.at.to_iso_string(),
            "kind": self.kind.id(),
            "origin": self.origin.map_or("user_review", SafeFixOrigin::id),
            "detail": self.detail,
        })
    }
}

/// Where entries go.
pub trait AuditSink: Send + Sync {
    /// Record one entry. Failures are swallowed: the trail must never stop
    /// the action it describes.
    fn record(&self, entry: &AuditEntry);
}

/// Drops every entry.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn record(&self, _entry: &AuditEntry) {}
}

/// Keeps entries in memory, for tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryAuditSink {
    entries: Arc<Mutex<Vec<AuditEntry>>>,
}

impl MemoryAuditSink {
    /// Every entry recorded so far, oldest first.
    #[must_use]
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }
}

impl AuditSink for MemoryAuditSink {
    fn record(&self, entry: &AuditEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry.clone());
        }
    }
}

/// Rotate the trail once it passes this size; one older generation is kept.
pub const AUDIT_ROTATE_BYTES: u64 = 4 * 1024 * 1024;

/// Appends JSON lines to `actions.jsonl` in a directory the app owns.
#[derive(Debug, Clone)]
pub struct FileAuditSink {
    path: PathBuf,
}

impl FileAuditSink {
    /// The trail file inside `directory` (created on first write).
    #[must_use]
    pub fn in_directory(directory: &Path) -> Self {
        Self {
            path: directory.join("actions.jsonl"),
        }
    }

    /// The file being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append(&self, line: &str) -> std::io::Result<()> {
        if let Some(directory) = self.path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        // Never write through a planted link: the trail is a regular file
        // or it does not exist yet.
        match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) if !metadata.is_file() => {
                return Err(std::io::Error::other("audit path is not a regular file"));
            }
            Ok(metadata) if metadata.len() >= AUDIT_ROTATE_BYTES => {
                let _ = std::fs::rename(&self.path, self.path.with_extension("jsonl.1"));
            }
            _ => {}
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")
    }
}

impl AuditSink for FileAuditSink {
    fn record(&self, entry: &AuditEntry) {
        let _ = self.append(&entry.to_json().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_json_lines_and_the_file_never_follows_a_non_file() {
        let directory = std::env::temp_dir().join(format!(
            "wfdiag-audit-{}-{}",
            std::process::id(),
            Timestamp::now().to_iso_string().replace(':', "-")
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = FileAuditSink::in_directory(&directory);
        sink.record(&AuditEntry {
            at: Timestamp::from_secs(1_700_000_000),
            kind: AuditKind::RunFinished,
            origin: Some(SafeFixOrigin::AfterScan),
            detail: json!({"run": {"runId": "r1"}}),
        });
        sink.record(&AuditEntry {
            at: Timestamp::from_secs(1_700_000_001),
            kind: AuditKind::Verified,
            origin: None,
            detail: json!({"resolved": ["low_disk_space"]}),
        });
        let text = std::fs::read_to_string(sink.path()).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "run_finished");
        assert_eq!(lines[0]["origin"], "after_scan");
        assert_eq!(lines[0]["detail"]["run"]["runId"], "r1");
        assert_eq!(lines[1]["origin"], "user_review");
        assert!(lines[1]["at"].as_str().unwrap().starts_with("2023-11-14"));

        let blocked = FileAuditSink::in_directory(&directory);
        std::fs::remove_file(blocked.path()).unwrap();
        std::fs::create_dir(blocked.path()).unwrap();
        blocked.record(&AuditEntry {
            at: Timestamp::from_secs(1),
            kind: AuditKind::SafeFixesFinished,
            origin: Some(SafeFixOrigin::User),
            detail: json!({}),
        });
        assert!(
            blocked.path().is_dir(),
            "a directory at the path is left alone"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
