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

#[test]
fn exact_is_refused_until_it_exists() {
    let dirs = folders(2);
    let journal = Journal::open_in_memory().unwrap();
    journal
        .create_sync(
            &NewSync {
                name: "capcut".into(),
                direction: SyncDirection::All,
                exact: true,
                anchor: None,
                on_conflict: ConflictAction::Quarantine,
                on_remove: OnRemove::SetAside,
                verify: VerifyLevel::Hash,
                cooldown: Duration::ZERO,
                first_check: FirstCheck::Full,
            },
            &[
                NewMember {
                    name: "a".into(),
                    connection: None,
                    path: dirs[0].path().to_string_lossy().to_string(),
                },
                NewMember {
                    name: "b".into(),
                    connection: None,
                    path: dirs[1].path().to_string_lossy().to_string(),
                },
            ],
        )
        .unwrap();

    let refused = open(&journal, "capcut", &MemoryStore::new());

    assert!(matches!(refused, Err(SyncError::NotYet(_))));
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
