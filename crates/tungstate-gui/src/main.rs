//! Tungstate's desktop window.
//!
//! Everything the command line can do is reachable here, because the point of
//! the window is not having to know the command line.
//!
//! The engine is synchronous and the window is not, so a run happens on its own
//! thread and reports back through Tauri events. See `bridge` for the joins.

// Without this a release build on Windows opens a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Tauri's command macro deserialises arguments and hands them over owned, and
// `State` must be taken by value for the macro to wire it up. Clippy reads that
// as waste; it is the framework's calling convention and cannot be borrowed.
#![allow(clippy::needless_pass_by_value)]

mod bridge;
mod dupes;
mod watching;

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
// `ends` is the shared reading of a composite location string. It lives in the
// journal crate because that crate owns `Endpoint` and `Connection`, and
// because the command line was already using every line of it.
use tungstate_execute::thumbs::Thumbs;
use tungstate_journal::{
    ConflictAction, Connection, ConnectionSettings, Endpoint, Journal, JournalError, Link, Locator,
    NewConnection, NewLink, Op, OpStatus, Order, Scheme, SourcePolicy, VerifyLevel, ends,
};
use tungstate_secret::{EnvOverride, KeyringStore, SecretStore, connection_key};
use tungstate_transfer::{IdenticalAction, Stop, Summary, Transfer};

use bridge::{Answer, ConflictChannel, EventProgress, Reply, WindowResolver};

/// Shared state for the whole window.
mod govern;

struct App {
    journal: Journal,
    conflicts: ConflictChannel,
    cancel: Arc<Stop>,
    /// The duplicate scan in flight, so it can be stopped. Its own flag
    /// rather than the transfer's: stopping a scan must not stop a drain.
    scan: dupes::Scan,
    /// Work the running transfer has not reached yet, and whether a worker
    /// exists to reach it. Shared rather than moved into the worker, which is
    /// the whole of what lets a transfer be added to one already in flight.
    queue: RunQueue<Link>,
    /// The folder watcher, and what it has done since the window opened.
    watcher: Arc<watching::Watching>,
    /// Where small pictures of what a scan found are kept.
    ///
    /// `None` when the cache directory could not be made, which costs pictures
    /// and nothing else: the scan still runs and the rows still list.
    thumbs: Option<Thumbs>,
    /// A ceiling on how many files move at once, when one has been asked for.
    ///
    /// Raises the number the governor climbs towards; it does not skip the
    /// handshake, and it does not stop tungstate backing off if the far side
    /// objects. Zero means "no ceiling asked for", which is the normal case.
    at_once_ceiling: AtomicUsize,
}

/// Work waiting for a worker, and whether a worker is there to take it.
///
/// The two facts live together because they have to be decided together. A
/// worker finishes when it finds the queue empty, and a submission starts a
/// worker when it finds none running; if those two read each other outside a
/// lock, a transfer submitted at the wrong moment is accepted by nobody and
/// sits there for ever.
struct RunQueue<T> {
    waiting: Mutex<VecDeque<T>>,
    /// Only ever written while `waiting` is held. That is the invariant the
    /// whole type rests on, and why this is not a bare `AtomicBool` on `App`.
    running: AtomicBool,
}

impl<T> RunQueue<T> {
    fn new() -> Self {
        Self {
            waiting: Mutex::new(VecDeque::new()),
            running: AtomicBool::new(false),
        }
    }

    /// Add work. `true` when the caller has to start a worker, `false` when
    /// one is already running and will reach it.
    fn submit(&self, items: impl IntoIterator<Item = T>) -> bool {
        let mut waiting = lock(&self.waiting);
        waiting.extend(items);
        !self.running.swap(true, Ordering::SeqCst)
    }

    /// The next piece of work, or `None` having recorded that the worker is
    /// finished. Called only by the worker.
    fn next(&self) -> Option<T> {
        let mut waiting = lock(&self.waiting);
        let next = waiting.pop_front();
        if next.is_none() {
            self.running.store(false, Ordering::SeqCst);
        }
        next
    }

    /// Drop everything still waiting, without ending the worker.
    fn clear(&self) {
        lock(&self.waiting).clear();
    }

    /// Give up being the worker, leaving anything waiting for the next one.
    /// The panic path: a submission afterwards starts a fresh worker.
    fn stand_down(&self) {
        let _waiting = lock(&self.waiting);
        self.running.store(false, Ordering::SeqCst);
    }

    fn waiting(&self) -> usize {
        lock(&self.waiting).len()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Take a lock, ignoring poisoning.
///
/// A panic in a worker leaves the queue poisoned, and refusing every later
/// transfer because of it would turn one bad run into a dead window. What is
/// behind it is a list of links, which a panic cannot leave half-written.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A link as the window shows it.
#[derive(Debug, Serialize)]
struct LinkView {
    name: String,
    source: String,
    destination: String,
    source_policy: String,
    verify: String,
    order: String,
    on_conflict: String,
    cooldown_secs: u64,
}

impl LinkView {
    /// Needs the journal because a remote end is stored as an id, and the name
    /// is what the window has to show.
    fn of(link: &Link, journal: &Journal) -> Self {
        Self {
            name: link.name.clone(),
            source: ends::describe(&link.source, journal),
            destination: ends::describe(&link.destination, journal),
            source_policy: link.source_policy.as_str().to_string(),
            verify: link.verify.as_str().to_string(),
            order: link.order.as_str().to_string(),
            on_conflict: link.on_conflict.as_str().to_string(),
            cooldown_secs: link.cooldown.as_secs(),
        }
    }
}

/// Open the machine journal, honouring the test override.
///
/// `TUNGSTATE_JOURNAL` is the same seam the command line has had since slice
/// 4b, and it is here for the same reason: without it, exercising the window
/// means writing to the real database on the machine doing the exercising.
/// Walking through a drain by hand should not leave connections and links in
/// the journal a person actually keeps their history in.
fn open_journal() -> tungstate_journal::Result<Journal> {
    match std::env::var_os("TUNGSTATE_JOURNAL") {
        Some(path) => Journal::open(Path::new(&path)),
        None => Journal::open_default(),
    }
}

/// Where small pictures live between scans.
///
/// The operating system's own cache directory, so it is somewhere a person
/// already understands and somewhere the system may clear without asking.
/// `TUNGSTATE_JOURNAL` moves it too, for the same reason it moves the journal:
/// walking through the window by hand should leave nothing behind.
fn cache_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TUNGSTATE_JOURNAL") {
        return Path::new(&path).parent().map(Path::to_path_buf);
    }
    directories::ProjectDirs::from("dev", "tungstate", "tungstate")
        .map(|dirs| dirs.cache_dir().to_path_buf())
}

/// One small picture, by the name a scan gave it.
fn serve_picture(thumbs: Option<&Thumbs>, path: &str) -> tauri::http::Response<Vec<u8>> {
    let name = path.trim_start_matches('/');
    // One path component and nothing else. A name with a slash or a dot-dot in
    // it is not a name this wrote, and serving it would let the window read any
    // file it can spell.
    let safe = !name.is_empty() && !name.contains('/') && !name.contains("..");
    let bytes = thumbs
        .filter(|_| safe)
        .and_then(|thumbs| std::fs::read(thumbs.at_name(name)).ok());
    match bytes {
        Some(bytes) => tauri::http::Response::builder()
            .header("Content-Type", "image/jpeg")
            .header("Cache-Control", "no-cache")
            .body(bytes)
            .unwrap_or_else(|_| empty_picture()),
        None => empty_picture(),
    }
}

/// What the picture scheme answers with when there is no picture.
///
/// An empty body with a plain status rather than an error page: the window
/// asked for a thumbnail, and "there isn't one" is a normal answer that the
/// row already knows how to draw around.
fn empty_picture() -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(404)
        .body(Vec::new())
        .unwrap_or_default()
}

/// Where passwords are kept. One store for the life of the window.
///
/// Always wrapped in [`EnvOverride`], so `TUNGSTATE_SECRET_<NAME>` supplies a
/// password without the keychain being touched at all — which is what makes a
/// walkthrough against a throwaway server possible without leaving a
/// credential behind on the machine running it.
fn secrets() -> impl SecretStore {
    EnvOverride(KeyringStore::new())
}

/// One end of a link as a backend, through the factory rather than by
/// assuming the local filesystem.
fn backend_for(
    end: &Endpoint,
    journal: &Journal,
) -> std::result::Result<Box<dyn tungstate_backend::Backend>, String> {
    tungstate_backend_opendal::open(end, journal, &secrets()).map_err(describe)
}

/// One journal entry as the window shows it.
#[derive(Debug, Serialize)]
struct OpView {
    id: i64,
    status: String,
    kind: String,
    source: Option<String>,
    destination: Option<String>,
    size: Option<u64>,
    hash: Option<String>,
    link: Option<String>,
    note: Option<String>,
    started_at: i64,
}

impl OpView {
    /// Needs the journal for the same reason [`LinkView::of`] does: a remote
    /// end is stored as an id, and "inbox/a.mp4" is not an answer to "where
    /// did this file go?".
    fn of(op: &Op, journal: &Journal) -> Self {
        Self {
            id: op.id.0,
            status: match op.status {
                OpStatus::Intended => "interrupted",
                OpStatus::Committed => "ok",
                OpStatus::Failed => "failed",
                OpStatus::Skipped => "skipped",
            }
            .to_string(),
            kind: format!("{:?}", op.kind).to_lowercase(),
            source: op.source.as_ref().map(|l| ends::place(l, journal)),
            destination: op.destination.as_ref().map(|l| ends::place(l, journal)),
            size: op.size,
            hash: op.hash.clone(),
            link: op.link.clone(),
            note: op.note.clone(),
            started_at: op.started_at,
        }
    }
}

/// What the window sends to create a link.
#[derive(Debug, Deserialize)]
struct NewLinkForm {
    name: String,
    source: String,
    destination: String,
    source_policy: String,
    verify: String,
    order: String,
    on_conflict: String,
    cooldown_secs: u64,
}

/// Totals as the window shows them.
#[derive(Debug, Clone, Serialize)]
struct SummaryView {
    transferred: u64,
    already_present: u64,
    skipped: u64,
    quarantined: u64,
    failed: u64,
    bytes: u64,
    recovered: u64,
    pruned: u64,
    cancelled: bool,
    destination_lost: bool,
    failures: Vec<FailureView>,
}

#[derive(Debug, Clone, Serialize)]
struct FailureView {
    path: String,
    reason: String,
}

