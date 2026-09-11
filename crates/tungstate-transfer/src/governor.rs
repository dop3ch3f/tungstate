//! How many files a run moves at once, decided by asking rather than by
//! guessing.
//!
//! The hazard this exists for is concrete. `OpenDAL`'s FTP connection pool is
//! sized 64, and its own source documents the reply a server gives when you
//! ask for too much: `421 There are too many connections from your internet
//! address`. We build without `OpenDAL`'s retry layer, so that arrives as a
//! straight failure. A naive "use sixteen threads" against a consumer NAS
//! would not be slow, it would fail files.
//!
//! So the limit is never a number somebody typed. It starts at a floor the
//! scheme can certainly take, and climbs only after the far side has agreed
//! to one more connection — which is what keeps a real transfer from being
//! the thing that discovers the limit.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use tungstate_backend::Backend;

/// Wall clock at one settled width before its throughput is believed.
///
/// Long enough to average over a slow chunk or a stalled round trip, short
/// enough that a run of a few dozen files still gets to try more than one
/// arrangement.
const SAMPLE: Duration = Duration::from_secs(6);

/// How much of the wider arrangement's throughput the narrower one has to
/// keep to be preferred.
///
/// Not 1.0: equal throughput on fewer connections is strictly better — it is
/// the same speed while asking less of the far side — so a narrower run only
/// has to be *nearly* as fast, not faster. Five per cent is inside the noise
/// of a network mount anyway.
const KEEP_RATIO: f64 = 0.95;

/// What a scheme can be asked for before anyone has measured anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Where a run begins. Certainly safe.
    pub floor: usize,
    /// Where it will stop climbing however willing the far side seems.
    pub ceiling: usize,
}

impl Limits {
    /// A directory this machine can already reach.
    ///
    /// No connection limit exists to find, so there is nothing to ramp
    /// towards: the constraint is the disk and the bus. Four helps a network
    /// mount, which is round-trip bound, without setting a spinning disk
    /// seeking against itself.
    #[must_use]
    pub fn local() -> Self {
        Self {
            floor: 4,
            ceiling: 4,
        }
    }

    /// A protocol with a per-client connection limit.
    ///
    /// Two, because an active FTP transfer holds a control connection *and* a
    /// data connection, and consumer NAS servers commonly cap a single client
    /// at somewhere between two and eight sockets. Four files would already be
    /// eight.
    #[must_use]
    pub fn networked() -> Self {
        Self {
            floor: 2,
            ceiling: 4,
        }
    }

    /// What the user asked for, clamped to something the far side can survive.
    ///
    /// An override raises the ceiling but never removes the handshake gate or
    /// the back-off: the point is to protect the far side, not to win an
    /// argument with its owner.
    #[must_use]
    pub fn overridden(self, requested: usize) -> Self {
        let requested = requested.max(1);
        Self {
            floor: self.floor.min(requested),
            ceiling: requested,
        }
    }
}

/// What the search for a good width is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trend {
    /// Timing the current width. Nothing has been compared yet.
    Measuring,
    /// The last narrowing was not worth it. Stay here and stop fiddling.
    Settled,
}

/// One width's worth of evidence.
#[derive(Debug)]
struct Sample {
    /// When this width started being timed.
    since: Instant,
    /// `moved` when it started.
    from: u64,
    /// Bytes per second at the last width tried, if one has been.
    previous: Option<f64>,
    trend: Trend,
}

/// Decides, and keeps deciding, how many transfers may be in flight.
#[derive(Debug)]
pub struct Governor {
    limits: Limits,
    /// Current permitted concurrency.
    limit: AtomicUsize,
    /// Set once the far side has objected. Climbing stops for the rest of the
    /// run: a server that has already said no is not worth asking twice.
    rebuffed: AtomicBool,
    /// What the handshake actually settled on. The real ceiling for the rest
    /// of the run — `limits.ceiling` is only what we were willing to ask for,
    /// and a rebuffed run must never climb back past what it was granted.
    agreed: AtomicUsize,
    sample: Mutex<Sample>,
    /// How long one width is timed for. A field rather than the constant
    /// directly so a test can ask its questions in milliseconds instead of
    /// sleeping through six seconds per decision.
    window: Duration,
    /// Woken when the limit rises, so a parked worker can pick up a file
    /// rather than a retired one having to be respawned.
    room: Condvar,
    /// Held only to wait on `room`. The state it guards is the limit, which
    /// lives in an atomic because it is read far more often than it changes.
    parked: Mutex<()>,
}

