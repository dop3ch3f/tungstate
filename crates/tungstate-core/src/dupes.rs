//! Which files are the same file, and which copy is worth keeping.
//!
//! Pure, like the planner: a [`Snapshot`] goes in, groups come out, and the
//! only I/O is whatever the caller's [`Digest`] does when asked. That is what
//! lets the whole of this be tested with no filesystem, including the property
//! that matters most: **every group keeps one copy in place**.
//!
//! Three tiers, in the order DESIGN §5 sets out, because each one is paid for
//! by the one before it:
//!
//! 1. **Size**, which a directory listing already has. Only size-equal files
//!    can be identical, and in an ordinary folder most sizes are unique.
//! 2. **A partial digest** of each candidate, which is one short read.
//! 3. **The whole file**, only for what survives both.
//!
//! Names never enter into it. `holiday-final-2.mp4` and `IMG_4471.mov` land in
//! the same group when they are the same recording, which is the whole point.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::attrs::Attributes;
use crate::plan::{Op, Parked, Plan};
use crate::policy::Mode;
use crate::snapshot::{self, Snapshot};

/// Reading files, on behalf of a pass that does no I/O of its own.
///
/// Two levels rather than one, because the second is expensive: `partial` is
/// asked of every size-match, `whole` only of what survives it. An
/// implementation is free to remember answers; this pass never asks twice for
/// the same path anyway.
pub trait Digest {
    /// A digest of the ends of the file: enough to tell two different files
    /// apart cheaply, never enough to call two files the same.
    ///
    /// # Errors
    /// Whatever reading the file failed with, as a sentence.
    fn partial(&mut self, path: &str) -> Result<String, String>;

    /// A digest of every byte.
    ///
    /// # Errors
    /// Whatever reading the file failed with, as a sentence.
    fn whole(&mut self, path: &str) -> Result<String, String>;
}

/// What the caller wants of a pass.
#[derive(Debug, Clone, Default)]
pub struct Wants {
    /// Paths to keep whichever way the tie-break would have gone. One per
    /// group at most; two in the same group is an error, not a race.
    pub pinned: BTreeSet<String>,
    /// Paths that already sit where the folder's rules would put them, which
    /// wins ahead of every other tie-break. The caller works these out,
    /// because knowing them means classifying, and classifying is not this
    /// module's business.
    pub settled: BTreeSet<String>,
    /// Skip the directory-level grouping.
    pub files_only: bool,
    /// Group on samples rather than reading every byte.
    ///
    /// For a network volume, where reading a file in full pulls it across the
    /// network: four 64 KiB samples and the exact size cost about a five
    /// thousandth of a 400 MB video. A group found this way is marked
    /// [`Group::sure`] `= false`, and **nothing may be moved on the strength
    /// of it** until it has been confirmed in full.
    pub sampled: bool,
}

/// One copy of some content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copy {
    /// Root-relative path.
    pub path: String,
    /// Size in bytes. The same for every copy in a group, by definition.
    pub size: u64,
    /// Last modification time, where the backend reported one.
    pub mtime: Option<Timestamp>,
}

/// Why one copy was chosen over the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kept {
    /// You said so: `--keep`, or a `.tungstate-keep` beside it.
    Pinned,
    /// It is already where the folder's rules would put it.
    Settled,
    /// It is the oldest of them.
    Oldest,
    /// The shallowest path, then the shortest, then alphabetical: all three
    /// are the same rule, and none of them is interesting enough to separate.
    Path,
}

/// Files with identical content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The content digest, which is also the group's identity: a window can
    /// remember a selection across a rescan by it.
    pub id: String,
    /// Size of one copy, in bytes.
    pub size: u64,
    /// The copy that stays where it is.
    pub keep: String,
    /// Why that one.
    pub why: Kept,
    /// The copies that would be dealt with. Never empty.
    pub extras: Vec<Copy>,
    /// Whether every byte was compared.
    ///
    /// False when the group came from samples, which is almost certainly the
    /// same file and is not proof. The window says so, and anything about to
    /// act confirms first.
    pub sure: bool,
}

impl Group {
    /// Bytes that come back if every extra copy goes.
    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.size * self.extras.len() as u64
    }
}

/// Directories holding the same set of files, by content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderGroup {
    /// A digest of the contents, stable across a rescan.
    pub id: String,
    /// The directory that stays.
    pub keep: String,
    /// The other directories, each a copy of it.
    pub extras: Vec<String>,
    /// How many files one of them holds.
    pub files: usize,
    /// What one of them weighs.
    pub bytes: u64,
    /// Whether every byte was compared, as [`Group::sure`].
    pub sure: bool,
}

