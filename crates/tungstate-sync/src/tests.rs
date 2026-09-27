//! Whole runs against real folders, with the real engine.
//!
//! Each test makes its own temporary folders and an in-memory journal, so they
//! are safe to run in parallel.

use std::path::Path;
use std::time::Duration;

use tungstate_journal::{
    ConflictAction, FirstCheck, Journal, NewMember, NewSync, OnRemove, SyncDirection, VerifyLevel,
};
use tungstate_secret::MemoryStore;

use super::*;

fn folders(n: usize) -> Vec<tempfile::TempDir> {
    (0..n)
        .map(|_| tempfile::tempdir().expect("temp dir"))
        .collect()
}

fn make(
    journal: &Journal,
    dirs: &[tempfile::TempDir],
    direction: SyncDirection,
    anchor: Option<&str>,
) {
    let members: Vec<NewMember> = dirs
        .iter()
        .enumerate()
        .map(|(i, dir)| NewMember {
            name: format!("m{i}"),
            connection: None,
            path: dir.path().to_string_lossy().to_string(),
        })
        .collect();
    journal
        .create_sync(
            &NewSync {
                name: "capcut".into(),
                direction,
                exact: false,
                anchor: anchor.map(str::to_string),
                on_conflict: ConflictAction::Quarantine,
                on_remove: OnRemove::SetAside,
                verify: VerifyLevel::Hash,
                cooldown: Duration::ZERO,
                first_check: FirstCheck::Full,
            },
            &members,
        )
        .expect("sync");
}

fn sync_once(journal: &Journal) -> (Decided, Ran) {
    let secrets = MemoryStore::new();
    let opened = open(journal, "capcut", &secrets).expect("open");
    let decided = decide(&opened, journal).expect("decide");
    let ran = run(
        &opened,
        &decided,
        journal,
        &mut tungstate_transfer::SilentProgress,
        None,
    )
    .expect("run");
    (decided, ran)
}

fn write(dir: &tempfile::TempDir, path: &str, bytes: &[u8]) {
    let full = dir.path().join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, bytes).unwrap();
}

fn read(dir: &tempfile::TempDir, path: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.path().join(path)).ok()
}

#[test]
fn three_folders_end_up_the_same_and_a_second_run_moves_nothing() {
    let dirs = folders(3);
    write(&dirs[0], "a.mp4", b"from the laptop");
    write(&dirs[1], "exports/b.mp4", b"from the nas");
    let journal = Journal::open_in_memory().unwrap();
    make(&journal, &dirs, SyncDirection::All, None);

    let (_, first) = sync_once(&journal);
    assert!(first.plan.is_some());
    for dir in &dirs {
        assert_eq!(read(dir, "a.mp4").as_deref(), Some(&b"from the laptop"[..]));
        assert_eq!(
            read(dir, "exports/b.mp4").as_deref(),
            Some(&b"from the nas"[..])
        );
    }

    let (decided, second) = sync_once(&journal);
    assert!(decided.plan.is_empty(), "{:#?}", decided.plan);
    assert_eq!(decided.read, 0, "a quiet run reads nothing");
    assert!(second.plan.is_none(), "nothing to do is not journalled");
}

#[test]
fn an_edit_moves_on_its_own_and_the_old_version_is_set_aside() {
    let dirs = folders(2);
    write(&dirs[0], "a.mp4", b"version one");
    write(&dirs[0], "b.mp4", b"untouched");
    let journal = Journal::open_in_memory().unwrap();
    make(&journal, &dirs, SyncDirection::All, None);
    sync_once(&journal);

    // A different length, and a later time, so the edit is seen whatever the
    // filesystem's time resolution is.
    std::thread::sleep(Duration::from_millis(20));
    write(&dirs[1], "a.mp4", b"version two, longer");
    let (decided, _) = sync_once(&journal);

    assert_eq!(decided.plan.legs.len(), 1, "{:#?}", decided.plan.legs);
    assert_eq!(decided.plan.legs[0].paths, ["a.mp4"]);
    assert_eq!(
        read(&dirs[0], "a.mp4").as_deref(),
        Some(&b"version two, longer"[..])
    );
    let aside = walk(&dirs[0].path().join(".tungstate-quarantine"));
    assert!(
        aside.iter().any(|bytes| bytes == b"version one"),
        "the replaced version is kept"
    );
}

#[test]
fn a_copy_keeps_the_time_it_was_written() {
    let dirs = folders(2);
    write(&dirs[0], "a.mp4", b"bytes");
    let then = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(dirs[0].path().join("a.mp4"))
        .unwrap()
        .set_modified(then)
        .unwrap();
    let journal = Journal::open_in_memory().unwrap();
    make(&journal, &dirs, SyncDirection::All, None);

    let (_, ran) = sync_once(&journal);

    assert_eq!(ran.legs[0].times_kept, 1);
    let copied = std::fs::metadata(dirs[1].path().join("a.mp4")).unwrap();
    assert_eq!(copied.modified().unwrap(), then);
}