impl Governor {
    /// Start at the floor.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            limit: AtomicUsize::new(limits.floor),
            rebuffed: AtomicBool::new(false),
            agreed: AtomicUsize::new(limits.floor),
            sample: Mutex::new(Sample {
                since: Instant::now(),
                from: 0,
                previous: None,
                trend: Trend::Measuring,
            }),
            room: Condvar::new(),
            parked: Mutex::new(()),
            window: SAMPLE,
            limits,
        }
    }

    /// Freeze what the handshake reached as the working ceiling.
    ///
    /// Called once, after the ramp and before the first file. Everything
    /// after this may narrow and may come back, but never past here.
    pub fn agree(&self) {
        self.agreed.store(self.limit(), Ordering::Relaxed);
    }

    /// The most workers this run will ever have in flight.
    ///
    /// Threads are spawned to this number and park when they are above the
    /// limit, because a thread that returns cannot be recalled and a measured
    /// limit has to be able to be wrong in both directions.
    #[must_use]
    pub fn agreed(&self) -> usize {
        self.agreed.load(Ordering::Relaxed)
    }

    /// Park until there is room for this worker, or until `done` says the run
    /// is over.
    ///
    /// Returns whether there is room. A timeout on the wait rather than a
    /// pure signal, because the thing that ends a run is the queue emptying,
    /// and nobody notifies about that.
    pub fn wait_for_room(&self, index: usize, done: &dyn Fn() -> bool) -> bool {
        let mut guard = self
            .parked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while index >= self.limit() {
            if done() {
                return false;
            }
            let (next, _) = self
                .room
                .wait_timeout(guard, Duration::from_millis(200))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = next;
        }
        true
    }

    /// Reconsider the width, given how many bytes the run has written so far.
    ///
    /// The counter lives on the run rather than here because the thing that
    /// knows a byte landed is the copy loop; the governor only needs the
    /// running total to divide by a window.
    ///
    /// Called between files, which is the only point a worker can act on the
    /// answer: one three gigabytes into a copy cannot stand down, and
    /// interrupting it to satisfy a measurement would throw the work away.
    #[must_use]
    pub fn reconsider(&self, moved: u64) -> Option<usize> {
        let mut sample = self
            .sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sample.trend == Trend::Settled {
            return None;
        }

        let elapsed = sample.since.elapsed();
        if elapsed < self.window {
            return None;
        }

        // Precision is irrelevant here: this feeds a ratio compared against
        // 0.95, and a run would have to move sixteen exabytes for the lost
        // mantissa bits to change the answer.
        #[allow(clippy::cast_precision_loss)]
        let rate = (moved - sample.from) as f64 / elapsed.as_secs_f64();
        let width = self.limit();

        match sample.previous {
            // Nothing to compare against yet. Try one fewer and find out.
            None if width > 1 => {
                self.set(width - 1);
                tracing::info!(from = width, to = width - 1, rate, "trying a narrower run");
            }
            None => sample.trend = Trend::Settled,

            // The narrowing held up. Equal speed on fewer connections is the
            // better arrangement, so keep going down while that stays true.
            Some(before) if rate >= before * KEEP_RATIO => {
                if width > 1 {
                    self.set(width - 1);
                    tracing::info!(
                        from = width,
                        to = width - 1,
                        rate,
                        before,
                        "narrower is no slower"
                    );
                } else {
                    sample.trend = Trend::Settled;
                }
            }

            // Clearly worse. Put the worker back and stop adjusting: the
            // alternative is oscillating around the same boundary for the
            // rest of the run.
            Some(before) => {
                self.set((width + 1).min(self.agreed()));
                sample.trend = Trend::Settled;
                tracing::info!(
                    back_to = self.limit(),
                    rate,
                    before,
                    "narrower was slower; settling"
                );
            }
        }

        sample.previous = Some(rate);
        sample.since = Instant::now();
        sample.from = moved;

        let now = self.limit();
        (now != width).then_some(now)
    }

    /// Move the limit and wake anyone parked above the old one.
    fn set(&self, width: usize) {
        self.limit.store(width, Ordering::Relaxed);
        self.room.notify_all();
    }

    /// How many may be in flight right now.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit.load(Ordering::Relaxed)
    }

    /// Whether the far side has objected at any point in this run.
    #[must_use]
    pub fn rebuffed(&self) -> bool {
        self.rebuffed.load(Ordering::Relaxed)
    }

    /// Consider using one more, asking the far side first.
    ///
    /// Returns the limit afterwards. The handshake is the whole point: a slot
    /// is proven before a file is trusted to it, so discovering the ceiling
    /// costs a refused connection rather than a half-sent file.
    pub fn try_promote(&self, destination: &dyn Backend) -> usize {
        let current = self.limit();
        if self.rebuffed() || current >= self.limits.ceiling {
            return current;
        }

        if accepts_another(destination) {
            let raised = self.limit.fetch_add(1, Ordering::Relaxed) + 1;
            self.room.notify_all();
            tracing::debug!(concurrency = raised, "the far side accepted one more");
            raised
        } else {
            // Not a failure: the answer to "may I have another?" was no, and
            // asking cost a handshake rather than a transfer.
            self.rebuffed.store(true, Ordering::Relaxed);
            tracing::info!(
                concurrency = current,
                "staying here; the far side declined another"
            );
            current
        }
    }

    /// The far side objected during real work. Halve, and stop climbing.
    ///
    /// A handshake measures a connection at rest and a transfer is heavier, so
    /// a limit that probed fine can still turn out to be too much. This is the
    /// safety net under the gate, not a substitute for it.
    pub fn rebuff(&self) {
        self.rebuffed.store(true, Ordering::Relaxed);
        let reduced = (self.limit() / 2).max(1);
        self.limit.store(reduced, Ordering::Relaxed);
        // Whatever the measurement was about to conclude, the far side has
        // just given a better answer than any timing could.
        if let Ok(mut sample) = self.sample.lock() {
            sample.trend = Trend::Settled;
        }
        tracing::warn!(concurrency = reduced, "the far side objected; using fewer");
    }
}

