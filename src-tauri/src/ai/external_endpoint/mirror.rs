use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct InodeTracker {
    path: PathBuf,
    inode: Option<u64>,
    mtime: Option<SystemTime>,
    len: Option<u64>,
}

impl InodeTracker {
    pub fn for_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (inode, mtime, len) = Self::inspect_file(&path);
        Self {
            path,
            inode,
            mtime,
            len,
        }
    }

    fn inspect_file(path: &Path) -> (Option<u64>, Option<SystemTime>, Option<u64>) {
        if let Ok(meta) = std::fs::metadata(path) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                (Some(meta.ino()), meta.modified().ok(), Some(meta.len()))
            }
            #[cfg(windows)]
            {
                (None, meta.modified().ok(), Some(meta.len()))
            }
            #[cfg(not(any(unix, windows)))]
            {
                (None, meta.modified().ok(), Some(meta.len()))
            }
        } else {
            (None, None, None)
        }
    }

    /// Checks if the underlying file has changed (swapped via rename, modified, or replaced).
    ///
    /// Returns `true` if a change was detected and updates the tracked state.
    /// Subsequent calls without file changes will return `false`.
    pub fn check_and_update(&mut self) -> bool {
        let (new_inode, new_mtime, new_len) = Self::inspect_file(&self.path);

        let changed = match (self.inode, new_inode) {
            (Some(old_ino), Some(cur_ino)) => {
                old_ino != cur_ino || self.mtime != new_mtime || self.len != new_len
            }
            _ => self.mtime != new_mtime || self.len != new_len,
        };

        if changed {
            self.inode = new_inode;
            self.mtime = new_mtime;
            self.len = new_len;
            true
        } else {
            false
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inode_tracker_detects_atomic_rename_file_swap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_file = temp_dir.path().join("active.duckdb");
        let new_file = temp_dir.path().join("staged.duckdb");

        std::fs::write(&target_file, b"version 1").unwrap();
        let mut tracker = InodeTracker::for_path(&target_file);
        assert!(!tracker.check_and_update());

        // Simulate DeepSignal atomic rename:
        std::fs::write(&new_file, b"version 2").unwrap();
        std::fs::rename(&new_file, &target_file).unwrap();

        assert!(tracker.check_and_update());
        assert!(!tracker.check_and_update());
    }
}
