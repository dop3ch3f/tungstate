//! The sync decision, on paper.
//!
//! Files here are content ids rather than bytes. Two contents can share a size
//! on purpose (0 and 1 are both ten bytes), so an edit that kept the size is a
//! case these tests reach rather than one they assume away.

use std::collections::BTreeMap;

use proptest::prelude::*;

use crate::sync::{
    Asked, Baseline, Because, Digests, Direction, FirstCheck, Member, MemberId, Mode, Now,
    OnConflict, OnRemove, Question, Removal, Removed, Resolve, Seen, SyncOp, SyncPlan, Why,
    apply_to, decide, decide_with, is_partial,
};

/// Where every member's bytes are, by content id.
type Contents = BTreeMap<(MemberId, String), u8>;

fn size_of(content: u8) -> u64 {
    if content < 2 { 10 } else { 20 }
}

fn digest_of(content: u8) -> String {
    format!("c{content}")
}

/// A world of members, what they hold, and what they held last run.
#[derive(Clone, Debug)]
struct World {
    members: Vec<Member>,
    contents: Contents,
    baseline: Baseline,
}

impl World {
    fn new(names: &[&str]) -> Self {
        let members = names
            .iter()
            .zip(1..)
            .map(|(name, id)| Member {
                id,
                name: (*name).to_string(),
                local: true,
                connection: None,
                case_sensitive: true,
                renames: true,
                files: BTreeMap::new(),
                parked: BTreeMap::new(),
            })
            .collect();
        Self {
            members,
            contents: Contents::new(),
            baseline: Baseline::new(),
        }
    }

    fn put(&mut self, member: MemberId, path: &str, content: u8, mtime: i64) -> &mut Self {
        let m = self.members.iter_mut().find(|m| m.id == member).unwrap();
        m.files.insert(
            path.to_string(),
            Now {
                size: size_of(content),
                mtime: Some(mtime),
            },
        );
        self.contents.insert((member, path.to_string()), content);
        self
    }

    fn drop_file(&mut self, member: MemberId, path: &str) -> &mut Self {
        let m = self.members.iter_mut().find(|m| m.id == member).unwrap();
        m.files.remove(path);
        self.contents.remove(&(member, path.to_string()));
        self
    }

    fn holds(&self, member: MemberId, path: &str) -> Option<u8> {
        self.contents.get(&(member, path.to_string())).copied()
    }

    /// Decide, reading whatever the decision asks to read.
    fn decide(&self, mode: &Mode) -> (SyncPlan, Digests) {
        self.decide_asked(mode, &Asked::default())
    }

    fn decide_asked(&self, mode: &Mode, asked: &Asked) -> (SyncPlan, Digests) {
        let mut digests = Digests::new();
        let mut last = Vec::new();
        for _ in 0..6 {
            let plan = decide_with(&self.members, &self.baseline, &digests, mode, asked);
            if plan.questions.is_empty() {
                return (plan, digests);
            }
            last.push(plan.questions.clone());
            for question in &plan.questions {
                let content = self.contents[&(question.member, question.path.clone())];
                digests.insert((question.member, question.path.clone()), digest_of(content));
            }
        }
        panic!("the decision kept asking: {last:?} with {digests:?}");
    }

    /// Decide and carry the plan out on paper.
    fn run(&mut self, mode: &Mode) -> SyncPlan {
        self.run_asked(mode, &Asked::default())
    }

    fn run_asked(&mut self, mode: &Mode, asked: &Asked) -> SyncPlan {
        let (plan, digests) = self.decide_asked(mode, asked);
        let (members, baseline) = apply_to(&self.members, &self.baseline, &digests, &plan);
        // The bytes follow the ops in order, so a copy of a name a rename
        // made a moment ago finds it.
        for op in &plan.ops {
            match op {
                SyncOp::Copy { from, to, path, .. } => {
                    let content = self.contents[&(*from, path.clone())];
                    self.contents.insert((*to, path.clone()), content);
                }
                SyncOp::Park {
                    from,
                    to,
                    path,
                    parked,
                    ..
                } => {
                    let content = self.contents[&(*from, path.clone())];
                    self.contents.insert((*to, parked.clone()), content);
                }
                SyncOp::Remove {
                    member, path, how, ..
                } => {
                    let content = self.contents.remove(&(*member, path.clone())).unwrap();
                    if let Removal::SetAside { to } = how {
                        self.contents.insert((*member, to.clone()), content);
                    }
                }
                SyncOp::Rename {
                    member, from, to, ..
                } => {
                    let content = self.contents.remove(&(*member, from.clone())).unwrap();
                    self.contents.insert((*member, to.clone()), content);
                }
                SyncOp::Record { .. } => {}
            }
        }
        self.members = members;
        self.baseline = baseline;
        plan
    }

    /// What a member keeps in its set-aside area, by content.
    fn parked(&self, member: MemberId) -> BTreeMap<String, u8> {
        self.contents
            .iter()
            .filter(|((m, path), _)| *m == member && path.starts_with(".tungstate-quarantine/"))
            .map(|((_, path), content)| (path.clone(), *content))
            .collect()
    }
}

