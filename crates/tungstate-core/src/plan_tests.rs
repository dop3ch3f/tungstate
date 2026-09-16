//! The planner, against hand-built folders.
//!
//! A snapshot is pure data, so every one of these is a few lines of structs.
//! That is the whole argument for `tungstate-core` having no I/O: the awkward
//! cases — a swap, a rotation, a directory that empties — are cheap to write
//! down and would each be a temp directory otherwise.

use std::collections::BTreeSet;

use jiff::Timestamp;

use crate::attrs::Attributes;
use crate::plan::{Because, Op, Parked, Plan, Reason};
use crate::policy::Policy;
use crate::snapshot::Snapshot;

fn policy(text: &str) -> Policy {
    Policy::parse(text).expect("the policy parses").policy
}

/// Route by extension into a directory named for it.
fn by_ext() -> Policy {
    policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = { ext = \"txt\" }\n\n\
         [[rule]]\nname = \"video\"\npath = \"Video\"\nmatch = { ext = \"mp4\" }\n",
    )
}

fn at(path: &str, size: u64) -> Attributes {
    Attributes::new(path, size, Timestamp::UNIX_EPOCH)
}

fn folder(paths: &[&str]) -> Snapshot {
    let entries: Vec<Attributes> = paths.iter().map(|p| at(p, 10)).collect();
    let mut directories = BTreeSet::new();
    for path in paths {
        for ancestor in crate::snapshot::ancestors(path) {
            directories.insert(ancestor);
        }
    }
    Snapshot::new(Timestamp::UNIX_EPOCH, entries, directories, true)
}

/// A folder with directories that exist beyond those the files imply.
fn folder_with_dirs(paths: &[&str], dirs: &[&str]) -> Snapshot {
    let mut snap = folder(paths);
    for dir in dirs {
        snap.directories.insert((*dir).to_string());
        let mut entry = at(dir, 0);
        entry.is_dir = true;
        snap.entries.push(entry);
    }
    Snapshot::new(
        snap.taken,
        snap.entries,
        snap.directories,
        snap.case_sensitive,
    )
}

fn moves(plan: &Plan) -> Vec<(String, String)> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            Op::Move { from, to, .. } | Op::Quarantine { from, to, .. } => {
                Some((from.clone(), to.clone()))
            }
            _ => None,
        })
        .collect()
}

fn reason_for(plan: &Plan, file: &str) -> Reason {
    plan.untouched
        .iter()
        .find(|u| u.file == file)
        .unwrap_or_else(|| panic!("`{file}` should be untouched: {:?}", plan.untouched))
        .reason
        .clone()
}

// --- the shape of an ordinary plan ---------------------------------------

#[test]
fn a_misplaced_file_moves_and_its_directory_is_made_first() {
    let snap = folder(&["a.txt"]);
    let plan = by_ext().plan(&snap);
    assert_eq!(
        plan.ops,
        [
            Op::MkDir {
                path: "Text".to_string()
            },
            Op::Move {
                from: "a.txt".to_string(),
                to: "Text/a.txt".to_string(),
                because: Because::Rule {
                    name: "text".to_string()
                },
            },
        ]
    );
}

#[test]
fn a_file_already_in_place_is_left_alone() {
    let snap = folder(&["Text/a.txt"]);
    let plan = by_ext().plan(&snap);
    assert!(plan.ops.is_empty(), "{:?}", plan.ops);
    assert_eq!(reason_for(&plan, "Text/a.txt"), Reason::AlreadyThere);
}

#[test]
fn a_file_no_rule_wants_is_named_with_its_reason() {
    let snap = folder(&["mystery.bin"]);
    let plan = by_ext().plan(&snap);
    assert!(plan.ops.is_empty());
    assert_eq!(reason_for(&plan, "mystery.bin"), Reason::Unmatched);
}

#[test]
fn every_file_is_accounted_for_exactly_once() {
    // The property that makes the output readable: an op or a reason, never
    // both and never neither.
    let snap = folder(&["a.txt", "b.mp4", "c.bin", "Text/d.txt"]);
    let plan = by_ext().plan(&snap);
    let mut named: Vec<String> = plan
        .ops
        .iter()
        .filter_map(Op::source)
        .map(String::from)
        .collect();
    named.extend(plan.untouched.iter().map(|u| u.file.clone()));
    named.sort();
    assert_eq!(named, ["Text/d.txt", "a.txt", "b.mp4", "c.bin"]);
}