impl FolderGroup {
    /// Bytes that come back if every copied directory goes.
    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.bytes * self.extras.len() as u64
    }
}

/// Two or more names for one file on the disk.
///
/// A hard link, in other words. Not a duplicate: there is one copy of the
/// content and dealing with one name would reclaim nothing. Reported because
/// somebody looking at a list of identical files deserves to know which of
/// them are the same file (DESIGN §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Linked {
    /// What the storage calls the file, opaque and only ever compared.
    pub id: String,
    /// Every name it has, path order.
    pub names: Vec<String>,
    /// Size of the one file.
    pub size: u64,
}

/// What a pass found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// Groups of identical files, largest reclaim first.
    pub groups: Vec<Group>,
    /// Directories copied whole, largest reclaim first. Files inside these are
    /// not repeated in `groups`: one decision, not four hundred.
    pub folders: Vec<FolderGroup>,
    /// Files whose size matched something else, so they were digested.
    pub digested: usize,
    /// Files read in full.
    pub read_whole: usize,
    /// Names that turned out to be the same file as another name.
    pub linked: Vec<Linked>,
}

impl Found {
    /// Bytes that come back if every extra copy goes.
    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.groups
            .iter()
            .map(Group::reclaimable)
            .chain(self.folders.iter().map(FolderGroup::reclaimable))
            .sum()
    }

    /// Extra copies, counted as files. **Not the number of operations**: a
    /// plan also removes the directories it empties, and slice 7b shipped
    /// "moved 9 file(s)" for five moved files by confusing the two.
    #[must_use]
    pub fn extra_files(&self) -> usize {
        self.groups
            .iter()
            .map(|group| group.extras.len())
            .chain(
                self.folders
                    .iter()
                    .map(|group| group.files * group.extras.len()),
            )
            .sum()
    }

    /// Names that are the same file as another name, counted.
    #[must_use]
    pub fn linked_names(&self) -> usize {
        self.linked.iter().map(|one| one.names.len() - 1).sum()
    }

    /// Groups that were matched on samples and have not been confirmed.
    #[must_use]
    pub fn unsure(&self) -> usize {
        self.groups.iter().filter(|group| !group.sure).count()
            + self.folders.iter().filter(|group| !group.sure).count()
    }

    /// True when there is nothing to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.folders.is_empty()
    }
}

/// Anything that stops a pass finishing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Trouble {
    /// A file could not be read.
    #[error("{path}: {why}")]
    Unreadable {
        /// The file.
        path: String,
        /// What the backend said.
        why: String,
    },
    /// Two pinned paths hold the same content, so "keep this one" has two
    /// answers. Refused rather than resolved: the pin is the one instruction
    /// here that came from a person.
    #[error("both {first} and {second} are pinned, and they are the same file")]
    TwoPins {
        /// One of them.
        first: String,
        /// The other.
        second: String,
    },
}

