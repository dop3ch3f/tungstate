//! The sync decision, on paper.
//!
//! Files here are content ids rather than bytes. Two contents can share a size
//! on purpose (0 and 1 are both ten bytes), so an edit that kept the size is a
//! case these tests reach rather than one they assume away.

use std::collections::BTreeMap;

use proptest::prelude::*;

use crate::sync::{
    Baseline, Because, Digests, Direction, FirstCheck, Member, MemberId, Mode, Now, Question, Seen,
    SyncOp, SyncPlan, Why, apply_to, decide, is_partial,
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
                files: BTreeMap::new(),
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
        let mut digests = Digests::new();
        for _ in 0..4 {
            let plan = decide(&self.members, &self.baseline, &digests, mode);
            if plan.questions.is_empty() {
                return (plan, digests);
            }
            for question in &plan.questions {
                let content = self.contents[&(question.member, question.path.clone())];
                digests.insert((question.member, question.path.clone()), digest_of(content));
            }
        }
        panic!("the decision kept asking");
    }

    /// Decide and carry the plan out on paper.
    fn run(&mut self, mode: &Mode) -> SyncPlan {
        let (plan, digests) = self.decide(mode);
        let (members, baseline) = apply_to(&self.members, &self.baseline, &digests, &plan);
        for op in &plan.ops {
            if let SyncOp::Copy { from, to, path, .. } = op {
                let content = self.contents[&(*from, path.clone())];
                self.contents.insert((*to, path.clone()), content);
            }
        }
        self.members = members;
        self.baseline = baseline;
        plan
    }
}

fn mode(direction: Direction, anchor: Option<MemberId>) -> Mode {
    Mode {
        direction,
        anchor,
        first_check: FirstCheck::Full,
        cooldown_ms: 0,
        now_ms: 1_000_000,
    }
}

fn copies(plan: &SyncPlan) -> Vec<(MemberId, MemberId, String)> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Copy { from, to, path, .. } => Some((*from, *to, path.clone())),
            SyncOp::Record { .. } => None,
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
        files: BTreeMap::new(),
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
fn three_different_edits_are_a_conflict_and_nothing_moves() {
    let mut world = World::new(&["a", "b", "c"]);
    world.put(1, "x", 0, 5);
    let all = mode(Direction::All, None);
    world.run(&all);

    world.put(1, "x", 1, 9).put(2, "x", 2, 9).put(3, "x", 3, 9);
    let plan = world.run(&all);

    assert!(copies(&plan).is_empty());
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
            SyncOp::Copy { .. } => None,
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
    (0u8..3, 1..=count, 0u8..3).prop_map(|(direction, anchor, check)| {
        let direction = [Direction::Push, Direction::Pull, Direction::All][usize::from(direction)];
        Mode {
            direction,
            anchor: (direction != Direction::All).then_some(anchor),
            first_check: [FirstCheck::Full, FirstCheck::Sampled, FirstCheck::Size]
                [usize::from(check)],
            cooldown_ms: 0,
            now_ms: 1_000_000,
        }
    })
}

fn world_and_mode() -> impl Strategy<Value = (Draw, Mode)> {
    draws().prop_flat_map(|draw| {
        let n = draw.len();
        (Just(draw), modes(n))
    })
}

proptest! {
    #[test]
    fn deciding_again_after_a_run_does_nothing((draw, mode) in world_and_mode()) {
        let mut world = world_of(&draw);
        world.run(&mode);
        let (again, _) = world.decide(&mode);
        prop_assert!(again.ops.is_empty(), "{:#?}", again.ops);
    }

    #[test]
    fn each_direction_keeps_its_promise((draw, mode) in world_and_mode()) {
        let mut world = world_of(&draw);
        world.run(&mode);
        let (again, _) = world.decide(&mode);
        let settled = |path: &str| !again.left_alone.iter().any(|l| l.path == path);
        // Under "size only", equal sizes are the promise; otherwise content.
        let same = |a: u8, b: u8| {
            if mode.first_check == FirstCheck::Size { size_of(a) == size_of(b) } else { a == b }
        };
        let ids: Vec<MemberId> = world.members.iter().map(|m| m.id).collect();
        for path in PATHS.iter().copied().filter(|p| settled(p)) {
            match mode.direction {
                Direction::All => {
                    let held: Vec<Option<u8>> = ids.iter().map(|id| world.holds(*id, path)).collect();
                    let present: Vec<u8> = held.iter().flatten().copied().collect();
                    prop_assert!(
                        present.is_empty() || (present.len() == ids.len() && present.iter().all(|c| same(*c, present[0]))),
                        "{path}: {held:?}"
                    );
                }
                Direction::Push => {
                    let anchor = mode.anchor.unwrap();
                    if let Some(content) = world.holds(anchor, path) {
                        for id in &ids {
                            let theirs = world.holds(*id, path);
                            prop_assert!(theirs.is_some_and(|c| same(c, content)), "{path}: {id} has {theirs:?}");
                        }
                    }
                }
                Direction::Pull => {
                    let anchor = mode.anchor.unwrap();
                    if ids.iter().any(|id| *id != anchor && world.holds(*id, path).is_some()) {
                        prop_assert!(world.holds(anchor, path).is_some(), "{path} missing on the anchor");
                    }
                }
            }
        }
    }

    #[test]
    fn a_plan_is_the_same_every_time((draw, mode) in world_and_mode()) {
        let world = world_of(&draw);
        prop_assert_eq!(world.decide(&mode).0, world.decide(&mode).0);
    }
}
