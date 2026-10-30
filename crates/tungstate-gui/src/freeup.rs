//! Free up space: send the phone's photos and videos to a connection, check
//! each one there, and keep the list of those that arrived checked. Only
//! those may be cleared from the phone afterwards.
//!
//! A photo library is not a folder the engine can walk, so this copies out,
//! then sends: each batch is exported into the app's cache, laid out as it
//! will land (`<year>/<Photos|Videos|Screenshots>/<name>`), and moved from
//! there by the ordinary transfer engine. The engine deletes each exported
//! copy once it is verified at the far end, which keeps the cache small.
//!
//! No journal table of its own. A rerun is safe by construction: a photo
//! already on the connection comes back from the engine "already present",
//! having been read on both sides, and counts as checked like any other.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_photos::{Asset, AssetKind, Filter, Photos, PhotosExt, Removed};
use tungstate_backend::Backend;
use tungstate_backend::local::LocalBackend;
use tungstate_journal::{
    ConflictAction, Endpoint, Journal, JournalError, Link, NewLink, Order, SourcePolicy,
    VerifyLevel,
};
use tungstate_transfer::{FileOutcome, FixedResolver, Original, Progress, Stop, Transfer};

use crate::{App, backend_for, cache_dir, describe, lock};

/// Most bytes copied out of the library before they are sent. The cache a
/// run needs is about this much, or one video, whichever is bigger.
const BATCH_BYTES: u64 = 500 * 1024 * 1024;

/// How often the running screen is told where the run is.
const TELL_EVERY: Duration = Duration::from_millis(150);

/// The photo library, as a run needs it. The plugin on a phone; a fake in
/// the tests, so the whole run is exercised on a desktop.
pub(crate) trait Library {
    /// What the filter covers, favourites included.
    fn list(&self, filter: &Filter) -> Result<Vec<Asset>, String>;
    /// Copy one photo or video, untouched, into a file at `to`.
    fn export(&self, asset: &Asset, to: &Path) -> Result<(), String>;
}

impl<R: Runtime> Library for Photos<R> {
    fn list(&self, filter: &Filter) -> Result<Vec<Asset>, String> {
        Photos::list(self, filter.clone()).map_err(|e| e.to_string())
    }

    fn export(&self, asset: &Asset, to: &Path) -> Result<(), String> {
        Photos::export(self, &asset.id, to).map_err(|e| e.to_string())
    }
}

/// What the person chose on the Free up screen.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Start {
    /// The connection's name.
    connection: String,
    /// Where under the connection, as `Phone`.
    folder: String,
    filter: Filter,
    #[serde(default)]
    years_left_out: Vec<i32>,
    /// Favourites stay on the phone unless this is set.
    #[serde(default)]
    favourites: bool,
    /// `readback` or `size`.
    verify: String,
}

/// The run in flight, and what the last one left to clear.
#[derive(Default)]
pub(crate) struct FreeUp {
    stop: Arc<Stop>,
    /// Set under the gate while a run is going, so a transfer pressed on the
    /// desktop side is refused rather than queued behind work it cannot see.
    running: AtomicBool,
    /// The photos the last run checked on the connection, by the library's id.
    /// Kept here rather than handed to the window, so nothing the window says
    /// can widen what gets cleared.
    checked: Mutex<Vec<String>>,
}

impl FreeUp {
    pub(crate) fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Where a run is, as the running screen shows it.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Tick {
    /// Photos and videos dealt with, whatever became of them.
    pub done: usize,
    /// Of those, the ones checked on the connection: safe to clear.
    pub checked: usize,
    pub count: usize,
    /// Bytes dealt with, plus what of the file in flight has gone.
    pub bytes: u64,
    pub total: u64,
    /// The photo or video in hand.
    pub current: String,
}

/// How a run ended.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Ran {
    /// Checked on the connection, so safe to clear.
    pub checked: usize,
    pub checked_bytes: u64,
    /// Chosen but not checked: still only on the phone, or not proven to be
    /// anywhere else. These stay.
    pub kept: usize,
    /// Pause or Stop was pressed.
    pub stopped: bool,
    /// Why the run ended early, when it was not asked to.
    pub problem: Option<String>,
    /// Why the first photo that stayed did, so a run where nothing could be
    /// sent says what is wrong rather than only that something is.
    pub reason: Option<String>,
    /// The checked photos, by id. Not sent to the window.
    #[serde(skip)]
    pub safe: Vec<String>,
}

