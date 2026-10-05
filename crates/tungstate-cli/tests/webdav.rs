//! The backend, the drain and a sync, over a real `WebDAV` server.
//!
//! Gated behind `--features webdav-integration` and pointed at a server by
//! `TUNGSTATE_WEBDAV_*`. CI runs it on Linux against rclone's `WebDAV` server.
//! Bring one up with:
//!
//! ```text
//! docker run -d --name tungstate-webdav -p 8088:8080 rclone/rclone:1.71 \
//!   serve webdav /data --addr :8080 --user tungstate --pass s3cret
//! ```
#![cfg(feature = "webdav-integration")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tungstate_backend::Backend;
use tungstate_journal::{Endpoint, Journal, NewConnection, Scheme};
use tungstate_secret::SecretStore as _;

struct Server {
    endpoint: String,
    user: String,
    password: String,
}

/// A panic rather than a skip, as in the other suites: asking for this
/// feature is asking for these tests to run.
fn server() -> Server {
    let var =
        |name: &str, fallback: &str| std::env::var(name).unwrap_or_else(|_| fallback.to_string());
    Server {
        endpoint: var("TUNGSTATE_WEBDAV_ENDPOINT", "http://127.0.0.1:8088"),
        user: var("TUNGSTATE_WEBDAV_USER", "tungstate"),
        password: var("TUNGSTATE_WEBDAV_PASSWORD", "s3cret"),
    }
}

fn unique(label: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{label}-{stamp}")
}

/// A backend rooted at a folder of this test's own, so tests sharing one
/// server stay parallel-safe.
fn backend_at(root: &str, password: &str) -> Box<dyn Backend> {
    let server = server();
    let journal = Journal::open_in_memory().unwrap();
    let id = journal
        .create_connection(&NewConnection {
            name: "dav".to_string(),
            scheme: Scheme::WebDav,
            host: None,
            port: None,
            username: Some(server.user.clone()),
            root: root.to_string(),
            options: [("endpoint".to_string(), server.endpoint.clone())].into(),
        })
        .unwrap();
    let secrets = tungstate_secret::MemoryStore::new();
    secrets
        .set(&tungstate_secret::connection_key("dav"), password)
        .unwrap();
    tungstate_backend_opendal::open(&Endpoint::remote(id, PathBuf::new()), &journal, &secrets)
        .unwrap()
}

fn own(label: &str) -> Box<dyn Backend> {
    backend_at(&unique(label), &server().password)
}

fn put(backend: &dyn Backend, path: &str, bytes: &[u8]) {
    // In small writes, as the engine sends a file: the case `OpenDAL`'s own
    // WebDAV writer refuses after the first.
    let mut sink = backend.create_write(Path::new(path)).expect("create");
    for piece in bytes.chunks(64 * 1024) {
        sink.write_all(piece).expect("write");
    }
    sink.finish().expect("finish");
}

fn get(backend: &dyn Backend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend
        .open_read(Path::new(path))
        .expect("open")
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

#[test]
fn a_file_far_bigger_than_one_write_streams_up_intact() {
    let backend = own("big");
    let body: Vec<u8> = (0..6 * 1024 * 1024_u32).map(|i| (i % 251) as u8).collect();
    put(backend.as_ref(), "2024/trip/clip.mp4", &body);

    assert_eq!(get(backend.as_ref(), "2024/trip/clip.mp4"), body);
    assert_eq!(
        backend.stat(Path::new("2024/trip/clip.mp4")).unwrap().len,
        body.len() as u64
    );
    assert_eq!(
        backend
            .read_range(Path::new("2024/trip/clip.mp4"), 1_000_000, 10)
            .unwrap(),
        body[1_000_000..1_000_010]
    );
}

#[test]
fn names_with_spaces_and_symbols_land_where_they_are_listed() {
    let backend = own("names");
    put(backend.as_ref(), "My Photos/Holiday #1/café?.jpg", b"c");
    let listed = backend.read_dir(Path::new("My Photos/Holiday #1")).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, Path::new("My Photos/Holiday #1/café?.jpg"));
    assert_eq!(
        get(backend.as_ref(), "My Photos/Holiday #1/café?.jpg"),
        b"c"
    );
}

