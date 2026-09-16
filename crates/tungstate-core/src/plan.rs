//! What would have to happen to a folder, and in what order.
//!
//! Pure, like the classifier it reads: a [`Snapshot`] goes in and a [`Plan`]
//! comes out, with no filesystem anywhere near it. The planner reads the same
//! [`Outcome`] that `tungstate explain` prints, so what the command line showed
//! you is what the plan does.
//!
//! **No operation here removes content.** There is no `Trash` and no `Delete`:
//! `on_conflict = "replace"` parks the existing file under
//! [`snapshot::QUARANTINE`], which is what the drain already does and what its
//! comment argues for. That is why "nothing is lost" is trivially true of a
//! plan rather than carefully true.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::attrs::Attributes;
use crate::classify::Outcome;
use crate::graph::Graph;
use crate::policy::{Mode, OnConflict, Policy};
use crate::snapshot::{self, Snapshot};
use crate::template::{canonical_path, sanitize};

/// One thing the plan would do.
///
/// Every path is root-relative with forward slashes, and no op can name a path
/// outside the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Create a directory a move needs.
    MkDir {
        /// The directory.
        path: String,
    },
    /// Move a file to where the policy says it belongs.
    Move {
        /// Where it is.
        from: String,
        /// Where it should be.
        to: String,
        /// Why.
        because: Because,
    },
    /// Park a file the planner will not decide about.
    Quarantine {
        /// Where it is.
        from: String,
        /// Under [`snapshot::QUARANTINE`].
        to: String,
        /// Why.
        because: Parked,
    },
    /// Remove a directory this plan's own moves emptied.
    RmDir {
        /// The directory.
        path: String,
    },
}

/// Why a file moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Because {
    /// A rule routed it.
    Rule {
        /// The rule's name.
        name: String,
    },
    /// No rule matched, and `[folder].inbox` says where those go.
    Inbox,
    /// Something wanted this name first, so this one is numbered.
    Numbered {
        /// The name it asked for.
        wanted: String,
    },
    /// Half of a swap: out to a temporary name, so two files can trade places.
    MakeWay,
}

/// Why a file is parked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Parked {
    /// `on_conflict = "quarantine"`: a different file holds the name.
    Conflict {
        /// The file that holds it.
        holder: String,
    },
    /// `on_conflict = "replace"`: moved aside so the newcomer can have the name.
    Replaced {
        /// The file taking its place.
        by: String,
    },
}

/// A file the plan leaves alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Untouched {
    /// The file.
    pub file: String,
    /// Why nothing happens to it.
    pub reason: Reason,
}

/// Why a file is left alone.
///
/// The first four are the non-`Routed` [`Outcome`] variants; the rest are what
/// the planner decides for itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// Already where the policy wants it.
    AlreadyThere,
    /// `[folder].ignore`, or `symlinks = "ignore"`.
    Ignored {
        /// The pattern or setting that said so.
        because: String,
    },
    /// Inside a unit `[folder].opaque` says never to look inside.
    Opaque {
        /// The pattern that named the unit.
        pattern: String,
    },
    /// A rule took it but a variable it needs has no value.
    Unresolvable {
        /// The rule.
        rule: String,
        /// The variable that stopped it.
        var: String,
        /// Why.
        reason: String,
    },
    /// No rule took it, and there is no inbox.
    Unmatched,
    /// Written too recently; `[defaults].cooldown` has not elapsed.
    Cooling {
        /// How much longer to wait.
        seconds_left: u64,
    },
    /// `on_conflict = "skip"`, and a different file holds the name.
    Conflicted {
        /// The file that holds it.
        holder: String,
    },
    /// Something that is not going anywhere is in the way.
    Blocked {
        /// What is in the way.
        by: String,
        /// What kind of obstacle it is.
        detail: String,
    },
}

