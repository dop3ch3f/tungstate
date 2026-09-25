//! Keeping a set of folders in step: what should move, decided on paper.
//!
//! A sync is several **members**, each a folder somewhere, and a **baseline**:
//! what each member held after the last run. Every run asks the same question
//! of every path: what should the set hold, and which members are missing it?
//!
//! Two rules carry most of the weight, and both are in DESIGN §4d:
//!
//! - **A member is only ever compared with itself.** "Has this changed?" is
//!   this member's size and modification time against *its own* last reading.
//!   Clocks never cross members: FTP rounds to the minute, a NAS keeps its own
//!   timezone, and neither is wrong about itself.
//! - **Content is only compared when a decision needs it.** One changed member
//!   is simply delivered. Two, or a receiver that also changed, need their
//!   bytes compared, and that is a [`Question`] handed back to the caller,
//!   because reading bytes is I/O and this module does none.
//!
//! Pure: no I/O and ordered maps only, so the same input gives a
//! byte-identical plan, and [`apply_to`] can play a plan out on paper for the
//! property tests to decide again and expect nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::dupes::{free_name_by, stem_and_extension};
use crate::plan::Blast;
use crate::snapshot::{QUARANTINE, Snapshot, is_reserved};

/// A member's id, as the journal numbers it.
pub type MemberId = i64;

/// Which members may send and which may receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The anchor sends; every other member ends up holding what it holds.
    Push,
    /// The anchor receives; it ends up holding what every other member holds.
    Pull,
    /// Everybody sends and receives; every member ends up with everything.
    All,
}

/// How hard to look when two members hold a path nobody has seen before.
///
/// Only first contact: once a path is in the baseline, every member is judged
/// against its own reading and this never comes up again for it. A speed and
/// accuracy trade the person makes, so it is stored per sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FirstCheck {
    /// Read both copies in full. Exact; the first run of a big archive over a
    /// network takes as long as reading it.
    #[default]
    Full,
    /// Samples from the start, middle and end. Fast; two copies differing
    /// only between the samples are taken to match.
    Sampled,
    /// Equal sizes are taken to match, and nothing is read. Fastest; an edit
    /// that kept the size is missed.
    Size,
}

/// What happens to a path several members changed differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnConflict {
    /// Each member keeps its own, and the others' versions are parked in its
    /// set-aside area, named after the member they came from.
    #[default]
    Quarantine,
    /// The contested name is retired: every version lives on every member,
    /// named after the member it came from.
    Rename,
    /// Nothing moves until a person says which.
    Skip,
}

/// What taking a copy off a member means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnRemove {
    /// Into the member's set-aside area, from where `undo` puts it back.
    #[default]
    SetAside,
    /// Gone. For the archive that exists to reclaim space.
    Delete,
}

/// How a run decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub direction: Direction,
    /// The member `Push` sends from and `Pull` receives into. `None` for `All`.
    pub anchor: Option<MemberId>,
    pub first_check: FirstCheck,
    /// A file written more recently than this is still being written, and its
    /// path is left alone this run. Milliseconds.
    pub cooldown_ms: i64,
    /// The moment the decision is made, in milliseconds since the epoch.
    pub now_ms: i64,
    /// Deletions and extras are carried too, so the promised equality is
    /// exact rather than "at least".
    pub exact: bool,
    pub on_conflict: OnConflict,
    pub on_remove: OnRemove,
}

/// One file as a member reports it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    pub size: u64,
    /// Milliseconds, as *this member* reports it. `None` where the backend
    /// cannot say, and then the file is judged by its bytes.
    pub mtime: Option<i64>,
}

/// What a member held at a path after the last run that touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// False for "absent, by decision": the set agreed this path is gone here.
    pub present: bool,
    pub size: u64,
    pub mtime: Option<i64>,
    /// The content's digest, where one was read. Not every agreement reads
    /// bytes: [`FirstCheck::Size`] records none.
    pub hash: Option<String>,
}

/// One member as a decision sees it.
#[derive(Debug, Clone)]
pub struct Member {
    pub id: MemberId,
    pub name: String,
    /// On this machine. Reading from it costs no network, so it is the first
    /// choice of source for a copy.
    pub local: bool,
    /// Which connection it is reached through, `None` for this machine. Two
    /// members on one connection copy between themselves without the bytes
    /// crossing two services.
    pub connection: Option<i64>,
    /// False where two paths differing only in case are one file.
    pub case_sensitive: bool,
    /// False where nothing can be renamed (FTP), so nothing can be set aside
    /// and a carried tidy has to be a copy.
    pub renames: bool,
    /// Every file, by root-relative path with forward slashes.
    pub files: BTreeMap<String, Now>,
    /// What its set-aside area holds, by root-relative path
    /// (`.tungstate-quarantine/...`), so a conflict already parked is not
    /// parked again and a new name never lands on an old one.
    pub parked: BTreeMap<String, Now>,
}

impl Member {
    /// A member's files from a snapshot of it: files only, and never
    /// tungstate's own reserved directories or a transfer's partial file.
    #[must_use]
    pub fn listing(snapshot: &Snapshot) -> BTreeMap<String, Now> {
        snapshot
            .entries
            .iter()
            .filter(|entry| !entry.is_dir && !entry.is_symlink)
            .map(|entry| (entry.relative_path(), entry))
            .filter(|(path, _)| !is_reserved(path) && !is_partial(path))
            .map(|(path, entry)| {
                (
                    path,
                    Now {
                        size: entry.size,
                        mtime: entry.mtime.map(jiff::Timestamp::as_millisecond),
                    },
                )
            })
            .collect()
    }
}

/// Whether this is a transfer's half-written file, `name.tungstate-42.part`.
/// The engine's temp names are its own; syncing one would copy a copy in
/// progress.
#[must_use]
pub fn is_partial(path: &str) -> bool {
    let Some(stem) = path.strip_suffix(".part") else {
        return false;
    };
    stem.rsplit_once(".tungstate-")
        .is_some_and(|(_, id)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// The baseline: every member's last reading of every path it has held.
pub type Baseline = BTreeMap<(MemberId, String), Seen>;

/// Digests read so far, by member and path, in answer to [`Question`]s.
///
/// A sampled digest is written with [`SAMPLED`] in front. Sampled and whole
/// digests of the same file differ, so the two must never be compared, and a
/// sampled one is never remembered: it would meet a whole one on a later run
/// and call identical files different.
pub type Digests = BTreeMap<(MemberId, String), String>;

/// What a sampled digest starts with. See [`Digests`].
pub const SAMPLED: &str = "sampled:";

/// A read the decision needs before it can decide a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Question {
    pub member: MemberId,
    pub path: String,
    /// Read it all, or sample it.
    pub sampled: bool,
    pub because: Because,
}

/// Why bytes have to be read. The preview says it, because it is the
/// difference between a run that is free and one that reads everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Because {
    /// Two members hold this path and it is not yet known whether they agree.
    Agreement,
    /// This member reports no modification times, so only its bytes can say
    /// whether it changed.
    NoTimes,
    /// A file vanished and one of the same size appeared on the same member:
    /// the same bytes mean it was moved, not deleted and added.
    Moved,
    /// A conflicting version may already be parked from an earlier run.
    Parked,
}

