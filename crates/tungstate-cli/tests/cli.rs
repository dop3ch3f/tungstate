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
fn folder_add_refuses_a_path_that_is_not_there() {
    // Was `folder_add_is_a_stub_for_now`, which pinned exit code 2 and "not
    // implemented yet" from slice 0 until slice 7b gave the window something
    // to iterate. The command is real now, so what is worth pinning is that a
    // path it cannot read is refused rather than governed.
    tungstate()
        .args(["folder", "add", "/tmp/definitely-not-a-real-folder-xyzzy"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot read"));
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

// ---------------------------------------------------------------------------
// `explain` and `policy validate`

/// A governed folder with a policy in it and three files worth explaining.
///
/// The photo carries a real EXIF `DateTimeOriginal`, so the trace shows the
/// camera's date winning over the file's modification time — the one fact
/// this whole slice exists to make visible.
fn governed() -> tempfile::TempDir {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(root.join(".tungstate")).unwrap();
    std::fs::write(root.join(".tungstate/policy.toml"), POLICY).unwrap();

    std::fs::write(
        root.join("IMG_0001.JPG"),
        tungstate_attrs::jpeg_with_exif_date("2023:12:25 08:30:00"),
    )
    .unwrap();
    // 12 MiB, so it lands in the middle size bucket rather than the small one.
    let mut clip = vec![0_u8; 12 * 1024 * 1024];
    clip[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x18]);
    clip[4..12].copy_from_slice(b"ftypmp42");
    std::fs::write(root.join("holiday.mp4"), &clip).unwrap();
    std::fs::write(root.join("papers.zip"), b"PK\x03\x04not really a zip").unwrap();
    std::fs::write(root.join(".DS_Store"), b"junk").unwrap();
    home
}

const POLICY: &str = r#"# The two skins on one model: one layout template, one narrow rule.
[folder]
name = "downloads"
inbox = "_inbox"
ignore = [".DS_Store", "*.part"]

[defaults]
on_conflict = "quarantine"
cooldown = "30s"

[[rule]]
name = "media-by-origin"
path = "{date:%Y}/{date:%m}/{category}/{size_range}"
match = { mime = ["image/*", "video/*"] }
vars.date       = { from = ["exif.DateTimeOriginal", "mtime"], tz = "utc" }
vars.category   = { from = "mime", map = { "image/*" = "Photos", "video/*" = "Videos" } }
vars.size_range = { from = "size", bucket = ["<10MiB", "10MiB-1GiB", ">1GiB"] }
rename = "{date:%Y%m%d_%H%M%S}_{hash:8}.{ext|lower}"

[[rule]]
name = "archives"
path = "Archives/{ext|upper}"
match = { ext = ["zip", "tar", "gz", "7z"] }
"#;

fn explain(home: &tempfile::TempDir, file: &str) -> assert_cmd::Command {
    let mut command = sandboxed(home);
    command
        .arg("explain")
        .arg(home.path().join("Downloads").join(file));
    command
}

#[test]
fn explain_finds_the_policy_by_walking_up_and_traces_the_decision() {
    let home = governed();
    let output = explain(&home, "IMG_0001.JPG")
        .output()
        .expect("explain runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace = String::from_utf8(output.stdout).expect("the trace is utf-8");
    insta::assert_snapshot!(trace);
}

#[test]
fn explain_routes_a_photo_by_its_camera_date_not_its_mtime() {
    let home = governed();
    // The file was written seconds ago, so an mtime-derived path would be
    // this year. The EXIF date is 2023, and that is what must win.
    explain(&home, "IMG_0001.JPG")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "destination  2023/12/Photos/under-10MiB/20231225_083000_",
        ))
        .stdout(predicates::str::contains(
            "from exif.DateTimeOriginal (meta)",
        ));
}

#[test]
fn explain_reads_the_other_two_files_the_way_the_policy_says() {
    let home = governed();
    explain(&home, "holiday.mp4")
        .assert()
        .success()
        .stdout(predicates::str::contains("Videos/10MiB-1GiB"))
        // No EXIF in an MP4, so the chain falls through to the file's own time.
        .stdout(predicates::str::contains("exif.DateTimeOriginal absent"))
        .stdout(predicates::str::contains("from mtime (stat)"));

    explain(&home, "papers.zip")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "destination  Archives/ZIP/papers.zip",
        ))
        .stdout(predicates::str::contains("no: mime"));

    explain(&home, ".DS_Store")
        .assert()
        .success()
        .stdout(predicates::str::contains("ignored"))
        .stdout(predicates::str::contains(".DS_Store"));
}

