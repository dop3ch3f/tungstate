//! Finding duplicates from the window: a scan that can be watched and stopped,
//! and clearing what the person picked.
//!
//! The thinking is `tungstate_core::dupes`; this is the seam around it. Two
//! things here that no other folder command has needed:
//!
//! - **Progress.** A drive scan takes minutes, so it reports as it goes rather
//!   than returning in silence. `docs/SEAM.md` lists progress for a long pass
//!   as a known gap; this fills it for the pass that needs it most.
//! - **A stop.** The same flag the drain uses. A scan only reads, so stopping
//!   it leaves nothing half-done.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jiff::Timestamp;
use serde::Serialize;
use tungstate_backend::Backend;
use tungstate_core::dupes::{self, Extras, Trouble, Wants};
use tungstate_core::policy::Mode;
use tungstate_core::similar::{self, Band as Closeness};
use tungstate_execute::digest::{Cached, Watch};
use tungstate_execute::eye::{Eye, Seen};
use tungstate_execute::thumbs::Thumbs;
use tungstate_journal::Journal;

/// Where the answer to "set aside or trash?" is kept, shared with the command
/// line so the question is asked once for the whole product.
pub const ACTION_SETTING: &str = "dedupe.extras";

/// Where the last few scanned places are kept.
pub const RECENT_SETTING: &str = "dupes.recent";

/// How many are worth offering. More than this is a list to read rather than
/// a shortcut to click.
const RECENT_KEPT: usize = 6;

/// The places scanned before, newest first.
///
/// # Errors
/// [`tungstate_journal::JournalError`] as a sentence, if the row cannot be read.
pub fn recent(journal: &Journal) -> Result<Vec<String>, String> {
    let stored = journal
        .setting(RECENT_SETTING)
        .map_err(|error| error.to_string())?;
    Ok(stored
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default())
}

/// Remember a place, newest first and without repeats.
fn remember(journal: &Journal, target: &str) {
    let mut places = recent(journal).unwrap_or_default();
    places.retain(|place| place != target);
    places.insert(0, target.to_string());
    places.truncate(RECENT_KEPT);
    if let Ok(text) = serde_json::to_string(&places) {
        let _ = journal.remember_setting(RECENT_SETTING, &text);
    }
}

/// How far a scan has got.
#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    /// Files the pass has considered.
    pub looked: usize,
    /// Files read from the storage.
    pub read: usize,
    /// Files whose digest was already known.
    pub recalled: usize,
    /// Bytes pulled from the storage.
    pub bytes: u64,
    /// The file being handled.
    pub path: String,
    /// Which pass is running: looking for identical files, or for ones that
    /// merely resemble each other.
    pub stage: &'static str,
}

/// Which drawer of the window a group belongs in.
///
/// Wider than the kinds the fingerprinting knows about, because this answers
/// "where did my disk go" as well as "what can be deleted", and a folder full
/// of archives is an answer to the first question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Still pictures.
    Pictures,
    /// Moving pictures.
    Video,
    /// Sound.
    Sound,
    /// Anything somebody reads or writes.
    Documents,
    /// Zips, disk images, and the rest.
    Archives,
    /// A macOS application, which is a folder pretending to be a file.
    Applications,
    /// A whole directory copied.
    Folders,
    /// Everything else.
    Other,
}

/// Which kind of thing a path is, from its name.
#[must_use]
pub fn kind_of(path: &str, folder: bool) -> Kind {
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "app" {
        return Kind::Applications;
    }
    if folder {
        return Kind::Folders;
    }
    match tungstate_likeness::kind_of(None, name) {
        Some(tungstate_likeness::Kind::Picture) => Kind::Pictures,
        Some(tungstate_likeness::Kind::Moving) => Kind::Video,
        Some(tungstate_likeness::Kind::Sound) => Kind::Sound,
        None => match extension.as_str() {
            "pdf" | "doc" | "docx" | "pages" | "txt" | "md" | "rtf" | "odt" | "xls" | "xlsx"
            | "numbers" | "csv" | "ppt" | "pptx" | "key" | "epub" => Kind::Documents,
            "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "dmg" | "iso" | "pkg" => {
                Kind::Archives
            }
            _ => Kind::Other,
        },
    }
}