/// Something a run does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOp {
    /// Carry `path` from one member to another. Anything the target held
    /// there has already been taken off by a [`SyncOp::Remove`] before it.
    Copy {
        from: MemberId,
        to: MemberId,
        path: String,
        size: u64,
    },
    /// Carry a conflicting version into the target's set-aside area, under a
    /// name that says whose it is.
    Park {
        from: MemberId,
        to: MemberId,
        path: String,
        parked: String,
        size: u64,
    },
    /// Take a copy off a member, the way the sync was told to.
    Remove {
        member: MemberId,
        path: String,
        how: Removal,
        because: Removed,
    },
    /// Rename on one member: a tidy carried across, or a contested name
    /// retired. `hash` is the content's, for remembering it under its new name.
    Rename {
        member: MemberId,
        from: String,
        to: String,
        hash: Option<String>,
    },
    /// Remember what a member holds, and move nothing. A member that already
    /// agrees is acknowledged, so next run it is not read again.
    Record {
        member: MemberId,
        path: String,
        seen: Seen,
    },
}

/// How a copy is taken off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// Renamed into the member's set-aside area, at `to`.
    SetAside { to: String },
    /// Deleted outright. A run that does this cannot be undone.
    Delete,
}

/// Why a copy is taken off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Removed {
    /// Another member deleted it.
    Deleted,
    /// The member holds something the promise says it must not.
    Extra,
    /// A newer version is arriving in its place.
    Superseded,
    /// `sync forget` said so.
    Forgotten,
    /// The file moved to a new name that has just been copied here, on a
    /// member that cannot rename.
    Moved,
}

/// Why a path was left alone.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// Written on this member within the cooldown: still being written.
    TooRecent { member: MemberId },
    /// Changed on a member that does not send under this direction.
    OnlyTheAnchorSends { member: MemberId },
    /// Changed on the anchor, which only receives under this direction.
    OnlyTheAnchorReceives { member: MemberId },
    /// Two or more members changed it differently. Never resolved by
    /// comparing clocks.
    Conflict { members: Vec<MemberId> },
    /// The target holds a name differing only in case, and cannot hold both.
    CaseClash { member: MemberId, existing: String },
    /// A copy would have to be set aside on a member that cannot rename.
    NeedsRename { member: MemberId },
}

/// A path left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeftAlone {
    pub path: String,
    pub why: Why,
}

/// What one member would see happen. Copies and removals are separate
/// numbers and are never added together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberBlast {
    /// Files copied into it.
    pub arriving: usize,
    /// Bytes in those.
    pub arriving_bytes: u64,
    /// Files copied out of it to somewhere else.
    pub leaving: usize,
    /// Files whose version it holds would be taken off for a newer one.
    pub replacing: usize,
    /// Files taken off because another member deleted them, because they are
    /// extra, or because they were forgotten.
    pub removing: usize,
    /// Of `replacing` and `removing`, those deleted outright.
    pub deleting: usize,
    /// Conflicting versions parked in its set-aside area.
    pub parking: usize,
    /// Files renamed in place.
    pub renaming: usize,
    /// Files it holds.
    pub of: usize,
    /// Whether `replacing` and `removing` together pass [`Blast`]'s limits.
    pub over_limit: bool,
}

/// Every copy between one pair of members, in the order they will run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leg {
    pub from: MemberId,
    pub to: MemberId,
    /// Source paths.
    pub paths: Vec<String>,
    pub bytes: u64,
    /// Source paths that land under another name: conflicting versions going
    /// into the set-aside area.
    pub parked: BTreeMap<String, String>,
}

/// A run, decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    pub ops: Vec<SyncOp>,
    pub left_alone: Vec<LeftAlone>,
    /// Reads needed before some paths can be decided. Those paths have no ops
    /// yet: answer these, decide again, and they will.
    pub questions: Vec<Question>,
    pub blast: BTreeMap<MemberId, MemberBlast>,
    /// Copies grouped by pair, which is how the engine runs them.
    pub legs: Vec<Leg>,
    /// Members that list nothing where the baseline says they held files.
    /// An unmounted volume looks exactly like everything deleted.
    pub hollow: Vec<MemberId>,
}

impl SyncPlan {
    /// Whether nothing would move and nothing needs remembering.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Whether `undo` could take this run back: nothing is deleted outright.
    #[must_use]
    pub fn reversible(&self) -> bool {
        !self.ops.iter().any(|op| {
            matches!(
                op,
                SyncOp::Remove {
                    how: Removal::Delete,
                    ..
                }
            )
        })
    }
}

/// What a person asked for beyond "run": `sync resolve` and `sync forget`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asked {
    /// A conflict settled by hand, by path.
    pub resolve: BTreeMap<String, Resolve>,
    /// Files, or folders, to take off every member for good.
    pub forget: BTreeSet<String>,
    /// Decide only the paths named here, and leave everything else for a run.
    pub only: bool,
}

impl Asked {
    fn forgets(&self, path: &str) -> bool {
        self.forget.iter().any(|gone| {
            path.strip_prefix(gone.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    }

    fn wants(&self, path: &str) -> bool {
        !self.only || self.resolve.contains_key(path) || self.forgets(path)
    }
}

/// How a person settled a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    /// This member's version, everywhere.
    Keep(MemberId),
    /// Every version, everywhere, each named after its member.
    KeepBoth,
}

/// A member's state at one path, against its own baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Present, and never recorded here (or recorded absent).
    New,
    /// Absent, and never recorded here: it joined after the file existed.
    Missing,
    /// Present and as last recorded.
    Unchanged,
    /// Present and different from last recorded.
    Changed,
    /// Recorded present, now absent.
    Deleted,
    /// Recorded absent, still absent.
    Gone,
}

impl State {
    fn moved(self) -> bool {
        matches!(self, Self::New | Self::Changed)
    }
}

/// Everything a decision reads, in one place.
struct Ctx<'a> {
    members: &'a [Member],
    baseline: &'a Baseline,
    digests: &'a Digests,
    mode: &'a Mode,
    asked: &'a Asked,
}

impl Ctx<'_> {
    fn anchored(&self, member: &Member) -> bool {
        self.mode.anchor == Some(member.id)
    }

    fn sends(&self, member: &Member) -> bool {
        match self.mode.direction {
            Direction::All => true,
            Direction::Push => self.anchored(member),
            Direction::Pull => !self.anchored(member),
        }
    }

    fn receives(&self, member: &Member) -> bool {
        match self.mode.direction {
            Direction::All => true,
            Direction::Push => !self.anchored(member),
            Direction::Pull => self.anchored(member),
        }
    }

    fn seen_of(&self, member: &Member, path: &str) -> Seen {
        seen_of(member, path, self.baseline, self.digests)
    }

    fn row(&self, member: &Member, path: &str) -> Option<&Seen> {
        self.baseline.get(&(member.id, path.to_string()))
    }

    /// How to take `path` off `member`, or `None` where it would have to be
    /// set aside and the member cannot rename.
    fn removal(&self, names: &mut Names, member: &Member, path: &str) -> Option<Removal> {
        match self.mode.on_remove {
            OnRemove::Delete => Some(Removal::Delete),
            OnRemove::SetAside if member.renames => Some(Removal::SetAside {
                to: names.claim(&[member.id], &format!("{QUARANTINE}/{path}")),
            }),
            OnRemove::SetAside => None,
        }
    }
}