/// Find the duplicates in a snapshot.
///
/// # Errors
/// [`Trouble::Unreadable`] if the digest fails, [`Trouble::TwoPins`] if two
/// pinned paths are the same content.
pub fn find(snapshot: &Snapshot, digest: &mut dyn Digest, wants: &Wants) -> Result<Found, Trouble> {
    let all: Vec<&Attributes> = snapshot
        .entries
        .iter()
        .filter(|entry| !entry.is_dir && !entry.is_symlink && entry.size > 0)
        .filter(|entry| !snapshot::is_reserved(&entry.relative_path()))
        .collect();

    // Two names for one file are not two files. Only the first name goes
    // forward, so a hard-linked pair is never offered as a duplicate whose
    // extra copy would reclaim nothing when it is dealt with.
    let mut by_identity: BTreeMap<&str, Vec<&Attributes>> = BTreeMap::new();
    for file in &all {
        if let Some(id) = file.identity.as_deref() {
            by_identity.entry(id).or_default().push(file);
        }
    }
    let mut linked = Vec::new();
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    for (id, names) in by_identity.iter().filter(|(_, names)| names.len() > 1) {
        let mut paths: Vec<String> = names.iter().map(|file| file.relative_path()).collect();
        // The plainest path first, the same order the tie-break uses, so the
        // name that goes forward is the one a person would call the file's
        // own rather than whichever sorted first.
        paths.sort_by_key(|path| (path.matches('/').count(), path.len(), path.clone()));
        for alias in paths.iter().skip(1) {
            aliases.insert(alias.clone());
        }
        linked.push(Linked {
            id: (*id).to_string(),
            names: paths,
            size: names[0].size,
        });
    }

    let files: Vec<&Attributes> = all
        .into_iter()
        .filter(|file| !aliases.contains(&file.relative_path()))
        .collect();

    // 1. Size. Anything alone at its size cannot be a duplicate, and is never
    //    read at all.
    let mut by_size: BTreeMap<u64, Vec<&Attributes>> = BTreeMap::new();
    for file in files {
        by_size.entry(file.size).or_default().push(file);
    }

    let mut found = Found {
        linked,
        ..Found::default()
    };
    let mut by_hash: BTreeMap<String, Vec<&Attributes>> = BTreeMap::new();
    for (_, candidates) in by_size.iter().filter(|(_, same)| same.len() > 1) {
        // 2. The ends of the file, which splits most same-size sets apart.
        let mut by_partial: BTreeMap<String, Vec<&Attributes>> = BTreeMap::new();
        for file in candidates {
            let path = file.relative_path();
            let mark = digest.partial(&path).map_err(|why| Trouble::Unreadable {
                path: path.clone(),
                why,
            })?;
            found.digested += 1;
            by_partial.entry(mark).or_default().push(file);
        }

        // 3. Every byte, and only now are two files called the same. Skipped
        //    on a network volume, where that means pulling both files across
        //    it; the samples stand in, and the group says it is unconfirmed.
        for (sampled, same_ends) in by_partial.iter().filter(|(_, same)| same.len() > 1) {
            for file in same_ends {
                if wants.sampled {
                    by_hash.entry(sampled.clone()).or_default().push(file);
                    continue;
                }
                let path = file.relative_path();
                let whole = digest.whole(&path).map_err(|why| Trouble::Unreadable {
                    path: path.clone(),
                    why,
                })?;
                found.read_whole += 1;
                by_hash.entry(whole).or_default().push(file);
            }
        }
    }

    let groups = groups_of(&by_hash, wants, !wants.sampled)?;
    let (folders, covered) = if wants.files_only {
        (Vec::new(), BTreeSet::new())
    } else {
        folders_of(snapshot, &by_hash, &groups, wants, !wants.sampled)
    };

    // A file inside a copied directory is dealt with by the directory, so it
    // is not offered twice.
    found.groups = groups
        .into_iter()
        .filter(|group| {
            !covered.contains(&group.keep)
                && group
                    .extras
                    .iter()
                    .any(|copy| !covered.contains(&copy.path))
        })
        .map(|mut group| {
            group.extras.retain(|copy| !covered.contains(&copy.path));
            group
        })
        .filter(|group| !group.extras.is_empty())
        .collect();
    found.folders = folders;
    found.groups.sort_by(|a, b| {
        b.reclaimable()
            .cmp(&a.reclaimable())
            .then_with(|| a.keep.cmp(&b.keep))
    });
    found.folders.sort_by(|a, b| {
        b.reclaimable()
            .cmp(&a.reclaimable())
            .then_with(|| a.keep.cmp(&b.keep))
    });
    Ok(found)
}

/// Turn "these paths share a digest" into "this one stays, those go".
fn groups_of(
    by_hash: &BTreeMap<String, Vec<&Attributes>>,
    wants: &Wants,
    sure: bool,
) -> Result<Vec<Group>, Trouble> {
    let mut groups = Vec::new();
    for (hash, copies) in by_hash.iter().filter(|(_, copies)| copies.len() > 1) {
        let paths: Vec<String> = copies.iter().map(|file| file.relative_path()).collect();
        let pinned: Vec<&String> = paths
            .iter()
            .filter(|path| wants.pinned.contains(*path))
            .collect();
        if pinned.len() > 1 {
            return Err(Trouble::TwoPins {
                first: pinned[0].clone(),
                second: pinned[1].clone(),
            });
        }

        let (keep, why) = choose(copies, wants);
        let extras = copies
            .iter()
            .filter(|file| file.relative_path() != keep)
            .map(|file| Copy {
                path: file.relative_path(),
                size: file.size,
                mtime: file.mtime,
            })
            .collect();
        groups.push(Group {
            id: hash.clone(),
            size: copies[0].size,
            keep,
            why,
            extras,
            sure,
        });
    }
    Ok(groups)
}

