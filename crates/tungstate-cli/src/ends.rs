//! Reading a link end off the command line.
//!
//! An end used to be a path and nothing else. It is now either a path on this
//! machine or a path inside a named connection, and the command line has to
//! tell them apart without a mode flag.

use std::path::PathBuf;

use tungstate_journal::{Endpoint, Journal, JournalError};

/// Why an end could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum EndError {
    /// The argument looked like `name:path` but there is no such connection.
    #[error(
        "no connection named `{0}`\n  \
         `{0}:…` is read as a connection because `{0}` is two or more of \
         [A-Za-z0-9_-] before a colon.\n  \
         List them with `tungstate connection list`."
    )]
    UnknownConnection(String),

    /// The journal could not be read.
    #[error("could not read connections")]
    Journal(#[from] JournalError),
}

/// Split `name:path` into its parts, or `None` if this is an ordinary path.
///
/// The rule exists because `C:\Users\me` is a real Windows path. A single
/// character before the colon is always a drive letter; two or more of
/// `[A-Za-z0-9_-]` is a connection. No drive letter is ever two characters, so
/// the two cases cannot collide.
#[must_use]
pub fn connection_prefix(raw: &str) -> Option<(&str, &str)> {
    let (name, rest) = raw.split_once(':')?;
    let plausible = name.len() >= 2
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    plausible.then_some((name, rest))
}

/// Resolve one end of a link.
///
/// `explicit` is `--from-connection` / `--to-connection`, which is the
/// unambiguous form and always wins. Otherwise `raw` is checked for the
/// `name:path` convenience, and failing that is a path on this machine.
///
/// # Errors
/// [`EndError::UnknownConnection`] if a connection was named and does not
/// exist, or [`EndError::Journal`] if the lookup itself fails.
pub fn parse_end(
    raw: &str,
    explicit: Option<&str>,
    journal: &Journal,
) -> Result<Endpoint, EndError> {
    if let Some(name) = explicit {
        let connection = lookup(journal, name)?;
        return Ok(Endpoint::remote(connection, PathBuf::from(raw)));
    }

    match connection_prefix(raw) {
        // A shape that reads as a connection and is not one is a typo, not a
        // file called `nsa:inbox`. Saying so beats silently creating a link
        // into a directory of that literal name.
        Some((name, path)) => Ok(Endpoint::remote(
            lookup(journal, name)?,
            PathBuf::from(path),
        )),
        None => Ok(Endpoint::local(raw)),
    }
}

fn lookup(journal: &Journal, name: &str) -> Result<tungstate_journal::ConnectionId, EndError> {
    match journal.connection_by_name(name) {
        Ok(connection) => Ok(connection.id),
        Err(JournalError::UnknownConnection(_)) => {
            Err(EndError::UnknownConnection(name.to_string()))
        }
        Err(other) => Err(EndError::Journal(other)),
    }
}

/// How an end is written back out, in `link list` and in error messages.
#[must_use]
pub fn describe(end: &Endpoint, journal: &Journal) -> String {
    match end.connection {
        None => end.path.display().to_string(),
        Some(id) => {
            let name = journal
                .connection_by_id(id)
                .map_or_else(|_| format!("#{}", id.0), |c| c.name);
            format!("{name}:{}", end.path.display())
        }
    }
}

/// Where one end of a recorded operation was, written the way the user would.
///
/// A local location is its full path; a remote one is `connection:path`,
/// because the path on its own means nothing without the place it is in.
#[must_use]
pub fn place(location: &tungstate_journal::Location, journal: &Journal) -> String {
    let full = location.display_path();
    match location.connection {
        None => full,
        Some(id) => {
            let name = journal
                .connection_by_id(id)
                .map_or_else(|_| format!("#{}", id.0), |c| c.name);
            format!("{name}:{full}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tungstate_journal::{NewConnection, Scheme};

    fn journal_with_nas() -> Journal {
        let journal = Journal::open_in_memory().unwrap();
        journal
            .create_connection(&NewConnection {
                name: "nas".to_string(),
                scheme: Scheme::Fs,
                host: None,
                port: None,
                username: None,
                root: "/volume1".to_string(),
                options: BTreeMap::new(),
            })
            .unwrap();
        journal
    }

    #[test]
    fn a_named_connection_resolves_to_a_remote_end() {
        let journal = journal_with_nas();
        let end = parse_end("nas:inbox", None, &journal).unwrap();
        assert!(end.is_remote());
        assert_eq!(end.path, std::path::Path::new("inbox"));
    }

    #[test]
    fn a_windows_drive_letter_stays_a_local_path() {
        // The case the two-character rule exists for. No drive letter is ever
        // two characters, so `C:\Users\x` can never be read as a connection.
        let journal = journal_with_nas();
        for candidate in [r"C:\Users\x", "D:/data", r"c:\tmp"] {
            let end = parse_end(candidate, None, &journal).unwrap();
            assert!(!end.is_remote(), "`{candidate}` should be local");
            assert_eq!(end.path, std::path::Path::new(candidate));
        }
    }

    #[test]
    fn an_ordinary_path_stays_local() {
        let journal = journal_with_nas();
        for candidate in ["/Users/x/Videos", "./relative", "videos"] {
            assert!(!parse_end(candidate, None, &journal).unwrap().is_remote());
        }
    }

    #[test]
    fn a_connection_shaped_name_that_does_not_exist_is_an_error() {
        let journal = journal_with_nas();
        assert!(matches!(
            parse_end("unknown:path", None, &journal),
            Err(EndError::UnknownConnection(name)) if name == "unknown"
        ));
    }

    #[test]
    fn the_explicit_flag_wins_and_the_argument_stays_a_path() {
        // `--to-connection nas` with a path that itself contains a colon.
        let journal = journal_with_nas();
        let end = parse_end("odd:name", Some("nas"), &journal).unwrap();
        assert!(end.is_remote());
        assert_eq!(end.path, std::path::Path::new("odd:name"));
    }

    #[test]
    fn an_end_describes_itself_the_way_it_was_typed() {
        let journal = journal_with_nas();
        let remote = parse_end("nas:inbox/2026", None, &journal).unwrap();
        assert_eq!(describe(&remote, &journal), "nas:inbox/2026");
        assert_eq!(describe(&Endpoint::local("/Users/x"), &journal), "/Users/x");
    }

    #[test]
    fn a_recorded_location_names_the_place_it_was_in() {
        let journal = journal_with_nas();
        let id = journal.connection_by_name("nas").unwrap().id;

        let remote = tungstate_journal::Location {
            connection: Some(id),
            root: "inbox".into(),
            path: "a.mp4".into(),
        };
        // Always `/`, on every platform: that is the separator the far side
        // uses, and a backslash here would be a name no server has heard of.
        assert_eq!(place(&remote, &journal), "nas:inbox/a.mp4");

        // A local one is the opposite: it must read the way this machine
        // spells a path, backslashes and all, so it is built rather than
        // written out.
        let local = tungstate_journal::Location::new("/Users/x", "a.mp4");
        let native = std::path::Path::new("/Users/x")
            .join("a.mp4")
            .display()
            .to_string();
        assert_eq!(place(&local, &journal), native);
    }

    #[test]
    fn the_root_of_a_connection_is_writable_as_a_bare_name() {
        let journal = journal_with_nas();
        let end = parse_end("nas:", None, &journal).unwrap();
        assert!(end.is_remote());
        assert_eq!(end.path, std::path::Path::new(""));
    }
}