/// How much of the folder a plan would move.
///
/// The limit lives here rather than in the command line so that `plan` and
/// `apply` cannot come to different conclusions about what "too much" means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blast {
    /// Files that would move or be parked.
    pub files: usize,
    /// Files seen.
    pub of: usize,
    /// Bytes those files add up to.
    pub bytes: u64,
    /// Directories that would be created.
    pub created: usize,
    /// Directories that would be removed.
    pub removed: usize,
    /// Whether this is past [`Blast::LIMIT_FILES`] or [`Blast::LIMIT_SHARE`].
    pub over_limit: bool,
}

impl Blast {
    /// More files than this trips the breaker (DESIGN §4, rail 3).
    pub const LIMIT_FILES: usize = 500;
    /// One file in this many trips it too: a fifth of the folder.
    pub const LIMIT_SHARE: usize = 5;

    fn over(files: usize, of: usize) -> bool {
        // Integer arithmetic rather than a ratio in floating point, which
        // `clippy::pedantic` rightly refuses for a count this large.
        files > Self::LIMIT_FILES || (of > 0 && files * Self::LIMIT_SHARE >= of)
    }
}

/// An ordered set of operations that would eliminate a folder's drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// `[folder].name`, so a saved plan says which folder it is for.
    pub folder: String,
    /// The folder's mode. `plan` ignores it; `apply` will not.
    pub mode: Mode,
    /// When the snapshot was taken.
    pub taken: Timestamp,
    /// A fingerprint of the snapshot this was built from, so `apply` can tell
    /// that the folder has moved on and replan rather than act on stale facts.
    pub snapshot: String,
    /// The operations, in an order that can actually be carried out.
    pub ops: Vec<Op>,
    /// Every file the plan does not touch, and why.
    pub untouched: Vec<Untouched>,
    /// How much this would move.
    pub blast: Blast,
    /// Whether carrying this out would leave the folder with nothing more to
    /// do. False means the policy does not converge — see [`Plan::unsettled`].
    pub settles: bool,
    /// Files that would still want to move after this plan was carried out.
    ///
    /// Empty when [`Plan::settles`]. A rename template that reads `{name}` and
    /// adds to it — `"copy-{name}"`, or anything that survives `sanitize` as a
    /// prefix — renders differently once the file has been renamed, so the
    /// next pass moves it again, and the next. Harmless in a dry run and not
    /// harmless at all under a daemon in `enforce` mode, so it is found here
    /// and reported rather than discovered in the logs.
    pub unsettled: Vec<String>,
}

/// Anything that stops a plan being applied on paper.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// An op's precondition did not hold when the plan reached it.
    #[error("`{op}` cannot run here: {why}")]
    OutOfOrder {
        /// The operation, described.
        op: String,
        /// What was not true.
        why: String,
    },
}

impl Op {
    /// Where this op takes a file from, if it moves one.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        match self {
            Op::Move { from, .. } | Op::Quarantine { from, .. } => Some(from),
            Op::MkDir { .. } | Op::RmDir { .. } => None,
        }
    }

    /// Where this op puts a file, if it moves one.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        match self {
            Op::Move { to, .. } | Op::Quarantine { to, .. } => Some(to),
            Op::MkDir { .. } | Op::RmDir { .. } => None,
        }
    }

    /// The directory this op creates or removes, if it does either.
    #[must_use]
    pub fn directory(&self) -> Option<&str> {
        match self {
            Op::MkDir { path } | Op::RmDir { path } => Some(path),
            Op::Move { .. } | Op::Quarantine { .. } => None,
        }
    }

    /// The total order ops are canonicalised into before the graph is built.
    ///
    /// Sorting first means the graph's node indices *are* content order, so
    /// breaking a tie by index is breaking it by content — which is what makes
    /// the same folder give byte-identical output every time.
    fn ordering(&self) -> (u8, &str, &str) {
        match self {
            Op::MkDir { path } => (0, path.as_str(), ""),
            Op::Move { from, to, .. } => (1, from.as_str(), to.as_str()),
            Op::Quarantine { from, to, .. } => (2, from.as_str(), to.as_str()),
            Op::RmDir { path } => (3, path.as_str(), ""),
        }
    }
}

/// A file that wants to be somewhere else.
struct Want {
    from: String,
    to: String,
    because: Because,
    size: u64,
}

