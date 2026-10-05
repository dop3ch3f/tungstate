//! Turning a `Path` into a path on the server.
//!
//! SFTP paths are `/`-separated whatever either machine is, and are the
//! server's own: absolute, or relative to where the account lands.

use std::path::{Component, Path};

use tungstate_backend::{BackendError, Result};

/// The parts of `path`, checked, in order. Empty for the link end itself.
///
/// # Errors
/// [`BackendError::PathEscapesRoot`] for `..`, and
/// [`BackendError::PathNotRelative`] for anything rooted or drive-prefixed.
pub fn parts(path: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => found.push(part.to_string_lossy().into_owned()),
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

/// The connection's folder as the server is asked for it. Empty means where
/// the account lands, which SFTP calls `.`.
#[must_use]
pub fn root(typed: &str) -> String {
    let trimmed = typed.trim();
    let absolute = trimmed.starts_with('/');
    let inner: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
    match (absolute, inner.is_empty()) {
        (true, true) => "/".to_string(),
        (true, false) => format!("/{}", inner.join("/")),
        (false, true) => ".".to_string(),
        (false, false) => inner.join("/"),
    }
}

/// `parts` inside `base`.
#[must_use]
pub fn under(base: &str, parts: &[String]) -> String {
    if parts.is_empty() {
        return base.to_string();
    }
    let joined = parts.join("/");
    match base {
        "." => joined,
        "/" => format!("/{joined}"),
        _ => format!("{base}/{joined}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_is_absolute_or_where_the_account_lands() {
        assert_eq!(root("/volume1/media/"), "/volume1/media");
        assert_eq!(root("  media//photos "), "media/photos");
        assert_eq!(root(""), ".");
        assert_eq!(root("/"), "/");
    }

    #[test]
    fn paths_join_under_any_root() {
        let a = |s: &str| s.to_string();
        assert_eq!(under(".", &[a("a.jpg")]), "a.jpg");
        assert_eq!(under("/", &[a("a"), a("b.jpg")]), "/a/b.jpg");
        assert_eq!(
            under("/volume1/media", &[a("2024"), a("a b.jpg")]),
            "/volume1/media/2024/a b.jpg"
        );
        assert_eq!(under("media", &[]), "media");
    }

    #[test]
    fn a_path_cannot_climb_out_or_start_elsewhere() {
        assert!(matches!(
            parts(Path::new("a/../../etc")),
            Err(BackendError::PathEscapesRoot(_))
        ));
        assert!(matches!(
            parts(Path::new("/etc/passwd")),
            Err(BackendError::PathNotRelative(_))
        ));
        assert_eq!(parts(Path::new("./a/b")).unwrap(), ["a", "b"]);
    }
}
