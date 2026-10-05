//! The backend over a real OpenSSH server.
//!
//! Gated behind `--features sftp-integration` and pointed at a server by
//! `TUNGSTATE_SFTP_*`. CI runs it on Linux against `atmoz/sftp`. Bring one up
//! with two keys, one locked with the passphrase `open sesame`:
//!
//! ```text
//! ssh-keygen -q -t ed25519 -N '' -f keys/id_ed25519
//! ssh-keygen -q -t ed25519 -N 'open sesame' -f keys/id_locked
//! cat keys/*.pub > keys/authorized
//! docker run -d --name tungstate-sftp -p 2223:22 \
//!   -v "$PWD/keys/authorized:/home/tungstate/.ssh/keys/authorized.pub:ro" \
//!   atmoz/sftp tungstate:s3cret:1000::media
//! ```
#![cfg(feature = "sftp-integration")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tungstate_backend::{Backend, BackendError};
use tungstate_backend_sftp::{Auth, Settings, SftpBackend};

fn var(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn keys() -> PathBuf {
    PathBuf::from(var("TUNGSTATE_SFTP_KEYS", "/tmp/tungstate-sftp-keys"))
}

fn settings(root: &str) -> Settings {
    Settings {
        host: var("TUNGSTATE_SFTP_HOST", "127.0.0.1"),
        port: var("TUNGSTATE_SFTP_PORT", "2223").parse().ok(),
        root: root.to_string(),
        username: var("TUNGSTATE_SFTP_USER", "tungstate"),
        auth: Auth::Password(var("TUNGSTATE_SFTP_PASSWORD", "s3cret")),
        host_key: None,
    }
}

/// The server's key, learnt the way a person would: by being refused once.
fn server_key() -> String {
    match SftpBackend::connect(&settings("media"), Path::new(""), "nas") {
        Err(BackendError::HostUnknown { key, .. }) => key,
        other => panic!("a first connection must ask about the server: {other:?}"),
    }
}

fn trusted(root: &str) -> Settings {
    Settings {
        host_key: Some(server_key()),
        ..settings(root)
    }
}

/// A folder of this test's own inside `media`, and a backend rooted at it.
fn own(label: &str) -> SftpBackend {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let folder = format!("{label}-{stamp}");
    let media = SftpBackend::connect(&trusted("media"), Path::new(""), "nas").expect("media");
    media
        .create_dir_all(Path::new(&folder))
        .expect("own folder");
    SftpBackend::connect(&trusted(&format!("media/{folder}")), Path::new(""), "nas")
        .expect("rooted")
}

fn put(backend: &SftpBackend, path: &str, bytes: &[u8]) {
    let mut sink = backend.create_write(Path::new(path)).expect("create");
    // A megabyte at a time, as the engine sends.
    for piece in bytes.chunks(1024 * 1024) {
        sink.write_all(piece).expect("write");
    }
    sink.finish().expect("finish");
}

fn get(backend: &SftpBackend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend
        .open_read(Path::new(path))
        .expect("open")
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

#[test]
fn a_server_not_trusted_yet_is_refused_with_its_fingerprint() {
    match SftpBackend::connect(&settings("media"), Path::new(""), "nas") {
        Err(BackendError::HostUnknown {
            fingerprint, key, ..
        }) => {
            assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
            assert!(key.starts_with("ssh-"), "{key}");
        }
        other => panic!("expected the server to need trusting: {other:?}"),
    }
}

#[test]
fn a_server_whose_key_changed_is_refused_and_says_so() {
    // A real key, but not this server's.
    let someone_else =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    let wrong = Settings {
        host_key: Some(someone_else.to_string()),
        ..settings("media")
    };
    match SftpBackend::connect(&wrong, Path::new(""), "nas") {
        Err(BackendError::HostKeyChanged { fingerprint, .. }) => {
            assert!(fingerprint.starts_with("SHA256:"));
        }
        other => panic!("expected a changed key to stop it: {other:?}"),
    }
}

#[test]
fn a_wrong_password_says_so() {
    let wrong = Settings {
        auth: Auth::Password("not-it".into()),
        ..trusted("media")
    };
    assert!(matches!(
        SftpBackend::connect(&wrong, Path::new(""), "nas"),
        Err(BackendError::Auth { .. })
    ));
}

#[test]
fn a_key_signs_in_with_or_without_a_passphrase() {
    let plain = Settings {
        auth: Auth::Key {
            path: keys().join("id_ed25519"),
            passphrase: None,
        },
        ..trusted("media")
    };
    SftpBackend::connect(&plain, Path::new(""), "nas").expect("an unlocked key");

    let locked = Settings {
        auth: Auth::Key {
            path: keys().join("id_locked"),
            passphrase: Some("open sesame".into()),
        },
        ..trusted("media")
    };
    SftpBackend::connect(&locked, Path::new(""), "nas").expect("a key and its passphrase");
}

#[test]
fn a_file_much_larger_than_one_request_round_trips_intact() {
    let backend = own("big");
    let body: Vec<u8> = (0..5 * 1024 * 1024 + 123_u32)
        .map(|i| (i % 251) as u8)
        .collect();
    backend.create_dir_all(Path::new("2024/trip")).unwrap();
    put(&backend, "2024/trip/clip.mp4", &body);

    assert_eq!(get(&backend, "2024/trip/clip.mp4"), body);
    assert_eq!(
        backend.stat(Path::new("2024/trip/clip.mp4")).unwrap().len,
        body.len() as u64
    );
    assert_eq!(
        backend
            .read_range(Path::new("2024/trip/clip.mp4"), 3_000_001, 70_000)
            .unwrap(),
        body[3_000_001..3_070_001]
    );
    // A range running past the end is short, not an error.
    let tail = backend
        .read_range(Path::new("2024/trip/clip.mp4"), body.len() as u64 - 10, 100)
        .unwrap();
    assert_eq!(tail, body[body.len() - 10..]);
}

#[test]
fn a_listing_says_what_a_stat_would() {
    let backend = own("complete");
    backend.create_dir_all(Path::new("2024")).unwrap();
    put(&backend, "a.jpg", b"a");
    put(&backend, "2024/b.mp4", b"bb");

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
    assert!(
        backend.capabilities().atomic_rename,
        "OpenSSH offers posix-rename"
    );
    put(&backend, "new.part", b"new");
    put(&backend, "film.mp4", b"old");
    backend
        .rename(Path::new("new.part"), Path::new("film.mp4"))
        .unwrap();
    assert_eq!(get(&backend, "film.mp4"), b"new");
    assert!(backend.stat(Path::new("new.part")).is_err());
}

#[test]
fn removing_takes_files_and_empty_folders_away() {
    let backend = own("remove");
    backend.create_dir_all(Path::new("full")).unwrap();
    put(&backend, "full/a.txt", b"a");
    assert!(backend.remove_dir(Path::new("full")).is_err(), "not empty");
    backend.remove_file(Path::new("full/a.txt")).unwrap();
    backend.remove_dir(Path::new("full")).unwrap();
    let missing = backend.stat(Path::new("full")).unwrap_err();
    assert!(
        matches!(&missing, BackendError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound),
        "{missing:?}"
    );
}

#[test]
fn a_modification_time_is_set_and_read_back() {
    let backend = own("time");
    put(&backend, "a.txt", b"a");
    let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    assert!(backend.set_modified(Path::new("a.txt"), then).unwrap());
    assert_eq!(
        backend.stat(Path::new("a.txt")).unwrap().modified,
        Some(then)
    );
}

#[test]
fn a_link_end_not_made_yet_is_empty_and_a_missing_folder_is_unreachable() {
    let media = SftpBackend::connect(&trusted("media"), Path::new("not-yet"), "nas").unwrap();
    assert_eq!(
        media.read_dir(Path::new("")).unwrap(),
        [] as [tungstate_backend::Entry; 0]
    );
    assert!(matches!(
        SftpBackend::connect(&trusted("media/nowhere-at-all"), Path::new(""), "nas"),
        Err(BackendError::RootUnreachable(_))
    ));
}

#[test]
fn a_path_cannot_climb_out_of_the_folder() {
    let backend = own("escape");
    assert!(matches!(
        backend.stat(Path::new("../../etc/passwd")),
        Err(BackendError::PathEscapesRoot(_))
    ));
}

#[test]
fn workers_making_the_same_folders_at_once_all_succeed() {
    // The engine's workers each make the folders their file needs, and two
    // files in one new folder ask for it at the same moment.
    let backend = own("race");
    std::thread::scope(|scope| {
        let made: Vec<_> = (0..6)
            .map(|_| scope.spawn(|| backend.create_dir_all(Path::new("2024/trip/day1"))))
            .collect();
        for one in made {
            one.join()
                .unwrap()
                .expect("every worker finds or makes the folder");
        }
    });
    assert!(backend.stat(Path::new("2024/trip/day1")).unwrap().is_dir);
}