/// The tie-break from DESIGN §5: pinned, then already in its place, then the
/// oldest, then the path.
///
/// `birthtime` is what DESIGN names, and almost no backend reports one, so
/// mtime stands in for it. A copy usually carries the original's mtime, which
/// is what makes that the right stand-in rather than a convenient one.
fn choose(copies: &[&Attributes], wants: &Wants) -> (String, Kept) {
    let path_order = |file: &Attributes| {
        let path = file.relative_path();
        (path.matches('/').count(), path.len(), path)
    };

    if let Some(file) = copies
        .iter()
        .find(|file| wants.pinned.contains(&file.relative_path()))
    {
        return (file.relative_path(), Kept::Pinned);
    }
    let settled: Vec<&&Attributes> = copies
        .iter()
        .filter(|file| wants.settled.contains(&file.relative_path()))
        .collect();
    // Only when it separates them. If every copy is where the rules would put
    // it, "kept because it is in its place" is true of the others too, and a
    // reason that does not distinguish is a reason that misleads.
    if settled.len() < copies.len()
        && let Some(file) = settled.iter().min_by_key(|file| path_order(file))
    {
        return (file.relative_path(), Kept::Settled);
    }
    let oldest = copies.iter().filter_map(|file| file.mtime).min();
    if let Some(oldest) = oldest {
        let mut at_oldest: Vec<&&Attributes> = copies
            .iter()
            .filter(|file| file.mtime == Some(oldest))
            .collect();
        at_oldest.sort_by_key(|file| path_order(file));
        // Only interesting when it actually separates them: every copy sharing
        // one mtime is the ordinary case on a filesystem that rounds, and
        // saying "kept because oldest" there would be a lie about the reason.
        if at_oldest.len() < copies.len() {
            return (at_oldest[0].relative_path(), Kept::Oldest);
        }
    }
    let mut all: Vec<&&Attributes> = copies.iter().collect();
    all.sort_by_key(|file| path_order(file));
    (all[0].relative_path(), Kept::Path)
}

/// Directories whose files are the same files, by content and by layout.
///
/// Layout as well as content: two directories holding the same five videos
/// under different names are not "the same folder copied", and treating them
/// as one would hide a real difference behind a bulk action.
///
/// Returns the groups, and every file path they account for.
fn folders_of(
    snapshot: &Snapshot,
    by_hash: &BTreeMap<String, Vec<&Attributes>>,
    groups: &[Group],
    wants: &Wants,
    sure: bool,
) -> (Vec<FolderGroup>, BTreeSet<String>) {
    // Only a directory whose every file has a twin somewhere can be a copy.
    let twinned: BTreeMap<String, String> = by_hash
        .iter()
        .filter(|(_, copies)| copies.len() > 1)
        .flat_map(|(hash, copies)| {
            copies
                .iter()
                .map(move |file| (file.relative_path(), hash.clone()))
        })
        .collect();

    let mut by_contents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut weights: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for directory in &snapshot.directories {
        if snapshot::is_reserved(directory) {
            continue;
        }
        let inside: Vec<&Attributes> = snapshot
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .filter(|entry| entry.relative_path().starts_with(&format!("{directory}/")))
            .collect();
        if inside.is_empty() {
            continue;
        }
        let mut fingerprint = Vec::new();
        for file in &inside {
            let path = file.relative_path();
            let Some(hash) = twinned.get(&path) else {
                fingerprint.clear();
                break;
            };
            let Some(relative) = path.strip_prefix(&format!("{directory}/")) else {
                continue;
            };
            fingerprint.push(format!("{relative}\u{0}{hash}"));
        }
        if fingerprint.is_empty() {
            continue;
        }
        fingerprint.sort();
        let id = blake3::hash(fingerprint.join("\n").as_bytes())
            .to_hex()
            .to_string();
        weights.insert(
            directory.clone(),
            (inside.len(), inside.iter().map(|file| file.size).sum()),
        );
        by_contents.entry(id).or_default().push(directory.clone());
    }

    let mut folders = Vec::new();
    let mut covered = BTreeSet::new();
    for (id, mut directories) in by_contents.into_iter().filter(|(_, dirs)| dirs.len() > 1) {
        // A directory inside another copied directory is that one's business.
        if directories.iter().any(|dir| {
            folders
                .iter()
                .any(|group: &FolderGroup| inside_any(dir, group))
        }) {
            continue;
        }
        directories.sort_by_key(|dir| (dir.matches('/').count(), dir.len(), dir.clone()));
        let keep = directories
            .iter()
            .find(|dir| {
                wants
                    .pinned
                    .iter()
                    .any(|pin| pin == *dir || pin.starts_with(&format!("{dir}/")))
            })
            .cloned()
            .unwrap_or_else(|| directories[0].clone());
        let extras: Vec<String> = directories.into_iter().filter(|dir| *dir != keep).collect();
        let (files, bytes) = weights.get(&keep).copied().unwrap_or((0, 0));
        for extra in &extras {
            for group in groups {
                for copy in &group.extras {
                    if copy.path.starts_with(&format!("{extra}/")) {
                        covered.insert(copy.path.clone());
                    }
                }
                if group.keep.starts_with(&format!("{extra}/")) {
                    covered.insert(group.keep.clone());
                }
            }
        }
        folders.push(FolderGroup {
            id,
            keep,
            extras,
            files,
            bytes,
            sure,
        });
    }
    (folders, covered)
}

