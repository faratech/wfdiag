//! Opaque, current-user protected `ChatGPT` document. The lock lives beside the
//! document, so atomic replacement never replaces the locked inode.
use crate::persistence::{DpapiProtector, SecretProtector, atomic_write_file};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

pub trait ProtectedDocumentGuard: Send + Sync {
    fn load(&self) -> Result<Option<Zeroizing<String>>, String>;
    fn save(&self, document: &str) -> Result<(), String>;
}

pub trait ProtectedDocumentStorage: Send + Sync {
    /// Non-blocking: callers retry under their own cancellation/deadline budget.
    fn try_lock(&self) -> Result<Option<Box<dyn ProtectedDocumentGuard>>, String>;
}

#[derive(Debug, Default)]
pub struct WindowsDpapiDocumentStorage;

struct DpapiDocumentGuard {
    _lock: File,
    path: PathBuf,
}

impl ProtectedDocumentStorage for WindowsDpapiDocumentStorage {
    fn try_lock(&self) -> Result<Option<Box<dyn ProtectedDocumentGuard>>, String> {
        let root = dirs::data_local_dir()
            .ok_or("Local app data is unavailable")?
            .join("WFDiag");
        std::fs::create_dir_all(&root)
            .map_err(|_| "Could not create the ChatGPT account directory")?;
        let path = root.join("chatgpt-accounts.dpapi");
        let lock_path = root.join("chatgpt-accounts.lock");
        for candidate in [&path, &lock_path] {
            if std::fs::symlink_metadata(candidate)
                .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
            {
                return Err("ChatGPT storage must be a regular file".into());
            }
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| "Could not open the ChatGPT account lock")?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(Box::new(DpapiDocumentGuard { _lock: lock, path }))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(_) => Err("Could not lock the ChatGPT account store".into()),
        }
    }
}

impl ProtectedDocumentGuard for DpapiDocumentGuard {
    fn load(&self) -> Result<Option<Zeroizing<String>>, String> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("ChatGPT account storage could not be read".into()),
        };
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "ChatGPT account storage could not be read")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("ChatGPT account storage exceeded its size limit".into());
        }
        DpapiProtector.unprotect(&bytes).map(Some).map_err(|_| {
            "ChatGPT account storage could not be decrypted; reconnect your account".into()
        })
    }

    fn save(&self, document: &str) -> Result<(), String> {
        let bytes = DpapiProtector
            .protect(document)
            .map_err(|_| "ChatGPT accounts could not be encrypted")?;
        atomic_write_file(&self.path, &bytes)
            .map_err(|_| "ChatGPT accounts could not be saved".into())
    }
}

/// Explicit in-memory storage for headless hosts; never a plaintext disk fallback.
#[derive(Clone, Default)]
pub struct MemoryProtectedDocumentStorage {
    contents: Arc<Mutex<Option<Zeroizing<String>>>>,
    locked: Arc<AtomicBool>,
}
struct MemoryGuard(MemoryProtectedDocumentStorage);
impl Drop for MemoryGuard {
    fn drop(&mut self) {
        self.0.locked.store(false, Ordering::Release);
    }
}
impl ProtectedDocumentStorage for MemoryProtectedDocumentStorage {
    fn try_lock(&self) -> Result<Option<Box<dyn ProtectedDocumentGuard>>, String> {
        Ok(self
            .locked
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then(|| Box::new(MemoryGuard(self.clone())) as Box<dyn ProtectedDocumentGuard>))
    }
}
impl ProtectedDocumentGuard for MemoryGuard {
    fn load(&self) -> Result<Option<Zeroizing<String>>, String> {
        self.0
            .contents
            .lock()
            .map(|v| v.clone())
            .map_err(|_| "ChatGPT memory store is unavailable".into())
    }
    fn save(&self, document: &str) -> Result<(), String> {
        *self
            .0
            .contents
            .lock()
            .map_err(|_| "ChatGPT memory store is unavailable")? =
            Some(Zeroizing::new(document.to_owned()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_serializes_owners_and_preserves_document() {
        let store = MemoryProtectedDocumentStorage::default();
        let first = store.try_lock().unwrap().unwrap();
        first.save("private document").unwrap();
        assert!(store.try_lock().unwrap().is_none());
        drop(first);
        assert_eq!(
            store
                .try_lock()
                .unwrap()
                .unwrap()
                .load()
                .unwrap()
                .unwrap()
                .as_str(),
            "private document"
        );
    }
}
