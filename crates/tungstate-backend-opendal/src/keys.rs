//! Turning a `Path` into a key a remote will accept.
//!
//! `Path::join` produces `a\b` on Windows. An FTP or S3 key containing a
//! backslash is not "the same key with a different separator" — it is a
//! different, usually corrupt, key, and on some servers it is a directory
//! traversal. So remote paths never go through `Path::join`.

use std::path::{Component, Path};

use tungstate_backend::{BackendError, Result};

/// The escape rule, unchanged from `local.rs`, plus a literal `/` join.
///
/// Matching `Component` exhaustively rather than checking `is_absolute()` is
/// the same reasoning as the local backend: on Windows `/etc/passwd` is not
/// absolute, having no drive prefix, yet it still escapes. A future
/// `Component` variant becomes a compile error here rather than a silent hole.
///
/// # Errors
/// [`BackendError::PathEscapesRoot`] for `..`, or
/// [`BackendError::PathNotRelative`] for anything rooted or drive-prefixed.
pub fn remote_key(path: &Path) -> Result<String> {
    let mut key = String::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if !key.is_empty() {
                    key.push('/');
                }
                // Lossy for the same reason the journal stores paths lossily: a
                // non-UTF-8 filename cannot be expressed as a protocol key at
                // all, and mangling it here is more honest than a panic.
                key.push_str(&part.to_string_lossy());
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
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ordinary_relative_paths_become_slash_joined_keys() {
        assert_eq!(remote_key(Path::new("a/b.mp4")).unwrap(), "a/b.mp4");
        assert_eq!(remote_key(Path::new("./a")).unwrap(), "a");
        assert_eq!(remote_key(Path::new("")).unwrap(), "");
    }

    #[test]
    fn a_windows_style_path_still_yields_forward_slashes() {
        // The trap this module exists for. `PathBuf::join` on Windows would
        // produce `2024\holiday.mp4`, which is a different key on the far side.
        let built = PathBuf::from("2024").join("holiday.mp4");
        assert_eq!(remote_key(&built).unwrap(), "2024/holiday.mp4");
    }

    #[test]
    fn escapes_are_refused() {
        for escape in ["../etc/passwd", "a/../../b"] {
            assert!(matches!(
                remote_key(Path::new(escape)),
                Err(BackendError::PathEscapesRoot(_))
            ));
        }
        for rooted in ["/etc/passwd", "/"] {
            assert!(matches!(
                remote_key(Path::new(rooted)),
                Err(BackendError::PathNotRelative(_))
            ));
        }
    }

    proptest::proptest! {
        /// The invariant, over paths no hand-written list would think to try.
        ///
        /// The generator has no backslash in it deliberately. A backslash
        /// inside a filename on Unix is part of the name, not a separator, and
        /// passing it through verbatim is correct; what must never happen is a
        /// backslash appearing because *we* joined with one.
        #[test]
        fn no_generated_path_yields_a_dangerous_key(
            segments in proptest::collection::vec(
                proptest::prop_oneof![
                    "[a-zA-Z0-9._ -]{0,8}",
                    proptest::strategy::Just("..".to_string()),
                    proptest::strategy::Just(".".to_string()),
                    proptest::strategy::Just(String::new()),
                ],
                0..8,
            )
        ) {
            let candidate = segments.join("/");
            if let Ok(key) = remote_key(Path::new(&candidate)) {
                proptest::prop_assert!(!key.contains('\\'), "`{candidate}` -> `{key}`");
                proptest::prop_assert!(!key.starts_with('/'), "`{candidate}` -> `{key}`");
                proptest::prop_assert!(
                    key.is_empty()
                        || key.split('/').all(|p| p != ".." && p != "." && !p.is_empty()),
                    "`{candidate}` -> `{key}`"
                );
            }
        }
    }
}
