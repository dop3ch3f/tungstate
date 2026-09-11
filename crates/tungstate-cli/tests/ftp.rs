//! The drain, over a real FTP server.
//!
//! Gated behind `--features ftp-integration` and pointed at a server by
//! `TUNGSTATE_FTP_*`. Nothing else in the suite needs a network, and CI runs
//! this on Linux only; the engine's no-rename behaviour is covered on all
//! three platforms by a fake backend in `tungstate-transfer`.
//!
//! Bring a server up with:
//!
//! ```text
//! docker run -d --name tungstate-ftp -p 2121:21 -p 30000-30009:30000-30009 \
//!   -e USERS="tungstate|s3cret|/ftp/tungstate|10000" \
//!   -e ADDRESS=127.0.0.1 -e MIN_PORT=30000 -e MAX_PORT=30009 \
//!   delfer/alpine-ftp-server
//! ```
#![cfg(feature = "ftp-integration")]

use assert_cmd::Command;

struct Server {
    host: String,
    port: String,
    user: String,
    password: String,
    /// The directory on the server that link paths hang off.
    ///
    /// Not `/`. `OpenDAL` does a `CWD` to the connection root, and on a server where
    /// the login lands in `/ftp/<user>` a root of `/` is the machine's real
    /// root, which the account cannot write to. Getting this wrong is the
    /// first mistake anyone will make, so the default here is the right
    /// answer for the documented container.
    root: String,
}

/// Deliberately a panic rather than a skip. Asking for this feature is asking
/// for these tests to run; quietly passing without a server would be a lie.
fn server() -> Server {
    let var =
        |name: &str, fallback: &str| std::env::var(name).unwrap_or_else(|_| fallback.to_string());
    Server {
        host: var("TUNGSTATE_FTP_HOST", "127.0.0.1"),
        port: var("TUNGSTATE_FTP_PORT", "2121"),
        user: var("TUNGSTATE_FTP_USER", "tungstate"),
        password: var("TUNGSTATE_FTP_PASSWORD", "s3cret"),
        root: var("TUNGSTATE_FTP_ROOT", "/ftp/tungstate"),
    }
}

/// A command with its own journal, and its password passed the way a headless
/// machine would pass one.
///
/// `TUNGSTATE_SECRETS=memory` alone is not enough: it is per-process, so what
/// `connection add` stored would be gone by the time `connection test` ran.
/// `TUNGSTATE_SECRET_<NAME>` survives because the parent sets it on each
/// child, which is also exactly how a NAS daemon or a container gets one.
fn tungstate(home: &tempfile::TempDir, password: &str) -> Command {
    let mut command =
        Command::cargo_bin("tungstate").expect("binary `tungstate` should be built by cargo test");
    command
        .env("TUNGSTATE_JOURNAL", home.path().join("journal.db"))
        .env("TUNGSTATE_SECRETS", "memory")
        .env("TUNGSTATE_SECRET_NAS", password);
    command
}

/// The common case: the server's real password.
fn cli(home: &tempfile::TempDir) -> Command {
    tungstate(home, &server().password)
}

/// Add a connection whose root is a directory of this test's own, so tests
/// sharing one server stay parallel-safe.
fn connect(home: &tempfile::TempDir, password: &str) {
    let server = server();
    tungstate(home, password)
        .args(["connection", "add", "nas", "--scheme", "ftp"])
        .args(["--host", &server.host, "--port", &server.port])
        .args(["--user", &server.user, "--root", &server.root])
        .arg("--secret-stdin")
        .write_stdin("\n")
        .assert()
        .success();
}

/// A link end nothing else in the suite will use.
///
/// The uniqueness is in the *link end*, not the connection root, for two
/// reasons. It matches reality — a NAS's `/volume1/media` exists and the
/// folder you drain into may not — and it exercises the rule slice 4b
/// established: the connection must be there, a folder inside it need not be.
fn unique_end(label: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{label}-{stamp}")
}

#[test]
fn a_connection_to_a_real_server_is_reachable() {
    let home = tempfile::tempdir().unwrap();
    connect(&home, &server().password);

    cli(&home)
        .args(["connection", "test", "nas"])
        .assert()
        .success()
        .stdout(predicates::str::contains("is reachable"));
}