impl From<&Summary> for SummaryView {
    fn from(summary: &Summary) -> Self {
        Self {
            transferred: summary.transferred,
            already_present: summary.already_present,
            skipped: summary.skipped,
            quarantined: summary.quarantined,
            failed: summary.failed,
            bytes: summary.bytes,
            recovered: summary.recovered,
            pruned: summary.pruned,
            cancelled: summary.cancelled,
            destination_lost: summary.destination_lost,
            failures: summary
                .failures
                .iter()
                .map(|failure| FailureView {
                    path: failure.path.display().to_string(),
                    reason: failure.reason.clone(),
                })
                .collect(),
        }
    }
}

// --- governed folders -----------------------------------------------------
//
// The window's half of the second half. The engine keeps its vocabulary; these
// hand the screen plain words, because somebody opening this app has a messy
// folder rather than a mental model of a reconciliation controller.

#[tauri::command]
fn governed(state: State<'_, App>) -> Result<Vec<govern::FolderView>, String> {
    let folders = state.journal.folders().map_err(describe)?;
    Ok(folders
        .into_iter()
        .map(|folder| {
            let root = PathBuf::from(&folder.root);
            let has_rules = govern::has_rules(&root);
            govern::FolderView {
                // Loaded here rather than when the folder is opened, so a
                // broken policy is visible in the list rather than only after
                // clicking into it.
                broken: has_rules.then(|| govern::rules_at(&root).err()).flatten(),
                name: folder.name,
                root: folder.root,
                has_rules,
            }
        })
        .collect())
}

#[tauri::command]
fn govern_folder(root: String, state: State<'_, App>) -> Result<(), String> {
    let path = PathBuf::from(&root);
    if !path.is_dir() {
        return Err(format!("{root} is not a folder"));
    }
    state
        .journal
        .add_folder(&root, &govern::label_for(&path))
        .map(|_| ())
        .map_err(describe)
}

#[tauri::command]
fn forget_folder(root: String, state: State<'_, App>) -> Result<(), String> {
    state.journal.remove_folder(&root).map_err(describe)
}

#[tauri::command]
fn layouts() -> Vec<govern::LayoutView> {
    govern::layouts()
}

#[tauri::command]
fn give_rules(root: String, layout: String) -> Result<(), String> {
    govern::write_layout(&PathBuf::from(root), &layout).map(|_| ())
}

/// Read the shape a folder already has, without changing anything.
///
/// `async` here and on the four commands below: a plain command runs on the
/// main thread on macOS, and these walk the whole folder, so the window froze.
#[tauri::command(async)]
fn learn_folder(root: String) -> Result<govern::LearnedView, String> {
    govern::learn(Path::new(&root))
}

/// What every starting layout would do to this folder, side by side.
#[tauri::command(async)]
fn compare_folder(root: String) -> Result<Vec<tungstate_core::compare::Outcome>, String> {
    govern::compare(Path::new(&root))
}

#[tauri::command]
fn rules_text(root: String) -> Result<String, String> {
    govern::rules_text(&PathBuf::from(root))
}

#[tauri::command(async)]
fn folder_preview(root: String, state: State<'_, App>) -> Result<govern::PreviewView, String> {
    govern::preview(&PathBuf::from(root), &state.journal)
}

#[tauri::command(async)]
fn tidy_folder(root: String, state: State<'_, App>) -> Result<govern::TidyDone, String> {
    let path = PathBuf::from(&root);
    let backend = tungstate_backend::local::LocalBackend::new(path.clone());

    // Close the books on anything a dead process left in flight, before
    // planning against a journal that still describes a world that stopped
    // being true.
    tungstate_execute::resolve_interrupted(&backend, &state.journal, &root)
        .map_err(|e| e.to_string())?;

    let (_, snapshot, plan) = govern::plan_for(&path)?;
    if plan.ops.is_empty() {
        return Ok(govern::TidyDone {
            moved: 0,
            skipped: 0,
            failed: 0,
            already_tidy: true,
        });
    }
    // The one refusal the window keeps, because it is a bug in the rules
    // rather than a judgement about scale: the blast radius is shown in the
    // preview the user just read, and a window that drew two trees has already
    // had the conversation `--yes` exists to force.
    if !plan.settles {
        return Err(format!(
            "these rules never settle: tidying would leave {} file(s) still wanting to move.              A rename that reads {{name}} and adds to it renders differently once the file is              renamed. Fix the rule first.",
            plan.unsettled.len()
        ));
    }

    let applied = tungstate_execute::apply(&plan, &snapshot, &backend, &state.journal, &root)
        .map_err(|e| e.to_string())?;

    Ok(govern::TidyDone {
        moved: govern::files_moved(&plan, applied.skipped.len(), applied.failed.len()),
        skipped: applied.skipped.len(),
        failed: applied.failed.len(),
        already_tidy: false,
    })
}

/// Look for duplicates under `target`, which may be a folder or
/// `connection:folder`.
///
/// Reports progress as it goes: a drive scan takes minutes, and a window that
/// says nothing for minutes looks hung. Async, like every command that walks
/// a folder.
#[tauri::command(async)]
fn find_duplicates(
    target: String,
    similar: bool,
    app: AppHandle,
    state: State<'_, App>,
) -> Result<dupes::FoundView, String> {
    let reporter = app.clone();
    let mut watcher = move |progress: &dupes::ScanProgress| {
        // Dropped rather than raised: a window that cannot hear progress is
        // not a reason to abandon a scan that is working.
        let _ = reporter.emit("dupes://progress", progress);
    };
    dupes::scan(
        &target,
        similar,
        &state.journal,
        &state.scan,
        state.thumbs.as_ref(),
        &mut watcher,
    )
}

/// Put back what a clearing set aside.
///
/// Not the folder half's `put_back`: that one needs a `.tungstate/policy.toml`
/// and this window scans folders that have none.
#[tauri::command(async)]
fn undo_duplicates(target: String, plan: i64, state: State<'_, App>) -> Result<usize, String> {
    dupes::put_back(&target, plan, &state.journal)
}

/// Whether folders are being kept in order, and what has happened.
#[tauri::command]
fn watch_state(state: State<'_, App>) -> Result<watching::WatchView, String> {
    let on = watching::wanted(&state.journal)?;
    Ok(state.watcher.view(on))
}

/// Turn watching on or off, and remember which.
///
/// Turning it on starts a loop at once rather than waiting for the next
/// launch: a switch that does nothing until you restart is a switch people
/// stop believing.
#[tauri::command]
fn set_watching(on: bool, app: AppHandle, state: State<'_, App>) -> Result<(), String> {
    watching::remember(&state.journal, on)?;
    if on {
        begin_watching(&app, &state.watcher);
    } else {
        state.watcher.halt();
    }
    Ok(())
}

/// Start the watcher when the window opens, unless it has been turned off.
///
/// Here rather than lazily from a screen: the folder is meant to be kept in
/// order whether or not anybody is looking at the page that says so.
fn begin_watching_if_wanted(app: &AppHandle, watcher: &Arc<watching::Watching>) {
    let wanted = open_journal()
        .ok()
        .and_then(|journal| watching::wanted(&journal).ok());
    if wanted == Some(true) {
        begin_watching(app, watcher);
    }
}

/// Start the watcher, with the window as its audience.
fn begin_watching(app: &AppHandle, watcher: &Arc<watching::Watching>) {
    let Ok(journal) = open_journal() else {
        return;
    };
    let reporter = app.clone();
    watching::start(watcher, journal, move |notice| {
        // Dropped rather than raised: a window that cannot hear the watcher is
        // not a reason to stop keeping folders in order.
        let _ = reporter.emit("watch://noticed", notice);
    });
}

/// The places scanned before, newest first.
#[tauri::command]
fn recent_scans(state: State<'_, App>) -> Result<Vec<String>, String> {
    dupes::recent(&state.journal)
}

/// Stop the scan that is running, if one is.
#[tauri::command]
fn stop_finding_duplicates(state: State<'_, App>) {
    state.scan.stop();
}

/// Deal with the copies the person ticked.
///
/// `paths` are copies, not groups: every checkbox on screen is its own copy.
/// Anything matched on samples is confirmed byte for byte first, and the
/// engine refuses ticks that would leave a group with nothing.
#[tauri::command(async)]
fn clear_duplicates(
    target: String,
    paths: Vec<String>,
    similar: bool,
    extras: String,
    state: State<'_, App>,
) -> Result<dupes::ClearedView, String> {
    let extras = match extras.as_str() {
        "set-aside" => tungstate_core::dupes::Extras::SetAside,
        "trash" => tungstate_core::dupes::Extras::Trash,
        other => return Err(format!("`{other}` is not a choice: set-aside or trash")),
    };
    dupes::clear(
        &target,
        &paths,
        similar,
        extras,
        &state.journal,
        &state.scan,
    )
}

/// What the person said should happen to extra copies, if they have said.
#[tauri::command]
fn duplicate_action(state: State<'_, App>) -> Result<Option<String>, String> {
    state
        .journal
        .setting(dupes::ACTION_SETTING)
        .map_err(describe)
}

/// Remember what should happen to extra copies.
#[tauri::command]
fn set_duplicate_action(action: String, state: State<'_, App>) -> Result<(), String> {
    if !matches!(action.as_str(), "set-aside" | "trash") {
        return Err(format!("`{action}` is not a choice: set-aside or trash"));
    }
    state
        .journal
        .remember_setting(dupes::ACTION_SETTING, &action)
        .map_err(describe)
}

#[tauri::command(async)]
fn put_back(root: String, plan: i64, state: State<'_, App>) -> Result<govern::PutBackDone, String> {
    let path = PathBuf::from(&root);
    let backend = tungstate_backend::local::LocalBackend::new(path.clone());
    let policy = govern::rules_at(&path)?;
    let fresh = tungstate_attrs::survey(&backend, &policy).map_err(|e| e.to_string())?;
    let undone = tungstate_execute::undo(
        tungstate_journal::plans::PlanId(plan),
        &fresh,
        &backend,
        &state.journal,
        &root,
    )
    .map_err(|e| e.to_string())?;
    // Same trap as `tidy_folder`: `undone.done` counts operations, and undoing
    // a plan also recreates the directories it emptied and removes the ones it
    // made. Only the renames moved a file, so only they are worth counting.
    let files = state
        .journal
        .ops_for_plan(tungstate_journal::plans::PlanId(plan))
        .map_or(undone.done, |ops| {
            ops.iter()
                .filter(|o| o.kind == tungstate_journal::OpKind::Rename)
                .count()
        });
    Ok(govern::PutBackDone { files })
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt as _;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |chosen| {
        let _ = tx.send(chosen);
    });
    let chosen = rx
        .recv()
        .map_err(|_| "the folder chooser closed".to_string())?;
    Ok(chosen.and_then(|p| p.into_path().ok()))
}

