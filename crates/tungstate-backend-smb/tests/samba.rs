//! The backend over a real SMB server.
//!
//! Gated behind `--features samba-integration` and pointed at a server by
//! `TUNGSTATE_SMB_*`. CI runs it on Linux against Samba. Bring one up with:
//!
//! ```text
//! docker run -d --name tungstate-samba -p 445:445 \
//!   -e NETBIOS_DISABLE=1 -e AVAHI_DISABLE=1 -e WSDD2_DISABLE=1 \
//!   -e ACCOUNT_tungstate=s3cret -e UID_tungstate=1000 \
//!   -e 'SAMBA_VOLUME_CONFIG_media=[media]; path=/shares/media; valid users = tungstate; guest ok = no; read only = no' \
//!   ghcr.io/servercontainers/samba:smbd-only-latest
//! docker exec tungstate-samba sh -c 'mkdir -p /shares/media && chown 1000 /shares/media'
//! ```
#![cfg(feature = "samba-integration")]

use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use tungstate_backend::{Backend, BackendError};
use tungstate_backend_smb::{Settings, SmbBackend};

fn settings(root: &str) -> Settings {
    let var =
        |name: &str, fallback: &str| std::env::var(name).unwrap_or_else(|_| fallback.to_string());
    Settings {
        host: var("TUNGSTATE_SMB_HOST", "127.0.0.1"),
        port: var("TUNGSTATE_SMB_PORT", "445").parse().ok(),
        root: root.to_string(),
        username: var("TUNGSTATE_SMB_USER", "tungstate"),
        password: var("TUNGSTATE_SMB_PASSWORD", "s3cret"),
        require_encryption: false,
    }
}

fn share() -> String {
    std::env::var("TUNGSTATE_SMB_SHARE").unwrap_or_else(|_| "media".to_string())
}

/// A folder of this test's own inside the share, made first, and a backend
/// rooted at it. Tests share one server, so each keeps to its own folder.
fn own(label: &str) -> (SmbBackend, String) {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let folder = format!("{label}-{stamp}");
    let at_share = SmbBackend::connect(&settings(&share()), Path::new(""), "nas").expect("share");
    at_share
        .create_dir_all(Path::new(&folder))
        .expect("own folder");
    let root = format!("{}/{folder}", share());
    (
        SmbBackend::connect(&settings(&root), Path::new(""), "nas").expect("rooted"),
        root,
    )
}

fn put(backend: &SmbBackend, path: &str, bytes: &[u8]) {
    let mut sink = backend.create_write(Path::new(path)).expect("create");
    sink.write_all(bytes).expect("write");
    sink.finish().expect("finish");
}

fn get(backend: &SmbBackend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend
        .open_read(Path::new(path))
        .expect("open")
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

#[test]
fn a_file_round_trips_and_its_size_is_reported() {
    let (backend, _) = own("round-trip");
    put(&backend, "hello.txt", b"hello over smb");
    assert_eq!(get(&backend, "hello.txt"), b"hello over smb");
    let meta = backend.stat(Path::new("hello.txt")).unwrap();
    assert_eq!(meta.len, 14);
    assert!(!meta.is_dir);
    assert!(meta.modified.is_some());
}

#[test]
fn a_file_larger_than_one_request_streams_intact() {
    // Many times the 1 MiB request size, and not a multiple of it, so reads
    // and writes are genuinely split and the last of each is short.
    let (backend, _) = own("large");
    let big: Vec<u8> = (0..20_000_003_u32).map(|n| (n % 251) as u8).collect();
    put(&backend, "big.bin", &big);
    assert_eq!(
        backend.stat(Path::new("big.bin")).unwrap().len,
        big.len() as u64
    );
    assert_eq!(get(&backend, "big.bin"), big);
    assert_eq!(
        backend.read_prefix(Path::new("big.bin"), 10).unwrap(),
        &big[..10]
    );
}

#[test]
fn an_empty_file_reads_as_zero_bytes_rather_than_hanging() {
    let (backend, _) = own("empty");
    put(&backend, "nothing.txt", b"");
    assert!(get(&backend, "nothing.txt").is_empty());
}

#[test]
fn folders_are_made_and_listed_with_paths_relative_to_the_link_end() {
    let (backend, _) = own("listing");
    backend.create_dir_all(Path::new("2024/trip")).unwrap();
    // Again: finding it there is not an error.
    backend.create_dir_all(Path::new("2024/trip")).unwrap();
    put(&backend, "2024/trip/a.jpg", b"a");
    put(&backend, "top.txt", b"t");

    let mut top: Vec<String> = backend
        .read_dir(Path::new(""))
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{}{}",
                e.path.display(),
                if e.meta.is_dir { "/" } else { "" }
            )
        })
        .collect();
    top.sort();
    assert_eq!(top, ["2024/", "top.txt"]);
    let inner = backend.read_dir(Path::new("2024/trip")).unwrap();
    assert_eq!(inner.len(), 1);
    assert_eq!(inner[0].path, Path::new("2024/trip/a.jpg"));
}