#[test]
fn explain_emits_the_same_trace_as_json() {
    let home = governed();
    let output = explain(&home, "IMG_0001.JPG")
        .arg("--json")
        .output()
        .expect("explain --json runs");
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json emits valid JSON");

    assert_eq!(json["policy"], ".tungstate/policy.toml");
    assert_eq!(json["folder"], "downloads");
    assert_eq!(json["file"], "IMG_0001.JPG");
    assert_eq!(json["tier"], "whole");

    let rules = json["rules"].as_array().expect("rules is a list");
    assert_eq!(rules.len(), 2);
    // The line a rule is reported at is the line its `name` sits on.
    assert_eq!(rules[0]["name"], "media-by-origin");
    assert_eq!(rules[0]["result"]["kind"], "matched");
    assert_eq!(rules[0]["line"], 12);
    assert_eq!(rules[1]["name"], "archives");
    assert_eq!(rules[1]["result"]["kind"], "failed");
    assert_eq!(rules[1]["result"]["constraint"], "ext");
    assert_eq!(rules[1]["line"], 21);

    let outcome = &json["outcome"];
    assert_eq!(outcome["kind"], "routed");
    assert_eq!(outcome["rule"], "media-by-origin");
    assert_eq!(outcome["in_place"], false);
    let destination = outcome["destination"].as_str().expect("a destination");
    assert!(
        destination.starts_with("2023/12/Photos/under-10MiB/20231225_083000_"),
        "{destination}"
    );

    let date = &outcome["vars"][0];
    assert_eq!(date["name"], "date");
    assert_eq!(date["source"], "exif.DateTimeOriginal");
    assert_eq!(date["tier"], "meta");
    assert_eq!(date["tz"], "utc");
    assert_eq!(date["skipped"].as_array().unwrap().len(), 0);
    // ISO-8601, not the camera's own `2023:12:25 08:30:00`: the trace showing
    // a parsed instant is how you can tell tungstate read it as a date rather
    // than as a string it failed to understand.
    assert_eq!(date["raw"], "2023-12-25T08:30:00");

    let category = outcome["vars"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "category")
        .expect("category is traced");
    assert_eq!(category["raw"], "image/jpeg");
    assert_eq!(category["value"], "Photos");
    assert_eq!(category["via"], "map `image/*` = \"Photos\"");

    assert_eq!(
        outcome["path"]["template"],
        "{date:%Y}/{date:%m}/{category}/{size_range}"
    );
    assert_eq!(outcome["path"]["text"], "2023/12/Photos/under-10MiB");
    let rename = &outcome["rename"];
    assert_eq!(rename["steps"][2]["variable"], "ext");
    assert_eq!(rename["steps"][2]["filters"][0]["filter"], "lower");
    assert_eq!(rename["steps"][2]["filters"][0]["result"], "jpg");
    assert_eq!(json["warnings"].as_array().unwrap().len(), 0);
}

#[test]
fn reordering_two_rules_changes_where_a_file_goes() {
    let home = governed();
    let policy = home.path().join("Downloads/.tungstate/policy.toml");

    // A zip is not media, so it starts in `archives`. Give the first rule a
    // reach that covers it, and the first-match-wins order takes it instead.
    let wide = POLICY.replace(
        r#"match = { mime = ["image/*", "video/*"] }"#,
        r#"match = { mime = "*" }"#,
    );
    std::fs::write(&policy, &wide).unwrap();
    explain(&home, "papers.zip")
        .assert()
        .success()
        .stdout(predicates::str::contains("no rule matched").not())
        .stdout(predicates::str::contains("archives"));

    // Put `archives` above it and the zip goes back to the archive shelf,
    // which is the demonstration that file order is the precedence.
    let head = wide.split("[[rule]]").next().unwrap().to_string();
    let rules: Vec<&str> = wide.split("[[rule]]").skip(1).collect();
    let swapped = format!("{head}[[rule]]{}[[rule]]{}", rules[1], rules[0]);
    std::fs::write(&policy, &swapped).unwrap();
    explain(&home, "papers.zip")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "destination  Archives/ZIP/papers.zip",
        ));

    // And back: reverting the reorder reverts the answer.
    std::fs::write(&policy, &wide).unwrap();
    explain(&home, "papers.zip")
        .assert()
        .success()
        .stdout(predicates::str::contains("destination  Archives/ZIP/papers.zip").not());
}

