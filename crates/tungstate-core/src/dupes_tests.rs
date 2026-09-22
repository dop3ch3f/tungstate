//! The duplicate pass, against hand-built folders.
//!
//! The digest is a lookup table here, which is the point of the trait: the
//! grouping, the tie-break and the plan can all be checked without a single
//! byte of I/O, including the property that no group ever loses its last copy.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use proptest::prelude::*;

use crate::attrs::Attributes;
use crate::dupes::{self, Digest, Extras, Kept, Trouble, Wants};
use crate::plan::Op;
use crate::policy::Mode;
use crate::snapshot::Snapshot;

/// A digest that reads nothing: each path is mapped to the content it holds.
struct Table {
    content: BTreeMap<String, String>,
    partials: usize,
    wholes: usize,
}

impl Table {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            content: pairs
                .iter()
                .map(|(path, content)| ((*path).to_string(), (*content).to_string()))
                .collect(),
            partials: 0,
            wholes: 0,
        }
    }
}

impl Digest for Table {
    fn partial(&mut self, path: &str) -> Result<String, String> {
        self.partials += 1;
        // The ends of the content, which is what a real partial digest sees.
        let content = self.content.get(path).ok_or("no such file")?;
        let ends: String = content
            .chars()
            .take(2)
            .chain(content.chars().rev().take(2))
            .collect();
        Ok(ends)
    }

    fn whole(&mut self, path: &str) -> Result<String, String> {
        self.wholes += 1;
        self.content
            .get(path)
            .cloned()
            .ok_or("no such file".to_string())
    }
}

fn at(path: &str, size: u64, mtime: Option<i64>) -> Attributes {
    let now = Timestamp::from_second(1_700_000_000).expect("a valid moment");
    let mut file = Attributes::new(path, size, now);
    file.mtime = mtime.map(|seconds| Timestamp::from_second(seconds).expect("a valid moment"));
    file
}

fn folder(files: &[Attributes]) -> Snapshot {
    let directories: BTreeSet<String> = files
        .iter()
        .flat_map(|file| crate::snapshot::ancestors(&file.relative_path()))
        .collect();
    Snapshot::new(
        Timestamp::from_second(1_700_000_000).expect("a valid moment"),
        files.to_vec(),
        directories,
        true,
    )
}

