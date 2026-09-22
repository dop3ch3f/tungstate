//! Reading files for the duplicate pass, and remembering what was read.
//!
//! [`tungstate_core::dupes`] does no I/O: it asks this. Two levels, because
//! the second is the expensive one — a partial digest is 128 KiB however large
//! the file, and a whole one is every byte.
//!
//! Every answer is written to the journal, keyed by the path and checked
//! against size and mtime. A second pass over a folder nobody has touched
//! reads nothing at all; a file edited in place keeps its name, so the check
//! is what makes a remembered digest safe rather than merely fast.

use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use tungstate_backend::Backend;
use tungstate_core::dupes::Digest;
use tungstate_journal::{Journal, Remembered};

/// How much of each end a partial digest reads. DESIGN §5.
const ENDS: u64 = 64 * 1024;

/// A [`Digest`] that reads through a backend and remembers what it computed.
pub struct Cached<'a> {
    backend: &'a dyn Backend,
    journal: &'a Journal,
    root: String,
    /// Digests taken this pass, so nothing is read twice even before the
    /// journal is consulted.
    hits: usize,
    reads: usize,
}

impl<'a> Cached<'a> {
    /// A digest for one folder.
    #[must_use]
    pub fn new(backend: &'a dyn Backend, journal: &'a Journal, root: &str) -> Self {
        Self {
            backend,
            journal,
            root: root.to_string(),
            hits: 0,
            reads: 0,
        }
    }

    /// How many digests came from the journal rather than the disk.
    #[must_use]
    pub fn hits(&self) -> usize {
        self.hits
    }

    /// How many files were actually read.
    #[must_use]
    pub fn reads(&self) -> usize {
        self.reads
    }

    /// Size and mtime as the cache keys them: mtime in whole seconds, because
    /// that is the most every backend agrees on (FTP rounds to the minute).
    fn stamp(&self, path: &str) -> Result<(u64, Option<i64>), String> {
        let meta = self
            .backend
            .stat(Path::new(path))
            .map_err(|error| error.to_string())?;
        let mtime = meta.modified.and_then(|at| {
            at.duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|since| i64::try_from(since.as_secs()).ok())
        });
        Ok((meta.len, mtime))
    }

    fn recall(&mut self, path: &str, whole: bool) -> Result<Option<String>, String> {
        let (size, mtime) = self.stamp(path)?;
        let found = self
            .journal
            .remembered(&self.root, path, size, mtime)
            .map_err(|error| error.to_string())?;
        let digest = found.and_then(|kept| if whole { kept.whole } else { kept.partial });
        if digest.is_some() {
            self.hits += 1;
        }
        Ok(digest)
    }

    fn keep(&self, path: &str, digest: &Remembered) -> Result<(), String> {
        let (size, mtime) = self.stamp(path)?;
        self.journal
            .remember(&self.root, path, size, mtime, digest)
            .map_err(|error| error.to_string())
    }
}

impl Digest for Cached<'_> {
    fn partial(&mut self, path: &str) -> Result<String, String> {
        if let Some(found) = self.recall(path, false)? {
            return Ok(found);
        }
        let (size, _) = self.stamp(path)?;
        let mut hasher = blake3::Hasher::new();
        // The size goes in first, so two different files whose ends match by
        // chance are still told apart at this tier rather than at the next.
        hasher.update(&size.to_le_bytes());
        let head = self
            .backend
            .read_prefix(Path::new(path), ENDS)
            .map_err(|error| error.to_string())?;
        hasher.update(&head);
        if size > ENDS * 2 {
            // No ranged read from the end exists on the trait, so the tail is
            // reached by reading and discarding. Still one pass over the file
            // at worst, and on a local disk the skipped bytes cost nothing.
            let mut reader = self
                .backend
                .open_read(Path::new(path))
                .map_err(|error| error.to_string())?;
            let skip = size - ENDS;
            std::io::copy(&mut (&mut reader).take(skip), &mut std::io::sink())
                .map_err(|error| error.to_string())?;
            let mut tail = Vec::with_capacity(usize::try_from(ENDS).unwrap_or(0));
            reader
                .take(ENDS)
                .read_to_end(&mut tail)
                .map_err(|error| error.to_string())?;
            hasher.update(&tail);
        }
        self.reads += 1;
        let digest = hasher.finalize().to_hex().to_string();
        self.keep(
            path,
            &Remembered {
                partial: Some(digest.clone()),
                whole: None,
            },
        )?;
        Ok(digest)
    }

    fn whole(&mut self, path: &str) -> Result<String, String> {
        if let Some(found) = self.recall(path, true)? {
            return Ok(found);
        }
        let mut reader = self
            .backend
            .open_read(Path::new(path))
            .map_err(|error| error.to_string())?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        self.reads += 1;
        let digest = hasher.finalize().to_hex().to_string();
        self.keep(
            path,
            &Remembered {
                partial: None,
                whole: Some(digest.clone()),
            },
        )?;
        Ok(digest)
    }
}
