//! Syncs in the window: making and changing one, previewing a run, running it
//! where it can be watched and stopped, and putting a run back.
//!
//! The thinking is `tungstate_sync`; this is the seam around it. Everything
//! crosses as data (docs/SEAM.md rule 1): counts as numbers, reasons as kinds,
//! members by name. The window picks the words.
//!
//! A conflict is known before anything moves, so it is asked about in the
//! preview rather than mid-copy: the person's answers come back in [`Ask`]
//! and go into the run as `sync resolve` would put them.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tungstate_core::sync::{self as core, Removal, Removed, SyncOp, Why};
use tungstate_journal::{Journal, Launch, PlanId, Sync, SyncDirection, SyncSettings, ends};
use tungstate_sync::setup::{self, Refused};
use tungstate_sync::{Decided, Opened, Refusal};
use tungstate_transfer::{Progress, RunShape, Stop};

use crate::bridge::EventProgress;
use crate::{App, RunQueue, lock};

/// A sync as its row and its page draw it.
#[derive(Debug, Clone, Serialize)]
pub struct SyncView {
    pub name: String,
    /// `push`, `pull` or `all`.
    pub direction: &'static str,
    pub exact: bool,
    /// The anchor's member name, for `push` and `pull`.
    pub anchor: Option<String>,
    pub on_conflict: &'static str,
    pub on_remove: &'static str,
    pub verify: &'static str,
    pub cooldown_secs: u64,
    pub first_check: &'static str,
    /// `no`, `ask` or `quietly`.
    pub launch: &'static str,
    pub members: Vec<MemberView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberView {
    pub name: String,
    /// `~/CapCut` or `nas:capcut`.
    pub at: String,
    /// Reached through a connection rather than a folder on this machine.
    pub remote: bool,
}

impl SyncView {
    pub fn of(sync: &Sync, journal: &Journal) -> Self {
        let name_of = |id| {
            sync.members
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.name.clone())
        };
        Self {
            name: sync.name.clone(),
            direction: sync.direction.as_str(),
            exact: sync.exact,
            anchor: sync.anchor.and_then(name_of),
            on_conflict: sync.on_conflict.as_str(),
            on_remove: sync.on_remove.as_str(),
            verify: sync.verify.as_str(),
            cooldown_secs: sync.cooldown.as_secs(),
            first_check: sync.first_check.as_str(),
            launch: sync.launch.as_str(),
            members: sync
                .members
                .iter()
                .map(|m| MemberView {
                    name: m.name.clone(),
                    at: where_is(m, journal),
                    remote: m.connection.is_some(),
                })
                .collect(),
        }
    }
}

fn where_is(member: &tungstate_journal::Member, journal: &Journal) -> String {
    let end = match member.connection {
        Some(c) => tungstate_journal::Endpoint::remote(c, std::path::PathBuf::from(&member.path)),
        None => tungstate_journal::Endpoint::local(&member.path),
    };
    ends::describe(&end, journal)
}

/// Every sync, in the order they were made.
pub fn list(journal: &Journal) -> Result<Vec<SyncView>, String> {
    Ok(journal
        .syncs()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|sync| SyncView::of(sync, journal))
        .collect())
}

/// Why making or changing a sync was refused, as data. Mirrors
/// `tungstate_sync::setup::Refused`; the window words it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefusedView {
    DeleteNeedsExact,
    ReplaceHasNoMeaning,
    Unknown {
        setting: &'static str,
        value: String,
    },
    AllHasNoAnchor,
    NoSuchAnchor {
        member: String,
    },
    MoreNamesThanFolders,
    TooFew,
    NotAFolder {
        folder: String,
    },
    BadEnd {
        folder: String,
        why: String,
    },
    SameName {
        member: String,
    },
    Overlap {
        first: String,
        second: String,
    },
    Taken {
        name: String,
    },
    /// Anything else, already worded by whoever refused.
    Other {
        why: String,
    },
}

