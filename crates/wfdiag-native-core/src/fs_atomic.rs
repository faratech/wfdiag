//! Crash-safe file writes: temp file + fsync + atomic replace.
//!
//! Plain truncate-and-write destroys the previous file if the process dies
//! mid-write. Every durable store (settings.json, DPAPI key blobs,
//! `EncryptedStorage` payloads) goes through [`write_file`] instead.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Write `contents` to `path` atomically: data lands in a private sibling
/// staging file that is flushed with `sync_all` before an atomic replace, so a
/// crash mid-write can never truncate or corrupt the previous contents.
/// Creates the parent directory when missing — callers pass first-run
/// paths (config dirs, credential stores) that may not exist yet.
///
/// # Errors
/// Returns a human-readable message when the parent directory cannot be
/// created, the staging file cannot be written or flushed, or the replace
/// fails.
pub fn write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }
    let temp_path = temp_sibling(path);
    match stage_and_replace(&temp_path, path, contents) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Staging names are unique per attempt (#208), so a failed write
            // would otherwise litter the directory with orphans forever.
            let _ = fs::remove_file(&temp_path);
            Err(error)
        }
    }
}

fn stage_and_replace(temp_path: &Path, path: &Path, contents: &[u8]) -> Result<(), String> {
    {
        let mut file = fs::File::create(temp_path)
            .map_err(|e| format!("Failed to create temp file {}: {e}", temp_path.display()))?;
        file.write_all(contents)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        file.sync_all()
            .map_err(|e| format!("Failed to flush {}: {e}", path.display()))?;
    }
    replace(temp_path, path)?;
    #[cfg(not(windows))]
    sync_parent(path)?;
    Ok(())
}

/// Sibling staging path (`<name>.<pid>.<counter>.tmp`) so the final replace
/// stays on one volume.
///
/// Issue #208: this used to be a fixed `<name>.tmp`, so two writers to the
/// same destination — two processes, or two threads on one store — shared a
/// single staging file. The second `File::create` truncated the first's
/// half-written buffer and both replaces then published whatever bytes
/// survived. Pid + a per-process counter makes every attempt's staging file
/// private.
fn temp_sibling(path: &Path) -> PathBuf {
    static ATTEMPT: AtomicU64 = AtomicU64::new(0);
    let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".{}.{attempt}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// fsync the parent directory so the *rename* is durable too (#208). On POSIX
/// filesystems `rename(2)` is atomic but the new directory entry may still sit
/// in the page cache; without this, a power loss right after a successful
/// `write_file` can resurrect the previous file.
#[cfg(not(windows))]
fn sync_parent(path: &Path) -> Result<(), String> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    let dir = fs::File::open(parent)
        .map_err(|e| format!("Failed to open {} for sync: {e}", parent.display()))?;
    dir.sync_all()
        .map_err(|e| format!("Failed to sync {}: {e}", parent.display()))
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn replace(temp_path: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    // Wide paths: MoveFileExW handles long/unicode destinations that
    // fs::rename cannot on Windows.
    let encode = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let temp_wide = encode(temp_path);
    let file_wide = encode(path);

    // MOVEFILE_WRITE_THROUGH already flushes the directory metadata, which is
    // what `sync_parent` does on the POSIX branch.
    unsafe {
        MoveFileExW(
            PCWSTR(temp_wide.as_ptr()),
            PCWSTR(file_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| format!("Failed to finalize {}: {e}", path.display()))
}

#[cfg(not(windows))]
fn replace(temp_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temp_path, path)
        .map_err(|error| format!("Failed to finalize {}: {}", path.display(), error))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What one concurrent writer observed across its attempts.
    struct Outcome {
        writes: usize,
        staging_failures: Vec<String>,
        torn: Vec<usize>,
    }

    #[test]
    fn write_then_read_roundtrips_and_leaves_no_temp() {
        // The parent is deliberately NOT pre-created: the helper must create
        // missing directories (first-run config/credential paths depend on
        // this — a missing parent here is exactly what broke CI once).
        let dir = std::env::temp_dir()
            .join("wfdiag_fs_atomic_test")
            .join("nested");
        fs::remove_dir_all(&dir).ok();
        let path = dir.join("roundtrip.json");

        write_file(&path, br#"{"v":1}"#).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"v\":1}");

        // Overwrite proves replace works against an existing destination.
        write_file(&path, br#"{"v":2}"#).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"v\":2}");
        assert!(staging_files(&dir).is_empty());

        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn staging_paths_are_unique_per_attempt() {
        // #208 in one assertion: a fixed `<name>.tmp` made these equal.
        let path = std::env::temp_dir()
            .join("wfdiag_fs_atomic_unique")
            .join("settings.json");
        let first = temp_sibling(&path);
        let second = temp_sibling(&path);
        assert_ne!(first, second);

        for staged in [&first, &second] {
            // Same directory, so the replace is a same-volume rename.
            assert_eq!(staged.parent(), path.parent());
            let name = staged.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with("settings.json."), "name: {name}");
            assert_eq!(
                staged.extension().and_then(std::ffi::OsStr::to_str),
                Some("tmp")
            );
            assert!(
                name.contains(&std::process::id().to_string()),
                "staging name must carry the pid: {name}"
            );
        }
    }

    #[test]
    fn concurrent_writers_never_share_a_staging_file() {
        // #208: with one fixed `<name>.tmp` the two writers below truncated
        // each other mid-write, so the file each one published could be a
        // prefix or a mixture. The payload lengths differ, so every published
        // byte string must equal one payload exactly.
        let dir = std::env::temp_dir().join("wfdiag_fs_atomic_concurrent");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.json");

        let alpha = vec![b'a'; 400_000];
        let beta = vec![b'b'; 150_000];

        let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
            let handles: Vec<_> = [&alpha, &beta]
                .into_iter()
                .map(|payload| {
                    let path = path.clone();
                    let (alpha, beta) = (&alpha, &beta);
                    scope.spawn(move || {
                        let mut outcome = Outcome {
                            writes: 0,
                            staging_failures: Vec::new(),
                            torn: Vec::new(),
                        };
                        for _ in 0..24 {
                            match write_file(&path, payload) {
                                Ok(()) => outcome.writes += 1,
                                // Only the destination rename may lose an
                                // OS-level race with the sibling writer;
                                // staging is private and must never collide.
                                Err(error) => {
                                    if !error.starts_with("Failed to finalize") {
                                        outcome.staging_failures.push(error);
                                    }
                                    continue;
                                }
                            }
                            // Whatever is published right now must be one
                            // complete payload, never a torn mixture.
                            if let Ok(seen) = fs::read(&path)
                                && seen != *alpha
                                && seen != *beta
                            {
                                outcome.torn.push(seen.len());
                            }
                        }
                        outcome
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("writer thread"))
                .collect()
        });

        for outcome in &outcomes {
            assert!(
                outcome.staging_failures.is_empty(),
                "staging must never collide: {:?}",
                outcome.staging_failures
            );
            assert!(
                outcome.writes > 0,
                "each writer must land at least one complete file"
            );
            assert!(
                outcome.torn.is_empty(),
                "torn file published (lengths {:?}); expected {} or {}",
                outcome.torn,
                alpha.len(),
                beta.len()
            );
        }

        let published = fs::read(&path).unwrap();
        assert!(
            published == alpha || published == beta,
            "torn file: {} bytes",
            published.len()
        );
        assert!(staging_files(&dir).is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    fn staging_files(dir: &Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }
}