// --- vacating, swaps and rotations ---------------------------------------

/// Route each file to the name the *other* one currently has.
fn swapper() -> Policy {
    policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"a-to-b\"\npath = \"\"\nrename = \"b.txt\"\n\
         match = { glob = \"a.txt\" }\n\n\
         [[rule]]\nname = \"b-to-a\"\npath = \"\"\nrename = \"a.txt\"\n\
         match = { glob = \"b.txt\" }\n",
    )
}

#[test]
fn a_file_vacates_before_another_takes_its_name() {
    // c.txt -> b.txt, and b.txt is itself going to a.txt. Nothing is a cycle
    // here, but the order is not the order the ops were built in.
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"b-to-a\"\npath = \"\"\nrename = \"a.txt\"\n\
         match = { glob = \"b.txt\" }\n\n\
         [[rule]]\nname = \"c-to-b\"\npath = \"\"\nrename = \"b.txt\"\n\
         match = { glob = \"c.txt\" }\n",
    );
    let plan = p.plan(&folder(&["b.txt", "c.txt"]));
    assert_eq!(
        moves(&plan),
        [
            ("b.txt".to_string(), "a.txt".to_string()),
            ("c.txt".to_string(), "b.txt".to_string()),
        ],
        "b must leave before c arrives"
    );
    assert!(plan.apply_to(&folder(&["b.txt", "c.txt"])).is_ok());
}

#[test]
fn two_files_trading_places_cost_exactly_one_temporary_name() {
    let snap = folder(&["a.txt", "b.txt"]);
    let plan = swapper().plan(&snap);
    let temporaries = moves(&plan)
        .iter()
        .filter(|(_, to)| to.contains("tungstate-swap"))
        .count();
    assert_eq!(temporaries, 1, "{:?}", moves(&plan));
    // And the order actually runs.
    let after = plan.apply_to(&snap).expect("the order is executable");
    let mut names: Vec<String> = after
        .entries
        .iter()
        .map(Attributes::relative_path)
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt"]);
}

#[test]
fn a_three_way_rotation_also_costs_exactly_one() {
    // The point of breaking a cycle by temporary name rather than by pairwise
    // swaps: a rotation of any length needs one, not one per member.
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"a\"\npath = \"\"\nrename = \"b.txt\"\nmatch = { glob = \"a.txt\" }\n\n\
         [[rule]]\nname = \"b\"\npath = \"\"\nrename = \"c.txt\"\nmatch = { glob = \"b.txt\" }\n\n\
         [[rule]]\nname = \"c\"\npath = \"\"\nrename = \"a.txt\"\nmatch = { glob = \"c.txt\" }\n",
    );
    let snap = folder(&["a.txt", "b.txt", "c.txt"]);
    let plan = p.plan(&snap);
    let temporaries = moves(&plan)
        .iter()
        .filter(|(_, to)| to.contains("tungstate-swap"))
        .count();
    assert_eq!(temporaries, 1, "{:?}", moves(&plan));
    assert!(plan.apply_to(&snap).is_ok());
}

// --- conflicts with something that is not moving --------------------------

fn conflicting(action: &str) -> Policy {
    policy(&format!(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\non_conflict = \"{action}\"\n\n\
         [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = {{ ext = \"txt\" }}\n"
    ))
}

/// `a.txt` wants `Text/a.txt`, which a `.bin` file already holds and no rule
/// will move.
fn blocked_folder() -> Snapshot {
    folder(&["a.txt", "Text/a.txt.keep"])
}

#[test]
fn skip_leaves_the_arriving_file_where_it_is_and_says_who_has_the_name() {
    let snap = folder(&["a.txt", "Text/a.txt"]);
    // `Text/a.txt` is in place, so it is not moving; `a.txt` wants its name.
    let plan = conflicting("skip").plan(&snap);
    assert!(moves(&plan).is_empty(), "{:?}", moves(&plan));
    assert_eq!(
        reason_for(&plan, "a.txt"),
        Reason::Conflicted {
            holder: "Text/a.txt".to_string()
        }
    );
}

