//! The drain and a sync, over a real S3 service.
//!
//! Gated behind `--features s3-integration` and pointed at a service by
//! `TUNGSTATE_S3_*`. CI runs it on Linux against `SeaweedFS`; an object store
//! has no rename, so this is the engine's no-rename path over a second
//! protocol.
//!
//! Bring a service up with (`s3.json` holding one identity, `tungstate` /
//! `s3cret-s3cret`, allowed Admin, Read, Write and List):
//!
//! ```text
//! docker run -d --name tungstate-s3 -p 9000:8333 -v "$PWD/s3.json:/etc/s3.json:ro" \
//!   chrislusf/seaweedfs server -s3 -s3.config=/etc/s3.json -dir=/data
//! docker exec tungstate-s3 sh -c "echo 's3.bucket.create -name tungstate' | weed shell"
//! ```
#![cfg(feature = "s3-integration")]

use assert_cmd::Command;

struct Service {
    endpoint: String,
    bucket: String,
    key: String,
    secret: String,
}

/// A panic rather than a skip, as in the FTP suite: asking for this feature
/// is asking for these tests to run.
fn service() -> Service {
    let var =
        |name: &str, fallback: &str| std::env::var(name).unwrap_or_else(|_| fallback.to_string());
    Service {
        endpoint: var("TUNGSTATE_S3_ENDPOINT", "http://127.0.0.1:9000"),
        bucket: var("TUNGSTATE_S3_BUCKET", "tungstate"),
        key: var("TUNGSTATE_S3_KEY", "tungstate"),
        secret: var("TUNGSTATE_S3_SECRET", "s3cret-s3cret"),
    }
}

/// A command with its own journal, the secret passed as a headless machine
/// would pass one.
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
    tungstate(home, &service().secret)
}

/// A connection rooted at a folder of this test's own, so tests sharing one
/// bucket stay parallel-safe.
fn connect(home: &tempfile::TempDir, secret: &str, root: &str) {
    let service = service();
    tungstate(home, secret)
        .args(["connection", "add", "nas", "--scheme", "s3"])
        .args(["--endpoint", &service.endpoint, "--bucket", &service.bucket])
        .args(["--user", &service.key, "--root", root])
        .arg("--secret-stdin")
        .write_stdin("\n")
        .assert()
        .success();
}

fn unique(label: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{label}-{stamp}")
}

/// A second client that shares nothing with the one under test, so a check
/// reads the service rather than our own cache.
mod raw {
    use std::sync::LazyLock;

    use opendal::Operator;

    static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    });

    fn bucket() -> Operator {
        // This process's own HTTP client: the binary under test is another
        // process with its own. reqwest's features are the workspace's, so it
        // wants a TLS provider even for plain http, the same one the app uses.
        static CLIENT: std::sync::Once = std::sync::Once::new();
        CLIENT.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let client = reqwest::Client::builder().build().expect("http client");
            opendal::HttpTransporter::install_default(
                opendal_http_transport_reqwest::ReqwestTransport::new(client),
            );
        });
        let service = super::service();
        let builder = opendal::services::S3::default()
            .endpoint(&service.endpoint)
            .bucket(&service.bucket)
            .region("us-east-1")
            .access_key_id(&service.key)
            .secret_access_key(&service.secret)
            .disable_config_load()
            .disable_ec2_metadata();
        Operator::new(builder).expect("operator")
    }

    pub fn fetch(key: &str) -> Option<Vec<u8>> {
        RUNTIME
            .block_on(bucket().read(key))
            .ok()
            .map(|buffer| buffer.to_vec())
    }

    pub fn put(key: &str, bytes: &[u8]) {
        RUNTIME
            .block_on(bucket().write(key, bytes.to_vec()))
            .expect("write");
    }
}

fn fetch(key: &str) -> Vec<u8> {
    raw::fetch(key).unwrap_or_else(|| panic!("`{key}` is not in the bucket"))
}

#[test]
fn a_bucket_is_reachable_and_accepts_files() {
    let home = tempfile::tempdir().unwrap();
    connect(&home, &service().secret, &unique("probe"));

    cli(&home)
        .args(["connection", "test", "nas"])
        .assert()
        .success()
        .stdout(predicates::str::contains("is reachable"))
        .stdout(predicates::str::contains("accepts files"));
}

