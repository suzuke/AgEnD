//! Unique temporary directories that are removed on drop.
//!
//! Must NOT: reuse or delete paths it did not create.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp dir, deleted (recursively) on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates `<tmp>/agend-test-<label>-<pid>-<n>`. Fails if it already exists.
    pub fn new(label: &str) -> io::Result<TempDir> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("agend-test-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir(&path)?;
        Ok(TempDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_are_unique_and_removed_on_drop() {
        let a = TempDir::new("unit").unwrap();
        let b = TempDir::new("unit").unwrap();
        assert_ne!(a.path(), b.path());
        std::fs::write(a.path().join("f"), b"x").unwrap();
        let kept = a.path().to_path_buf();
        drop(a);
        assert!(!kept.exists());
        assert!(b.path().is_dir());
    }
}