fn mode(direction: Direction, anchor: Option<MemberId>) -> Mode {
    Mode {
        direction,
        anchor,
        first_check: FirstCheck::Full,
        cooldown_ms: 0,
        now_ms: 1_000_000,
        exact: false,
        on_conflict: OnConflict::Quarantine,
        on_remove: OnRemove::SetAside,
    }
}

fn exact(direction: Direction, anchor: Option<MemberId>) -> Mode {
    Mode {
        exact: true,
        ..mode(direction, anchor)
    }
}

fn removals(plan: &SyncPlan) -> Vec<(MemberId, String, Removed)> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Remove {
                member,
                path,
                because,
                ..
            } => Some((*member, path.clone(), *because)),
            _ => None,
        })
        .collect()
}

fn copies(plan: &SyncPlan) -> Vec<(MemberId, MemberId, String)> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Copy { from, to, path, .. } => Some((*from, *to, path.clone())),
            _ => None,
        })
        .collect()
}

// --- the acceptance cases --------------------------------------------------

#[test]
fn three_members_end_up_with_everything_and_a_second_run_does_nothing() {
    let mut world = World::new(&["laptop", "nas", "spare"]);
    world
        .put(1, "a.mp4", 0, 5)
        .put(2, "b.mp4", 2, 5)
        .put(3, "c.mp4", 3, 5);
    let all = mode(Direction::All, None);

    let first = world.run(&all);
    assert_eq!(
        copies(&first).len(),
        6,
        "each file to the two members without it"
    );
    for member in 1..=3 {
        for path in ["a.mp4", "b.mp4", "c.mp4"] {
            assert!(world.holds(member, path).is_some(), "{member} lacks {path}");
        }
    }

    let second = world.run(&all);
    assert!(second.is_empty(), "{second:#?}");
    assert!(
        second.questions.is_empty(),
        "nothing is read on a quiet run"
    );
}

#[test]
fn an_edit_on_one_member_goes_to_the_other_two_and_nothing_else_moves() {
    let mut world = World::new(&["laptop", "nas", "spare"]);
    world.put(1, "a.mp4", 0, 5).put(1, "b.mp4", 2, 5);
    let all = mode(Direction::All, None);
    world.run(&all);

    // Content 1 is the same size as content 0: an edit that kept the size.
    world.put(2, "a.mp4", 1, 9);
    let plan = world.run(&all);

    let mut moved = copies(&plan);
    moved.sort();
    assert_eq!(moved, [(2, 1, "a.mp4".into()), (2, 3, "a.mp4".into())]);
    assert_eq!(world.holds(1, "a.mp4"), Some(1));
    assert_eq!(world.holds(3, "a.mp4"), Some(1));
}

#[test]
fn push_leaves_a_non_anchor_file_alone_and_says_why() {
    let mut world = World::new(&["laptop", "nas"]);
    world.put(2, "only-on-nas.mp4", 0, 5);
    let push = mode(Direction::Push, Some(1));

    let plan = world.run(&push);

    assert!(copies(&plan).is_empty());
    assert!(
        plan.left_alone
            .iter()
            .any(|l| l.why == Why::OnlyTheAnchorSends { member: 2 }),
        "{plan:#?}"
    );
}

#[test]
fn a_copy_that_matches_the_anchor_is_remembered_not_reported() {
    // Found by hand against the NAS: a file already on both sides, identical,
    // was recorded as agreeing and also listed as left alone.
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "same.jpg", 2, 5).put(2, "same.jpg", 2, 8);

    let plan = world.run(&mode(Direction::Push, Some(1)));

    assert!(plan.left_alone.is_empty(), "{:#?}", plan.left_alone);
    assert!(copies(&plan).is_empty());
}

#[test]
fn push_puts_back_what_a_non_anchor_deleted() {
    // Every member holds at least what the anchor holds: that is the promise.
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "a.mp4", 0, 5);
    let push = mode(Direction::Push, Some(1));
    world.run(&push);

    world.drop_file(2, "a.mp4");
    let plan = world.run(&push);

    assert_eq!(copies(&plan), [(1, 2, "a.mp4".into())]);
}

#[test]
fn pull_is_the_mirror_of_push() {
    let mut world = World::new(&["laptop", "nas"]);
    world
        .put(1, "only-on-laptop.mp4", 0, 5)
        .put(2, "on-nas.mp4", 2, 5);
    let pull = mode(Direction::Pull, Some(1));

    let plan = world.run(&pull);

    assert_eq!(copies(&plan), [(2, 1, "on-nas.mp4".into())]);
    assert!(
        plan.left_alone
            .iter()
            .any(|l| l.why == Why::OnlyTheAnchorReceives { member: 1 }),
        "{plan:#?}"
    );
}

