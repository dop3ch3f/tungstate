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