/// Ask the far side for one more connection, cheaply.
///
/// A `stat` of a name that will not exist. `NotFound` is a *success* here: the
/// question is whether the connection was allowed, not whether the file was
/// there. Nothing is written, so a refusal costs a line in the server's log
/// and nothing else.
fn accepts_another(destination: &dyn Backend) -> bool {
    let name = format!(".tungstate-slot-{}", std::process::id());
    match destination.stat(Path::new(&name)) {
        // The file is absent, as expected — but the connection happened.
        Err(tungstate_backend::BackendError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            true
        }
        // Anything that is not "no such file" means the far side would not
        // talk to us, which is the answer we were looking for.
        Err(_) => false,
        // A file of that name really exists. Odd, but the connection worked.
        Ok(_) => true,
    }
}

/// Whether an error is the far side saying "not so many at once".
///
/// FTP says `421`, fixed by RFC 959. Everything else is matched on the shape
/// rather than the text: a refused or reset connection under load means the
/// same thing. Contained here because it is string-matching, and its failure
/// mode is benign — an unrecognised objection simply does not reduce the
/// limit, which is where we were before this existed.
#[must_use]
pub fn is_overload(error: &dyn std::error::Error) -> bool {
    const FTP_TOO_MANY: &str = "421";

    let mut rendered = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }
    let rendered = rendered.to_lowercase();

    rendered.contains(FTP_TOO_MANY)
        || rendered.contains("too many")
        || rendered.contains("connection refused")
        || rendered.contains("connection reset")
}