/// The names each member holds or will hold, so a copy never lands on a
/// name differing only in case and a new name never lands on a taken one.
struct Names {
    members: BTreeMap<MemberId, Held>,
}

struct Held {
    fold: bool,
    /// Files, keyed as this member compares names.
    files: BTreeMap<String, String>,
    /// Every name in use, files and set-aside alike, keyed the same way.
    taken: BTreeSet<String>,
}

impl Held {
    fn key(&self, path: &str) -> String {
        if self.fold {
            path.to_lowercase()
        } else {
            path.to_string()
        }
    }
}

impl Names {
    fn new(members: &[Member]) -> Self {
        let members = members
            .iter()
            .map(|member| {
                let mut held = Held {
                    fold: !member.case_sensitive,
                    files: BTreeMap::new(),
                    taken: BTreeSet::new(),
                };
                for path in member.files.keys() {
                    held.files.insert(held.key(path), path.clone());
                    held.taken.insert(held.key(path));
                }
                for path in member.parked.keys() {
                    held.taken.insert(held.key(path));
                }
                (member.id, held)
            })
            .collect();
        Self { members }
    }

    /// A name the member holds that differs from `path` only in case.
    fn clash(&self, member: MemberId, path: &str) -> Option<&String> {
        let held = self.members.get(&member)?;
        held.files
            .get(&held.key(path))
            .filter(|existing| existing.as_str() != path)
    }

    fn hold(&mut self, member: MemberId, path: &str) {
        if let Some(held) = self.members.get_mut(&member) {
            let key = held.key(path);
            held.files.insert(key.clone(), path.to_string());
            held.taken.insert(key);
        }
    }

    /// `wanted`, or the first numbered variant free on every one of `on`, and
    /// taken on each from now on.
    fn claim(&mut self, on: &[MemberId], wanted: &str) -> String {
        let name = free_name_by(wanted, |candidate| {
            on.iter().any(|id| {
                self.members
                    .get(id)
                    .is_some_and(|held| held.taken.contains(&held.key(candidate)))
            })
        });
        for id in on {
            if let Some(held) = self.members.get_mut(id) {
                let key = held.key(&name);
                held.taken.insert(key);
            }
        }
        name
    }
}

/// `clips/clip.mp4` as `clips/clip (nas).mp4`.
fn named_after(path: &str, member: &str) -> String {
    let (stem, extension) = stem_and_extension(path);
    if stem.is_empty() || stem.ends_with('/') {
        return format!("{path} ({member})");
    }
    format!("{stem} ({member}){extension}")
}

/// Where a conflicting version from `member` is parked.
fn parked_name(path: &str, member: &str) -> String {
    format!("{QUARANTINE}/{}", named_after(path, member))
}

/// Whether `candidate` is `wanted` or a numbered variant of it, the way
/// `Names::claim` numbers one.
fn is_numbered(candidate: &str, wanted: &str) -> bool {
    if candidate == wanted {
        return true;
    }
    let (stem, extension) = stem_and_extension(wanted);
    candidate
        .strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.strip_suffix(extension.as_str()))
        .is_some_and(|nth| !nth.is_empty() && nth.bytes().all(|b| b.is_ascii_digit()))
}

/// Decide a run. See the module docs and `docs/slices/09b-folder-sync.md`.
#[must_use]
pub fn decide(members: &[Member], baseline: &Baseline, digests: &Digests, mode: &Mode) -> SyncPlan {
    decide_with(members, baseline, digests, mode, &Asked::default())
}

/// [`decide`], with conflicts settled and paths forgotten by hand.
#[must_use]
pub fn decide_with(
    members: &[Member],
    baseline: &Baseline,
    digests: &Digests,
    mode: &Mode,
    asked: &Asked,
) -> SyncPlan {
    let ctx = Ctx {
        members,
        baseline,
        digests,
        mode,
        asked,
    };
    let mut plan = SyncPlan::default();
    let paths: BTreeSet<&str> = members
        .iter()
        .flat_map(|member| member.files.keys().map(String::as_str))
        .chain(
            baseline
                .iter()
                .filter(|((member, _), _)| members.iter().any(|m| m.id == *member))
                .map(|((_, path), _)| path.as_str()),
        )
        .filter(|path| !is_reserved(path) && !is_partial(path) && asked.wants(path))
        .collect();

    let mut names = Names::new(members);
    let settled = if asked.only {
        BTreeSet::new()
    } else {
        detect_moves(&ctx, &mut names, &mut plan)
    };
    for path in paths {
        if !settled.contains(path) {
            decide_path(&ctx, path, &mut names, &mut plan);
        }
    }

    plan.hollow = members
        .iter()
        .filter(|member| {
            member.files.is_empty()
                && baseline
                    .iter()
                    .any(|((id, _), seen)| *id == member.id && seen.present)
        })
        .map(|member| member.id)
        .collect();
    plan.questions.sort();
    plan.questions.dedup();
    plan.left_alone.sort();
    plan.left_alone.dedup();
    plan.legs = legs(members, &plan.ops);
    plan.blast = blast(members, &plan.ops);
    plan
}

/// Step 1: each member against its own baseline. `None` when the path is
/// left alone this run, or waits on a read.
fn classify<'a>(
    ctx: &Ctx<'a>,
    path: &str,
    plan: &mut SyncPlan,
) -> Option<Vec<(&'a Member, State)>> {
    let mut states = Vec::with_capacity(ctx.members.len());
    let mut unread = false;
    for member in ctx.members {
        let now = member.files.get(path);
        let row = ctx.row(member, path);
        let seen = row.filter(|seen| seen.present);
        let state = match (seen, now) {
            (None, Some(_)) => State::New,
            (None, None) if row.is_some() => State::Gone,
            (None, None) => State::Missing,
            (Some(_), None) => State::Deleted,
            (Some(seen), Some(now)) => match changed(member, path, seen, now, ctx.digests) {
                Some(true) => State::Changed,
                Some(false) => State::Unchanged,
                None => {
                    plan.questions.push(Question {
                        member: member.id,
                        path: path.to_string(),
                        sampled: false,
                        because: Because::NoTimes,
                    });
                    unread = true;
                    continue;
                }
            },
        };
        if state.moved() && too_recent(ctx.mode, now) {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::TooRecent { member: member.id },
            });
            return None;
        }
        states.push((member, state));
    }
    // Every member that needs reading is asked for at once, so a path costs
    // one round of reads rather than one per member.
    (!unread).then_some(states)
}

fn too_recent(mode: &Mode, now: Option<&Now>) -> bool {
    now.and_then(|now| now.mtime)
        .is_some_and(|at| mode.now_ms - at < mode.cooldown_ms)
}

