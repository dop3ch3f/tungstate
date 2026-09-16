//! Everything one pass over a folder saw, as of one moment.
//!
//! Pure data. `tungstate-attrs` fills one in by reading a backend; the planner
//! only ever reads it. Nothing here knows where the folder lives on disk, which
//! is what keeps the whole planner testable with no filesystem.

use std::collections::BTreeSet;

use jiff::Timestamp;

use crate::attrs::Attributes;

/// Where a file the planner refuses to decide about is parked.
///
/// Spelled exactly as `tungstate-transfer` spells it, because a person looking
/// for something set aside should find one directory whichever half of the
/// product set it aside. The two definitions are separate only so that the
/// transfer engine need not depend on the policy model for a string; slice 7
/// unifies them when the executor forces the question.
pub const QUARANTINE: &str = ".tungstate-quarantine";

/// The directories tungstate keeps its own things in, which it never governs.
///
/// Reserved independently of the policy's `ignore` list, for two different
/// reasons. Without `.tungstate`, nothing stops a policy routing its own
/// `policy.toml` into `Documents/`. Without `.tungstate-quarantine`, a file
/// parked there would be classified again on the next pass and routed straight
/// back out — so planning would never converge, which is the one property the
/// whole design rests on.
pub const RESERVED: [&str; 2] = [".tungstate", QUARANTINE];

/// A folder as one pass over it found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The moment the pass began. Every entry's `now` is this instant, so
    /// every file in a plan is classified as of one time rather than as of
    /// whenever the walk happened to reach it.
    pub taken: Timestamp,
    /// Every file, plus every directory the walk was told not to descend into.
    /// Sorted by path, and paths are unique.
    ///
    /// Full attributes rather than a decision already made: that is what lets
    /// a plan be applied to a snapshot on paper and the result classified
    /// again, which is the whole of the convergence test.
    pub entries: Vec<Attributes>,
    /// Every directory that exists, root-relative. The root itself is not in it.
    pub directories: BTreeSet<String>,
    /// False when paths differing only in case are the same file, which is the
    /// default on macOS and on Windows. From `Backend::capabilities()`.
    pub case_sensitive: bool,
}

impl Snapshot {
    /// Build a snapshot, sorting the entries so a plan over one tree is the
    /// same however the filesystem happened to list it.
    #[must_use]
    pub fn new(
        taken: Timestamp,
        mut entries: Vec<Attributes>,
        directories: BTreeSet<String>,
        case_sensitive: bool,
    ) -> Self {
        entries.sort_by_key(Attributes::relative_path);
        Self {
            taken,
            entries,
            directories,
            case_sensitive,
        }
    }

    /// Every path that is occupied, whether by a file or by a directory.
    ///
    /// What the planner asks before deciding a name is free.
    #[must_use]
    pub fn occupied(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .map(Attributes::relative_path)
            .chain(self.directories.iter().cloned())
            .collect()
    }

    /// The key a path is compared under.
    ///
    /// Lowercased on a case-insensitive volume, because `Photo.JPG` and
    /// `photo.jpg` are one file there — plan them as two and they collide at
    /// the moment of applying, which is the worst time to find out.
    #[must_use]
    pub fn key(&self, path: &str) -> String {
        if self.case_sensitive {
            path.to_string()
        } else {
            path.to_lowercase()
        }
    }
}

/// Whether `path` is one of the [`RESERVED`] directories, or inside one.
///
/// A prefix test on the string alone would catch `.tungstaterc` as well, so
/// the match is on whole path segments.
#[must_use]
pub fn is_reserved(path: &str) -> bool {
    RESERVED
        .iter()
        .any(|reserved| path == *reserved || path.starts_with(&format!("{reserved}/")))
}

/// Every directory above `path`, shallowest first, excluding `path` itself.
///
/// `a/b/c.mp4` gives `["a", "a/b"]`. The order is what a caller creating them
/// wants, and reversing it is what a caller removing them wants.
#[must_use]
pub fn ancestors(path: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut so_far = String::new();
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    for part in parts {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(part);
        found.push(so_far.clone());
    }
    found
}