#[test]
fn push_never_takes_from_a_member_that_is_not_the_anchor() {
    let dirs = folders(2);
    write(&dirs[0], "mine.mp4", b"laptop");
    write(&dirs[1], "theirs.mp4", b"nas");
    let journal = Journal::open_in_memory().unwrap();
    make(&journal, &dirs, SyncDirection::Push, Some("m0"));

    sync_once(&journal);

    assert!(read(&dirs[1], "mine.mp4").is_some());
    assert!(
        read(&dirs[0], "theirs.mp4").is_none(),
        "the laptop only sends"
    );
}

#[test]
fn a_member_added_later_is_filled_on_the_next_run() {
    let dirs = folders(3);
    write(&dirs[0], "a.mp4", b"bytes");
    let journal = Journal::open_in_memory().unwrap();
    make(&journal, &dirs[..2], SyncDirection::All, None);
    sync_once(&journal);

    let sync = journal.sync_by_name("capcut").unwrap();
    journal
        .add_member(
            sync.id,
            &NewMember {
                name: "spare".into(),
                connection: None,
                path: dirs[2].path().to_string_lossy().to_string(),
            },
        )
        .unwrap();
    sync_once(&journal);

    assert_eq!(read(&dirs[2], "a.mp4").as_deref(), Some(&b"bytes"[..]));
}

fn make_with(
    journal: &Journal,
    dirs: &[tempfile::TempDir],
    exact: bool,
    on_conflict: ConflictAction,
    on_remove: OnRemove,
) {
    let members: Vec<NewMember> = dirs
        .iter()
        .enumerate()
        .map(|(i, dir)| NewMember {
            name: format!("m{i}"),
            connection: None,
            path: dir.path().to_string_lossy().to_string(),
        })
        .collect();
    journal
        .create_sync(
            &NewSync {
                name: "capcut".into(),
                direction: SyncDirection::All,
                exact,
                anchor: None,
                on_conflict,
                on_remove,
                verify: VerifyLevel::Hash,
                cooldown: Duration::ZERO,
                first_check: FirstCheck::Full,
            },
            &members,
        )
        .expect("sync");
}

fn exact_pair(journal: &Journal, on_remove: OnRemove) -> Vec<tempfile::TempDir> {
    let dirs = folders(2);
    write(&dirs[0], "project/a.mp4", b"a project");
    write(&dirs[0], "keep.mp4", b"stays");
    make_with(journal, &dirs, true, ConflictAction::Quarantine, on_remove);
    sync_once(journal);
    dirs
}

fn undo_last(journal: &Journal) -> Undone {
    let opened = open(journal, "capcut", &MemoryStore::new()).expect("open");
    let run = standing(journal, &opened.sync, 1).expect("runs")[0].id;
    undo(&opened, journal, run).expect("undo")
}

#[test]
fn an_exact_deletion_is_set_aside_everywhere_and_undo_puts_it_all_back() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = exact_pair(&journal, OnRemove::SetAside);
    let before = journal
        .baseline_for(journal.sync_by_name("capcut").unwrap().id)
        .unwrap();

    std::fs::remove_dir_all(dirs[0].path().join("project")).unwrap();
    let (_, ran) = sync_once(&journal);

    assert_eq!(ran.taken_off, 1);
    assert!(read(&dirs[1], "project/a.mp4").is_none());
    assert!(
        !dirs[1].path().join("project").exists(),
        "the folder its last file left goes too"
    );
    assert_eq!(
        read(&dirs[1], ".tungstate-quarantine/project/a.mp4").as_deref(),
        Some(&b"a project"[..])
    );
    let ops = journal.ops_for_plan(ran.plan.unwrap()).unwrap();
    assert!(
        ops.iter()
            .all(|op| op.kind == tungstate_journal::OpKind::Rename)
    );

    let undone = undo_last(&journal);

    assert_eq!(undone.put_back, 1);
    assert_eq!(
        read(&dirs[1], "project/a.mp4").as_deref(),
        Some(&b"a project"[..])
    );
    assert!(
        !dirs[1]
            .path()
            .join(".tungstate-quarantine/project")
            .exists(),
        "and the set-aside folder it came back from"
    );
    assert_eq!(
        undone.revived,
        [("m0".to_string(), "project/a.mp4".to_string())]
    );
    let sync = journal.sync_by_name("capcut").unwrap();
    let after: Vec<_> = journal
        .baseline_for(sync.id)
        .unwrap()
        .into_iter()
        .filter(|r| r.path != "project/a.mp4" || r.member != sync.members[0].id)
        .collect();
    let expected: Vec<_> = before
        .into_iter()
        .filter(|r| r.path != "project/a.mp4" || r.member != sync.members[0].id)
        .collect();
    assert_eq!(after, expected, "the baseline is as it was before the run");

    sync_once(&journal);
    assert_eq!(
        read(&dirs[0], "project/a.mp4").as_deref(),
        Some(&b"a project"[..]),
        "and the deletion that caused it is undone too"
    );
    let (decided, _) = sync_once(&journal);
    assert!(decided.plan.is_empty(), "{:#?}", decided.plan);
}