/// What a filter found, less the years left out and, unless asked for, the
/// favourites. Oldest first, then by id, so a rerun meets them in the same
/// order and names them the same way.
pub(crate) fn chosen(
    mut assets: Vec<Asset>,
    years_left_out: &[i32],
    favourites: bool,
) -> Vec<Asset> {
    let left: HashSet<i32> = years_left_out.iter().copied().collect();
    assets.retain(|a| !left.contains(&a.year) && (favourites || !a.favourite));
    assets.sort_by(|a, b| a.taken_ms.cmp(&b.taken_ms).then_with(|| a.id.cmp(&b.id)));
    assets
}

/// Where each photo lands under the run's folder: `<year>/<kind>/<name>`.
///
/// Two photos can share a name in one folder (`IMG_0001.JPG` from two
/// cameras), and SMB folders ignore case, so the later one takes a short tag
/// from its id. A tag, not a counter, so the same photo gets the same name
/// on every run whatever else is chosen.
pub(crate) fn layout(assets: &[Asset]) -> Vec<PathBuf> {
    let mut taken = HashSet::new();
    assets
        .iter()
        .map(|asset| {
            let dir = Path::new(&asset.year.to_string()).join(folder_for(asset.kind));
            let name = clean(&asset.name, &asset.id);
            let mut path = dir.join(&name);
            if !taken.insert(path.to_string_lossy().to_lowercase()) {
                let (stem, ext) = match name.rsplit_once('.') {
                    Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
                    _ => (name.as_str(), String::new()),
                };
                let tag = tag(&asset.id);
                let mut n = 1;
                loop {
                    let suffix = if n == 1 {
                        tag.clone()
                    } else {
                        format!("{tag}-{n}")
                    };
                    path = dir.join(format!("{stem}-{suffix}{ext}"));
                    if taken.insert(path.to_string_lossy().to_lowercase()) {
                        break;
                    }
                    n += 1;
                }
            }
            path
        })
        .collect()
}

fn folder_for(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Photo => "Photos",
        AssetKind::Video => "Videos",
        AssetKind::Screenshot => "Screenshots",
    }
}

/// A name the library gave, made safe as one path component on any
/// connection: no separators, nothing Windows or SMB refuse, no `..`.
fn clean(name: &str, id: &str) -> String {
    let swapped: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = swapped.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        format!("item-{}", tag(id))
    } else {
        trimmed.to_string()
    }
}

/// Eight hex digits from an id, the same on every run and every machine.
/// FNV-1a: std's hasher is free to change between Rust releases.
fn tag(id: &str) -> String {
    let hash = id.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    format!("{hash:08x}")
}

/// Consecutive runs of `sizes` adding up to at most `limit`, except that a
/// single item bigger than the limit is a batch of its own.
fn batches(sizes: &[u64], limit: u64) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut held = 0_u64;
    for (i, &size) in sizes.iter().enumerate() {
        if i > start && held + size > limit {
            out.push(start..i);
            start = i;
            held = 0;
        }
        held += size;
    }
    if start < sizes.len() {
        out.push(start..sizes.len());
    }
    out
}

/// Whether an outcome proves the photo is on the connection.
///
/// Sent and checked, or found already there having read both copies. In both
/// the engine has already removed the exported copy, which is the cue it was
/// satisfied. Anything else, including a copy it was told to keep, is not
/// proof, and the photo stays.
fn safe(outcome: FileOutcome) -> bool {
    matches!(
        outcome,
        FileOutcome::Transferred | FileOutcome::AlreadyPresent(Original::Removed)
    )
}

