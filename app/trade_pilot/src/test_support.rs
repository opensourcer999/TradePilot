//! Helpers for the tests: a folder of its own in the temporary folder of the system that is removed again when the test is over
//! (also when it fails), so a run of the tests leaves nothing behind.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh folder in the temporary folder of the system, removed with all it holds when the value is dropped. The folder is not made
/// yet; code that needs it makes it (like the real data folder).
pub struct TempDir(PathBuf);

impl TempDir {
    /// A name of its own: the label, the process and a counter, so tests that run side by side never share a folder.
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let number = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tradepilot-test-{label}-{}-{number}", std::process::id()));
        let _ignored = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_temp_folder_is_new_each_time_and_gone_with_its_owner() {
        let kept;
        {
            let first = TempDir::new("support");
            let second = TempDir::new("support");
            assert_ne!(*first, *second);
            std::fs::create_dir_all(first.join("deeper")).unwrap();
            std::fs::write(first.join("deeper").join("file.txt"), "x").unwrap();
            kept = first.to_path_buf();
            assert!(kept.is_dir());
        }
        assert!(!kept.exists(), "removed when dropped");
    }
}