#[test]
fn explain_takes_a_policy_that_is_not_in_the_folder() {
    let home = governed();
    let elsewhere = home.path().join("other.toml");
    std::fs::write(
        &elsewhere,
        "[folder]\nname = \"flat\"\n\n[[rule]]\nname = \"everything\"\npath = \"All/{ext|upper}\"\n",
    )
    .unwrap();

    explain(&home, "papers.zip")
        .arg("--policy")
        .arg(&elsewhere)
        .assert()
        .success()
        .stdout(predicates::str::contains("destination  All/ZIP/papers.zip"));
}

#[test]
fn explain_without_a_policy_anywhere_says_so() {
    let home = sandbox();
    std::fs::write(home.path().join("loose.txt"), b"x").unwrap();
    sandboxed(&home)
        .arg("explain")
        .arg(home.path().join("loose.txt"))
        .assert()
        .failure()
        .stderr(predicates::str::contains(".tungstate/policy.toml"));
}

#[test]
fn policy_validate_accepts_a_good_policy_and_reports_a_bad_one() {
    let home = governed();
    let policy = home.path().join("Downloads/.tungstate/policy.toml");

    sandboxed(&home)
        .args(["policy", "validate", "--policy"])
        .arg(&policy)
        .assert()
        .success()
        .stdout(predicates::str::contains("2 rule(s)"))
        .stdout(predicates::str::contains("reads every file in full"));

    // Four ways to break it, each of which must point at its own line.
    for (broken, line, needle) in [
        ("mode = \"aggressive\"\n", 4, "unknown variant"),
        (
            "[[rule]]\nname = \"z\"\npath = \"{name|shout}\"\n",
            27,
            "not a filter",
        ),
        (
            "[[rule]]\nname = \"z\"\npath = \"{date:%Y\"\n",
            27,
            "never closed",
        ),
        (
            "[[rule]]\nname = \"z\"\npath = \"{year}\"\n",
            27,
            "unknown variable",
        ),
    ] {
        let text = if broken.starts_with("[[rule]]") {
            format!("{POLICY}\n{broken}")
        } else {
            POLICY.replacen("inbox =", &format!("{broken}inbox ="), 1)
        };
        std::fs::write(&policy, &text).unwrap();
        let output = sandboxed(&home)
            .args(["policy", "validate", "--policy"])
            .arg(&policy)
            .output()
            .expect("validate runs");
        assert_eq!(
            output.status.code(),
            Some(1),
            "a broken policy must exit 1: {broken}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "expected `{needle}` in:\n{stderr}");
        assert!(
            stderr.contains(&format!("policy.toml:{line}:")),
            "expected line {line} in:\n{stderr}"
        );
    }
}

#[test]
fn a_shadowed_rule_is_a_warning_not_a_failure() {
    let home = governed();
    let policy = home.path().join("Downloads/.tungstate/policy.toml");
    std::fs::write(
        &policy,
        "[folder]\nname = \"x\"\n\n[[rule]]\nname = \"images\"\npath = \"Images\"\n\
         match = { mime = \"image/*\" }\n\n[[rule]]\nname = \"pngs\"\npath = \"PNG\"\n\
         match = { mime = \"image/png\" }\n",
    )
    .unwrap();

    sandboxed(&home)
        .args(["policy", "validate", "--policy"])
        .arg(&policy)
        .assert()
        // A warning does not fail the load: the policy is usable, and the
        // rule that can never fire is a thing to know, not a thing to stop on.
        .success()
        .stderr(predicates::str::contains("can never fire"))
        .stderr(predicates::str::contains("`pngs`"))
        .stderr(predicates::str::contains("`images`"));
}

#[test]
fn explain_reaches_a_file_through_a_connection() {
    // The `fs` scheme goes through the same OpenDAL factory a NAS does, with
    // no server and no network, so the remote path is exercised on every
    // platform rather than only on the Linux FTP job.
    let home = sandbox();
    let store = home.path().join("store");
    std::fs::create_dir_all(store.join("incoming")).unwrap();
    std::fs::write(
        store.join("incoming/IMG_0002.JPG"),
        tungstate_attrs::jpeg_with_exif_date("2019:07:04 17:05:00"),
    )
    .unwrap();

    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "fs", "--root"])
        .arg(&store)
        .assert()
        .success();

    let policy = home.path().join("nas.toml");
    std::fs::write(
        &policy,
        "[folder]\nname = \"nas\"\n\n[[rule]]\nname = \"photos\"\n\
         path = \"{d:%Y}/{d:%m}/{parent}\"\n\
         vars.d = { from = [\"exif.DateTimeOriginal\", \"mtime\"], tz = \"utc\" }\n",
    )
    .unwrap();

    // The connection's own root stands in for the governed folder, so
    // `{parent}` means what it means locally.
    sandboxed(&home)
        .args(["explain", "nas:incoming/IMG_0002.JPG", "--policy"])
        .arg(&policy)
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "destination  2019/07/incoming/IMG_0002.JPG",
        ))
        .stdout(predicates::str::contains(
            "from exif.DateTimeOriginal (meta)",
        ));

    // Without a policy there is nothing to walk up to on the far side, and
    // saying so beats a confusing "no policy found" about a local directory.
    sandboxed(&home)
        .args(["explain", "nas:incoming/IMG_0002.JPG"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--policy"));

    // A name that reads as a connection and is not one is a typo, not a file.
    sandboxed(&home)
        .args(["explain", "nsa:incoming/IMG_0002.JPG", "--policy"])
        .arg(&policy)
        .assert()
        .failure()
        .stderr(predicates::str::contains("no connection named `nsa`"));
}

#[test]
fn a_networked_connection_saved_without_a_root_says_what_that_means() {
    // The failure this prevents cost a real afternoon: OpenDAL normalises an
    // empty root to `/`, so the drain wrote relative to the NAS's own root
    // directory, every file came back `553 Permission denied`, and nothing in
    // the product mentioned the one field that was wrong.
    let home = sandbox();
    sandboxed(&home)
        .args(["connection", "add", "nas", "--scheme", "ftp"])
        .args(["--host", "nas.local", "--user", "me", "--secret-stdin"])
        .write_stdin("\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("server's own `/`"))
        .stdout(predicates::str::contains("connection test"));

    // Naming a root settles it, whatever the root is: `/` may genuinely be
    // right on a server that chroots the login, and that is the user's call.
    sandboxed(&home)
        .args(["connection", "update", "nas", "--root", "/volume1/media"])
        .assert()
        .success()
        .stdout(predicates::str::contains("server's own `/`").not());

    // A local filesystem connection has no far side to be wrong about.
    sandboxed(&home)
        .args(["connection", "add", "here", "--scheme", "fs", "--root"])
        .arg(home.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("server's own `/`").not());
}

#[test]
fn a_link_can_be_removed_and_its_history_survives() {
    let home = sandbox();
    let source = home.path().join("src");
    let dest = home.path().join("dst");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(source.join("a.mp4"), b"a video").unwrap();

    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg(&dest)
        .args(["--name", "gone", "--move", "--cooldown", "0"])
        .assert()
        .success();

    // Never run, so nothing refers to it and it goes outright.
    sandboxed(&home)
        .args(["link", "remove", "gone"])
        .assert()
        .success()
        .stdout(predicates::str::contains("removed link `gone`"))
        .stdout(predicates::str::contains("record is kept").not());
    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("no links configured"));

    // The same name again, then a run, then removal keeps the history.
    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg(&dest)
        .args(["--name", "gone", "--move", "--cooldown", "0"])
        .assert()
        .success();
    sandboxed(&home)
        .args(["link", "run", "gone"])
        .assert()
        .success();

    sandboxed(&home)
        .args(["link", "remove", "gone"])
        .assert()
        .success()
        .stdout(predicates::str::contains("record is kept"));

    // Gone from the list, and its history still reads.
    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("no links configured"));
    sandboxed(&home)
        .arg("log")
        .arg(dest.join("a.mp4"))
        .assert()
        .success()
        .stdout(predicates::str::contains("ok"));

    sandboxed(&home)
        .args(["link", "remove", "gone"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no link named `gone`"));
}

#[test]
fn storage_can_be_exported_reset_and_brought_back() {
    let home = sandbox();
    let store = home.path().join("store");
    std::fs::create_dir_all(&store).unwrap();
    sandboxed(&home)
        .args(["connection", "add", "keep", "--scheme", "fs", "--root"])
        .arg(&store)
        .assert()
        .success();
    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&store)
        .arg("keep:inbox")
        .args(["--name", "saved", "--copy", "--cooldown", "0"])
        .assert()
        .success();

    // An export is a document, and it carries no passwords: it is a file that
    // gets copied around, and the keychain is where credentials live.
    let doc = home.path().join("export.json");
    sandboxed(&home)
        .args(["storage", "export"])
        .arg(&doc)
        .assert()
        .success()
        .stdout(predicates::str::contains("no passwords"));
    let text = std::fs::read_to_string(&doc).unwrap();
    assert!(text.contains("tungstate_export"), "it names its own format");
    assert!(text.contains("\"saved\""), "and carries the link");
    for forbidden in ["password", "secret"] {
        assert!(!text.contains(forbidden), "`{forbidden}` is in an export");
    }

    // Reset archives first and leaves a clean slate.
    sandboxed(&home)
        .args(["storage", "reset"])
        .assert()
        .success()
        .stdout(predicates::str::contains("archived everything"));
    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("no links configured"));

    // The archive describes itself well enough to choose from.
    let listed = sandboxed(&home)
        .args(["storage", "archives"])
        .output()
        .unwrap();
    let listing = String::from_utf8(listed.stdout).unwrap();
    assert!(listing.contains("1 link(s), 1 connection(s)"), "{listing}");
    let name = listing
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();

    sandboxed(&home)
        .args(["storage", "restore", name])
        .assert()
        .success()
        .stdout(predicates::str::contains("reversible"));
    sandboxed(&home)
        .args(["link", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("saved"));

    // Importing into a journal that already holds something is refused, so
    // there is never a half-merged state nobody can reason about.
    sandboxed(&home)
        .args(["storage", "import"])
        .arg(&doc)
        .assert()
        .failure()
        .stderr(predicates::str::contains("not empty"));
}

#[test]
fn a_reset_keeps_the_history_of_a_run_that_finished() {
    // The refusal while work is unfinished is tested in the journal, where an
    // interrupted operation can be written directly; from out here the only
    // way to make one is to kill a run mid-file. What this covers is the other
    // half: a completed run's history survives a reset, in the archive.
    let home = sandbox();
    let source = home.path().join("src");
    let dest = home.path().join("dst");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(source.join("big.bin"), vec![0_u8; 4096]).unwrap();

    sandboxed(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg(&dest)
        .args(["--name", "done", "--move", "--cooldown", "0"])
        .assert()
        .success();
    sandboxed(&home)
        .args(["link", "run", "done"])
        .assert()
        .success()
        .stdout(predicates::str::contains("1 transferred"));

    sandboxed(&home)
        .args(["storage", "reset"])
        .assert()
        .success();
    // Gone from here...
    sandboxed(&home)
        .arg("log")
        .arg(dest.join("big.bin"))
        .assert()
        .success()
        .stdout(predicates::str::contains("no matching operations"));
    // ...and kept over there.
    let listing = String::from_utf8(
        sandboxed(&home)
            .args(["storage", "archives"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(listing.contains("1 operation(s)"), "{listing}");

    // And the file itself was never in question: a reset moves nothing.
    assert!(
        dest.join("big.bin").is_file(),
        "a reset must not touch files"
    );
}

// --- `tungstate plan` -----------------------------------------------------

fn plan_in(home: &tempfile::TempDir) -> assert_cmd::Command {
    let mut command = sandboxed(home);
    command.arg("plan").arg(home.path().join("Downloads"));
    command
}

/// `governed()` with the cooldown turned off.
///
/// Its 30-second cooldown is right for `explain`, which is about one file and
/// wants the setting exercised; for `plan` it would mean every file in a
/// freshly written fixture reads as "still settling" and no operation is ever
/// visible. `a_file_written_too_recently_waits` covers the cooldown itself.
fn planned() -> tempfile::TempDir {
    let home = governed();
    let policy = home.path().join("Downloads/.tungstate/policy.toml");
    let text = std::fs::read_to_string(&policy).unwrap();
    std::fs::write(
        &policy,
        text.replace("cooldown = \"30s\"", "cooldown = \"0s\""),
    )
    .unwrap();
    home
}

/// Every file under a root, with its size and its modification time.
///
/// What `plan_changes_nothing` compares. Times as well as names, because a
/// command that rewrote a file with identical content would still be a
/// command that touched it.
fn census(root: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("the folder is readable")
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(std::fs::DirEntry::path);
        for entry in entries {
            let meta = entry.metadata().expect("metadata");
            if meta.is_dir() {
                out.push(format!("d {}", entry.path().display()));
                walk(&entry.path(), out);
            } else {
                out.push(format!(
                    "f {} {} {:?}",
                    entry.path().display(),
                    meta.len(),
                    meta.modified().ok()
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

#[test]
fn plan_changes_nothing() {
    // The property the whole command rests on, and the reason it does not
    // write the plan it prints: `tungstate plan` is safe to run on anything.
    let home = governed();
    let root = home.path().join("Downloads");
    let before = census(&root);

    let output = plan_in(&home).output().expect("plan runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(before, census(&root), "`plan` touched the folder");
}

#[test]
fn plan_walks_up_to_the_policy_and_says_what_would_happen() {
    let home = planned();
    let mut command = sandboxed(&home);
    command.arg("plan").arg(home.path().join("Downloads"));
    let output = command.output().expect("plan runs");
    let text = String::from_utf8(output.stdout).expect("utf-8");

    assert!(text.contains("folder \"downloads\""), "{text}");
    assert!(text.contains("Archives/ZIP/papers.zip"), "{text}");
    assert!(
        text.contains("(archives)"),
        "the deciding rule is named: {text}"
    );
    assert!(text.ends_with("Nothing has been changed.\n"), "{text}");
}

#[test]
fn plan_names_the_ignored_file_and_says_it_is_ignored() {
    // Not "written too recently", which is what it said before the cooldown
    // check moved after the decision.
    let home = planned();
    let output = plan_in(&home).output().expect("plan runs");
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains(".DS_Store"), "{text}");
    assert!(text.contains("ignore `.DS_Store`"), "{text}");
}

#[test]
fn plan_as_json_is_valid_and_carries_the_same_decisions() {
    let home = planned();
    let mut command = plan_in(&home);
    let output = command.arg("--json").output().expect("plan runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the plan is JSON");

    assert_eq!(document["folder"], "downloads");
    assert!(document["settles"].as_bool().expect("settles is a bool"));
    let ops = document["ops"].as_array().expect("ops is an array");
    assert!(
        ops.iter().any(|op| op["to"] == "Archives/ZIP/papers.zip"),
        "{ops:#?}"
    );
    assert!(document["snapshot"].as_str().is_some_and(|s| s.len() == 64));
}

#[test]
fn plan_refuses_a_folder_with_no_policy_and_says_what_to_do() {
    let home = sandbox();
    let mut command = sandboxed(&home);
    let output = command.arg("plan").arg(home.path()).output().expect("runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(stderr.contains(".tungstate/policy.toml"), "{stderr}");
}

#[test]
fn plan_refuses_a_connection_rather_than_half_supporting_one() {
    let home = sandbox();
    let mut command = sandboxed(&home);
    command
        .arg("connection")
        .arg("add")
        .arg("nas")
        .arg("--scheme")
        .arg("fs")
        .arg("--root")
        .arg(home.path().to_str().expect("utf-8 path"));
    command.assert().success();

    let mut command = sandboxed(&home);
    let output = command.arg("plan").arg("nas:inbox").output().expect("runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "not-built-yet is exit code 2"
    );
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(stderr.contains("not built yet"), "{stderr}");
}

// --- `tungstate apply` and `tungstate undo` -------------------------------

fn tidy_folder() -> tempfile::TempDir {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(root.join("old/deeper")).unwrap();
    std::fs::create_dir_all(root.join(".tungstate")).unwrap();
    std::fs::write(
        root.join(".tungstate/policy.toml"),
        "[folder]\nname = \"downloads\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = { ext = \"txt\" }\n",
    )
    .unwrap();
    std::fs::write(root.join("a.txt"), b"one").unwrap();
    std::fs::write(root.join("old/b.txt"), b"two").unwrap();
    std::fs::write(root.join("old/deeper/c.txt"), b"three").unwrap();
    std::fs::write(root.join("keep.bin"), b"four").unwrap();
    home
}

fn tungstate_in(home: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    let mut command = sandboxed(home);
    for arg in args {
        command.arg(arg);
    }
    command.arg(home.path().join("Downloads"));
    command.output().expect("the command runs")
}

#[test]
fn apply_tidies_the_folder_and_undo_puts_it_back() {
    let home = tidy_folder();
    let root = home.path().join("Downloads");
    let before = census(&root);

    let applied = tungstate_in(&home, &["apply", "--yes"]);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(root.join("Text/a.txt").exists());
    assert!(!root.join("old").exists(), "the emptied directory is swept");

    // And nothing left to do, on a real filesystem.
    let planned = tungstate_in(&home, &["plan"]);
    let text = String::from_utf8(planned.stdout).expect("utf-8");
    assert!(text.contains("nothing to do"), "{text}");

    let undone = tungstate_in(&home, &["undo"]);
    assert!(
        undone.status.success(),
        "{}",
        String::from_utf8_lossy(&undone.stderr)
    );
    assert_eq!(before, census(&root), "undo did not restore the folder");
}

#[test]
fn apply_refuses_a_large_reorganisation_and_names_the_limit() {
    let home = tidy_folder();
    let output = tungstate_in(&home, &["apply"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(stderr.contains("500-file / 20% limit"), "{stderr}");
    assert!(stderr.contains("--yes"), "{stderr}");
    // Refused before anything moved.
    assert!(home.path().join("Downloads/a.txt").exists());
}

#[test]
fn apply_refuses_a_policy_that_never_settles() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(root.join(".tungstate")).unwrap();
    std::fs::write(
        root.join(".tungstate/policy.toml"),
        "[folder]\nname = \"churn\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"prefix\"\npath = \"\"\nrename = \"copy-{name}\"\n\
         match = { ext = \"txt\" }\n",
    )
    .unwrap();
    std::fs::write(root.join("a.txt"), b"x").unwrap();

    let output = tungstate_in(&home, &["apply"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(stderr.contains("does not settle"), "{stderr}");
    assert!(root.join("a.txt").exists(), "nothing was moved");
}

#[test]
fn a_saved_plan_is_refused_once_the_folder_has_moved_on() {
    let home = tidy_folder();
    let root = home.path().join("Downloads");

    let mut command = sandboxed(&home);
    let saved = command
        .arg("plan")
        .arg(&root)
        .arg("--json")
        .output()
        .expect("plan runs");
    let path = home.path().join("saved.json");
    std::fs::write(&path, &saved.stdout).unwrap();

    std::fs::write(root.join("surprise.txt"), b"new").unwrap();

    let mut command = sandboxed(&home);
    let output = command
        .arg("apply")
        .arg(&root)
        .arg("--plan")
        .arg(&path)
        .arg("--yes")
        .output()
        .expect("apply runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(
        stderr.contains("changed since this plan was made"),
        "{stderr}"
    );
    assert!(root.join("a.txt").exists(), "nothing was moved");
}

#[test]
fn undo_is_refused_twice_for_the_same_reorganisation() {
    let home = tidy_folder();
    assert!(tungstate_in(&home, &["apply", "--yes"]).status.success());
    assert!(tungstate_in(&home, &["undo"]).status.success());

    let again = tungstate_in(&home, &["undo"]);
    let text = String::from_utf8(again.stdout).expect("utf-8");
    assert!(text.contains("nothing to undo"), "{text}");
}

#[test]
fn applying_an_already_tidy_folder_says_so_rather_than_doing_nothing_quietly() {
    let home = tidy_folder();
    assert!(tungstate_in(&home, &["apply", "--yes"]).status.success());
    let again = tungstate_in(&home, &["apply", "--yes"]);
    assert!(again.status.success());
    let text = String::from_utf8(again.stdout).expect("utf-8");
    assert!(text.contains("already matches its policy"), "{text}");
}

// --- `tungstate folder` and `tungstate init` ------------------------------

#[test]
fn a_folder_added_is_a_folder_listed() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();

    let mut command = sandboxed(&home);
    let added = command
        .arg("folder")
        .arg("add")
        .arg(&root)
        .output()
        .expect("runs");
    assert!(added.status.success());

    let mut command = sandboxed(&home);
    let listed = command.arg("folder").arg("list").output().expect("runs");
    let text = String::from_utf8(listed.stdout).expect("utf-8");
    assert!(text.contains("Downloads"), "{text}");
}

#[test]
fn a_folder_with_no_rules_is_told_what_to_do_about_it() {
    // A governed folder with no rules does nothing at all, and the reason is
    // not guessable from anywhere. So it is said, with the choices.
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();

    let mut command = sandboxed(&home);
    let output = command
        .arg("folder")
        .arg("add")
        .arg(&root)
        .output()
        .expect("runs");
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("no rules yet"), "{text}");
    assert!(text.contains("downloads"), "the layouts are listed: {text}");
}

#[test]
fn the_same_folder_cannot_be_governed_twice() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();

    for expected in [true, false] {
        let mut command = sandboxed(&home);
        let output = command
            .arg("folder")
            .arg("add")
            .arg(&root)
            .output()
            .expect("runs");
        assert_eq!(output.status.success(), expected);
    }
}

#[test]
fn init_writes_a_starting_layout_and_says_nothing_has_moved() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.pdf"), b"%PDF-1.4 not really").unwrap();

    let mut command = sandboxed(&home);
    let output = command
        .arg("init")
        .arg("--template")
        .arg("downloads")
        .arg(&root)
        .output()
        .expect("runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join(".tungstate/policy.toml").is_file());
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("nothing has moved"), "{text}");
    // And the file it wrote is one the engine accepts.
    assert!(root.join("a.pdf").exists(), "nothing was moved");
}

#[test]
fn init_never_overwrites_rules_somebody_already_has() {
    // A policy is somebody's work. This command exists to get them started,
    // not to start them over.
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(root.join(".tungstate")).unwrap();
    std::fs::write(root.join(".tungstate/policy.toml"), b"# mine\n").unwrap();

    let mut command = sandboxed(&home);
    let output = command
        .arg("init")
        .arg("--template")
        .arg("downloads")
        .arg(&root)
        .output()
        .expect("runs");
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(root.join(".tungstate/policy.toml")).unwrap(),
        "# mine\n"
    );
}

#[test]
fn init_with_no_template_shows_the_choices() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();

    let mut command = sandboxed(&home);
    let output = command.arg("init").arg(&root).output().expect("runs");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf-8");
    for name in ["downloads", "photos", "documents", "by-type"] {
        assert!(text.contains(name), "`{name}` missing from: {text}");
    }
}

#[test]
fn an_unknown_layout_lists_the_real_ones() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();

    let mut command = sandboxed(&home);
    let output = command
        .arg("init")
        .arg("--template")
        .arg("nonsense")
        .arg(&root)
        .output()
        .expect("runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(stderr.contains("downloads"), "{stderr}");
}

#[test]
fn forgetting_a_folder_keeps_its_rules_on_disk() {
    let home = sandbox();
    let root = home.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();
    let mut command = sandboxed(&home);
    command
        .arg("folder")
        .arg("add")
        .arg(&root)
        .assert()
        .success();
    let mut command = sandboxed(&home);
    command
        .arg("init")
        .arg("--template")
        .arg("by-type")
        .arg(&root)
        .assert()
        .success();

    let mut command = sandboxed(&home);
    let output = command
        .arg("folder")
        .arg("remove")
        .arg(&root)
        .output()
        .expect("runs");
    assert!(output.status.success());
    assert!(
        root.join(".tungstate/policy.toml").is_file(),
        "forgetting a folder is forgetting to watch it, not deleting its rules"
    );
}

/// A folder already organised by hand, in the shape the five-level yardstick
/// describes: year / app / kind / extension / size band.
fn already_organised(root: &std::path::Path) {
    let jpeg = [0xffu8, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    // Backdated to match the directory each sits in. A year directory is only
    // read as a year when the files agree with it -- otherwise `2019` is just
    // somebody's project name -- so a fixture written "now" would be read as a
    // name, correctly, and prove nothing about years.
    for (path, epoch_secs) in [
        ("2024/WhatsApp/Photo/jpg/under-100MB/a.jpg", 1_717_200_000),
        ("2024/WhatsApp/Photo/jpg/under-100MB/b.jpg", 1_717_200_000),
        ("2025/Telegram/Photo/jpg/under-100MB/c.jpg", 1_748_736_000),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().expect("a parent")).unwrap();
        std::fs::write(&file, jpeg).unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(epoch_secs);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .expect("reopen to set the time")
            .set_modified(when)
            .expect("set the modification time");
    }
}

#[test]
fn learning_a_folder_writes_rules_that_leave_it_exactly_as_it_was() {
    let home = sandbox();
    let root = home.path().join("media");
    already_organised(&root);

    sandboxed(&home)
        .args(["folder", "learn"])
        .arg(&root)
        .assert()
        .success()
        // The shape is read back, and nothing is written without `--write`.
        .stdout(predicates::str::contains(
            "year / name / kind / extension / size",
        ))
        .stdout(predicates::str::contains("nothing has been written"));
    assert!(
        !root.join(".tungstate/policy.toml").exists(),
        "learning must not write unless asked"
    );

    sandboxed(&home)
        .args(["folder", "learn", "--write"])
        .arg(&root)
        .assert()
        .success();
    let written = std::fs::read_to_string(root.join(".tungstate/policy.toml")).expect("written");
    assert!(written.contains("WhatsApp"), "{written}");
    assert!(
        !written.contains("inbox"),
        "a learned policy must not sweep what it did not explain:\n{written}"
    );

    // The point of the whole command: those rules describe the folder well
    // enough that they would not touch it.
    sandboxed(&home)
        .args(["plan"])
        .arg(&root)
        .assert()
        .success()
        .stdout(predicates::str::contains("0 file(s) would move"));

    // And it refuses to overwrite rules somebody already has.
    sandboxed(&home)
        .args(["folder", "learn", "--write"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicates::str::contains("already has rules"));
}
