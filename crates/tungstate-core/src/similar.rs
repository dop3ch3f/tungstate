//! Which files are nearly the same file.
//!
//! The exact pass in [`crate::dupes`] can lean on size: two files of different
//! lengths cannot be identical, and in an ordinary folder most lengths are
//! unique, so most files are never read. None of that is available here. A
//! photograph exported at half the size is a different length by design, so
//! every candidate has to be looked at, and then every candidate compared
//! against every other one.
//!
//! Compared against every other one is [`O(n²)`], which is fine for a thousand
//! files and is not fine for a hundred thousand. So comparison happens in two
//! steps, the same shape the exact pass uses for reading:
//!
//! 1. Each file has a **64 bit signature**. A [`Tree`] holds them and answers
//!    *everything within 12 bits of this* by visiting a small part of itself.
//! 2. Only what that returns is compared in **detail**, which is the
//!    expensive and accurate answer.
//!
//! Pure, like the rest of core: the reading, the decoding and the arithmetic
//! all live behind [`Look`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dupes::{Bundle, Copy, STOPPED, Trouble};
use crate::snapshot::{self, Snapshot};

/// How many of the 64 signature bits may differ before two files are not even
/// worth comparing in detail.
///
/// Measured rather than guessed: re-encoded, resized and cropped copies of one
/// picture come out with identical signatures, and unrelated pictures sit 17
/// bits or more apart. Twelve leaves room either side of that gap.
/// `cargo run -p tungstate-likeness --example score` is how it was measured
/// and how to measure it again.
pub const RADIUS: u32 = 12;

/// Alike enough to call it the same thing in a different wrapper.
pub const SAME_AT: u8 = 90;

/// Alike enough to be worth showing somebody.
pub const SIMILAR_AT: u8 = 70;

/// The kinds of file this pass looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// A still picture.
    Picture,
    /// Moving pictures.
    Moving,
    /// Sound.
    Sound,
}

/// A file's likeness, as numbers this module can index and compare.
///
/// Plain data on purpose. Core does not decode anything and does not depend on
/// anything that does; the shape of these numbers is the whole of what it
/// knows about how they were arrived at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// Which algorithm produced it. Never compared across two of these.
    pub algo: String,
    /// The 64 bits the tree indexes.
    pub signature: u64,
    /// What decides a pair the tree brought together.
    pub detail: Vec<u64>,
    /// How much of the thing there is: pixels for a picture, samples for
    /// sound. Decides which copy leads a group, because the copy with the
    /// most in it is the one worth keeping.
    pub weight: u64,
}

/// Looking at files, on behalf of a pass that does no I/O of its own.
pub trait Look {
    /// Which kind of file this is, without opening it.
    ///
    /// Cheap by contract: it is asked about every file in the folder, and
    /// answering `None` is how the vast majority of them are never opened.
    fn sort(&self, path: &str, mime: Option<&str>) -> Option<Sort>;

    /// Fingerprint one file.
    ///
    /// # Errors
    /// A sentence saying why not, which is reported rather than fatal.
    fn print(&mut self, path: &str) -> Result<Mark, String>;

    /// How alike two fingerprints are, from 0 to 100, or `None` when they
    /// cannot be compared at all.
    fn alike(&self, one: &Mark, other: &Mark) -> Option<u8>;
}

/// One file that resembles the one leading its group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Near {
    /// Where it is, how big, and when it was written.
    pub copy: Copy,
    /// How alike, from 0 to 100.
    pub alike: u8,
}

/// How strong the claim about a group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    /// The same thing in a different wrapper: a re-export, a resize, a second
    /// save at another quality.
    Same,
    /// Alike enough to be worth a look, which is usually two photographs of
    /// one moment rather than two copies of one photograph.
    Similar,
}

/// Files that resemble one another, around the best of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cluster {
    /// Stable across a rescan, so a window can remember a selection.
    pub id: String,
    /// What kind of files these are.
    pub sort: Sort,
    /// How strong the claim is.
    pub band: Band,
    /// The best copy present, and the one every other member was measured
    /// against.
    pub leader: Copy,
    /// The rest, closest first.
    pub others: Vec<Near>,
}