/// How strong the claim about a group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Claim {
    /// Byte for byte. Proof.
    Identical,
    /// The same thing in a different wrapper.
    Same,
    /// Looks like the same moment.
    Similar,
}

/// One copy, as a row in the middle pane.
#[derive(Debug, Clone, Serialize)]
pub struct CopyView {
    /// Where it is, relative to what was scanned.
    pub path: String,
    /// Just the name, for the row.
    pub name: String,
    /// Size in bytes. A directory's is everything under it.
    pub size: u64,
    /// When it was written, where the storage says.
    pub mtime: Option<Timestamp>,
    /// True for the copy the pass would keep, which is the one that starts
    /// unticked.
    pub keep: bool,
    /// How alike this copy is to the kept one. Absent when the match is proof.
    pub alike: Option<u8>,
    /// The small picture of it, if one was made.
    pub thumb: Option<String>,
}

/// One group of copies, whichever pass found it.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// Stable across a rescan, so a selection survives one.
    pub id: String,
    /// Which drawer it belongs in.
    pub kind: Kind,
    /// How strong the claim is.
    pub claim: Claim,
    /// True when the copies are whole directories.
    pub folder: bool,
    /// Bytes that come back if every copy but one goes.
    pub reclaimable: u64,
    /// Whether every byte was compared, for the copies that were.
    pub sure: bool,
    /// How many files one directory holds. Zero for a file group.
    pub files: usize,
    /// The copies, the kept one first.
    pub copies: Vec<CopyView>,
}

/// What a scan found, as the window draws it.
#[derive(Debug, Clone, Serialize)]
pub struct FoundView {
    /// What was scanned, spelled the way it was asked for.
    pub root: String,
    /// Every group, largest reclaim first.
    pub rows: Vec<Row>,
    /// Names that are the same file as another name.
    pub linked: Vec<dupes::Linked>,
    /// Files that could not be looked at, and why.
    pub unchecked: Vec<similar::Unchecked>,
    /// Files looked at.
    pub files: usize,
    /// Extra copies, as files rather than operations.
    pub extra_files: usize,
    /// Bytes that would come back.
    pub reclaimable: u64,
    /// Groups matched on samples, not yet confirmed.
    pub unsure: usize,
    /// Whether the bytes are a network away, which is why they were sampled.
    pub networked: bool,
    /// Whether the desktop's trash can be offered for this place.
    pub can_trash: bool,
    /// Whether resemblances were looked for at all.
    pub looked_alike: bool,
}

/// What clearing did.
#[derive(Debug, Clone, Serialize)]
pub struct ClearedView {
    /// Files dealt with. Not operations: a plan also removes the directories
    /// it empties.
    pub files: usize,
    /// Bytes reclaimed.
    pub bytes: u64,
    /// The plan, for putting it back.
    pub plan: i64,
    /// Whether it can be put back at all.
    pub reversible: bool,
    /// Groups the confirmation found were not identical after all.
    pub dropped: usize,
    /// Files that could not be dealt with.
    pub failed: Vec<String>,
}

/// A scan in flight.
#[derive(Debug, Default)]
pub struct Scan {
    stop: Arc<AtomicBool>,
}

impl Scan {
    /// Ask whatever is scanning to stop.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// A fresh flag for a new scan, so a stop cannot carry over into it.
    fn begin(&self) -> Arc<AtomicBool> {
        self.stop.store(false, Ordering::Relaxed);
        Arc::clone(&self.stop)
    }
}

/// Everything a pass needs, opened once and handed round.
struct Place {
    backend: Box<dyn Backend>,
    root: String,
    networked: bool,
}

/// Open whatever the person pointed at: a folder, or `connection:folder`.
fn place(target: &str, journal: &Journal) -> Result<Place, String> {
    let end = tungstate_journal::ends::parse_end(target, None, journal)
        .map_err(|error| error.to_string())?;
    let backend = tungstate_backend_opendal::open(&end, journal, &crate::secrets())
        .map_err(|error| error.to_string())?;
    let networked = backend.capabilities().networked;
    Ok(Place {
        backend,
        root: target.to_string(),
        networked,
    })
}