impl Policy {
    /// Everything that would have to happen for this folder to match this
    /// policy, in an order that can be carried out.
    ///
    /// Pure: the same snapshot always gives the same plan, down to the bytes.
    #[must_use]
    pub fn plan(&self, snap: &Snapshot) -> Plan {
        let mut plan = self.build(snap);
        // Carry it out on paper and plan again. The check costs a second pass
        // over a folder nothing has been done to, which is a fair price for
        // being the only way to know a policy settles: convergence is a
        // property of the policy and the folder together, so no amount of
        // reading the policy alone can answer it.
        if let Ok(after) = plan.apply_to(snap) {
            let again = self.build(&after);
            plan.settles = again.ops.is_empty();
            plan.unsettled = again
                .ops
                .iter()
                .filter_map(Op::source)
                .map(String::from)
                .collect();
        }
        plan
    }

    /// The plan itself, without asking whether it settles. Separate so the
    /// check can call it without calling itself.
    fn build(&self, snap: &Snapshot) -> Plan {
        let mut untouched = Vec::new();
        let wants = self.wanted(snap, &mut untouched);
        let (mut ops, mut untouched) = self.assign(snap, wants, untouched);
        ops.extend(directory_ops(snap, &ops));
        untouched.sort_by(|a, b| a.file.cmp(&b.file));

        let blast = measure(snap, &ops);
        let ops = order(ops, &snap.occupied());

        Plan {
            folder: self.folder.name.clone(),
            mode: self.folder.mode,
            taken: snap.taken,
            snapshot: fingerprint(snap),
            ops,
            untouched,
            blast,
            settles: true,
            unsettled: Vec::new(),
        }
    }

    /// Phase 1: where every file would like to be, and why the rest stay put.
    fn wanted(&self, snap: &Snapshot, untouched: &mut Vec<Untouched>) -> Vec<Want> {
        let mut wants = Vec::new();
        let cooldown = self.defaults.cooldown;

        for entry in &snap.entries {
            let file = entry.relative_path();
            // Directories are occupants, never movers. Moving an opaque unit
            // as a whole needs rules that match directories, which is a later
            // slice; `pre_rules` returns before any rule sees one.
            if entry.is_dir {
                continue;
            }

            // Before anything else, and the same rule the drain applies, so
            // both "what would happen" commands hold a file back for the same
            // reason rather than for two that nearly agree.
            if let Some(left) = cooling(entry, snap.taken, cooldown) {
                untouched.push(Untouched {
                    file,
                    reason: Reason::Cooling { seconds_left: left },
                });
                continue;
            }

            let (to, because) = match self.place(entry) {
                Outcome::Ignored { because } => {
                    untouched.push(Untouched {
                        file,
                        reason: Reason::Ignored { because },
                    });
                    continue;
                }
                Outcome::Opaque { pattern } => {
                    untouched.push(Untouched {
                        file,
                        reason: Reason::Opaque { pattern },
                    });
                    continue;
                }
                Outcome::Unresolvable {
                    rule, var, reason, ..
                } => {
                    untouched.push(Untouched {
                        file,
                        reason: Reason::Unresolvable { rule, var, reason },
                    });
                    continue;
                }
                Outcome::Unmatched { inbox: None } => {
                    untouched.push(Untouched {
                        file,
                        reason: Reason::Unmatched,
                    });
                    continue;
                }
                Outcome::Unmatched { inbox: Some(inbox) } => {
                    let directory = canonical_path(&inbox);
                    // A file already in the inbox stays there. Without this a
                    // file in `_inbox` plans into `_inbox/_inbox`, and the
                    // second plan is not empty — the one thing a reconciler
                    // must never do.
                    if entry.parent == directory {
                        untouched.push(Untouched {
                            file,
                            reason: Reason::AlreadyThere,
                        });
                        continue;
                    }
                    (
                        format!("{directory}/{}", sanitize(&entry.name)),
                        Because::Inbox,
                    )
                }
                Outcome::Routed { in_place: true, .. } => {
                    untouched.push(Untouched {
                        file,
                        reason: Reason::AlreadyThere,
                    });
                    continue;
                }
                Outcome::Routed {
                    destination, rule, ..
                } => (destination, Because::Rule { name: rule }),
            };

            // A policy may not route anything into tungstate's own keeping.
            // The classifier cannot know these names; this is the one place
            // that does.
            if snapshot::is_reserved(&to) {
                untouched.push(Untouched {
                    file,
                    reason: Reason::Blocked {
                        by: to,
                        detail: "tungstate keeps that directory for itself".to_string(),
                    },
                });
                continue;
            }

            wants.push(Want {
                from: file,
                to,
                because,
                size: entry.size,
            });
        }
        wants
    }
}