impl Cluster {
    /// Bytes that come back if every other member goes.
    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.others.iter().map(|near| near.copy.size).sum()
    }

    /// Every path in the group, the leader first.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.leader.path.as_str())
            .chain(self.others.iter().map(|near| near.copy.path.as_str()))
    }
}

/// A file that could not be fingerprinted, and why.
///
/// Counted and named rather than skipped: a finder that quietly passes over
/// what it cannot read tells you your drive is clean when it has not looked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unchecked {
    /// The file.
    pub path: String,
    /// What went wrong, in the words of whatever tried.
    pub why: String,
}

/// What a pass found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resembling {
    /// Groups, largest reclaim first.
    pub clusters: Vec<Cluster>,
    /// How many files were fingerprinted.
    pub looked: usize,
    /// The ones that could not be.
    pub unchecked: Vec<Unchecked>,
}

impl Resembling {
    /// Bytes that come back if every group keeps only its leader.
    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.clusters.iter().map(Cluster::reclaimable).sum()
    }

    /// True when there is nothing to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }

    /// Every group, in the shape [`crate::dupes::decide`] needs.
    #[must_use]
    pub fn bundles(&self) -> Vec<Bundle<'_>> {
        self.clusters
            .iter()
            .map(|cluster| Bundle {
                id: &cluster.id,
                members: cluster.paths().collect(),
                folder: false,
            })
            .collect()
    }
}

/// What the caller wants of a pass.
#[derive(Debug, Clone, Default)]
pub struct Wants {
    /// Paths the exact pass has already accounted for.
    ///
    /// An extra copy that is byte for byte identical is not news here, and
    /// listing it in both halves of the window is how a person ends up
    /// deciding about one file twice.
    pub skip: BTreeSet<String>,
}

/// Find what resembles what.
///
/// # Errors
/// [`Trouble::Stopped`] if the person stopped it. Nothing else here is fatal:
/// a file that cannot be read is [`Unchecked`].
pub fn resemble(
    snapshot: &Snapshot,
    look: &mut dyn Look,
    wants: &Wants,
) -> Result<Resembling, Trouble> {
    let mut marked: Vec<(Copy, Sort, Mark)> = Vec::new();
    let mut unchecked = Vec::new();

    for file in &snapshot.entries {
        let path = file.relative_path();
        if file.is_dir || file.is_symlink || file.size == 0 {
            continue;
        }
        if snapshot::is_reserved(&path) || wants.skip.contains(&path) {
            continue;
        }
        let Some(sort) = look.sort(&path, file.mime.as_deref()) else {
            continue;
        };
        match look.print(&path) {
            Ok(mark) => marked.push((
                Copy {
                    path: path.clone(),
                    size: file.size,
                    mtime: file.mtime,
                },
                sort,
                mark,
            )),
            Err(why) if why == STOPPED => return Err(Trouble::Stopped),
            Err(why) => unchecked.push(Unchecked { path, why }),
        }
    }

    let looked = marked.len();
    // The best copy has to lead its group, and which copy is best is only
    // known once everything has been looked at. Sorting here rather than
    // choosing later is what makes the leader the first member the clustering
    // meets, which is the whole of the rule.
    marked.sort_by(|(one, _, left), (other, _, right)| {
        right
            .weight
            .cmp(&left.weight)
            .then(other.size.cmp(&one.size))
            .then(one.mtime.cmp(&other.mtime))
            .then(one.path.cmp(&other.path))
    });

    let mut clusters = Vec::new();
    for sort in [Sort::Picture, Sort::Moving, Sort::Sound] {
        let of_this_sort: Vec<&(Copy, Sort, Mark)> =
            marked.iter().filter(|(_, it, _)| *it == sort).collect();
        clusters.extend(cluster(&of_this_sort, sort, look));
    }
    clusters.sort_by(|one, other| {
        other
            .reclaimable()
            .cmp(&one.reclaimable())
            .then(one.id.cmp(&other.id))
    });

    Ok(Resembling {
        clusters,
        looked,
        unchecked,
    })
}

