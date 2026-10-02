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

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use jiff::Timestamp;
use tungstate_backend::Backend;
use tungstate_backend::counted::{Stage, Tally};
use tungstate_core::Snapshot;
use tungstate_core::dupes::{Digest, STOPPED};
use tungstate_journal::{Journal, Remembered};

/// How much each sample reads. DESIGN §5 asks for the two ends; the interior
/// ones are what make a sampled match worth reporting on a network volume.
const SAMPLE: u64 = 64 * 1024;

/// Where the samples are taken, as fractions of the file. The ends catch a
/// header and a trailer; the interior two catch two files that share both,
/// which is every re-encode of the same video.
const AT: [f64; 2] = [0.33, 0.66];

/// Told how far a pass has got. Boxed because the caller decides what to do
/// with it: the window emits an event, the command line counts.
type Watcher<'a> = Box<dyn FnMut(&Watch) + Send + 'a>;

/// Asked before every file, so a scan can be abandoned.
type Asked<'a> = Box<dyn Fn() -> bool + Send + 'a>;

/// How far a pass has got, for whoever is watching it.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    /// Files digested so far, however the answer was arrived at.
    pub looked: usize,
    /// Files read from the storage.
    pub read: usize,
    /// Files whose digest came from the journal.
    pub recalled: usize,
    /// Bytes pulled from the storage.
    pub bytes: u64,
    /// The file being handled right now.
    pub path: String,
}

/// A [`Digest`] that reads through a backend and remembers what it computed.
pub struct Cached<'a> {
    backend: &'a dyn Backend,
    journal: &'a Journal,
    root: String,
    /// Digests taken this pass, so nothing is read twice even before the
    /// journal is consulted.
    hits: usize,
    reads: usize,
    /// Digests the journal already knew because a transfer computed them.
    recorded: usize,
    /// Bytes read from the storage.
    bytes: u64,
    /// Told how far the pass has got, if anybody is watching. A scan of a
    /// drive takes minutes, and a window with no progress looks hung.
    watcher: Option<Watcher<'a>>,
    /// Asked before every file. `true` ends the pass where it stands, which
    /// is safe because this only ever reads.
    stop: Option<Asked<'a>>,
    looked: usize,
    /// Size and mtime by path, as the listing saw them. Each digest needs
    /// both, and asking the storage again costs a round trip per file.
    known: HashMap<String, Stamp>,
}