#[test]
fn a_run_that_deletes_outright_cannot_be_put_back() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = exact_pair(&journal, OnRemove::Delete);

    std::fs::remove_file(dirs[0].path().join("keep.mp4")).unwrap();
    let (_, ran) = sync_once(&journal);
    assert!(read(&dirs[1], "keep.mp4").is_none());
    assert!(read(&dirs[1], ".tungstate-quarantine/keep.mp4").is_none());

    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let refused = undo(&opened, &journal, ran.plan.unwrap());

    assert!(
        matches!(&refused, Err(SyncError::Refused(why)) if why.contains("deleted")),
        "{refused:?}"
    );
}

#[test]
fn runs_are_put_back_newest_first() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = exact_pair(&journal, OnRemove::SetAside);
    std::fs::remove_file(dirs[0].path().join("keep.mp4")).unwrap();
    let (_, older) = sync_once(&journal);
    write(&dirs[1], "new.mp4", b"later");
    sync_once(&journal);

    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let refused = undo(&opened, &journal, older.plan.unwrap());

    assert!(
        matches!(&refused, Err(SyncError::Refused(why)) if why.contains("first")),
        "{refused:?}"
    );
}

#[test]
fn a_copy_changed_since_it_was_delivered_is_not_taken_off() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);
    write(&dirs[0], "a.mp4", b"delivered");
    make(&journal, &dirs, SyncDirection::All, None);
    sync_once(&journal);
    write(&dirs[1], "a.mp4", b"edited after");

    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let run = standing(&journal, &opened.sync, 1).unwrap()[0].id;
    let refused = undo(&opened, &journal, run);

    assert!(matches!(refused, Err(SyncError::Refused(_))), "{refused:?}");
    assert_eq!(
        read(&dirs[1], "a.mp4").as_deref(),
        Some(&b"edited after"[..])
    );
}

#[test]
fn a_conflict_is_parked_once_and_asked_about_again() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);
    write(&dirs[0], "clip.mp4", b"first");
    make(&journal, &dirs, SyncDirection::All, None);
    sync_once(&journal);
    std::thread::sleep(Duration::from_millis(20));
    write(&dirs[0], "clip.mp4", b"the laptop's edit");
    write(&dirs[1], "clip.mp4", b"the nas's own edit");

    let (first, _) = sync_once(&journal);
    let (second, _) = sync_once(&journal);

    assert!(
        first
            .plan
            .left_alone
            .iter()
            .any(|l| matches!(l.why, core::Why::Conflict { .. }))
    );
    assert_eq!(
        read(&dirs[0], ".tungstate-quarantine/clip (m1).mp4").as_deref(),
        Some(&b"the nas's own edit"[..])
    );
    assert_eq!(
        read(&dirs[1], ".tungstate-quarantine/clip (m0).mp4").as_deref(),
        Some(&b"the laptop's edit"[..])
    );
    assert_eq!(
        read(&dirs[0], "clip.mp4").as_deref(),
        Some(&b"the laptop's edit"[..])
    );
    assert!(second.plan.ops.is_empty(), "{:#?}", second.plan.ops);
    assert!(
        second
            .plan
            .left_alone
            .iter()
            .any(|l| matches!(l.why, core::Why::Conflict { .. })),
        "still asked about"
    );
    assert_eq!(walk(&dirs[0].path().join(".tungstate-quarantine")).len(), 1);
}

#[test]
fn a_forgotten_file_stays_gone_and_undo_brings_it_back() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);
    write(&dirs[0], "old.mp4", b"old");
    write(&dirs[0], "new.mp4", b"new");
    make(&journal, &dirs, SyncDirection::All, None);
    sync_once(&journal);

    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let asked = core::Asked {
        forget: ["old.mp4".to_string()].into(),
        only: true,
        ..core::Asked::default()
    };
    let decided = decide_with(&opened, &journal, &asked).unwrap();
    run(
        &opened,
        &decided,
        &journal,
        &mut tungstate_transfer::SilentProgress,
        None,
    )
    .unwrap();

    assert!(read(&dirs[0], "old.mp4").is_none());
    assert!(read(&dirs[1], "old.mp4").is_none());
    let (again, _) = sync_once(&journal);
    assert!(again.plan.is_empty(), "not copied back: {:#?}", again.plan);

    undo_last(&journal);
    assert!(read(&dirs[0], "old.mp4").is_some());
    assert!(read(&dirs[1], "old.mp4").is_some());
}