#[cfg(test)]
impl Governor {
    /// Same governor, timing each width over `window` instead of [`SAMPLE`].
    pub(crate) fn sampling_every(limits: Limits, window: Duration) -> Self {
        Self {
            window,
            ..Self::new(limits)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tungstate_backend::{Capabilities, Entry, Meta, RootToken, WriteFinish};

    /// A backend that accepts only so many slot enquiries, then refuses.
    ///
    /// Stands in for a server with `max_per_ip` set: the first few handshakes
    /// succeed and the next is turned away.
    struct Doorman {
        allowed: usize,
        asked: AtomicUsize,
    }

    impl Doorman {
        fn new(allowed: usize) -> Self {
            Self {
                allowed,
                asked: AtomicUsize::new(0),
            }
        }
    }

    fn refused() -> tungstate_backend::BackendError {
        tungstate_backend::BackendError::Remote {
            endpoint: "doorman".to_string(),
            operation: "stat",
            source: "421 There are too many connections from your internet address".into(),
        }
    }

    impl Backend for Doorman {
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                atomic_rename: true,
                hard_links: false,
                case_sensitive: true,
            }
        }
        fn root_token(&self) -> tungstate_backend::Result<RootToken> {
            Ok(RootToken { device: None })
        }
        fn stat(&self, path: &Path) -> tungstate_backend::Result<Meta> {
            if self.asked.fetch_add(1, Ordering::Relaxed) < self.allowed {
                // Accepted the connection; the file is simply not there.
                return Err(tungstate_backend::BackendError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
                });
            }
            Err(refused())
        }
        fn read_dir(&self, _p: &Path) -> tungstate_backend::Result<Vec<Entry>> {
            Ok(Vec::new())
        }
        fn open_read(&self, _p: &Path) -> tungstate_backend::Result<Box<dyn std::io::Read + Send>> {
            Err(refused())
        }
        fn create_write(&self, _p: &Path) -> tungstate_backend::Result<Box<dyn WriteFinish>> {
            Err(refused())
        }
        fn rename(&self, _f: &Path, _t: &Path) -> tungstate_backend::Result<()> {
            Ok(())
        }
        fn remove_file(&self, _p: &Path) -> tungstate_backend::Result<()> {
            Ok(())
        }
        fn remove_dir(&self, _p: &Path) -> tungstate_backend::Result<()> {
            Ok(())
        }
        fn create_dir_all(&self, _p: &Path) -> tungstate_backend::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_run_starts_at_the_floor() {
        let governor = Governor::new(Limits::networked());
        assert_eq!(governor.limit(), 2);
        assert!(!governor.rebuffed());
    }

    #[test]
    fn a_willing_server_is_climbed_towards_but_never_past_the_ceiling() {
        let governor = Governor::new(Limits::networked());
        let door = Doorman::new(usize::MAX);

        assert_eq!(governor.try_promote(&door), 3);
        assert_eq!(governor.try_promote(&door), 4);
        assert_eq!(
            governor.try_promote(&door),
            4,
            "the ceiling holds however willing the far side is"
        );
    }

    #[test]
    fn a_refusal_costs_a_handshake_and_stops_the_climb() {
        // The whole point: the limit is discovered by being told no to a
        // connection, not by a transfer dying halfway.
        let governor = Governor::new(Limits::networked());
        let door = Doorman::new(1);

        assert_eq!(governor.try_promote(&door), 3, "the first ask is allowed");
        assert_eq!(governor.try_promote(&door), 3, "the second is declined");
        assert!(governor.rebuffed());

        // And it does not keep asking a server that has already said no.
        let before = door.asked.load(Ordering::Relaxed);
        governor.try_promote(&door);
        assert_eq!(door.asked.load(Ordering::Relaxed), before);
    }

    #[test]
    fn an_objection_during_real_work_halves_and_never_reaches_zero() {
        // An override raises the ceiling; a run still starts at the floor and
        // climbs, so reaching 8 would take five accepted handshakes.
        let governor = Governor::new(Limits::local().overridden(8));
        assert_eq!(governor.limit(), 4, "the floor, not the override");

        governor.rebuff();
        assert_eq!(governor.limit(), 2);
        governor.rebuff();
        assert_eq!(governor.limit(), 1);
        governor.rebuff();
        assert_eq!(governor.limit(), 1, "one is the floor of the floor");
    }

