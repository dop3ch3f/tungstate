//! What finding duplicates costs, measured: files, requests, bytes, seconds.
//!
//! Not part of the product. It makes a folder that looks like a photo library,
//! then runs the duplicate pass over it the way the window does, twice (the
//! second time with the digests remembered), and prints a table per stage.
//!
//! ```text
//! cargo run --release -p tungstate-cli --example dupes_bench -- make DIR [FILES]
//! cargo run --release -p tungstate-cli --example dupes_bench -- local DIR
//! cargo run --release -p tungstate-cli --example dupes_bench -- slow DIR
//! TUNGSTATE_SMB_PASSWORD=… cargo run --release -p tungstate-cli --example dupes_bench -- \
//!     smb HOST PORT USER SHARE/FOLDER
//! ```
//!
//! `local` also measures a mounted share: point it at a folder under the
//! mount and the backend notices the network itself. `slow` wraps the local
//! backend in a fake network, 20 ms per request and 50 MB/s shared, so the
//! time follows requests and bytes the way a NAS's does, and repeats exactly.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use tungstate_backend::counted::{Counted, Stage, Tally};
use tungstate_backend::local::LocalBackend;
use tungstate_backend::{Backend, Capabilities, Entry, Meta, Result, RootToken, WriteFinish};
use tungstate_core::dupes::{self, Found, Wants};
use tungstate_execute::digest::{Cached, Staged};
use tungstate_journal::Journal;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |at: usize| {
        args.get(at)
            .cloned()
            .unwrap_or_else(|| usage("missing an argument"))
    };
    match arg(0).as_str() {
        "make" => {
            let files = args.get(2).map_or(5_000, |n| {
                n.parse().unwrap_or_else(|_| usage("FILES is a number"))
            });
            make(Path::new(&arg(1)), files);
        }
        "local" => {
            let local = LocalBackend::new(PathBuf::from(arg(1)));
            measure(&local, &arg(1));
        }
        "slow" => {
            let local = LocalBackend::new(PathBuf::from(arg(1)));
            measure(&Slow::new(&local), &arg(1));
        }
        "smb" => {
            let password = std::env::var("TUNGSTATE_SMB_PASSWORD")
                .unwrap_or_else(|_| usage("set TUNGSTATE_SMB_PASSWORD"));
            let settings = tungstate_backend_smb::Settings {
                host: arg(1),
                port: Some(arg(2).parse().unwrap_or_else(|_| usage("PORT is a number"))),
                root: arg(4),
                username: arg(3),
                password,
                require_encryption: false,
            };
            let smb = tungstate_backend_smb::SmbBackend::connect(&settings, Path::new(""), "bench")
                .unwrap_or_else(|error| panic!("could not reach the share: {error}"));
            measure(&smb, &format!("smb:{}", arg(4)));
        }
        _ => usage("make, local, slow or smb"),
    }
}

fn usage(why: &str) -> ! {
    eprintln!(
        "dupes_bench: {why}\n  make DIR [FILES] | local DIR | slow DIR | smb HOST PORT USER SHARE/FOLDER"
    );
    std::process::exit(2)
}

// ---------------------------------------------------------------------------
// Measuring

/// Run the pass cold, then again with what it remembered, and print both.
fn measure(backend: &dyn Backend, root: &str) {
    let journal = Journal::open_in_memory().expect("an in-memory journal");
    println!("| run | stage | files | requests | read | seconds |");
    println!("|---|---|---:|---:|---:|---:|");
    let mut found = Vec::new();
    for run in ["cold", "warm"] {
        let tally = Arc::new(Tally::new());
        let counted = Counted::new(backend, Arc::clone(&tally));
        let (listed, this) = pass(&counted, &journal, root, &tally);
        let files = [listed, this.digested, this.read_whole];
        for (stage, files) in Stage::ALL.into_iter().zip(files) {
            let spent = tally.spent(stage);
            println!(
                "| {run} | {stage:?} | {files} | {} | {} | {:.1} |",
                spent.requests,
                megabytes(spent.bytes),
                spent.time.as_secs_f64()
            );
        }
        found.push(this);
    }
    let first = &found[0];
    println!(
        "\n{} group(s), {} unconfirmed, {} extra file(s), {} reclaimable",
        first.groups.len() + first.folders.len(),
        first.unsure(),
        first.extra_files(),
        megabytes(first.reclaimable())
    );
    assert_eq!(found[0], found[1], "a warm run finds what a cold one did");
}