/// How much longer a file has to sit still, if it is still cooling.
fn cooling(entry: &Attributes, taken: Timestamp, cooldown: std::time::Duration) -> Option<u64> {
    let mtime = entry.mtime?;
    if cooldown.is_zero() {
        return None;
    }
    let elapsed = taken
        .as_millisecond()
        .saturating_sub(mtime.as_millisecond());
    let elapsed = u64::try_from(elapsed).unwrap_or(u64::MAX);
    let window = u64::try_from(cooldown.as_millis()).unwrap_or(u64::MAX);
    (elapsed < window).then(|| (window - elapsed).div_ceil(1000))
}

impl Policy {
    /// Phases 2 and 3: settle who gets which name, and what happens to
    /// whoever does not.
    fn assign(
        &self,
        snap: &Snapshot,
        wants: Vec<Want>,
        mut untouched: Vec<Untouched>,
    ) -> (Vec<Op>, Vec<Untouched>) {
        let mut stayers = stayers(snap, &wants);
        let mut claimed: BTreeSet<String> = BTreeSet::new();
        let mut ops = Vec::new();

        for want in wants {
            let free = |candidate: &str, claimed: &BTreeSet<String>| {
                let key = snap.key(candidate);
                // A candidate the mover is *itself* sitting on counts as free.
                // That is what makes numbering survive a replan: `scan-1.pdf`
                // asked to be `scan.pdf`, is refused, tries `scan-1.pdf`, and
                // finds itself there — so it has arrived rather than needing a
                // `scan-2.pdf`, and the plan after that one is empty.
                key == snap.key(&want.from)
                    || (!claimed.contains(&key) && !stayers.contains_key(&key))
            };

            let resolved = if free(&want.to, &claimed) {
                Resolved::Take(want.to.clone(), want.because.clone())
            } else if let Some(stayer) = stayers.get(&snap.key(&want.to)) {
                // Something that is not moving holds the name.
                if stayer.is_dir {
                    // Not a conflict action. `on_conflict` is four words about
                    // two *files* disagreeing over a name; quarantining a whole
                    // directory tree is not what any of them promised.
                    Resolved::Leave(Reason::Blocked {
                        by: stayer.path.clone(),
                        detail: "a directory holds that name".to_string(),
                    })
                } else {
                    match self.defaults.on_conflict {
                        OnConflict::Skip => Resolved::Leave(Reason::Conflicted {
                            holder: stayer.path.clone(),
                        }),
                        OnConflict::Quarantine => Resolved::Park(Parked::Conflict {
                            holder: stayer.path.clone(),
                        }),
                        OnConflict::Replace => Resolved::Evict(stayer.path.clone()),
                        OnConflict::Rename => next_free(&want, &free, &claimed),
                    }
                }
            } else {
                // Another mover asked first. Numbering is the rule DESIGN §9
                // gives for this, whatever `on_conflict` says: that setting is
                // about an existing file, not about two arrivals.
                next_free(&want, &free, &claimed)
            };

            match resolved {
                Resolved::Take(to, because) => {
                    claimed.insert(snap.key(&to));
                    // Numbering can land a file back on the name it already
                    // has: `one-1.txt` asks for `one.txt`, is refused, and the
                    // next free candidate is where it is standing. It has
                    // arrived, so there is nothing to do -- and emitting a
                    // move from a path to itself would mean the folder never
                    // settled.
                    if to == want.from {
                        untouched.push(Untouched {
                            file: want.from,
                            reason: Reason::AlreadyThere,
                        });
                    } else {
                        ops.push(Op::Move {
                            from: want.from,
                            to,
                            because,
                        });
                    }
                }
                Resolved::Evict(holder) => {
                    let parked = format!("{}/{holder}", snapshot::QUARANTINE);
                    claimed.insert(snap.key(&parked));
                    ops.push(Op::Quarantine {
                        from: holder.clone(),
                        to: parked,
                        because: Parked::Replaced {
                            by: want.from.clone(),
                        },
                    });
                    stayers.remove(&snap.key(&holder));
                    claimed.insert(snap.key(&want.to));
                    ops.push(Op::Move {
                        from: want.from,
                        to: want.to,
                        because: want.because,
                    });
                }
                Resolved::Park(because) => {
                    let parked = format!("{}/{}", snapshot::QUARANTINE, want.from);
                    claimed.insert(snap.key(&parked));
                    ops.push(Op::Quarantine {
                        from: want.from,
                        to: parked,
                        because,
                    });
                }
                Resolved::Leave(reason) => untouched.push(Untouched {
                    file: want.from,
                    reason,
                }),
            }
            let _ = want.size;
        }

        (ops, untouched)
    }
}

