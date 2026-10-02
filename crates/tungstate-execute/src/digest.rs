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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, UNIX_EPOCH};

use jiff::Timestamp;
use tungstate_backend::counted::{Stage, Tally};
use tungstate_backend::{Backend, BackendError};
use tungstate_core::Snapshot;
use tungstate_core::dupes::{Digest, STOPPED, Trouble};
use tungstate_journal::{Journal, Remembered};
use tungstate_transfer::{Governor, Limits, is_overload};

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
    /// How many files are read at once, when it is not left to the governor.
    width: Option<usize>,
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
            width: None,
        }
    }

    /// Read `width` files at once rather than sensing how many the storage
    /// takes. For tests and measurements; nothing a person sees sets it.
    #[must_use]
    pub fn at_once(mut self, width: usize) -> Self {
        self.width = Some(width.max(1));
        self
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

    /// The digest without reading the file, if anything already knows it:
    /// this cache, or for a whole digest, the transfer that wrote the file.
    fn known_answer(&mut self, path: &str, whole: bool) -> Result<Option<String>, String> {
        let (size, mtime) = self.stamp(path)?;
        let found = self
            .journal
            .remembered(&self.root, path, size, mtime)
            .map_err(|error| error.to_string())?
            .and_then(|kept| if whole { kept.whole } else { kept.partial });
        if found.is_some() {
            self.hits += 1;
            return Ok(found);
        }
        if !whole {
            return Ok(None);
        }
        // A verified transfer wrote this file and recorded what it hashed, so
        // for anything tungstate put here the answer costs nothing at all.
        let Some(found) = self
            .journal
            .hash_at(&self.root, path, size, mtime.map(|seconds| seconds * 1_000))
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        self.recorded += 1;
        self.keep(path, whole, &found)?;
        Ok(Some(found))
    }

    /// Count a file just read, and remember what it hashed to.
    fn learned(&mut self, path: &str, whole: bool, read: &Hashed) -> Result<(), String> {
        self.reads += 1;
        self.bytes += read.bytes;
        self.keep(path, whole, &read.digest)
    }

    fn keep(&self, path: &str, whole: bool, digest: &str) -> Result<(), String> {
        let (size, mtime) = self.stamp(path)?;
        let remembered = if whole {
            Remembered {
                whole: Some(digest.to_string()),
                ..Remembered::default()
            }
        } else {
            Remembered {
                partial: Some(digest.to_string()),
                ..Remembered::default()
            }
        };
        self.journal
            .remember(&self.root, path, size, mtime, &remembered)
            .map_err(|error| error.to_string())
    }

    /// One file, asked on its own.
    fn one(&mut self, path: &str, whole: bool) -> Result<String, String> {
        self.note(path)?;
        if let Some(found) = self.known_answer(path, whole)? {
            return Ok(found);
        }
        let (size, _) = self.stamp(path)?;
        let read = read(self.backend, path, size, whole).map_err(|error| error.to_string())?;
        self.learned(path, whole, &read)?;
        Ok(read.digest)
    }

    /// Several files, read `width` at a time.
    ///
    /// The journal is only touched on this thread, which is the one that
    /// owns it: the readers do backend requests and hashing and nothing else,
    /// and hand each answer back to be remembered in the order it arrives.
    fn many(&mut self, paths: &[String], whole: bool) -> Result<Vec<String>, Trouble> {
        let mut answers: Vec<Option<String>> = vec![None; paths.len()];
        let mut jobs: Vec<(usize, u64)> = Vec::new();
        for (at, path) in paths.iter().enumerate() {
            let failed = |why| trouble(path, why);
            if let Some(found) = self.known_answer(path, whole).map_err(failed)? {
                self.note(path).map_err(failed)?;
                answers[at] = Some(found);
            } else {
                jobs.push((at, self.stamp(path).map_err(failed)?.0));
            }
        }

        if !jobs.is_empty() {
            let governor = self.governor();
            let backend = self.backend;
            several(
                &governor,
                &jobs,
                |&(at, size)| read(backend, &paths[at], size, whole),
                |arrived| {
                    let Some((&(at, _), read)) = arrived else {
                        return if self.stop.as_ref().is_some_and(|asked| asked()) {
                            Err(Trouble::Stopped)
                        } else {
                            Ok(())
                        };
                    };
                    let path = &paths[at];
                    let failed = |why| trouble(path, why);
                    let read = read.map_err(|error| failed(error.to_string()))?;
                    self.note(path).map_err(failed)?;
                    self.learned(path, whole, &read).map_err(failed)?;
                    answers[at] = Some(read.digest);
                    Ok(())
                },
            )?;
        }
        Ok(answers
            .into_iter()
            .map(|answer| answer.expect("every file was answered or the pass stopped"))
            .collect())
    }

    /// How many files to read at once, decided as a transfer decides it.
    ///
    /// Four over a network, but asked for one at a time from two, so a NAS
    /// that allows fewer says so with a refused `stat` rather than a failed
    /// read; eight on a disk on this machine, where there is nothing to ask.
    fn governor(&self) -> Governor {
        let fixed = |width: usize| {
            Governor::new(Limits {
                floor: width,
                ceiling: width,
                minimum: 1,
            })
        };
        if let Some(width) = self.width {
            return fixed(width);
        }
        if !self.backend.capabilities().networked {
            return fixed(8);
        }
        let governor = Governor::new(Limits {
            floor: 2,
            ceiling: 4,
            minimum: 1,
        });
        loop {
            let before = governor.limit();
            if governor.try_promote(self.backend) == before {
                break;
            }
        }
        governor.agree();
        governor
    }
}

