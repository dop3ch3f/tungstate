//! Small pictures, kept outside the journal.
//!
//! A thumbnail is a file, not a fact, so it lives in the operating system's
//! own cache directory rather than in the database. Deleting the whole folder
//! costs the next scan a redraw and nothing else, which is what a cache should
//! be worth.

use std::path::{Path, PathBuf};

/// Where the small pictures for one machine live.
#[derive(Debug, Clone)]
pub struct Thumbs {
    dir: PathBuf,
}

impl Thumbs {
    /// Keep pictures under `dir`, making it if it is not there.
    ///
    /// # Errors
    /// Whatever making the directory failed with.
    pub fn at(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// The name a file's picture is kept under.
    ///
    /// Derived from the path **and** its size and mtime, so an edited file
    /// gets a new name rather than the old picture. The cost is that the old
    /// one lingers, which is what [`sweep`](Self::sweep) is for.
    #[must_use]
    pub fn name(root: &str, path: &str, size: u64, mtime: Option<i64>) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(root.as_bytes());
        hasher.update(b"\0");
        hasher.update(path.as_bytes());
        hasher.update(&size.to_le_bytes());
        hasher.update(&mtime.unwrap_or_default().to_le_bytes());
        format!("{}.jpg", &hasher.finalize().to_hex()[..24])
    }

    /// Where a picture of that name would be.
    #[must_use]
    pub fn at_name(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Write one.
    ///
    /// # Errors
    /// Whatever writing the file failed with.
    pub fn keep(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::write(self.at_name(name), bytes)
    }

    /// Throw away everything kept here.
    ///
    /// # Errors
    /// Whatever removing the directory failed with.
    pub fn sweep(&self) -> std::io::Result<()> {
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|it| it == "jpg") {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    /// Throws away the oldest pictures until the rest fit in `most` bytes.
    ///
    /// Oldest by when each was written, which for a cache read far more than
    /// it is written is close enough to least recently used, and costs no
    /// bookkeeping on every read. Returns how many went.
    ///
    /// # Errors
    /// Whatever listing the directory failed with. A picture that will not
    /// go is skipped: another process may be serving it.
    pub fn trim(&self, most: u64) -> std::io::Result<usize> {
        let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|it| it != "jpg") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let written = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                kept.push((written, meta.len(), path));
            }
        }
        let mut total: u64 = kept.iter().map(|(_, len, _)| len).sum();
        kept.sort_by_key(|(written, ..)| *written);
        let mut gone = 0;
        for (_, len, path) in kept {
            if total <= most {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                gone += 1;
            }
        }
        Ok(gone)
    }

    /// The directory itself, for whoever has to serve these to a window.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    #[test]
    fn trimming_keeps_the_newest_pictures_under_the_limit() {
        let dir = tempfile::tempdir().expect("dir");
        let thumbs = Thumbs::at(dir.path()).expect("thumbs");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        for (name, later) in [("a.jpg", 0), ("b.jpg", 1), ("c.jpg", 2)] {
            thumbs.keep(name, &[0u8; 100]).expect("keep");
            let file = std::fs::File::options()
                .write(true)
                .open(thumbs.at_name(name))
                .expect("open");
            file.set_modified(old + std::time::Duration::from_secs(later * 10))
                .expect("age");
        }
        // Somebody else's file in the same folder is never counted or removed.
        std::fs::write(dir.path().join("notes.txt"), [0u8; 500]).expect("other");

        assert_eq!(thumbs.trim(250).expect("trim"), 1);
        assert!(!thumbs.at_name("a.jpg").exists(), "the oldest goes first");
        assert!(thumbs.at_name("b.jpg").exists() && thumbs.at_name("c.jpg").exists());
        assert!(dir.path().join("notes.txt").exists());
        assert_eq!(
            thumbs.trim(250).expect("again"),
            0,
            "already under the limit"
        );
    }
}