#[test]
fn a_member_added_later_is_filled_from_one_on_this_machine() {
    let mut world = World::new(&["nas", "laptop"]);
    world.members[0].local = false;
    world.members[0].connection = Some(7);
    world.put(1, "a.mp4", 0, 5).put(2, "a.mp4", 0, 6);
    let all = mode(Direction::All, None);
    world.run(&all);

    world.members.push(Member {
        id: 3,
        name: "spare".into(),
        local: false,
        connection: Some(8),
        case_sensitive: true,
        renames: true,
        files: BTreeMap::new(),
        parked: BTreeMap::new(),
    });
    let plan = world.run(&all);

    assert_eq!(
        copies(&plan),
        [(2, 3, "a.mp4".into())],
        "from the laptop, not the nas"
    );
}

#[test]
fn a_member_without_times_is_read_and_the_plan_says_why() {
    let mut world = World::new(&["laptop", "ftp"]);
    world.put(1, "a.mp4", 0, 5).put(2, "a.mp4", 0, 5);
    let all = mode(Direction::All, None);
    world.run(&all);
    world.members[1].files.get_mut("a.mp4").unwrap().mtime = None;

    let plan = decide(&world.members, &world.baseline, &Digests::new(), &all);

    assert_eq!(
        plan.questions,
        [Question {
            member: 2,
            path: "a.mp4".into(),
            sampled: false,
            because: Because::NoTimes,
        }]
    );
    assert!(
        world.run(&all).is_empty(),
        "read, found unchanged, nothing to do"
    );
}

#[test]
fn an_edit_beats_a_deletion() {
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "a.mp4", 0, 5);
    let all = mode(Direction::All, None);
    world.run(&all);

    world.drop_file(1, "a.mp4").put(2, "a.mp4", 2, 9);
    let plan = world.run(&all);

    assert_eq!(copies(&plan), [(2, 1, "a.mp4".into())]);
}

#[test]
fn three_different_edits_are_a_conflict_and_skip_moves_nothing() {
    let mut world = World::new(&["a", "b", "c"]);
    world.put(1, "x", 0, 5);
    let mut all = mode(Direction::All, None);
    all.on_conflict = OnConflict::Skip;
    world.run(&all);

    world.put(1, "x", 1, 9).put(2, "x", 2, 9).put(3, "x", 3, 9);
    let plan = world.run(&all);

    assert!(plan.ops.is_empty(), "{plan:#?}");
    assert!(
        plan.left_alone
            .iter()
            .any(|l| matches!(&l.why, Why::Conflict { members } if members.len() == 3)),
        "{plan:#?}"
    );
}

#[test]
fn a_first_meeting_is_checked_as_hard_as_the_sync_was_told() {
    // Contents 0 and 1 share a size: only reading them tells them apart.
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "a.mp4", 0, 5).put(2, "a.mp4", 1, 7);

    let mut size = mode(Direction::All, None);
    size.first_check = FirstCheck::Size;
    let trusted = decide(&world.members, &world.baseline, &Digests::new(), &size);
    assert!(trusted.questions.is_empty(), "size only reads nothing");
    assert!(
        copies(&trusted).is_empty(),
        "and takes equal sizes to match"
    );

    let mut sampled = size;
    sampled.first_check = FirstCheck::Sampled;
    let asked = decide(&world.members, &world.baseline, &Digests::new(), &sampled);
    assert!(asked.questions.iter().all(|q| q.sampled), "{asked:#?}");

    let (full, _) = world.decide(&mode(Direction::All, None));
    assert!(
        full.left_alone
            .iter()
            .any(|l| matches!(l.why, Why::Conflict { .. })),
        "read in full, they differ"
    );
}

#[test]
fn a_sampled_digest_is_never_remembered() {
    // A later run would compare it with a whole digest of the same bytes and
    // call two identical files different.
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "a.mp4", 0, 5).put(2, "a.mp4", 0, 7);
    let mut sampled = mode(Direction::All, None);
    sampled.first_check = FirstCheck::Sampled;
    let mut digests = Digests::new();
    for member in [1, 2] {
        digests.insert(
            (member, "a.mp4".into()),
            format!("{}x", crate::sync::SAMPLED),
        );
    }

    let plan = decide(&world.members, &world.baseline, &digests, &sampled);

    let recorded: Vec<&Option<String>> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Record { seen, .. } => Some(&seen.hash),
            _ => None,
        })
        .collect();
    assert_eq!(recorded, [&None, &None], "{plan:#?}");
}

#[test]
fn a_file_still_being_written_is_left_until_it_settles() {
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "export.mp4", 0, 999_990);
    let mut all = mode(Direction::All, None);
    all.cooldown_ms = 30_000;

    let plan = world.run(&all);

    assert!(plan.ops.is_empty());
    assert!(matches!(
        plan.left_alone[0].why,
        Why::TooRecent { member: 1 }
    ));
}