/// Everything one run needs besides the library.
pub(crate) struct Run<'a> {
    pub journal: &'a Journal,
    pub link: &'a Link,
    pub destination: &'a dyn Backend,
    /// The cache folder exports go to. Emptied by the caller.
    pub work: &'a Path,
    pub stop: &'a Arc<Stop>,
}

/// Send `chosen`, batch by batch, and say which arrived checked.
///
/// Never errors: a run that cannot go on says why in [`Ran::problem`] and
/// keeps every photo it did not prove safe.
pub(crate) fn carry(
    run: &Run<'_>,
    library: &dyn Library,
    chosen: &[Asset],
    tell: &mut (dyn FnMut(&Tick) + Send),
) -> Ran {
    let paths = layout(chosen);
    let sizes: Vec<u64> = chosen.iter().map(|a| a.size).collect();
    let source = LocalBackend::new(run.work.to_path_buf());
    let mut ran = Ran::default();
    let mut tick = Tick {
        count: chosen.len(),
        total: sizes.iter().sum(),
        ..Tick::default()
    };

    'batches: for range in batches(&sizes, BATCH_BYTES) {
        let mut sending = Vec::new();
        let mut owner = HashMap::new();
        for i in range {
            if run.stop.asked() {
                ran.stopped = true;
                break 'batches;
            }
            let asset = &chosen[i];
            tick.current.clone_from(&asset.name);
            tell(&tick);
            let to = run.work.join(&paths[i]);
            let exported = to
                .parent()
                .map_or(Ok(()), |dir| {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())
                })
                .and_then(|()| library.export(asset, &to));
            match exported {
                Ok(()) => {
                    owner.insert(paths[i].clone(), i);
                    sending.push(paths[i].clone());
                }
                Err(why) => {
                    tracing::warn!(name = %asset.name, %why, "could not copy a photo out of the library");
                    ran.reason.get_or_insert(why);
                    tick.done += 1;
                    tick.bytes += asset.size;
                }
            }
        }
        if sending.is_empty() {
            continue;
        }

        let mut recorder = Recorder {
            sizes: owner
                .iter()
                .map(|(path, &i)| (path.clone(), chosen[i].size))
                .collect(),
            outcomes: BTreeMap::new(),
            tick: &mut tick,
            tell: &mut *tell,
        };
        let sent = Transfer::new(
            run.link,
            &source,
            run.destination,
            run.journal,
            &mut FixedResolver(ConflictAction::Rename),
            &mut recorder,
        )
        .cancellable(Arc::clone(run.stop))
        .run_selection(&sending);

        for (path, outcome) in &recorder.outcomes {
            if safe(*outcome) {
                let asset = &chosen[owner[path]];
                ran.safe.push(asset.id.clone());
                ran.checked_bytes += asset.size;
            }
        }
        // Whatever the engine did not take is a copy of something still on
        // the phone; it only costs the phone space.
        for path in &sending {
            let _ = std::fs::remove_file(run.work.join(path));
        }

        if let Ok(summary) = &sent
            && let Some(first) = summary.failures.first()
        {
            ran.reason.get_or_insert_with(|| first.reason.clone());
        }
        match sent {
            Ok(summary) if summary.destination_lost => {
                ran.problem = Some("the connection went away part-way, so the run stopped".into());
                break;
            }
            Ok(summary) if summary.cancelled => {
                ran.stopped = true;
                break;
            }
            Ok(_) => {}
            Err(error) => {
                ran.problem = Some(describe(error));
                break;
            }
        }
    }

    ran.checked = ran.safe.len();
    ran.kept = chosen.len() - ran.checked;
    ran
}

