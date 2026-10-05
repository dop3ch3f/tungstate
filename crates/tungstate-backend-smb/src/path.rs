//! Turning a `Path` into a name inside an SMB share.
//!
//! SMB separates with `\` and reads `:` as the start of an alternate data
//! stream. On macOS and Linux both are ordinary characters in a file name, so
//! a single component such as `a\..\b` passes every check a local path would
//! and then climbs out of the folder on the server. Those characters are
//! refused rather than escaped: there is no spelling of them SMB accepts.

use std::path::{Component, Path};

use tungstate_backend::{BackendError, Result};

/// The parts of `path`, checked, in order. Empty for the root.
///
/// # Errors
/// [`BackendError::PathEscapesRoot`] for `..` or for a part SMB would read as
/// a separator or a stream, and [`BackendError::PathNotRelative`] for anything
/// rooted or drive-prefixed.
pub fn parts(path: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.contains(['\\', ':']) {
                    return Err(BackendError::PathEscapesRoot(path.to_path_buf()));
                }
                found.push(part.into_owned());
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(BackendError::PathNotRelative(path.to_path_buf()));
            }
            Component::ParentDir => {
                return Err(BackendError::PathEscapesRoot(path.to_path_buf()));
            }
        }
    }
    Ok(found)
}

/// The share name and the folder inside it, from a connection's root.
///
/// Either slash separates here, because this is typed by a person, and a
/// Windows user writes `media\backups`. `None` when no share is named.
#[must_use]
pub fn split_root(root: &str) -> Option<(String, Vec<String>)> {
    let mut pieces = root
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|piece| !piece.is_empty());
    let share = pieces.next()?.to_string();
    Some((share, pieces.map(str::to_string).collect()))
}

/// Parts joined the way SMB names a file within a share.
#[must_use]
pub fn join(parts: &[String]) -> String {
    parts.join("\\")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn ordinary_paths_become_their_parts() {
        assert_eq!(parts(Path::new("a/b.mp4")).unwrap(), ["a", "b.mp4"]);
        assert_eq!(parts(Path::new("./a")).unwrap(), ["a"]);
        assert_eq!(
            parts(Path::new("")).unwrap(),
            [] as [std::string::String; 0]
        );
        assert_eq!(
            join(&parts(Path::new("2024/trip/a.jpg")).unwrap()),
            r"2024\trip\a.jpg"
        );
    }

    #[test]
    fn a_name_smb_would_read_as_a_way_out_is_refused() {
        for sneaky in [r"a\..\..\etc", "file.txt:hidden", "..", "a/../b"] {
            assert!(
                matches!(
                    parts(Path::new(sneaky)),
                    Err(BackendError::PathEscapesRoot(_))
                ),
                "{sneaky}"
            );
        }
        assert!(matches!(
            parts(&PathBuf::from("/etc/passwd")),
            Err(BackendError::PathNotRelative(_))
        ));
        // One name holding a backslash on macOS and Linux, which SMB would
        // read as a way into another folder. On Windows the backslash already
        // separated it into two ordinary parts before it got here.
        if cfg!(windows) {
            assert_eq!(parts(Path::new(r"x\y")).unwrap(), ["x", "y"]);
        } else {
            assert!(matches!(
                parts(Path::new(r"x\y")),
                Err(BackendError::PathEscapesRoot(_))
            ));
        }
    }

    #[test]
    fn a_root_names_its_share_first_whichever_slash_was_typed() {
        assert_eq!(split_root("media"), Some(("media".into(), vec![])));
        assert_eq!(
            split_root(r"/media\backups/2026/"),
            Some(("media".into(), vec!["backups".into(), "2026".into()]))
        );
        assert_eq!(split_root(" / "), None);
    }

    proptest::proptest! {
        /// Whatever a name contains, what comes back never has a separator or
        /// a stream marker inside a part, so it cannot name anything outside.
        #[test]
        fn no_accepted_part_can_leave_the_folder(name in ".{0,40}") {
            if let Ok(found) = parts(Path::new(&name)) {
                for part in found {
                    proptest::prop_assert!(!part.contains(['\\', ':', '/']));
                    proptest::prop_assert!(part != "..");
                }
            }
        }
    }
}