#[test]
fn rename_numbers_the_arriving_file() {
    let snap = folder(&["a.txt", "Text/a.txt"]);
    let plan = conflicting("rename").plan(&snap);
    assert_eq!(
        moves(&plan),
        [("a.txt".to_string(), "Text/a-1.txt".to_string())]
    );
}

#[test]
fn quarantine_parks_the_arriving_file_and_names_the_holder() {
    let snap = folder(&["a.txt", "Text/a.txt"]);
    let plan = conflicting("quarantine").plan(&snap);
    assert_eq!(
        plan.ops
            .iter()
            .filter(|op| matches!(op, Op::Quarantine { .. }))
            .collect::<Vec<_>>(),
        [&Op::Quarantine {
            from: "a.txt".to_string(),
            to: ".tungstate-quarantine/a.txt".to_string(),
            because: Parked::Conflict {
                holder: "Text/a.txt".to_string()
            },
        }]
    );
}

#[test]
fn replace_parks_the_existing_file_rather_than_destroying_it() {
    // The decision this slice took from the drain: replace is which copy you
    // want, not permission to destroy the other one. Both files survive.
    let snap = folder(&["a.txt", "Text/a.txt"]);
    let plan = conflicting("replace").plan(&snap);
    assert_eq!(
        moves(&plan),
        [
            (
                "Text/a.txt".to_string(),
                ".tungstate-quarantine/Text/a.txt".to_string()
            ),
            ("a.txt".to_string(), "Text/a.txt".to_string()),
        ]
    );
    let after = plan.apply_to(&snap).expect("the order is executable");
    assert_eq!(after.entries.iter().filter(|e| !e.is_dir).count(), 2);
}

#[test]
fn a_directory_in_the_way_is_never_a_conflict_action() {
    // `on_conflict` is four words about two files disagreeing over a name.
    // Quarantining a whole directory tree is not one of them.
    let snap = folder_with_dirs(&["a.txt"], &["Text", "Text/a.txt"]);
    let plan = conflicting("quarantine").plan(&snap);
    assert!(moves(&plan).is_empty(), "{:?}", moves(&plan));
    assert!(matches!(reason_for(&plan, "a.txt"), Reason::Blocked { .. }));
    let _ = blocked_folder();
}

// --- two files wanting one name ------------------------------------------

#[test]
fn two_arrivals_wanting_one_name_are_numbered_in_path_order() {
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"flatten\"\npath = \"Text\"\nrename = \"one.txt\"\n\
         match = { ext = \"txt\" }\n",
    );
    let plan = p.plan(&folder(&["b.txt", "a.txt"]));
    assert_eq!(
        moves(&plan),
        [
            ("a.txt".to_string(), "Text/one.txt".to_string()),
            ("b.txt".to_string(), "Text/one-1.txt".to_string()),
        ]
    );
}

#[test]
fn numbering_is_stable_across_a_replan() {
    // The rule that makes it so: a candidate the mover is itself sitting on
    // counts as free. Get this wrong and the loser becomes `one-2`, then
    // `one-3`, for ever -- and the folder never settles.
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\non_conflict = \"rename\"\n\n\
         [[rule]]\nname = \"flatten\"\npath = \"Text\"\nrename = \"one.txt\"\n\
         match = { ext = \"txt\" }\n",
    );
    let snap = folder(&["a.txt", "b.txt"]);
    let first = p.plan(&snap);
    let after = first.apply_to(&snap).expect("executable");
    let second = p.plan(&after);
    assert!(
        second.ops.is_empty(),
        "the second plan should be empty, got {:?}",
        second.ops
    );
}

// --- directories ----------------------------------------------------------

#[test]
fn a_directory_the_plan_empties_is_removed() {
    let snap = folder(&["old/a.txt"]);
    let plan = by_ext().plan(&snap);
    assert!(
        plan.ops.contains(&Op::RmDir {
            path: "old".to_string()
        }),
        "{:?}",
        plan.ops
    );
    assert!(plan.apply_to(&snap).is_ok());
}

