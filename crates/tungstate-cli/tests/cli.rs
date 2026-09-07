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
