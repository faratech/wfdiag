//! Crash-safe file writes: temp file + fsync + atomic replace.
//!
//! Plain truncate-and-write destroys the previous file if the process dies
//! mid-write. Every durable store (settings.json, DPAPI key blobs,
//! EncryptedStorage payloads) goes through [`write_file`] instead.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Write `contents` to `path` atomically: data lands in a sibling `.tmp`
/// file that is flushed with `sync_all` before an atomic replace, so a
/// crash mid-write can never truncate or corrupt the previous contents.
/// Creates the parent directory when missing — callers pass first-run
/// paths (config dirs, credential stores) that may not exist yet.
pub fn write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }
    let temp_path = temp_sibling(path);
    {
        let mut file = fs::File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file {}: {e}", temp_path.display()))?;
        file.write_all(contents)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        file.sync_all()
            .map_err(|e| format!("Failed to flush {}: {e}", path.display()))?;
    }
    replace(&temp_path, path)
}

/// Sibling temp path (`<name>.tmp`) so the final replace stays on one volume.
fn temp_sibling(path: &Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(windows)]
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
        assert!(!temp_sibling(&path).exists());

        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }
}