#[test]
fn a_directory_that_was_already_empty_is_left_alone() {
    // The line between tidying up after yourself and deleting things you
    // never touched. Without a journal of who created a directory we cannot
    // know who made this one, so it is not ours to remove.
    let snap = folder_with_dirs(&["a.txt"], &["empty"]);
    let plan = by_ext().plan(&snap);
    assert!(
        !plan.ops.iter().any(|op| matches!(op, Op::RmDir { .. })),
        "{:?}",
        plan.ops
    );
}

#[test]
fn a_directory_emptied_but_reused_as_an_ancestor_is_kept() {
    // `Text/old/a.txt` moves to `Text/a.txt`: `Text/old` empties and goes,
    // `Text` empties of files and stays, because the file lands in it.
    let snap = folder(&["Text/old/a.txt"]);
    let plan = by_ext().plan(&snap);
    let removed: Vec<&str> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::RmDir { path } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(removed, ["Text/old"], "{:?}", plan.ops);
    assert!(plan.apply_to(&snap).is_ok());
}

#[test]
fn nested_directories_empty_from_the_inside_out() {
    let snap = folder(&["deep/deeper/deepest/a.txt"]);
    let plan = by_ext().plan(&snap);
    let removed: Vec<&str> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::RmDir { path } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        removed,
        ["deep/deeper/deepest", "deep/deeper", "deep"],
        "deepest first, or a parent is not empty when its turn comes"
    );
    assert!(plan.apply_to(&snap).is_ok());
}

#[test]
fn nothing_is_ever_planned_inside_tungstates_own_directories() {
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"sneak\"\npath = \".tungstate\"\nmatch = { ext = \"txt\" }\n",
    );
    let plan = p.plan(&folder(&["a.txt"]));
    assert!(plan.ops.is_empty(), "{:?}", plan.ops);
    assert!(matches!(reason_for(&plan, "a.txt"), Reason::Blocked { .. }));
}

// --- cooldown and blast radius -------------------------------------------

#[test]
fn a_file_written_too_recently_waits() {
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"30s\"\n\n\
         [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = { ext = \"txt\" }\n",
    );
    let now = Timestamp::UNIX_EPOCH + std::time::Duration::from_secs(600);
    let mut fresh = Attributes::new("a.txt", 10, now);
    fresh.mtime = Some(now - std::time::Duration::from_secs(5));
    let mut settled = Attributes::new("b.txt", 10, now);
    settled.mtime = Some(now - std::time::Duration::from_secs(300));

    let snap = Snapshot::new(now, vec![fresh, settled], BTreeSet::new(), true);
    let plan = p.plan(&snap);
    assert_eq!(
        moves(&plan),
        [("b.txt".to_string(), "Text/b.txt".to_string())]
    );
    assert_eq!(
        reason_for(&plan, "a.txt"),
        Reason::Cooling { seconds_left: 25 }
    );
}

#[test]
fn the_blast_radius_counts_files_and_trips_on_a_fifth_of_the_folder() {
    let snap = folder(&["a.txt", "b.txt", "keep.bin", "other.bin"]);
    let plan = by_ext().plan(&snap);
    assert_eq!(plan.blast.files, 2);
    assert_eq!(plan.blast.of, 4);
    assert_eq!(plan.blast.bytes, 20);
    assert!(plan.blast.over_limit, "half the folder is past a fifth");
}

#[test]
fn a_small_share_of_a_large_folder_does_not_trip_the_breaker() {
    let mut paths: Vec<String> = (0..100).map(|n| format!("keep{n}.bin")).collect();
    paths.push("a.txt".to_string());
    let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
    let plan = by_ext().plan(&folder(&borrowed));
    assert_eq!(plan.blast.files, 1);
    assert!(!plan.blast.over_limit);
}

// --- determinism ----------------------------------------------------------

#[test]
fn the_same_folder_plans_the_same_way_every_time() {
    let snap = folder(&["z.txt", "a.mp4", "m.txt", "nested/deep/q.mp4"]);
    let first = by_ext().plan(&snap);
    let second = by_ext().plan(&snap);
    assert_eq!(first, second);
}

#[test]
fn the_order_of_the_entries_does_not_change_the_plan() {
    // `Snapshot::new` sorts, so a filesystem that lists in a different order
    // cannot give a different plan.
    let forwards = by_ext().plan(&folder(&["a.txt", "b.mp4", "c.txt"]));
    let backwards = by_ext().plan(&folder(&["c.txt", "b.mp4", "a.txt"]));
    assert_eq!(forwards.ops, backwards.ops);
}