#[tauri::command]
fn list_links(state: State<'_, App>) -> Result<Vec<LinkView>, String> {
    state
        .journal
        .links()
        .map(|links| {
            links
                .iter()
                .map(|link| LinkView::of(link, &state.journal))
                .collect()
        })
        .map_err(describe)
}

#[tauri::command]
fn create_link(form: NewLinkForm, state: State<'_, App>) -> Result<(), String> {
    let source_policy = SourcePolicy::parse(&form.source_policy)
        .ok_or("choose whether originals are deleted, trashed, or kept")?;
    let verify =
        VerifyLevel::parse(&form.verify).ok_or("verification must be size, hash, or readback")?;
    let order = Order::parse(&form.order).ok_or("that ordering is not one I know")?;
    let on_conflict =
        ConflictAction::parse(&form.on_conflict).ok_or("that conflict action is not one I know")?;

    let route = resolve_route(&form.source, &form.destination, &state.journal)?;
    refuse_impossible(&route, source_policy, on_conflict)?;

    // Through the backend rather than `Path::is_dir`: for a remote the
    // question is whether the far side has a directory there, and this
    // machine's filesystem has no opinion on that. Catching it here rather
    // than at the first file means the mistake is visible while the form is
    // still on screen.
    let source_backend = backend_for(&route.source, &state.journal)?;
    if !source_backend.stat(Path::new("")).map_err(describe)?.is_dir {
        return Err(format!("{} is not a folder", form.source));
    }

    if route.source.connection == route.destination.connection {
        if route.source.path == route.destination.path {
            return Err("source and destination are the same folder".to_string());
        }
        if route.destination.path.starts_with(&route.source.path) {
            return Err(
                "the destination is inside the source, which would drain into itself".to_string(),
            );
        }
    }

    state
        .journal
        .create_link(&NewLink {
            name: form.name,
            source: route.source,
            destination: route.destination,
            source_policy,
            verify,
            order,
            on_conflict,
            cooldown: Duration::from_secs(form.cooldown_secs),
            saved: true,
        })
        .map(|_| ())
        .map_err(describe)
}

/// A connection as the window shows it.
///
/// `Connection` is not `Serialize` — the journal crate has no serde dependency
/// and does not want one — so this is the same `*View` pattern `LinkView` and
/// `OpView` use.
///
/// `encrypted` and `networked` are derived here rather than in TypeScript. The
/// answer comes from `Scheme::is_encrypted`, which is the one place in the
/// program that knows, and a second implementation in the front end would be a
/// second thing to get wrong about a warning that matters.
#[derive(Debug, Serialize)]
struct ConnectionView {
    name: String,
    scheme: String,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    root: String,
    options: BTreeMap<String, String>,
    encrypted: bool,
    networked: bool,
    /// Why this connection will refuse every write, when it will.
    ///
    /// Derived in Rust rather than tested for in the template, so the CLI and
    /// the window cannot come to different conclusions about the same row.
    rootless: Option<&'static str>,
}

impl From<Connection> for ConnectionView {
    fn from(connection: Connection) -> Self {
        Self {
            name: connection.name,
            scheme: connection.scheme.as_str().to_string(),
            host: connection.host,
            port: connection.port,
            username: connection.username,
            rootless: connection.scheme.rootless_warning(&connection.root),
            root: connection.root,
            options: connection.options,
            encrypted: connection.scheme.is_encrypted(),
            networked: connection.scheme.is_networked(),
        }
    }
}

/// What the window sends to add or edit a connection.
///
/// `name` is present on an edit and ignored there: the command takes the name
/// it is editing separately, because a rename would have to move the keychain
/// entry too and [`ConnectionSettings`] deliberately cannot express one.
#[derive(Debug, Deserialize)]
struct ConnectionForm {
    name: String,
    scheme: String,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    root: String,
    options: BTreeMap<String, String>,
}

impl ConnectionForm {
    fn settings(&self) -> std::result::Result<ConnectionSettings, String> {
        Ok(ConnectionSettings {
            scheme: Scheme::parse(&self.scheme).ok_or("the protocol must be fs, ftp or ftps")?,
            // A blank field in a form is an empty string, not a missing value,
            // and an empty hostname is a hostname nothing can connect to.
            host: blank_to_none(self.host.as_deref()),
            port: self.port,
            username: blank_to_none(self.username.as_deref()),
            root: self.root.clone(),
            options: self.options.clone(),
        })
    }
}

fn blank_to_none(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

/// What a reachability check found, for the row to report.
#[derive(Debug, Serialize)]
struct ProbeView {
    entries: usize,
    /// Which directory those entries were in. "reachable, 18 entries" is
    /// useless when the 18 are the server's own `/bin` because the root was
    /// left at its default, so the count never travels without it.
    root: String,
    /// The first few names, for the same reason the count travels with the
    /// root: whether those entries are your folders or the server's own is the
    /// question, and a number cannot answer it.
    names: Vec<String>,
    /// Whether the root will accept a file. `None` for a connection on this
    /// machine, where there is no far side to refuse one.
    accepts_files: Option<bool>,
}

#[tauri::command]
fn list_connections(state: State<'_, App>) -> Result<Vec<ConnectionView>, String> {
    state
        .journal
        .connections()
        .map(|connections| connections.into_iter().map(Into::into).collect())
        .map_err(describe)
}

/// Add a connection, and store its password if the protocol has one.
///
/// The secret crosses the Tauri IPC as a plain `String`. That boundary is
/// in-process on localhost with the CSP locked to `'self'`, and a form has no
/// alternative to offer. It is never put in a `tracing` field, and the Vue side
/// clears its ref after submitting.
#[tauri::command]
fn add_connection(
    form: ConnectionForm,
    secret: Option<String>,
    state: State<'_, App>,
) -> Result<(), String> {
    let settings = form.settings()?;
    state
        .journal
        .create_connection(&NewConnection {
            name: form.name.clone(),
            scheme: settings.scheme,
            host: settings.host,
            port: settings.port,
            username: settings.username,
            root: settings.root,
            options: settings.options,
        })
        .map_err(describe)?;

    // The row is already written, so a keychain failure says plainly what did
    // and did not happen rather than leaving the user to guess.
    if settings.scheme.authenticates()
        && let Some(secret) = secret.filter(|s| !s.is_empty())
        && let Err(error) = secrets().set(&connection_key(&form.name), &secret)
    {
        return Err(format!(
            "`{}` was saved, but its password was not: {}",
            form.name,
            describe(error)
        ));
    }
    Ok(())
}

#[tauri::command]
fn update_connection(
    name: String,
    form: ConnectionForm,
    state: State<'_, App>,
) -> Result<(), String> {
    state
        .journal
        .update_connection(&name, &form.settings()?)
        .map_err(describe)
}

/// Replace the stored password.
///
/// Offered unconditionally rather than only when one is missing: reading a
/// keychain item from an unsigned binary prompts on macOS, so asking "does
/// this have a password?" would put a system dialog on screen just to decide
/// how to draw a button.
#[tauri::command]
fn set_connection_password(
    name: String,
    secret: String,
    state: State<'_, App>,
) -> Result<(), String> {
    // Looked up first so a typo names itself rather than writing a keychain
    // entry nothing will ever read.
    state.journal.connection_by_name(&name).map_err(describe)?;
    let key = connection_key(&name);
    // A blank answer removes the password rather than storing an empty one,
    // which would be a credential that exists and always fails.
    let store = secrets();
    if secret.is_empty() {
        store.delete(&key).map_err(describe)
    } else {
        store.set(&key, &secret).map_err(describe)
    }
}

/// Check a connection is reachable, without transferring anything.
///
/// A listing of the root, never a write: proving we can reach a NAS must not
/// leave a file in it.
#[tauri::command]
fn test_connection(name: String, state: State<'_, App>) -> Result<ProbeView, String> {
    let connection = state.journal.connection_by_name(&name).map_err(describe)?;
    let root = if connection.root.is_empty() {
        "/".to_string()
    } else {
        connection.root.clone()
    };
    let found = tungstate_backend_opendal::probe(&connection, &state.journal, &secrets())
        .map_err(describe)?;
    let names = found
        .iter()
        .take(12)
        .map(|entry| {
            let name = entry
                .path
                .file_name()
                .unwrap_or(entry.path.as_os_str())
                .to_string_lossy();
            if entry.meta.is_dir {
                format!("{name}/")
            } else {
                name.into_owned()
            }
        })
        .collect();
    // Listing and writing are different permissions, and only one of them is
    // what a drain needs. Asked once here rather than by every file in a run.
    let accepts_files = if connection.scheme.is_networked() {
        Some(
            tungstate_backend_opendal::probe_writable(&connection, &state.journal, &secrets())
                .map_err(describe)?,
        )
    } else {
        None
    };
    Ok(ProbeView {
        entries: found.len(),
        root,
        names,
        accepts_files,
    })
}

/// Forget a connection, and the password that went with it.
#[tauri::command]
fn remove_connection(name: String, state: State<'_, App>) -> Result<(), String> {
    let connection = state.journal.connection_by_name(&name).map_err(describe)?;

    // The row first: if a link still points at it the foreign key refuses, and
    // deleting the password before finding that out would break a live link.
    match state.journal.delete_connection(&name) {
        Ok(()) => {}
        // The foreign key knows something references the row but not what.
        // "Remove these two links first" is actionable where "it is in use" is
        // a guessing game.
        Err(JournalError::ConnectionInUse(_)) => {
            let links = state.journal.links_using(connection.id).map_err(describe)?;
            return Err(format!(
                "`{name}` is still used by {}: {}. Remove {} first.",
                plural(links.len(), "link", "links"),
                links.join(", "),
                if links.len() == 1 { "it" } else { "them" },
            ));
        }
        Err(other) => return Err(describe(other)),
    }

    // A password left behind would be read by the next connection to take this
    // name, which is not a credential anyone meant to reuse.
    secrets().delete(&connection_key(&name)).map_err(describe)
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// One row in a browser pane.
#[derive(Debug, Serialize)]
struct EntryView {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: Option<i64>,
}

/// A directory listing, with everything a pane needs to draw itself.
#[derive(Debug, Serialize)]
struct Listing {
    path: String,
    parent: Option<String>,
    entries: Vec<EntryView>,
}

/// A shortcut offered in the location bar.
#[derive(Debug, Serialize)]
struct Place {
    label: String,
    path: String,
}

/// Where the two panes were last pointed.
///
/// Reopening the app in the folders you left is table stakes for a file
/// browser, and it means a long drain can be resumed without re-navigating.
#[derive(Debug, Default, Serialize, Deserialize)]
struct PaneState {
    left: Option<String>,
    right: Option<String>,
}

fn pane_state_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "tungstate").map(|d| d.data_dir().join("panes.json"))
}