/// A snapshot of anything, governed or not.
///
/// The probe policy, as `learn` uses: duplicate-finding has nothing to do with
/// rules, so needing a governed folder would be a rule invented by plumbing.
fn look(backend: &dyn Backend) -> Result<tungstate_core::Snapshot, String> {
    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    tungstate_attrs::survey(backend, &probe).map_err(|error| error.to_string())
}

/// Scan, reporting progress through `watcher` and stopping when asked.
///
/// # Errors
/// A sentence: the place could not be opened, a file could not be read, or
/// the scan was stopped.
pub fn scan(
    target: &str,
    also_similar: bool,
    journal: &Journal,
    scan: &Scan,
    thumbs: Option<&Thumbs>,
    watcher: &mut (dyn FnMut(&ScanProgress) + Send),
) -> Result<FoundView, String> {
    let place = place(target, journal)?;
    if also_similar && place.networked {
        return Err(
            "finding files that are nearly the same means decoding every one of them, and over a connection that means downloading every one of them. Try a folder on this machine."
                .to_string(),
        );
    }
    let snapshot = look(place.backend.as_ref())?;
    let stop = scan.begin();
    let files = snapshot
        .entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .count();

    let found = {
        let stop = Arc::clone(&stop);
        let mut digest = Cached::new(place.backend.as_ref(), journal, &place.root)
            .watched_by(Box::new(|watch: &Watch| {
                watcher(&ScanProgress {
                    looked: watch.looked,
                    read: watch.read,
                    recalled: watch.recalled,
                    bytes: watch.bytes,
                    path: watch.path.clone(),
                    stage: "identical",
                });
            }))
            .stopping_when(Box::new(move || stop.load(Ordering::Relaxed)));
        let wants = Wants {
            // Over a network, reading every byte means pulling every byte, so
            // the pass samples and the groups say they are unconfirmed.
            sampled: place.networked,
            ..Wants::default()
        };
        dupes::find(&snapshot, &mut digest, &wants).map_err(describe)?
    };

    // The second pass only ever runs on this machine, so it reads the
    // filesystem directly rather than through the backend: decoding needs the
    // whole file and there is no sampled shortcut the way there is for a hash.
    let resembling = if also_similar {
        let stop = Arc::clone(&stop);
        let here = PathBuf::from(&place.root);
        let mut eye = Eye::new(&here, journal)
            .watched_by(Box::new(|seen: &Seen| {
                watcher(&ScanProgress {
                    looked: seen.looked,
                    read: seen.decoded,
                    recalled: seen.recalled,
                    bytes: 0,
                    path: seen.path.clone(),
                    stage: "alike",
                });
            }))
            .stopping_when(Box::new(move || stop.load(Ordering::Relaxed)));
        if let Some(thumbs) = thumbs {
            eye = eye.keeping_pictures(thumbs);
        }
        let wants = similar::Wants {
            skip: found
                .groups
                .iter()
                .flat_map(|group| group.extras.iter().map(|copy| copy.path.clone()))
                .collect(),
        };
        let resembling = similar::resemble(&snapshot, &mut eye, &wants).map_err(describe)?;
        Some((resembling, eye))
    } else {
        None
    };

    remember(journal, &place.root);
    let mut rows = rows_of(&found, &snapshot);
    let mut unchecked = Vec::new();
    if let Some((resembling, eye)) = resembling {
        rows.extend(resembling_rows(&resembling, &eye));
        unchecked = resembling.unchecked;
    }
    rows.sort_by(|one, other| {
        other
            .reclaimable
            .cmp(&one.reclaimable)
            .then(one.id.cmp(&other.id))
    });

    Ok(FoundView {
        root: place.root,
        files,
        extra_files: rows.iter().map(Row::extra_files).sum(),
        reclaimable: rows.iter().map(|row| row.reclaimable).sum(),
        unsure: found.unsure(),
        networked: place.networked,
        // The desktop's trash is this machine's, so a connection is offered
        // set-aside and nothing else.
        can_trash: place
            .backend
            .on_this_machine(Path::new(""))
            .is_ok_and(|found| found.is_some()),
        looked_alike: also_similar,
        rows,
        linked: found.linked,
        unchecked,
    })
}