#[test]
fn a_name_differing_only_in_case_is_not_written_over() {
    let mut world = World::new(&["linux", "mac"]);
    world.members[1].case_sensitive = false;
    world.put(1, "Clip.mp4", 0, 5).put(2, "clip.mp4", 2, 5);

    let plan = world.run(&mode(Direction::All, None));

    assert!(
        !copies(&plan)
            .iter()
            .any(|(_, to, path)| *to == 2 && path == "Clip.mp4"),
        "{plan:#?}"
    );
    assert!(
        plan.left_alone
            .iter()
            .any(|l| matches!(l.why, Why::CaseClash { member: 2, .. }))
    );
}

#[test]
fn a_transfer_in_progress_is_not_a_file_to_sync() {
    assert!(is_partial("clips/holiday.mp4.tungstate-42.part"));
    assert!(!is_partial("clips/holiday.part"));
    assert!(!is_partial("clips/notes.tungstate-draft.part"));
}

#[test]
fn the_same_input_gives_the_same_plan() {
    let mut world = World::new(&["a", "b", "c"]);
    world.put(1, "x", 0, 5).put(2, "y", 2, 5).put(3, "z", 3, 5);
    let all = mode(Direction::All, None);
    assert_eq!(world.decide(&all).0, world.decide(&all).0);
}

// --- 9c: exact, removal, conflicts, forget, carried tidies -------------------

/// Two members that agree on `a.mp4` and `b.mp4`, after one run.
fn settled_pair(mode: &Mode) -> World {
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "a.mp4", 0, 5).put(1, "b.mp4", 2, 5);
    world.run(mode);
    world
}

#[test]
fn exact_all_sets_a_deletion_aside_on_the_others() {
    let all = exact(Direction::All, None);
    let mut world = settled_pair(&all);

    world.drop_file(1, "a.mp4");
    let plan = world.run(&all);

    assert_eq!(removals(&plan), [(2, "a.mp4".into(), Removed::Deleted)]);
    assert!(plan.reversible());
    assert_eq!(world.holds(2, "a.mp4"), None);
    assert_eq!(
        world.parked(2),
        BTreeMap::from([(".tungstate-quarantine/a.mp4".to_string(), 0)])
    );
    assert!(world.run(&all).is_empty(), "and it stays gone");
}

#[test]
fn exact_with_delete_deletes_and_cannot_be_undone() {
    let mut all = exact(Direction::All, None);
    all.on_remove = OnRemove::Delete;
    let mut world = settled_pair(&all);

    world.drop_file(1, "a.mp4");
    let plan = world.run(&all);

    assert!(matches!(
        plan.ops[0],
        SyncOp::Remove {
            how: Removal::Delete,
            ..
        }
    ));
    assert!(!plan.reversible());
    assert!(world.parked(2).is_empty());
    assert_eq!(plan.blast[&2].deleting, 1);
}

#[test]
fn without_exact_a_hand_deletion_is_copied_back() {
    let all = mode(Direction::All, None);
    let mut world = settled_pair(&all);

    world.drop_file(1, "a.mp4");
    let plan = world.run(&all);

    assert_eq!(copies(&plan), [(2, 1, "a.mp4".into())]);
    assert!(removals(&plan).is_empty());
}

#[test]
fn under_exact_an_edit_still_beats_a_deletion() {
    let all = exact(Direction::All, None);
    let mut world = settled_pair(&all);

    world.drop_file(1, "a.mp4").put(2, "a.mp4", 1, 9);
    let plan = world.run(&all);

    assert_eq!(copies(&plan), [(2, 1, "a.mp4".into())]);
    assert!(removals(&plan).is_empty());
}

#[test]
fn exact_push_takes_extras_and_edits_off_the_others() {
    let push = exact(Direction::Push, Some(1));
    let mut world = settled_pair(&push);

    world.put(2, "extra.mp4", 3, 9).put(2, "b.mp4", 3, 9);
    let plan = world.run(&push);

    let mut removed = removals(&plan);
    removed.sort();
    assert_eq!(
        removed,
        [
            (2, "b.mp4".into(), Removed::Superseded),
            (2, "extra.mp4".into(), Removed::Extra),
        ]
    );
    assert_eq!(copies(&plan), [(1, 2, "b.mp4".into())]);
    assert_eq!(world.holds(2, "b.mp4"), Some(2));
    assert!(plan.left_alone.is_empty(), "replaced, not reported");
}

#[test]
fn exact_pull_takes_what_only_the_anchor_has_off_it() {
    let pull = exact(Direction::Pull, Some(1));
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "mine.mp4", 0, 5).put(2, "theirs.mp4", 2, 5);

    let plan = world.run(&pull);

    assert_eq!(removals(&plan), [(1, "mine.mp4".into(), Removed::Extra)]);
    assert_eq!(copies(&plan), [(2, 1, "theirs.mp4".into())]);
}

#[test]
fn an_edit_replaces_the_old_version_by_setting_it_aside_first() {
    let all = mode(Direction::All, None);
    let mut world = settled_pair(&all);

    world.put(2, "a.mp4", 1, 9);
    let plan = world.run(&all);

    assert_eq!(removals(&plan), [(1, "a.mp4".into(), Removed::Superseded)]);
    assert_eq!(plan.blast[&1].replacing, 1);
    assert_eq!(world.holds(1, "a.mp4"), Some(1));
    assert_eq!(world.parked(1).into_values().collect::<Vec<_>>(), [0]);
}