#[allow(clippy::too_many_lines)]
fn decide_path(ctx: &Ctx<'_>, path: &str, names: &mut Names, plan: &mut SyncPlan) {
    if ctx.asked.forgets(path) {
        forget_path(ctx, path, names, plan);
        return;
    }
    let Some(states) = classify(ctx, path, plan) else {
        return;
    };
    let state_of = |id: MemberId| states.iter().find(|(m, _)| m.id == id).map(|(_, s)| *s);
    if let Some(Resolve::Keep(chosen)) = ctx.asked.resolve.get(path)
        && let Some(lead) = ctx
            .members
            .iter()
            .find(|m| m.id == *chosen && m.files.contains_key(path))
    {
        keep_one(ctx, path, lead, &states, names, plan);
        return;
    }
    let mode = ctx.mode;

    // 2. A change on a member that may not send goes nowhere, and is said.
    //    Said at the end, not here: one that turns out to match what is being
    //    delivered is simply remembered, and reporting it would be a false
    //    alarm. Under `exact` it is not said at all, since it is replaced.
    let unsent: Vec<&Member> = states
        .iter()
        .filter(|(member, state)| state.moved() && !ctx.sends(member))
        .map(|(member, _)| *member)
        .collect();
    let report_unsent = |plan: &mut SyncPlan, except: &dyn Fn(&Member) -> bool| {
        if mode.exact {
            return;
        }
        for member in unsent.iter().filter(|m| !except(m)) {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: if ctx.anchored(member) {
                    Why::OnlyTheAnchorReceives { member: member.id }
                } else {
                    Why::OnlyTheAnchorSends { member: member.id }
                },
            });
        }
    };

    // 3. The desired content: what moved on a sender, or failing that what a
    //    sender still holds unchanged. Under `exact` it can be nothing at
    //    all: no sender holds it, or under `All` somebody deleted it.
    let senders_in = |wanted: &dyn Fn(State) -> bool| -> Vec<&Member> {
        states
            .iter()
            .filter(|(member, state)| wanted(*state) && ctx.sends(member))
            .map(|(member, _)| *member)
            .collect()
    };
    let changed_senders = senders_in(&State::moved);
    let unchanged_senders = senders_in(&|s| s == State::Unchanged);
    let deleted = states.iter().any(|(_, s)| *s == State::Deleted);
    let gone_everywhere = changed_senders.is_empty()
        && mode.exact
        && (unchanged_senders.is_empty() || (mode.direction == Direction::All && deleted));
    let holders: Vec<&Member> = if !changed_senders.is_empty() {
        changed_senders
    } else if gone_everywhere {
        Vec::new()
    } else {
        unchanged_senders
    };

    let Some(lead) = holders.first().copied() else {
        if gone_everywhere {
            remove_everywhere(ctx, path, &states, names, plan);
            return;
        }
        report_unsent(plan, &|_| false);
        // Nothing to deliver. A deletion that stands is remembered, so it is
        // not reported again next run.
        for (member, state) in &states {
            if *state == State::Deleted {
                plan.ops.push(record_absent(member, path));
            }
        }
        return;
    };

    // Everyone whose content has to be compared with the lead's: the other
    // holders, any receiver that moved on its own, and any sender that still
    // holds an older version it will not be sent a newer one of. Under
    // `--pull` two members may send different versions to one anchor, and
    // that is a conflict now, not a surprise on the next run.
    let mut must_agree: Vec<&Member> = holders[1..].to_vec();
    for (member, state) in &states {
        if holders.iter().any(|h| h.id == member.id) {
            continue;
        }
        let moved_receiver = state.moved() && ctx.receives(member);
        let stranded = *state == State::Unchanged && ctx.sends(member) && !ctx.receives(member);
        if moved_receiver || stranded {
            must_agree.push(member);
        }
    }
    let mut asked = false;
    let mut disagree = Vec::new();
    for other in &must_agree {
        let both_still = state_of(lead.id) == Some(State::Unchanged)
            && state_of(other.id) == Some(State::Unchanged);
        match agree(ctx, lead, other, path, both_still) {
            Agreement::Same => {}
            Agreement::Different => disagree.push(other.id),
            Agreement::Ask(questions) => {
                plan.questions.extend(questions);
                asked = true;
            }
        }
    }
    if asked {
        return;
    }
    // Under `exact` a receiver that may not send is replaced, not argued
    // with: only senders can be in conflict.
    let rivals: Vec<&Member> = must_agree
        .iter()
        .copied()
        .filter(|m| disagree.contains(&m.id) && (!mode.exact || ctx.sends(m)))
        .collect();
    if !rivals.is_empty() {
        report_unsent(plan, &|m| disagree.contains(&m.id));
        let mut set = vec![lead];
        set.extend(rivals);
        conflict(ctx, path, &set, &states, names, plan);
        return;
    }

    // 4. Deliver to every receiver not already holding it; remember the rest.
    let content = ctx.seen_of(lead, path);
    let lead_moved = state_of(lead.id).is_some_and(State::moved);
    let agrees = |member: &Member| {
        holders.iter().any(|h| h.id == member.id)
            || (must_agree.iter().any(|m| m.id == member.id) && !disagree.contains(&member.id))
    };
    // Against content that just moved, only digests can say a receiver
    // already holds it: an edit can keep the size. That happens when an
    // earlier run landed it but could not finish the path, and without
    // asking, every run would copy it again. Asked before anything is
    // decided, so a path waiting on a read has no ops yet.
    let theirs = |member: &Member, seen: &Seen| {
        seen.hash.clone().or_else(|| {
            ctx.digests
                .get(&(member.id, path.to_string()))
                .filter(|d| !d.starts_with(SAMPLED))
                .cloned()
        })
    };
    if lead_moved {
        let mut reads = Vec::new();
        for (member, state) in &states {
            let Some(seen) = ctx.row(member, path) else {
                continue;
            };
            if agrees(member)
                || !ctx.receives(member)
                || *state != State::Unchanged
                || seen.size != content.size
            {
                continue;
            }
            if content.hash.is_none() {
                reads.push(lead.id);
            }
            if theirs(member, seen).is_none() {
                reads.push(member.id);
            }
        }
        if !reads.is_empty() {
            plan.questions
                .extend(reads.into_iter().map(|member| Question {
                    member,
                    path: path.to_string(),
                    sampled: false,
                    because: Because::Agreement,
                }));
            return;
        }
    }

    let mut records = Vec::new();
    let mut withheld = false;
    for (member, state) in &states {
        if agrees(member) {
            if *state != State::Unchanged {
                records.push(SyncOp::Record {
                    member: member.id,
                    path: path.to_string(),
                    seen: reading(ctx, lead, member, path),
                });
            }
            continue;
        }
        if !ctx.receives(member) {
            // A deletion on a member that does not receive stands.
            if *state == State::Deleted {
                records.push(record_absent(member, path));
            }
            continue;
        }
        if *state == State::Unchanged
            && ctx.row(member, path).is_some_and(|seen| {
                match (theirs(member, seen), &content.hash) {
                    (Some(x), Some(y)) => x == *y,
                    // Against content that did not move, the two readings
                    // are enough.
                    _ => !lead_moved && same_content(seen, &content),
                }
            })
        {
            continue;
        }
        if deliver(ctx, path, &holders, member, content.size, names, plan).is_err() {
            withheld = true;
        }
    }
    report_unsent(plan, &|m| must_agree.iter().any(|a| a.id == m.id));
    // A path is remembered only once every part of it could happen, or the
    // next run would see members that each look unchanged and hold different
    // bytes, and call it a conflict instead of finishing.
    if !withheld {
        plan.ops.extend(records);
    }
}