#[test]
fn a_tidy_on_one_member_is_carried_as_a_rename() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);
    write(&dirs[0], "clip.mp4", b"an export");
    make_with(
        &journal,
        &dirs,
        true,
        ConflictAction::Quarantine,
        OnRemove::SetAside,
    );
    sync_once(&journal);

    std::fs::create_dir_all(dirs[0].path().join("2026")).unwrap();
    std::fs::rename(
        dirs[0].path().join("clip.mp4"),
        dirs[0].path().join("2026/clip.mp4"),
    )
    .unwrap();
    let (decided, ran) = sync_once(&journal);

    assert!(decided.plan.legs.is_empty(), "{:#?}", decided.plan.legs);
    assert_eq!(ran.renamed, 1);
    assert_eq!(
        read(&dirs[1], "2026/clip.mp4").as_deref(),
        Some(&b"an export"[..])
    );
    assert!(read(&dirs[1], "clip.mp4").is_none());
    let (again, _) = sync_once(&journal);
    assert!(again.plan.is_empty(), "{:#?}", again.plan);
}

#[test]
fn a_file_written_after_deciding_is_not_set_aside() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = exact_pair(&journal, OnRemove::SetAside);
    std::fs::remove_file(dirs[0].path().join("keep.mp4")).unwrap();
    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let decided = decide(&opened, &journal).unwrap();

    std::thread::sleep(Duration::from_millis(20));
    write(&dirs[1], "keep.mp4", b"someone is still working on this");
    let ran = run(
        &opened,
        &decided,
        &journal,
        &mut tungstate_transfer::SilentProgress,
        None,
    )
    .unwrap();

    assert_eq!(ran.taken_off, 0);
    assert_eq!(ran.missed.len(), 1);
    assert_eq!(
        read(&dirs[1], "keep.mp4").as_deref(),
        Some(&b"someone is still working on this"[..])
    );
}

#[test]
fn an_empty_member_is_refused_by_name() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = exact_pair(&journal, OnRemove::SetAside);
    std::fs::remove_dir_all(dirs[1].path().join("project")).unwrap();
    std::fs::remove_file(dirs[1].path().join("keep.mp4")).unwrap();

    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let decided = decide(&opened, &journal).unwrap();

    assert!(refusals(&opened, &decided).contains(&Refusal::Hollow {
        member: "m1".into(),
        held: 2,
    }));
}

/// A member that cannot rename, the way an FTP server cannot.
struct NoRename(Box<dyn tungstate_backend::Backend>);

impl tungstate_backend::Backend for NoRename {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        tungstate_backend::Capabilities {
            atomic_rename: false,
            ..self.0.capabilities()
        }
    }
    fn root_token(&self) -> tungstate_backend::Result<tungstate_backend::RootToken> {
        self.0.root_token()
    }
    fn stat(&self, path: &Path) -> tungstate_backend::Result<tungstate_backend::Meta> {
        self.0.stat(path)
    }
    fn read_dir(&self, path: &Path) -> tungstate_backend::Result<Vec<tungstate_backend::Entry>> {
        self.0.read_dir(path)
    }
    fn open_read(&self, path: &Path) -> tungstate_backend::Result<Box<dyn std::io::Read + Send>> {
        self.0.open_read(path)
    }
    fn create_write(
        &self,
        path: &Path,
    ) -> tungstate_backend::Result<Box<dyn tungstate_backend::WriteFinish>> {
        self.0.create_write(path)
    }
    fn rename(&self, from: &Path, _to: &Path) -> tungstate_backend::Result<()> {
        Err(tungstate_backend::BackendError::Io {
            path: from.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::Unsupported, "no rename here"),
        })
    }
    fn remove_file(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.0.remove_dir(path)
    }
    fn create_dir_all(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.0.create_dir_all(path)
    }
}