#[test]
fn a_second_supersession_does_not_land_on_the_first() {
    let all = mode(Direction::All, None);
    let mut world = settled_pair(&all);
    world.put(2, "a.mp4", 1, 9);
    world.run(&all);

    world.put(2, "a.mp4", 3, 12);
    world.run(&all);

    assert_eq!(
        world.parked(1),
        BTreeMap::from([
            (".tungstate-quarantine/a.mp4".to_string(), 0),
            (".tungstate-quarantine/a-2.mp4".to_string(), 1),
        ])
    );
}

#[test]
fn a_member_that_cannot_rename_is_left_alone_while_the_others_are_copied() {
    let all = mode(Direction::All, None);
    let mut world = World::new(&["laptop", "nas", "ftp"]);
    world.members[2].renames = false;
    world.put(1, "a.mp4", 0, 5);
    world.run(&all);

    world.put(1, "a.mp4", 1, 9);
    let plan = world.run(&all);

    assert_eq!(copies(&plan), [(1, 2, "a.mp4".into())]);
    assert!(
        plan.left_alone
            .iter()
            .any(|l| l.why == Why::NeedsRename { member: 3 })
    );
    assert!(
        !plan
            .ops
            .iter()
            .any(|op| matches!(op, SyncOp::Record { .. })),
        "nothing about the path is remembered until all of it happens"
    );
    assert!(world.run(&all).is_empty(), "and it does not copy again");
}

#[test]
fn an_empty_member_is_hollow() {
    let all = exact(Direction::All, None);
    let mut world = settled_pair(&all);
    world.drop_file(2, "a.mp4").drop_file(2, "b.mp4");

    let (plan, _) = world.decide(&all);

    assert_eq!(plan.hollow, [2]);
}

#[test]
fn taking_off_a_fifth_of_a_member_is_over_the_limit() {
    let all = exact(Direction::All, None);
    let mut world = World::new(&["laptop", "nas"]);
    for n in 0..10 {
        world.put(1, &format!("{n}.mp4"), 2, 5);
    }
    world.run(&all);

    world.drop_file(1, "0.mp4");
    let (one, _) = world.decide(&all);
    assert!(!one.blast[&2].over_limit, "one in ten");

    world.drop_file(1, "1.mp4");
    let (two, _) = world.decide(&all);
    assert!(two.blast[&2].over_limit, "two in ten is a fifth");
}

fn conflicted(mode: &Mode) -> World {
    let mut world = World::new(&["laptop", "nas"]);
    world.put(1, "clips/x.mp4", 0, 5);
    world.run(mode);
    world
        .put(1, "clips/x.mp4", 1, 9)
        .put(2, "clips/x.mp4", 3, 9);
    world
}

#[test]
fn a_conflict_parks_each_version_named_after_its_member() {
    let all = mode(Direction::All, None);
    let mut world = conflicted(&all);

    let plan = world.run(&all);

    assert_eq!(
        world.parked(1),
        BTreeMap::from([(".tungstate-quarantine/clips/x (nas).mp4".to_string(), 3)])
    );
    assert_eq!(
        world.parked(2),
        BTreeMap::from([(".tungstate-quarantine/clips/x (laptop).mp4".to_string(), 1)])
    );
    assert_eq!(world.holds(1, "clips/x.mp4"), Some(1), "each keeps its own");
    assert!(
        plan.left_alone
            .iter()
            .any(|l| matches!(l.why, Why::Conflict { .. }))
    );

    let again = world.run(&all);
    assert!(again.ops.is_empty(), "parked once: {again:#?}");
    assert!(
        again
            .left_alone
            .iter()
            .any(|l| matches!(l.why, Why::Conflict { .. })),
        "and still asked about"
    );
}

#[test]
fn a_parked_name_already_taken_is_numbered() {
    let all = mode(Direction::All, None);
    let mut world = conflicted(&all);
    world.members[0].parked.insert(
        ".tungstate-quarantine/clips/x (nas).mp4".into(),
        Now {
            size: 10,
            mtime: Some(1),
        },
    );
    world
        .contents
        .insert((1, ".tungstate-quarantine/clips/x (nas).mp4".into()), 0);

    world.run(&all);

    assert_eq!(
        world
            .parked(1)
            .get(".tungstate-quarantine/clips/x (nas)-2.mp4"),
        Some(&3)
    );
}

#[test]
fn a_three_way_conflict_parks_two_versions_on_each() {
    let all = mode(Direction::All, None);
    let mut world = World::new(&["a", "b", "c"]);
    world.put(1, "x", 0, 5);
    world.run(&all);
    world.put(1, "x", 1, 9).put(2, "x", 2, 9).put(3, "x", 3, 9);

    world.run(&all);

    for member in 1..=3 {
        assert_eq!(world.parked(member).len(), 2, "{member}");
    }
}