/// Copy the path to `member` from the best of `holders`, taking off what it
/// holds there first. `Err` when that cannot be taken off.
#[allow(clippy::too_many_arguments)]
fn deliver(
    ctx: &Ctx<'_>,
    path: &str,
    holders: &[&Member],
    member: &Member,
    size: u64,
    names: &mut Names,
    plan: &mut SyncPlan,
) -> Result<(), ()> {
    if let Some(existing) = names.clash(member.id, path) {
        plan.left_alone.push(LeftAlone {
            path: path.to_string(),
            why: Why::CaseClash {
                member: member.id,
                existing: existing.clone(),
            },
        });
        return Ok(());
    }
    if member.files.contains_key(path) {
        let Some(how) = ctx.removal(names, member, path) else {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::NeedsRename { member: member.id },
            });
            return Err(());
        };
        plan.ops.push(SyncOp::Remove {
            member: member.id,
            path: path.to_string(),
            how,
            because: Removed::Superseded,
        });
    }
    plan.ops.push(SyncOp::Copy {
        from: route(holders, member).id,
        to: member.id,
        path: path.to_string(),
        size,
    });
    names.hold(member.id, path);
    Ok(())
}

/// What to remember of a member that agrees. Where agreement was taken on
/// size alone, no digest is remembered: two copies judged the same by size
/// may carry different digests read for some other reason, and remembering
/// them would call the two different on the next run.
fn reading(ctx: &Ctx<'_>, lead: &Member, member: &Member, path: &str) -> Seen {
    let mut seen = ctx.seen_of(member, path);
    if ctx.mode.first_check == FirstCheck::Size && first_meeting(ctx, lead, member, path) {
        seen.hash = None;
    }
    seen
}

/// Whether two copies are meeting for the first time: neither has been
/// recorded at this path. Asked of the pair, not the set, so a copy that
/// landed elsewhere in a run that could not finish does not change how these
/// two are compared next time.
fn first_meeting(ctx: &Ctx<'_>, a: &Member, b: &Member, path: &str) -> bool {
    ctx.row(a, path).is_none() && ctx.row(b, path).is_none()
}

fn record_absent(member: &Member, path: &str) -> SyncOp {
    SyncOp::Record {
        member: member.id,
        path: path.to_string(),
        seen: absent(),
    }
}

/// Under `exact`, nothing should be at `path`: take it off every receiver
/// that holds it.
fn remove_everywhere(
    ctx: &Ctx<'_>,
    path: &str,
    states: &[(&Member, State)],
    names: &mut Names,
    plan: &mut SyncPlan,
) {
    let because = if states.iter().any(|(_, s)| *s == State::Deleted) {
        Removed::Deleted
    } else {
        Removed::Extra
    };
    let mut withheld = false;
    for (member, _) in states {
        if !member.files.contains_key(path) || !ctx.receives(member) {
            continue;
        }
        if let Some(how) = ctx.removal(names, member, path) {
            plan.ops.push(SyncOp::Remove {
                member: member.id,
                path: path.to_string(),
                how,
                because,
            });
        } else {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::NeedsRename { member: member.id },
            });
            withheld = true;
        }
    }
    if !withheld {
        for (member, state) in states {
            if *state == State::Deleted {
                plan.ops.push(record_absent(member, path));
            }
        }
    }
}

/// `sync forget`: off every member, and remembered as gone everywhere so it
/// does not come back. All or nothing: a forget that reached some members
/// and not others would be copied back to them by the next run.
fn forget_path(ctx: &Ctx<'_>, path: &str, names: &mut Names, plan: &mut SyncPlan) {
    let holders: Vec<&Member> = ctx
        .members
        .iter()
        .filter(|m| m.files.contains_key(path))
        .collect();
    let stuck: Vec<&Member> = holders
        .iter()
        .copied()
        .filter(|m| ctx.mode.on_remove == OnRemove::SetAside && !m.renames)
        .collect();
    if !stuck.is_empty() {
        for member in stuck {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::NeedsRename { member: member.id },
            });
        }
        return;
    }
    for member in &holders {
        if let Some(how) = ctx.removal(names, member, path) {
            plan.ops.push(SyncOp::Remove {
                member: member.id,
                path: path.to_string(),
                how,
                because: Removed::Forgotten,
            });
        }
    }
    for member in ctx.members {
        if !member.files.contains_key(path) && ctx.row(member, path).is_some_and(|s| s.present) {
            plan.ops.push(record_absent(member, path));
        }
    }
}

/// Several members changed `path` differently: `set`, lead first.
fn conflict(
    ctx: &Ctx<'_>,
    path: &str,
    set: &[&Member],
    states: &[(&Member, State)],
    names: &mut Names,
    plan: &mut SyncPlan,
) {
    let how = match ctx.asked.resolve.get(path) {
        Some(Resolve::KeepBoth) => OnConflict::Rename,
        _ => ctx.mode.on_conflict,
    };
    if how != OnConflict::Rename || !keep_both(ctx, path, set, states, names, plan) {
        plan.left_alone.push(LeftAlone {
            path: path.to_string(),
            why: Why::Conflict {
                members: set.iter().map(|m| m.id).collect(),
            },
        });
    }
    if how == OnConflict::Quarantine {
        park(ctx, path, set, names, plan);
    }
}

/// Each member keeps its own, and receives every other member's version into
/// its set-aside area. Nothing is remembered, so it is asked about again; a
/// version already parked by an earlier run is recognised by its bytes and
/// not parked twice.
fn park(ctx: &Ctx<'_>, path: &str, set: &[&Member], names: &mut Names, plan: &mut SyncPlan) {
    let mut parks = Vec::new();
    let mut asked = false;
    // Every receiver, in the conflict or not: under `--pull` the members in
    // conflict all send, and it is the anchor that has to hear about it.
    for to in ctx.members.iter().filter(|m| ctx.receives(m)) {
        for from in set
            .iter()
            .filter(|m| m.id != to.id && ctx.sends(m) && m.files.contains_key(path))
        {
            let theirs = ctx
                .digests
                .get(&(from.id, path.to_string()))
                .filter(|d| !d.starts_with(SAMPLED))
                .cloned()
                .or_else(|| ctx.seen_of(from, path).hash);
            let Some(theirs) = theirs else {
                plan.questions.push(Question {
                    member: from.id,
                    path: path.to_string(),
                    sampled: false,
                    because: Because::Parked,
                });
                asked = true;
                continue;
            };
            let size = from.files[path].size;
            let wanted = parked_name(path, &from.name);
            let mut already = false;
            for (candidate, now) in &to.parked {
                if now.size != size || !is_numbered(candidate, &wanted) {
                    continue;
                }
                match ctx.digests.get(&(to.id, candidate.clone())) {
                    Some(digest) if *digest == theirs => already = true,
                    Some(_) => {}
                    None => {
                        plan.questions.push(Question {
                            member: to.id,
                            path: candidate.clone(),
                            sampled: false,
                            because: Because::Parked,
                        });
                        asked = true;
                    }
                }
            }
            if !already {
                parks.push((from.id, to.id, wanted, size));
            }
        }
    }
    if asked {
        return;
    }
    for (from, to, wanted, size) in parks {
        plan.ops.push(SyncOp::Park {
            from,
            to,
            path: path.to_string(),
            parked: names.claim(&[to], &wanted),
            size,
        });
    }
}