#[test]
fn a_member_that_cannot_rename_keeps_its_copy_and_the_others_are_filled() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(3);
    write(&dirs[0], "a.mp4", b"one");
    make(&journal, &dirs, SyncDirection::All, None);
    sync_once(&journal);
    std::thread::sleep(Duration::from_millis(20));
    write(&dirs[0], "a.mp4", b"one, edited");

    let mut opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let inner = std::mem::replace(
        &mut opened.places[2].backend,
        Box::new(tungstate_backend::local::LocalBackend::new(
            dirs[2].path().to_path_buf(),
        )),
    );
    opened.places[2].backend = Box::new(NoRename(inner));
    let decided = decide(&opened, &journal).unwrap();
    run(
        &opened,
        &decided,
        &journal,
        &mut tungstate_transfer::SilentProgress,
        None,
    )
    .unwrap();

    assert!(
        decided
            .plan
            .left_alone
            .iter()
            .any(|l| matches!(l.why, core::Why::NeedsRename { .. }))
    );
    assert_eq!(
        read(&dirs[1], "a.mp4").as_deref(),
        Some(&b"one, edited"[..])
    );
    assert_eq!(read(&dirs[2], "a.mp4").as_deref(), Some(&b"one"[..]));
    let again = decide(&opened, &journal).unwrap();
    assert!(again.plan.is_empty(), "{:#?}", again.plan);
}

/// The contents of every file under a directory.
fn walk(at: &Path) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(at) else {
        return found;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            found.extend(walk(&entry.path()));
        } else if let Ok(bytes) = std::fs::read(entry.path()) {
            found.push(bytes);
        }
    }
    found
}

#[test]
fn a_leg_checks_copies_the_way_the_sync_says_now() {
    // A leg's link is made once and reused, so the level it was made with
    // must not outlive `sync set --verify`.
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);
    write(&dirs[0], "a.mp4", b"bytes");
    make(&journal, &dirs, SyncDirection::All, None);
    sync_once(&journal);
    let sync = journal.sync_by_name("capcut").unwrap();
    journal
        .update_sync(
            sync.id,
            &tungstate_journal::SyncSettings {
                exact: false,
                on_conflict: sync.on_conflict,
                on_remove: sync.on_remove,
                verify: VerifyLevel::Readback,
                cooldown: sync.cooldown,
                first_check: sync.first_check,
            },
        )
        .unwrap();

    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let link = leg_link(&opened, &journal, &opened.places[0], &opened.places[1]).unwrap();

    assert_eq!(link.verify, VerifyLevel::Readback);
}

#[test]
fn a_stopped_run_remembers_nothing_it_did_not_finish() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);
    write(&dirs[0], "a.mp4", b"one");
    write(&dirs[0], "b.mp4", b"two");
    make(&journal, &dirs, SyncDirection::All, None);
    let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
    let decided = decide(&opened, &journal).unwrap();
    let stop = std::sync::Arc::new(tungstate_transfer::Stop::new());
    stop.after_this_file();

    let ran = run_until(
        &opened,
        &decided,
        &journal,
        &mut tungstate_transfer::SilentProgress,
        None,
        Some(stop),
    )
    .unwrap();

    assert!(ran.stopped);
    assert!(read(&dirs[1], "a.mp4").is_none());
    let (again, _) = sync_once(&journal);
    assert_eq!(
        again.plan.legs[0].paths.len(),
        2,
        "all of it is decided again"
    );
    assert_eq!(read(&dirs[1], "b.mp4").as_deref(), Some(&b"two"[..]));
}

fn setup_of(dirs: &[tempfile::TempDir], direction: SyncDirection) -> setup::Setup {
    setup::Setup {
        name: "capcut".into(),
        ends: dirs
            .iter()
            .map(|d| d.path().to_string_lossy().to_string())
            .collect(),
        names: vec!["laptop".into(), "nas".into()],
        direction,
        anchor: None,
        settings: setup::settings(false, "quarantine", "set-aside", "hash", 0, "full").unwrap(),
    }
}

#[test]
fn a_sync_is_made_with_its_first_member_as_anchor_and_refused_twice() {
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);

    let made = setup::create(&journal, &setup_of(&dirs, SyncDirection::Push)).unwrap();

    assert_eq!(made.anchor, Some(made.members[0].id));
    assert_eq!(
        setup::create(&journal, &setup_of(&dirs, SyncDirection::Push)).unwrap_err(),
        setup::Refused::Taken("capcut".into())
    );
}