/// A digest, and how many bytes it took.
struct Hashed {
    digest: String,
    bytes: u64,
}

/// Read and hash one file: four samples, or every byte.
///
/// Touches the backend and nothing else, so it can run on any thread.
fn read(backend: &dyn Backend, path: &str, size: u64, whole: bool) -> Result<Hashed, BackendError> {
    if whole {
        every_byte(backend, path)
    } else {
        sample(backend, path, size)
    }
}

fn sample(backend: &dyn Backend, path: &str, size: u64) -> Result<Hashed, BackendError> {
    let mut bytes = 0;
    let mut at = |offset: u64| -> Result<Vec<u8>, BackendError> {
        let piece = backend.read_range(Path::new(path), offset, SAMPLE)?;
        bytes += piece.len() as u64;
        Ok(piece)
    };
    let mut hasher = blake3::Hasher::new();
    // The size goes in first, so two different files whose samples match
    // by chance are still told apart here rather than at the next tier.
    hasher.update(&size.to_le_bytes());
    hasher.update(&at(0)?);
    for fraction in AT {
        // Interior samples: two files that share a header and a trailer
        // are the ordinary case for anything from one camera or encoder.
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        let offset = (size as f64 * fraction) as u64;
        if offset > SAMPLE && size > offset + SAMPLE {
            hasher.update(&at(offset)?);
        }
    }
    if size > SAMPLE * 2 {
        hasher.update(&at(size - SAMPLE)?);
    }
    Ok(Hashed {
        digest: hasher.finalize().to_hex().to_string(),
        bytes,
    })
}

fn every_byte(backend: &dyn Backend, path: &str) -> Result<Hashed, BackendError> {
    let mut reader = backend.open_read(Path::new(path))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut bytes = 0;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| BackendError::Io {
                path: path.into(),
                source,
            })?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok(Hashed {
        digest: hasher.finalize().to_hex().to_string(),
        bytes,
    })
}

/// A digest's failure, as the pass reports it.
fn trouble(path: &str, why: String) -> Trouble {
    if why == STOPPED {
        Trouble::Stopped
    } else {
        Trouble::Unreadable {
            path: path.to_string(),
            why,
        }
    }
}

/// Run `work` over `jobs` with as many in flight as `governor` allows, and
/// hand each answer to `take` on this thread as it arrives.
///
/// `take` is also called with `None` whenever nothing has arrived for a
/// moment, so it can notice a request to stop during a long read. Its first
/// `Err` ends the run: nothing new starts, what is in flight finishes and is
/// thrown away.
///
/// A refusal from the far side halves the width and puts the file back in
/// the queue, the same back-off a transfer uses; refused at a width of one,
/// it is an ordinary failure.
fn several<J: Sync, T: Send>(
    governor: &Governor,
    jobs: &[J],
    work: impl Fn(&J) -> Result<T, BackendError> + Sync,
    mut take: impl FnMut(Option<(&J, Result<T, BackendError>)>) -> Result<(), Trouble>,
) -> Result<(), Trouble> {
    let next = AtomicUsize::new(0);
    let retry: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let over = AtomicBool::new(false);
    let (sent, arrived) = mpsc::channel();

    std::thread::scope(|scope| {
        for worker in 0..governor.agreed().min(jobs.len()) {
            let sent = sent.clone();
            let (next, retry, over, work) = (&next, &retry, &over, &work);
            scope.spawn(move || {
                let done = || over.load(Ordering::Relaxed);
                while !done() && governor.wait_for_room(worker, &done) {
                    let queued = retry.lock().unwrap_or_else(PoisonError::into_inner).pop();
                    let job = queued.or_else(|| {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        (at < jobs.len()).then_some(at)
                    });
                    // Nothing to start, but a refused file may yet come back
                    // to the queue, so wait for the run to end rather than
                    // leave it to a worker the back-off has parked.
                    let Some(at) = job else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    let answer = work(&jobs[at]);
                    if let Err(error) = &answer
                        && is_overload(error)
                        && governor.limit() > 1
                    {
                        governor.rebuff();
                        retry
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(at);
                        continue;
                    }
                    if sent.send((at, answer)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sent);

        let mut left = jobs.len();
        let outcome = loop {
            if left == 0 {
                break Ok(());
            }
            let step = match arrived.recv_timeout(Duration::from_millis(100)) {
                Ok((at, answer)) => {
                    left -= 1;
                    take(Some((&jobs[at], answer)))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => take(None),
                // Every worker gone with answers owed: only if one panicked,
                // and the scope rethrows that panic as it closes.
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            };
            if let Err(trouble) = step {
                break Err(trouble);
            }
        };
        over.store(true, Ordering::Relaxed);
        outcome
    })
}

impl Digest for Cached<'_> {
    fn partial(&mut self, path: &str) -> Result<String, String> {
        self.one(path, false)
    }

    fn whole(&mut self, path: &str) -> Result<String, String> {
        self.one(path, true)
    }

    fn partials(&mut self, paths: &[String]) -> Result<Vec<String>, Trouble> {
        self.many(paths, false)
    }

    fn wholes(&mut self, paths: &[String]) -> Result<Vec<String>, Trouble> {
        self.many(paths, true)
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

    fn partials(&mut self, paths: &[String]) -> Result<Vec<String>, Trouble> {
        self.tally.enter(Stage::Sample);
        self.inner.partials(paths)
    }

    fn wholes(&mut self, paths: &[String]) -> Result<Vec<String>, Trouble> {
        self.tally.enter(Stage::Whole);
        self.inner.wholes(paths)
    }
}