#[test]
fn a_wrong_secret_key_says_so_rather_than_blaming_the_network() {
    let home = tempfile::tempdir().unwrap();
    connect(&home, "definitely-not-the-secret", &unique("wrong"));

    tungstate(&home, "definitely-not-the-secret")
        .args(["connection", "test", "nas"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("credentials"));
}

#[test]
fn a_whole_drain_lands_in_a_bucket_and_reclaims_the_source() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("src");
    std::fs::create_dir_all(source.join("2024")).unwrap();

    // Several S3 parts' worth. The engine writes a megabyte at a time and S3
    // refuses a part under 5 MiB that is not the last; OpenDAL gathers the
    // writes into parts itself, and this proves it does.
    let big: Vec<u8> = (0..20_000_000_u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(source.join("big.bin"), &big).unwrap();
    std::fs::write(source.join("note.txt"), b"hello").unwrap();
    std::fs::write(source.join("2024/trip.txt"), b"trip").unwrap();

    let root = unique("drain");
    connect(&home, &service().secret, &root);
    cli(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("nas:inbox")
        .args([
            "--name",
            "to-bucket",
            "--move",
            "--verify",
            "readback",
            "--cooldown",
            "0",
        ])
        .assert()
        .success();

    cli(&home)
        .args(["link", "run", "to-bucket"])
        .assert()
        .success()
        .stdout(predicates::str::contains("3 transferred"));

    let landed = fetch(&format!("{root}/inbox/big.bin"));
    assert_eq!(landed.len(), big.len(), "the large file must be complete");
    assert_eq!(landed, big, "and byte-identical");
    assert_eq!(fetch(&format!("{root}/inbox/2024/trip.txt")), b"trip");
    assert!(!source.join("big.bin").exists(), "source must be reclaimed");
    assert!(!source.join("2024/trip.txt").exists());
}

#[test]
fn a_file_already_in_the_bucket_is_recognised_rather_than_sent_again() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("same.mp4"), b"the very same bytes").unwrap();
    let root = unique("present");
    raw::put(&format!("{root}/same.mp4"), b"the very same bytes");

    connect(&home, &service().secret, &root);
    cli(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg("nas:")
        .args(["--name", "to-bucket", "--copy", "--cooldown", "0"])
        .assert()
        .success();
    cli(&home)
        .args(["link", "run", "to-bucket"])
        .assert()
        .success()
        .stdout(predicates::str::contains("0 transferred"))
        .stdout(predicates::str::contains("1 already there"));
}

#[test]
fn replace_is_refused_when_the_link_is_created() {
    // An object store has no rename, so the file already there cannot be
    // moved aside first. Better said now than halfway through a drain.
    let home = tempfile::tempdir().unwrap();
    connect(&home, &service().secret, &unique("replace"));

    cli(&home)
        .args(["link", "add", "/tmp/from", "nas:inbox"])
        .args(["--name", "x", "--copy", "--on-conflict", "replace"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "needs a destination that can rename",
        ));
}

/// A sync between a folder here and a fresh folder in the bucket.
fn bucket_sync(
    home: &tempfile::TempDir,
    label: &str,
    extra: &[&str],
) -> (std::path::PathBuf, String) {
    let laptop = home.path().join("laptop");
    std::fs::create_dir_all(&laptop).unwrap();
    std::fs::write(laptop.join("a.mp4"), b"version one").unwrap();
    std::fs::write(laptop.join("b.mp4"), b"stays").unwrap();
    let root = unique(label);
    connect(home, &service().secret, &root);
    cli(home)
        .args(["sync", "add", "capcut"])
        .arg(&laptop)
        .arg("nas:capcut")
        .args(["--all", "--cooldown", "0"])
        .args(extra)
        .assert()
        .success();
    cli(home)
        .args(["sync", "run", "capcut", "--yes"])
        .assert()
        .success();
    assert_eq!(fetch(&format!("{root}/capcut/a.mp4")), b"version one");
    (laptop, root)
}

#[test]
fn a_sync_with_a_bucket_runs_and_a_second_run_moves_nothing() {
    let home = tempfile::tempdir().unwrap();
    let (laptop, root) = bucket_sync(&home, "sync-twice", &[]);

    std::fs::write(laptop.join("c.mp4"), b"brand new").unwrap();
    cli(&home)
        .args(["sync", "run", "capcut", "--yes"])
        .assert()
        .success();
    assert_eq!(fetch(&format!("{root}/capcut/c.mp4")), b"brand new");

    cli(&home)
        .args(["sync", "preview", "capcut"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing to do"));
}

#[test]
fn an_exact_sync_deletes_from_a_bucket_when_told_to() {
    let home = tempfile::tempdir().unwrap();
    let (laptop, root) = bucket_sync(&home, "sync-delete", &["--exact", "--on-remove", "delete"]);

    std::fs::remove_file(laptop.join("a.mp4")).unwrap();
    cli(&home)
        .args(["sync", "run", "capcut", "--yes"])
        .assert()
        .success()
        .stdout(predicates::str::contains("cannot be undone"));

    assert!(raw::fetch(&format!("{root}/capcut/a.mp4")).is_none());
    assert_eq!(fetch(&format!("{root}/capcut/b.mp4")), b"stays");
}