#[test]
fn what_cannot_be_is_refused_as_data() {
    use setup::Refused;
    let journal = Journal::open_in_memory().unwrap();
    let dirs = folders(2);

    assert_eq!(
        setup::settings(false, "quarantine", "delete", "hash", 0, "full").unwrap_err(),
        Refused::DeleteNeedsExact
    );
    assert_eq!(
        setup::settings(true, "replace", "delete", "hash", 0, "full").unwrap_err(),
        Refused::ReplaceHasNoMeaning
    );
    let mut all = setup_of(&dirs, SyncDirection::All);
    all.anchor = Some("laptop".into());
    assert_eq!(
        setup::create(&journal, &all).unwrap_err(),
        Refused::AllHasNoAnchor
    );
    let mut inside = setup_of(&dirs, SyncDirection::All);
    std::fs::create_dir_all(dirs[0].path().join("inside")).unwrap();
    inside.ends[1] = dirs[0].path().join("inside").to_string_lossy().to_string();
    assert_eq!(
        setup::create(&journal, &inside).unwrap_err(),
        Refused::Overlap("laptop".into(), "nas".into())
    );
    let mut one = setup_of(&dirs[..1], SyncDirection::All);
    one.names.truncate(1);
    assert_eq!(setup::create(&journal, &one).unwrap_err(), Refused::TooFew);
    assert!(journal.syncs().unwrap().is_empty(), "nothing half-made");
}

// --- following (slice 9e) ---------------------------------------------------

mod following {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::follow::{self, Followed, Handle, Heard, How, Looked, Outcome, Pace, Poll, State};

    fn quick() -> Pace {
        Pace {
            quick: Duration::from_millis(200),
            slowest: Duration::from_millis(800),
            retries: vec![Duration::from_millis(100), Duration::from_millis(300)],
            sweep: Duration::from_secs(3600),
            echo: Duration::from_secs(5),
            busy: Duration::from_millis(100),
            debounce: Duration::from_millis(100),
            tick: Duration::from_millis(20),
        }
    }

    #[test]
    fn a_quiet_member_is_listed_less_often_and_a_change_brings_it_back_to_quick() {
        let pace = Pace::default();
        let now = Instant::now();
        let mut poll = Poll::new(now, &pace);

        assert_eq!(
            poll.looked(now, &pace, Ok("a".into())),
            Looked::Same,
            "a first look is a starting point"
        );
        let mut every = Vec::new();
        for _ in 0..6 {
            poll.looked(now, &pace, Ok("a".into()));
            every.push(poll.every().as_secs());
        }
        assert_eq!(
            every,
            [60, 120, 240, 480, 600, 600],
            "doubling up to ten minutes"
        );

        assert_eq!(poll.looked(now, &pace, Ok("b".into())), Looked::Changed);
        assert_eq!(poll.every(), Duration::from_secs(30));
    }

    #[test]
    fn an_unreachable_member_is_said_once_and_tried_at_one_two_five_then_ten_minutes() {
        let pace = Pace::default();
        let now = Instant::now();
        let mut poll = Poll::new(now, &pace);
        poll.looked(now, &pace, Ok("a".into()));

        let mut said = Vec::new();
        let mut waits = Vec::new();
        for _ in 0..6 {
            said.push(poll.looked(now, &pace, Err("no route to host".into())));
            waits.push((poll.next() - now).as_secs() / 60);
        }

        assert_eq!(said[0], Looked::Paused("no route to host".into()));
        assert!(
            said[1..].iter().all(|l| *l == Looked::StillPaused),
            "no error per poll"
        );
        assert_eq!(waits, [1, 2, 5, 10, 10, 10]);
        assert_eq!(
            poll.looked(now, &pace, Ok("a".into())),
            Looked::Back,
            "and it comes back on its own"
        );
    }

    #[test]
    fn what_a_run_writes_is_expected_back_folders_included() {
        let journal = Journal::open_in_memory().unwrap();
        let dirs = folders(2);
        write(&dirs[0], "exports/cut.mp4", b"bytes");
        make(&journal, &dirs, SyncDirection::All, None);
        let opened = open(&journal, "capcut", &MemoryStore::new()).unwrap();
        let decided = decide(&opened, &journal).unwrap();

        let wrote = follow::written(&opened, &decided.plan);

        let target = dirs[1].path().join("exports/cut.mp4");
        let resolved = tungstate_journal::resolve_for_lookup(&target);
        assert!(
            wrote.contains(&target) || wrote.contains(&resolved),
            "{wrote:?}"
        );
        assert!(
            wrote.iter().any(|p| p.ends_with("exports")),
            "the folder it lands in"
        );
        assert!(
            !wrote.iter().any(|p| p.starts_with(dirs[0].path())),
            "the source is only read"
        );
    }

    /// A loop in a thread: its handle, what it said, how many runs it asked
    /// for, and the thread to join.
    type Following = (
        Arc<Handle>,
        Arc<Mutex<Vec<Heard>>>,
        Arc<Mutex<usize>>,
        std::thread::JoinHandle<()>,
    );