#[test]
fn rename_retires_the_contested_name() {
    let mut all = mode(Direction::All, None);
    all.on_conflict = OnConflict::Rename;
    let mut world = conflicted(&all);

    world.run(&all);

    for member in [1, 2] {
        assert_eq!(world.holds(member, "clips/x.mp4"), None);
        assert_eq!(world.holds(member, "clips/x (laptop).mp4"), Some(1));
        assert_eq!(world.holds(member, "clips/x (nas).mp4"), Some(3));
    }
    assert!(world.run(&all).is_empty(), "settled");
}

#[test]
fn keep_puts_one_version_everywhere_and_sets_the_other_aside() {
    let all = mode(Direction::All, None);
    let mut world = conflicted(&all);
    let asked = Asked {
        resolve: BTreeMap::from([("clips/x.mp4".to_string(), Resolve::Keep(2))]),
        only: true,
        ..Asked::default()
    };

    let plan = world.run_asked(&all, &asked);

    assert_eq!(copies(&plan), [(2, 1, "clips/x.mp4".into())]);
    assert_eq!(world.holds(1, "clips/x.mp4"), Some(3));
    assert_eq!(world.parked(1).into_values().collect::<Vec<_>>(), [1]);
    assert!(world.run(&all).is_empty(), "settled");
}

#[test]
fn keep_both_retires_the_name_whatever_the_sync_says() {
    let all = mode(Direction::All, None);
    let mut world = conflicted(&all);
    let asked = Asked {
        resolve: BTreeMap::from([("clips/x.mp4".to_string(), Resolve::KeepBoth)]),
        only: true,
        ..Asked::default()
    };

    world.run_asked(&all, &asked);

    assert_eq!(world.holds(2, "clips/x (laptop).mp4"), Some(1));
    assert!(world.run(&all).is_empty());
}

#[test]
fn a_forgotten_file_leaves_every_member_and_does_not_come_back() {
    let all = mode(Direction::All, None);
    let mut world = settled_pair(&all);
    let asked = Asked {
        forget: ["a.mp4".to_string()].into(),
        only: true,
        ..Asked::default()
    };

    let plan = world.run_asked(&all, &asked);

    assert_eq!(removals(&plan).len(), 2);
    assert_eq!(world.holds(1, "a.mp4"), None);
    assert_eq!(world.holds(2, "a.mp4"), None);
    assert_eq!(world.holds(1, "b.mp4"), Some(2), "only what was named");
    assert!(
        world.run(&all).is_empty(),
        "not brought back by a non-exact run"
    );
}

#[test]
fn forgetting_a_folder_takes_everything_under_it_and_nothing_beside_it() {
    let all = mode(Direction::All, None);
    let mut world = World::new(&["laptop", "nas"]);
    world
        .put(1, "old/a.mp4", 0, 5)
        .put(1, "old/b.mp4", 2, 5)
        .put(1, "older.mp4", 3, 5);
    world.run(&all);
    let asked = Asked {
        forget: ["old".to_string()].into(),
        only: true,
        ..Asked::default()
    };

    let plan = world.run_asked(&all, &asked);

    assert_eq!(removals(&plan).len(), 4);
    assert_eq!(world.holds(1, "older.mp4"), Some(3));
}

#[test]
fn a_forget_that_cannot_set_aside_everywhere_does_nothing() {
    let all = mode(Direction::All, None);
    let mut world = settled_pair(&all);
    world.members[1].renames = false;
    let asked = Asked {
        forget: ["a.mp4".to_string()].into(),
        only: true,
        ..Asked::default()
    };

    let plan = world.run_asked(&all, &asked);

    assert!(plan.ops.is_empty());
    assert!(
        plan.left_alone
            .iter()
            .any(|l| l.why == Why::NeedsRename { member: 2 })
    );
}

/// A tidy on the laptop: `a.mp4` becomes `2026/a.mp4`, same bytes.
fn tidied(mode: &Mode) -> World {
    let mut world = settled_pair(mode);
    // As a real run leaves it: the nas has the digest its copy computed, and
    // the laptop, which only ever sent the file, has none.
    world
        .baseline
        .get_mut(&(2, "a.mp4".to_string()))
        .unwrap()
        .hash = Some(digest_of(0));
    world.drop_file(1, "a.mp4").put(1, "2026/a.mp4", 0, 5);
    world
}

#[test]
fn a_tidy_is_carried_as_a_rename_and_nothing_is_copied() {
    for mode in [mode(Direction::All, None), exact(Direction::All, None)] {
        let mut world = tidied(&mode);

        let plan = world.run(&mode);

        assert!(copies(&plan).is_empty(), "{plan:#?}");
        assert!(removals(&plan).is_empty());
        assert_eq!(world.holds(2, "2026/a.mp4"), Some(0));
        assert_eq!(world.holds(2, "a.mp4"), None);
        assert_eq!(plan.blast[&2].renaming, 1);
        assert!(world.run(&mode).is_empty(), "and the two stay in step");
    }
}