#[test]
fn renaming_replaces_what_was_there_and_removing_takes_it_away() {
    let (backend, _) = own("rename");
    put(&backend, "a.tmp", b"new");
    put(&backend, "a.txt", b"old");
    backend
        .rename(Path::new("a.tmp"), Path::new("a.txt"))
        .unwrap();
    assert_eq!(get(&backend, "a.txt"), b"new");
    assert!(backend.stat(Path::new("a.tmp")).is_err());

    backend.create_dir_all(Path::new("empty")).unwrap();
    backend.remove_file(Path::new("a.txt")).unwrap();
    backend.remove_dir(Path::new("empty")).unwrap();
    assert!(backend.read_dir(Path::new("")).unwrap().is_empty());
}

#[test]
fn a_folder_that_is_not_empty_is_not_removed() {
    let (backend, _) = own("not-empty");
    backend.create_dir_all(Path::new("keep")).unwrap();
    put(&backend, "keep/me.txt", b"x");
    assert!(backend.remove_dir(Path::new("keep")).is_err());
    assert_eq!(get(&backend, "keep/me.txt"), b"x");
}

#[test]
fn a_missing_file_reports_not_found_rather_than_a_protocol_error() {
    // The engine reads "not found" as "nothing is there" in three places.
    let (backend, _) = own("missing");
    match backend.stat(Path::new("nope.txt")) {
        Err(BackendError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected not found, got {other:?}"),
    }
    assert!(matches!(
        backend.stat(Path::new("no/such/folder.txt")),
        Err(BackendError::Io { .. })
    ));
}

#[test]
fn a_link_end_that_does_not_exist_yet_is_reachable_and_made_on_first_write() {
    let (_, root) = own("link-end");
    let end = SmbBackend::connect(&settings(&root), Path::new("inbox/2026"), "nas").unwrap();
    // Not made yet: empty, as a sync's new member is, rather than missing.
    assert!(end.read_dir(Path::new("")).unwrap().is_empty());
    // A folder inside it that is not there is still an error.
    assert!(end.read_dir(Path::new("nope")).is_err());
    end.root_token()
        .expect("the connection's folder is what must exist");
    end.create_dir_all(Path::new("")).unwrap();
    let mut sink = end.create_write(Path::new("a.txt")).unwrap();
    sink.write_all(b"landed").unwrap();
    sink.finish().unwrap();

    // Seen from the connection's folder, it is inside `inbox/2026`.
    let from_root = SmbBackend::connect(&settings(&root), Path::new(""), "nas").unwrap();
    assert_eq!(get(&from_root, "inbox/2026/a.txt"), b"landed");
    // And the link end sees only what is inside it.
    assert_eq!(end.read_dir(Path::new("")).unwrap().len(), 1);
}

#[test]
fn a_vanished_folder_stops_the_run_and_is_never_made_again() {
    let (backend, root) = own("vanishing");
    let at_share = SmbBackend::connect(&settings(&share()), Path::new(""), "nas").unwrap();
    let folder = root.split('/').next_back().unwrap().to_string();
    at_share.remove_dir(Path::new(&folder)).unwrap();

    assert!(matches!(
        backend.root_token(),
        Err(BackendError::RootUnreachable(_))
    ));
    // Listing it is not mistaken for an empty folder not made yet.
    assert!(matches!(
        backend.read_dir(Path::new("")),
        Err(BackendError::RootUnreachable(_))
    ));
    assert!(backend.create_dir_all(Path::new("recreated")).is_err());
    assert!(
        at_share.stat(Path::new(&folder)).is_err(),
        "it must not come back"
    );
}

#[test]
fn a_wrong_password_says_so() {
    let mut wrong = settings(&share());
    wrong.password = "definitely-not-it".into();
    assert!(matches!(
        SmbBackend::connect(&wrong, Path::new(""), "nas"),
        Err(BackendError::Auth { .. })
    ));
}

#[test]
fn a_share_that_does_not_exist_is_unreachable_rather_than_a_protocol_puzzle() {
    assert!(matches!(
        SmbBackend::connect(&settings("no-such-share"), Path::new(""), "nas"),
        Err(BackendError::RootUnreachable(_))
    ));
}

#[test]
fn a_modification_time_can_be_set_and_is_read_back() {
    let (backend, _) = own("mtime");
    put(&backend, "old.mp4", b"x");
    let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert!(backend.set_modified(Path::new("old.mp4"), then).unwrap());
    let read = backend
        .stat(Path::new("old.mp4"))
        .unwrap()
        .modified
        .unwrap();
    assert_eq!(
        read.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        1_700_000_000
    );
}