/// Something holding a name that is not going anywhere.
struct Stayer {
    path: String,
    is_dir: bool,
}

/// Everything occupying a name that no want will vacate, keyed for comparison.
///
/// Directories are always in here; so is any file no rule wanted to move.
fn stayers(snap: &Snapshot, wants: &[Want]) -> BTreeMap<String, Stayer> {
    let moving: BTreeSet<&str> = wants.iter().map(|w| w.from.as_str()).collect();
    let mut stayers: BTreeMap<String, Stayer> = snap
        .directories
        .iter()
        .map(|directory| {
            (
                snap.key(directory),
                Stayer {
                    path: directory.clone(),
                    is_dir: true,
                },
            )
        })
        .collect();
    for entry in &snap.entries {
        let path = entry.relative_path();
        if entry.is_dir || moving.contains(path.as_str()) {
            continue;
        }
        stayers.insert(
            snap.key(&path),
            Stayer {
                path,
                is_dir: false,
            },
        );
    }
    stayers
}

/// What the planner settled on for one file.
enum Resolved {
    /// Move it here.
    Take(String, Because),
    /// Park the named occupant, then move in.
    Evict(String),
    /// Park the mover itself.
    Park(Parked),
    /// Leave it alone.
    Leave(Reason),
}

/// The first numbered name nothing else has.
fn next_free(
    want: &Want,
    free: &impl Fn(&str, &BTreeSet<String>) -> bool,
    claimed: &BTreeSet<String>,
) -> Resolved {
    for n in 1..=1000_u32 {
        let candidate = numbered(&want.to, n);
        if free(&candidate, claimed) {
            return Resolved::Take(
                candidate,
                Because::Numbered {
                    wanted: want.to.clone(),
                },
            );
        }
    }
    // A thousand files wanting one name is a policy problem, not a planner
    // problem. Saying so beats looping.
    Resolved::Leave(Reason::Blocked {
        by: want.to.clone(),
        detail: "a thousand files want that name".to_string(),
    })
}

/// `Photos/clip.mp4` numbered 2 is `Photos/clip-2.mp4`.
///
/// Before the *last* extension, which is where `{stem}` and `{ext}` already
/// put the boundary, so a numbered name is spelled the way the policy language
/// would spell it. `archive.tar.gz` gives `archive.tar-2.gz`, which is uglier
/// than `archive-2.tar.gz` and consistent, and consistency is what stops the
/// numbering rule needing a second rule about when it does not apply.
fn numbered(path: &str, n: u32) -> String {
    let (directory, name) = match path.rsplit_once('/') {
        Some((directory, name)) => (Some(directory), name),
        None => (None, path),
    };
    let numbered = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}-{n}.{ext}"),
        _ => format!("{name}-{n}"),
    };
    // `sanitize` is idempotent, so this costs nothing on a name that is
    // already fine and cannot let a number resurrect a reserved spelling.
    let numbered = sanitize(&numbered);
    match directory {
        Some(directory) => format!("{directory}/{numbered}"),
        None => numbered,
    }
}