impl Row {
    /// Copies that would be dealt with if the whole group were ticked, counted
    /// as **files**: a directory group is its files, and a plan also removes
    /// the directories it empties.
    fn extra_files(&self) -> usize {
        let extras = self.copies.len() - 1;
        if self.folder {
            extras * self.files
        } else {
            extras
        }
    }
}

/// The exact pass's findings, as rows.
fn rows_of(found: &dupes::Found, snapshot: &tungstate_core::Snapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    for group in &found.groups {
        let mut copies = vec![as_copy(&group.kept, true, None)];
        copies.extend(group.extras.iter().map(|copy| as_copy(copy, false, None)));
        rows.push(Row {
            id: group.id.clone(),
            kind: kind_of(&group.keep, false),
            claim: Claim::Identical,
            folder: false,
            reclaimable: group.reclaimable(),
            sure: group.sure,
            files: 0,
            copies,
        });
    }
    for group in &found.folders {
        let when = |path: &str| {
            snapshot
                .entries
                .iter()
                .filter(|entry| entry.relative_path().starts_with(&format!("{path}/")))
                .filter_map(|entry| entry.mtime)
                .max()
        };
        let mut copies = vec![CopyView {
            name: leaf(&group.keep),
            path: group.keep.clone(),
            size: group.bytes,
            mtime: when(&group.keep),
            keep: true,
            alike: None,
            thumb: None,
        }];
        copies.extend(group.extras.iter().map(|extra| CopyView {
            name: leaf(extra),
            path: extra.clone(),
            size: group.bytes,
            mtime: when(extra),
            keep: false,
            alike: None,
            thumb: None,
        }));
        rows.push(Row {
            id: group.id.clone(),
            kind: kind_of(&group.keep, true),
            claim: Claim::Identical,
            folder: true,
            reclaimable: group.reclaimable(),
            sure: group.sure,
            files: group.files,
            copies,
        });
    }
    rows
}

/// The second pass's findings, as rows, with their pictures attached.
fn resembling_rows(resembling: &similar::Resembling, eye: &Eye<'_>) -> Vec<Row> {
    resembling
        .clusters
        .iter()
        .map(|cluster| {
            let picture = |path: &str| {
                eye.picture_of(path).and_then(|at| {
                    at.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                })
            };
            let mut copies = vec![CopyView {
                name: leaf(&cluster.leader.path),
                path: cluster.leader.path.clone(),
                size: cluster.leader.size,
                mtime: cluster.leader.mtime,
                keep: true,
                alike: None,
                thumb: picture(&cluster.leader.path),
            }];
            copies.extend(cluster.others.iter().map(|near| CopyView {
                name: leaf(&near.copy.path),
                path: near.copy.path.clone(),
                size: near.copy.size,
                mtime: near.copy.mtime,
                keep: false,
                alike: Some(near.alike),
                thumb: picture(&near.copy.path),
            }));
            Row {
                id: cluster.id.clone(),
                kind: match cluster.sort {
                    similar::Sort::Picture => Kind::Pictures,
                    similar::Sort::Moving => Kind::Video,
                    similar::Sort::Sound => Kind::Sound,
                },
                claim: match cluster.band {
                    Closeness::Same => Claim::Same,
                    Closeness::Similar => Claim::Similar,
                },
                folder: false,
                reclaimable: cluster.reclaimable(),
                sure: true,
                files: 0,
                copies,
            }
        })
        .collect()
}

fn as_copy(copy: &dupes::Copy, keep: bool, alike: Option<u8>) -> CopyView {
    CopyView {
        name: leaf(&copy.path),
        path: copy.path.clone(),
        size: copy.size,
        mtime: copy.mtime,
        keep,
        alike,
        thumb: None,
    }
}

fn leaf(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(path, |(_, name)| name)
        .to_string()
}

