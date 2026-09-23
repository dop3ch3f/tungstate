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

use serde::{Deserialize, Serialize};
use tungstate_backend::Backend;
use tungstate_core::dupes::{self, Extras, Found, Trouble, Wants};
use tungstate_core::policy::Mode;
use tungstate_execute::digest::{Cached, Watch};
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
}

/// What a scan found, as the window draws it.
#[derive(Debug, Clone, Serialize)]
pub struct FoundView {
    /// What was scanned, spelled the way it was asked for.
    pub root: String,
    /// Groups of identical files.
    pub groups: Vec<dupes::Group>,
    /// Folders copied whole.
    pub folders: Vec<dupes::FolderGroup>,
    /// Names that are the same file as another name.
    pub linked: Vec<dupes::Linked>,
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

/// Which copy the person wants kept, when it is not the one the engine chose.
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    /// The group, by its stable id.
    pub group: String,
    /// The copy to keep.
    pub keep: String,
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
    journal: &Journal,
    scan: &Scan,
    mut watcher: impl FnMut(&ScanProgress) + Send,
) -> Result<FoundView, String> {
    let place = place(target, journal)?;
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
            .watched_by(Box::new(move |watch: &Watch| {
                watcher(&ScanProgress {
                    looked: watch.looked,
                    read: watch.read,
                    recalled: watch.recalled,
                    bytes: watch.bytes,
                    path: watch.path.clone(),
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

    remember(journal, &place.root);
    Ok(FoundView {
        root: place.root,
        files,
        extra_files: found.extra_files(),
        reclaimable: found.reclaimable(),
        unsure: found.unsure(),
        networked: place.networked,
        // The desktop's trash is this machine's, so a connection is offered
        // set-aside and nothing else.
        can_trash: place
            .backend
            .on_this_machine(Path::new(""))
            .is_ok_and(|found| found.is_some()),
        groups: found.groups,
        folders: found.folders,
        linked: found.linked,
    })
}

/// Confirm, plan and carry out what the person picked.
///
/// `only` is the groups to act on, by id; `choices` are the copies to keep
/// where that is not the one the engine chose. The pass runs again rather than
/// the window sending back what it was shown: the digests are remembered, so
/// a second pass over an untouched folder reads nothing, and acting on what
/// the folder is *now* is the only honest thing to do.
///
/// # Errors
/// A sentence, from opening, reading, planning or applying.
pub fn clear(
    target: &str,
    only: &[String],
    choices: &[Choice],
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
    let mut digest = Cached::new(place.backend.as_ref(), journal, &place.root)
        .stopping_when(Box::new(move || stop.load(Ordering::Relaxed)));
    // Only the pins for groups being acted on: a pin for a group left alone
    // would change which copy another pass keeps, which nobody asked for.
    let wants = Wants {
        sampled: place.networked,
        pinned: choices
            .iter()
            .filter(|choice| only.contains(&choice.group))
            .map(|choice| choice.keep.clone())
            .collect(),
        ..Wants::default()
    };
    let found = dupes::find(&snapshot, &mut digest, &wants).map_err(describe)?;

    // Only what was asked for, and only after every byte has been compared.
    let mut dropped = 0;
    let mut chosen = Found::default();
    for group in found.groups.iter().filter(|group| only.contains(&group.id)) {
        match dupes::confirm(group, &mut digest).map_err(describe)? {
            Some(confirmed) => chosen.groups.push(confirmed),
            None => dropped += 1,
        }
    }
    for group in found
        .folders
        .iter()
        .filter(|group| only.contains(&group.id))
    {
        match dupes::confirm_folder(group, &snapshot, &mut digest).map_err(describe)? {
            Some(confirmed) => chosen.folders.push(confirmed),
            None => dropped += 1,
        }
    }
    if chosen.is_empty() {
        return Ok(ClearedView {
            files: 0,
            bytes: 0,
            plan: 0,
            reversible: true,
            dropped,
            failed: Vec::new(),
        });
    }

    let name = label_for(&place.root);
    let plan = dupes::plan(&snapshot, &chosen, extras, &name, Mode::Observe);
    let applied = tungstate_execute::apply(
        &plan,
        &snapshot,
        place.backend.as_ref(),
        journal,
        &place.root,
    )
    .map_err(|error| error.to_string())?;

    Ok(ClearedView {
        files: chosen.extra_files(),
        bytes: chosen.reclaimable(),
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

    #[test]
    fn a_scan_finds_the_copy_and_says_what_it_would_free() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let mut seen = 0;

        let found = scan(
            &dir.path().to_string_lossy(),
            &journal,
            &scanning,
            |_progress| seen += 1,
        )
        .expect("the scan runs");

        assert_eq!(found.groups.len(), 1);
        assert_eq!(found.extra_files, 1, "one extra copy, not one operation");
        assert_eq!(found.reclaimable, "the same bytes".len() as u64);
        assert_eq!(found.unsure, 0, "a local disk is read in full");
        assert!(!found.networked);
        assert!(found.can_trash, "the desktop's trash is available here");
        assert!(seen > 0, "progress was reported as it went");
    }

    #[test]
    fn clearing_sets_the_extra_copy_aside_and_keeps_one() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();
        let found = scan(&root, &journal, &scanning, |_| {}).expect("the scan runs");
        let group = found.groups[0].id.clone();

        let cleared =
            clear(&root, &[group], &[], Extras::SetAside, &journal, &scanning).expect("it clears");

        assert_eq!(cleared.files, 1);
        assert_eq!(cleared.dropped, 0);
        assert!(cleared.reversible);
        assert!(
            dir.path().join("keep.bin").exists(),
            "one copy stays exactly where it was"
        );
        assert!(
            !dir.path().join("copies/other-name.bin").exists(),
            "and the extra one is dealt with"
        );
        assert!(
            dir.path()
                .join(".tungstate-quarantine/copies/other-name.bin")
                .exists(),
            "set aside, not gone"
        );
    }

    #[test]
    fn the_copy_you_pick_is_the_one_that_stays() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();
        let found = scan(&root, &journal, &scanning, |_| {}).expect("the scan runs");
        let group = found.groups[0].id.clone();

        // The engine would have kept `keep.bin`: the plainest path.
        let cleared = clear(
            &root,
            std::slice::from_ref(&group),
            &[Choice {
                group: group.clone(),
                keep: "copies/other-name.bin".to_string(),
            }],
            Extras::SetAside,
            &journal,
            &scanning,
        )
        .expect("it clears");

        assert_eq!(cleared.files, 1);
        assert!(
            dir.path().join("copies/other-name.bin").exists(),
            "the copy that was picked stays"
        );
        assert!(
            !dir.path().join("keep.bin").exists(),
            "and the other one is the one dealt with"
        );
    }

    #[test]
    fn a_group_nobody_ticked_is_left_alone() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();
        let root = dir.path().to_string_lossy().to_string();
        scan(&root, &journal, &scanning, |_| {}).expect("the scan runs");

        let cleared = clear(&root, &[], &[], Extras::SetAside, &journal, &scanning)
            .expect("it does nothing, successfully");

        assert_eq!(cleared.files, 0);
        assert!(dir.path().join("keep.bin").exists());
        assert!(dir.path().join("copies/other-name.bin").exists());
    }

    #[test]
    fn a_scan_that_is_stopped_says_so_rather_than_answering() {
        let dir = folder();
        let journal = Journal::open_in_memory().expect("journal");
        let scanning = Scan::default();

        // Pressed while it runs, which is the only way it can be pressed: a
        // scan clears the flag as it starts, so a stop from last time cannot
        // kill this one.
        let stopped = scan(
            &dir.path().to_string_lossy(),
            &journal,
            &scanning,
            |_progress| scanning.stop(),
        )
        .expect_err("it stops");

        assert_eq!(stopped, "stopped");
        assert!(
            dir.path().join("copies/other-name.bin").exists(),
            "a stopped scan changes nothing, because a scan only ever reads"
        );
    }
}