#[tauri::command]
fn last_panes(state: State<'_, App>) -> PaneState {
    let saved: PaneState = pane_state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    PaneState {
        left: saved.left.and_then(|at| still_open(&at, &state.journal)),
        right: saved.right.and_then(|at| still_open(&at, &state.journal)),
    }
}

/// Where a remembered pane can still open, if anywhere.
///
/// The file outlives what it names: a folder gets deleted, or the journal is
/// swapped for one without that connection, and the window used to open on a
/// raw error. A local folder falls back to its nearest parent that exists. A
/// connection is kept only if the journal knows it; whether its folder is
/// still there is not asked, because that is a network call at launch.
fn still_open(at: &str, journal: &Journal) -> Option<String> {
    let end = ends::parse_end(at, None, journal).ok()?;
    if end.connection.is_some() {
        return Some(at.to_string());
    }
    end.path
        .ancestors()
        .find(|dir| dir.is_dir())
        .map(|dir| dir.display().to_string())
}

#[tauri::command]
fn remember_panes(panes: PaneState) {
    // Losing this is a cosmetic annoyance, never a correctness problem, so a
    // failure here is not worth interrupting the user for.
    if let Some(path) = pane_state_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(raw) = serde_json::to_string(&panes) {
            let _ = std::fs::write(path, raw);
        }
    }
}

#[tauri::command]
fn places(state: State<'_, App>) -> Vec<Place> {
    let mut places = Vec::new();
    if let Some(home) = directories_home() {
        for (label, sub) in [
            ("Home", ""),
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Movies", "Movies"),
        ] {
            let path = if sub.is_empty() {
                home.clone()
            } else {
                home.join(sub)
            };
            if path.is_dir() {
                places.push(Place {
                    label: label.to_string(),
                    path: path.display().to_string(),
                });
            }
        }
    }
    // Mounted volumes are where a NAS appears once Finder has connected to it.
    #[cfg(target_os = "macos")]
    if let Ok(volumes) = std::fs::read_dir("/Volumes") {
        let mut mounted: Vec<_> = volumes
            .flatten()
            .filter(|e| e.path().is_dir())
            // `.timemachine` and its like are the system's own mounts, hidden
            // in Finder, and the first of them became the default pane.
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            // The boot volume is already reachable as /; listing it as a
            // "volume" only invites someone to drain their own system disk.
            // It is a link to / from /Volumes, so ask where it leads.
            .filter(|e| std::fs::canonicalize(e.path()).map_or(true, |real| real != Path::new("/")))
            .map(|e| Place {
                label: e.file_name().to_string_lossy().into_owned(),
                path: e.path().display().to_string(),
            })
            .collect();
        mounted.sort_by(|a, b| a.label.cmp(&b.label));
        places.append(&mut mounted);
    }
    // Last, and on every platform. A connection is the route that works when
    // the mount does not — and it is the only entry Windows and Linux get,
    // where the volume scan above contributes nothing at all.
    //
    // `name:` is the connection's root written the way `parse_end` reads it,
    // so this needs no new shape: a place is still a label and a location.
    if let Ok(connections) = state.journal.connections() {
        places.extend(connections.into_iter().map(|connection| Place {
            path: format!("{}:", connection.name),
            label: connection.name,
        }));
    }
    places
}

fn directories_home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// List a directory for one side of the browser.
///
/// `path` is a composite location — `/Users/me/Videos` or `nas:inbox/2026` —
/// read by the same parser the command line uses, so a pane and a link spec
/// cannot disagree about what a string means.
#[tauri::command]
fn browse(path: String, state: State<'_, App>) -> Result<Listing, String> {
    listing_for(&path, &state.journal)
}

/// The body of [`browse`], without the `State` wrapper.
///
/// Split out so it can be tested: a `#[tauri::command]` needs an `AppHandle` to
/// call, and this is the function where a mistake about what a location string
/// means actually shows up.
fn listing_for(path: &str, journal: &Journal) -> Result<Listing, String> {
    let end = ends::parse_end(path, None, journal).map_err(describe)?;
    // Canonical rather than whatever was typed: `nas:inbox/` and `nas:inbox`
    // are the same place, and every child built below hangs off this one
    // string.
    let here = ends::describe(&end, journal);

    // A backend rooted at the directory being shown, so the same path rules
    // that protect a transfer also apply to browsing.
    let backend = tungstate_backend_opendal::open(&end, journal, &secrets()).map_err(describe)?;

    let mut entries: Vec<EntryView> = backend
        .read_dir(Path::new(""))
        .map_err(describe)?
        .into_iter()
        .filter(|entry| !entry.meta.is_symlink)
        .map(|entry| {
            let name = entry.path.display().to_string();
            EntryView {
                path: ends::join_display(&here, &name),
                is_dir: entry.meta.is_dir,
                size: entry.meta.len,
                modified: entry.meta.modified.and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .and_then(|d| i64::try_from(d.as_millis()).ok())
                }),
                name,
            }
        })
        .filter(|entry| !entry.name.starts_with('.'))
        .collect();

    // Folders first, then by name, which is what every file browser does and
    // what the eye expects.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(Listing {
        parent: ends::parent_display(&here),
        path: here,
        entries,
    })
}

/// One direction of a transfer: what goes from where to where.
#[derive(Debug, Clone, Deserialize)]
struct Leg {
    source: String,
    destination: String,
    /// Names relative to `source`.
    names: Vec<String>,
}

/// What the browser sends to move or copy.
///
/// Two legs means an exchange: what is ticked on the left goes right while what
/// is ticked on the right goes left, run one after the other so each is
/// journaled and resumable on its own.
#[derive(Debug, Deserialize)]
struct TransferRequest {
    legs: Vec<Leg>,
    source_policy: String,
    verify: String,
    on_conflict: String,
    /// The order files are taken in. Absent means largest-first, which is what
    /// this was hard-coded to before the dialog could say.
    #[serde(default)]
    order: Option<String>,
    /// Seconds a file must have been untouched. Absent means none: the user is
    /// looking at these files and chose them, so there is nothing to wait for.
    #[serde(default)]
    cooldown_secs: Option<u64>,
    /// When set, the pair is remembered and can be run again later.
    save_as: Option<String>,
}

/// The settings a validated transfer request resolves to.
struct Plan {
    source: Endpoint,
    destination: Endpoint,
    source_policy: SourcePolicy,
    verify: VerifyLevel,
    on_conflict: ConflictAction,
    order: Order,
    cooldown: Duration,
}

/// The parts of a transfer only the journal can answer: which places the two
/// ends are in, and what the destination's protocol is able to do.
///
/// Resolved once and passed in so [`plan_transfer`] stays a pure function of
/// its inputs, which is what makes it testable without a journal or a keychain.
struct Route {
    source: Endpoint,
    destination: Endpoint,
    /// `None` when the destination is this machine.
    destination_scheme: Option<Scheme>,
}

/// Read a leg's two composite location strings and look up what they name.
fn resolve_route(
    source: &str,
    destination: &str,
    journal: &Journal,
) -> std::result::Result<Route, String> {
    let source = ends::parse_end(source, None, journal).map_err(describe)?;
    let destination = ends::parse_end(destination, None, journal).map_err(describe)?;
    let destination_scheme = destination
        .connection
        .map(|id| journal.connection_by_id(id).map(|c| c.scheme))
        .transpose()
        .map_err(describe)?;
    Ok(Route {
        source,
        destination,
        destination_scheme,
    })
}

/// The two refusals the CLI makes when a link is created, made here too.
///
/// The window must not be able to create a link the command line would reject;
/// the two surfaces are the same product and a rule that holds in only one of
/// them is a rule nobody can rely on. Both are about the destination's
/// protocol rather than its path, which is why they need [`Route`] and not
/// just the endpoints.
fn refuse_impossible(
    route: &Route,
    source_policy: SourcePolicy,
    on_conflict: ConflictAction,
) -> std::result::Result<(), String> {
    // `trash::delete` drives this desktop's trash and knows nothing about a
    // remote. Remote trash (`.tungstate-trash/`, DESIGN.md §4) is a later
    // slice, so say so now rather than at the first file.
    if source_policy == SourcePolicy::Trash && route.source.is_remote() {
        return Err(
            "moving originals to the trash needs a source on this machine, and this \
                    one is on a connection. Choose to delete them once verified, or to leave \
                    them alone."
                .to_string(),
        );
    }

    // `replace` keeps the file already there by moving it aside first, and
    // moving needs rename, which FTP does not give us. Refused here rather
    // than at the first clash, halfway through a drain.
    if on_conflict == ConflictAction::Replace
        && route
            .destination_scheme
            .is_some_and(|scheme| !scheme.can_rename())
    {
        return Err(
            "this destination cannot move the file already there aside, so replacing \
                    it is not something it can promise. Use set aside, keep both, or leave \
                    that one here."
                .to_string(),
        );
    }

    Ok(())
}

/// Check a request from the window before anything touches the disk.
///
/// Kept separate from the command so it can be tested. The nesting check is the
/// one that matters: draining a folder into its own subfolder would feed the
/// walk its own output.
fn plan_transfer(
    request: &TransferRequest,
    leg: &Leg,
    route: Route,
) -> std::result::Result<Plan, String> {
    let source_policy = SourcePolicy::parse(&request.source_policy)
        .ok_or("choose what happens to the originals")?;
    let verify = VerifyLevel::parse(&request.verify).ok_or("unknown verification level")?;
    let on_conflict =
        ConflictAction::parse(&request.on_conflict).ok_or("unknown conflict action")?;
    let order = match request.order.as_deref() {
        None => Order::LargestFirst,
        Some(text) => Order::parse(text).ok_or("unknown order")?,
    };
    let cooldown = Duration::from_secs(request.cooldown_secs.unwrap_or(0));

    if leg.names.is_empty() {
        return Err("nothing is selected".to_string());
    }

    refuse_impossible(&route, source_policy, on_conflict)?;
    let Route {
        source,
        destination,
        ..
    } = route;

    // Scoped to one place on purpose. Path containment says nothing across two
    // different connections: `inbox` on the NAS is not inside `inbox` here, and
    // refusing that pair would block the ordinary drain.
    if source.connection == destination.connection {
        if source.path == destination.path {
            return Err("those are the same folder".to_string());
        }
        if destination.path.starts_with(&source.path) {
            return Err(
                "the destination is inside the source, which would copy into itself".to_string(),
            );
        }
        if source.path.starts_with(&destination.path) {
            return Err(
                "the source is inside the destination, which would copy into itself".to_string(),
            );
        }
    }

    Ok(Plan {
        source,
        destination,
        source_policy,
        verify,
        on_conflict,
        order,
        cooldown,
    })
}