/// Confirm, plan and carry out what the person ticked.
///
/// `paths` are the copies to deal with, one by one, because a window where
/// every copy has its own checkbox can ask for two of four and no description
/// in terms of whole groups can say that.
///
/// The passes run again rather than the window sending back what it was shown:
/// the digests and the fingerprints are remembered, so a second pass over an
/// untouched folder reads nothing, and acting on what the folder is **now** is
/// the only honest thing to do.
///
/// # Errors
/// A sentence, from opening, reading, planning or applying. Notably
/// [`dupes::Trouble::WouldEmpty`] when the ticks would leave a group with no
/// copy at all, which is refused here rather than prevented on screen.
pub fn clear(
    target: &str,
    paths: &[String],
    also_similar: bool,
    extras: Extras,
    journal: &Journal,
    scan_state: &Scan,
) -> Result<ClearedView, String> {
    let place = place(target, journal)?;
    // Checked here rather than trusted from the window: a command that can be
    // called any other way must not try to trash a path it cannot reach.
    if matches!(extras, Extras::Trash) && place.networked {
        return Err("the trash is this machine's; set aside instead".to_string());
    }

    let snapshot = look(place.backend.as_ref())?;
    let stop = scan_state.begin();
    let wanted: std::collections::BTreeSet<String> = paths.iter().cloned().collect();
    let mut digest = Cached::new(place.backend.as_ref(), journal, &place.root)
        .stopping_when(Box::new(move || stop.load(Ordering::Relaxed)));
    let wants = Wants {
        sampled: place.networked,
        ..Wants::default()
    };
    let found = dupes::find(&snapshot, &mut digest, &wants).map_err(describe)?;

    // Samples are enough to show a group and never enough to move a file, so
    // anything ticked that was matched on samples is read in full first, and
    // a copy that turns out to differ is dropped rather than moved.
    let mut dropped = 0;
    let mut settled = dupes::Found::default();
    for group in &found.groups {
        if group.sure || !group.copies_touched(&wanted) {
            settled.groups.push(group.clone());
            continue;
        }
        match dupes::confirm(group, &mut digest).map_err(describe)? {
            Some(confirmed) => settled.groups.push(confirmed),
            None => dropped += 1,
        }
    }
    for group in &found.folders {
        if group.sure || !group.copies_touched(&wanted) {
            settled.folders.push(group.clone());
            continue;
        }
        match dupes::confirm_folder(group, &snapshot, &mut digest).map_err(describe)? {
            Some(confirmed) => settled.folders.push(confirmed),
            None => dropped += 1,
        }
    }

    let resembling = if also_similar {
        let here = PathBuf::from(&place.root);
        let mut eye = Eye::new(&here, journal);
        let skip = settled
            .groups
            .iter()
            .flat_map(|group| group.extras.iter().map(|copy| copy.path.clone()))
            .collect();
        similar::resemble(&snapshot, &mut eye, &similar::Wants { skip }).map_err(describe)?
    } else {
        similar::Resembling::default()
    };

    let mut bundles = settled.bundles();
    bundles.extend(resembling.bundles());
    let dealings = dupes::decide(&bundles, &wanted).map_err(describe)?;
    if dealings.is_empty() {
        return Ok(ClearedView {
            files: 0,
            bytes: 0,
            plan: 0,
            reversible: true,
            dropped,
            failed: Vec::new(),
        });
    }

    let files = files_in(&dealings, &snapshot);
    let bytes = bytes_in(&dealings, &snapshot);
    let name = label_for(&place.root);
    let plan = dupes::plan_dealings(&snapshot, &dealings, extras, &name, Mode::Observe);
    let applied = tungstate_execute::apply(
        &plan,
        &snapshot,
        place.backend.as_ref(),
        journal,
        &place.root,
    )
    .map_err(|error| error.to_string())?;

    Ok(ClearedView {
        files,
        bytes,
        plan: applied.plan.0,
        reversible: extras.reversible(),
        dropped,
        failed: applied
            .failed
            .iter()
            .map(|failure| format!("{}: {}", failure.path, failure.why))
            .collect(),
    })
}

