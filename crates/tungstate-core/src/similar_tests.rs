//! The near-duplicate pass, against hand-built folders.
//!
//! The eye is a lookup table here, the way the exact pass's digest is: every
//! property that matters — what leads a group, that alike does not chain, that
//! a signature search misses nothing it should have found — is about the
//! arithmetic rather than about any real photograph.

use std::collections::BTreeMap;

use jiff::Timestamp;
use proptest::prelude::*;

use crate::attrs::Attributes;
use crate::dupes::{STOPPED, Trouble};
use crate::similar::{self, Band, Look, Mark, SAME_AT, SIMILAR_AT, Sort, Wants};
use crate::snapshot::Snapshot;

/// An eye that reads nothing.
///
/// A file's "content" here is a 64 bit number standing in for what it looks
/// like, and alike is the share of bits two of them agree on, stretched so
/// that two unrelated numbers score near zero rather than near fifty. That is
/// the same shape the real picture side uses, which is what makes these tests
/// about the pass rather than about the drawing.
struct Eye {
    looks: BTreeMap<String, (Sort, u64, u64)>,
    /// Files whose name says picture and whose body is rubbish.
    broken: std::collections::BTreeSet<String>,
    stop_after: Option<usize>,
    looked: usize,
}

impl Eye {
    fn new(files: &[(&str, Sort, u64, u64)]) -> Self {
        Self {
            looks: files
                .iter()
                .map(|(path, sort, bits, weight)| ((*path).to_string(), (*sort, *bits, *weight)))
                .collect(),
            broken: std::collections::BTreeSet::new(),
            stop_after: None,
            looked: 0,
        }
    }
}

impl Look for Eye {
    fn sort(&self, path: &str, _mime: Option<&str>) -> Option<Sort> {
        self.looks.get(path).map(|(sort, _, _)| *sort)
    }

    fn print(&mut self, path: &str) -> Result<Mark, String> {
        if self.stop_after.is_some_and(|at| self.looked >= at) {
            return Err(STOPPED.to_string());
        }
        if self.broken.contains(path) {
            return Err("not a picture after all".to_string());
        }
        self.looked += 1;
        let (sort, bits, weight) = self.looks.get(path).ok_or("no such file")?;
        Ok(Mark {
            algo: match sort {
                Sort::Picture => "pic1",
                Sort::Moving => "vid1",
                Sort::Sound => "snd1",
            }
            .to_string(),
            signature: *bits,
            detail: vec![*bits],
            weight: *weight,
        })
    }

    fn alike(&self, one: &Mark, other: &Mark) -> Option<u8> {
        if one.algo != other.algo {
            return None;
        }
        // Twice the share of bits that agree, less a hundred, so two unrelated
        // numbers score near nothing instead of near fifty. Integers only, so
        // the test needs no allowance for a cast that cannot lose anything.
        let agree = 64 - (one.detail[0] ^ other.detail[0]).count_ones();
        Some(u8::try_from(agree.saturating_mul(200).saturating_sub(6400) / 64).unwrap_or(100))
    }
}

fn at(path: &str, size: u64, mtime: i64) -> Attributes {
    let now = Timestamp::from_second(1_700_000_000).expect("a valid moment");
    let mut file = Attributes::new(path, size, now);
    file.mtime = Some(Timestamp::from_second(mtime).expect("a valid moment"));
    file
}

