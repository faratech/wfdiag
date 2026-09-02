//! Disk-space breakdown: how much of the system drive the reclaimable and
//! well-known consumers hold, measured inside a strict budget.
//!
//! Portable by design: the walker uses only `std::fs`, never follows
//! symlinks or junctions, and reports every truncation, so the Windows
//! collector only resolves the folders and serialises the result. No absolute
//! user-profile path leaves this module — exports get pasted on forums.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The consumers the breakdown knows how to name and act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerId {
    Downloads,
    Desktop,
    Documents,
    Videos,
    Pictures,
    UserTemp,
    WindowsTemp,
    SoftwareDistribution,
    RecycleBin,
    HibernationFile,
    PageFile,
    WindowsOld,
    OneDriveCache,
}

impl ConsumerId {
    /// Plain label for the issue description and the raw output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Downloads => "Downloads",
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
            Self::Videos => "Videos",
            Self::Pictures => "Pictures",
            Self::UserTemp => "Temporary files",
            Self::WindowsTemp => "Windows temporary files",
            Self::SoftwareDistribution => "Windows Update download cache",
            Self::RecycleBin => "Recycle Bin",
            Self::HibernationFile => "Hibernation file",
            Self::PageFile => "Page file",
            Self::WindowsOld => "Previous Windows installation (Windows.old)",
            Self::OneDriveCache => "OneDrive local cache",
        }
    }

    /// The vetted catalog remediation that reclaims or reviews this consumer.
    /// Hibernation and page files are informational: the app never suggests
    /// turning them off.
    #[must_use]
    pub const fn remediation(self) -> Option<&'static str> {
        match self {
            Self::Downloads => Some("open_downloads_folder"),
            Self::Desktop
            | Self::Documents
            | Self::Videos
            | Self::Pictures
            | Self::WindowsOld
            | Self::OneDriveCache => Some("open_storage_settings"),
            Self::UserTemp => Some("clear_temp_files"),
            Self::WindowsTemp => Some("clear_windows_temp"),
            Self::SoftwareDistribution => Some("windows_update_reset"),
            Self::RecycleBin => Some("empty_recycle_bin"),
            Self::HibernationFile | Self::PageFile => None,
        }
    }

    /// Whether the remediation actually frees space (as opposed to opening a
    /// folder or settings page for the user to decide).
    #[must_use]
    pub const fn reclaimable(self) -> bool {
        matches!(
            self,
            Self::UserTemp | Self::WindowsTemp | Self::SoftwareDistribution | Self::RecycleBin
        )
    }
}

/// How a consumer is measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    /// Walk the directory tree (bounded by the budget).
    Directory,
    /// One file; its size is its metadata length.
    SingleFile,
    /// Already measured by the platform (the Recycle Bin shell query).
    Measured { bytes: u64, entries: u64 },
}

/// One consumer to measure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerTarget {
    pub id: ConsumerId,
    pub root: PathBuf,
    pub kind: TargetKind,
}

/// The walk's limits. The whole measurement shares one budget so a huge
/// Downloads folder cannot starve the consumers after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBudget {
    pub max_entries: usize,
    pub max_duration: Duration,
    pub max_depth: usize,
}

impl Default for WalkBudget {
    fn default() -> Self {
        Self {
            max_entries: 250_000,
            max_duration: Duration::from_secs(4),
            max_depth: 12,
        }
    }
}

/// One measured consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // independent facts about one walk, all serialised
pub struct ConsumerUsage {
    pub id: ConsumerId,
    pub label: &'static str,
    pub bytes: u64,
    pub entries: u64,
    /// The budget ran out before this consumer was fully walked; `bytes` is a
    /// lower bound.
    pub truncated: bool,
    /// The root itself could not be read (typically a standard user on a
    /// system folder).
    pub access_denied: bool,
    /// The root does not exist on this machine (no hibernation file, no
    /// Windows.old); reported so the reader knows it was checked.
    pub missing: bool,
    pub remediation: Option<&'static str>,
    pub reclaimable: bool,
}