// --- the four invariants --------------------------------------------------
//
// DESIGN §10 names these, and DESIGN §1 says to write the first one on the
// wall. They are generated over random folders and random policies, because
// the cases that break a planner are the ones nobody thinks to write down.

use proptest::prelude::*;

/// Path segments, including the ones designed to escape a root.
fn segment() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-c]{1,2}",
        "[a-c]{1,2}\\.(txt|mp4|bin)",
        Just("..".to_string()),
        Just(".".to_string()),
        Just("Text".to_string()),
    ]
}

fn a_folder() -> impl Strategy<Value = Snapshot> {
    proptest::collection::vec(
        (
            proptest::collection::vec(segment(), 1..4),
            0_u64..1000,
            any::<bool>(),
        ),
        0..8,
    )
    .prop_map(|raw| {
        let mut entries: Vec<Attributes> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut directories = BTreeSet::new();
        for (parts, size, is_dir) in raw {
            let path = crate::template::canonical_path(&parts.join("/"));
            if path.is_empty() || !seen.insert(path.clone()) {
                continue;
            }
            for ancestor in crate::snapshot::ancestors(&path) {
                directories.insert(ancestor);
            }
            let mut entry = at(&path, size);
            entry.is_dir = is_dir;
            if is_dir {
                directories.insert(path.clone());
            }
            entries.push(entry);
        }
        // A path cannot be both a file and a directory.
        entries.retain(|e| e.is_dir || !directories.contains(&e.relative_path()));
        Snapshot::new(Timestamp::UNIX_EPOCH, entries, directories, true)
    })
}

/// Policies whose templates are adversarial about escaping the root.
fn a_policy() -> impl Strategy<Value = Policy> {
    prop_oneof![
        Just("Text"),
        Just("../outside"),
        Just("{ext}/{range}"),
        Just("/absolute"),
        Just("{parent}/sorted"),
        Just("a/../../b"),
        Just(""),
    ]
    .prop_flat_map(|path| {
        prop_oneof![
            Just(String::new()),
            Just("one.txt".to_string()),
            Just("{stem}.{ext}".to_string()),
            Just("../{name}".to_string()),
        ]
        .prop_map(move |rename| (path, rename))
    })
    .prop_flat_map(|(path, rename)| {
        prop_oneof![
            Just("rename"),
            Just("skip"),
            Just("replace"),
            Just("quarantine"),
        ]
        .prop_map(move |action| {
            let rename = if rename.is_empty() {
                String::new()
            } else {
                format!("rename = \"{rename}\"\n")
            };
            policy(&format!(
                "[folder]\nname = \"p\"\ninbox = \"_inbox\"\n\
                 [defaults]\ncooldown = \"0s\"\non_conflict = \"{action}\"\n\n\
                 [[rule]]\nname = \"only\"\npath = \"{path}\"\n{rename}\
                 match = {{ ext = [\"txt\", \"mp4\"] }}\n\
                 vars.range = {{ from = \"size\", bucket = [\"<10MiB\", \">10MiB\"] }}\n"
            ))
        })
    })
}