/// Retire the contested name: every version is renamed after its member on
/// the member that holds it and copied to every receiver, and any other copy
/// of the old name is taken off. `false` when a member that would have to
/// rename or set something aside cannot.
fn keep_both(
    ctx: &Ctx<'_>,
    path: &str,
    set: &[&Member],
    states: &[(&Member, State)],
    names: &mut Names,
    plan: &mut SyncPlan,
) -> bool {
    let in_set = |m: &Member| set.iter().any(|s| s.id == m.id);
    let outside: Vec<&Member> = states
        .iter()
        .map(|(m, _)| *m)
        .filter(|m| m.files.contains_key(path) && !in_set(m))
        .collect();
    let stuck: Vec<&Member> = set
        .iter()
        .copied()
        .filter(|m| !m.renames)
        .chain(
            outside
                .iter()
                .copied()
                .filter(|m| ctx.mode.on_remove == OnRemove::SetAside && !m.renames),
        )
        .collect();
    if !stuck.is_empty() {
        for member in stuck {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::NeedsRename { member: member.id },
            });
        }
        return false;
    }
    let everyone: Vec<MemberId> = ctx.members.iter().map(|m| m.id).collect();
    for member in set {
        let name = names.claim(&everyone, &named_after(path, &member.name));
        let size = member.files[path].size;
        plan.ops.push(SyncOp::Rename {
            member: member.id,
            from: path.to_string(),
            to: name.clone(),
            hash: ctx.seen_of(member, path).hash,
        });
        names.hold(member.id, &name);
        if !ctx.sends(member) {
            continue;
        }
        for to in ctx
            .members
            .iter()
            .filter(|m| m.id != member.id && ctx.receives(m))
        {
            plan.ops.push(SyncOp::Copy {
                from: member.id,
                to: to.id,
                path: name.clone(),
                size,
            });
            names.hold(to.id, &name);
        }
    }
    for member in outside {
        if let Some(how) = ctx.removal(names, member, path) {
            plan.ops.push(SyncOp::Remove {
                member: member.id,
                path: path.to_string(),
                how,
                because: Removed::Superseded,
            });
        }
    }
    for (member, state) in states {
        if *state == State::Deleted && !in_set(member) {
            plan.ops.push(record_absent(member, path));
        }
    }
    true
}

/// `sync resolve --keep`: `lead`'s version everywhere, whichever way the
/// sync normally runs, since a person chose it.
fn keep_one(
    ctx: &Ctx<'_>,
    path: &str,
    lead: &Member,
    states: &[(&Member, State)],
    names: &mut Names,
    plan: &mut SyncPlan,
) {
    let state_of = |id: MemberId| states.iter().find(|(m, _)| m.id == id).map(|(_, s)| *s);
    let mut differ = BTreeSet::new();
    let mut asked = false;
    for (other, state) in states {
        if other.id == lead.id || !other.files.contains_key(path) {
            continue;
        }
        let both_still = state_of(lead.id) == Some(State::Unchanged) && *state == State::Unchanged;
        match agree(ctx, lead, other, path, both_still) {
            Agreement::Same => {}
            Agreement::Different => {
                differ.insert(other.id);
            }
            Agreement::Ask(questions) => {
                plan.questions.extend(questions);
                asked = true;
            }
        }
    }
    if asked {
        return;
    }
    let size = lead.files[path].size;
    let mut records = Vec::new();
    let mut withheld = false;
    if state_of(lead.id) != Some(State::Unchanged) {
        records.push(SyncOp::Record {
            member: lead.id,
            path: path.to_string(),
            seen: reading(ctx, lead, lead, path),
        });
    }
    for (member, state) in states {
        if member.id == lead.id {
            continue;
        }
        let holds = member.files.contains_key(path);
        if holds && !differ.contains(&member.id) {
            if *state != State::Unchanged {
                records.push(SyncOp::Record {
                    member: member.id,
                    path: path.to_string(),
                    seen: reading(ctx, lead, member, path),
                });
            }
        } else if holds || ctx.receives(member) {
            if deliver(ctx, path, &[lead], member, size, names, plan).is_err() {
                withheld = true;
            }
        } else if *state == State::Deleted {
            records.push(record_absent(member, path));
        }
    }
    if !withheld {
        plan.ops.extend(records);
    }
}

/// A governed member tidies between runs, so a file vanishes and the same
/// bytes appear under another name. Seen as a deletion and an addition, an
/// exact sync would set the old name aside everywhere and copy the new one;
/// a non-exact one would copy the old name straight back, and the sync and
/// the policy would fight forever. Carried as the rename it was, nothing is
/// copied twice. Returns the paths it settled.
#[allow(clippy::too_many_lines)]
fn detect_moves(ctx: &Ctx<'_>, names: &mut Names, plan: &mut SyncPlan) -> BTreeSet<String> {
    let mut settled = BTreeSet::new();
    let mut waiting = BTreeSet::new();
    for mover in ctx.members.iter().filter(|m| ctx.sends(m)) {
        // Only paths whose recorded digest is known can be matched; a sync
        // told to check by size alone has none, and falls back to add and
        // delete. Grouped by size, so a tidy of thousands of files looks up
        // its candidates rather than comparing every new file with every
        // vanished one.
        let mut gone: BTreeMap<u64, Vec<(&String, Seen)>> = BTreeMap::new();
        for ((_, path), seen) in ctx
            .baseline
            .range((mover.id, String::new())..)
            .take_while(|((id, _), _)| *id == mover.id)
        {
            if !seen.present || mover.files.contains_key(path) || is_reserved(path) {
                continue;
            }
            if let Some(hash) = recorded_hash(ctx, path, seen) {
                gone.entry(seen.size).or_default().push((
                    path,
                    Seen {
                        hash: Some(hash),
                        ..seen.clone()
                    },
                ));
            }
        }
        if gone.is_empty() {
            continue;
        }
        for (fresh, now) in &mover.files {
            if settled.contains(fresh) || ctx.row(mover, fresh).is_some_and(|s| s.present) {
                continue;
            }
            let candidates: Vec<&(&String, Seen)> = gone
                .get(&now.size)
                .into_iter()
                .flatten()
                .filter(|(old, _)| !settled.contains(*old))
                .collect();
            if candidates.is_empty() {
                continue;
            }
            if too_recent(ctx.mode, Some(now)) {
                for path in std::iter::once(fresh).chain(candidates.iter().map(|(old, _)| *old)) {
                    plan.left_alone.push(LeftAlone {
                        path: path.clone(),
                        why: Why::TooRecent { member: mover.id },
                    });
                    settled.insert(path.clone());
                }
                continue;
            }
            let Some(digest) = ctx
                .digests
                .get(&(mover.id, fresh.clone()))
                .filter(|d| !d.starts_with(SAMPLED))
            else {
                plan.questions.push(Question {
                    member: mover.id,
                    path: fresh.clone(),
                    sampled: false,
                    because: Because::Moved,
                });
                // Decided once the read is in, not as an addition now. Kept
                // apart from `settled` so every member's reads are asked for
                // in the same round.
                waiting.insert(fresh.clone());
                waiting.extend(candidates.iter().map(|(old, _)| (*old).clone()));
                continue;
            };
            let Some((old, seen)) = candidates
                .iter()
                .find(|(_, seen)| seen.hash.as_ref() == Some(digest))
                .map(|pair| (pair.0, &pair.1))
            else {
                continue;
            };
            if !cleanly_moved(ctx, mover, old, seen, fresh) {
                continue;
            }
            settled.insert(old.clone());
            settled.insert(fresh.clone());
            carry_move(ctx, mover, old, fresh, now.size, digest, names, plan);
        }
    }
    settled.append(&mut waiting);
    settled
}