/// Group one kind of file around leaders.
///
/// Alike is not a relation that chains: A can resemble B and B resemble C
/// while A and C have nothing in common. Joining every close pair into one
/// blob is how these tools end up suggesting somebody delete a photograph that
/// looks like nothing else in the group. So every member is measured against
/// **the leader**, never against a neighbour, which bounds how different two
/// members can be.
fn cluster(files: &[&(Copy, Sort, Mark)], sort: Sort, look: &dyn Look) -> Vec<Cluster> {
    let mut tree = Tree::default();
    let mut leaders: Vec<(&Copy, &Mark, Vec<Near>)> = Vec::new();

    for (copy, _, mark) in files {
        let mut best: Option<(usize, u8)> = None;
        for at in tree.within(mark.signature, RADIUS) {
            let (_, leader, _) = &leaders[at];
            let Some(score) = look.alike(leader, mark) else {
                continue;
            };
            if score >= SIMILAR_AT && best.is_none_or(|(_, so_far)| score > so_far) {
                best = Some((at, score));
            }
        }
        if let Some((at, score)) = best {
            leaders[at].2.push(Near {
                copy: (*copy).clone(),
                alike: score,
            });
        } else {
            tree.add(mark.signature, leaders.len());
            leaders.push((copy, mark, Vec::new()));
        }
    }

    let mut out = Vec::new();
    for (leader, mark, mut others) in leaders {
        if others.is_empty() {
            continue;
        }
        others.sort_by(|one, other| {
            other
                .alike
                .cmp(&one.alike)
                .then(one.copy.path.cmp(&other.copy.path))
        });
        // A group holding both a re-export and a photograph of the same moment
        // makes two claims of different strengths. Splitting them keeps the
        // safe one safe instead of dragging it down to the weaker wording.
        let (same, similar): (Vec<Near>, Vec<Near>) =
            others.drain(..).partition(|near| near.alike >= SAME_AT);
        for (band, members) in [(Band::Same, same), (Band::Similar, similar)] {
            if members.is_empty() {
                continue;
            }
            out.push(Cluster {
                id: format!(
                    "{}-{:016x}-{}",
                    match band {
                        Band::Same => "same",
                        Band::Similar => "like",
                    },
                    mark.signature,
                    leader.path
                ),
                sort,
                band,
                leader: leader.clone(),
                others: members,
            });
        }
    }
    out
}

/// A tree of 64 bit signatures that answers "what is within `radius` of this".
///
/// A BK-tree. Every node keeps its children under the distance from itself, so
/// a search that has reached a node at distance `d` need only walk the
/// children numbered `d - radius` to `d + radius`. That holds because Hamming
/// distance obeys the triangle inequality, and it is the whole trick: the rest
/// of the tree is skipped without being looked at.
#[derive(Default)]
pub(crate) struct Tree {
    nodes: Vec<Node>,
}

struct Node {
    signature: u64,
    /// What the caller wanted to remember about it.
    label: usize,
    /// Children, by their distance from this node.
    children: BTreeMap<u32, usize>,
}

impl Tree {
    pub(crate) fn add(&mut self, signature: u64, label: usize) {
        let fresh = self.nodes.len();
        if self.nodes.is_empty() {
            self.nodes.push(Node {
                signature,
                label,
                children: BTreeMap::new(),
            });
            return;
        }
        let mut at = 0;
        loop {
            let apart = (self.nodes[at].signature ^ signature).count_ones();
            // An exact repeat of a signature belongs under it at distance
            // zero, which is a child like any other rather than a special case.
            if let Some(next) = self.nodes[at].children.get(&apart) {
                at = *next;
            } else {
                self.nodes[at].children.insert(apart, fresh);
                self.nodes.push(Node {
                    signature,
                    label,
                    children: BTreeMap::new(),
                });
                return;
            }
        }
    }

    pub(crate) fn within(&self, signature: u64, radius: u32) -> Vec<usize> {
        let mut found = Vec::new();
        if self.nodes.is_empty() {
            return found;
        }
        let mut to_visit = vec![0usize];
        while let Some(at) = to_visit.pop() {
            let node = &self.nodes[at];
            let apart = (node.signature ^ signature).count_ones();
            if apart <= radius {
                found.push(node.label);
            }
            let low = apart.saturating_sub(radius);
            for (_, child) in node.children.range(low..=apart + radius) {
                to_visit.push(*child);
            }
        }
        found
    }
}