/// Every path an op names.
fn touched(plan: &Plan) -> Vec<&str> {
    plan.ops
        .iter()
        .flat_map(|op| {
            [op.source(), op.target(), op.directory()]
                .into_iter()
                .flatten()
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// (a) Reconciliation is idempotent and convergent. Run it, apply it, run
    /// it again, and there is nothing left to do. This is the property that
    /// makes "run it continuously" safe, and the one everything else rests on.
    ///
    /// A policy can defeat it -- `rename = "copy-{name}"` renders differently
    /// once the file has been renamed -- so the honest statement of the
    /// invariant is that the folder settles *or the plan says it will not*.
    /// Silently churning is the one outcome ruled out.
    #[test]
    fn a_folder_settles_or_the_plan_says_it_will_not(
        snap in a_folder(), policy in a_policy()
    ) {
        let plan = policy.plan(&snap);
        let after = plan.apply_to(&snap).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let again = policy.plan(&after);
        prop_assert_eq!(
            again.ops.is_empty(), plan.settles,
            "first: {:?}\nsecond: {:?}", plan.ops, again.ops
        );
    }

    /// And a plan that settles really does settle, twice over: applying the
    /// second (empty) plan changes nothing either.
    #[test]
    fn a_settled_folder_stays_settled(snap in a_folder(), policy in a_policy()) {
        let plan = policy.plan(&snap);
        prop_assume!(plan.settles);
        let after = plan.apply_to(&snap).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let again = policy.plan(&after);
        prop_assert!(again.settles);
        let settled = again.apply_to(&after).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(settled.entries.len(), after.entries.len());
    }

    /// (b) No content is ever lost. With no `Trash` and no `Delete` in the op
    /// set, every file that went in comes out — under some name.
    #[test]
    fn no_file_is_lost_or_duplicated(snap in a_folder(), policy in a_policy()) {
        let plan = policy.plan(&snap);
        let after = plan.apply_to(&snap).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let before: Vec<u64> = {
            let mut v: Vec<u64> = snap.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).collect();
            v.sort_unstable();
            v
        };
        let survived: Vec<u64> = {
            let mut v: Vec<u64> = after.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).collect();
            v.sort_unstable();
            v
        };
        prop_assert_eq!(before, survived);
    }

    /// (c) Nothing outside the root is ever named. The classifier's own escape
    /// test, lifted from one destination to every path in every operation.
    #[test]
    fn no_operation_names_a_path_outside_the_root(
        snap in a_folder(), policy in a_policy()
    ) {
        let plan = policy.plan(&snap);
        for path in touched(&plan) {
            prop_assert!(!path.is_empty());
            prop_assert!(!path.starts_with('/'), "`{}` is rooted", path);
            for component in std::path::Path::new(path).components() {
                prop_assert!(
                    matches!(component, std::path::Component::Normal(_)),
                    "`{}` has a non-normal component {:?}", path, component
                );
            }
            for segment in path.split('/') {
                prop_assert!(segment != "." && segment != ".." && !segment.is_empty(), "`{}`", path);
            }
        }
    }

    /// (d) The ordering works. Not "the graph is acyclic", which tests the
    /// sort against itself, but "replaying this order against a folder never
    /// reaches an operation that cannot run" — which is what the order is for.
    #[test]
    fn the_emitted_order_can_actually_be_carried_out(
        snap in a_folder(), policy in a_policy()
    ) {
        let plan = policy.plan(&snap);
        prop_assert!(plan.apply_to(&snap).is_ok(), "{:?}", plan.apply_to(&snap).err());
    }

    /// Every file is accounted for exactly once, as an op or as a reason.
    #[test]
    fn nothing_falls_between_the_ops_and_the_reasons(
        snap in a_folder(), policy in a_policy()
    ) {
        let plan = policy.plan(&snap);
        let mut named: Vec<String> = plan.ops.iter().filter_map(Op::source).map(String::from).collect();
        named.extend(plan.untouched.iter().map(|u| u.file.clone()));
        named.sort();
        let mut files: Vec<String> = snap.entries.iter()
            .filter(|e| !e.is_dir).map(Attributes::relative_path).collect();
        files.sort();
        // A `Quarantine` of a file that was not going anywhere adds a source
        // nothing asked about, so the plan names at least every file.
        for file in &files {
            prop_assert!(named.contains(file), "`{}` is neither moved nor explained", file);
        }
    }
}

#[test]
fn a_policy_that_never_settles_says_so_rather_than_churning() {
    // `copy-{name}` renders differently once the file is renamed, so each
    // pass renames it again. Found by the convergence property rather than by
    // anyone thinking of it, which is the argument for having the property.
    let p = policy(
        "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"prefix\"\npath = \"\"\nrename = \"copy-{name}\"\n\
         match = { ext = \"txt\" }\n",
    );
    let plan = p.plan(&folder(&["a.txt"]));
    assert!(!plan.settles, "{:?}", plan.ops);
    assert_eq!(plan.unsettled, ["copy-a.txt"]);
}

#[test]
fn an_ordinary_policy_settles() {
    let plan = by_ext().plan(&folder(&["a.txt", "b.mp4", "old/c.txt"]));
    assert!(plan.settles);
    assert!(plan.unsettled.is_empty());
}