/// What a transfer would do, for the dialog to show before anything happens.
#[derive(Debug, Serialize)]
struct PreviewView {
    /// Names ticked on both sides, which would clash in both directions.
    overlapping: Vec<String>,
    fresh: u64,
    same_size: u64,
    clashes: u64,
    too_recent: u64,
    bytes: u64,
    removes_originals: bool,
    items: Vec<ProspectView>,
}

#[derive(Debug, Serialize)]
struct ProspectView {
    path: String,
    size: u64,
    /// move, check, clash or hold.
    outcome: &'static str,
    existing: Option<u64>,
    /// Which leg this belongs to, so an exchange can be read at a glance.
    towards: &'static str,
}

/// What running a saved link would do, without doing any of it.
///
/// Its own command rather than a case inside `preview_transfer`: that one
/// takes the ticked files and the settings from the dialog, and a saved link
/// is the opposite — it already carries both, and the selection it remembers
/// is the one to honour.
#[tauri::command]
fn preview_link(name: String, state: State<'_, App>) -> Result<PreviewView, String> {
    let link = state.journal.link_by_name(&name).map_err(describe)?;
    let source = backend_for(&link.source, &state.journal)?;
    let destination = backend_for(&link.destination, &state.journal)?;

    // The batch it was asked to move, where it has one. No rows means the
    // whole root, which is what a saved folder-pair means.
    let chosen = state.journal.files_for(link.id).map_err(describe)?;
    let only = (!chosen.is_empty()).then_some(chosen);

    let preview = tungstate_transfer::preview(
        &link,
        source.as_ref(),
        destination.as_ref(),
        only.as_deref(),
    )
    .map_err(describe)?;

    Ok(PreviewView {
        overlapping: Vec::new(),
        fresh: preview.fresh,
        same_size: preview.same_size,
        clashes: preview.clashes,
        too_recent: preview.too_recent,
        bytes: preview.bytes,
        removes_originals: preview.removes_originals,
        items: preview.items.iter().map(prospect_view).collect(),
    })
}

#[tauri::command]
fn preview_transfer(
    request: TransferRequest,
    state: State<'_, App>,
) -> Result<PreviewView, String> {
    let mut view = PreviewView {
        fresh: 0,
        same_size: 0,
        clashes: 0,
        too_recent: 0,
        bytes: 0,
        removes_originals: false,
        overlapping: overlapping_names(&request.legs),
        items: Vec::new(),
    };

    for leg in &request.legs {
        let route = resolve_route(&leg.source, &leg.destination, &state.journal)?;
        let plan = plan_transfer(&request, leg, route)?;

        // A throwaway link carrying the chosen settings. Nothing is stored: a
        // preview must not leave a trace any more than it moves a file.
        let link = Link {
            id: tungstate_journal::LinkId(0),
            name: String::from("preview"),
            source: plan.source.clone(),
            destination: plan.destination.clone(),
            source_policy: plan.source_policy,
            verify: plan.verify,
            order: plan.order,
            on_conflict: plan.on_conflict,
            cooldown: plan.cooldown,
            saved: false,
            deleted_at: None,
        };

        let source = backend_for(&plan.source, &state.journal)?;
        let destination = backend_for(&plan.destination, &state.journal)?;
        let names: Vec<PathBuf> = leg.names.iter().map(PathBuf::from).collect();
        let preview =
            tungstate_transfer::preview(&link, source.as_ref(), destination.as_ref(), Some(&names))
                .map_err(describe)?;

        view.fresh += preview.fresh;
        view.same_size += preview.same_size;
        view.clashes += preview.clashes;
        view.too_recent += preview.too_recent;
        view.bytes += preview.bytes;
        view.removes_originals |= preview.removes_originals;
        view.items.extend(preview.items.iter().map(|item| {
            let (outcome, existing) = match &item.prospect {
                tungstate_transfer::Prospect::Fresh => ("move", None),
                tungstate_transfer::Prospect::SameSize { existing } => ("check", Some(*existing)),
                tungstate_transfer::Prospect::Clash { existing } => ("clash", Some(*existing)),
                tungstate_transfer::Prospect::TooRecent => ("hold", None),
            };
            ProspectView {
                path: item.path.display().to_string(),
                size: item.size,
                outcome,
                existing,
                towards: if leg.source == request.legs[0].source {
                    "forward"
                } else {
                    "back"
                },
            }
        }));
    }

    Ok(view)
}

/// One prospect as the window shows it.
///
/// `towards` is "forward" here: a saved link has one direction, and the
/// distinction only means something for the two legs of an exchange.
fn prospect_view(item: &tungstate_transfer::Prospective) -> ProspectView {
    let (outcome, existing) = match item.prospect {
        tungstate_transfer::Prospect::Fresh => ("move", None),
        tungstate_transfer::Prospect::SameSize { existing } => ("check", Some(existing)),
        tungstate_transfer::Prospect::Clash { existing } => ("clash", Some(existing)),
        tungstate_transfer::Prospect::TooRecent => ("hold", None),
    };
    ProspectView {
        path: item.path.display().to_string(),
        size: item.size,
        outcome,
        existing,
        towards: "forward",
    }
}

/// Names ticked on both sides at once.
///
/// Worth naming in the preview: such a file clashes in both directions, and the
/// result of resolving it twice is rarely what anyone intended.
fn overlapping_names(legs: &[Leg]) -> Vec<String> {
    if legs.len() < 2 {
        return Vec::new();
    }
    let first: std::collections::BTreeSet<&String> = legs[0].names.iter().collect();
    legs[1]
        .names
        .iter()
        .filter(|n| first.contains(n))
        .cloned()
        .collect()
}

#[tauri::command]
fn start_transfer(request: TransferRequest, app: AppHandle) -> Result<String, String> {
    start_transfer_inner(request, app).inspect_err(|error| {
        // Also in the log, so a terminal run or a bug report carries the
        // reason without anyone having to read it off the screen.
        tracing::error!(%error, "a transfer could not be started");
    })
}

fn start_transfer_inner(request: TransferRequest, app: AppHandle) -> Result<String, String> {
    if request.legs.is_empty() {
        return Err("nothing is selected".to_string());
    }
    let state = app.state::<App>();

    // Validate every leg before creating anything, so a bad second leg cannot
    // leave a half-configured exchange behind.
    let plans: Vec<Plan> = request
        .legs
        .iter()
        .map(|leg| {
            resolve_route(&leg.source, &leg.destination, &state.journal)
                .and_then(|route| plan_transfer(&request, leg, route))
        })
        .collect::<std::result::Result<_, _>>()?;

    // Saving only makes sense for a single direction: a link is one source and
    // one destination, and an exchange is two of them.
    let saved = request.save_as.is_some() && plans.len() == 1;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let mut names = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        let name = match (&request.save_as, saved) {
            (Some(chosen), true) => chosen.clone(),
            _ => format!("browser-{stamp}-{index}"),
        };
        state
            .journal
            .create_link(&NewLink {
                name: name.clone(),
                source: plan.source.clone(),
                destination: plan.destination.clone(),
                source_policy: plan.source_policy,
                verify: plan.verify,
                order: plan.order,
                on_conflict: plan.on_conflict,
                // Defaults to none when the dialog does not say: the user is
                // looking at these files and chose them, so there is no reason
                // to hold back something written moments ago.
                cooldown: plan.cooldown,
                saved,
            })
            .map_err(describe)?;

        // Written down, not carried in memory. A selection that only exists
        // in a worker thread dies with the process, and a resumed run with
        // nothing to consult walks the whole source root instead of the batch.
        let created = state.journal.link_by_name(&name).map_err(describe)?;
        let chosen: Vec<PathBuf> = request.legs[index]
            .names
            .iter()
            .map(PathBuf::from)
            .collect();
        state
            .journal
            .set_files(created.id, &chosen)
            .map_err(describe)?;

        names.push(name);
    }

    let accepted = spawn_run(&app, names)?;
    // The count of files is what the dialog reports; whether this joined a run
    // already going is separate and reaches the window through `queued`.
    let _ = app.emit("transfer://queued", &accepted);
    Ok(names_summary(&request))
}

fn names_summary(request: &TransferRequest) -> String {
    let total: usize = request.legs.iter().map(|l| l.names.len()).sum();
    format!("{total}")
}

#[tauri::command]
fn history(path: String, state: State<'_, App>) -> Result<Vec<OpView>, String> {
    state
        .journal
        .history(&tungstate_journal::resolve_for_lookup(
            std::path::Path::new(&path),
        ))
        .map(|ops| {
            ops.iter()
                .map(|op| OpView::of(op, &state.journal))
                .collect()
        })
        .map_err(describe)
}

#[tauri::command]
fn whereis(target: String, state: State<'_, App>) -> Result<Vec<OpView>, String> {
    // A BLAKE3 digest is 64 hex characters and no real path looks like one.
    let is_hash = target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit());
    let locator = if is_hash {
        Locator::Hash(&target)
    } else {
        Locator::Path(std::path::Path::new(&target))
    };
    state
        .journal
        .whereis(&locator)
        .map(|ops| {
            ops.iter()
                .map(|op| OpView::of(op, &state.journal))
                .collect()
        })
        .map_err(describe)
}

#[tauri::command]
fn recent(state: State<'_, App>) -> Result<Vec<OpView>, String> {
    state
        .journal
        .recent(200)
        .map(|ops| {
            ops.iter()
                .map(|op| OpView::of(op, &state.journal))
                .collect()
        })
        .map_err(describe)
}

/// Files sitting in a link's quarantine folder, waiting for a decision.
///
/// Through the destination backend, because that is how the engine put them
/// there. This used to walk `std::fs` against `link.destination.path`, which
/// for a remote link is a connection-relative path resolved against this
/// machine's working directory — so it found nothing and reported "nothing is
/// set aside" however many files there were.
#[tauri::command]
fn quarantined(link: String, state: State<'_, App>) -> Result<Vec<String>, String> {
    let link = state.journal.link_by_name(&link).map_err(describe)?;
    let destination = backend_for(&link.destination, &state.journal)?;
    tungstate_transfer::quarantined(destination.as_ref())
        .map(|paths| paths.iter().map(|p| p.display().to_string()).collect())
        .map_err(describe)
}

/// A run that was begun and never finished, as the window shows it.
#[derive(Debug, Serialize)]
struct InterruptedView {
    link: String,
    source: String,
    destination: String,
    files: usize,
    bytes: u64,
    /// The first few filenames, so the banner can say what rather than only
    /// how many.
    names: Vec<String>,
}

