//! Trusting a server, the drain and a sync, over a real SFTP server.
//!
//! Gated behind `--features sftp-integration` and pointed at a server by
//! `TUNGSTATE_SFTP_*`; the server is the one the backend's own suite uses
//! (see `tungstate-backend-sftp/tests/openssh.rs`).
#![cfg(feature = "sftp-integration")]

use assert_cmd::Command;
use predicates::prelude::*;

fn var(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn unique(label: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{label}-{stamp}")
}

/// A command with its own journal, the secret passed as a headless machine
/// would pass one.
fn cli(home: &tempfile::TempDir, secret: &str) -> Command {
    let mut command =
        Command::cargo_bin("tungstate").expect("binary `tungstate` should be built by cargo test");
    command
        .env("TUNGSTATE_JOURNAL", home.path().join("journal.db"))
        .env("TUNGSTATE_SECRETS", "memory")
        .env("TUNGSTATE_SECRET_NAS", secret);
    command
}

fn password() -> String {
    var("TUNGSTATE_SFTP_PASSWORD", "s3cret")
}

/// Add `nas`, rooted at a fresh folder under `media`, with `extra` flags.
fn add(home: &tempfile::TempDir, secret: &str, extra: &[&str]) -> String {
    let root = format!("media/{}", unique("cli"));
    cli(home, secret)
        .args(["connection", "add", "nas", "--scheme", "sftp"])
        .args(["--host", &var("TUNGSTATE_SFTP_HOST", "127.0.0.1")])
        .args(["--port", &var("TUNGSTATE_SFTP_PORT", "2223")])
        .args(["--user", &var("TUNGSTATE_SFTP_USER", "tungstate")])
        .args(["--root", "media"])
        .args(extra)
        .arg("--secret-stdin")
        .write_stdin("\n")
        .assert()
        .success();
    root
}

/// The fingerprint `connection test` reports for a server not yet trusted.
fn fingerprint(home: &tempfile::TempDir, secret: &str) -> String {
    let said = cli(home, secret)
        .args(["connection", "test", "nas"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("connection trust nas"))
        .get_output()
        .stderr
        .clone();
    let said = String::from_utf8_lossy(&said);
    let start = said.find("SHA256:").expect("a fingerprint in the message");
    said[start..]
        .split(|c: char| c.is_whitespace() || c == ';')
        .next()
        .unwrap()
        .to_string()
}

fn trusted(home: &tempfile::TempDir, secret: &str) {
    let shown = fingerprint(home, secret);
    cli(home, secret)
        .args(["connection", "trust", "nas", "--fingerprint", &shown])
        .assert()
        .success()
        .stdout(predicate::str::contains("trusted `nas`"));
}

#[test]
fn a_server_is_trusted_only_once_its_fingerprint_is_agreed() {
    let home = tempfile::tempdir().unwrap();
    add(&home, &password(), &[]);
    let shown = fingerprint(&home, &password());

    // A script that names the wrong fingerprint trusts nothing.
    cli(&home, &password())
        .args([
            "connection",
            "trust",
            "nas",
            "--fingerprint",
            "SHA256:not-it",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing was trusted"));
    // Nor does one with no terminal to answer on and no fingerprint named.
    cli(&home, &password())
        .args(["connection", "trust", "nas"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(&shown));

    cli(&home, &password())
        .args(["connection", "trust", "nas", "--fingerprint", &shown])
        .assert()
        .success();
    cli(&home, &password())
        .args(["connection", "test", "nas"])
        .assert()
        .success()
        .stdout(predicate::str::contains("accepts files"));
    cli(&home, &password())
        .args(["connection", "trust", "nas"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already trusted"));
}

#[test]
fn a_wrong_password_says_so_once_the_server_is_trusted() {
    let home = tempfile::tempdir().unwrap();
    add(&home, "not-it", &[]);
    trusted(&home, "not-it");
    cli(&home, "not-it")
        .args(["connection", "test", "nas"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("credentials"));
}

#[test]
fn a_key_and_its_passphrase_sign_in() {
    let home = tempfile::tempdir().unwrap();
    let keys = std::path::PathBuf::from(var("TUNGSTATE_SFTP_KEYS", "/tmp/tungstate-sftp-keys"));
    let key = keys.join("id_locked");
    add(&home, "open sesame", &["--key", key.to_str().unwrap()]);
    trusted(&home, "open sesame");
    cli(&home, "open sesame")
        .args(["connection", "test", "nas"])
        .assert()
        .success();
}

#[test]
fn a_whole_drain_lands_over_sftp_and_reclaims_the_source() {
    let home = tempfile::tempdir().unwrap();
    add(&home, &password(), &[]);
    trusted(&home, &password());
    let inbox = unique("inbox");
    let source = home.path().join("src");
    std::fs::create_dir_all(source.join("2024")).unwrap();
    let big: Vec<u8> = (0..3_000_000_u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(source.join("big.bin"), &big).unwrap();
    std::fs::write(source.join("2024/trip.txt"), b"trip").unwrap();

    cli(&home, &password())
        .args(["link", "add"])
        .arg(&source)
        .arg(format!("nas:{inbox}"))
        .args([
            "--name",
            "to-nas",
            "--move",
            "--verify",
            "readback",
            "--cooldown",
            "0",
        ])
        .assert()
        .success();
    cli(&home, &password())
        .args(["link", "run", "to-nas"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 transferred"));
    assert!(!source.join("big.bin").exists(), "moved, so reclaimed");

    // A sync back the other way finds the same bytes there.
    let back = home.path().join("back");
    std::fs::create_dir_all(&back).unwrap();
    cli(&home, &password())
        .args(["sync", "add", "pair"])
        .arg(&back)
        .arg(format!("nas:{inbox}"))
        .args(["--all", "--cooldown", "0"])
        .assert()
        .success();
    cli(&home, &password())
        .args(["sync", "run", "pair", "--yes"])
        .assert()
        .success();
    assert_eq!(std::fs::read(back.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(back.join("2024/trip.txt")).unwrap(), b"trip");
    cli(&home, &password())
        .args(["sync", "preview", "pair"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to do"));
}