/// The duplicate pass as the window runs it: list, then sample or read.
fn pass(backend: &dyn Backend, journal: &Journal, root: &str, tally: &Tally) -> (usize, Found) {
    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    tally.enter(Stage::List);
    let snapshot = tungstate_attrs::survey_at(backend, &probe, tungstate_attrs::Tier::Stat)
        .expect("the folder lists");
    let listed = snapshot
        .entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .count();
    let wants = Wants {
        sampled: backend.capabilities().networked,
        ..Wants::default()
    };
    let mut digest = Cached::new(backend, journal, root).knowing(&snapshot);
    let found = dupes::find(&snapshot, &mut Staged::new(&mut digest, tally), &wants)
        .expect("the pass runs");
    tally.stop();
    (listed, found)
}

#[allow(clippy::cast_precision_loss)]
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

// ---------------------------------------------------------------------------
// A network that repeats exactly

/// The local backend, made to cost what a NAS costs: a fixed wait per request
/// and one shared pipe for every byte.
struct Slow<'a> {
    inner: &'a dyn Backend,
    pipe: Arc<Pipe>,
}

/// Bytes queue for one link, whichever thread asked for them.
struct Pipe {
    free_at: Mutex<Instant>,
}

const LATENCY: Duration = Duration::from_millis(20);
const BYTES_PER_SECOND: f64 = 50e6;

impl Pipe {
    /// Wait until `bytes` would have arrived, behind everything already sent.
    ///
    /// Reads come 8 KiB at a time and a sleep that short overshoots by more
    /// than it lasts, so time is owed until there is enough to sleep through,
    /// and an overshoot is credited to the next read rather than lost. Only a
    /// pipe idle for longer than `IDLE` starts again from now.
    fn carry(&self, bytes: u64) {
        const IDLE: Duration = Duration::from_millis(50);
        const SETTLE: Duration = Duration::from_millis(5);
        #[allow(clippy::cast_precision_loss)]
        let takes = Duration::from_secs_f64(bytes as f64 / BYTES_PER_SECOND);
        let now = Instant::now();
        let owed = {
            let mut free_at = self.free_at.lock().unwrap_or_else(PoisonError::into_inner);
            if *free_at + IDLE < now {
                *free_at = now;
            }
            *free_at += takes;
            free_at.saturating_duration_since(now)
        };
        if owed >= SETTLE {
            std::thread::sleep(owed);
        }
    }
}

impl<'a> Slow<'a> {
    fn new(inner: &'a dyn Backend) -> Self {
        Self {
            inner,
            pipe: Arc::new(Pipe {
                free_at: Mutex::new(Instant::now()),
            }),
        }
    }
}

/// One round trip's wait, taken by the thread that asked.
fn delayed<T>(answer: Result<T>) -> Result<T> {
    std::thread::sleep(LATENCY);
    answer
}

struct SlowRead {
    inner: Box<dyn Read + Send>,
    pipe: Arc<Pipe>,
}

impl Read for SlowRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.pipe.carry(read as u64);
        Ok(read)
    }
}