#[test]
fn a_tidy_is_a_copy_and_a_delete_where_the_other_cannot_rename() {
    let mut all = mode(Direction::All, None);
    all.on_remove = OnRemove::Delete;
    let mut world = tidied(&all);
    world.members[1].renames = false;

    let plan = world.run(&all);

    assert_eq!(copies(&plan), [(1, 2, "2026/a.mp4".into())]);
    assert_eq!(removals(&plan), [(2, "a.mp4".into(), Removed::Moved)]);
    assert!(world.run(&all).is_empty());
}

#[test]
fn a_tidy_that_would_need_a_set_aside_it_cannot_have_is_left_alone() {
    let all = mode(Direction::All, None);
    let mut world = tidied(&all);
    world.members[1].renames = false;

    let plan = world.run(&all);

    assert!(plan.ops.is_empty(), "{plan:#?}");
    assert!(
        plan.left_alone
            .iter()
            .any(|l| l.why == Why::NeedsRename { member: 2 })
    );
}

#[test]
fn different_bytes_under_a_new_name_are_not_a_move() {
    let all = exact(Direction::All, None);
    let mut world = settled_pair(&all);
    world.drop_file(1, "a.mp4").put(1, "2026/a.mp4", 1, 5);

    let plan = world.run(&all);

    assert_eq!(copies(&plan), [(1, 2, "2026/a.mp4".into())]);
    assert_eq!(removals(&plan), [(2, "a.mp4".into(), Removed::Deleted)]);
}

// --- properties --------------------------------------------------------------

const PATHS: [&str; 4] = ["a", "b", "c/d", "e"];

/// Per member, per path: maybe a file (content, mtime); and maybe a baseline
/// row (present, content, mtime). Arbitrary on purpose: the decision has to
/// cope with any history, not only the ones it would write.
type Draw = Vec<Vec<(Option<(u8, i64)>, Option<(bool, u8, i64)>)>>;

