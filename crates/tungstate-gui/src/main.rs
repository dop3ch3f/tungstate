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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tungstate_backend::Backend as _;
use tungstate_backend::local::LocalBackend;
use tungstate_journal::{
    ConflictAction, Journal, Link, Locator, NewLink, Op, OpStatus, Order, SourcePolicy, VerifyLevel,
};
use tungstate_transfer::{Summary, Transfer};

use bridge::{ConflictChannel, EventProgress, Reply, WindowResolver};

/// Shared state for the whole window.
struct App {
    journal: Journal,
    conflicts: ConflictChannel,
    cancel: Arc<AtomicBool>,
    running: AtomicBool,
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

impl From<&Link> for LinkView {
    fn from(link: &Link) -> Self {
        Self {
            name: link.name.clone(),
            source: link.source_root.display().to_string(),
            destination: link.destination_root.display().to_string(),
            source_policy: link.source_policy.as_str().to_string(),
            verify: link.verify.as_str().to_string(),
            order: link.order.as_str().to_string(),
            on_conflict: link.on_conflict.as_str().to_string(),
            cooldown_secs: link.cooldown.as_secs(),
        }
    }
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

impl From<&Op> for OpView {
    fn from(op: &Op) -> Self {
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
            source: op.source.as_ref().map(|l| l.full().display().to_string()),
            destination: op
                .destination
                .as_ref()
                .map(|l| l.full().display().to_string()),
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

#[tauri::command]
fn list_links(state: State<'_, App>) -> Result<Vec<LinkView>, String> {
    state
        .journal
        .links()
        .map(|links| links.iter().map(LinkView::from).collect())
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

    let source = PathBuf::from(&form.source);
    let destination = PathBuf::from(&form.destination);

    // Catching this here rather than at the first file means the mistake is
    // visible while the form is still on screen.
    if !source.is_dir() {
        return Err(format!("{} is not a folder", source.display()));
    }
    if source == destination {
        return Err("source and destination are the same folder".to_string());
    }
    if destination.starts_with(&source) {
        return Err(
            "the destination is inside the source, which would drain into itself".to_string(),
        );
    }

    state
        .journal
        .create_link(&NewLink {
            name: form.name,
            source_root: source,
            destination_root: destination,
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
fn last_panes() -> PaneState {
    pane_state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
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
fn places() -> Vec<Place> {
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
            // The boot volume is already reachable as /; listing it as a
            // "volume" only invites someone to drain their own system disk.
            .filter(|e| !std::path::Path::new("/").join(e.file_name()).exists())
            .map(|e| Place {
                label: e.file_name().to_string_lossy().into_owned(),
                path: e.path().display().to_string(),
            })
            .collect();
        mounted.sort_by(|a, b| a.label.cmp(&b.label));
        places.append(&mut mounted);
    }
    places
}

fn directories_home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// List a directory for one side of the browser.
#[tauri::command]
fn browse(path: String) -> Result<Listing, String> {
    let root = PathBuf::from(&path);
    // A backend rooted at the directory being shown, so the same path rules that
    // protect a transfer also apply to browsing. Constructing one does no I/O.
    let backend = LocalBackend::new(root.clone());

    let mut entries: Vec<EntryView> = backend
        .read_dir(std::path::Path::new(""))
        .map_err(describe)?
        .into_iter()
        .filter(|entry| !entry.meta.is_symlink)
        .map(|entry| {
            let name = entry.path.display().to_string();
            EntryView {
                path: root.join(&entry.path).display().to_string(),
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
        parent: root.parent().map(|p| p.display().to_string()),
        path: root.display().to_string(),
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
    /// When set, the pair is remembered and can be run again later.
    save_as: Option<String>,
}

/// The settings a validated transfer request resolves to.
struct Plan {
    source: PathBuf,
    destination: PathBuf,
    source_policy: SourcePolicy,
    verify: VerifyLevel,
    on_conflict: ConflictAction,
}

/// Check a request from the window before anything touches the disk.
///
/// Kept separate from the command so it can be tested. The nesting check is the
/// one that matters: draining a folder into its own subfolder would feed the
/// walk its own output.
fn plan_transfer(request: &TransferRequest, leg: &Leg) -> std::result::Result<Plan, String> {
    let source_policy = SourcePolicy::parse(&request.source_policy)
        .ok_or("choose what happens to the originals")?;
    let verify = VerifyLevel::parse(&request.verify).ok_or("unknown verification level")?;
    let on_conflict =
        ConflictAction::parse(&request.on_conflict).ok_or("unknown conflict action")?;

    let source = PathBuf::from(&leg.source);
    let destination = PathBuf::from(&leg.destination);

    if leg.names.is_empty() {
        return Err("nothing is selected".to_string());
    }
    if source == destination {
        return Err("those are the same folder".to_string());
    }
    if destination.starts_with(&source) {
        return Err(
            "the destination is inside the source, which would copy into itself".to_string(),
        );
    }
    if source.starts_with(&destination) {
        return Err(
            "the source is inside the destination, which would copy into itself".to_string(),
        );
    }

    Ok(Plan {
        source,
        destination,
        source_policy,
        verify,
        on_conflict,
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

#[tauri::command]
fn preview_transfer(request: TransferRequest) -> Result<PreviewView, String> {
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
        let plan = plan_transfer(&request, leg)?;

        // A throwaway link carrying the chosen settings. Nothing is stored: a
        // preview must not leave a trace any more than it moves a file.
        let link = Link {
            id: tungstate_journal::LinkId(0),
            name: String::from("preview"),
            source_root: plan.source.clone(),
            destination_root: plan.destination.clone(),
            source_policy: plan.source_policy,
            verify: plan.verify,
            order: Order::LargestFirst,
            on_conflict: plan.on_conflict,
            cooldown: Duration::ZERO,
            saved: false,
        };

        let source = LocalBackend::new(plan.source);
        let destination = LocalBackend::new(plan.destination);
        let names: Vec<PathBuf> = leg.names.iter().map(PathBuf::from).collect();
        let preview = tungstate_transfer::preview(&link, &source, &destination, Some(&names))
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
    if request.legs.is_empty() {
        return Err("nothing is selected".to_string());
    }
    // Validate every leg before creating anything, so a bad second leg cannot
    // leave a half-configured exchange behind.
    let plans: Vec<Plan> = request
        .legs
        .iter()
        .map(|leg| plan_transfer(&request, leg))
        .collect::<std::result::Result<_, _>>()?;

    // Saving only makes sense for a single direction: a link is one source and
    // one destination, and an exchange is two of them.
    let saved = request.save_as.is_some() && plans.len() == 1;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());

    let state = app.state::<App>();
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
                source_root: plan.source.clone(),
                destination_root: plan.destination.clone(),
                source_policy: plan.source_policy,
                verify: plan.verify,
                order: Order::LargestFirst,
                on_conflict: plan.on_conflict,
                // The user is looking at these files and chose them, so there is
                // no reason to hold back something written moments ago.
                cooldown: Duration::ZERO,
                saved,
            })
            .map_err(describe)?;
        names.push(name);
    }

    let selections: Vec<Vec<PathBuf>> = request
        .legs
        .iter()
        .map(|leg| leg.names.iter().map(PathBuf::from).collect())
        .collect();

    spawn_run(&app, names, selections)?;
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
        .history(std::path::Path::new(&path))
        .map(|ops| ops.iter().map(OpView::from).collect())
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
        .map(|ops| ops.iter().map(OpView::from).collect())
        .map_err(describe)
}

#[tauri::command]
fn recent(state: State<'_, App>) -> Result<Vec<OpView>, String> {
    state
        .journal
        .recent(200)
        .map(|ops| ops.iter().map(OpView::from).collect())
        .map_err(describe)
}

/// Files sitting in a link's quarantine folder, waiting for a decision.
#[tauri::command]
fn quarantined(link: String, state: State<'_, App>) -> Result<Vec<String>, String> {
    let link = state.journal.link_by_name(&link).map_err(describe)?;
    let root = link.destination_root.join(".tungstate-quarantine");
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut found = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(relative) = path.strip_prefix(&root) {
                found.push(relative.display().to_string());
            }
        }
    }
    found.sort();
    Ok(found)
}

#[tauri::command]
fn cancel_run(state: State<'_, App>) {
    state.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn resolve_conflict(
    action: String,
    apply_to_all: bool,
    state: State<'_, App>,
) -> Result<(), String> {
    let action = ConflictAction::parse(&action).ok_or("that conflict action is not one I know")?;
    if state.conflicts.reply(Reply {
        action,
        apply_to_all,
    }) {
        Ok(())
    } else {
        Err("nothing is waiting for that answer any more".to_string())
    }
}

#[tauri::command]
fn run_link(name: String, app: AppHandle) -> Result<(), String> {
    spawn_run(&app, vec![name], vec![Vec::new()])
}

/// Run each leg in turn on one worker thread, reporting a single combined result.
///
/// Sequential rather than parallel: two legs of an exchange can touch the same
/// names, and running them at once would race. Each leg is its own link, so each
/// is journaled and resumable on its own terms.
fn spawn_run(
    app: &AppHandle,
    links: Vec<String>,
    selections: Vec<Vec<PathBuf>>,
) -> Result<(), String> {
    let state = app.state::<App>();

    // compare_exchange rather than load-then-store: two rapid clicks on Run
    // would otherwise both see "not running" and start two drains over the same
    // files.
    if state
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("a transfer is already running".to_string());
    }

    let mut resolved = Vec::new();
    for name in &links {
        match state.journal.link_by_name(name) {
            Ok(link) => resolved.push(link),
            Err(error) => {
                state.running.store(false, Ordering::SeqCst);
                return Err(describe(error));
            }
        }
    }

    state.cancel.store(false, Ordering::Relaxed);
    let replies = state.conflicts.open();
    let cancel = Arc::clone(&state.cancel);

    let worker = app.clone();
    // The engine is synchronous and will block for as long as the drain takes.
    // Its own thread keeps the window responsive throughout.
    std::thread::spawn(move || {
        let state = worker.state::<App>();
        let fallback = resolved[0].on_conflict;
        let mut resolver = WindowResolver::new(worker.clone(), replies, fallback);
        let mut progress = EventProgress::new(worker.clone());
        let mut total = Summary::default();
        let mut failure = None;

        for (index, link) in resolved.iter().enumerate() {
            let source = LocalBackend::new(link.source_root.clone());
            let destination = LocalBackend::new(link.destination_root.clone());
            let chosen = selections.get(index).cloned().unwrap_or_default();

            let mut transfer = Transfer::new(
                link,
                &source,
                &destination,
                &state.journal,
                &mut resolver,
                &mut progress,
            )
            .cancellable(Arc::clone(&cancel));

            let outcome = if chosen.is_empty() {
                transfer.run()
            } else {
                transfer.run_selection(&chosen)
            };

            match outcome {
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

        state.conflicts.close();
        state.running.store(false, Ordering::SeqCst);

        let _ = match failure {
            Some(message) => worker.emit("transfer://error", message),
            None => worker.emit("transfer://done", SummaryView::from(&total)),
        };
    });

    Ok(())
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
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let journal = match Journal::open_default() {
        Ok(journal) => journal,
        Err(error) => {
            // No journal means no history and no crash recovery, so there is
            // nothing safe to do. Fail loudly rather than half-working.
            eprintln!("tungstate could not open its journal: {}", describe(error));
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(App {
            journal,
            conflicts: ConflictChannel::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            running: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            list_links,
            create_link,
            run_link,
            browse,
            places,
            last_panes,
            remember_panes,
            start_transfer,
            preview_transfer,
            cancel_run,
            resolve_conflict,
            history,
            whereis,
            recent,
            quarantined,
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
            save_as: None,
        }
    }

    #[test]
    fn an_ordinary_transfer_is_accepted() {
        let r = request(vec![leg("/tmp/from", "/tmp/to")]);
        assert!(plan_transfer(&r, &r.legs[0]).is_ok());
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
                plan_transfer(&r, &r.legs[0]).is_err(),
                "`{from}` -> `{to}` should have been refused"
            );
        }
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_still_fine() {
        // starts_with on paths compares components, so `videos-old` is not
        // inside `videos`. A naive string prefix check would refuse this.
        let r = request(vec![leg("/tmp/videos", "/tmp/videos-old")]);
        assert!(plan_transfer(&r, &r.legs[0]).is_ok());
    }

    #[test]
    fn an_empty_selection_is_refused() {
        let mut r = request(vec![leg("/tmp/from", "/tmp/to")]);
        r.legs[0].names.clear();
        assert!(plan_transfer(&r, &r.legs[0]).is_err());
    }

    #[test]
    fn unknown_settings_are_refused_rather_than_guessed() {
        let mut r = request(vec![leg("/tmp/from", "/tmp/to")]);
        r.source_policy = "obliterate".to_string();
        assert!(plan_transfer(&r, &r.legs[0]).is_err());

        let mut r = request(vec![leg("/tmp/from", "/tmp/to")]);
        r.verify = "vibes".to_string();
        assert!(plan_transfer(&r, &r.legs[0]).is_err());
    }

    #[test]
    fn both_legs_of_an_exchange_are_validated() {
        // A bad second leg must be caught before the first creates anything.
        let r = request(vec![
            leg("/tmp/left", "/tmp/right"),
            leg("/tmp/right", "/tmp/right/inside"),
        ]);
        assert!(plan_transfer(&r, &r.legs[0]).is_ok());
        assert!(plan_transfer(&r, &r.legs[1]).is_err());
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
}