impl Backend for Slow<'_> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            networked: true,
            ..self.inner.capabilities()
        }
    }

    fn root_token(&self) -> Result<RootToken> {
        delayed(self.inner.root_token())
    }

    fn stat(&self, path: &Path) -> Result<Meta> {
        delayed(self.inner.stat(path))
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>> {
        delayed(self.inner.read_dir(path))
    }

    fn listing_is_complete(&self) -> bool {
        self.inner.listing_is_complete()
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>> {
        let inner = delayed(self.inner.open_read(path))?;
        Ok(Box::new(SlowRead {
            inner,
            pipe: Arc::clone(&self.pipe),
        }))
    }

    fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> {
        let bytes = delayed(self.inner.read_prefix(path, len))?;
        self.pipe.carry(bytes.len() as u64);
        Ok(bytes)
    }

    fn read_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
        let bytes = delayed(self.inner.read_range(path, offset, len))?;
        self.pipe.carry(bytes.len() as u64);
        Ok(bytes)
    }

    fn set_modified(&self, path: &Path, at: SystemTime) -> Result<bool> {
        delayed(self.inner.set_modified(path, at))
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteFinish>> {
        delayed(self.inner.create_write(path))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        delayed(self.inner.rename(from, to))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        delayed(self.inner.remove_file(path))
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        delayed(self.inner.remove_dir(path))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        delayed(self.inner.create_dir_all(path))
    }
}

// ---------------------------------------------------------------------------
// A folder that looks like a photo library

/// splitmix64: a fixed seed gives the same folder on every machine.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.next() % (high - low)
    }

    fn below(&mut self, high: usize) -> usize {
        usize::try_from(self.next() % high as u64).expect("small")
    }
}

/// What one file holds: three 4 KiB blocks of noise, zeros between. Sparse,
/// so a 300 MB video costs 12 KiB of disk; APFS fills the holes in anything
/// under about 32 MB, so the photos still cost what they weigh.
#[derive(Clone, Copy)]
struct Content {
    size: u64,
    head: u64,
    middle: u64,
    tail: u64,
}

const BLOCK: usize = 4096;

fn make(root: &Path, files: usize) {
    let mut dice = Dice(0x7475_6e67_7374_6174);
    let copies = files / 5;
    let originals = files - copies;
    let years = 2019..2025;
    let mut made: Vec<(String, Content)> = Vec::with_capacity(files);

    for n in 0..originals {
        let year = years.start + dice.between(0, 6);
        let video = dice.below(100) < 15;
        let size = if video {
            dice.between(20_000_000, 300_000_000)
        } else {
            dice.between(1_500_000, 9_000_000)
        };
        let id = n as u64 + 1;
        let mut content = Content {
            size,
            head: id,
            middle: id,
            tail: id,
        };
        // One in fifty matches an earlier file's size and is not that file:
        // half differ everywhere, half only in the middle, where a sample of
        // the ends and the thirds never looks.
        if n > 0 && dice.below(50) == 0 {
            let (_, twin) = made[dice.below(made.len())];
            content.size = twin.size;
            if dice.below(2) == 0 {
                content.head = twin.head;
                content.tail = twin.tail;
            }
        }
        let path = if video {
            format!("Videos/{year}/VID_{n:05}.MOV")
        } else {
            format!("Photos/{year}/IMG_{n:05}.JPG")
        };
        made.push((path, content));
    }
    for n in 0..copies {
        let (from, content) = made[dice.below(originals)].clone();
        let name = Path::new(&from)
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .to_string();
        let path = match dice.below(3) {
            0 => format!("Backup/phone-{n:05}/{name}"),
            1 => format!("Exports/{n:05} {name}"),
            _ => format!("Downloads/copy {n:05} of {name}"),
        };
        made.push((path, content));
    }

    let mut bytes = 0;
    for (path, content) in &made {
        write_sparse(&root.join(path), *content);
        bytes += content.size;
    }
    println!(
        "made {} files ({originals} originals, {copies} copies), {} apparent",
        made.len(),
        megabytes(bytes)
    );
}

fn write_sparse(path: &Path, content: Content) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    let mut file = std::fs::File::create(path).expect("a file");
    file.set_len(content.size).expect("a length");
    let middle = content.size / 2;
    let tail = content.size.saturating_sub(BLOCK as u64);
    for (at, seed) in [
        (0, content.head),
        (middle, content.middle),
        (tail, content.tail),
    ] {
        let mut dice = Dice(seed);
        let block: Vec<u8> = (0..BLOCK / 8)
            .flat_map(|_| dice.next().to_le_bytes())
            .collect();
        file.seek(SeekFrom::Start(at)).expect("seek");
        file.write_all(&block).expect("write");
    }
}