/// The full breakdown, largest consumer first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiskUsageReport {
    pub drive: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub consumers: Vec<ConsumerUsage>,
    pub budget_exhausted: bool,
    pub elapsed_ms: u64,
}

/// A predicate for files that only look local (cloud placeholders); the
/// Windows collector checks the offline/recall attributes, tests pass
/// `|_| false`.
pub type PlaceholderCheck<'a> = &'a dyn Fn(&fs::Metadata) -> bool;

struct Budget<'a> {
    limits: WalkBudget,
    started: Instant,
    entries: usize,
    now: &'a dyn Fn() -> Instant,
    exhausted: bool,
}

impl Budget<'_> {
    fn spend(&mut self) -> bool {
        if self.exhausted {
            return false;
        }
        self.entries += 1;
        if self.entries > self.limits.max_entries
            || (self.now)().saturating_duration_since(self.started) > self.limits.max_duration
        {
            self.exhausted = true;
            return false;
        }
        true
    }
}

struct WalkOutcome {
    bytes: u64,
    entries: u64,
    truncated: bool,
    access_denied: bool,
    missing: bool,
}

/// Sum a directory tree without following symlinks or junctions.
fn walk_directory(
    root: &Path,
    budget: &mut Budget<'_>,
    placeholder: PlaceholderCheck<'_>,
) -> WalkOutcome {
    let mut outcome = WalkOutcome {
        bytes: 0,
        entries: 0,
        truncated: false,
        access_denied: false,
        missing: false,
    };
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            // A symlink or file where a folder was expected: not ours to count.
            outcome.missing = true;
            return outcome;
        }
        Err(error) => {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                outcome.access_denied = true;
            } else {
                outcome.missing = true;
            }
            return outcome;
        }
    }
    let mut pending: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((directory, depth)) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                if depth == 0 && error.kind() == std::io::ErrorKind::PermissionDenied {
                    outcome.access_denied = true;
                }
                continue;
            }
        };
        for entry in entries.flatten() {
            if !budget.spend() {
                outcome.truncated = true;
                return outcome;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if depth + 1 < budget.limits.max_depth {
                    pending.push((entry.path(), depth + 1));
                } else {
                    outcome.truncated = true;
                }
                continue;
            }
            if placeholder(&metadata) {
                continue;
            }
            outcome.entries += 1;
            outcome.bytes = outcome.bytes.saturating_add(metadata.len());
        }
    }
    outcome
}

fn measure_single_file(path: &Path) -> WalkOutcome {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => WalkOutcome {
            bytes: metadata.len(),
            entries: 1,
            truncated: false,
            access_denied: false,
            missing: false,
        },
        Ok(_) => WalkOutcome {
            bytes: 0,
            entries: 0,
            truncated: false,
            access_denied: false,
            missing: true,
        },
        Err(error) => WalkOutcome {
            bytes: 0,
            entries: 0,
            truncated: false,
            access_denied: error.kind() == std::io::ErrorKind::PermissionDenied,
            missing: error.kind() != std::io::ErrorKind::PermissionDenied,
        },
    }
}

/// Measure every target under one shared budget. `now` is injected so the
/// time limit is testable; `drive`, `total_bytes` and `free_bytes` come from
/// the platform.
#[must_use]
pub fn measure(
    drive: &str,
    total_bytes: u64,
    free_bytes: u64,
    targets: &[ConsumerTarget],
    limits: WalkBudget,
    now: &dyn Fn() -> Instant,
    placeholder: PlaceholderCheck<'_>,
) -> DiskUsageReport {
    let started = now();
    let mut budget = Budget {
        limits,
        started,
        entries: 0,
        now,
        exhausted: false,
    };
    let mut consumers: Vec<ConsumerUsage> = targets
        .iter()
        .map(|target| {
            let outcome = match &target.kind {
                TargetKind::Directory => walk_directory(&target.root, &mut budget, placeholder),
                TargetKind::SingleFile => measure_single_file(&target.root),
                TargetKind::Measured { bytes, entries } => WalkOutcome {
                    bytes: *bytes,
                    entries: *entries,
                    truncated: false,
                    access_denied: false,
                    missing: false,
                },
            };
            ConsumerUsage {
                id: target.id,
                label: target.id.label(),
                bytes: outcome.bytes,
                entries: outcome.entries,
                truncated: outcome.truncated,
                access_denied: outcome.access_denied,
                missing: outcome.missing,
                remediation: target.id.remediation(),
                reclaimable: target.id.reclaimable(),
            }
        })
        .collect();
    consumers.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.label.cmp(b.label)));
    let elapsed = now().saturating_duration_since(started);
    DiskUsageReport {
        drive: drive.to_string(),
        total_bytes,
        free_bytes,
        consumers,
        budget_exhausted: budget.exhausted,
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
    }
}

