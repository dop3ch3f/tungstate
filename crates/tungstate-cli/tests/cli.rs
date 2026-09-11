//! End-to-end tests that run the built `tungstate` binary.

use assert_cmd::Command;

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