    #[test]
    fn a_local_backend_is_not_ramped_because_there_is_nothing_to_ask() {
        let limits = Limits::local();
        assert_eq!(limits.floor, limits.ceiling);

        let governor = Governor::new(limits);
        let door = Doorman::new(usize::MAX);
        assert_eq!(
            governor.try_promote(&door),
            4,
            "already at the ceiling, so no handshake is spent"
        );
        assert_eq!(door.asked.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_override_raises_the_ceiling_but_keeps_the_gate() {
        let governor = Governor::new(Limits::networked().overridden(6));
        let door = Doorman::new(2);

        assert_eq!(governor.limit(), 2, "still starts low and climbs");
        assert_eq!(governor.try_promote(&door), 3);
        assert_eq!(governor.try_promote(&door), 4);
        assert_eq!(
            governor.try_promote(&door),
            4,
            "asking for 6 does not make the server give 6"
        );
    }

    #[test]
    fn an_overload_is_recognised_by_what_the_server_actually_says() {
        let too_many = refused();
        assert!(is_overload(&too_many));

        for text in [
            "Connection refused (os error 111)",
            "connection reset by peer",
        ] {
            let error = tungstate_backend::BackendError::Remote {
                endpoint: "nas".to_string(),
                operation: "write",
                source: text.into(),
            };
            assert!(is_overload(&error), "`{text}` should read as overload");
        }

        // An ordinary failure must not make a run quietly slower.
        let missing = tungstate_backend::BackendError::Io {
            path: std::path::PathBuf::from("a.mp4"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        };
        assert!(!is_overload(&missing));
    }
}

#[cfg(test)]
mod narrowing {
    use super::*;

    /// Drive one decision: pretend `bytes` moved over one whole window.
    fn after(governor: &Governor, window: Duration, total: &mut u64, bytes: u64) -> Option<usize> {
        std::thread::sleep(window);
        *total += bytes;
        governor.reconsider(*total)
    }

    #[test]
    fn a_width_that_buys_nothing_is_given_up() {
        // The case the user hit: four in flight over a mount that serialises
        // them anyway. Four is no faster than three, so three is better --
        // same speed, fewer connections held open on the far side.
        let window = Duration::from_millis(40);
        let governor = Governor::sampling_every(Limits::local(), window);
        governor.agree();
        assert_eq!(governor.limit(), 4);

        // Doubling rather than holding steady: `sleep` overshoots by a few
        // per cent, so "the same bytes each window" is not the same rate each
        // window, and the comparison here is within five per cent by design.
        let mut total = 0;
        assert_eq!(after(&governor, window, &mut total, 1_000), Some(3));
        assert_eq!(after(&governor, window, &mut total, 2_000), Some(2));
        assert_eq!(after(&governor, window, &mut total, 4_000), Some(1));
    }

    #[test]
    fn it_stops_at_one_rather_than_below_it() {
        let window = Duration::from_millis(40);
        let governor = Governor::sampling_every(Limits::local(), window);
        governor.agree();

        let mut total = 0;
        let mut bytes = 1_000;
        for _ in 0..6 {
            after(&governor, window, &mut total, bytes);
            bytes *= 2;
        }
        assert_eq!(governor.limit(), 1, "a run always has at least one worker");
    }

    #[test]
    fn a_narrowing_that_costs_throughput_is_undone_and_then_left_alone() {
        let window = Duration::from_millis(40);
        let governor = Governor::sampling_every(Limits::local(), window);
        governor.agree();

        let mut total = 0;
        // First window at four, establishing the baseline.
        assert_eq!(after(&governor, window, &mut total, 4_000), Some(3));
        // Three turns out to be much slower, so four comes back.
        assert_eq!(after(&governor, window, &mut total, 1_000), Some(4));
        // And it stops fiddling, rather than oscillating for the rest of the run.
        assert_eq!(after(&governor, window, &mut total, 4_000), None);
        assert_eq!(governor.limit(), 4);
    }

    #[test]
    fn nothing_is_decided_before_a_full_window_has_passed() {
        let governor = Governor::sampling_every(Limits::local(), Duration::from_secs(30));
        governor.agree();
        assert_eq!(governor.reconsider(9_999), None);
        assert_eq!(governor.limit(), 4);
    }

    #[test]
    fn it_never_climbs_back_past_what_the_far_side_granted() {
        // A rebuffed run agreed to two. A measurement must not use the
        // optimistic ceiling to hand back a connection the server refused.
        let window = Duration::from_millis(40);
        let governor = Governor::sampling_every(Limits::networked(), window);
        governor.agree();
        assert_eq!(governor.agreed(), 2);

        let mut total = 0;
        assert_eq!(after(&governor, window, &mut total, 4_000), Some(1));
        assert_eq!(after(&governor, window, &mut total, 1_000), Some(2));
        assert_eq!(after(&governor, window, &mut total, 4_000), None);
        assert!(governor.limit() <= governor.agreed());
    }
}
