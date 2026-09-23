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

    /// The directory itself, for whoever has to serve these to a window.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
