//! The drain and a sync, over a real SMB share with nothing mounted.
//!
//! Gated behind `--features smb-integration` and pointed at a server by
//! `TUNGSTATE_SMB_*`. CI runs it on Linux against Samba; bring one up as
//! `crates/tungstate-backend-smb/tests/samba.rs` describes.
#![cfg(feature = "smb-integration")]

use std::io::{Read, Write};
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt as _;
use tungstate_backend::Backend;
use tungstate_backend_smb::{Settings, SmbBackend};

fn var(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn password() -> String {
    var("TUNGSTATE_SMB_PASSWORD", "s3cret")
}

/// A folder of this test's own inside the share, made through a second,
/// independent client; the connection under test is rooted at it.
fn own(label: &str) -> (String, SmbBackend) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let share = var("TUNGSTATE_SMB_SHARE", "media");
    let folder = format!("{label}-{stamp}");
    let settings = |root: &str| Settings {
        host: var("TUNGSTATE_SMB_HOST", "127.0.0.1"),
        port: var("TUNGSTATE_SMB_PORT", "445").parse().ok(),
        root: root.to_string(),
        username: var("TUNGSTATE_SMB_USER", "tungstate"),
        password: password(),
        require_encryption: false,
    };
    SmbBackend::connect(&settings(&share), Path::new(""), "raw")
        .expect("share")
        .create_dir_all(Path::new(&folder))
        .expect("own folder");
    let root = format!("{share}/{folder}");
    let raw = SmbBackend::connect(&settings(&root), Path::new(""), "raw").expect("raw");
    (root, raw)
}

fn tungstate(home: &tempfile::TempDir, secret: &str) -> Command {
    let mut command =
        Command::cargo_bin("tungstate").expect("binary `tungstate` should be built by cargo test");
    command
        .env("TUNGSTATE_JOURNAL", home.path().join("journal.db"))
        .env("TUNGSTATE_SECRETS", "memory")
        .env("TUNGSTATE_SECRET_NAS", secret);
    command
}

fn cli(home: &tempfile::TempDir) -> Command {
    tungstate(home, &password())
}

fn connect(home: &tempfile::TempDir, secret: &str, root: &str) {
    tungstate(home, secret)
        .args(["connection", "add", "nas", "--scheme", "smb"])
        .args(["--host", &var("TUNGSTATE_SMB_HOST", "127.0.0.1")])
        .args(["--port", &var("TUNGSTATE_SMB_PORT", "445")])
        .args(["--user", &var("TUNGSTATE_SMB_USER", "tungstate")])
        .args(["--root", root])
        .arg("--secret-stdin")
        .write_stdin("\n")
        .assert()
        .success();
}

fn fetch(raw: &SmbBackend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    raw.open_read(Path::new(path))
        .unwrap_or_else(|error| panic!("`{path}` is not on the share: {error}"))
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

fn put(raw: &SmbBackend, path: &str, bytes: &[u8]) {
    let mut sink = raw.create_write(Path::new(path)).expect("create");
    sink.write_all(bytes).expect("write");
    sink.finish().expect("finish");
}

#[test]
fn a_share_is_reachable_and_accepts_files() {
    let home = tempfile::tempdir().unwrap();
    let (root, raw) = own("probe");
    put(&raw, "already-here.txt", b"x");
    connect(&home, &password(), &root);

    cli(&home)
        .args(["connection", "test", "nas"])
        .assert()
        .success()
        .stdout(predicates::str::contains("is reachable"))
        .stdout(predicates::str::contains("already-here.txt"))
        .stdout(predicates::str::contains("accepts files"));
}

#[test]
fn a_wrong_password_says_so_rather_than_blaming_the_network() {
    let home = tempfile::tempdir().unwrap();
    let (root, _) = own("wrong");
    connect(&home, "definitely-not-it", &root);

    tungstate(&home, "definitely-not-it")
        .args(["connection", "test", "nas"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("credentials"));
}

#[test]
fn a_whole_drain_lands_on_the_share_and_reclaims_the_source() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("src");
    std::fs::create_dir_all(source.join("2024")).unwrap();
    let big: Vec<u8> = (0..3_500_000_u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(source.join("big.bin"), &big).unwrap();
    std::fs::write(source.join("note.txt"), b"hello").unwrap();
    std::fs::write(source.join("2024/trip.txt"), b"trip").unwrap();

    let (root, raw) = own("drain");
    connect(&home, &password(), &root);
    cli(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("nas:inbox")
        .args([
            "--name",
            "to-share",
            "--move",
            "--verify",
            "readback",
            "--cooldown",
            "0",
        ])
        .assert()
        .success();
    cli(&home)
        .args(["link", "run", "to-share"])
        .assert()
        .success()
        .stdout(predicates::str::contains("3 transferred"));

    assert_eq!(fetch(&raw, "inbox/big.bin"), big);
    assert_eq!(fetch(&raw, "inbox/2024/trip.txt"), b"trip");
    assert!(!source.join("big.bin").exists(), "source must be reclaimed");
    // SMB renames, so nothing is left under a temporary name.
    let names: Vec<String> = raw
        .read_dir(Path::new("inbox"))
        .unwrap()
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert!(
        names.iter().all(|name| !name.contains(".tungstate")),
        "{names:?}"
    );
}

#[test]
fn replace_is_allowed_because_a_share_can_rename() {
    // The opposite of FTP and S3: the file already there can be moved aside
    // first, so the promise `replace` makes can be kept.
    let home = tempfile::tempdir().unwrap();
    let (root, _) = own("replace");
    connect(&home, &password(), &root);
    cli(&home)
        .args(["link", "add", "/tmp/from", "nas:inbox"])
        .args(["--name", "x", "--copy", "--on-conflict", "replace"])
        .assert()
        .success();
}

#[test]
fn a_sync_with_a_share_sets_a_changed_version_aside_rather_than_losing_it() {
    let home = tempfile::tempdir().unwrap();
    let laptop = home.path().join("laptop");
    std::fs::create_dir_all(&laptop).unwrap();
    std::fs::write(laptop.join("a.mp4"), b"version one").unwrap();
    let (root, raw) = own("sync");
    connect(&home, &password(), &root);
    cli(&home)
        .args(["sync", "add", "capcut"])
        .arg(&laptop)
        .arg("nas:capcut")
        .args(["--push", "--cooldown", "0"])
        .assert()
        .success();
    cli(&home)
        .args(["sync", "run", "capcut", "--yes"])
        .assert()
        .success();
    assert_eq!(fetch(&raw, "capcut/a.mp4"), b"version one");

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(laptop.join("a.mp4"), b"version two, longer").unwrap();
    cli(&home)
        .args(["sync", "run", "capcut", "--yes"])
        .assert()
        .success()
        .stdout(predicates::str::contains("cannot rename").not());
    assert_eq!(fetch(&raw, "capcut/a.mp4"), b"version two, longer");

    cli(&home)
        .args(["sync", "preview", "capcut"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing to do"));
}