/// Hears the engine, file by file: which arrived checked, and how far along
/// the run is.
struct Recorder<'a> {
    sizes: HashMap<PathBuf, u64>,
    outcomes: BTreeMap<PathBuf, FileOutcome>,
    tick: &'a mut Tick,
    tell: &'a mut (dyn FnMut(&Tick) + Send),
}

impl Recorder<'_> {
    fn name(path: &Path) -> String {
        path.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }
}

impl Progress for Recorder<'_> {
    fn starting(&mut self, path: &Path, _size: u64) {
        self.tick.current = Self::name(path);
        (self.tell)(self.tick);
    }

    fn advanced(&mut self, path: &Path, done: u64, _total: u64) {
        let mut now = self.tick.clone();
        now.bytes += done;
        now.current = Self::name(path);
        (self.tell)(&now);
    }

    fn finished(&mut self, path: &Path, outcome: FileOutcome) {
        self.outcomes.insert(path.to_path_buf(), outcome);
        self.tick.done += 1;
        self.tick.bytes += self.sizes.get(path).copied().unwrap_or(0);
        if safe(outcome) {
            self.tick.checked += 1;
        }
        (self.tell)(self.tick);
    }
}

/// The link a run goes through: one per connection, folder and check level.
///
/// Reused rather than made fresh, because the engine tidies up after an
/// interrupted run by link: a phone that died mid-file leaves a partial on
/// the connection, and only the same link's next run knows to remove it.
fn link_for(
    journal: &Journal,
    connection: &str,
    destination: &Endpoint,
    work: &Path,
    verify: VerifyLevel,
) -> Result<Link, String> {
    let name = format!(
        "free-up {connection}:{} {}",
        destination.path.display(),
        verify.as_str()
    );
    match journal.link_by_name(&name) {
        Ok(link) => return Ok(link),
        Err(JournalError::UnknownLink(_)) => {}
        Err(other) => return Err(describe(other)),
    }
    journal
        .create_link(&NewLink {
            name: name.clone(),
            source: Endpoint::local(work),
            destination: destination.clone(),
            source_policy: SourcePolicy::Delete,
            verify,
            order: Order::Discovered,
            // Two different photos with one name: keep both.
            on_conflict: ConflictAction::Rename,
            // Exported a moment ago, so every one would look too fresh.
            cooldown: Duration::ZERO,
            saved: false,
        })
        .map_err(describe)?;
    journal.link_by_name(&name).map_err(describe)
}

/// A folder under a connection, as typed: relative, no `..`, no empty parts.
fn folder(raw: &str) -> Result<PathBuf, String> {
    let parts: Vec<&str> = raw
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("choose a folder to send to".into());
    }
    if parts.iter().any(|p| *p == "." || *p == "..") {
        return Err("that folder name is not one a connection can use".into());
    }
    Ok(parts.iter().collect())
}

/// Start sending. Refused while a transfer or a sync runs; the run itself
/// goes on its own thread and reports through `freeup://progress` and
/// `freeup://done`.
#[tauri::command]
pub(crate) fn free_up_start(start: Start, app: AppHandle) -> Result<(), String> {
    let state = app.state::<App>();
    let verify = match start.verify.as_str() {
        "readback" => VerifyLevel::Readback,
        "size" => VerifyLevel::Size,
        _ => return Err("unknown check level".into()),
    };
    let under = folder(&start.folder)?;
    let connection = state
        .journal
        .connection_by_name(&start.connection)
        .map_err(describe)?;
    let work = cache_dir()
        .ok_or("this phone gave Tungstate nowhere to copy photos out to")?
        .join("freeup");

    {
        // The gate every run takes: nothing else may be writing.
        let _gate = lock(&state.gate);
        if state.syncing.queue.is_running() {
            return Err("a sync is running; free up space when it has finished".into());
        }
        // Holding the transfer queue is what makes syncs and the updater
        // see this run as the transfer it is.
        if !state.queue.claim() {
            return Err("a transfer is running; free up space when it has finished".into());
        }
        state.freeup.running.store(true, Ordering::SeqCst);
    }
    state.freeup.stop.clear();
    lock(&state.freeup.checked).clear();

    let worker = app.clone();
    std::thread::spawn(move || {
        let state = worker.state::<App>();
        let guard = Guard(worker.clone());
        let _ = worker.photos().keep_awake(true);
        let ran = go(
            &worker,
            &state,
            &start,
            connection.id,
            &under,
            &work,
            verify,
        )
        .unwrap_or_else(|problem| Ran {
            problem: Some(problem),
            ..Ran::default()
        });
        let _ = std::fs::remove_dir_all(&work);
        lock(&state.freeup.checked).clone_from(&ran.safe);
        // Released before saying so: the window clears as soon as it hears,
        // and clearing is refused while a run still holds the queue.
        drop(guard);
        let _ = worker.emit("freeup://done", &ran);
    });
    Ok(())
}