/// Phase 4: the directories the moves need, and the ones they empty.
fn directory_ops(snap: &Snapshot, ops: &[Op]) -> Vec<Op> {
    let leaving: BTreeSet<&str> = ops.iter().filter_map(Op::source).collect();
    let arriving: Vec<&str> = ops.iter().filter_map(Op::target).collect();

    // Where every file ends up.
    let mut after: BTreeSet<String> = snap
        .entries
        .iter()
        .filter(|e| !e.is_dir)
        .map(Attributes::relative_path)
        .filter(|p| !leaving.contains(p.as_str()))
        .collect();
    after.extend(arriving.iter().map(|p| (*p).to_string()));

    // Deepest first, so a directory whose only content was an empty
    // subdirectory can go once that subdirectory has.
    let mut candidates: Vec<&String> = snap.directories.iter().collect();
    candidates.sort_by_key(|d| std::cmp::Reverse(d.split('/').count()));

    let mut removed: BTreeSet<&str> = BTreeSet::new();
    for directory in candidates {
        if snapshot::is_reserved(directory) {
            continue;
        }
        let prefix = format!("{directory}/");
        // Only ever a directory *this plan* emptied. We cannot know who made
        // an empty directory without a journal of it, but we know exactly who
        // emptied one, because it is written in the plan just computed.
        let had_something = snap
            .entries
            .iter()
            .any(|e| e.relative_path().starts_with(&prefix))
            || snap.directories.iter().any(|d| d.starts_with(&prefix));
        let still_wanted = after.iter().any(|p| p.starts_with(&prefix))
            || snap
                .directories
                .iter()
                .any(|d| d.starts_with(&prefix) && !removed.contains(d.as_str()));
        if had_something && !still_wanted {
            removed.insert(directory);
        }
    }

    let existing: BTreeSet<&str> = snap.directories.iter().map(String::as_str).collect();
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for target in arriving {
        for ancestor in snapshot::ancestors(target) {
            if !existing.contains(ancestor.as_str()) {
                needed.insert(ancestor);
            }
        }
    }

    let mut made: Vec<Op> = needed.into_iter().map(|path| Op::MkDir { path }).collect();
    made.extend(removed.into_iter().map(|path| Op::RmDir {
        path: path.to_string(),
    }));
    made
}

/// Phase 5: put the ops in an order that can actually be carried out.
///
/// `existing` is what the folder holds right now, which is what tells a name
/// that must be *vacated* from one that will be *produced*.
fn order(mut ops: Vec<Op>, existing: &BTreeSet<String>) -> Vec<Op> {
    // Canonical order first, so the graph's indices *are* content order and
    // breaking a tie by index breaks it by content.
    ops.sort_by(|a, b| a.ordering().cmp(&b.ordering()));

    // Each pass breaks one cycle. The bound is a guard, not an expectation:
    // never loop forever is worth more than an assertion that cannot fire.
    for _ in 0..=ops.len() {
        let sorted = edges(&ops, existing).sort();
        if sorted.cyclic.is_empty() {
            return sorted.order.into_iter().map(|i| ops[i].clone()).collect();
        }
        let Some(broken) = break_one(&mut ops, &sorted.cyclic) else {
            // A knot with no move in it cannot be untied by renaming, and
            // should be unreachable. Report the ops rather than spin.
            let stuck: BTreeSet<usize> = sorted.cyclic;
            let mut kept: Vec<Op> = sorted.order.into_iter().map(|i| ops[i].clone()).collect();
            kept.extend(
                (0..ops.len())
                    .filter(|i| stuck.contains(i))
                    .map(|i| ops[i].clone()),
            );
            return kept;
        };
        debug_assert!(broken);
    }
    ops
}

