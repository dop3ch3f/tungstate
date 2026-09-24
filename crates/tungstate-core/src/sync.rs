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

use crate::snapshot::{Snapshot, is_reserved};

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
    /// Every file, by root-relative path with forward slashes.
    pub files: BTreeMap<String, Now>,
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
}

/// Something a run does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOp {
    /// Carry `path` from one member to another, replacing whatever the target
    /// held there. What it held is set aside by the engine, never lost.
    Copy {
        from: MemberId,
        to: MemberId,
        path: String,
        size: u64,
    },
    /// Remember what a member holds, and move nothing. A member that already
    /// agrees is acknowledged, so next run it is not read again.
    Record {
        member: MemberId,
        path: String,
        seen: Seen,
    },
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
    /// Two or more members changed it differently. Asked about in 9c; left
    /// alone until then, and never resolved by comparing clocks.
    Conflict { members: Vec<MemberId> },
    /// The target holds a name differing only in case, and cannot hold both.
    CaseClash { member: MemberId, existing: String },
}

/// A path left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeftAlone {
    pub path: String,
    pub why: Why,
}

/// What one member would see happen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberBlast {
    /// Files copied into it.
    pub arriving: usize,
    /// Bytes in those.
    pub arriving_bytes: u64,
    /// Files copied out of it to somewhere else.
    pub leaving: usize,
    /// Files whose current version it holds would be replaced, the old one
    /// set aside. A subset of `arriving`, said separately because it is the
    /// number that means something of yours changes.
    pub replacing: usize,
    /// Files it holds.
    pub of: usize,
}

/// Every copy between one pair of members, in the order they will run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leg {
    pub from: MemberId,
    pub to: MemberId,
    pub paths: Vec<String>,
    pub bytes: u64,
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
}