/// The run, from listing the library to the last batch.
fn go(
    app: &AppHandle,
    state: &App,
    start: &Start,
    connection: tungstate_journal::ConnectionId,
    under: &Path,
    work: &Path,
    verify: VerifyLevel,
) -> Result<Ran, String> {
    let photos = app.photos();
    let chosen = chosen(
        Library::list(photos, &start.filter)?,
        &start.years_left_out,
        start.favourites,
    );

    // What a killed run left in the cache is a copy of photos still on the
    // phone; start from nothing.
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;

    // The engine wants its destination to exist before it starts.
    backend_for(&Endpoint::remote(connection, ""), &state.journal)?
        .create_dir_all(under)
        .map_err(describe)?;
    let endpoint = Endpoint::remote(connection, under);
    let destination = backend_for(&endpoint, &state.journal)?;
    let link = link_for(&state.journal, &start.connection, &endpoint, work, verify)?;

    let mut last = Instant::now()
        .checked_sub(TELL_EVERY)
        .unwrap_or_else(Instant::now);
    let emitter = app.clone();
    let mut tell = move |tick: &Tick| {
        if last.elapsed() >= TELL_EVERY || tick.done == tick.count {
            last = Instant::now();
            let _ = emitter.emit("freeup://progress", tick);
        }
    };
    let run = Run {
        journal: &state.journal,
        link: &link,
        destination: destination.as_ref(),
        work,
        stop: &state.freeup.stop,
    };
    Ok(carry(&run, photos, &chosen, &mut tell))
}

/// Ends the run however its thread ends, a panic included: the screen may
/// sleep again, and transfers and syncs may start.
struct Guard(AppHandle);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.photos().keep_awake(false);
        let state = self.0.state::<App>();
        let _gate = lock(&state.gate);
        state.freeup.running.store(false, Ordering::SeqCst);
        state.queue.stand_down();
    }
}

/// Pause and Stop both land here: finish the photo in hand, then stop. What
/// was sent stays sent; pressing Free again picks up the rest.
#[tauri::command]
pub(crate) fn free_up_stop(state: tauri::State<'_, App>) {
    state.freeup.stop.after_this_file();
}