/// Every ordering constraint between the ops.
fn edges(ops: &[Op], existing: &BTreeSet<String>) -> Graph {
    let mut graph = Graph::new(ops.len());

    let mut from_index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut mkdir_index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rmdir_index: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, op) in ops.iter().enumerate() {
        if let Some(from) = op.source() {
            from_index.insert(from, index);
        }
        match op {
            Op::MkDir { path } => {
                mkdir_index.insert(path, index);
            }
            Op::RmDir { path } => {
                rmdir_index.insert(path, index);
            }
            _ => {}
        }
    }

    for (index, op) in ops.iter().enumerate() {
        if let Some(path) = op.directory() {
            for ancestor in snapshot::ancestors(path) {
                // 1. A parent directory is made before its child, and removed
                //    after it.
                if let Some(&parent) = mkdir_index.get(ancestor.as_str()) {
                    graph.require(parent, index);
                }
                if matches!(op, Op::RmDir { .. })
                    && let Some(&parent) = rmdir_index.get(ancestor.as_str())
                {
                    graph.require(index, parent);
                }
            }
        }

        if let Some(to) = op.target() {
            // 2. The directory a file lands in exists before it lands.
            for ancestor in snapshot::ancestors(to) {
                if let Some(&made) = mkdir_index.get(ancestor.as_str()) {
                    graph.require(made, index);
                }
            }
            // 3. Two ops naming one path, and which way round depends on
            //    whether that path is there now.
            if let Some(&other) = from_index.get(to) {
                if existing.contains(to) {
                    // Vacate before occupy. The only class that can cycle.
                    graph.require(other, index);
                } else {
                    // Nothing is there to vacate: the path is a temporary name
                    // this plan invents, so it has to be *made* before it can
                    // be moved on from. Getting this backwards makes a swap
                    // look like a fresh cycle on every pass, and the breaker
                    // invents a temporary for the temporary, for ever.
                    graph.require(index, other);
                }
            }
        }

        if let Some(from) = op.source() {
            // 4. A file moves out of the way before a directory takes its name.
            if let Some(&made) = mkdir_index.get(from) {
                graph.require(index, made);
            }
            // 5. Everything leaves a directory before it is removed.
            for ancestor in snapshot::ancestors(from) {
                if let Some(&gone) = rmdir_index.get(ancestor.as_str()) {
                    graph.require(index, gone);
                }
            }
        }
    }

    graph
}

/// Redirect one move in the knot through a temporary name.
///
/// The lowest-indexed move, so a given knot always breaks the same way — and
/// because indices are content order, "lowest" is a property of the folder
/// rather than of how the ops happened to be built.
fn break_one(ops: &mut Vec<Op>, cyclic: &BTreeSet<usize>) -> Option<bool> {
    let victim = cyclic
        .iter()
        .copied()
        .find(|&i| matches!(ops[i], Op::Move { .. }))?;
    let Op::Move { from, to, because } = ops[victim].clone() else {
        return None;
    };

    let occupied: BTreeSet<&str> = ops
        .iter()
        .filter_map(Op::target)
        .chain(ops.iter().filter_map(Op::source))
        .collect();
    let mut swap = format!("{to}.tungstate-swap");
    let mut n = 1;
    while occupied.contains(swap.as_str()) {
        swap = format!("{to}.tungstate-swap-{n}");
        n += 1;
    }

    // Out to a sibling of the destination, then in. A sibling because that
    // directory provably exists — something is already sitting in it, which is
    // the whole reason there is a cycle — so this needs no `MkDir`, and a
    // rename inside one directory is the cheapest and most certainly atomic
    // thing any backend offers. One temporary breaks a rotation of any length.
    ops[victim] = Op::Move {
        from,
        to: swap.clone(),
        because: Because::MakeWay,
    };
    ops.push(Op::Move {
        from: swap,
        to,
        because,
    });
    Some(true)
}

