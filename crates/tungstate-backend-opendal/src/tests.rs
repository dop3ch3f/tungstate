//! The adapter, exercised through `OpenDAL`'s own `fs` service.
//!
//! No server and no network, so these run identically on macOS, Windows and
//! Linux. The question they answer is the one the seam exists to answer: does
//! a backend reached through `OpenDAL` behave like `LocalBackend`?

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use tungstate_backend::{Backend, BackendError};
use tungstate_journal::{Endpoint, Journal, NewConnection, Scheme};
use tungstate_secret::MemoryStore;

use super::*;

/// A connection pointed at a fresh temp directory, plus a backend on it.
///
/// Every test gets its own directory and its own in-memory journal, so the
/// suite is safe to run in parallel.
fn rig() -> (tempfile::TempDir, Journal, Box<dyn Backend>) {
    rig_at("")
}

/// The same, with the link end pointed at `prefix` inside the connection.
fn rig_at(prefix: &str) -> (tempfile::TempDir, Journal, Box<dyn Backend>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let journal = Journal::open_in_memory().expect("journal");
    let id = journal
        .create_connection(&NewConnection {
            name: "scratch".to_string(),
            scheme: Scheme::Fs,
            host: None,
            port: None,
            username: None,
            root: dir.path().to_string_lossy().into_owned(),
            options: BTreeMap::new(),
        })
        .expect("connection");

    let backend = open(
        &Endpoint::remote(id, std::path::PathBuf::from(prefix)),
        &journal,
        &MemoryStore::new(),
    )
    .expect("backend");
    (dir, journal, backend)
}

#[test]
fn a_local_endpoint_still_gets_the_local_backend() {
    // The compatibility promise of the whole slice: `None` means local, and a
    // link written before connections existed keeps working untouched.
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open_in_memory().unwrap();

    let backend = open(&Endpoint::local(dir.path()), &journal, &MemoryStore::new()).unwrap();

    std::fs::write(dir.path().join("a.mp4"), b"local").unwrap();
    assert_eq!(backend.stat(Path::new("a.mp4")).unwrap().len, 5);
}