#[tauri::command]
fn interrupted(state: State<'_, App>) -> Result<Vec<InterruptedView>, String> {
    state
        .journal
        .interrupted()
        .map(|runs| {
            runs.iter()
                .map(|run| InterruptedView {
                    link: run.link.name.clone(),
                    source: ends::describe(&run.link.source, &state.journal),
                    destination: ends::describe(&run.link.destination, &state.journal),
                    files: run.ops.len(),
                    bytes: run.bytes(),
                    names: run
                        .ops
                        .iter()
                        .filter_map(|op| op.source.as_ref())
                        .map(|l| l.path.display().to_string())
                        .take(5)
                        .collect(),
                })
                .collect()
        })
        .map_err(describe)
}

/// Finish an interrupted run. Works for a one-off browser link, which is the
/// case that would otherwise be unreachable: `list_links` hides unsaved links.
#[tauri::command]
fn resume_interrupted(link: String, app: AppHandle) -> Result<Accepted, String> {
    spawn_run(&app, vec![link])
}

/// Abandon an interrupted run and reclaim what it left at the destination.
#[tauri::command]
fn discard_interrupted(link: String, state: State<'_, App>) -> Result<u64, String> {
    let found = state.journal.link_by_name(&link).map_err(describe)?;
    let destination = backend_for(&found.destination, &state.journal)?;
    tungstate_transfer::discard(&found, destination.as_ref(), &state.journal)
        .map(|removed| removed.bytes)
        .map_err(describe)
}

#[tauri::command]
fn cancel_run(state: State<'_, App>) {
    state.cancel.after_this_file();
}

/// Put the run down where it is, mid-file.
///
/// Separate command rather than an argument to `cancel_run` because they are
/// different promises and the window says so: this one abandons whatever is
/// being written. What it leaves is what a killed process leaves, so the
/// interrupted-run banner picks it up with Resume and Clean up already
/// attached — the kill switch needs no recovery of its own.
#[tauri::command]
fn stop_now(state: State<'_, App>) {
    state.cancel.now();
}

#[tauri::command]
fn resolve_conflict(
    action: String,
    apply_to_all: bool,
    state: State<'_, App>,
) -> Result<(), String> {
    let action = ConflictAction::parse(&action).ok_or("that conflict action is not one I know")?;
    if state.conflicts.reply(Reply {
        answer: Answer::Conflict(action),
        apply_to_all,
    }) {
        Ok(())
    } else {
        Err("nothing is waiting for that answer any more".to_string())
    }
}

/// Answer "an identical copy is already there — may the original here go?".
#[tauri::command]
fn resolve_identical(
    remove: bool,
    apply_to_all: bool,
    state: State<'_, App>,
) -> Result<(), String> {
    let action = if remove {
        IdenticalAction::DeleteOriginal
    } else {
        IdenticalAction::KeepOriginal
    };
    if state.conflicts.reply(Reply {
        answer: Answer::Identical(action),
        apply_to_all,
    }) {
        Ok(())
    } else {
        Err("nothing is waiting for that answer any more".to_string())
    }
}

/// Remove a link, keeping any history that refers to it.
///
/// Returns whether the row went or was retired, so the window can say which
/// happened rather than implying the record is gone when it is not.
/// An archive as the window shows it. Mirrors `Archive` in the journal.
#[derive(Debug, Serialize)]
struct ArchiveView {
    name: String,
    archived_at: i64,
    size: u64,
    /// `None` when the file cannot be read. Shown rather than hidden: an
    /// archive that will not open is exactly what someone needs to know about.
    links: Option<usize>,
    connections: Option<usize>,
    operations: Option<usize>,
}

impl From<tungstate_journal::Archive> for ArchiveView {
    fn from(archive: tungstate_journal::Archive) -> Self {
        Self {
            name: archive.name,
            archived_at: archive.archived_at,
            size: archive.size,
            links: archive.summary.map(|s| s.links),
            connections: archive.summary.map(|s| s.connections),
            operations: archive.summary.map(|s| s.operations),
        }
    }
}

#[tauri::command]
fn archives(state: State<'_, App>) -> Result<Vec<ArchiveView>, String> {
    state
        .journal
        .archives()
        .map(|found| found.into_iter().map(Into::into).collect())
        .map_err(describe)
}

/// Archive everything and start again empty. Returns the archive's name.
#[tauri::command]
fn reset_storage(state: State<'_, App>) -> Result<String, String> {
    refuse_while_running(&state)?;
    state.journal.reset().map_err(describe)
}

/// Bring an archive back. Returns the name the current state was put under,
/// so the window can say that this is reversible rather than claim it.
#[tauri::command]
fn restore_archive(name: String, state: State<'_, App>) -> Result<String, String> {
    refuse_while_running(&state)?;
    state.journal.restore(&name).map_err(describe)
}

#[tauri::command]
fn forget_archive(name: String, state: State<'_, App>) -> Result<(), String> {
    state.journal.forget_archive(&name).map_err(describe)
}

/// Ask for a file to write an export to, or `None` if the user cancelled.
///
/// The picker runs in Rust rather than through the dialog plugin's JavaScript
/// half, which would mean a new npm dependency for two calls. The plugin is
/// already here for the Rust side.
#[tauri::command]
async fn pick_save_file(suggested: String, app: AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt as _;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_file_name(&suggested)
        .add_filter("JSON", &["json"])
        .save_file(move |chosen| {
            let _ = tx.send(chosen);
        });
    let chosen = rx
        .recv()
        .map_err(|_| "the file picker closed unexpectedly")?;
    Ok(chosen.and_then(|p| p.into_path().ok()))
}

/// Ask for a file to read an export from, or `None` if the user cancelled.
#[tauri::command]
async fn pick_open_file(app: AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt as _;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("JSON", &["json"])
        .pick_file(move |chosen| {
            let _ = tx.send(chosen);
        });
    let chosen = rx
        .recv()
        .map_err(|_| "the file picker closed unexpectedly")?;
    Ok(chosen.and_then(|p| p.into_path().ok()))
}