fn world_of(draw: &Draw) -> World {
    let names: Vec<String> = (1..=draw.len()).map(|n| format!("m{n}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut world = World::new(&refs);
    for (m, row) in draw.iter().enumerate() {
        let id = i64::try_from(m).unwrap() + 1;
        for (p, (file, seen)) in row.iter().enumerate() {
            let path = PATHS[p];
            if let Some((content, mtime)) = file {
                world.put(id, path, *content, *mtime);
            }
            if let Some((present, mut content, mtime)) = *seen {
                // A file with the size and the time it was recorded with holds
                // what was recorded. An edit that kept both is invisible to
                // size-and-time checking by design, so a history containing
                // one is not a history this decision can be asked to get right.
                if let Some((now, when)) = file
                    && present
                    && *when == mtime
                    && size_of(*now) == size_of(content)
                {
                    content = *now;
                }
                world.baseline.insert(
                    (id, path.to_string()),
                    Seen {
                        present,
                        size: size_of(content),
                        mtime: Some(mtime),
                        hash: Some(digest_of(content)),
                    },
                );
            }
        }
    }
    world
}

fn draws() -> impl Strategy<Value = Draw> {
    let cell = (
        proptest::option::of((0u8..4, 0i64..3)),
        proptest::option::of((any::<bool>(), 0u8..4, 0i64..3)),
    );
    (2usize..=4).prop_flat_map(move |n| {
        proptest::collection::vec(proptest::collection::vec(cell.clone(), PATHS.len()), n)
    })
}

fn modes(members: usize) -> impl Strategy<Value = Mode> {
    let count = i64::try_from(members).unwrap();
    (
        (0u8..3, 1..=count, 0u8..3),
        (any::<bool>(), 0u8..3, any::<bool>()),
    )
        .prop_map(|((direction, anchor, check), (exact, conflict, delete))| {
            let direction =
                [Direction::Push, Direction::Pull, Direction::All][usize::from(direction)];
            Mode {
                direction,
                anchor: (direction != Direction::All).then_some(anchor),
                first_check: [FirstCheck::Full, FirstCheck::Sampled, FirstCheck::Size]
                    [usize::from(check)],
                cooldown_ms: 0,
                now_ms: 1_000_000,
                exact,
                on_conflict: [OnConflict::Quarantine, OnConflict::Rename, OnConflict::Skip]
                    [usize::from(conflict)],
                // Delete only where the command line allows it: with exact.
                on_remove: if exact && delete {
                    OnRemove::Delete
                } else {
                    OnRemove::SetAside
                },
            }
        })
}

/// A world, a way of running it, and which members can rename.
fn world_and_mode() -> impl Strategy<Value = (Draw, Mode, Vec<bool>)> {
    draws().prop_flat_map(|draw| {
        let n = draw.len();
        (
            Just(draw),
            modes(n),
            proptest::collection::vec(proptest::bool::weighted(0.8), n),
        )
    })
}

fn world_with(draw: &Draw, renames: &[bool]) -> World {
    let mut world = world_of(draw);
    for (member, renames) in world.members.iter_mut().zip(renames) {
        member.renames = *renames;
    }
    world
}

/// How many copies of each content the world holds, set-aside areas included.
fn census(world: &World) -> BTreeMap<u8, usize> {
    let mut counts = BTreeMap::new();
    for content in world.contents.values() {
        *counts.entry(*content).or_default() += 1;
    }
    counts
}

proptest! {
    #[test]
    fn deciding_again_after_a_run_does_nothing((draw, mode, renames) in world_and_mode()) {
        let mut world = world_with(&draw, &renames);
        world.run(&mode);
        let (again, _) = world.decide(&mode);
        prop_assert!(again.ops.is_empty(), "{:#?}", again.ops);
    }

    #[test]
    fn each_direction_keeps_its_promise((draw, mode, renames) in world_and_mode()) {
        let mut world = world_with(&draw, &renames);
        world.run(&mode);
        let (again, _) = world.decide(&mode);
        let settled = |path: &str| !again.left_alone.iter().any(|l| l.path == path);
        // Under "size only", equal sizes are the promise; otherwise content.
        let same = |a: Option<u8>, b: Option<u8>| match (a, b) {
            (Some(a), Some(b)) if mode.first_check == FirstCheck::Size => size_of(a) == size_of(b),
            (a, b) => a == b,
        };
        let ids: Vec<MemberId> = world.members.iter().map(|m| m.id).collect();
        for path in PATHS.iter().copied().filter(|p| settled(p)) {
            let held: Vec<Option<u8>> = ids.iter().map(|id| world.holds(*id, path)).collect();
            match mode.direction {
                Direction::All => {
                    let present: Vec<u8> = held.iter().flatten().copied().collect();
                    prop_assert!(
                        present.is_empty() || (present.len() == ids.len() && present.iter().all(|c| same(Some(*c), Some(present[0])))),
                        "{path}: {held:?}"
                    );
                }
                Direction::Push => {
                    let anchor = world.holds(mode.anchor.unwrap(), path);
                    for (id, theirs) in ids.iter().zip(&held) {
                        if mode.exact {
                            prop_assert!(same(anchor, *theirs), "{path}: {id} has {theirs:?}, anchor {anchor:?}");
                        } else if anchor.is_some() {
                            prop_assert!(theirs.is_some() && same(anchor, *theirs), "{path}: {id} has {theirs:?}");
                        }
                    }
                }
                Direction::Pull => {
                    let anchor = mode.anchor.unwrap();
                    let others = ids.iter().any(|id| *id != anchor && world.holds(*id, path).is_some());
                    if others {
                        prop_assert!(world.holds(anchor, path).is_some(), "{path} missing on the anchor");
                    } else if mode.exact {
                        prop_assert!(world.holds(anchor, path).is_none(), "{path} extra on the anchor");
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_is_lost_unless_the_sync_deletes((draw, mode, renames) in world_and_mode()) {
        let mode = Mode { on_remove: OnRemove::SetAside, ..mode };
        let mut world = world_with(&draw, &renames);
        let before = census(&world);
        world.run(&mode);
        let after = census(&world);
        for (content, count) in before {
            prop_assert!(after.get(&content).copied().unwrap_or(0) >= count, "{content} lost");
        }
    }

    #[test]
    fn only_a_set_aside_or_a_park_writes_a_reserved_name((draw, mode, renames) in world_and_mode()) {
        let world = world_with(&draw, &renames);
        let (plan, _) = world.decide(&mode);
        let reserved = |path: &str| crate::snapshot::is_reserved(path);
        for op in &plan.ops {
            match op {
                SyncOp::Copy { path, .. } | SyncOp::Record { path, .. } | SyncOp::Remove { path, .. } => {
                    prop_assert!(!reserved(path), "{op:?}");
                }
                SyncOp::Rename { from, to, .. } => prop_assert!(!reserved(from) && !reserved(to), "{op:?}"),
                SyncOp::Park { path, parked, .. } => prop_assert!(!reserved(path) && reserved(parked), "{op:?}"),
            }
            if let SyncOp::Remove { how: Removal::SetAside { to }, .. } = op {
                prop_assert!(reserved(to), "{op:?}");
            }
        }
    }

    #[test]
    fn a_plan_is_the_same_every_time((draw, mode, renames) in world_and_mode()) {
        let world = world_with(&draw, &renames);
        prop_assert_eq!(world.decide(&mode).0, world.decide(&mode).0);
    }
}

#[test]
fn a_parked_name_says_whose_it_is() {
    use crate::sync::SAMPLED;
    let _ = SAMPLED;
    let mut world = World::new(&["a", "b"]);
    world.put(1, "notes", 0, 5);
    world.run(&mode(Direction::All, None));
    world.put(1, "notes", 1, 9).put(2, "notes", 3, 9);
    world.run(&mode(Direction::All, None));
    assert!(
        world
            .parked(1)
            .contains_key(".tungstate-quarantine/notes (b)")
    );
}