#[test]
fn a_file_round_trips_through_the_adapter() {
    let (dir, _journal, backend) = rig();

    backend.create_dir_all(Path::new("2024")).unwrap();
    {
        let mut writer = backend.create_write(Path::new("2024/holiday.mp4")).unwrap();
        writer.write_all(b"hello").unwrap();
        writer.finish().unwrap();
    }

    let meta = backend.stat(Path::new("2024/holiday.mp4")).unwrap();
    assert_eq!(meta.len, 5);
    assert!(!meta.is_dir);
    assert!(!meta.is_symlink);
    assert_eq!(
        std::fs::read(dir.path().join("2024/holiday.mp4")).unwrap(),
        b"hello"
    );

    let mut contents = String::new();
    backend
        .open_read(Path::new("2024/holiday.mp4"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "hello");
}

#[test]
fn the_root_lists_and_stats_like_a_directory() {
    // The most-travelled call in the crate: the drain's walk starts here.
    let (_dir, _journal, backend) = rig();
    backend.create_dir_all(Path::new("sub")).unwrap();
    backend
        .create_write(Path::new("a.mp4"))
        .unwrap()
        .finish()
        .unwrap();

    assert!(backend.stat(Path::new("")).unwrap().is_dir);

    let mut entries = backend.read_dir(Path::new("")).unwrap();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(entries.len(), 2, "got {entries:?}");
    assert_eq!(entries[0].path, Path::new("a.mp4"));
    assert!(!entries[0].meta.is_dir);
    assert_eq!(entries[1].path, Path::new("sub"));
    assert!(entries[1].meta.is_dir);
}

#[test]
fn a_nested_listing_returns_paths_relative_to_the_root() {
    // Entries have to be feedable straight back in, or the recursive walk
    // starts prepending the directory twice.
    let (_dir, _journal, backend) = rig();
    backend.create_dir_all(Path::new("a/b")).unwrap();
    backend
        .create_write(Path::new("a/b/c.mp4"))
        .unwrap()
        .finish()
        .unwrap();

    let entries = backend.read_dir(Path::new("a/b")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, Path::new("a/b/c.mp4"));
    assert_eq!(backend.stat(&entries[0].path).unwrap().len, 0);
}

#[test]
fn renaming_and_removing_behave_like_the_local_backend() {
    let (_dir, _journal, backend) = rig();
    backend.create_dir_all(Path::new("d")).unwrap();
    backend
        .create_write(Path::new("d/a"))
        .unwrap()
        .finish()
        .unwrap();

    backend.rename(Path::new("d/a"), Path::new("d/b")).unwrap();
    assert!(backend.stat(Path::new("d/a")).is_err());
    assert!(backend.stat(Path::new("d/b")).is_ok());

    backend.remove_file(Path::new("d/b")).unwrap();
    backend.remove_dir(Path::new("d")).unwrap();
    assert!(backend.stat(Path::new("d")).is_err());
}

#[test]
fn a_missing_file_reports_not_found_rather_than_a_protocol_error() {
    // Three places in the engine read "stat failed" as "nothing is there".
    let (_dir, _journal, backend) = rig();
    match backend.stat(Path::new("missing.mp4")) {
        Err(BackendError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected a NotFound io error, got {other:?}"),
    }
}

#[test]
fn paths_that_escape_the_root_are_refused_before_any_request() {
    let (_dir, _journal, backend) = rig();

    assert!(matches!(
        backend.stat(Path::new("../secrets")),
        Err(BackendError::PathEscapesRoot(_))
    ));
    assert!(matches!(
        backend.open_read(Path::new("/etc/passwd")),
        Err(BackendError::PathNotRelative(_))
    ));
}

#[test]
fn a_vanished_root_is_never_conjured_back_into_existence() {
    // OpenDAL's `fs` builder creates its root if missing, which is exactly how
    // an unmounted NAS turns into an empty folder on the boot disk. The factory
    // has to refuse before OpenDAL ever sees the path.
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("unmounted");
    let journal = Journal::open_in_memory().unwrap();
    let id = journal
        .create_connection(&NewConnection {
            name: "nas".to_string(),
            scheme: Scheme::Fs,
            host: None,
            port: None,
            username: None,
            root: gone.to_string_lossy().into_owned(),
            options: BTreeMap::new(),
        })
        .unwrap();

    let result = open(
        &Endpoint::remote(id, std::path::PathBuf::new()),
        &journal,
        &MemoryStore::new(),
    );

    assert!(matches!(result, Err(OpenError::RootUnreachable { .. })));
    assert!(!gone.exists(), "the root must not have been conjured up");
}

#[test]
fn a_finished_write_is_complete_and_the_writer_is_spent() {
    let (dir, _journal, backend) = rig();

    let mut writer = backend.create_write(Path::new("v.mp4")).unwrap();
    writer.write_all(b"payload").unwrap();
    writer.finish().unwrap();

    assert_eq!(std::fs::read(dir.path().join("v.mp4")).unwrap(), b"payload");
    assert_eq!(backend.stat(Path::new("v.mp4")).unwrap().len, 7);
}

#[test]
fn a_file_larger_than_one_chunk_streams_intact() {
    // The engine reads in 1 MiB chunks, so the multi-request path is the real
    // one; a single small file would never exercise it.
    let (_dir, _journal, backend) = rig();
    let payload: Vec<u8> = (0..3_000_000_u32).map(|n| (n % 251) as u8).collect();

    {
        let mut writer = backend.create_write(Path::new("big.bin")).unwrap();
        writer.write_all(&payload).unwrap();
        writer.finish().unwrap();
    }

    let mut readback = Vec::new();
    let mut reader = backend.open_read(Path::new("big.bin")).unwrap();
    let mut chunk = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        readback.extend_from_slice(&chunk[..read]);
    }
    assert_eq!(readback, payload);
}

#[test]
fn capabilities_are_read_not_probed() {
    let (dir, _journal, backend) = rig();

    let capabilities = backend.capabilities();
    assert!(capabilities.atomic_rename, "fs renames in place");
    assert!(!capabilities.hard_links);

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "reading capabilities wrote something: {leftovers:?}"
    );
}