fn folder(files: &[Attributes]) -> Snapshot {
    let directories = files
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

/// A 64 bit number with `off` of its bits flipped from `from`.
fn nudge(from: u64, off: u32) -> u64 {
    let mut bits = from;
    for at in 0..off {
        bits ^= 1 << at;
    }
    bits
}

const SCENE: u64 = 0x0f0f_0f0f_0f0f_0f0f;

#[test]
fn a_re_export_is_found_and_the_bigger_copy_leads() {
    let snapshot = folder(&[
        at("holiday/IMG_4471.jpg", 4_000_000, 10),
        at("holiday/for the web.jpg", 200_000, 20),
    ]);
    let mut eye = Eye::new(&[
        ("holiday/IMG_4471.jpg", Sort::Picture, SCENE, 12_000_000),
        (
            "holiday/for the web.jpg",
            Sort::Picture,
            nudge(SCENE, 1),
            300_000,
        ),
    ]);

    let found = similar::resemble(&snapshot, &mut eye, &Wants::default()).expect("the pass runs");

    assert_eq!(found.clusters.len(), 1);
    let cluster = &found.clusters[0];
    assert_eq!(cluster.leader.path, "holiday/IMG_4471.jpg");
    assert_eq!(cluster.band, Band::Same);
    assert_eq!(cluster.others.len(), 1);
    assert_eq!(cluster.reclaimable(), 200_000);
}

#[test]
fn two_unrelated_pictures_are_left_alone() {
    let snapshot = folder(&[at("one.jpg", 900, 10), at("other.jpg", 900, 20)]);
    let mut eye = Eye::new(&[
        ("one.jpg", Sort::Picture, 0x0000_0000_ffff_ffff, 100),
        ("other.jpg", Sort::Picture, 0xffff_ffff_0000_0000, 100),
    ]);

    let found = similar::resemble(&snapshot, &mut eye, &Wants::default()).expect("the pass runs");

    assert!(found.is_empty());
    assert_eq!(found.looked, 2);
}

#[test]
fn a_sound_and_a_picture_are_never_compared() {
    let snapshot = folder(&[at("song.mp3", 900, 10), at("cover.jpg", 900, 20)]);
    let mut eye = Eye::new(&[
        ("song.mp3", Sort::Sound, SCENE, 100),
        ("cover.jpg", Sort::Picture, SCENE, 100),
    ]);

    let found = similar::resemble(&snapshot, &mut eye, &Wants::default()).expect("the pass runs");

    assert!(found.is_empty(), "identical numbers, different kinds");
}

#[test]
fn alike_does_not_chain() {
    // Each is close to the one before it and the ends are far apart. Joining
    // every close pair would put all three together and suggest deleting a
    // picture that looks like nothing else in the group.
    let snapshot = folder(&[
        at("a.jpg", 900, 10),
        at("b.jpg", 900, 20),
        at("c.jpg", 900, 30),
    ]);
    let mut eye = Eye::new(&[
        ("a.jpg", Sort::Picture, SCENE, 300),
        ("b.jpg", Sort::Picture, nudge(SCENE, 7), 200),
        ("c.jpg", Sort::Picture, nudge(SCENE, 14), 100),
    ]);

    let found = similar::resemble(&snapshot, &mut eye, &Wants::default()).expect("the pass runs");

    for cluster in &found.clusters {
        for near in &cluster.others {
            assert!(
                near.alike >= SIMILAR_AT,
                "{} joined {} at {}",
                near.copy.path,
                cluster.leader.path,
                near.alike
            );
        }
    }
}

#[test]
fn one_group_can_make_two_claims_of_different_strengths() {
    let snapshot = folder(&[
        at("shot.jpg", 3000, 10),
        at("shot copy.jpg", 2000, 20),
        at("a moment later.jpg", 1000, 30),
    ]);
    let mut eye = Eye::new(&[
        ("shot.jpg", Sort::Picture, SCENE, 300),
        ("shot copy.jpg", Sort::Picture, nudge(SCENE, 1), 200),
        ("a moment later.jpg", Sort::Picture, nudge(SCENE, 6), 100),
    ]);

    let found = similar::resemble(&snapshot, &mut eye, &Wants::default()).expect("the pass runs");

    let bands: Vec<Band> = found.clusters.iter().map(|one| one.band).collect();
    assert!(bands.contains(&Band::Same), "{bands:?}");
    assert!(bands.contains(&Band::Similar), "{bands:?}");
    for cluster in &found.clusters {
        assert_eq!(cluster.leader.path, "shot.jpg");
        for near in &cluster.others {
            match cluster.band {
                Band::Same => assert!(near.alike >= SAME_AT),
                Band::Similar => assert!(near.alike < SAME_AT && near.alike >= SIMILAR_AT),
            }
        }
    }
}

#[test]
fn what_the_exact_pass_already_took_is_not_offered_twice() {
    let snapshot = folder(&[at("one.jpg", 900, 10), at("a copy of one.jpg", 900, 20)]);
    let mut eye = Eye::new(&[
        ("one.jpg", Sort::Picture, SCENE, 100),
        ("a copy of one.jpg", Sort::Picture, SCENE, 100),
    ]);
    let wants = Wants {
        skip: ["a copy of one.jpg".to_string()].into_iter().collect(),
    };

    let found = similar::resemble(&snapshot, &mut eye, &wants).expect("the pass runs");

    assert!(found.is_empty());
    assert_eq!(found.looked, 1, "the skipped copy was never opened");
}

#[test]
fn a_file_that_cannot_be_read_is_named_rather_than_skipped() {
    let snapshot = folder(&[
        at("broken.jpg", 900, 10),
        at("fine.jpg", 900, 20),
        at("notes.txt", 900, 30),
    ]);
    let mut eye = Eye::new(&[
        ("broken.jpg", Sort::Picture, SCENE, 100),
        ("fine.jpg", Sort::Picture, SCENE, 100),
    ]);
    eye.broken.insert("broken.jpg".to_string());

    let found = similar::resemble(&snapshot, &mut eye, &Wants::default()).expect("the pass runs");

    assert_eq!(found.unchecked.len(), 1, "the one that failed is named");
    assert_eq!(found.unchecked[0].path, "broken.jpg");
    assert_eq!(found.unchecked[0].why, "not a picture after all");
    // A text file is not a failure: it was never a thing this looks at.
    assert_eq!(found.looked, 1);
}

#[test]
fn a_stopped_pass_says_stopped() {
    let snapshot = folder(&[
        at("one.jpg", 900, 10),
        at("two.jpg", 900, 20),
        at("three.jpg", 900, 30),
    ]);
    let mut eye = Eye::new(&[
        ("one.jpg", Sort::Picture, SCENE, 100),
        ("two.jpg", Sort::Picture, SCENE, 100),
        ("three.jpg", Sort::Picture, SCENE, 100),
    ]);
    eye.stop_after = Some(1);

    let stopped = similar::resemble(&snapshot, &mut eye, &Wants::default());

    assert_eq!(stopped, Err(Trouble::Stopped));
}

proptest! {
    /// However the numbers fall, a group never swallows its own leader and
    /// never holds a member that was not measured against that leader.
    #[test]
    fn every_group_keeps_its_leader_out_of_the_others(
        bits in prop::collection::vec(any::<u64>(), 1..24)
    ) {
        let files: Vec<Attributes> = bits
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let index = i64::try_from(index).unwrap_or(0);
                at(&format!("{index}.jpg"), 1000 + index.unsigned_abs(), index)
            })
            .collect();
        let looks: Vec<(String, Sort, u64, u64)> = bits
            .iter()
            .enumerate()
            .map(|(index, one)| (format!("{index}.jpg"), Sort::Picture, *one, index as u64))
            .collect();
        let borrowed: Vec<(&str, Sort, u64, u64)> = looks
            .iter()
            .map(|(path, sort, bits, weight)| (path.as_str(), *sort, *bits, *weight))
            .collect();

        let found = similar::resemble(&folder(&files), &mut Eye::new(&borrowed), &Wants::default())
            .expect("the pass runs");

        for cluster in &found.clusters {
            prop_assert!(!cluster.others.is_empty());
            for near in &cluster.others {
                prop_assert_ne!(&near.copy.path, &cluster.leader.path);
                prop_assert!(near.alike >= SIMILAR_AT);
            }
        }
        // No file is in two groups of the same band, which is what would let
        // one file be decided about twice.
        for band in [Band::Same, Band::Similar] {
            let mut seen = std::collections::BTreeSet::new();
            for cluster in found.clusters.iter().filter(|one| one.band == band) {
                for near in &cluster.others {
                    prop_assert!(seen.insert(near.copy.path.clone()));
                }
            }
        }
    }
}

proptest! {
    /// The tree is an optimisation, and an optimisation that misses answers is
    /// a duplicate finder that says your drive is clean. Against the same
    /// question asked the slow way, it must return every signature within the
    /// radius, and nothing outside it.
    #[test]
    fn the_tree_finds_everything_a_plain_scan_would(
        signatures in prop::collection::vec(any::<u64>(), 1..60),
        asked in any::<u64>(),
        radius in 0u32..64,
    ) {
        let mut tree = similar::Tree::default();
        for (label, one) in signatures.iter().enumerate() {
            tree.add(*one, label);
        }

        let mut found = tree.within(asked, radius);
        found.sort_unstable();
        let mut expected: Vec<usize> = signatures
            .iter()
            .enumerate()
            .filter(|(_, one)| (*one ^ asked).count_ones() <= radius)
            .map(|(label, _)| label)
            .collect();
        expected.sort_unstable();

        prop_assert_eq!(found, expected);
    }
}