impl From<Refused> for RefusedView {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::DeleteNeedsExact => Self::DeleteNeedsExact,
            Refused::ReplaceHasNoMeaning => Self::ReplaceHasNoMeaning,
            Refused::Unknown { setting, value } => Self::Unknown { setting, value },
            Refused::AllHasNoAnchor => Self::AllHasNoAnchor,
            Refused::NoSuchAnchor(member) => Self::NoSuchAnchor { member },
            Refused::MoreNamesThanFolders => Self::MoreNamesThanFolders,
            Refused::TooFew => Self::TooFew,
            Refused::NotAFolder(folder) => Self::NotAFolder { folder },
            Refused::BadEnd { raw, why } => Self::BadEnd { folder: raw, why },
            Refused::SameName(member) => Self::SameName { member },
            Refused::Overlap(first, second) => Self::Overlap { first, second },
            Refused::Taken(name) => Self::Taken { name },
            Refused::Journal(why) => Self::Other { why },
        }
    }
}

fn other(why: impl ToString) -> RefusedView {
    RefusedView::Other {
        why: why.to_string(),
    }
}

/// The settings as the window's form sends them, spelled as the engine
/// spells them.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsForm {
    pub exact: bool,
    pub on_conflict: String,
    pub on_remove: String,
    pub verify: String,
    pub cooldown_secs: u64,
    pub first_check: String,
    /// `no`, `ask` or `quietly`.
    pub launch: String,
}

impl SettingsForm {
    fn checked(&self) -> Result<(SyncSettings, Launch), RefusedView> {
        let settings = setup::settings(
            self.exact,
            &self.on_conflict,
            &self.on_remove,
            &self.verify,
            self.cooldown_secs,
            &self.first_check,
        )?;
        let launch = Launch::parse(&self.launch).ok_or_else(|| RefusedView::Unknown {
            setting: "launch",
            value: self.launch.clone(),
        })?;
        Ok((settings, launch))
    }
}

/// A new sync, as the window's form sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct NewSyncForm {
    pub name: String,
    /// Each a folder here, or `connection:path`.
    pub ends: Vec<String>,
    /// A name per member; an empty one is chosen from the folder.
    pub names: Vec<String>,
    /// `push`, `pull` or `all`.
    pub direction: String,
    pub anchor: Option<String>,
    pub settings: SettingsForm,
}

pub fn make(journal: &Journal, form: &NewSyncForm) -> Result<SyncView, RefusedView> {
    let direction = SyncDirection::parse(&form.direction).ok_or_else(|| RefusedView::Unknown {
        setting: "direction",
        value: form.direction.clone(),
    })?;
    let (settings, launch) = form.settings.checked()?;
    let sync = setup::create(
        journal,
        &setup::Setup {
            name: form.name.trim().to_string(),
            ends: form.ends.clone(),
            names: form.names.clone(),
            direction,
            anchor: form
                .anchor
                .clone()
                .filter(|_| direction != SyncDirection::All),
            settings,
        },
    )?;
    journal.set_launch(sync.id, launch).map_err(other)?;
    let sync = journal.sync_by_id(sync.id).map_err(other)?;
    Ok(SyncView::of(&sync, journal))
}

pub fn change(journal: &Journal, name: &str, form: &SettingsForm) -> Result<SyncView, RefusedView> {
    let sync = journal.sync_by_name(name).map_err(other)?;
    let (settings, launch) = form.checked()?;
    journal.update_sync(sync.id, &settings).map_err(other)?;
    journal.set_launch(sync.id, launch).map_err(other)?;
    let sync = journal.sync_by_id(sync.id).map_err(other)?;
    Ok(SyncView::of(&sync, journal))
}

pub fn add_member(
    journal: &Journal,
    name: &str,
    end: &str,
    called: Option<&str>,
) -> Result<SyncView, RefusedView> {
    let sync = journal.sync_by_name(name).map_err(other)?;
    setup::add_member(journal, &sync, end, called)?;
    let sync = journal.sync_by_id(sync.id).map_err(other)?;
    Ok(SyncView::of(&sync, journal))
}

pub fn remove_member(journal: &Journal, name: &str, member: &str) -> Result<SyncView, String> {
    let sync = journal.sync_by_name(name).map_err(|e| e.to_string())?;
    journal
        .remove_member(sync.id, member)
        .map_err(|e| e.to_string())?;
    let sync = journal.sync_by_id(sync.id).map_err(|e| e.to_string())?;
    Ok(SyncView::of(&sync, journal))
}

/// What a person has said about particular paths before a run.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Ask {
    /// Conflicts settled in the preview.
    #[serde(default)]
    pub resolve: Vec<Settled>,
    /// Files or folders to take off every member for good.
    #[serde(default)]
    pub forget: Vec<String>,
    /// Decide only these paths.
    #[serde(default)]
    pub only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settled {
    pub path: String,
    /// The member whose version goes everywhere, or `None` for every version,
    /// each named after its member.
    pub keep: Option<String>,
}