/// Whether `directory` sits inside one of a group's directories.
fn inside_any(directory: &str, group: &FolderGroup) -> bool {
    std::iter::once(&group.keep)
        .chain(group.extras.iter())
        .any(|other| directory.starts_with(&format!("{other}/")))
}

/// What happens to the copies that are not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Extras {
    /// Moved under [`snapshot::QUARANTINE`], inside the folder. One rename,
    /// so `undo` puts them back, and it works the same on a NAS.
    SetAside,
    /// Handed to the desktop's trash. Local only, and not undoable from here.
    Trash,
}

impl Extras {
    /// Whether a plan built with this can be undone.
    #[must_use]
    pub fn reversible(self) -> bool {
        matches!(self, Extras::SetAside)
    }
}

/// What to do about what was found, as a plan the executor already knows how
/// to carry out and the journal already knows how to record.
///
/// Directory groups are expanded here rather than in the executor: the person
/// decides about a folder, and the folder is its files.
#[must_use]
pub fn plan(snapshot: &Snapshot, found: &Found, extras: Extras, folder: &str, mode: Mode) -> Plan {
    let mut ops = Vec::new();
    // Names already spoken for in the set-aside area, under this volume's own
    // rules: on a case-insensitive disk `Photo.JPG` and `photo.jpg` are one
    // name, and two files sent to one name is a file lost at the moment of
    // the move.
    let mut taken: BTreeSet<String> = snapshot
        .occupied()
        .iter()
        .map(|path| snapshot.key(path))
        .collect();
    for group in &found.groups {
        for copy in &group.extras {
            ops.push(deal_with(
                &copy.path,
                &group.keep,
                extras,
                snapshot,
                &mut taken,
            ));
        }
    }
    for group in &found.folders {
        for extra in &group.extras {
            let prefix = format!("{extra}/");
            for file in snapshot.entries.iter().filter(|entry| !entry.is_dir) {
                let path = file.relative_path();
                if let Some(rest) = path.strip_prefix(&prefix) {
                    ops.push(deal_with(
                        &path,
                        &format!("{}/{rest}", group.keep),
                        extras,
                        snapshot,
                        &mut taken,
                    ));
                }
            }
        }
    }
    // Directories the plan itself empties go too, so a folder copied whole
    // leaves nothing behind but the one that stays.
    ops.extend(crate::plan::directory_ops(snapshot, &ops));

    let blast = crate::plan::measure(snapshot, &ops);
    let ops = crate::plan::order(ops, &snapshot.occupied());
    Plan {
        folder: folder.to_string(),
        mode,
        taken: snapshot.taken,
        snapshot: crate::plan::fingerprint(snapshot),
        ops,
        untouched: Vec::new(),
        blast,
        // A duplicate pass is not a policy: there is nothing for it to
        // converge to, and running it twice on the same folder finds nothing
        // the second time, which is the same thing said honestly.
        settles: true,
        unsettled: Vec::new(),
    }
}

/// One extra copy, dealt with the way the person asked.
fn deal_with(
    path: &str,
    keep: &str,
    extras: Extras,
    snapshot: &Snapshot,
    taken: &mut BTreeSet<String>,
) -> Op {
    match extras {
        Extras::SetAside => {
            let to = free_name(&format!("{}/{path}", snapshot::QUARANTINE), snapshot, taken);
            taken.insert(snapshot.key(&to));
            Op::Quarantine {
                from: path.to_string(),
                to,
                because: Parked::Duplicate {
                    of: keep.to_string(),
                },
            }
        }
        Extras::Trash => Op::Trash {
            path: path.to_string(),
            of: keep.to_string(),
        },
    }
}