impl SyncPlan {
    /// Whether nothing would move and nothing needs remembering.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
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

/// Decide a run. See the module docs and `docs/slices/09b-folder-sync.md`.
#[must_use]
pub fn decide(members: &[Member], baseline: &Baseline, digests: &Digests, mode: &Mode) -> SyncPlan {
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
        .filter(|path| !is_reserved(path) && !is_partial(path))
        .collect();

    // What each case-insensitive member will hold, folded, so a copy that
    // would collide with a name differing only in case is caught before it
    // lands on top of the wrong file.
    let mut folded: BTreeMap<MemberId, BTreeMap<String, String>> = members
        .iter()
        .filter(|member| !member.case_sensitive)
        .map(|member| {
            (
                member.id,
                member
                    .files
                    .keys()
                    .map(|path| (path.to_lowercase(), path.clone()))
                    .collect(),
            )
        })
        .collect();

    for path in paths {
        decide_path(
            members,
            baseline,
            digests,
            mode,
            path,
            &mut folded,
            &mut plan,
        );
    }

    plan.questions.sort();
    plan.questions.dedup();
    plan.left_alone.sort();
    plan.legs = legs(members, &plan.ops);
    plan.blast = blast(members, &plan.ops);
    plan
}

#[allow(clippy::too_many_lines)]
fn decide_path(
    members: &[Member],
    baseline: &Baseline,
    digests: &Digests,
    mode: &Mode,
    path: &str,
    folded: &mut BTreeMap<MemberId, BTreeMap<String, String>>,
    plan: &mut SyncPlan,
) {
    let key = |member: &Member| (member.id, path.to_string());
    let first_contact = members.iter().all(|m| !baseline.contains_key(&key(m)));

    // 1. Each member against its own baseline.
    let mut states = Vec::with_capacity(members.len());
    let mut unread = false;
    for member in members {
        let now = member.files.get(path);
        let seen = baseline.get(&key(member)).filter(|seen| seen.present);
        let state = match (seen, now) {
            (None, Some(_)) => State::New,
            (None, None) if baseline.contains_key(&key(member)) => State::Gone,
            (None, None) => State::Missing,
            (Some(_), None) => State::Deleted,
            (Some(seen), Some(now)) => match changed(member, path, seen, now, digests) {
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
        if let Some(Now {
            mtime: Some(at), ..
        }) = now
            && state.moved()
            && mode.now_ms - at < mode.cooldown_ms
        {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::TooRecent { member: member.id },
            });
            return;
        }
        states.push((member, state));
    }
    // Every member that needs reading is asked for at once, so a path costs
    // one round of reads rather than one per member.
    if unread {
        return;
    }
    let state_of = |id: MemberId| states.iter().find(|(m, _)| m.id == id).map(|(_, s)| *s);

    // 2. Who may send, and who may receive.
    let anchored = |member: &Member| mode.anchor == Some(member.id);
    let sends = |member: &Member| match mode.direction {
        Direction::All => true,
        Direction::Push => anchored(member),
        Direction::Pull => !anchored(member),
    };
    let receives = |member: &Member| match mode.direction {
        Direction::All => true,
        Direction::Push => !anchored(member),
        Direction::Pull => anchored(member),
    };

    // A change on a member that may not send goes nowhere, and is said. Said
    // at the end, not here: one that turns out to match what is being
    // delivered is simply remembered, and reporting it would be a false alarm.
    let unsent: Vec<&Member> = states
        .iter()
        .filter(|(member, state)| state.moved() && !sends(member))
        .map(|(member, _)| *member)
        .collect();
    let report_unsent = |plan: &mut SyncPlan, except: &dyn Fn(&Member) -> bool| {
        for member in unsent.iter().filter(|m| !except(m)) {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: if anchored(member) {
                    Why::OnlyTheAnchorReceives { member: member.id }
                } else {
                    Why::OnlyTheAnchorSends { member: member.id }
                },
            });
        }
    };

    // 3. The desired content: what moved on a sender, or failing that what a
    //    sender still holds unchanged.
    let changed_senders: Vec<&Member> = states
        .iter()
        .filter(|(member, state)| state.moved() && sends(member))
        .map(|(member, _)| *member)
        .collect();
    let holders: Vec<&Member> = if changed_senders.is_empty() {
        states
            .iter()
            .filter(|(member, state)| *state == State::Unchanged && sends(member))
            .map(|(member, _)| *member)
            .collect()
    } else {
        changed_senders
    };

    let Some(lead) = holders.first().copied() else {
        report_unsent(plan, &|_| false);
        // Nothing to deliver. A deletion that stands is remembered, so it is
        // not reported again next run.
        for (member, state) in &states {
            if *state == State::Deleted {
                plan.ops.push(SyncOp::Record {
                    member: member.id,
                    path: path.to_string(),
                    seen: absent(),
                });
            }
        }
        return;
    };

    // Everyone whose content has to be compared with the lead's: the other
    // holders, and any receiver that moved on its own.
    let mut must_agree: Vec<&Member> = holders[1..].to_vec();
    for (member, state) in &states {
        if state.moved() && receives(member) && !holders.iter().any(|h| h.id == member.id) {
            must_agree.push(member);
        }
    }
    let mut asked = false;
    let mut disagree = Vec::new();
    for other in &must_agree {
        let both_still = state_of(lead.id) == Some(State::Unchanged)
            && state_of(other.id) == Some(State::Unchanged);
        match agree(
            lead,
            other,
            path,
            baseline,
            digests,
            mode,
            first_contact,
            both_still,
        ) {
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
    if !disagree.is_empty() {
        report_unsent(plan, &|m| disagree.contains(&m.id));
        let mut members: Vec<MemberId> = vec![lead.id];
        members.extend(disagree);
        plan.left_alone.push(LeftAlone {
            path: path.to_string(),
            why: Why::Conflict { members },
        });
        return;
    }

    // 4. Deliver to every receiver not already holding it; remember the rest.
    let content = seen_of(lead, path, baseline, digests);
    let lead_moved = state_of(lead.id).is_some_and(State::moved);
    report_unsent(plan, &|m| must_agree.iter().any(|a| a.id == m.id));
    let holds = |member: &Member| {
        holders.iter().any(|h| h.id == member.id) || must_agree.iter().any(|m| m.id == member.id)
    };
    for (member, state) in &states {
        if holds(member) {
            if *state != State::Unchanged {
                plan.ops.push(SyncOp::Record {
                    member: member.id,
                    path: path.to_string(),
                    seen: seen_of(member, path, baseline, digests),
                });
            }
            continue;
        }
        if !receives(member) {
            // A deletion on a member that does not receive stands.
            if *state == State::Deleted {
                plan.ops.push(SyncOp::Record {
                    member: member.id,
                    path: path.to_string(),
                    seen: absent(),
                });
            }
            continue;
        }
        if *state == State::Unchanged
            && baseline
                .get(&(member.id, path.to_string()))
                .is_some_and(|seen| {
                    // Against content that just moved, only matching digests say
                    // it is already here: an edit can keep the size. Against
                    // content that did not move, the two readings are enough.
                    match (&seen.hash, &content.hash) {
                        (Some(x), Some(y)) => x == y,
                        _ => !lead_moved && same_content(seen, &content),
                    }
                })
        {
            continue;
        }
        if let Some(existing) = folded
            .get(&member.id)
            .and_then(|names| names.get(&path.to_lowercase()))
            .filter(|existing| existing.as_str() != path)
        {
            plan.left_alone.push(LeftAlone {
                path: path.to_string(),
                why: Why::CaseClash {
                    member: member.id,
                    existing: existing.clone(),
                },
            });
            continue;
        }
        let from = route(&holders, member);
        plan.ops.push(SyncOp::Copy {
            from: from.id,
            to: member.id,
            path: path.to_string(),
            size: content.size,
        });
        if let Some(names) = folded.get_mut(&member.id) {
            names.insert(path.to_lowercase(), path.to_string());
        }
    }
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
#[allow(clippy::too_many_arguments)]
fn agree(
    lead: &Member,
    other: &Member,
    path: &str,
    baseline: &Baseline,
    digests: &Digests,
    mode: &Mode,
    first_contact: bool,
    both_still: bool,
) -> Agreement {
    if both_still {
        let a = seen_of(lead, path, baseline, digests);
        let b = seen_of(other, path, baseline, digests);
        return if same_content(&a, &b) {
            Agreement::Same
        } else {
            Agreement::Different
        };
    }
    let (Some(a), Some(b)) = (lead.files.get(path), other.files.get(path)) else {
        // One of them holds it only in the baseline: compare what was recorded.
        let a = seen_of(lead, path, baseline, digests);
        let b = seen_of(other, path, baseline, digests);
        return if same_content(&a, &b) {
            Agreement::Same
        } else {
            Agreement::Different
        };
    };
    if a.size != b.size {
        return Agreement::Different;
    }
    if first_contact && mode.first_check == FirstCheck::Size {
        return Agreement::Same;
    }
    let sampled = first_contact && mode.first_check == FirstCheck::Sampled;
    // What was read this run, sampled or not, and failing that the recorded
    // digest of a member unchanged since it was recorded. Only remembering
    // drops sampled digests; comparing within one run needs them.
    let known = |member: &Member| {
        digests
            .get(&(member.id, path.to_string()))
            .cloned()
            .or_else(|| seen_of(member, path, baseline, digests).hash)
    };
    match (known(lead), known(other)) {
        (Some(x), Some(y)) => {
            if x == y {
                Agreement::Same
            } else {
                Agreement::Different
            }
        }
        (x, y) => {
            let mut questions = Vec::new();
            for (id, known) in [(lead.id, x), (other.id, y)] {
                if known.is_none() {
                    questions.push(Question {
                        member: id,
                        path: path.to_string(),
                        sampled,
                        because: Because::Agreement,
                    });
                }
            }
            Agreement::Ask(questions)
        }
    }
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

/// Copies grouped by pair, pairs in member order, paths in path order.
fn legs(members: &[Member], ops: &[SyncOp]) -> Vec<Leg> {
    let position = |id: MemberId| {
        members
            .iter()
            .position(|m| m.id == id)
            .unwrap_or(usize::MAX)
    };
    let mut grouped: BTreeMap<(usize, usize), Leg> = BTreeMap::new();
    for op in ops {
        if let SyncOp::Copy {
            from,
            to,
            path,
            size,
        } = op
        {
            let leg = grouped
                .entry((position(*from), position(*to)))
                .or_insert_with(|| Leg {
                    from: *from,
                    to: *to,
                    paths: Vec::new(),
                    bytes: 0,
                });
            leg.paths.push(path.clone());
            leg.bytes += size;
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
        if let SyncOp::Copy {
            from,
            to,
            path,
            size,
        } = op
        {
            if let Some(target) = blast.get_mut(to) {
                target.arriving += 1;
                target.arriving_bytes += size;
            }
            let replacing = members
                .iter()
                .find(|m| m.id == *to)
                .is_some_and(|m| m.files.contains_key(path));
            if replacing && let Some(target) = blast.get_mut(to) {
                target.replacing += 1;
            }
            if let Some(source) = blast.get_mut(from) {
                source.leaving += 1;
            }
        }
    }
    blast
}

/// Play a plan out on paper: every copy lands with the source's size and
/// time, and every record is remembered. What the property tests decide
/// again, expecting nothing.
#[must_use]
pub fn apply_to(
    members: &[Member],
    baseline: &Baseline,
    digests: &Digests,
    plan: &SyncPlan,
) -> (Vec<Member>, Baseline) {
    let mut after: Vec<Member> = members.to_vec();
    let mut remembered = baseline.clone();
    for op in &plan.ops {
        match op {
            SyncOp::Copy { from, to, path, .. } => {
                let Some(source) = members.iter().find(|m| m.id == *from) else {
                    continue;
                };
                let seen = seen_of(source, path, baseline, digests);
                if let Some(now) = source.files.get(path).copied()
                    && let Some(target) = after.iter_mut().find(|m| m.id == *to)
                {
                    target.files.insert(path.clone(), now);
                }
                remembered.insert((*to, path.clone()), seen);
            }
            SyncOp::Record { member, path, seen } => {
                remembered.insert((*member, path.clone()), seen.clone());
            }
        }
    }
    (after, remembered)
}