/// Size, and mtime in whole seconds, as the cache keys a file.
type Stamp = (u64, Option<i64>);

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
            recorded: 0,
            bytes: 0,
            watcher: None,
            stop: None,
            looked: 0,
            known: HashMap::new(),
        }
    }

    /// Take each file's size and mtime from `snapshot` rather than asking the
    /// storage for them again.
    ///
    /// For a pass over a listing taken moments ago. Not for confirming a
    /// group before acting on it: that wants the file as it is now, so a
    /// change since the listing is a cache miss and a fresh read.
    #[must_use]
    pub fn knowing(mut self, snapshot: &Snapshot) -> Self {
        self.known = snapshot
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| {
                // Whole seconds since 1970, as `stamp` reads a `SystemTime`:
                // nothing before it, and truncated rather than rounded.
                let mtime = entry
                    .mtime
                    .filter(|at| *at >= Timestamp::UNIX_EPOCH)
                    .map(Timestamp::as_second);
                (entry.relative_path(), (entry.size, mtime))
            })
            .collect();
        self
    }

    /// Report progress to `watcher` as the pass goes.
    #[must_use]
    pub fn watched_by(mut self, watcher: Watcher<'a>) -> Self {
        self.watcher = Some(watcher);
        self
    }

    /// Stop when `stop` says so. Checked before every file.
    #[must_use]
    pub fn stopping_when(mut self, stop: Asked<'a>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Called at the top of every digest: refuses when asked to stop, and
    /// tells whoever is watching where the pass has got to.
    fn note(&mut self, path: &str) -> Result<(), String> {
        if self.stop.as_ref().is_some_and(|asked| asked()) {
            return Err(STOPPED.to_string());
        }
        self.looked += 1;
        if let Some(watcher) = self.watcher.as_mut() {
            watcher(&Watch {
                looked: self.looked,
                read: self.reads,
                recalled: self.hits + self.recorded,
                bytes: self.bytes,
                path: path.to_string(),
            });
        }
        Ok(())
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

    /// How many whole-file digests came from a transfer's own record, so cost
    /// no reading at all.
    #[must_use]
    pub fn recorded(&self) -> usize {
        self.recorded
    }

    /// Bytes pulled from the storage this pass.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Size and mtime as the cache keys them: mtime in whole seconds, because
    /// that is the most every backend agrees on (FTP rounds to the minute).
    fn stamp(&self, path: &str) -> Result<Stamp, String> {
        if let Some(known) = self.known.get(path) {
            return Ok(*known);
        }
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

    /// `len` bytes from `offset`, as one ranged read where the backend can do
    /// one. Over FTP that is `REST` then `RETR`, so a sample of a 4 GB video
    /// costs 64 KiB rather than 4 GB.
    fn read_at(&mut self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let bytes = self
            .backend
            .read_range(Path::new(path), offset, len)
            .map_err(|error| error.to_string())?;
        self.bytes += bytes.len() as u64;
        Ok(bytes)
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
        self.note(path)?;
        if let Some(found) = self.recall(path, false)? {
            return Ok(found);
        }
        let (size, _) = self.stamp(path)?;
        let mut hasher = blake3::Hasher::new();
        // The size goes in first, so two different files whose samples match
        // by chance are still told apart here rather than at the next tier.
        hasher.update(&size.to_le_bytes());
        hasher.update(&self.read_at(path, 0, SAMPLE)?);
        for fraction in AT {
            // Interior samples: two files that share a header and a trailer
            // are the ordinary case for anything from one camera or encoder.
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation
            )]
            let at = (size as f64 * fraction) as u64;
            if at > SAMPLE && size > at + SAMPLE {
                hasher.update(&self.read_at(path, at, SAMPLE)?);
            }
        }
        if size > SAMPLE * 2 {
            hasher.update(&self.read_at(path, size - SAMPLE, SAMPLE)?);
        }
        self.reads += 1;
        let digest = hasher.finalize().to_hex().to_string();
        self.keep(
            path,
            &Remembered {
                partial: Some(digest.clone()),
                ..Remembered::default()
            },
        )?;
        Ok(digest)
    }

    fn whole(&mut self, path: &str) -> Result<String, String> {
        self.note(path)?;
        if let Some(found) = self.recall(path, true)? {
            return Ok(found);
        }
        // A verified transfer wrote this file and recorded what it hashed, so
        // for anything tungstate put here the answer costs nothing at all.
        let (size, mtime) = self.stamp(path)?;
        if let Some(found) = self
            .journal
            .hash_at(&self.root, path, size, mtime.map(|seconds| seconds * 1_000))
            .map_err(|error| error.to_string())?
        {
            self.recorded += 1;
            self.keep(
                path,
                &Remembered {
                    whole: Some(found.clone()),
                    ..Remembered::default()
                },
            )?;
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
            self.bytes += read as u64;
            hasher.update(&buffer[..read]);
        }
        self.reads += 1;
        let digest = hasher.finalize().to_hex().to_string();
        self.keep(
            path,
            &Remembered {
                whole: Some(digest.clone()),
                ..Remembered::default()
            },
        )?;
        Ok(digest)
    }
}

/// A [`Digest`] that tells a [`Tally`] which stage its reads belong to.
///
/// The pass asks for samples and whole files in turn, and the backend cannot
/// tell the two apart, so the switch happens here, one call above it.
pub struct Staged<'a> {
    inner: &'a mut dyn Digest,
    tally: &'a Tally,
}

impl<'a> Staged<'a> {
    /// Count `inner`'s reads into `tally`, by stage.
    #[must_use]
    pub fn new(inner: &'a mut dyn Digest, tally: &'a Tally) -> Self {
        Self { inner, tally }
    }
}

impl Digest for Staged<'_> {
    fn partial(&mut self, path: &str) -> Result<String, String> {
        self.tally.enter(Stage::Sample);
        self.inner.partial(path)
    }

    fn whole(&mut self, path: &str) -> Result<String, String> {
        self.tally.enter(Stage::Whole);
        self.inner.whole(path)
    }
}