#[test]
fn an_unregistered_scheme_says_so_rather_than_failing_obscurely() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal
        .create_connection(&NewConnection {
            name: "nas-ftp".to_string(),
            scheme: Scheme::Ftp,
            host: Some("nas.local".to_string()),
            port: Some(21),
            username: Some("me".to_string()),
            root: "/volume1".to_string(),
            options: BTreeMap::new(),
        })
        .unwrap();

    let result = open(
        &Endpoint::remote(id, std::path::PathBuf::new()),
        &journal,
        &MemoryStore::new(),
    );
    assert!(matches!(
        result,
        Err(OpenError::SchemeNotCompiled { scheme: "ftp", .. })
    ));
}

#[test]
fn a_link_end_that_does_not_exist_yet_is_created_on_first_write() {
    // The behaviour the manual run of this slice needs: `scratch:inbox` where
    // `inbox` has never existed. The connection is the storage and must be
    // there; a folder inside it is just a folder, created like any other.
    let (dir, _journal, backend) = rig_at("inbox");

    assert!(
        backend.root_token().is_ok(),
        "the connection is reachable even though `inbox` is not there yet"
    );
    assert!(!dir.path().join("inbox").exists());

    backend.create_dir_all(Path::new("")).unwrap();
    {
        let mut writer = backend.create_write(Path::new("a.mp4")).unwrap();
        writer.write_all(b"landed").unwrap();
        writer.finish().unwrap();
    }

    assert_eq!(
        std::fs::read(dir.path().join("inbox/a.mp4")).unwrap(),
        b"landed"
    );
    assert_eq!(backend.stat(Path::new("a.mp4")).unwrap().len, 6);
}

#[test]
fn a_link_end_sees_only_what_is_inside_it() {
    // The prefix is a root as far as the caller is concerned: a sibling folder
    // in the same connection must not appear in the listing, and must not be
    // reachable by path.
    let (dir, _journal, backend) = rig_at("inbox");
    std::fs::create_dir_all(dir.path().join("inbox/2026")).unwrap();
    std::fs::write(dir.path().join("inbox/2026/a.mp4"), b"mine").unwrap();
    std::fs::write(dir.path().join("elsewhere.mp4"), b"not mine").unwrap();

    let entries = backend.read_dir(Path::new("")).unwrap();
    assert_eq!(entries.len(), 1, "got {entries:?}");
    assert_eq!(entries[0].path, Path::new("2026"));

    let nested = backend.read_dir(Path::new("2026")).unwrap();
    assert_eq!(nested[0].path, Path::new("2026/a.mp4"));
    assert_eq!(backend.stat(&nested[0].path).unwrap().len, 4);

    assert!(
        backend.stat(Path::new("elsewhere.mp4")).is_err(),
        "a sibling of the link end must be out of reach"
    );
}

#[test]
fn a_vanished_connection_stops_a_drain_even_though_the_end_is_only_a_prefix() {
    // The rail that matters, at the new granularity. Deleting the connection
    // root is what an unmounting NAS looks like from here.
    let (dir, _journal, backend) = rig_at("inbox");
    backend.create_dir_all(Path::new("")).unwrap();
    assert!(backend.root_token().is_ok());

    std::fs::remove_dir_all(dir.path()).unwrap();

    assert!(matches!(
        backend.root_token(),
        Err(BackendError::RootUnreachable(_))
    ));
    assert!(matches!(
        backend.create_dir_all(Path::new("2026")),
        Err(BackendError::RootUnreachable(_))
    ));
    assert!(
        !dir.path().exists(),
        "the root must not have been recreated"
    );
}