/// The digest `path` was recorded with on the member that recorded `seen`,
/// or failing that on any member that recorded it at the same size. The
/// member a file was first sent from never read it; the members it was sent
/// to have the digest the copy computed.
fn recorded_hash(ctx: &Ctx<'_>, path: &str, seen: &Seen) -> Option<String> {
    seen.hash.clone().or_else(|| {
        ctx.members.iter().find_map(|m| {
            ctx.row(m, path)
                .filter(|row| row.present && row.size == seen.size)
                .and_then(|row| row.hash.clone())
        })
    })
}

/// Whether nobody else has touched either name since: every other member
/// either lacks the old name or holds it as recorded and will receive the
/// rename, and none holds the new one.
fn cleanly_moved(ctx: &Ctx<'_>, mover: &Member, old: &str, seen: &Seen, fresh: &str) -> bool {
    ctx.members
        .iter()
        .filter(|m| m.id != mover.id)
        .all(|other| {
            let fresh_free = !other.files.contains_key(fresh)
                && !ctx.row(other, fresh).is_some_and(|s| s.present);
            let old_ok = match (other.files.get(old), ctx.row(other, old)) {
                (None, row) => !row.is_some_and(|s| s.present),
                (Some(now), Some(row)) if row.present => {
                    ctx.receives(other)
                        && changed(other, old, row, now, ctx.digests) == Some(false)
                        && row.size == seen.size
                        && match (&row.hash, &seen.hash) {
                            (Some(x), Some(y)) => x == y,
                            _ => true,
                        }
                }
                (Some(_), _) => false,
            };
            fresh_free && old_ok
        })
}

#[allow(clippy::too_many_arguments)]
fn carry_move(
    ctx: &Ctx<'_>,
    mover: &Member,
    old: &str,
    fresh: &str,
    size: u64,
    digest: &str,
    names: &mut Names,
    plan: &mut SyncPlan,
) {
    let receivers: Vec<&Member> = ctx
        .members
        .iter()
        .filter(|m| m.id != mover.id && ctx.receives(m))
        .collect();
    let stuck: Vec<&Member> = receivers
        .iter()
        .copied()
        .filter(|m| {
            m.files.contains_key(old) && !m.renames && ctx.mode.on_remove == OnRemove::SetAside
        })
        .collect();
    if !stuck.is_empty() {
        for member in stuck {
            for path in [old, fresh] {
                plan.left_alone.push(LeftAlone {
                    path: path.to_string(),
                    why: Why::NeedsRename { member: member.id },
                });
            }
        }
        return;
    }
    let hash = Some(digest.to_string());
    for to in receivers {
        if to.files.contains_key(old) && to.renames {
            plan.ops.push(SyncOp::Rename {
                member: to.id,
                from: old.to_string(),
                to: fresh.to_string(),
                hash: hash.clone(),
            });
        } else {
            plan.ops.push(SyncOp::Copy {
                from: mover.id,
                to: to.id,
                path: fresh.to_string(),
                size,
            });
            if to.files.contains_key(old) {
                plan.ops.push(SyncOp::Remove {
                    member: to.id,
                    path: old.to_string(),
                    how: Removal::Delete,
                    because: Removed::Moved,
                });
            }
        }
        names.hold(to.id, fresh);
    }
    plan.ops.push(record_absent(mover, old));
    plan.ops.push(SyncOp::Record {
        member: mover.id,
        path: fresh.to_string(),
        seen: Seen {
            present: true,
            size,
            mtime: mover.files[fresh].mtime,
            hash,
        },
    });
}

/// Whether a present file differs from its own last reading. `None` when
/// only its bytes can say and they have not been read.
fn changed(member: &Member, path: &str, seen: &Seen, now: &Now, digests: &Digests) -> Option<bool> {
    if seen.size != now.size {
        return Some(true);
    }
    match (seen.mtime, now.mtime) {
        (Some(was), Some(is)) => Some(was != is),
        // No clock on this member: judge by bytes, where there is a digest to
        // judge against. With none recorded, an unchanged size is all there is.
        _ => match &seen.hash {
            None => Some(false),
            Some(was) => digests
                .get(&(member.id, path.to_string()))
                .map(|is| is != was),
        },
    }
}

enum Agreement {
    Same,
    Different,
    Ask(Vec<Question>),
}

/// Whether two members hold the same content at `path`.
///
/// Two members that have not moved since last run are compared by what was
/// recorded, never read again: that is most files on most runs, and reading
/// them would make every run as slow as the first.
fn agree(ctx: &Ctx<'_>, lead: &Member, other: &Member, path: &str, both_still: bool) -> Agreement {
    let first_contact = first_meeting(ctx, lead, other, path);
    let by_record = || {
        if same_content(&ctx.seen_of(lead, path), &ctx.seen_of(other, path)) {
            Agreement::Same
        } else {
            Agreement::Different
        }
    };
    if both_still {
        return by_record();
    }
    let (Some(a), Some(b)) = (lead.files.get(path), other.files.get(path)) else {
        // One of them holds it only in the baseline: compare what was recorded.
        return by_record();
    };
    if a.size != b.size {
        return Agreement::Different;
    }
    let verdict = |x: String, y: String| {
        if x == y {
            Agreement::Same
        } else {
            Agreement::Different
        }
    };
    // Sampled digests only ever meet sampled ones: the two kinds differ for
    // the same bytes.
    let whole = |member: &Member| {
        ctx.digests
            .get(&(member.id, path.to_string()))
            .filter(|d| !d.starts_with(SAMPLED))
            .cloned()
            .or_else(|| ctx.seen_of(member, path).hash)
    };
    let part = |member: &Member| {
        ctx.digests
            .get(&(member.id, path.to_string()))
            .filter(|d| d.starts_with(SAMPLED))
            .cloned()
    };
    // Size only means size only, even where bytes happen to have been read
    // for another reason: a verdict that depended on incidental reads would
    // be a different verdict on the next run, which has none.
    if first_contact && ctx.mode.first_check == FirstCheck::Size {
        return Agreement::Same;
    }
    let (x, y) = (whole(lead), whole(other));
    if let (Some(x), Some(y)) = (x.clone(), y.clone()) {
        return verdict(x, y);
    }
    // Sampling is only worth it while neither has been read in full; once one
    // has, reading the other in full is what makes the two comparable.
    let sampled =
        first_contact && ctx.mode.first_check == FirstCheck::Sampled && x.is_none() && y.is_none();
    if sampled && let (Some(x), Some(y)) = (part(lead), part(other)) {
        return verdict(x, y);
    }
    let mut questions = Vec::new();
    for member in [lead, other] {
        let known = if sampled { part(member) } else { whole(member) };
        if known.is_none() {
            questions.push(Question {
                member: member.id,
                path: path.to_string(),
                sampled,
                because: Because::Agreement,
            });
        }
    }
    Agreement::Ask(questions)
}

