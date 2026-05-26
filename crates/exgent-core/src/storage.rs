//! Minimal storage abstraction.
//!
//! Most existing code paths still talk to [`std::fs`] directly. The trait
//! here gives downstream callers (tests, alternate hosts, MCP-style runners)
//! a seam to plug in without bringing in a full virtual filesystem.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Read/write surface mirroring the subset of `std::fs` used by the runtime.
pub trait Storage: Send + Sync {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn append(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn rename(&self, src: &Path, dst: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
}

/// Production [`Storage`] backed by the host filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct FsStorage;

impl Storage for FsStorage {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)
    }

    fn append(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(bytes)
    }

    fn rename(&self, src: &Path, dst: &Path) -> io::Result<()> {
        std::fs::rename(src, dst)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|value| value.path()))
            .collect()
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }
}

/// In-memory [`Storage`] suitable for tests.
#[derive(Default)]
pub struct InMemoryStorage {
    files: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for InMemoryStorage {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn append(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut files = self.files.lock().unwrap();
        files
            .entry(path.to_path_buf())
            .or_default()
            .extend_from_slice(bytes);
        Ok(())
    }

    fn rename(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let mut files = self.files.lock().unwrap();
        let bytes = files
            .remove(src)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, src.display().to_string()))?;
        files.insert(dst.to_path_buf(), bytes);
        Ok(())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.files
            .lock()
            .unwrap()
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains_key(path)
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let files = self.files.lock().unwrap();
        let prefix = path.to_path_buf();
        Ok(files
            .keys()
            .filter(|candidate| candidate.starts_with(&prefix) && candidate.as_path() != path)
            .cloned()
            .collect())
    }

    fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_round_trips_writes() {
        let storage = InMemoryStorage::new();
        let path = PathBuf::from("/tmp/exgent-in-memory/sample.txt");

        storage.write(&path, b"hello").unwrap();
        assert!(storage.exists(&path));
        assert_eq!(storage.read(&path).unwrap(), b"hello");

        storage.append(&path, b" world").unwrap();
        assert_eq!(storage.read(&path).unwrap(), b"hello world");

        let renamed = path.with_file_name("renamed.txt");
        storage.rename(&path, &renamed).unwrap();
        assert!(!storage.exists(&path));
        assert_eq!(storage.read(&renamed).unwrap(), b"hello world");

        storage.remove(&renamed).unwrap();
        assert!(!storage.exists(&renamed));
    }
}
