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
        })
        .map(|_| ())
        .map_err(describe)
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

    let link = match state.journal.link_by_name(&name) {
        Ok(link) => link,
        Err(error) => {
            state.running.store(false, Ordering::SeqCst);
            return Err(describe(error));
        }
    };

    state.cancel.store(false, Ordering::Relaxed);
    let replies = state.conflicts.open();
    let cancel = Arc::clone(&state.cancel);

    let worker = app.clone();
    // The engine is synchronous and will block for as long as the drain takes.
    // Its own thread keeps the window responsive throughout.
    std::thread::spawn(move || {
        let state = worker.state::<App>();
        let source = LocalBackend::new(link.source_root.clone());
        let destination = LocalBackend::new(link.destination_root.clone());
        let mut resolver = WindowResolver::new(worker.clone(), replies, link.on_conflict);
        let mut progress = EventProgress::new(worker.clone());

        let outcome = Transfer::new(
            &link,
            &source,
            &destination,
            &state.journal,
            &mut resolver,
            &mut progress,
        )
        .cancellable(cancel)
        .run();

        state.conflicts.close();
        state.running.store(false, Ordering::SeqCst);

        let _ = match outcome {
            Ok(summary) => worker.emit("transfer://done", SummaryView::from(&summary)),
            Err(error) => worker.emit("transfer://error", describe(error)),
        };
    });

    Ok(())
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
