//! What can be said without a server. The protocol itself is proven against
//! Samba in `tests/samba.rs`.

use super::*;

#[test]
fn settings_never_print_the_password() {
    let settings = Settings {
        host: "nas.local".into(),
        port: None,
        root: "media".into(),
        username: "me".into(),
        password: "hunter2".into(),
        require_encryption: false,
    };
    let printed = format!("{settings:?}");
    assert!(printed.contains("nas.local"), "{printed}");
    assert!(!printed.contains("hunter2"), "{printed}");
}

#[test]
fn a_root_without_a_share_is_refused_before_any_network_call() {
    let settings = Settings {
        // Nothing listens here, so reaching the network would fail
        // differently and much more slowly.
        host: "192.0.2.1".into(),
        port: Some(9),
        root: "  /  ".into(),
        username: "me".into(),
        password: String::new(),
        require_encryption: false,
    };
    assert!(matches!(
        SmbBackend::connect(&settings, Path::new(""), "nas"),
        Err(BackendError::RootUnreachable(_))
    ));
}

#[test]
fn a_link_end_that_would_leave_the_folder_is_refused_before_any_network_call() {
    let settings = Settings {
        host: "192.0.2.1".into(),
        port: Some(9),
        root: "media".into(),
        username: "me".into(),
        password: String::new(),
        require_encryption: false,
    };
    assert!(matches!(
        SmbBackend::connect(&settings, Path::new("../other-share"), "nas"),
        Err(BackendError::PathEscapesRoot(_))
    ));
}

#[test]
fn a_time_becomes_the_ticks_smb_writes() {
    // The Unix epoch is 1601 plus 11,644,473,600 seconds, in tenths of a
    // microsecond.
    assert_eq!(
        windows_ticks(SystemTime::UNIX_EPOCH),
        Some(116_444_736_000_000_000)
    );
    let later = SystemTime::UNIX_EPOCH + Duration::new(1, 500);
    assert_eq!(windows_ticks(later), Some(116_444_736_010_000_005));
    // And back again, through the library's own reading of it.
    let round: SystemTime =
        smb::binrw_util::prelude::FileTime::from(windows_ticks(later).unwrap()).into();
    assert_eq!(
        round
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros(),
        1_000_000
    );
    assert_eq!(
        windows_ticks(SystemTime::UNIX_EPOCH - Duration::from_secs(1)),
        None
    );
}