    /// Follow a sync in a thread, running it for real, until stopped.
    fn following(journal: Arc<Journal>, dirs: &[tempfile::TempDir], pace: Pace) -> Following {
        let handle = Handle::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let runs = Arc::new(Mutex::new(0));
        let followed = Followed {
            sync: "capcut".into(),
            cooldown: Duration::from_millis(400),
            members: dirs
                .iter()
                .enumerate()
                .map(|(i, d)| follow::Member {
                    name: format!("m{i}"),
                    how: How::Here(d.path().to_path_buf()),
                })
                .collect(),
        };
        let (h, told, counted) = (Arc::clone(&handle), Arc::clone(&heard), Arc::clone(&runs));
        let thread = std::thread::spawn(move || {
            let mut tell = |e: &Heard| told.lock().unwrap().push(e.clone());
            let mut run = |name: &str| {
                *counted.lock().unwrap() += 1;
                let opened = open(&journal, name, &MemoryStore::new()).unwrap();
                let decided = decide(&opened, &journal).unwrap();
                let wrote = follow::written(&opened, &decided.plan);
                run(
                    &opened,
                    &decided,
                    &journal,
                    &mut tungstate_transfer::SilentProgress,
                    None,
                )
                .unwrap();
                Outcome::Done {
                    wrote,
                    unsettled: follow::unsettled(&decided),
                }
            };
            follow::follow(vec![followed], &pace, &h, &mut tell, &mut run).unwrap();
        });
        (handle, heard, runs, thread)
    }

    /// Wait for something, up to a generous limit: events arrive when the OS
    /// sends them, and a slow CI machine is not a failure.
    fn until(limit: Duration, what: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn a_saved_file_arrives_with_nothing_pressed_and_its_own_copy_starts_nothing() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let dirs = folders(2);
        make(&journal, &dirs, SyncDirection::All, None);
        let (handle, heard, runs, thread) = following(Arc::clone(&journal), &dirs, quick());
        assert!(
            until(Duration::from_secs(10), || *runs.lock().unwrap() >= 1),
            "it runs once at the start"
        );
        // Give the watcher a moment to be listening before the save.
        std::thread::sleep(Duration::from_millis(500));

        write(&dirs[0], "export.mp4", b"the finished cut");
        let arrived = until(Duration::from_secs(20), || {
            read(&dirs[1], "export.mp4").is_some()
        });
        let after = *runs.lock().unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let later = *runs.lock().unwrap();
        handle.stop();
        thread.join().unwrap();

        assert!(arrived, "the save reached the other folder");
        assert_eq!(later, after, "the copy it made did not start another run");
        assert!(heard.lock().unwrap().iter().any(|h| matches!(
            h,
            Heard::Member {
                state: State::Watching,
                ..
            }
        )));
    }

    #[test]
    fn a_file_still_being_written_at_the_start_is_carried_once_it_settles() {
        // Found by hand: nothing else will ever say when it stopped changing.
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let dirs = folders(2);
        make(&journal, &dirs, SyncDirection::All, None);
        let sync = journal.sync_by_name("capcut").unwrap();
        journal
            .update_sync(
                sync.id,
                &tungstate_journal::SyncSettings {
                    exact: false,
                    on_conflict: sync.on_conflict,
                    on_remove: sync.on_remove,
                    verify: sync.verify,
                    // Long enough that the opening run lands inside it even
                    // on a busy machine, which is the case under test.
                    cooldown: Duration::from_secs(5),
                    first_check: sync.first_check,
                },
            )
            .unwrap();
        write(&dirs[0], "just-saved.mp4", b"fresh");

        let (handle, _, runs, thread) = following(Arc::clone(&journal), &dirs, quick());
        let arrived = until(Duration::from_secs(20), || {
            read(&dirs[1], "just-saved.mp4").is_some()
        });
        handle.stop();
        thread.join().unwrap();

        // How many runs that took depends on how long the machine took to
        // start the first one, so only the arrival is asserted here; the
        // second run is `a_run_that_left_something_unsettled_is_run_again`.
        drop(runs);
        assert!(arrived, "carried once it had settled");
    }

    #[test]
    fn a_run_that_left_something_unsettled_is_run_again_with_no_event() {
        let followed = Followed {
            sync: "capcut".into(),
            cooldown: Duration::from_millis(200),
            members: Vec::new(),
        };
        let handle = Handle::new();
        let asked = Arc::new(Mutex::new(0usize));
        let (h, counted) = (Arc::clone(&handle), Arc::clone(&asked));
        let thread = std::thread::spawn(move || {
            let mut tell = |_: &Heard| {};
            let mut run = |_: &str| {
                let mut n = counted.lock().unwrap();
                *n += 1;
                Outcome::Done {
                    wrote: Vec::new(),
                    // Only the first run leaves something still being written.
                    unsettled: *n == 1,
                }
            };
            follow::follow(vec![followed], &quick(), &h, &mut tell, &mut run).unwrap();
        });

        let again = until(Duration::from_secs(5), || *asked.lock().unwrap() >= 2);
        std::thread::sleep(Duration::from_millis(600));
        let total = *asked.lock().unwrap();
        handle.stop();
        thread.join().unwrap();

        assert!(
            again,
            "run again once the cooldown passed, with nothing heard"
        );
        assert_eq!(total, 2, "and not again once it settled");
    }