/// A short, stable stand-in for a name, for the case that never happens.
fn uuid_ish(seed: &str) -> String {
    blake3::hash(seed.as_bytes()).to_hex()[..8].to_string()
}

/// `wanted`, or the first numbered variant of it nothing else claims.
///
/// Numbered before the extension, the way the planner numbers a name two
/// files both asked for, so `clip.mp4` becomes `clip-2.mp4`.
fn free_name(wanted: &str, snapshot: &Snapshot, taken: &BTreeSet<String>) -> String {
    if !taken.contains(&snapshot.key(wanted)) {
        return wanted.to_string();
    }
    let (stem, extension) = match wanted.rsplit_once('.') {
        Some((stem, extension)) if !stem.ends_with('/') => (stem, format!(".{extension}")),
        _ => (wanted, String::new()),
    };
    // Bounded: a thousand files under one name in one folder is not a real
    // folder, and an unbounded search here would be an unbounded loop.
    (2_u32..1_000)
        .map(|nth| format!("{stem}-{nth}{extension}"))
        .find(|candidate| !taken.contains(&snapshot.key(candidate)))
        .unwrap_or_else(|| format!("{stem}-{}{extension}", uuid_ish(wanted)))
}

/// Confirm a sampled group by comparing every byte, and say what survived.
///
/// Samples make a group worth showing; they never make it worth acting on.
/// Anything about to move files calls this first, and it reads only the group
/// being acted on rather than the folder.
///
/// # Errors
/// [`Trouble::Unreadable`] if a file cannot be read.
pub fn confirm(group: &Group, digest: &mut dyn Digest) -> Result<Option<Group>, Trouble> {
    if group.sure {
        return Ok(Some(group.clone()));
    }
    let read = |digest: &mut dyn Digest, path: &str| {
        digest.whole(path).map_err(|why| Trouble::Unreadable {
            path: path.to_string(),
            why,
        })
    };

    let keep = read(digest, &group.keep)?;
    let mut extras = Vec::new();
    for copy in &group.extras {
        if read(digest, &copy.path)? == keep {
            extras.push(copy.clone());
        }
    }
    if extras.is_empty() {
        // The samples agreed and the files do not. Nothing to do here, and
        // the caller says so rather than quietly moving on.
        return Ok(None);
    }
    Ok(Some(Group {
        id: keep,
        sure: true,
        extras,
        ..group.clone()
    }))
}

/// Confirm a sampled folder group by comparing every file in it.
///
/// A copied folder is its files, so confirming it is confirming each pair. A
/// directory whose files do not all match is dropped from the group rather
/// than the whole group being thrown away: one of three copies being
/// different is a fact about that one.
///
/// # Errors
/// [`Trouble::Unreadable`] if a file cannot be read.
pub fn confirm_folder(
    group: &FolderGroup,
    snapshot: &Snapshot,
    digest: &mut dyn Digest,
) -> Result<Option<FolderGroup>, Trouble> {
    if group.sure {
        return Ok(Some(group.clone()));
    }
    let read = |digest: &mut dyn Digest, path: &str| {
        digest.whole(path).map_err(|why| Trouble::Unreadable {
            path: path.to_string(),
            why,
        })
    };

    let inside: Vec<String> = snapshot
        .entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .filter_map(|entry| {
            entry
                .relative_path()
                .strip_prefix(&format!("{}/", group.keep))
                .map(ToString::to_string)
        })
        .collect();

    let mut wanted = BTreeMap::new();
    for relative in &inside {
        let path = format!("{}/{relative}", group.keep);
        wanted.insert(relative.clone(), read(digest, &path)?);
    }

    let mut extras = Vec::new();
    for extra in &group.extras {
        let mut matches = true;
        for (relative, expected) in &wanted {
            let path = format!("{extra}/{relative}");
            if read(digest, &path)? != *expected {
                matches = false;
                break;
            }
        }
        if matches {
            extras.push(extra.clone());
        }
    }
    if extras.is_empty() {
        return Ok(None);
    }
    Ok(Some(FolderGroup {
        extras,
        sure: true,
        ..group.clone()
    }))
}