#[test]
fn identical_content_under_different_names_is_one_group() {
    let snapshot = folder(&[
        at("holiday-final-2.mp4", 100, Some(20)),
        at("clips/IMG_4471.mov", 100, Some(30)),
        at("notes.txt", 12, Some(10)),
    ]);
    let mut digest = Table::new(&[
        ("holiday-final-2.mp4", "the same footage"),
        ("clips/IMG_4471.mov", "the same footage"),
        ("notes.txt", "something else"),
    ]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert_eq!(found.groups.len(), 1);
    assert_eq!(found.groups[0].keep, "holiday-final-2.mp4");
    assert_eq!(found.groups[0].extras.len(), 1);
    assert_eq!(found.extra_files(), 1);
    assert_eq!(found.reclaimable(), 100);
}

#[test]
fn a_file_alone_at_its_size_is_never_read() {
    let snapshot = folder(&[
        at("one.mp4", 10, Some(1)),
        at("two.mp4", 20, Some(1)),
        at("three.mp4", 30, Some(1)),
    ]);
    let mut digest = Table::new(&[("one.mp4", "a"), ("two.mp4", "b"), ("three.mp4", "c")]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert!(found.is_empty());
    assert_eq!(digest.partials, 0, "nothing was read at all");
    assert_eq!(digest.wholes, 0);
}

#[test]
fn same_size_but_different_ends_is_not_read_in_full() {
    let snapshot = folder(&[at("a.bin", 40, Some(1)), at("b.bin", 40, Some(1))]);
    let mut digest = Table::new(&[("a.bin", "abcd"), ("b.bin", "wxyz")]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert!(found.is_empty());
    assert_eq!(digest.partials, 2);
    assert_eq!(digest.wholes, 0, "the partial digest settled it");
}

#[test]
fn the_oldest_copy_is_the_one_that_stays() {
    let snapshot = folder(&[
        at("new/copy.mp4", 100, Some(900)),
        at("old/original.mp4", 100, Some(100)),
    ]);
    let mut digest = Table::new(&[("new/copy.mp4", "same"), ("old/original.mp4", "same")]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert_eq!(found.groups[0].keep, "old/original.mp4");
    assert_eq!(found.groups[0].why, Kept::Oldest);
}

#[test]
fn one_mtime_for_every_copy_falls_through_to_the_path() {
    // FTP rounds mtimes to the minute, so "they are all the same age" is the
    // ordinary case rather than the exception.
    let snapshot = folder(&[
        at("deep/inside/here/copy.mp4", 100, Some(500)),
        at("top.mp4", 100, Some(500)),
    ]);
    let mut digest = Table::new(&[("deep/inside/here/copy.mp4", "same"), ("top.mp4", "same")]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert_eq!(found.groups[0].keep, "top.mp4");
    assert_eq!(found.groups[0].why, Kept::Path);
}

#[test]
fn a_pinned_copy_wins_whatever_the_tie_break_says() {
    let snapshot = folder(&[
        at("new/copy.mp4", 100, Some(900)),
        at("old/original.mp4", 100, Some(100)),
    ]);
    let mut digest = Table::new(&[("new/copy.mp4", "same"), ("old/original.mp4", "same")]);
    let wants = Wants {
        pinned: ["new/copy.mp4".to_string()].into_iter().collect(),
        ..Wants::default()
    };

    let found = dupes::find(&snapshot, &mut digest, &wants).expect("the pass runs");

    assert_eq!(found.groups[0].keep, "new/copy.mp4");
    assert_eq!(found.groups[0].why, Kept::Pinned);
}

#[test]
fn two_pins_in_one_group_is_refused_rather_than_resolved() {
    let snapshot = folder(&[at("a.mp4", 100, Some(1)), at("b.mp4", 100, Some(1))]);
    let mut digest = Table::new(&[("a.mp4", "same"), ("b.mp4", "same")]);
    let wants = Wants {
        pinned: ["a.mp4".to_string(), "b.mp4".to_string()]
            .into_iter()
            .collect(),
        ..Wants::default()
    };

    let trouble = dupes::find(&snapshot, &mut digest, &wants).expect_err("it refuses");

    assert!(matches!(trouble, Trouble::TwoPins { .. }));
}

#[test]
fn a_folder_copied_whole_is_one_decision() {
    let snapshot = folder(&[
        at("trip/one.jpg", 10, Some(1)),
        at("trip/two.jpg", 20, Some(1)),
        at("trip copy/one.jpg", 10, Some(2)),
        at("trip copy/two.jpg", 20, Some(2)),
    ]);
    let mut digest = Table::new(&[
        ("trip/one.jpg", "first"),
        ("trip/two.jpg", "second"),
        ("trip copy/one.jpg", "first"),
        ("trip copy/two.jpg", "second"),
    ]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert_eq!(found.folders.len(), 1);
    assert_eq!(found.folders[0].keep, "trip");
    assert_eq!(found.folders[0].extras, vec!["trip copy".to_string()]);
    assert_eq!(found.folders[0].files, 2);
    assert!(
        found.groups.is_empty(),
        "the files inside are the folder's business, not offered twice"
    );
    assert_eq!(found.extra_files(), 2);
}

#[test]
fn files_only_skips_the_folder_grouping() {
    let snapshot = folder(&[
        at("trip/one.jpg", 10, Some(1)),
        at("trip copy/one.jpg", 10, Some(2)),
    ]);
    let mut digest = Table::new(&[("trip/one.jpg", "first"), ("trip copy/one.jpg", "first")]);
    let wants = Wants {
        files_only: true,
        ..Wants::default()
    };

    let found = dupes::find(&snapshot, &mut digest, &wants).expect("the pass runs");

    assert!(found.folders.is_empty());
    assert_eq!(found.groups.len(), 1);
}

#[test]
fn what_is_already_set_aside_is_not_a_duplicate_of_anything() {
    let snapshot = folder(&[
        at("keep.mp4", 100, Some(1)),
        at(".tungstate-quarantine/keep.mp4", 100, Some(1)),
    ]);
    let mut digest = Table::new(&[
        ("keep.mp4", "same"),
        (".tungstate-quarantine/keep.mp4", "same"),
    ]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert!(
        found.is_empty(),
        "a parked copy is already dealt with; offering it again is a loop"
    );
}

#[test]
fn empty_files_are_not_duplicates_of_each_other() {
    let snapshot = folder(&[at("a.txt", 0, Some(1)), at("b.txt", 0, Some(1))]);
    let mut digest = Table::new(&[("a.txt", ""), ("b.txt", "")]);

    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    assert!(
        found.is_empty(),
        "every empty file matches every other one, and none of them is a copy"
    );
}

#[test]
fn setting_aside_is_a_move_into_the_folders_own_set_aside_area() {
    let snapshot = folder(&[at("a.mp4", 100, Some(1)), at("dup/a.mp4", 100, Some(9))]);
    let mut digest = Table::new(&[("a.mp4", "same"), ("dup/a.mp4", "same")]);
    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    let plan = dupes::plan(&snapshot, &found, Extras::SetAside, "f", Mode::Observe);

    let parked: Vec<&Op> = plan
        .ops
        .iter()
        .filter(|op| matches!(op, Op::Quarantine { .. }))
        .collect();
    assert_eq!(parked.len(), 1);
    assert!(matches!(
        parked[0],
        Op::Quarantine { from, to, .. }
            if from == "dup/a.mp4" && to == ".tungstate-quarantine/dup/a.mp4"
    ));
    assert!(
        plan.ops.iter().all(|op| !matches!(op, Op::Trash { .. })),
        "nothing is trashed unless the person asked for it"
    );
}

#[test]
fn the_trash_is_only_ever_planned_when_asked_for() {
    let snapshot = folder(&[at("a.mp4", 100, Some(1)), at("dup/a.mp4", 100, Some(9))]);
    let mut digest = Table::new(&[("a.mp4", "same"), ("dup/a.mp4", "same")]);
    let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

    let plan = dupes::plan(&snapshot, &found, Extras::Trash, "f", Mode::Observe);

    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, Op::Trash { path, .. } if path == "dup/a.mp4"))
    );
    assert!(!Extras::Trash.reversible());
    assert!(Extras::SetAside.reversible());
}

proptest! {
    /// The property the whole slice rests on: whatever the folder, every group
    /// keeps exactly one copy where it is, and the copy that stays is never
    /// itself dealt with.
    #[test]
    fn every_group_keeps_one_copy(
        contents in proptest::collection::vec(0_u8..4, 1..12),
    ) {
        let files: Vec<Attributes> = contents
            .iter()
            .enumerate()
            .map(|(index, _)| {
                at(
                    &format!("file-{index}.bin"),
                    100,
                    Some(i64::try_from(index).unwrap_or(0)),
                )
            })
            .collect();
        let pairs: Vec<(String, String)> = contents
            .iter()
            .enumerate()
            .map(|(index, content)| (format!("file-{index}.bin"), format!("content-{content}")))
            .collect();
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();
        let snapshot = folder(&files);
        let mut digest = Table::new(&borrowed);

        let found = dupes::find(&snapshot, &mut digest, &Wants::default()).expect("the pass runs");

        // Every path appears at most once across every group.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for group in &found.groups {
            prop_assert!(seen.insert(group.keep.clone()), "a copy is kept twice over");
            for copy in &group.extras {
                prop_assert!(seen.insert(copy.path.clone()), "a copy is dealt with twice");
                prop_assert_ne!(&copy.path, &group.keep, "the kept copy is never an extra");
            }
        }

        // One file per distinct content survives, however many copies there were.
        let distinct: BTreeSet<&u8> = contents.iter().collect();
        let plan = dupes::plan(&snapshot, &found, Extras::SetAside, "f", Mode::Observe);
        let leaving: BTreeSet<&str> = plan.ops.iter().filter_map(Op::source).collect();
        prop_assert_eq!(
            files.len() - leaving.len(),
            distinct.len(),
            "exactly one copy of each distinct content stays in place"
        );
    }
}