/// Files a set of dealings covers. **Files, never operations**: a plan also
/// removes the directories it empties, and slice 7b shipped "moved 9 file(s)"
/// for five moved files by confusing the two.
fn files_in(dealings: &[dupes::Dealing], snapshot: &tungstate_core::Snapshot) -> usize {
    dealings
        .iter()
        .map(|dealing| {
            if dealing.folder {
                under(&dealing.path, snapshot).count()
            } else {
                1
            }
        })
        .sum()
}

fn bytes_in(dealings: &[dupes::Dealing], snapshot: &tungstate_core::Snapshot) -> u64 {
    dealings
        .iter()
        .map(|dealing| {
            if dealing.folder {
                under(&dealing.path, snapshot).map(|entry| entry.size).sum()
            } else {
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.relative_path() == dealing.path)
                    .map_or(0, |entry| entry.size)
            }
        })
        .sum()
}

fn under<'a>(
    path: &str,
    snapshot: &'a tungstate_core::Snapshot,
) -> impl Iterator<Item = &'a tungstate_core::attrs::Attributes> {
    let prefix = format!("{path}/");
    snapshot
        .entries
        .iter()
        .filter(move |entry| !entry.is_dir && entry.relative_path().starts_with(&prefix))
}

/// The folder's own name, for the plan's record.
fn label_for(root: &str) -> String {
    PathBuf::from(root).file_name().map_or_else(
        || root.to_string(),
        |name| name.to_string_lossy().to_string(),
    )
}