#[test]
fn a_listing_says_what_a_stat_would() {
    let backend = own("complete");
    put(backend.as_ref(), "a.jpg", b"a");
    put(backend.as_ref(), "2024/b.mp4", b"bb");

    assert!(backend.listing_is_complete());
    let mut seen = 0;
    for folder in ["", "2024"] {
        for entry in backend.read_dir(Path::new(folder)).unwrap() {
            assert_eq!(
                entry.meta,
                backend.stat(&entry.path).unwrap(),
                "{}",
                entry.path.display()
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 3, "a.jpg, 2024 and 2024/b.mp4");
}

#[test]
fn renaming_replaces_what_was_there() {
    let backend = own("rename");
    assert!(backend.capabilities().atomic_rename);
    put(backend.as_ref(), "new.part", b"new");
    put(backend.as_ref(), "film.mp4", b"old");
    backend
        .rename(Path::new("new.part"), Path::new("film.mp4"))
        .unwrap();
    assert_eq!(get(backend.as_ref(), "film.mp4"), b"new");
    assert!(backend.stat(Path::new("new.part")).is_err());
}

#[test]
fn an_upload_dropped_before_it_finishes_is_not_kept_whole() {
    let backend = own("dropped");
    let mut sink = backend.create_write(Path::new("half.mp4")).unwrap();
    sink.write_all(&[7; 256 * 1024]).unwrap();
    drop(sink);
    // Aborted mid-body, so either nothing is there or the server kept what
    // it had; never a file that looks finished at full size.
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Ok(meta) = backend.stat(Path::new("half.mp4")) {
        assert!(meta.len <= 256 * 1024);
    }
}

#[test]
fn a_link_end_not_made_yet_is_empty_rather_than_missing() {
    let backend = own("fresh");
    assert_eq!(
        backend.read_dir(Path::new("not-yet")).unwrap(),
        [] as [tungstate_backend::Entry; 0]
    );
}

/// A command with its own journal, the password passed as a headless machine
/// would pass one.
fn cli(home: &tempfile::TempDir) -> Command {
    let mut command =
        Command::cargo_bin("tungstate").expect("binary `tungstate` should be built by cargo test");
    command
        .env("TUNGSTATE_JOURNAL", home.path().join("journal.db"))
        .env("TUNGSTATE_SECRETS", "memory")
        .env("TUNGSTATE_SECRET_NAS", server().password);
    command
}

fn connect(home: &tempfile::TempDir, root: &str) {
    let server = server();
    cli(home)
        .args(["connection", "add", "nas", "--scheme", "webdav"])
        .args(["--endpoint", &server.endpoint, "--user", &server.user])
        .args(["--root", root, "--secret-stdin"])
        .write_stdin("\n")
        .assert()
        .success();
}

#[test]
fn a_wrong_password_says_so_rather_than_blaming_the_network() {
    let home = tempfile::tempdir().unwrap();
    connect(&home, &unique("wrong"));
    cli(&home)
        .env("TUNGSTATE_SECRET_NAS", "not-it")
        .args(["connection", "test", "nas"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("credentials"));
}

#[test]
fn a_connection_test_reaches_the_server() {
    let home = tempfile::tempdir().unwrap();
    connect(&home, &unique("probe"));
    cli(&home)
        .args(["connection", "test", "nas"])
        .assert()
        .success();
}

#[test]
fn a_whole_drain_lands_over_webdav_and_reclaims_the_source() {
    let home = tempfile::tempdir().unwrap();
    let root = unique("drain");
    connect(&home, &root);
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("2024")).unwrap();
    std::fs::write(source.path().join("a.jpg"), b"photo").unwrap();
    std::fs::write(source.path().join("2024/b.mp4"), vec![3_u8; 300_000]).unwrap();

    cli(&home)
        .args(["link", "add"])
        .arg(source.path())
        .args(["nas:inbox", "--name", "to-nas", "--move"])
        .args(["--verify", "readback", "--cooldown", "0"])
        .assert()
        .success();
    cli(&home)
        .args(["link", "run", "to-nas"])
        .assert()
        .success()
        .stdout(predicates::str::contains("2 transferred"));

    let landed = backend_at(&root, &server().password);
    assert_eq!(get(landed.as_ref(), "inbox/a.jpg"), b"photo");
    assert_eq!(
        get(landed.as_ref(), "inbox/2024/b.mp4"),
        vec![3_u8; 300_000]
    );
    assert!(!source.path().join("a.jpg").exists(), "moved, so reclaimed");
}

#[test]
fn a_sync_with_a_webdav_folder_runs_and_a_second_run_moves_nothing() {
    let home = tempfile::tempdir().unwrap();
    let root = unique("sync");
    connect(&home, &root);
    let laptop = tempfile::tempdir().unwrap();
    std::fs::write(laptop.path().join("notes.txt"), b"hello").unwrap();

    cli(&home)
        .args(["sync", "add", "both"])
        .arg(laptop.path())
        .arg("nas:shared")
        .args(["--all", "--cooldown", "0"])
        .assert()
        .success();
    cli(&home)
        .args(["sync", "run", "both", "--yes"])
        .assert()
        .success();
    let landed = backend_at(&root, &server().password);
    assert_eq!(get(landed.as_ref(), "shared/notes.txt"), b"hello");

    cli(&home)
        .args(["sync", "preview", "both"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing to do"));
}