pub use wfdiag_native_issues::evidence::size::format_bytes;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wfdiag_disk_usage_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn no_placeholders(_: &fs::Metadata) -> bool {
        false
    }

    #[test]
    fn sums_trees_ranks_consumers_and_reports_missing_roots() {
        let dir = scratch("sum");
        write(&dir.join("downloads/a.iso"), 3_000);
        write(&dir.join("downloads/nested/b.bin"), 1_000);
        write(&dir.join("temp/t.tmp"), 500);
        write(&dir.join("hiberfil.sys"), 700);
        let targets = vec![
            ConsumerTarget {
                id: ConsumerId::UserTemp,
                root: dir.join("temp"),
                kind: TargetKind::Directory,
            },
            ConsumerTarget {
                id: ConsumerId::Downloads,
                root: dir.join("downloads"),
                kind: TargetKind::Directory,
            },
            ConsumerTarget {
                id: ConsumerId::HibernationFile,
                root: dir.join("hiberfil.sys"),
                kind: TargetKind::SingleFile,
            },
            ConsumerTarget {
                id: ConsumerId::WindowsOld,
                root: dir.join("Windows.old"),
                kind: TargetKind::Directory,
            },
            ConsumerTarget {
                id: ConsumerId::RecycleBin,
                root: PathBuf::new(),
                kind: TargetKind::Measured {
                    bytes: 2_000,
                    entries: 7,
                },
            },
        ];
        let report = measure(
            "C:",
            100_000,
            40_000,
            &targets,
            WalkBudget::default(),
            &Instant::now,
            &no_placeholders,
        );
        let ranked: Vec<(ConsumerId, u64)> = report
            .consumers
            .iter()
            .map(|consumer| (consumer.id, consumer.bytes))
            .collect();
        assert_eq!(
            ranked,
            [
                (ConsumerId::Downloads, 4_000),
                (ConsumerId::RecycleBin, 2_000),
                (ConsumerId::HibernationFile, 700),
                (ConsumerId::UserTemp, 500),
                (ConsumerId::WindowsOld, 0),
            ]
        );
        let downloads = &report.consumers[0];
        assert_eq!(downloads.entries, 2);
        assert!(!downloads.truncated);
        assert_eq!(downloads.remediation, Some("open_downloads_folder"));
        assert!(!downloads.reclaimable);
        assert!(report.consumers[4].missing);
        assert!(report.consumers[1].reclaimable);
        assert!(report.consumers[2].remediation.is_none());
        assert!(!report.budget_exhausted);
        assert_eq!(report.drive, "C:");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_symlinks() {
        let dir = scratch("symlink");
        write(&dir.join("real/big.bin"), 5_000);
        fs::create_dir_all(dir.join("watched")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("watched/loop")).unwrap();
        std::os::unix::fs::symlink(dir.join("real/big.bin"), dir.join("watched/file")).unwrap();
        let targets = vec![ConsumerTarget {
            id: ConsumerId::Documents,
            root: dir.join("watched"),
            kind: TargetKind::Directory,
        }];
        let report = measure(
            "C:",
            1,
            1,
            &targets,
            WalkBudget::default(),
            &Instant::now,
            &no_placeholders,
        );
        assert_eq!(report.consumers[0].bytes, 0);
        assert_eq!(report.consumers[0].entries, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entry_budget_truncates_and_marks_the_consumer() {
        let dir = scratch("budget");
        for index in 0..20 {
            write(&dir.join(format!("many/{index}.bin")), 10);
        }
        write(&dir.join("after/z.bin"), 10);
        let targets = vec![
            ConsumerTarget {
                id: ConsumerId::Downloads,
                root: dir.join("many"),
                kind: TargetKind::Directory,
            },
            ConsumerTarget {
                id: ConsumerId::Desktop,
                root: dir.join("after"),
                kind: TargetKind::Directory,
            },
        ];
        let report = measure(
            "C:",
            1,
            1,
            &targets,
            WalkBudget {
                max_entries: 5,
                max_duration: Duration::from_secs(60),
                max_depth: 12,
            },
            &Instant::now,
            &no_placeholders,
        );
        assert!(report.budget_exhausted);
        let downloads = report
            .consumers
            .iter()
            .find(|consumer| consumer.id == ConsumerId::Downloads)
            .unwrap();
        assert!(downloads.truncated);
        assert!(downloads.bytes < 200);
        let desktop = report
            .consumers
            .iter()
            .find(|consumer| consumer.id == ConsumerId::Desktop)
            .unwrap();
        assert!(
            desktop.truncated,
            "a spent budget truncates later consumers too"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn time_budget_is_read_from_the_injected_clock() {
        let dir = scratch("clock");
        for index in 0..5 {
            write(&dir.join(format!("d/{index}.bin")), 10);
        }
        let targets = vec![ConsumerTarget {
            id: ConsumerId::Downloads,
            root: dir.join("d"),
            kind: TargetKind::Directory,
        }];
        let ticks = Cell::new(0_u32);
        let base = Instant::now();
        let clock = move || {
            let tick = ticks.get();
            ticks.set(tick + 1);
            base + Duration::from_secs(u64::from(tick) * 3)
        };
        let report = measure(
            "C:",
            1,
            1,
            &targets,
            WalkBudget {
                max_entries: 1_000,
                max_duration: Duration::from_secs(4),
                max_depth: 12,
            },
            &clock,
            &no_placeholders,
        );
        assert!(report.budget_exhausted);
        assert!(report.consumers[0].truncated);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn placeholders_are_not_counted_as_local_bytes() {
        let dir = scratch("placeholder");
        write(&dir.join("cloud/local.bin"), 100);
        write(&dir.join("cloud/remote.bin"), 900);
        let targets = vec![ConsumerTarget {
            id: ConsumerId::OneDriveCache,
            root: dir.join("cloud"),
            kind: TargetKind::Directory,
        }];
        let is_remote = |metadata: &fs::Metadata| metadata.len() == 900;
        let report = measure(
            "C:",
            1,
            1,
            &targets,
            WalkBudget::default(),
            &Instant::now,
            &is_remote,
        );
        assert_eq!(report.consumers[0].bytes, 100);
        assert_eq!(report.consumers[0].entries, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_consumer_remediation_exists_in_the_catalog() {
        let all = [
            ConsumerId::Downloads,
            ConsumerId::Desktop,
            ConsumerId::Documents,
            ConsumerId::Videos,
            ConsumerId::Pictures,
            ConsumerId::UserTemp,
            ConsumerId::WindowsTemp,
            ConsumerId::SoftwareDistribution,
            ConsumerId::RecycleBin,
            ConsumerId::HibernationFile,
            ConsumerId::PageFile,
            ConsumerId::WindowsOld,
            ConsumerId::OneDriveCache,
        ];
        for id in all {
            if let Some(remediation) = id.remediation() {
                assert!(
                    wfdiag_native_issues::remediation_catalog()
                        .iter()
                        .any(|metadata| metadata.id == remediation),
                    "{id:?} -> {remediation}"
                );
            }
        }
        assert_eq!(format_bytes(6_657_199_309), "6.2 GB");
    }
}
