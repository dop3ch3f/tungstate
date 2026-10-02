//! Counting what a backend is asked to do, stage by stage.
//!
//! Over a network the time goes on round trips and bytes moved, so those are
//! what `dedupe --timings` reports. [`Counted`] wraps any backend, forwards
//! every call untouched, and adds each request and each byte read to a
//! [`Tally`] under whichever [`Stage`] the caller says the work belongs to.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use crate::{Backend, Capabilities, Entry, Meta, Result, RootToken, WriteFinish};

/// What a pass is doing while it asks the backend for something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Walking the folder: listings and per-file metadata.
    List,
    /// Reading samples of the files whose size matched another's.
    Sample,
    /// Reading files in full.
    Whole,
}

impl Stage {
    /// Every stage, in the order a pass reaches them.
    pub const ALL: [Self; 3] = [Self::List, Self::Sample, Self::Whole];

    /// Stored in an atomic, so it travels as an index.
    const NONE: usize = usize::MAX;

    fn index(self) -> usize {
        self as usize
    }
}

/// What one stage cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spent {
    /// Calls made to the backend: a listing, a `stat`, an open, a ranged read.
    pub requests: u64,
    /// Bytes handed back by the backend.
    pub bytes: u64,
    /// Wall clock spent in the stage, whatever it went on.
    pub time: Duration,
}

#[derive(Debug, Default)]
struct Counters {
    requests: AtomicU64,
    bytes: AtomicU64,
    nanos: AtomicU64,
}

/// Running totals, safe to add to from several threads at once.
#[derive(Debug)]
pub struct Tally {
    stage: AtomicUsize,
    /// When the current stage was entered. A lock rather than an atomic
    /// because switching reads the old instant and writes the new one together.
    since: Mutex<Instant>,
    counters: [Counters; 3],
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            stage: AtomicUsize::new(Stage::NONE),
            since: Mutex::new(Instant::now()),
            counters: Default::default(),
        }
    }
}

impl Tally {
    /// Nothing counted yet, and no stage entered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Charge the time since the last switch to the stage that was running,
    /// then count everything from now on under `stage`.
    pub fn enter(&self, stage: Stage) {
        self.switch(stage.index());
    }

    /// Charge the running stage its time and count nothing more.
    pub fn stop(&self) {
        self.switch(Stage::NONE);
    }

    fn switch(&self, to: usize) {
        let mut since = self.since.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let from = self.stage.swap(to, Ordering::Relaxed);
        if let Some(counters) = self.counters.get(from) {
            let spent = u64::try_from((now - *since).as_nanos()).unwrap_or(u64::MAX);
            counters.nanos.fetch_add(spent, Ordering::Relaxed);
        }
        *since = now;
    }

    /// What `stage` has cost so far.
    #[must_use]
    pub fn spent(&self, stage: Stage) -> Spent {
        let counters = &self.counters[stage.index()];
        Spent {
            requests: counters.requests.load(Ordering::Relaxed),
            bytes: counters.bytes.load(Ordering::Relaxed),
            time: Duration::from_nanos(counters.nanos.load(Ordering::Relaxed)),
        }
    }

    fn current(&self) -> Option<&Counters> {
        self.counters.get(self.stage.load(Ordering::Relaxed))
    }

    fn request(&self) {
        if let Some(counters) = self.current() {
            counters.requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn read(&self, bytes: u64) {
        if let Some(counters) = self.current() {
            counters.bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }
}

/// A backend that counts what it is asked, and otherwise is the one it wraps.
pub struct Counted<'a> {
    inner: &'a dyn Backend,
    tally: Arc<Tally>,
}

impl<'a> Counted<'a> {
    /// Count every call made to `inner` into `tally`.
    #[must_use]
    pub fn new(inner: &'a dyn Backend, tally: Arc<Tally>) -> Self {
        Self { inner, tally }
    }

    fn ask<T>(&self, answer: Result<T>) -> Result<T> {
        self.tally.request();
        answer
    }

    fn bytes(&self, answer: Result<Vec<u8>>) -> Result<Vec<u8>> {
        let bytes = self.ask(answer)?;
        self.tally.read(bytes.len() as u64);
        Ok(bytes)
    }
}

/// A reader that adds what passes through it to a tally.
///
/// Owns an `Arc` rather than borrowing, because a reader handed out by a
/// backend must outlive the call that opened it.
struct CountedRead {
    inner: Box<dyn Read + Send>,
    tally: Arc<Tally>,
}

impl Read for CountedRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.tally.read(read as u64);
        Ok(read)
    }
}

impl Backend for Counted<'_> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn root_token(&self) -> Result<RootToken> {
        self.ask(self.inner.root_token())
    }

    fn stat(&self, path: &Path) -> Result<Meta> {
        self.ask(self.inner.stat(path))
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>> {
        self.ask(self.inner.read_dir(path))
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>> {
        let inner = self.ask(self.inner.open_read(path))?;
        Ok(Box::new(CountedRead {
            inner,
            tally: Arc::clone(&self.tally),
        }))
    }

    fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> {
        self.bytes(self.inner.read_prefix(path, len))
    }

    fn set_modified(&self, path: &Path, at: SystemTime) -> Result<bool> {
        self.ask(self.inner.set_modified(path, at))
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteFinish>> {
        self.ask(self.inner.create_write(path))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.ask(self.inner.rename(from, to))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.ask(self.inner.remove_file(path))
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        self.ask(self.inner.remove_dir(path))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.ask(self.inner.create_dir_all(path))
    }

    fn on_this_machine(&self, path: &Path) -> Result<Option<PathBuf>> {
        self.inner.on_this_machine(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalBackend;

    #[test]
    fn requests_and_bytes_land_in_the_stage_that_asked() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("a.bin"), vec![7_u8; 1000]).expect("file");
        let local = LocalBackend::new(dir.path().to_path_buf());
        let tally = Arc::new(Tally::new());
        let counted = Counted::new(&local, Arc::clone(&tally));

        tally.enter(Stage::List);
        counted.read_dir(Path::new("")).expect("listed");
        counted.stat(Path::new("a.bin")).expect("stat");
        tally.enter(Stage::Sample);
        assert_eq!(
            counted
                .read_prefix(Path::new("a.bin"), 100)
                .expect("read")
                .len(),
            100
        );
        tally.enter(Stage::Whole);
        let mut all = Vec::new();
        counted
            .open_read(Path::new("a.bin"))
            .expect("open")
            .read_to_end(&mut all)
            .expect("read");
        tally.stop();
        // Not counted: no stage is running.
        counted.stat(Path::new("a.bin")).expect("stat");

        let list = tally.spent(Stage::List);
        assert_eq!((list.requests, list.bytes), (2, 0));
        let sample = tally.spent(Stage::Sample);
        assert_eq!((sample.requests, sample.bytes), (1, 100));
        let whole = tally.spent(Stage::Whole);
        assert_eq!((whole.requests, whole.bytes), (1, 1000));
    }
}