/// Write an export to a path the user chose.
#[tauri::command]
fn export_storage(path: PathBuf, state: State<'_, App>) -> Result<(), String> {
    let document = state.journal.export().map_err(describe)?;
    let text =
        serde_json::to_string_pretty(&document).expect("an export is plain data and always prints");
    std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Read an export back in. Refuses unless the journal is empty.
#[tauri::command]
fn import_storage(path: PathBuf, state: State<'_, App>) -> Result<usize, String> {
    refuse_while_running(&state)?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let document = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not readable JSON: {e}", path.display()))?;
    state.journal.import(&document).map_err(describe)
}

/// Refuse to move the ground under a run that is going.
///
/// The journal is what a running transfer writes its intent to; swapping it
/// mid-flight would leave the run unable to record what it did.
fn refuse_while_running(state: &State<'_, App>) -> Result<(), String> {
    if state.queue.is_running() {
        return Err("a transfer is running; wait for it or stop it first".to_string());
    }
    Ok(())
}

#[tauri::command]
fn remove_link(name: String, state: State<'_, App>) -> Result<bool, String> {
    state
        .journal
        .remove_link(&name)
        .map(|removal| removal == tungstate_journal::Removal::Retired)
        .map_err(describe)
}

/// Ask for at most `files` at once, or `0` to stop asking.
///
/// Deliberately a property of the window rather than of a link: it is a
/// judgement about this network right now, the same way `--parallel` is a flag
/// on a run rather than a field on a link.
#[tauri::command]
fn set_at_once(files: usize, state: State<'_, App>) {
    // Capped rather than refused. A number nobody could have meant is a slip,
    // and the governor would reject it by handshake anyway.
    state
        .at_once_ceiling
        .store(files.min(64), Ordering::Relaxed);
}

#[tauri::command]
fn run_link(name: String, app: AppHandle) -> Result<Accepted, String> {
    spawn_run(&app, vec![name])
}

/// Run each leg in turn on one worker thread, reporting a single combined result.
///
/// Sequential rather than parallel: two legs of an exchange can touch the same
/// names, and running them at once would race. Each leg is its own link, so each
/// is journaled and resumable on its own terms.
fn spawn_run(app: &AppHandle, links: Vec<String>) -> Result<Accepted, String> {
    let state = app.state::<App>();

    // Resolved before anything is queued, so a bad name is refused here rather
    // than halfway through a run that has already started moving files.
    let mut wanted = VecDeque::with_capacity(links.len());
    for name in &links {
        wanted.push_back(state.journal.link_by_name(name).map_err(describe)?);
    }

    // The lock is held across the push *and* the decision to start a worker.
    // That is the whole correctness argument: a worker only ever clears
    // `running` while holding this same lock, so it cannot decide it has
    // finished in the window between this push and this check.
    if !state.queue.submit(wanted) {
        // A worker is already going and will reach these. The caller has to be
        // told, because the window blanks its progress list when a transfer
        // starts, and doing that here would wipe the live view of the run
        // these files just joined.
        return Ok(Accepted {
            started: false,
            waiting: state.queue.waiting(),
        });
    }

    state.cancel.clear();
    let replies = state.conflicts.open();
    let cancel = Arc::clone(&state.cancel);

    let worker = app.clone();
    // The engine is synchronous and will block for as long as the drain takes.
    // Its own thread keeps the window responsive throughout.
    std::thread::spawn(move || {
        let state = worker.state::<App>();
        // Clears `running` however this thread ends, including a panic. Set
        // with a plain store at the end, one panic would leave the flag stuck
        // true and every later Move would be refused with "a transfer is
        // already running" — for the rest of the session, with no way back
        // but restarting the app.
        let _guard = RunGuard(worker.clone());
        let mut resolver = WindowResolver::new(worker.clone(), replies, ConflictAction::Quarantine);
        let mut progress = EventProgress::new(worker.clone());
        let mut total = Summary::default();
        let mut failure = None;

        // `next` is what ends the run: it hands back `None` only after it has
        // recorded that this worker is finished, under the lock that a
        // submission takes to decide whether to start one.
        while let Some(link) = state.queue.next() {
            // Per link rather than per run: a queue that can grow has no
            // meaningful "first link" whose conflict rule speaks for the rest.
            resolver.follow_conflict_rule(link.on_conflict);

            let ends = backend_for(&link.source, &state.journal).and_then(|source| {
                backend_for(&link.destination, &state.journal).map(|dest| (source, dest))
            });
            let (source, destination) = match ends {
                Ok(pair) => pair,
                Err(message) => {
                    failure = Some(message);
                    break;
                }
            };
            let mut transfer = Transfer::new(
                &link,
                source.as_ref(),
                destination.as_ref(),
                &state.journal,
                &mut resolver,
                &mut progress,
            )
            .cancellable(Arc::clone(&cancel));
            let ceiling = state.at_once_ceiling.load(Ordering::Relaxed);
            if ceiling > 0 {
                transfer = transfer.parallel(ceiling);
            }

            match transfer.run() {
                Ok(summary) => {
                    let stop = summary.cancelled || summary.destination_lost;
                    accumulate(&mut total, summary);
                    if stop {
                        break;
                    }
                }
                Err(error) => {
                    failure = Some(describe(error));
                    break;
                }
            }
        }

        // Every `break` above is a stop, and a stop has to mean the queue as
        // well. Leaving work behind would have the next unrelated Move quietly
        // run whatever Stop was pressed on.
        state.queue.clear();

        let _ = match failure {
            Some(message) => worker.emit("transfer://error", message),
            None => worker.emit("transfer://done", SummaryView::from(&total)),
        };
    });

    Ok(Accepted {
        started: true,
        waiting: 0,
    })
}

/// What happened to a submission: a new run, or work added to one in flight.
#[derive(Debug, Serialize)]
struct Accepted {
    /// True when this call started the worker, which is when the window may
    /// clear the progress it is showing.
    started: bool,
    /// How much is queued behind the file being moved right now.
    waiting: usize,
}

/// Releases the "a transfer is running" flag however the worker thread ends.
///
/// `Drop` runs while a thread unwinds, so this holds even if the engine or a
/// backend panics. The alternative — a store at the end of the happy path —
/// turns one panic into a window that refuses every transfer until it is
/// restarted.
struct RunGuard(AppHandle);

impl Drop for RunGuard {
    fn drop(&mut self) {
        let state = self.0.state::<App>();
        state.conflicts.close();
        state.queue.stand_down();
    }
}

/// Fold one leg's result into the running total for the whole operation.
fn accumulate(total: &mut Summary, leg: Summary) {
    total.transferred += leg.transferred;
    total.already_present += leg.already_present;
    total.skipped += leg.skipped;
    total.quarantined += leg.quarantined;
    total.failed += leg.failed;
    total.bytes += leg.bytes;
    total.recovered += leg.recovered;
    total.pruned += leg.pruned;
    total.cancelled |= leg.cancelled;
    total.destination_lost |= leg.destination_lost;
    total.failures.extend(leg.failures);
}

/// Render an error and everything underneath it.
///
/// The top line alone is rarely enough; the cause is where the answer is.
fn describe(error: impl std::error::Error) -> String {
    use std::fmt::Write as _;

    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        // write! into a String cannot fail, and swallowing the Result keeps the
        // signature honest: rendering an error must not itself be fallible.
        let _ = write!(message, ": {cause}");
        source = cause.source();
    }
    message
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            // Ours at info, everybody else's at warn. A media decoder narrates
            // every atom of a container it does not like, and a corrupt mp4
            // would otherwise fill somebody's terminal with a library's
            // internal monologue. RUST_LOG still overrides all of it.
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,tungstate=info".into()),
        )
        .init();

    let journal = match open_journal() {
        Ok(journal) => journal,
        Err(error) => {
            // No journal means no history and no crash recovery, so there is
            // nothing safe to do. Fail loudly rather than half-working.
            eprintln!("tungstate could not open its journal: {}", describe(error));
            std::process::exit(1);
        }
    };

    let thumbs = cache_dir().and_then(|dir| Thumbs::at(&dir.join("thumbnails")).ok());
    let serving = thumbs.clone();
    let watcher: Arc<watching::Watching> = Arc::default();
    let starting = Arc::clone(&watcher);

    tauri::Builder::default()
        // Small pictures reach the window over their own scheme rather than
        // inside a command's answer: fifty photographs as base64 is most of a
        // megabyte of JSON on every redraw, and this is a file on disk being
        // asked for by name.
        .register_uri_scheme_protocol("thumb", move |_ctx, request| {
            serve_picture(serving.as_ref(), request.uri().path())
        })
        // Deliberately no close handler. The window and the run are the same
        // thing until slice 10 puts the engine in a daemon, and pretending
        // otherwise — hiding the window so a drain continues invisibly — buys
        // a half-daemon with no way back on a platform without a dock icon.
        // Closing stops the drain, which is safe: the operation stays
        // `intended`, and the next launch offers to finish or clear it.
        .plugin(tauri_plugin_dialog::init())
        .manage(App {
            journal,
            conflicts: ConflictChannel::default(),
            cancel: Arc::new(Stop::new()),
            scan: dupes::Scan::default(),
            watcher: Arc::clone(&watcher),
            thumbs,
            queue: RunQueue::new(),
            at_once_ceiling: AtomicUsize::new(0),
        })
        .setup(move |app| {
            begin_watching_if_wanted(&app.handle().clone(), &starting);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            governed,
            govern_folder,
            forget_folder,
            layouts,
            give_rules,
            learn_folder,
            compare_folder,
            rules_text,
            folder_preview,
            tidy_folder,
            put_back,
            pick_folder,
            find_duplicates,
            stop_finding_duplicates,
            recent_scans,
            undo_duplicates,
            watch_state,
            set_watching,
            clear_duplicates,
            duplicate_action,
            set_duplicate_action,
            list_links,
            create_link,
            run_link,
            remove_link,
            set_at_once,
            archives,
            reset_storage,
            restore_archive,
            forget_archive,
            export_storage,
            import_storage,
            pick_save_file,
            pick_open_file,
            browse,
            places,
            last_panes,
            remember_panes,
            start_transfer,
            preview_transfer,
            preview_link,
            cancel_run,
            stop_now,
            resolve_conflict,
            resolve_identical,
            history,
            whereis,
            recent,
            quarantined,
            interrupted,
            resume_interrupted,
            discard_interrupted,
            list_connections,
            add_connection,
            update_connection,
            set_connection_password,
            test_connection,
            remove_connection,
        ])
        .run(tauri::generate_context!())
        .expect("the window could not start");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(source: &str, destination: &str) -> Leg {
        Leg {
            source: source.to_string(),
            destination: destination.to_string(),
            names: vec!["a.mp4".to_string()],
        }
    }

    fn request(legs: Vec<Leg>) -> TransferRequest {
        TransferRequest {
            legs,
            source_policy: "delete".to_string(),
            verify: "hash".to_string(),
            on_conflict: "quarantine".to_string(),
            order: None,
            cooldown_secs: None,
            save_as: None,
        }
    }

    /// Both ends on this machine, which is what every path-containment case
    /// below is about. Built by hand rather than through `resolve_route`: that
    /// one needs a journal, and none of these rules do.
    fn local(leg: &Leg) -> Route {
        Route {
            source: Endpoint::local(&leg.source),
            destination: Endpoint::local(&leg.destination),
            destination_scheme: None,
        }
    }

    /// A connection id nothing looks up. Only its identity matters here: what
    /// the guard compares is whether two ends are in the *same* place.
    fn place(n: i64) -> tungstate_journal::ConnectionId {
        tungstate_journal::ConnectionId(n)
    }

    #[test]
    fn an_ordinary_transfer_is_accepted() {
        let r = request(vec![leg("/tmp/from", "/tmp/to")]);
        assert!(plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_ok());
    }

    #[test]
    fn a_folder_cannot_drain_into_itself() {
        // Either nesting direction feeds the walk its own output.
        for (from, to) in [
            ("/tmp/videos", "/tmp/videos"),
            ("/tmp/videos", "/tmp/videos/archive"),
            ("/tmp/videos/archive", "/tmp/videos"),
        ] {
            let r = request(vec![leg(from, to)]);
            assert!(
                plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_err(),
                "`{from}` -> `{to}` should have been refused"
            );
        }
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_still_fine() {
        // starts_with on paths compares components, so `videos-old` is not
        // inside `videos`. A naive string prefix check would refuse this.
        let r = request(vec![leg("/tmp/videos", "/tmp/videos-old")]);
        assert!(plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_ok());
    }

    #[test]
    fn an_empty_selection_is_refused() {
        let mut r = request(vec![leg("/tmp/from", "/tmp/to")]);
        r.legs[0].names.clear();
        assert!(plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_err());
    }

    #[test]
    fn unknown_settings_are_refused_rather_than_guessed() {
        let mut r = request(vec![leg("/tmp/from", "/tmp/to")]);
        r.source_policy = "obliterate".to_string();
        assert!(plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_err());

        let mut r = request(vec![leg("/tmp/from", "/tmp/to")]);
        r.verify = "vibes".to_string();
        assert!(plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_err());
    }

    #[test]
    fn both_legs_of_an_exchange_are_validated() {
        // A bad second leg must be caught before the first creates anything.
        let r = request(vec![
            leg("/tmp/left", "/tmp/right"),
            leg("/tmp/right", "/tmp/right/inside"),
        ]);
        assert!(plan_transfer(&r, &r.legs[0], local(&r.legs[0])).is_ok());
        assert!(plan_transfer(&r, &r.legs[1], local(&r.legs[1])).is_err());
    }

    #[test]
    fn two_different_places_are_never_nested_in_each_other() {
        // `inbox` on the NAS is not inside `/Users/me/inbox`, and path
        // containment cannot tell. Refusing this pair would block the drain
        // the whole program exists for.
        let r = request(vec![leg("nas:inbox", "/Users/me/inbox")]);
        let route = Route {
            source: Endpoint::remote(place(1), "inbox"),
            destination: Endpoint::local("/Users/me/inbox"),
            destination_scheme: None,
        };
        assert!(plan_transfer(&r, &r.legs[0], route).is_ok());
    }

    #[test]
    fn one_place_can_still_drain_into_itself() {
        // The other half of the same rule: within one connection, containment
        // means exactly what it means locally.
        let r = request(vec![leg("nas:inbox", "nas:inbox/2026")]);
        let route = Route {
            source: Endpoint::remote(place(1), "inbox"),
            destination: Endpoint::remote(place(1), "inbox/2026"),
            destination_scheme: Some(Scheme::Ftp),
        };
        assert!(plan_transfer(&r, &r.legs[0], route).is_err());
    }

    #[test]
    fn originals_on_a_connection_cannot_go_to_the_trash() {
        // This desktop's trash knows nothing about a NAS, and there is no
        // remote trash yet. Said while the dialog is open, not at the first
        // file.
        let mut r = request(vec![leg("nas:inbox", "/Users/me/inbox")]);
        r.source_policy = "trash".to_string();
        let route = Route {
            source: Endpoint::remote(place(1), "inbox"),
            destination: Endpoint::local("/Users/me/inbox"),
            destination_scheme: None,
        };
        assert!(plan_transfer(&r, &r.legs[0], route).is_err());

        // The same policy with a local source is the ordinary case.
        let r = request(vec![leg("/Users/me/inbox", "nas:inbox")]);
        let route = Route {
            source: Endpoint::local("/Users/me/inbox"),
            destination: Endpoint::remote(place(1), "inbox"),
            destination_scheme: Some(Scheme::Ftp),
        };
        assert!(plan_transfer(&r, &r.legs[0], route).is_ok());
    }

    #[test]
    fn replace_is_refused_where_nothing_can_be_moved_aside() {
        // Replace keeps the file already there by renaming it first, and FTP
        // gives us no rename. The CLI refuses this at link creation; so does
        // the window, or the two surfaces disagree about the same link.
        let mut r = request(vec![leg("/Users/me/inbox", "nas:inbox")]);
        r.on_conflict = "replace".to_string();
        let route = Route {
            source: Endpoint::local("/Users/me/inbox"),
            destination: Endpoint::remote(place(1), "inbox"),
            destination_scheme: Some(Scheme::Ftp),
        };
        assert!(plan_transfer(&r, &r.legs[0], route).is_err());

        // A destination that can rename takes it.
        let route = Route {
            source: Endpoint::local("/Users/me/inbox"),
            destination: Endpoint::remote(place(1), "inbox"),
            destination_scheme: Some(Scheme::Fs),
        };
        assert!(plan_transfer(&r, &r.legs[0], route).is_ok());
    }

    /// A journal with one `fs` connection rooted at a real directory.
    ///
    /// `fs` because it is the scheme that needs no password: the whole point is
    /// to walk a *connection* rather than a local path, and no test in this
    /// workspace may touch the developer's keychain.
    fn journal_over(root: &std::path::Path) -> Journal {
        let journal = Journal::open_in_memory().unwrap();
        journal
            .create_connection(&tungstate_journal::NewConnection {
                name: "nas".to_string(),
                scheme: Scheme::Fs,
                host: None,
                port: None,
                username: None,
                root: root.display().to_string(),
                options: BTreeMap::new(),
            })
            .unwrap();
        journal
    }

    #[test]
    fn a_connection_root_lists_its_children_as_connection_paths() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("inbox")).unwrap();
        std::fs::write(home.path().join("a.mp4"), b"x").unwrap();
        let journal = journal_over(home.path());

        let listing = listing_for("nas:", &journal).unwrap();

        assert_eq!(listing.path, "nas:", "the root writes as a bare name");
        assert_eq!(
            listing.parent, None,
            "there is nothing above a connection's root, so Up is disabled"
        );
        // Folders first, then by name — and every child is a location the
        // pane can be handed straight back.
        let paths: Vec<&str> = listing.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["nas:inbox", "nas:a.mp4"]);
    }

    #[test]
    fn walking_into_a_connection_and_back_out_again_round_trips() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("inbox/2026")).unwrap();
        std::fs::write(home.path().join("inbox/2026/holiday.mp4"), b"x").unwrap();
        let journal = journal_over(home.path());

        // Down two levels, taking each child's own path as the next location,
        // which is exactly what a double-click in the pane does.
        let first_child = |at: &str| listing_for(at, &journal).unwrap().entries[0].path.clone();
        let inbox = first_child("nas:");
        assert_eq!(inbox, "nas:inbox");
        let year = first_child(&inbox);
        assert_eq!(year, "nas:inbox/2026");

        let deep = listing_for(&year, &journal).unwrap();
        assert_eq!(deep.entries[0].path, "nas:inbox/2026/holiday.mp4");

        // And back up, one Up-button click at a time, to the root and no
        // further.
        assert_eq!(deep.parent.as_deref(), Some("nas:inbox"));
        let up = listing_for(deep.parent.as_deref().unwrap(), &journal).unwrap();
        assert_eq!(up.parent.as_deref(), Some("nas:"));
        assert_eq!(listing_for("nas:", &journal).unwrap().parent, None);
    }

    #[test]
    fn a_local_pane_still_lists_native_paths() {
        // The other half: nothing about this slice may change what a local
        // pane shows, and a local child must read the way this machine spells
        // a path.
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("a.mp4"), b"x").unwrap();
        let journal = Journal::open_in_memory().unwrap();

        let listing = listing_for(&home.path().display().to_string(), &journal).unwrap();
        assert_eq!(
            listing.entries[0].path,
            home.path().join("a.mp4").display().to_string()
        );
        assert!(listing.parent.is_some(), "a temp dir has a parent");
    }

    #[test]
    fn a_remembered_folder_that_was_deleted_opens_at_its_parent() {
        // The pane used to open on "No such file or directory".
        let home = tempfile::tempdir().unwrap();
        let gone = home.path().join("ChatExport").join("video_files");
        let journal = Journal::open_in_memory().unwrap();

        assert_eq!(
            still_open(&gone.display().to_string(), &journal),
            Some(home.path().display().to_string())
        );
        assert_eq!(
            still_open(&home.path().display().to_string(), &journal),
            Some(home.path().display().to_string()),
            "a folder that is still there is left alone"
        );
    }

    #[test]
    fn a_remembered_connection_this_journal_does_not_have_is_dropped() {
        // Another journal's connection, or one since removed. Dropped, so the
        // pane opens on its ordinary default rather than on an error.
        let journal = Journal::open_in_memory().unwrap();
        assert_eq!(still_open("Area51:media", &journal), None);
    }

    #[test]
    fn browsing_a_connection_that_does_not_exist_says_which_one() {
        // The typo case. Answering with an empty listing, or with a local
        // directory literally named `nsa:inbox`, would both be worse than
        // saying the name is unknown.
        let journal = Journal::open_in_memory().unwrap();
        let error = listing_for("nsa:inbox", &journal).unwrap_err();
        assert!(error.contains("nsa"), "the message names the typo: {error}");
    }

    #[test]
    fn a_name_ticked_on_both_sides_is_reported() {
        // It would clash in both directions, and resolving it twice is rarely
        // what anyone meant.
        let mut r = request(vec![
            leg("/tmp/left", "/tmp/right"),
            leg("/tmp/right", "/tmp/left"),
        ]);
        r.legs[0].names = vec!["shared.mp4".into(), "onlyleft.mp4".into()];
        r.legs[1].names = vec!["shared.mp4".into(), "onlyright.mp4".into()];

        assert_eq!(overlapping_names(&r.legs), vec!["shared.mp4".to_string()]);
    }

    #[test]
    fn one_direction_has_nothing_to_overlap_with() {
        let r = request(vec![leg("/tmp/left", "/tmp/right")]);
        assert!(overlapping_names(&r.legs).is_empty());
    }

    // ---------------------------------------------------------------------
    // RunQueue: adding work to a transfer that is already going

    #[test]
    fn the_first_submission_starts_a_worker_and_later_ones_do_not() {
        let queue = RunQueue::new();
        assert!(
            queue.submit(["a"]),
            "nobody is running, so somebody must start"
        );
        assert!(
            !queue.submit(["b"]),
            "a worker exists, so a second one would race it over the same files"
        );
        assert_eq!(queue.waiting(), 2);

        assert_eq!(queue.next(), Some("a"));
        assert_eq!(queue.next(), Some("b"));
        // Draining is what ends the worker, and only then may one start again.
        assert_eq!(queue.next(), None);
        assert!(queue.submit(["c"]));
    }

    #[test]
    fn work_added_while_the_worker_is_mid_run_is_picked_up_without_a_new_worker() {
        let queue = RunQueue::new();
        assert!(queue.submit(["a"]));
        let first = queue.next();
        assert_eq!(first, Some("a"));

        // The worker is between files here: it has taken one and not yet asked
        // for another. This is the whole feature.
        assert!(!queue.submit(["b"]), "the running worker should take it");
        assert_eq!(queue.next(), Some("b"));
    }

    #[test]
    fn a_stop_empties_the_queue_but_leaves_the_worker_to_finish_reporting() {
        let queue = RunQueue::new();
        queue.submit(["a", "b", "c"]);
        assert_eq!(queue.next(), Some("a"));
        queue.clear();
        assert_eq!(queue.waiting(), 0);
        // Still the worker until it says otherwise, so a Move arriving now
        // does not start a second one alongside the one winding down.
        assert!(!queue.submit(["d"]));
    }

    #[test]
    fn standing_down_leaves_the_work_for_whoever_comes_next() {
        // The panic path. Losing the queue would be worse than running it
        // late, and the flag has to clear or the window refuses everything
        // for the rest of the session.
        let queue = RunQueue::new();
        queue.submit(["a", "b"]);
        queue.next();
        queue.stand_down();

        assert_eq!(queue.waiting(), 1);
        assert!(queue.submit(["c"]), "a new worker is needed");
        assert_eq!(queue.next(), Some("b"));
        assert_eq!(queue.next(), Some("c"));
    }

    #[test]
    fn no_submission_is_ever_accepted_by_nobody() {
        // The race the lock exists for. A worker deciding it is finished and a
        // submission deciding somebody is already running must never both
        // conclude "not my problem", or that work sits there for ever.
        //
        // Run as a real contest rather than as an argument: submitters hammer
        // a worker that is constantly draining to empty, and every item has to
        // come out exactly once.
        const SUBMITTERS: usize = 4;
        const EACH: usize = 250;

        let queue: Arc<RunQueue<usize>> = Arc::new(RunQueue::new());
        let taken = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicBool::new(false));

        std::thread::scope(|scope| {
            for worker in 0..SUBMITTERS {
                let queue = Arc::clone(&queue);
                let taken = Arc::clone(&taken);
                let done = Arc::clone(&done);
                scope.spawn(move || {
                    for i in 0..EACH {
                        let item = worker * EACH + i;
                        if queue.submit([item]) {
                            // We are the worker. Drain until empty, exactly as
                            // the real one does, then stop being the worker.
                            while let Some(got) = queue.next() {
                                lock(&taken).push(got);
                            }
                        }
                    }
                    if worker == 0 {
                        done.store(true, Ordering::SeqCst);
                    }
                });
            }
        });

        // Whatever the interleaving left behind belongs to nobody now, so
        // drain it the way a fresh submission would.
        if queue.submit(std::iter::empty()) {
            while let Some(got) = queue.next() {
                lock(&taken).push(got);
            }
        }
        assert!(done.load(Ordering::SeqCst));

        let mut got = lock(&taken).clone();
        got.sort_unstable();
        let expected: Vec<usize> = (0..SUBMITTERS * EACH).collect();
        assert_eq!(got, expected, "every submission must be taken exactly once");
    }
}
