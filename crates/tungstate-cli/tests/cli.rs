//! End-to-end tests that run the built `tungstate` binary.

use assert_cmd::Command;
// `.not()` on a predicate, for asserting a marker is absent.
use predicates::prelude::PredicateBooleanExt as _;

fn tungstate() -> Command {
    Command::cargo_bin("tungstate").expect("binary `tungstate` should be built by cargo test")
}

#[test]
fn version_flag_prints_the_api_version() {
    tungstate()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains(tungstate_api::VERSION));
}

#[test]
fn folder_add_is_a_stub_for_now() {
    tungstate()
        .args(["folder", "add", "/tmp/example"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("not implemented yet"));
}

#[test]
fn help_output_is_stable() {
    let output = tungstate().arg("--help").output().expect("help should run");
    let help = String::from_utf8(output.stdout).expect("help output should be utf-8");
    insta::assert_snapshot!(help);
}

/// A command pointed at its own throwaway journal and an in-process secret
/// store, so nothing here touches the developer's real state and the whole
/// file is safe to run in parallel.
fn sandboxed(home: &tempfile::TempDir) -> Command {
    let mut command = tungstate();
    command
        .env("TUNGSTATE_JOURNAL", home.path().join("journal.db"))
        .env("TUNGSTATE_SECRETS", "memory");
    command
}

fn sandbox() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

#[test]
fn a_connection_can_be_added_listed_tested_and_removed() {
    let home = sandbox();
    let root = home.path().join("store");
    std::fs::create_dir_all(&root).unwrap();

    sandboxed(&home)
        .args(["connection", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("no connections configured"));

    sandboxed(&home)
        .args(["connection", "add", "scratch", "--scheme", "fs", "--root"])
        .arg(&root)
        .assert()
        .success()
        .stdout(predicates::str::contains("added connection `scratch`"));

    sandboxed(&home)
        .args(["connection", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("scratch"))
        .stdout(predicates::str::contains("fs"));

    std::fs::write(root.join("already-here.txt"), b"x").unwrap();
    sandboxed(&home)
        .args(["connection", "test", "scratch"])
        .assert()
        .success()
        .stdout(predicates::str::contains("is reachable"))
        .stdout(predicates::str::contains("1 entries"));

    sandboxed(&home)
        .args(["connection", "remove", "scratch"])
        .assert()
        .success()
        .stdout(predicates::str::contains("removed connection `scratch`"));

    sandboxed(&home)
        .args(["connection", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("no connections configured"));
}

#[test]
fn connection_update_changes_only_what_was_named() {
    let home = sandbox();
    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "ftp"])
        .args(["--host", "nas.local", "--port", "21", "--user", "me"])
        .args(["--root", "/volume1", "--secret-stdin"])
        .write_stdin("\n")
        .assert()
        .success();

    sandboxed(&home)
        .args(["connection", "update", "nas", "--root", "/volume2/media"])
        .assert()
        .success()
        .stdout(predicates::str::contains("updated connection `nas`"));

    // The root moved and nothing else did. The flags that were not typed are
    // the assertion here, not the one that was.
    sandboxed(&home)
        .args(["connection", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("root=/volume2/media"))
        .stdout(predicates::str::contains("nas.local:21"))
        .stdout(predicates::str::contains("as me"))
        .stdout(predicates::str::contains("ftp"));

    // A scheme change to something encrypted also drops the warning marker,
    // which is how `list` says the password stops crossing the wire in clear.
    sandboxed(&home)
        .args(["connection", "update", "nas", "--scheme", "ftps"])
        .assert()
        .success();
    sandboxed(&home)
        .args(["connection", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ftps"))
        .stdout(predicates::str::contains("root=/volume2/media"))
        .stdout(predicates::str::contains("[UNENCRYPTED]").not());
}

#[test]
fn connection_update_refuses_a_name_and_a_scheme_it_does_not_know() {
    let home = sandbox();
    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root", "/"])
        .assert()
        .success();

    sandboxed(&home)
        .args(["connection", "update", "nope", "--root", "/x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no connection named `nope`"));

    sandboxed(&home)
        .args(["connection", "update", "nas", "--scheme", "telepathy"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("must be fs, ftp or ftps"));
}

#[test]
fn a_connection_a_link_uses_can_still_be_updated() {
    // The asymmetry with `remove`, from the outside: a link holds a
    // connection id, so editing where the connection points re-points the
    // link too. That is what makes an edit safe and a delete not.
    let home = sandbox();
    let root = home.path().join("store");
    let source = home.path().join("from");
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    std::fs::create_dir_all(&source).unwrap();

    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root"])
        .arg(&root)
        .assert()
        .success();
    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("nas:inbox")
        .args(["--name", "drain", "--move"])
        .assert()
        .success();

    let moved = home.path().join("store-moved");
    std::fs::create_dir_all(moved.join("inbox")).unwrap();
    sandboxed(&home)
        .args(["connection", "update", "nas", "--root"])
        .arg(&moved)
        .assert()
        .success();

    // The link still names the same connection, and the connection now points
    // somewhere else. `link list` resolving at all is the property.
    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nas:inbox"));
}

#[test]
fn connection_password_reports_what_it_did_and_refuses_what_it_cannot_do() {
    let home = sandbox();
    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "ftp"])
        .args(["--host", "nas.local", "--secret-stdin"])
        .write_stdin("\n")
        .assert()
        .success();

    sandboxed(&home)
        .args(["connection", "password", "nas", "--secret-stdin"])
        .write_stdin("hunter2\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("stored a new password for `nas`"));

    sandboxed(&home)
        .args(["connection", "password", "nas", "--secret-stdin"])
        .write_stdin("\n")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "removed the stored password for `nas`",
        ));

    sandboxed(&home)
        .args(["connection", "password", "nope", "--secret-stdin"])
        .write_stdin("x\n")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no connection named `nope`"));

    // A password for a scheme that never sends one would sit in the keychain
    // unread for ever, so it is refused rather than quietly accepted.
    sandboxed(&home)
        .args([
            "connection",
            "add",
            "scratch",
            "--scheme",
            "fs",
            "--root",
            "/",
        ])
        .assert()
        .success();
    sandboxed(&home)
        .args(["connection", "password", "scratch", "--secret-stdin"])
        .write_stdin("x\n")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("does not authenticate"));
}

#[test]
fn a_duplicate_connection_name_is_refused() {
    let home = sandbox();
    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root", "/"])
        .assert()
        .success();
    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root", "/"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}

#[test]
fn a_connection_a_link_uses_cannot_be_removed() {
    let home = sandbox();
    let root = home.path().join("store");
    let source = home.path().join("from");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&source).unwrap();

    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root"])
        .arg(&root)
        .assert()
        .success();
    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("nas:inbox")
        .args(["--name", "drain", "--move"])
        .assert()
        .success();

    sandboxed(&home)
        .args(["connection", "remove", "nas"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("still used by at least one link"));
}

#[test]
fn a_link_end_can_name_a_connection() {
    let home = sandbox();
    let root = home.path().join("store");
    let source = home.path().join("from");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&source).unwrap();

    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root"])
        .arg(&root)
        .assert()
        .success();
    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("nas:inbox/2026")
        .args(["--name", "drain", "--move"])
        .assert()
        .success();

    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nas:inbox/2026"));
}

#[test]
fn a_windows_drive_letter_is_not_read_as_a_connection() {
    // The rule the `name:path` convenience needs: `C:\Users\x` is a path.
    let home = sandbox();
    let source = home.path().join("from");
    std::fs::create_dir_all(&source).unwrap();

    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg(r"C:\Users\x")
        .args(["--name", "drain", "--copy"])
        .assert()
        .success();

    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains(r"C:\Users\x"));
}

#[test]
fn an_unknown_connection_in_a_link_end_is_an_error() {
    let home = sandbox();
    sandboxed(&home)
        .args(["link", "add", "/tmp/from", "unknown:path"])
        .args(["--name", "drain", "--copy"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no connection named `unknown`"));
}

#[test]
fn trashing_originals_is_refused_when_the_source_is_remote() {
    // `trash::delete` is local-only. Refused when the link is created rather
    // than partway through a drain.
    let home = sandbox();
    let root = home.path().join("store");
    std::fs::create_dir_all(&root).unwrap();

    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root"])
        .arg(&root)
        .assert()
        .success();

    sandboxed(&home)
        .args(["link", "add", "nas:inbox", "/tmp/to"])
        .args(["--name", "ingest", "--move", "--trash"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("is remote"))
        .stderr(predicates::str::contains("--copy"));
}

#[test]
fn a_whole_drain_runs_through_a_connection() {
    // The runnable outcome of the slice: files land, sources are gone, and the
    // destination was reached through OpenDAL rather than through std::fs.
    let home = sandbox();
    let source = home.path().join("from");
    let destination = home.path().join("to");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(destination.join("inbox")).unwrap();
    std::fs::write(source.join("holiday.mp4"), b"a video").unwrap();

    sandboxed(&home)
        .args(["connection", "add", "scratch", "--scheme", "fs", "--root"])
        .arg(&destination)
        .assert()
        .success();
    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("scratch:inbox")
        .args(["--name", "viaopendal", "--move", "--cooldown", "0"])
        .assert()
        .success();

    sandboxed(&home)
        .args(["link", "preview", "viaopendal"])
        .assert()
        .success()
        .stdout(predicates::str::contains("holiday.mp4"))
        .stdout(predicates::str::contains("scratch:inbox"));
    assert!(
        source.join("holiday.mp4").exists(),
        "a preview moves nothing"
    );

    sandboxed(&home)
        .args(["link", "run", "viaopendal"])
        .assert()
        .success()
        .stdout(predicates::str::contains("1 transferred"));

    assert_eq!(
        std::fs::read(destination.join("inbox/holiday.mp4")).unwrap(),
        b"a video"
    );
    assert!(
        !source.join("holiday.mp4").exists(),
        "the original should have been reclaimed"
    );
}
