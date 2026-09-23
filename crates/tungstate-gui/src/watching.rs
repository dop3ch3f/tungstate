//! The window keeping folders in order while it is open.
//!
//! The loop is `tungstate_watch`. This is the part that belongs to a window:
//! one background thread, a switch that is remembered, and a short record of
//! what happened so the person can see it rather than being told to go and
//! read History.
//!
//! **While the window is open.** There is no background service until slice
//! 10, and the screen says so beside the switch rather than leaving somebody
//! to work it out when their laptop files nothing overnight.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tungstate_backend::Backend as _;
use tungstate_backend::local::LocalBackend;
use tungstate_journal::Journal;
use tungstate_watch::{Noticed, Stop, Watched};

/// Where the answer to "watch while open?" is kept.
pub const SETTING: &str = "watch.enabled";

/// How many things it has done are worth keeping on screen.
const KEPT: usize = 12;

/// One thing the watcher did, as the window draws it.
#[derive(Debug, Clone, Serialize)]
pub struct NoticeView {
    /// Which of the things that can happen this is.
    pub kind: &'static str,
    /// The folder it concerns, empty for the opening line.
    pub folder: String,
    /// Files involved, where that means anything.
    pub files: usize,
    /// The plan, so it can be undone. Zero when there is nothing to undo.
    pub plan: i64,
    /// What went wrong, for trouble.
    pub why: String,
    /// When, as milliseconds since the epoch, so the window can say "just now".
    pub at: i64,
}

/// What the window knows about the watcher.
#[derive(Debug, Clone, Serialize)]
pub struct WatchView {
    /// Whether watching is turned on.
    pub on: bool,
    /// Whether a loop is actually running right now.
    pub running: bool,
    /// Folders being watched for events.
    pub watching: usize,
    /// Folders that only get the periodic sweep, because they are on a share.
    pub sweeping: usize,
    /// What has happened since the window opened, newest first.
    pub recent: Vec<NoticeView>,
}

/// The watcher's state, for the length of one window.
#[derive(Debug, Default)]
pub struct Watching {
    /// Asked to stop. Replaced every time watching starts.
    stop: Mutex<Option<Stop>>,
    /// What has happened, newest last.
    seen: Arc<Mutex<Vec<NoticeView>>>,
    /// How many folders are in each camp, from the opening line.
    counts: Arc<Mutex<(usize, usize)>>,
}

impl Watching {
    /// Whether a loop is running.
    #[must_use]
    pub fn running(&self) -> bool {
        self.stop
            .lock()
            .is_ok_and(|held| held.as_ref().is_some_and(|stop| !stop.asked()))
    }

    /// Stop whatever is running.
    pub fn halt(&self) {
        if let Ok(mut held) = self.stop.lock()
            && let Some(stop) = held.take()
        {
            stop.ask();
        }
    }

    /// What the window draws.
    #[must_use]
    pub fn view(&self, on: bool) -> WatchView {
        let (watching, sweeping) = self.counts.lock().map(|held| *held).unwrap_or_default();
        let mut recent = self
            .seen
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default();
        recent.reverse();
        WatchView {
            on,
            running: self.running(),
            watching,
            sweeping,
            recent,
        }
    }

    /// Remember one thing, and keep the list short.
    ///
    /// *Waiting* and *trouble* are standing conditions rather than events:
    /// the same folder still has the same files waiting, or its rules still
    /// will not load. Each new one replaces the last for that folder, or the
    /// list fills with one copy of the same sentence per sweep. A *tidied*
    /// is a real event and stacks, because two of them are two sets of files.
    fn remember(&self, notice: NoticeView) {
        if let Ok(mut held) = self.seen.lock() {
            if matches!(notice.kind, "waiting" | "trouble") {
                held.retain(|seen| !(seen.kind == notice.kind && seen.folder == notice.folder));
            }
            held.push(notice);
            let over = held.len().saturating_sub(KEPT);
            held.drain(..over);
        }
    }
}

/// Whether watching is turned on. On unless somebody said otherwise.
///
/// # Errors
/// [`tungstate_journal::JournalError`] as a sentence, if the row cannot be read.
pub fn wanted(journal: &Journal) -> Result<bool, String> {
    Ok(journal
        .setting(SETTING)
        .map_err(|error| error.to_string())?
        .is_none_or(|value| value != "off"))
}

/// Remember whether watching is turned on.
///
/// # Errors
/// [`tungstate_journal::JournalError`] as a sentence, if the row cannot be written.
pub fn remember(journal: &Journal, on: bool) -> Result<(), String> {
    journal
        .remember_setting(SETTING, if on { "on" } else { "off" })
        .map_err(|error| error.to_string())
}

/// Every governed folder, ready to be watched.
///
/// # Errors
/// [`tungstate_journal::JournalError`] as a sentence, if the folders cannot be read.
pub fn folders(journal: &Journal) -> Result<Vec<Watched>, String> {
    Ok(journal
        .folders()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|folder| {
            let root = PathBuf::from(&folder.root);
            Watched {
                name: folder.name,
                // A share says nothing about a change another machine made, so
                // it is swept rather than watched.
                networked: LocalBackend::new(root.clone()).capabilities().networked,
                root,
            }
        })
        .collect())
}

/// Turn one thing that happened into what the window draws, and what it says.
#[must_use]
pub fn as_view(noticed: &Noticed, at: i64) -> NoticeView {
    let mut view = NoticeView {
        kind: "settled",
        folder: String::new(),
        files: 0,
        plan: 0,
        why: String::new(),
        at,
    };
    match noticed {
        Noticed::Started { watching, sweeping } => {
            view.kind = "started";
            view.files = *watching;
            view.plan = i64::try_from(*sweeping).unwrap_or(0);
        }
        Noticed::Tidied {
            folder,
            files,
            plan,
        } => {
            view.kind = "tidied";
            view.folder.clone_from(folder);
            view.files = *files;
            view.plan = *plan;
        }
        Noticed::Waiting { folder, files } => {
            view.kind = "waiting";
            view.folder.clone_from(folder);
            view.files = *files;
        }
        Noticed::Settled { folder } => {
            view.folder.clone_from(folder);
        }
        Noticed::Trouble { folder, why } => {
            view.kind = "trouble";
            view.folder.clone_from(folder);
            view.why.clone_from(why);
        }
    }
    view
}

/// Start watching, stopping anything already running.
///
/// Spawns one thread and returns at once. `told` hears everything, which is
/// how the event reaches the window.
pub fn start(
    state: &Arc<Watching>,
    journal: Journal,
    mut told: impl FnMut(&NoticeView) + Send + 'static,
) {
    state.halt();
    let Ok(folders) = folders(&journal) else {
        return;
    };
    if folders.is_empty() {
        return;
    }
    let stop = Stop::new();
    if let Ok(mut held) = state.stop.lock() {
        *held = Some(stop.clone());
    }
    let here = Arc::clone(state);
    std::thread::spawn(move || {
        let sweep = std::time::Duration::from_secs(3600);
        let _ = tungstate_watch::watch(&folders, &journal, sweep, &stop, &mut |noticed| {
            let view = as_view(noticed, now_millis());
            if let Noticed::Started { watching, sweeping } = noticed
                && let Ok(mut counts) = here.counts.lock()
            {
                *counts = (*watching, *sweeping);
            }
            // The opening line is state, not news; everything else is both.
            if !matches!(noticed, Noticed::Started { .. } | Noticed::Settled { .. }) {
                here.remember(view.clone());
            }
            told(&view);
        });
    });
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}