    #[test]
    fn saving_again_inside_the_cooldown_puts_the_run_off_rather_than_running_twice() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let dirs = folders(2);
        make(&journal, &dirs, SyncDirection::All, None);
        let (handle, _, runs, thread) = following(Arc::clone(&journal), &dirs, quick());
        assert!(until(Duration::from_secs(10), || *runs.lock().unwrap() >= 1));
        std::thread::sleep(Duration::from_millis(500));
        let before = *runs.lock().unwrap();

        write(&dirs[0], "export.mp4", b"first");
        std::thread::sleep(Duration::from_millis(200));
        write(&dirs[0], "export.mp4", b"second, longer");
        assert!(until(Duration::from_secs(20), || read(
            &dirs[1],
            "export.mp4"
        )
        .as_deref()
            == Some(&b"second, longer"[..])));
        std::thread::sleep(Duration::from_secs(1));
        let ran = *runs.lock().unwrap() - before;
        handle.stop();
        thread.join().unwrap();

        assert_eq!(ran, 1, "one run for the two saves");
    }

    #[test]
    fn a_member_that_cannot_be_listed_is_paused_once_and_run_when_it_is_back() {
        let tries = Arc::new(Mutex::new(0usize));
        let counted = Arc::clone(&tries);
        let followed = Followed {
            sync: "nas-only".into(),
            cooldown: Duration::ZERO,
            members: vec![follow::Member {
                name: "nas".into(),
                how: How::Listed(Box::new(move || {
                    let mut n = counted.lock().unwrap();
                    *n += 1;
                    // Reachable, then down for three looks, then back.
                    if (2..=4).contains(&*n) {
                        Err("no route to host".into())
                    } else {
                        Ok("same".into())
                    }
                })),
            }],
        };
        let handle = Handle::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let runs = Arc::new(Mutex::new(0usize));
        let (h, told, counted_runs) = (Arc::clone(&handle), Arc::clone(&heard), Arc::clone(&runs));
        let thread = std::thread::spawn(move || {
            let mut tell = |e: &Heard| told.lock().unwrap().push(e.clone());
            let mut run = |_: &str| {
                *counted_runs.lock().unwrap() += 1;
                Outcome::Done {
                    wrote: Vec::new(),
                    unsettled: false,
                }
            };
            follow::follow(vec![followed], &quick(), &h, &mut tell, &mut run).unwrap();
        });

        assert!(until(Duration::from_secs(10), || *tries.lock().unwrap() >= 6));
        handle.stop();
        thread.join().unwrap();

        let heard = heard.lock().unwrap();
        let paused = heard
            .iter()
            .filter(|h| {
                matches!(
                    h,
                    Heard::Member {
                        state: State::Paused { .. },
                        ..
                    }
                )
            })
            .count();
        assert_eq!(paused, 1, "said once, not once per look: {heard:?}");
        assert!(
            *runs.lock().unwrap() >= 2,
            "run at the start, and again when it came back"
        );
    }

    #[test]
    fn a_held_sync_waits_for_a_person_and_runs_once_resumed() {
        let followed = Followed {
            sync: "capcut".into(),
            cooldown: Duration::ZERO,
            members: Vec::new(),
        };
        let handle = Handle::new();
        let asked = Arc::new(Mutex::new(0usize));
        let (h, counted) = (Arc::clone(&handle), Arc::clone(&asked));
        let mut pace = quick();
        pace.sweep = Duration::from_millis(100);
        let thread = std::thread::spawn(move || {
            let mut tell = |_: &Heard| {};
            let mut run = |_: &str| {
                let mut n = counted.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    Outcome::Held
                } else {
                    Outcome::Done {
                        wrote: Vec::new(),
                        unsettled: false,
                    }
                }
            };
            follow::follow(vec![followed], &pace, &h, &mut tell, &mut run).unwrap();
        });

        std::thread::sleep(Duration::from_millis(600));
        let while_held = *asked.lock().unwrap();
        handle.resume("capcut");
        assert!(until(Duration::from_secs(5), || *asked.lock().unwrap()
            > while_held));
        handle.stop();
        thread.join().unwrap();

        assert_eq!(while_held, 1, "not run again while held, even on the sweep");
    }
}