#[test]
fn a_wrong_password_says_so_rather_than_blaming_the_network() {
    // The most common FTP failure by far. "not reachable" would send the user
    // looking at their router.
    let home = tempfile::tempdir().unwrap();
    connect(&home, "definitely-not-the-password");

    tungstate(&home, "definitely-not-the-password")
        .args(["connection", "test", "nas"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("credentials"));
}

#[test]
fn a_whole_drain_lands_over_ftp_and_reclaims_the_source() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("src");
    std::fs::create_dir_all(source.join("2024")).unwrap();

    // Larger than the engine's 1 MiB chunk, so the streaming read and the
    // chunked write are both genuinely exercised over the wire.
    let big: Vec<u8> = (0..2_500_000_u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(source.join("big.bin"), &big).unwrap();
    std::fs::write(source.join("note.txt"), b"hello").unwrap();
    std::fs::write(source.join("2024/trip.txt"), b"trip").unwrap();

    let end = unique_end("drain");
    connect(&home, &server().password);

    cli(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg(format!("nas:{end}"))
        .args([
            "--name",
            "over-ftp",
            "--move",
            "--verify",
            "readback",
            "--cooldown",
            "0",
        ])
        .assert()
        .success();

    cli(&home)
        .args(["link", "run", "over-ftp"])
        .assert()
        .success()
        .stdout(predicates::str::contains("3 transferred"));

    // Read back through a second, independent connection rather than through
    // the one that wrote, so this checks the server rather than our cache.
    let landed = fetch(&format!("{end}/big.bin"));
    assert_eq!(landed.len(), big.len(), "the large file must be complete");
    assert_eq!(landed, big, "and byte-identical");
    assert_eq!(fetch(&format!("{end}/2024/trip.txt")), b"trip");

    assert!(!source.join("big.bin").exists(), "source must be reclaimed");
    assert!(!source.join("2024/trip.txt").exists());
}

#[test]
fn an_interrupted_drain_over_ftp_resumes_and_sweeps() {
    // Over FTP the partial can only be under the real name, and OpenDAL's own
    // temporary file is randomly named, so both have to be cleaned up by the
    // next run or the destination accumulates junk forever.
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("holiday.mp4"), b"was in flight").unwrap();

    let end = unique_end("resume");
    connect(&home, &server().password);
    cli(&home)
        .arg("link")
        .arg("add")
        .arg(&source)
        .arg(format!("nas:{end}"))
        .args(["--name", "over-ftp", "--move", "--cooldown", "0"])
        .assert()
        .success();

    // Stage exactly what a crash mid-file leaves behind.
    put(&format!("{end}/holiday.mp4"), b"half a fi");
    put(&format!("{end}/holiday.mp4.k3xq9wpz"), b"half a fi");
    put(&format!("{end}/keep-me.txt"), b"not ours");
    begin_interrupted_op(&home, "over-ftp");

    cli(&home)
        .args(["link", "run", "over-ftp"])
        .assert()
        .success()
        .stdout(predicates::str::contains("re-queued"));

    assert_eq!(
        fetch(&format!("{end}/holiday.mp4")),
        b"was in flight",
        "the file must be replaced, not appended to"
    );
    assert_eq!(
        fetch(&format!("{end}/keep-me.txt")),
        b"not ours",
        "the sweep must not touch a file that is not a temporary"
    );
    assert!(
        !exists(&format!("{end}/holiday.mp4.k3xq9wpz")),
        "the abandoned temporary must be swept"
    );
    assert!(!source.join("holiday.mp4").exists());
}

#[test]
fn replace_is_refused_when_the_link_is_created() {
    // FTP cannot rename through OpenDAL, so the existing file cannot be moved
    // aside. Better to say so now than halfway through a drain.
    let home = tempfile::tempdir().unwrap();
    connect(&home, &server().password);

    cli(&home)
        .args(["link", "add", "/tmp/from", "nas:inbox"])
        .args(["--name", "x", "--copy", "--on-conflict", "replace"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "needs a destination that can rename",
        ));
}

#[test]
fn an_ftp_destination_is_told_that_hash_only_proves_what_was_sent() {
    let home = tempfile::tempdir().unwrap();
    connect(&home, &server().password);

    cli(&home)
        .args(["link", "add", "/tmp/from", "nas:inbox"])
        .args(["--name", "x", "--move"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--verify readback"));
}

// --- Talking to the server directly, so assertions do not go through the code
// --- under test. A tiny hand-rolled client is enough for STOR/RETR/DELE.

mod raw {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    pub struct Ftp {
        control: BufReader<TcpStream>,
    }

    /// Nothing here waits forever. A wedged data connection should fail a test in
    /// seconds, not hang the suite until someone notices.
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    impl Drop for Ftp {
        fn drop(&mut self) {
            // Servers cap concurrent logins; a suite that leaks control
            // connections starts failing for reasons that have nothing to do
            // with the code under test.
            let _ = self.control.get_mut().write_all(b"QUIT\r\n");
        }
    }

    impl Ftp {
        pub fn connect(host: &str, port: u16, user: &str, password: &str) -> Self {
            let stream = TcpStream::connect((host, port)).expect("ftp control connection");
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TIMEOUT)).unwrap();
            let mut ftp = Self {
                control: BufReader::new(stream),
            };
            ftp.read_reply();
            ftp.command(&format!("USER {user}"));
            ftp.command(&format!("PASS {password}"));
            ftp.command("TYPE I");
            ftp
        }

        fn read_reply(&mut self) -> String {
            // A multi-line reply repeats its code with a hyphen until the last
            // line, which repeats it with a space.
            let mut first = String::new();
            self.control.read_line(&mut first).expect("ftp reply");
            let code = first.chars().take(3).collect::<String>();
            if first.chars().nth(3) == Some('-') {
                loop {
                    let mut line = String::new();
                    self.control.read_line(&mut line).expect("ftp reply");
                    if line.starts_with(&code) && line.chars().nth(3) == Some(' ') {
                        break;
                    }
                }
            }
            first
        }

        pub fn command(&mut self, line: &str) -> String {
            self.control
                .get_mut()
                .write_all(format!("{line}\r\n").as_bytes())
                .expect("ftp command");
            self.read_reply()
        }

        /// Open a passive data connection for the next transfer command.
        fn passive(&mut self) -> TcpStream {
            let reply = self.command("PASV");
            let inside = reply
                .split(['(', ')'])
                .nth(1)
                .unwrap_or_else(|| panic!("no address in PASV reply `{reply}`"));
            let parts: Vec<u16> = inside
                .split(',')
                .map(|p| p.trim().parse().expect("pasv octet"))
                .collect();
            let host = format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3]);
            let port = parts[4] * 256 + parts[5];
            let data = TcpStream::connect((host.as_str(), port)).expect("ftp data connection");
            data.set_read_timeout(Some(TIMEOUT)).unwrap();
            data.set_write_timeout(Some(TIMEOUT)).unwrap();
            data
        }

        pub fn store(&mut self, path: &str, bytes: &[u8]) {
            for parent in parents(path) {
                self.command(&format!("MKD {parent}"));
            }
            let mut data = self.passive();
            let opened = self.command(&format!("STOR {path}"));
            assert!(opened.starts_with('1'), "STOR {path} was refused: {opened}");
            data.write_all(bytes).expect("ftp store");
            drop(data);
            let done = self.read_reply();
            assert!(done.starts_with('2'), "STOR {path} did not finish: {done}");
        }

        pub fn retrieve(&mut self, path: &str) -> Option<Vec<u8>> {
            let mut data = self.passive();
            let reply = self.command(&format!("RETR {path}"));
            if reply.starts_with('5') {
                // Drop the data connection we opened and never used, or the
                // server's passive port range runs out partway through a run.
                drop(data);
                return None;
            }
            let mut bytes = Vec::new();
            data.read_to_end(&mut bytes).expect("ftp retrieve");
            self.read_reply();
            Some(bytes)
        }
    }

    /// Every ancestor directory of `path`, outermost first.
    fn parents(path: &str) -> Vec<String> {
        let mut built = String::new();
        let mut out = Vec::new();
        let segments: Vec<&str> = path.split('/').collect();
        for segment in &segments[..segments.len().saturating_sub(1)] {
            if segment.is_empty() {
                continue;
            }
            if !built.is_empty() {
                built.push('/');
            }
            built.push_str(segment);
            out.push(built.clone());
        }
        out
    }
}

fn client() -> raw::Ftp {
    let server = server();
    raw::Ftp::connect(
        &server.host,
        server.port.parse().expect("port"),
        &server.user,
        &server.password,
    )
}

fn put(path: &str, bytes: &[u8]) {
    client().store(path, bytes);
}

fn fetch(path: &str) -> Vec<u8> {
    client()
        .retrieve(path)
        .unwrap_or_else(|| panic!("`{path}` is not on the server"))
}

fn exists(path: &str) -> bool {
    client().retrieve(path).is_some()
}

/// Write an `intended` operation with no outcome, which is what a crash leaves.
fn begin_interrupted_op(home: &tempfile::TempDir, link: &str) {
    use tungstate_journal::{Journal, Location, NewOp, OpKind};

    let journal = Journal::open(&home.path().join("journal.db")).expect("journal");
    let found = journal.link_by_name(link).expect("link");
    journal
        .begin(&NewOp {
            kind: OpKind::Move,
            source: Some(Location::within(&found.source, "holiday.mp4")),
            destination: Some(Location::within(&found.destination, "holiday.mp4")),
            size: Some(13),
            link: Some(found.name.clone()),
            link_id: Some(found.id),
        })
        .expect("intended op");
}