/// A fingerprint of what the snapshot saw, so `apply` can tell the folder has
/// moved on since the plan was made and replan rather than act on stale facts.
fn fingerprint(snap: &Snapshot) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in &snap.entries {
        hasher.update(entry.relative_path().as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.size.to_le_bytes().as_slice());
        hasher.update(
            entry
                .mtime
                .map_or(0, Timestamp::as_millisecond)
                .to_le_bytes()
                .as_slice(),
        );
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// How much of the folder this would move.
fn measure(snap: &Snapshot, ops: &[Op]) -> Blast {
    let sizes: BTreeMap<String, u64> = snap
        .entries
        .iter()
        .filter(|e| !e.is_dir)
        .map(|e| (e.relative_path(), e.size))
        .collect();
    let of = snap.entries.iter().filter(|e| !e.is_dir).count();
    let moved: Vec<&str> = ops.iter().filter_map(Op::source).collect();
    let bytes = moved
        .iter()
        .filter_map(|path| sizes.get(*path))
        .copied()
        .sum();
    let files = moved.len();
    Blast {
        files,
        of,
        bytes,
        created: ops
            .iter()
            .filter(|op| matches!(op, Op::MkDir { .. }))
            .count(),
        removed: ops
            .iter()
            .filter(|op| matches!(op, Op::RmDir { .. }))
            .count(),
        over_limit: Blast::over(files, of),
    }
}

impl Plan {
    /// The plan's meaning, as a pure function: the snapshot this would produce
    /// if every op succeeded, in the order given.
    ///
    /// Each op's precondition is checked as it is reached, so this proves the
    /// emitted order is *executable* rather than merely acyclic — which is a
    /// stronger thing to know, and the same cost. It is also what slice 7's
    /// executor will be checked against: run the real thing, survey again, and
    /// compare with what this predicted.
    ///
    /// # Errors
    /// [`PlanError::OutOfOrder`] if an op is reached when it cannot run.
    pub fn apply_to(&self, before: &Snapshot) -> Result<Snapshot, PlanError> {
        let mut files: BTreeMap<String, Attributes> = before
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| (e.relative_path(), e.clone()))
            .collect();
        let mut directories = before.directories.clone();
        let kept: Vec<Attributes> = before
            .entries
            .iter()
            .filter(|e| e.is_dir)
            .cloned()
            .collect();

        let out_of_order = |op: &Op, why: &str| PlanError::OutOfOrder {
            op: format!("{op:?}"),
            why: why.to_string(),
        };

        for op in &self.ops {
            match op {
                Op::MkDir { path } => {
                    if files.contains_key(path) {
                        return Err(out_of_order(op, "a file still holds that name"));
                    }
                    for ancestor in snapshot::ancestors(path) {
                        if !directories.contains(&ancestor) {
                            return Err(out_of_order(op, "its parent does not exist yet"));
                        }
                    }
                    directories.insert(path.clone());
                }
                Op::Move { from, to, .. } | Op::Quarantine { from, to, .. } => {
                    let Some(mut moving) = files.remove(from) else {
                        return Err(out_of_order(op, "there is nothing there to move"));
                    };
                    if files.contains_key(to) || directories.contains(to) {
                        return Err(out_of_order(op, "the destination is still occupied"));
                    }
                    // Quarantine makes its own directory on the way, the way
                    // the drain does; a plan does not spell that out.
                    let under_quarantine = snapshot::is_reserved(to);
                    for ancestor in snapshot::ancestors(to) {
                        if !directories.contains(&ancestor) {
                            if !under_quarantine {
                                return Err(out_of_order(op, "its directory does not exist yet"));
                            }
                            directories.insert(ancestor);
                        }
                    }
                    moving.relocate(to);
                    files.insert(to.clone(), moving);
                }
                Op::RmDir { path } => {
                    let prefix = format!("{path}/");
                    if files.keys().any(|p| p.starts_with(&prefix))
                        || directories.iter().any(|d| d.starts_with(&prefix))
                    {
                        return Err(out_of_order(op, "it is not empty"));
                    }
                    if !directories.remove(path) {
                        return Err(out_of_order(op, "there is no such directory"));
                    }
                }
            }
        }

        let mut entries: Vec<Attributes> = files.into_values().collect();
        // Directories the walk recorded but never entered are still there, and
        // still occupy their names.
        entries.extend(
            kept.into_iter()
                .filter(|d| directories.contains(&d.relative_path())),
        );
        Ok(Snapshot::new(
            before.taken,
            entries,
            directories,
            before.case_sensitive,
        ))
    }
}