/// Trouble as a sentence the window can show.
fn describe(trouble: Trouble) -> String {
    match trouble {
        Trouble::Stopped => "stopped".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with two copies of one thing, one of something else.
    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("keep.bin"), b"the same bytes").expect("write");
        std::fs::create_dir_all(dir.path().join("copies")).expect("dir");
        std::fs::write(dir.path().join("copies/other-name.bin"), b"the same bytes").expect("write");
        std::fs::write(dir.path().join("alone.bin"), b"nothing like it").expect("write");
        dir
    }

    /// A picture, and the same picture at a quarter of the size.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn two_sizes_of_one_picture(dir: &Path) {
        use std::f32::consts::TAU;
        let draw = |width: u32, height: u32| {
            image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| {
                let across = f64::from(x) / f64::from(width);
                let down = f64::from(y) / f64::from(height);
                let value = (across * f64::from(TAU) * 3.0).sin() * (down * f64::from(TAU)).cos();
                let level = ((value + 1.0) * 127.5).clamp(0.0, 255.0) as u8;
                image::Rgb([level, 90, 255 - level])
            }))
        };
        draw(800, 600).save(dir.join("IMG_4471.jpg")).expect("save");
        draw(200, 150)
            .save(dir.join("for the web.jpg"))
            .expect("save");
    }

    fn scanned(root: &str, journal: &Journal, scanning: &Scan, similar: bool) -> FoundView {
        scan(root, similar, journal, scanning, None, &mut |_| {}).expect("the scan runs")
    }

    #[test]
    fn a_scan_finds_the_copy_and_says_what_it_would_free() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let mut seen = 0;

        let found = scan(
            &dir.path().to_string_lossy(),
            false,
            &journal,
            &scanning,
            None,
            &mut |_progress| seen += 1,
        )
        .expect("the scan runs");

        assert_eq!(found.rows.len(), 1);
        assert_eq!(found.rows[0].claim, Claim::Identical);
        assert_eq!(found.rows[0].copies.len(), 2);
        assert!(found.rows[0].copies[0].keep, "the kept copy comes first");
        assert!(!found.rows[0].copies[1].keep);
        assert_eq!(found.extra_files, 1, "one extra copy, not one operation");
        assert_eq!(found.reclaimable, "the same bytes".len() as u64);
        assert_eq!(found.unsure, 0, "a local disk is read in full");
        assert!(!found.networked);
        assert!(found.can_trash, "the desktop's trash is available here");
        assert!(seen > 0, "progress was reported as it went");
    }

    #[test]
    fn clearing_deals_with_the_copy_ticked_and_leaves_the_other() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();
        let found = scanned(&root, &journal, &scanning, false);
        let extra = found.rows[0]
            .copies
            .iter()
            .find(|copy| !copy.keep)
            .expect("an extra copy")
            .path
            .clone();

        let cleared = clear(
            &root,
            &[extra],
            false,
            Extras::SetAside,
            &journal,
            &scanning,
        )
        .expect("it clears");

        assert_eq!(cleared.files, 1);
        assert!(cleared.reversible);
        assert!(dir.path().join("keep.bin").exists(), "one copy stays");
        assert!(!dir.path().join("copies/other-name.bin").exists());
    }

    #[test]
    fn the_copy_kept_is_whichever_one_nobody_ticked() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();

        // Tick the copy the pass would have kept, and leave the other.
        clear(
            &root,
            &["keep.bin".to_string()],
            false,
            Extras::SetAside,
            &journal,
            &scanning,
        )
        .expect("it clears");

        assert!(!dir.path().join("keep.bin").exists());
        assert!(dir.path().join("copies/other-name.bin").exists());
    }

    #[test]
    fn ticking_every_copy_is_refused_rather_than_obeyed() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();

        let refused = clear(
            &root,
            &["keep.bin".to_string(), "copies/other-name.bin".to_string()],
            false,
            Extras::SetAside,
            &journal,
            &scanning,
        );

        assert!(refused.is_err(), "{refused:?}");
        assert!(dir.path().join("keep.bin").exists(), "nothing moved");
        assert!(dir.path().join("copies/other-name.bin").exists());
    }

    #[test]
    fn a_group_nobody_ticked_is_left_alone() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();

        let cleared =
            clear(&root, &[], false, Extras::SetAside, &journal, &scanning).expect("it runs");

        assert_eq!(cleared.files, 0);
        assert!(dir.path().join("copies/other-name.bin").exists());
    }

    #[test]
    fn a_re_export_shows_up_as_its_own_kind_of_claim() {
        let dir = tempfile::tempdir().expect("temp dir");
        two_sizes_of_one_picture(dir.path());
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();

        let found = scanned(&dir.path().to_string_lossy(), &journal, &scanning, true);

        assert!(found.looked_alike);
        let row = found
            .rows
            .iter()
            .find(|row| row.claim == Claim::Same)
            .expect("a resemblance");
        assert_eq!(row.kind, Kind::Pictures);
        assert_eq!(row.copies.len(), 2);
        assert_eq!(row.copies[0].name, "IMG_4471.jpg", "the bigger one leads");
        assert!(row.copies[1].alike.is_some_and(|score| score >= 90));
    }

    #[test]
    fn a_resemblance_can_be_cleared_like_anything_else() {
        let dir = tempfile::tempdir().expect("temp dir");
        two_sizes_of_one_picture(dir.path());
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();

        let cleared = clear(
            &root,
            &["for the web.jpg".to_string()],
            true,
            Extras::SetAside,
            &journal,
            &scanning,
        )
        .expect("it clears");

        assert_eq!(cleared.files, 1);
        assert!(dir.path().join("IMG_4471.jpg").exists());
        assert!(!dir.path().join("for the web.jpg").exists());
    }

    #[test]
    fn a_stopped_scan_says_stopped_and_changes_nothing() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();

        // From inside the callback, which is the only moment a person can
        // press it: starting a scan clears the flag, so setting it first
        // would be a test of nothing.
        let stopped = scan(
            &dir.path().to_string_lossy(),
            false,
            &journal,
            &scanning,
            None,
            &mut |_progress| scanning.stop(),
        );

        assert_eq!(stopped.err(), Some("stopped".to_string()));
        assert!(dir.path().join("copies/other-name.bin").exists());
    }

    #[test]
    fn a_kind_is_worked_out_from_the_name() {
        assert_eq!(kind_of("holiday/IMG_1.jpg", false), Kind::Pictures);
        assert_eq!(kind_of("clip.MOV", false), Kind::Video);
        assert_eq!(kind_of("song.flac", false), Kind::Sound);
        assert_eq!(kind_of("invoice.pdf", false), Kind::Documents);
        assert_eq!(kind_of("papers.zip", false), Kind::Archives);
        assert_eq!(kind_of("Some App.app", true), Kind::Applications);
        assert_eq!(kind_of("holiday", true), Kind::Folders);
        assert_eq!(kind_of("notes", false), Kind::Other);
    }
}