/// Clear what the last run checked from the phone, through the system's own
/// prompt, and say how many went. Only ever those ids: the window asks for
/// "what was checked", never names photos itself.
///
/// Async so the wait for the person's answer is off the main thread, which
/// the prompt itself needs. The window calls it only while it is on screen,
/// so nothing is removed while the app is in the background.
#[tauri::command(async)]
pub(crate) fn free_up_clear(app: AppHandle) -> Result<Removed, String> {
    let state = app.state::<App>();
    if state.freeup.running() {
        return Err("still sending; clear them once it has finished".into());
    }
    let ids = lock(&state.freeup.checked).clone();
    let removed = app.photos().remove(&ids).map_err(|e| e.to_string())?;
    // Declined leaves them listed, so the person can be asked again.
    if removed.removed > 0 {
        lock(&state.freeup.checked).clear();
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use tungstate_backend::WriteFinish;

    const YEAR_MS: i64 = 365 * 86_400_000;

    fn asset(id: &str, name: &str, year: i32, kind: AssetKind, size: u64) -> Asset {
        Asset {
            id: id.into(),
            name: name.into(),
            size,
            taken_ms: i64::from(year - 1970) * YEAR_MS,
            year,
            kind,
            favourite: false,
        }
    }

    /// Bytes that differ per photo, so an identical file at the far end is
    /// really the same photo.
    fn bytes_of(asset: &Asset) -> Vec<u8> {
        let size = usize::try_from(asset.size).unwrap();
        asset.id.bytes().cycle().take(size).collect()
    }

    #[derive(Default)]
    struct FakeLibrary {
        assets: Vec<Asset>,
        /// Ids whose export fails, as an unreadable photo would.
        unreadable: HashSet<String>,
        exported: Mutex<Vec<String>>,
    }

    impl Library for FakeLibrary {
        fn list(&self, _filter: &Filter) -> Result<Vec<Asset>, String> {
            Ok(self.assets.clone())
        }
        fn export(&self, asset: &Asset, to: &Path) -> Result<(), String> {
            if self.unreadable.contains(&asset.id) {
                return Err("could not read it".into());
            }
            lock(&self.exported).push(asset.id.clone());
            std::fs::write(to, bytes_of(asset)).map_err(|e| e.to_string())
        }
    }

    struct Rig {
        journal: Journal,
        work: tempfile::TempDir,
        dest: tempfile::TempDir,
        verify: VerifyLevel,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                journal: Journal::open_in_memory().unwrap(),
                work: tempfile::tempdir().unwrap(),
                dest: tempfile::tempdir().unwrap(),
                verify: VerifyLevel::Readback,
            }
        }

        fn run(&self, library: &FakeLibrary, stop: &Arc<Stop>) -> Ran {
            self.run_over(
                library,
                stop,
                &LocalBackend::new(self.dest.path().to_path_buf()),
            )
        }

        fn run_over(
            &self,
            library: &FakeLibrary,
            stop: &Arc<Stop>,
            destination: &dyn Backend,
        ) -> Ran {
            let link = link_for(
                &self.journal,
                "nas",
                &Endpoint::local(self.dest.path()),
                self.work.path(),
                self.verify,
            )
            .unwrap();
            let run = Run {
                journal: &self.journal,
                link: &link,
                destination,
                work: self.work.path(),
                stop,
            };
            let picked = chosen(library.list(&Filter::default()).unwrap(), &[], false);
            carry(&run, library, &picked, &mut |_| {})
        }

        fn landed(&self, relative: &str) -> Option<Vec<u8>> {
            std::fs::read(self.dest.path().join(relative)).ok()
        }
    }

    fn three() -> FakeLibrary {
        FakeLibrary {
            assets: vec![
                asset("1", "IMG_1.jpg", 2024, AssetKind::Photo, 3000),
                asset("2", "VID_2.mp4", 2025, AssetKind::Video, 9000),
                asset("3", "Screenshot_3.png", 2025, AssetKind::Screenshot, 500),
            ],
            ..FakeLibrary::default()
        }
    }

    #[test]
    fn each_photo_lands_under_its_year_and_kind() {
        let rig = Rig::new();
        let library = three();
        let ran = rig.run(&library, &Arc::new(Stop::new()));

        assert_eq!(ran.checked, 3);
        assert_eq!(
            rig.landed("2024/Photos/IMG_1.jpg"),
            Some(bytes_of(&library.assets[0]))
        );
        assert_eq!(
            rig.landed("2025/Videos/VID_2.mp4"),
            Some(bytes_of(&library.assets[1]))
        );
        assert_eq!(
            rig.landed("2025/Screenshots/Screenshot_3.png"),
            Some(bytes_of(&library.assets[2]))
        );
    }

    #[test]
    fn a_photo_that_could_not_be_copied_out_stays() {
        let rig = Rig::new();
        let mut library = three();
        library.unreadable.insert("2".into());
        let ran = rig.run(&library, &Arc::new(Stop::new()));

        assert_eq!(ran.safe, vec!["1".to_string(), "3".to_string()]);
        assert_eq!(ran.kept, 1);
        assert_eq!(ran.reason.as_deref(), Some("could not read it"));
        assert_eq!(rig.landed("2025/Videos/VID_2.mp4"), None);
    }

    /// Writes rubbish for anything named `bad`, as a failing disk would.
    struct Corrupting(LocalBackend);

    impl Backend for Corrupting {
        fn capabilities(&self) -> tungstate_backend::Capabilities {
            self.0.capabilities()
        }
        fn root_token(&self) -> tungstate_backend::Result<tungstate_backend::RootToken> {
            self.0.root_token()
        }
        fn stat(&self, path: &Path) -> tungstate_backend::Result<tungstate_backend::Meta> {
            self.0.stat(path)
        }
        fn read_dir(
            &self,
            path: &Path,
        ) -> tungstate_backend::Result<Vec<tungstate_backend::Entry>> {
            self.0.read_dir(path)
        }
        fn open_read(
            &self,
            path: &Path,
        ) -> tungstate_backend::Result<Box<dyn std::io::Read + Send>> {
            self.0.open_read(path)
        }
        fn create_write(&self, path: &Path) -> tungstate_backend::Result<Box<dyn WriteFinish>> {
            let inner = self.0.create_write(path)?;
            if path.to_string_lossy().contains("bad") {
                Ok(Box::new(Corrupt(inner)))
            } else {
                Ok(inner)
            }
        }
        fn rename(&self, from: &Path, to: &Path) -> tungstate_backend::Result<()> {
            self.0.rename(from, to)
        }
        fn remove_file(&self, path: &Path) -> tungstate_backend::Result<()> {
            self.0.remove_file(path)
        }
        fn remove_dir(&self, path: &Path) -> tungstate_backend::Result<()> {
            self.0.remove_dir(path)
        }
        fn create_dir_all(&self, path: &Path) -> tungstate_backend::Result<()> {
            self.0.create_dir_all(path)
        }
    }

    struct Corrupt(Box<dyn WriteFinish>);

    impl Write for Corrupt {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let flipped: Vec<u8> = buf.iter().map(|b| b ^ 0xFF).collect();
            self.0.write_all(&flipped)?;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }

    impl WriteFinish for Corrupt {
        fn finish(self: Box<Self>) -> tungstate_backend::Result<()> {
            self.0.finish()
        }
    }

    #[test]
    fn a_photo_whose_copy_failed_its_check_stays() {
        let rig = Rig::new();
        let library = FakeLibrary {
            assets: vec![
                asset("1", "good.jpg", 2025, AssetKind::Photo, 2000),
                asset("2", "bad.jpg", 2025, AssetKind::Photo, 2000),
            ],
            ..FakeLibrary::default()
        };
        let corrupting = Corrupting(LocalBackend::new(rig.dest.path().to_path_buf()));
        let ran = rig.run_over(&library, &Arc::new(Stop::new()), &corrupting);

        assert_eq!(
            ran.safe,
            vec!["1".to_string()],
            "only the photo read back intact is safe"
        );
        assert_eq!(ran.kept, 1);
        assert!(
            ran.reason.is_some_and(|why| why.contains("bad.jpg")),
            "says which and why"
        );
    }

    #[test]
    fn a_rerun_counts_what_already_arrived_as_checked_without_sending_it_twice() {
        let rig = Rig::new();
        let library = three();
        // The first run's answer is thrown away, as a killed app loses it.
        let _ = rig.run(&library, &Arc::new(Stop::new()));
        let again = rig.run(&library, &Arc::new(Stop::new()));

        assert_eq!(
            again.checked, 3,
            "already there, read on both sides, so safe"
        );
        let names: Vec<_> = std::fs::read_dir(rig.dest.path().join("2025/Videos"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "no second copy under a new name: {names:?}");
    }

    #[test]
    fn a_stopped_run_proves_nothing_it_did_not_send() {
        let rig = Rig::new();
        let library = three();
        let stop = Arc::new(Stop::new());
        stop.after_this_file();
        let ran = rig.run(&library, &stop);

        assert!(ran.stopped);
        assert_eq!(ran.safe, Vec::<String>::new());
        assert_eq!(ran.kept, 3);
        assert!(lock(&library.exported).is_empty());
    }

    #[test]
    fn the_exported_copies_do_not_outlive_the_run() {
        let rig = Rig::new();
        let mut library = three();
        library.unreadable.insert("3".into());
        let _ = rig.run(&library, &Arc::new(Stop::new()));

        let left: Vec<_> = walk(rig.work.path());
        assert!(left.is_empty(), "the cache still holds {left:?}");
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flat_map(|e| {
                let path = e.unwrap().path();
                if path.is_dir() {
                    walk(&path)
                } else {
                    vec![path]
                }
            })
            .collect()
    }

    #[test]
    fn years_left_out_and_favourites_are_not_chosen() {
        let mut loved = asset("4", "loved.jpg", 2025, AssetKind::Photo, 1);
        loved.favourite = true;
        let all = vec![
            asset("1", "a.jpg", 2024, AssetKind::Photo, 1),
            asset("2", "b.jpg", 2025, AssetKind::Photo, 1),
            asset("3", "c.jpg", 2026, AssetKind::Photo, 1),
            loved,
        ];
        let ids = |v: Vec<Asset>| v.into_iter().map(|a| a.id).collect::<Vec<_>>();

        assert_eq!(ids(chosen(all.clone(), &[2026], false)), ["1", "2"]);
        assert_eq!(ids(chosen(all, &[2026], true)), ["1", "2", "4"]);
    }

    #[test]
    fn names_that_clash_in_one_folder_are_told_apart_the_same_way_every_time() {
        let a = asset("10", "IMG_0001.JPG", 2025, AssetKind::Photo, 1);
        let b = asset("11", "img_0001.jpg", 2025, AssetKind::Photo, 1);
        let other_year = asset("12", "IMG_0001.JPG", 2024, AssetKind::Photo, 1);

        let paths = layout(&[a.clone(), b.clone(), other_year]);
        assert_eq!(paths[0], Path::new("2025/Photos/IMG_0001.JPG"));
        assert_eq!(
            paths[1],
            Path::new("2025/Photos").join(format!("img_0001-{}.jpg", tag("11")))
        );
        assert_eq!(
            paths[2],
            Path::new("2024/Photos/IMG_0001.JPG"),
            "another folder, no clash"
        );
        assert_eq!(layout(&[a, b])[1], paths[1]);
    }

    #[test]
    fn a_name_cannot_climb_out_of_its_folder() {
        assert_eq!(clean("../../etc/passwd", "1"), "_.._etc_passwd");
        assert_eq!(clean("a:b?.jpg", "1"), "a_b_.jpg");
        assert_eq!(clean("..", "7"), format!("item-{}", tag("7")));
        assert_eq!(folder("Phone/"), Ok(PathBuf::from("Phone")));
        assert!(folder("../Phone").is_err());
        assert!(folder(" / ").is_err());
    }

    #[test]
    fn batches_stay_under_the_limit_and_a_big_video_goes_alone() {
        assert_eq!(batches(&[4, 4, 4, 20, 1], 10), vec![0..2, 2..3, 3..4, 4..5]);
        assert_eq!(batches(&[], 10), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(batches(&[3, 3, 3], 10), vec![0..3]);
    }
}