/// What a member holds at `path` now, as it would be recorded.
fn seen_of(member: &Member, path: &str, baseline: &Baseline, digests: &Digests) -> Seen {
    let key = (member.id, path.to_string());
    match member.files.get(path) {
        Some(now) => Seen {
            present: true,
            size: now.size,
            mtime: now.mtime,
            hash: digests
                .get(&key)
                .filter(|d| !d.starts_with(SAMPLED))
                .cloned()
                .or_else(|| {
                    // Unchanged since recorded, so the recorded digest still holds.
                    baseline
                        .get(&key)
                        .filter(|seen| {
                            seen.present && seen.size == now.size && seen.mtime == now.mtime
                        })
                        .and_then(|seen| seen.hash.clone())
                }),
        },
        None => baseline.get(&key).cloned().unwrap_or_else(absent),
    }
}

/// Whether two readings describe the same content, as far as they can say.
fn same_content(a: &Seen, b: &Seen) -> bool {
    a.present == b.present
        && a.size == b.size
        && match (&a.hash, &b.hash) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}

fn absent() -> Seen {
    Seen {
        present: false,
        size: 0,
        mtime: None,
        hash: None,
    }
}

/// Where a copy reads from: a member on this machine if one holds it, then one
/// on the target's own connection, then the first that does.
fn route<'a>(holders: &[&'a Member], to: &Member) -> &'a Member {
    holders
        .iter()
        .find(|h| h.local)
        .or_else(|| {
            holders
                .iter()
                .find(|h| h.connection.is_some() && h.connection == to.connection)
        })
        .copied()
        .unwrap_or(holders[0])
}

/// Copies and parks grouped by pair, pairs in member order, paths in path
/// order.
fn legs(members: &[Member], ops: &[SyncOp]) -> Vec<Leg> {
    let position = |id: MemberId| {
        members
            .iter()
            .position(|m| m.id == id)
            .unwrap_or(usize::MAX)
    };
    let mut grouped: BTreeMap<(usize, usize), Leg> = BTreeMap::new();
    for op in ops {
        let (from, to, path, size, parked) = match op {
            SyncOp::Copy {
                from,
                to,
                path,
                size,
            } => (*from, *to, path, *size, None),
            SyncOp::Park {
                from,
                to,
                path,
                parked,
                size,
            } => (*from, *to, path, *size, Some(parked)),
            _ => continue,
        };
        let leg = grouped
            .entry((position(from), position(to)))
            .or_insert_with(|| Leg {
                from,
                to,
                paths: Vec::new(),
                bytes: 0,
                parked: BTreeMap::new(),
            });
        leg.paths.push(path.clone());
        leg.bytes += size;
        if let Some(parked) = parked {
            leg.parked.insert(path.clone(), parked.clone());
        }
    }
    grouped.into_values().collect()
}

fn blast(members: &[Member], ops: &[SyncOp]) -> BTreeMap<MemberId, MemberBlast> {
    let mut blast: BTreeMap<MemberId, MemberBlast> = members
        .iter()
        .map(|m| {
            (
                m.id,
                MemberBlast {
                    of: m.files.len(),
                    ..MemberBlast::default()
                },
            )
        })
        .collect();
    for op in ops {
        match op {
            SyncOp::Copy { from, to, size, .. } => {
                if let Some(target) = blast.get_mut(to) {
                    target.arriving += 1;
                    target.arriving_bytes += size;
                }
                if let Some(source) = blast.get_mut(from) {
                    source.leaving += 1;
                }
            }
            SyncOp::Park { to, .. } => {
                if let Some(target) = blast.get_mut(to) {
                    target.parking += 1;
                }
            }
            SyncOp::Remove {
                member,
                how,
                because,
                ..
            } => {
                let Some(target) = blast.get_mut(member) else {
                    continue;
                };
                match because {
                    Removed::Superseded => target.replacing += 1,
                    // The other half of a rename, counted as one.
                    Removed::Moved => target.renaming += 1,
                    Removed::Deleted | Removed::Extra | Removed::Forgotten => target.removing += 1,
                }
                if *how == Removal::Delete && *because != Removed::Moved {
                    target.deleting += 1;
                }
            }
            SyncOp::Rename { member, .. } => {
                if let Some(target) = blast.get_mut(member) {
                    target.renaming += 1;
                }
            }
            SyncOp::Record { .. } => {}
        }
    }
    for member in blast.values_mut() {
        // A replaced version counts: a member restored from an old backup
        // looks like every file edited, and would otherwise all be replaced.
        let taken_off = member.removing + member.replacing;
        member.over_limit = taken_off > 0 && Blast::over(taken_off, member.of);
    }
    blast
}

/// Play a plan out on paper: every op in order, every copy landing with the
/// source's size and time, and every record remembered. What the property
/// tests decide again, expecting nothing.
#[must_use]
pub fn apply_to(
    members: &[Member],
    baseline: &Baseline,
    digests: &Digests,
    plan: &SyncPlan,
) -> (Vec<Member>, Baseline) {
    let mut after: Vec<Member> = members.to_vec();
    let mut remembered = baseline.clone();
    let find = |after: &[Member], id: MemberId| after.iter().position(|m| m.id == id);
    for op in &plan.ops {
        match op {
            SyncOp::Copy { from, to, path, .. } => {
                let Some(source) = find(&after, *from) else {
                    continue;
                };
                let seen = seen_of(&after[source], path, &remembered, digests);
                if let Some(now) = after[source].files.get(path).copied()
                    && let Some(target) = find(&after, *to)
                {
                    after[target].files.insert(path.clone(), now);
                }
                remembered.insert((*to, path.clone()), seen);
            }
            SyncOp::Park {
                from,
                to,
                path,
                parked,
                ..
            } => {
                if let Some(source) = find(&after, *from)
                    && let Some(now) = after[source].files.get(path).copied()
                    && let Some(target) = find(&after, *to)
                {
                    after[target].parked.insert(parked.clone(), now);
                }
            }
            SyncOp::Remove {
                member, path, how, ..
            } => {
                if let Some(target) = find(&after, *member)
                    && let Some(now) = after[target].files.remove(path)
                    && let Removal::SetAside { to } = how
                {
                    after[target].parked.insert(to.clone(), now);
                }
                remembered.insert((*member, path.clone()), absent());
            }
            SyncOp::Rename {
                member,
                from,
                to,
                hash,
            } => {
                if let Some(target) = find(&after, *member)
                    && let Some(now) = after[target].files.remove(from)
                {
                    after[target].files.insert(to.clone(), now);
                    remembered.insert(
                        (*member, to.clone()),
                        Seen {
                            present: true,
                            size: now.size,
                            mtime: now.mtime,
                            hash: hash.clone(),
                        },
                    );
                }
                remembered.insert((*member, from.clone()), absent());
            }
            SyncOp::Record { member, path, seen } => {
                remembered.insert((*member, path.clone()), seen.clone());
            }
        }
    }
    (after, remembered)
}