impl Ask {
    fn into_core(self, opened: &Opened) -> Result<core::Asked, String> {
        let mut resolve = BTreeMap::new();
        for settled in self.resolve {
            let how = match settled.keep {
                None => core::Resolve::KeepBoth,
                Some(name) => core::Resolve::Keep(
                    opened
                        .places
                        .iter()
                        .find(|p| p.member.name == name)
                        .ok_or_else(|| {
                            format!("`{}` has no member called `{name}`", opened.sync.name)
                        })?
                        .member
                        .id
                        .0,
                ),
            };
            resolve.insert(settled.path, how);
        }
        Ok(core::Asked {
            resolve,
            forget: self
                .forget
                .into_iter()
                .map(|path| {
                    path.replace('\\', "/")
                        .trim_start_matches("./")
                        .trim_end_matches('/')
                        .to_string()
                })
                .filter(|path| !path.is_empty())
                .collect(),
            only: self.only,
        })
    }
}

/// A run, decided and not yet carried out: everything the preview draws.
#[derive(Debug, Clone, Serialize)]
pub struct SyncPreviewView {
    pub sync: String,
    /// What `run_sync` is given back, so it can refuse a run that has
    /// changed since this was looked at.
    pub fingerprint: String,
    /// Nothing would move and nothing needs remembering.
    pub empty: bool,
    /// False when the run would delete anything outright.
    pub reversible: bool,
    pub members: Vec<MemberPreview>,
    pub legs: Vec<LegView>,
    pub read: ReadView,
    /// What would be taken off, file by file. Replaced versions are counted
    /// on their member, not listed: the file is still there, newer.
    pub removed: Vec<RemovedView>,
    pub conflicts: Vec<ConflictView>,
    pub left_alone: Vec<LeftAloneView>,
    pub refusals: Vec<RefusalView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberPreview {
    pub name: String,
    pub at: String,
    pub arriving: usize,
    pub arriving_bytes: u64,
    pub leaving: usize,
    pub replacing: usize,
    pub removing: usize,
    pub deleting: usize,
    pub renaming: usize,
    pub parking: usize,
    pub holds: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegView {
    pub from: String,
    pub to: String,
    pub files: usize,
    pub parked: usize,
    pub bytes: u64,
    /// Between two members elsewhere, so the bytes come through this machine.
    pub through_here: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadView {
    pub files: usize,
    pub sampled: usize,
    pub no_times: usize,
    pub moves: usize,
    pub parked: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemovedView {
    pub member: String,
    pub path: String,
    /// `deleted` (another member deleted it), `extra` (the promise says it
    /// must not be there) or `forgotten`.
    pub because: &'static str,
    /// Deleted outright rather than set aside.
    pub gone: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictView {
    pub path: String,
    pub versions: Vec<VersionView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionView {
    pub member: String,
    pub size: u64,
    /// Milliseconds, as that member reports it. Shown, never compared across
    /// members: their clocks do not agree.
    pub modified: Option<i64>,
}

/// A path left alone, and why, as data.
#[derive(Debug, Clone, Serialize)]
pub struct LeftAloneView {
    pub path: String,
    /// `too_recent`, `only_anchor_sends`, `only_anchor_receives`, `conflict`,
    /// `case_clash` or `needs_rename`.
    pub why: &'static str,
    /// The members the reason is about.
    pub members: Vec<String>,
    /// For `case_clash`: the name already there.
    pub existing: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefusalView {
    Hollow {
        member: String,
        held: usize,
    },
    Blast {
        member: String,
        taking_off: usize,
        of: usize,
    },
}

impl From<Refusal> for RefusalView {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Hollow { member, held } => Self::Hollow { member, held },
            Refusal::Blast {
                member,
                taking_off,
                of,
            } => Self::Blast {
                member,
                taking_off,
                of,
            },
        }
    }
}

/// Open and decide, which is everything a preview is.
fn decided(journal: &Journal, name: &str, ask: Ask) -> Result<(Opened, Decided), String> {
    let opened = tungstate_sync::open(journal, name, &crate::secrets()).map_err(crate::describe)?;
    let asked = ask.into_core(&opened)?;
    let decided = tungstate_sync::decide_with(&opened, journal, &asked).map_err(crate::describe)?;
    Ok((opened, decided))
}

pub fn preview(journal: &Journal, name: &str, ask: Ask) -> Result<SyncPreviewView, String> {
    let (opened, decided) = decided(journal, name, ask)?;
    Ok(preview_of(&opened, &decided))
}

#[allow(clippy::too_many_lines)]
fn preview_of(opened: &Opened, decided: &Decided) -> SyncPreviewView {
    let plan = &decided.plan;
    let name = |id: i64| opened.name(id).to_string();
    let local = |id: i64| {
        opened
            .places
            .iter()
            .find(|p| p.member.id.0 == id)
            .is_some_and(|p| p.member.connection.is_none() && !p.networked)
    };
    let files_of = |id: i64| decided.members.iter().find(|m| m.id == id);
    SyncPreviewView {
        sync: opened.sync.name.clone(),
        fingerprint: plan.fingerprint(),
        empty: plan.is_empty(),
        reversible: plan.reversible(),
        members: opened
            .places
            .iter()
            .map(|place| {
                let b = plan
                    .blast
                    .get(&place.member.id.0)
                    .cloned()
                    .unwrap_or_default();
                MemberPreview {
                    name: place.member.name.clone(),
                    at: place.described.clone(),
                    arriving: b.arriving,
                    arriving_bytes: b.arriving_bytes,
                    leaving: b.leaving,
                    replacing: b.replacing,
                    removing: b.removing,
                    deleting: b.deleting,
                    renaming: b.renaming,
                    parking: b.parking,
                    holds: b.of,
                }
            })
            .collect(),
        legs: plan
            .legs
            .iter()
            .map(|leg| LegView {
                from: name(leg.from),
                to: name(leg.to),
                files: leg.paths.len(),
                parked: leg.parked.len(),
                bytes: leg.bytes,
                through_here: !local(leg.from) && !local(leg.to),
            })
            .collect(),
        read: ReadView {
            files: decided.read,
            sampled: decided.sampled,
            no_times: decided.read_for_no_times,
            moves: decided.read_for_moves,
            parked: decided.read_for_parked,
        },
        removed: plan
            .ops
            .iter()
            .filter_map(|op| match op {
                SyncOp::Remove {
                    member,
                    path,
                    how,
                    because,
                } => {
                    let because = match because {
                        Removed::Deleted => "deleted",
                        Removed::Extra => "extra",
                        Removed::Forgotten => "forgotten",
                        Removed::Superseded | Removed::Moved => return None,
                    };
                    Some(RemovedView {
                        member: name(*member),
                        path: path.clone(),
                        because,
                        gone: *how == Removal::Delete,
                    })
                }
                _ => None,
            })
            .collect(),
        conflicts: plan
            .left_alone
            .iter()
            .filter_map(|left| match &left.why {
                Why::Conflict { members } => Some(ConflictView {
                    path: left.path.clone(),
                    versions: members
                        .iter()
                        .filter_map(|id| {
                            let now = files_of(*id)?.files.get(&left.path)?;
                            Some(VersionView {
                                member: name(*id),
                                size: now.size,
                                modified: now.mtime,
                            })
                        })
                        .collect(),
                }),
                _ => None,
            })
            .collect(),
        left_alone: plan
            .left_alone
            .iter()
            .map(|left| {
                let (why, members, existing) = match &left.why {
                    Why::TooRecent { member } => ("too_recent", vec![*member], None),
                    Why::OnlyTheAnchorSends { member } => {
                        ("only_anchor_sends", vec![*member], None)
                    }
                    Why::OnlyTheAnchorReceives { member } => {
                        ("only_anchor_receives", vec![*member], None)
                    }
                    Why::Conflict { members } => ("conflict", members.clone(), None),
                    Why::CaseClash { member, existing } => {
                        ("case_clash", vec![*member], Some(existing.clone()))
                    }
                    Why::NeedsRename { member } => ("needs_rename", vec![*member], None),
                };
                LeftAloneView {
                    path: left.path.clone(),
                    why,
                    members: members.into_iter().map(name).collect(),
                    existing,
                }
            })
            .collect(),
        refusals: tungstate_sync::refusals(opened, decided)
            .into_iter()
            .map(RefusalView::from)
            .collect(),
    }
}

/// One run asked for: the sync, what was said about it, and the preview it
/// was asked from.
pub struct Job {
    pub name: String,
    pub ask: Ask,
    /// The preview's fingerprint. The run is refused if it would now do
    /// something else: nothing moves that was not seen.
    pub expected: String,
    /// The refusals the preview showed were read, and the person said go.
    pub confirmed: bool,
}

/// The window's syncs in flight: one at a time, each after the one before.
pub struct Syncing {
    pub queue: RunQueue<Job>,
    pub stop: Arc<Stop>,
}

impl Default for Syncing {
    fn default() -> Self {
        Self {
            queue: RunQueue::new(),
            stop: Arc::new(Stop::new()),
        }
    }
}

/// What a run did, for the done screen and History.
#[derive(Debug, Clone, Serialize)]
pub struct SyncRanView {
    pub sync: String,
    pub plan: Option<i64>,
    pub legs: Vec<LegRanView>,
    pub taken_off: usize,
    pub renamed: usize,
    pub missed: Vec<MissedView>,
    pub stopped: bool,
    pub reversible: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegRanView {
    pub from: String,
    pub to: String,
    pub copied: u64,
    pub bytes: u64,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissedView {
    pub member: String,
    pub path: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncErrorEvent {
    pub sync: String,
    /// `changed` (the run is not the one previewed), `refused` (it needs a
    /// yes it was not given) or `failed`.
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueuedView {
    /// This call started a worker, so the window may clear what it shows.
    pub started: bool,
    pub waiting: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegEvent {
    pub sync: String,
    /// Which of the preview's legs, from zero.
    pub index: usize,
    pub from: String,
    pub to: String,
}

/// Progress for a sync: the transfer events, as `sync://…`, and a
/// `sync://leg` as each leg begins.
///
/// The engine runs the preview's legs in order and each one is a transfer
/// that says `began` once, so counting `began` is counting legs.
struct SyncProgress {
    events: EventProgress,
    app: AppHandle,
    sync: String,
    legs: Vec<(String, String)>,
    next: usize,
}

impl Progress for SyncProgress {
    fn began(&mut self, shape: RunShape) {
        if let Some((from, to)) = self.legs.get(self.next) {
            let _ = self.app.emit(
                "sync://leg",
                LegEvent {
                    sync: self.sync.clone(),
                    index: self.next,
                    from: from.clone(),
                    to: to.clone(),
                },
            );
        }
        self.next += 1;
        self.events.began(shape);
    }
    fn planned(&mut self, files: &[tungstate_transfer::Planned]) {
        self.events.planned(files);
    }
    fn at_once(&mut self, files: usize) {
        self.events.at_once(files);
    }
    fn checking(&mut self, path: &std::path::Path, done: u64, total: u64) {
        self.events.checking(path, done, total);
    }
    fn advanced(&mut self, path: &std::path::Path, done: u64, total: u64) {
        self.events.advanced(path, done, total);
    }
    fn starting(&mut self, path: &std::path::Path, size: u64) {
        self.events.starting(path, size);
    }
    fn finished(&mut self, path: &std::path::Path, outcome: tungstate_transfer::FileOutcome) {
        self.events.finished(path, outcome);
    }
}

/// Queue a run. Syncs run one after another, never beside a transfer: two
/// runs writing one folder at once would race on names.
pub fn submit(app: &AppHandle, job: Job) -> Result<QueuedView, String> {
    let state = app.state::<App>();
    // One gate for both kinds of run, so a sync and a transfer deciding to
    // start at the same moment cannot both see the other idle.
    let _gate = lock(&state.gate);
    if state.queue.is_running() {
        return Err("a transfer is running; run the sync when it has finished".into());
    }
    if !state.syncing.queue.submit([job]) {
        return Ok(QueuedView {
            started: false,
            waiting: state.syncing.queue.waiting(),
        });
    }
    state.syncing.stop.clear();
    let worker = app.clone();
    std::thread::spawn(move || {
        let state = worker.state::<App>();
        let guard = Guard::new(worker.clone());
        while let Some(job) = state.syncing.queue.next() {
            let name = job.name.clone();
            let ran = carry(&worker, &state.journal, &state.syncing.stop, job);
            let _ = match ran {
                Ok(ran) => worker.emit("sync://done", ran),
                Err((kind, message)) => worker.emit(
                    "sync://error",
                    SyncErrorEvent {
                        sync: name,
                        kind,
                        message,
                    },
                ),
            };
            if state.syncing.stop.asked() {
                // A stop means the queue as well: what was waiting should
                // not start behind the person's back.
                state.syncing.queue.clear();
            }
        }
        guard.disarm();
    });
    Ok(QueuedView {
        started: true,
        waiting: 0,
    })
}

/// Stops being the worker if the thread ends by panicking.
///
/// Only then: on the normal path `next` has already released the queue, and
/// releasing it again could release a worker that claimed it since.
struct Guard {
    app: AppHandle,
    armed: bool,
}

impl Guard {
    fn new(app: AppHandle) -> Self {
        Self { app, armed: true }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.armed {
            self.app.state::<App>().syncing.queue.stand_down();
        }
    }
}

fn carry(
    app: &AppHandle,
    journal: &Journal,
    stop: &Arc<Stop>,
    job: Job,
) -> Result<SyncRanView, (&'static str, String)> {
    let failed = |why: String| ("failed", why);
    let (opened, decided) = decided(journal, &job.name, job.ask).map_err(failed)?;
    if decided.plan.fingerprint() != job.expected {
        return Err((
            "changed",
            "something changed since the preview was made; look again before running".into(),
        ));
    }
    if !job.confirmed && !tungstate_sync::refusals(&opened, &decided).is_empty() {
        return Err(("refused", "this run needs a yes it was not given".into()));
    }
    let mut progress = SyncProgress {
        events: EventProgress::for_sync(app.clone()),
        app: app.clone(),
        sync: job.name.clone(),
        legs: decided
            .plan
            .legs
            .iter()
            .map(|leg| {
                (
                    opened.name(leg.from).to_string(),
                    opened.name(leg.to).to_string(),
                )
            })
            .collect(),
        next: 0,
    };
    let ran = tungstate_sync::run_until(
        &opened,
        &decided,
        journal,
        &mut progress,
        None,
        Some(Arc::clone(stop)),
    )
    .map_err(|e| failed(crate::describe(e)))?;
    Ok(SyncRanView {
        sync: job.name,
        plan: ran.plan.map(|p| p.0),
        legs: ran
            .legs
            .iter()
            .map(|leg| LegRanView {
                from: leg.from.clone(),
                to: leg.to.clone(),
                copied: leg.summary.transferred,
                bytes: leg.summary.bytes,
                failed: leg.summary.failures.len(),
            })
            .collect(),
        taken_off: ran.taken_off,
        renamed: ran.renamed,
        missed: ran
            .missed
            .iter()
            .map(|m| MissedView {
                member: m.member.clone(),
                path: m.path.clone(),
                why: m.why.clone(),
            })
            .collect(),
        stopped: ran.stopped,
        reversible: decided.plan.reversible(),
    })
}

/// One past run, as a history row draws it.
#[derive(Debug, Clone, Serialize)]
pub struct PastSyncView {
    pub plan: i64,
    pub sync: String,
    pub applied_at: i64,
    pub copied: u64,
    pub taken_off: u64,
    pub renamed: u64,
    pub bytes: u64,
    pub undone: bool,
    pub undoable: bool,
}

/// How many past runs the section lists.
const PAST_SHOWN: usize = 50;

pub fn past(journal: &Journal) -> Result<Vec<PastSyncView>, String> {
    Ok(journal
        .past_syncs(PAST_SHOWN)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|run| PastSyncView {
            plan: run.plan.id.0,
            sync: run.sync,
            applied_at: run.plan.applied_at,
            copied: run.copied,
            taken_off: run.taken_off,
            renamed: run.renamed,
            bytes: run.bytes,
            undone: run.plan.is_undone(),
            undoable: run.plan.is_undoable(),
        })
        .collect())
}

/// What putting a run back did.
#[derive(Debug, Clone, Serialize)]
pub struct UndoneView {
    pub plan: i64,
    pub put_back: usize,
    pub taken_off: usize,
    pub parked_left: usize,
    pub revived: Vec<RevivedView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevivedView {
    pub member: String,
    pub path: String,
}

pub fn put_back(journal: &Journal, name: &str, plan: i64) -> Result<UndoneView, String> {
    let opened = tungstate_sync::open(journal, name, &crate::secrets()).map_err(crate::describe)?;
    let undone = tungstate_sync::undo(&opened, journal, PlanId(plan)).map_err(crate::describe)?;
    Ok(UndoneView {
        plan,
        put_back: undone.put_back,
        taken_off: undone.taken_off,
        parked_left: undone.parked_left,
        revived: undone
            .revived
            .into_iter()
            .map(|(member, path)| RevivedView { member, path })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(n: usize) -> Vec<tempfile::TempDir> {
        (0..n).map(|_| tempfile::tempdir().unwrap()).collect()
    }

    fn settings(exact: bool, on_remove: &str) -> SettingsForm {
        SettingsForm {
            exact,
            on_conflict: "quarantine".into(),
            on_remove: on_remove.into(),
            verify: "hash".into(),
            cooldown_secs: 0,
            first_check: "full".into(),
            launch: "quietly".into(),
        }
    }

    fn form(dirs: &[tempfile::TempDir]) -> NewSyncForm {
        NewSyncForm {
            name: " capcut ".into(),
            ends: dirs
                .iter()
                .map(|d| d.path().to_string_lossy().to_string())
                .collect(),
            names: vec!["laptop".into(), String::new()],
            direction: "push".into(),
            anchor: None,
            settings: settings(true, "set-aside"),
        }
    }

    #[test]
    fn a_sync_made_in_the_window_reads_back_as_it_was_asked_for() {
        let journal = Journal::open_in_memory().unwrap();
        let dirs = folders(2);

        let made = make(&journal, &form(&dirs)).unwrap();

        assert_eq!(made.name, "capcut", "trimmed");
        assert_eq!(made.direction, "push");
        assert_eq!(made.anchor.as_deref(), Some("laptop"));
        assert_eq!(made.launch, "quietly");
        assert!(made.exact);
        assert_eq!(made.members[0].name, "laptop");
        assert!(!made.members[1].name.is_empty(), "named after its folder");
    }

    #[test]
    fn a_refusal_crosses_as_a_kind_the_window_can_word() {
        let journal = Journal::open_in_memory().unwrap();
        let dirs = folders(2);
        let mut asked = form(&dirs);
        asked.settings = settings(false, "delete");

        let refused = make(&journal, &asked).unwrap_err();

        assert_eq!(
            serde_json::to_value(&refused).unwrap()["kind"],
            "delete_needs_exact"
        );
        assert!(journal.syncs().unwrap().is_empty());
    }

    #[test]
    fn a_preview_counts_and_lists_what_would_be_taken_off() {
        let journal = Journal::open_in_memory().unwrap();
        let dirs = folders(2);
        std::fs::write(dirs[0].path().join("a.mp4"), b"one").unwrap();
        std::fs::write(dirs[0].path().join("b.mp4"), b"two").unwrap();
        let mut asked = form(&dirs);
        asked.direction = "all".into();
        make(&journal, &asked).unwrap();
        let (opened, decided) = decided(&journal, "capcut", Ask::default()).unwrap();
        tungstate_sync::run(
            &opened,
            &decided,
            &journal,
            &mut tungstate_transfer::SilentProgress,
            None,
        )
        .unwrap();

        std::fs::remove_file(dirs[0].path().join("a.mp4")).unwrap();
        let preview = preview(&journal, "capcut", Ask::default()).unwrap();

        assert_eq!(preview.members[1].removing, 1);
        assert_eq!(preview.removed.len(), 1);
        assert_eq!(preview.removed[0].because, "deleted");
        assert!(!preview.removed[0].gone, "set aside, not deleted");
        assert_eq!(preview.refusals.len(), 1, "one of two is past a fifth");
        assert!(preview.reversible);
    }

    #[test]
    fn a_conflict_in_the_preview_names_each_version_and_its_member() {
        let journal = Journal::open_in_memory().unwrap();
        let dirs = folders(2);
        std::fs::write(dirs[0].path().join("clip.mp4"), b"laptop's").unwrap();
        std::fs::write(dirs[1].path().join("clip.mp4"), b"the nas's, longer").unwrap();
        let mut asked = form(&dirs);
        asked.direction = "all".into();
        make(&journal, &asked).unwrap();

        let preview = preview(&journal, "capcut", Ask::default()).unwrap();

        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(preview.conflicts[0].versions.len(), 2);
        assert_eq!(preview.conflicts[0].versions[0].member, "laptop");
        assert_eq!(preview.left_alone[0].why, "conflict");

        let settled = Ask {
            resolve: vec![Settled {
                path: "clip.mp4".into(),
                keep: Some("laptop".into()),
            }],
            ..Ask::default()
        };
        let resolved = super::preview(&journal, "capcut", settled).unwrap();
        assert!(resolved.conflicts.is_empty());
        assert_eq!(resolved.members[1].replacing, 1);
    }
}
