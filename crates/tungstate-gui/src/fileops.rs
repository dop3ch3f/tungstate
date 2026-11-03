//! The file manager's own small operations on the phone: make a folder,
//! rename one thing, delete what is selected. Copy and Move are transfers
//! and go through the engine's queue; these are single steps that do not.
//!
//! Each step is journaled before it is tried, like every transfer's, so
//! History says what happened to a file whichever screen did it.
//!
//! Paths are the window's composite locations (`/storage/emulated/0/DCIM`
//! or `Area51:Photos/2026`). Every one is split into its folder and one name,
//! and the backend is rooted at the folder, so the backend's own path rules
//! keep each step inside it.

use std::path::{Path, PathBuf};

use tauri::State;
use tungstate_backend::Backend;
use tungstate_journal::{Endpoint, Journal, Location, NewOp, OpKind, Outcome, ends};

use crate::{App, backend_for, describe};

/// The folder a location is in, as an endpoint, and its own name there.
fn split(path: &str, journal: &Journal) -> Result<(Endpoint, String), String> {
    let parent =
        ends::parent_display(path).ok_or("that is the top of a place, which cannot be changed")?;
    let end = ends::parse_end(&parent, None, journal).map_err(describe)?;
    let here = ends::parse_end(path, None, journal).map_err(describe)?;
    let name = here
        .path
        .file_name()
        .ok_or("that is the top of a place, which cannot be changed")?
        .to_string_lossy()
        .into_owned();
    Ok((end, name))
}

/// A name as typed for a new folder or a rename: one path component, not
/// empty, not a dot or two, nothing a connection would read as a separator.
fn name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("type a name".into());
    }
    if name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("a name cannot contain / or \\, or be . or ..".into());
    }
    Ok(name.to_string())
}

/// Journals one step, does it, and records how it went.
fn step(
    journal: &Journal,
    op: &NewOp,
    work: impl FnOnce() -> tungstate_backend::Result<()>,
) -> Result<(), String> {
    let id = journal.begin(op).map_err(describe)?;
    let done = work().map_err(describe);
    let outcome = match &done {
        Ok(()) => Outcome::Committed { hash: None },
        Err(error) => Outcome::Failed {
            error: error.clone(),
        },
    };
    journal.finish(id, &outcome).map_err(describe)?;
    done
}

fn op(kind: OpKind, source: Option<Location>, destination: Option<Location>) -> NewOp {
    NewOp {
        kind,
        source,
        destination,
        size: None,
        link: None,
        link_id: None,
    }
}

/// Make a folder called `called` inside `inside`.
#[tauri::command(async)]
pub(crate) fn make_folder(
    inside: String,
    called: String,
    state: State<'_, App>,
) -> Result<String, String> {
    make_folder_in(&inside, &called, &state.journal)
}

fn make_folder_in(inside: &str, called: &str, journal: &Journal) -> Result<String, String> {
    let called = name(called)?;
    let end = ends::parse_end(inside, None, journal).map_err(describe)?;
    let backend = backend_for(&end, journal)?;
    if backend.stat(Path::new(&called)).is_ok() {
        return Err(format!("there is already something called {called} here"));
    }
    let made = op(OpKind::MkDir, None, Some(Location::within(&end, &called)));
    step(journal, &made, || {
        backend.create_dir_all(Path::new(&called))
    })?;
    Ok(ends::join_display(inside, &called))
}

/// Give one file or folder a new name in the same folder. Refuses a name
/// already taken: the backend's rename would replace it without a word.
#[tauri::command(async)]
pub(crate) fn rename_entry(
    path: String,
    to: String,
    state: State<'_, App>,
) -> Result<String, String> {
    rename_in(&path, &to, &state.journal)
}

fn rename_in(path: &str, to: &str, journal: &Journal) -> Result<String, String> {
    let to = name(to)?;
    let (end, from) = split(path, journal)?;
    if from == to {
        return Ok(path.to_string());
    }
    let backend = backend_for(&end, journal)?;
    // A case-only rename on a folder that ignores case finds itself here;
    // that one is allowed, since it replaces nothing.
    if backend.stat(Path::new(&to)).is_ok() && !from.eq_ignore_ascii_case(&to) {
        return Err(format!("there is already something called {to} here"));
    }
    let renamed = op(
        OpKind::Rename,
        Some(Location::within(&end, &from)),
        Some(Location::within(&end, &to)),
    );
    step(journal, &renamed, || {
        backend.rename(Path::new(&from), Path::new(&to))
    })?;
    let parent = ends::parent_display(path).unwrap_or_default();
    Ok(ends::join_display(&parent, &to))
}

/// Delete these for good: there is no bin on a phone or a connection. The
/// window asks first. Stops at the first that fails, and says how many went.
#[tauri::command(async)]
pub(crate) fn delete_entries(paths: Vec<String>, state: State<'_, App>) -> Result<usize, String> {
    delete_in(&paths, &state.journal)
}

fn delete_in(paths: &[String], journal: &Journal) -> Result<usize, String> {
    let mut gone = 0;
    for path in paths {
        let (end, name) = split(path, journal)?;
        let backend = backend_for(&end, journal)?;
        let meta = backend.stat(Path::new(&name)).map_err(describe)?;
        if meta.is_dir {
            gone += remove_tree(backend.as_ref(), &end, Path::new(&name), journal)
                .map_err(|e| format!("{e} ({gone} removed before this)"))?;
        } else {
            let removed = op(OpKind::Remove, Some(Location::within(&end, &name)), None);
            step(journal, &removed, || backend.remove_file(Path::new(&name)))
                .map_err(|e| format!("{e} ({gone} removed before this)"))?;
            gone += 1;
        }
    }
    Ok(gone)
}

/// A folder and everything in it, deepest first, one journaled step per
/// file and folder. Counts files and folders alike.
///
/// Recursion depth is the folder's depth, which a phone's storage keeps
/// shallow; each level holds only its own listing.
fn remove_tree(
    backend: &dyn Backend,
    end: &Endpoint,
    dir: &Path,
    journal: &Journal,
) -> Result<usize, String> {
    let mut gone = 0;
    for entry in backend.read_dir(dir).map_err(describe)? {
        let child: PathBuf = dir.join(entry.path.file_name().unwrap_or(entry.path.as_os_str()));
        if entry.meta.is_dir && !entry.meta.is_symlink {
            gone += remove_tree(backend, end, &child, journal)?;
        } else {
            let removed = op(OpKind::Remove, Some(Location::within(end, &child)), None);
            step(journal, &removed, || backend.remove_file(&child))?;
            gone += 1;
        }
    }
    let removed = op(OpKind::RmDir, Some(Location::within(end, dir)), None);
    step(journal, &removed, || backend.remove_dir(dir))?;
    Ok(gone + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> Journal {
        Journal::open_in_memory().unwrap()
    }

    fn at(dir: &Path, rest: &str) -> String {
        dir.join(rest).display().to_string()
    }

    #[test]
    fn a_new_folder_is_made_once_and_named_as_typed() {
        let root = tempfile::tempdir().unwrap();
        let j = journal();
        let made = make_folder_in(&root.path().display().to_string(), "  Holiday  ", &j).unwrap();
        assert_eq!(made, at(root.path(), "Holiday"));
        assert!(root.path().join("Holiday").is_dir());
        assert!(make_folder_in(&root.path().display().to_string(), "Holiday", &j).is_err());
    }

    #[test]
    fn names_that_would_leave_the_folder_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let here = root.path().display().to_string();
        for bad in ["", "..", "a/b", "a\\b"] {
            assert!(make_folder_in(&here, bad, &journal()).is_err(), "{bad:?}");
        }
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_rename_never_replaces_another_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.jpg"), b"a").unwrap();
        std::fs::write(root.path().join("b.jpg"), b"b").unwrap();
        let j = journal();

        assert!(rename_in(&at(root.path(), "a.jpg"), "b.jpg", &j).is_err());
        assert_eq!(std::fs::read(root.path().join("b.jpg")).unwrap(), b"b");

        let to = rename_in(&at(root.path(), "a.jpg"), "c.jpg", &j).unwrap();
        assert_eq!(to, at(root.path(), "c.jpg"));
        assert_eq!(std::fs::read(root.path().join("c.jpg")).unwrap(), b"a");
        assert!(!root.path().join("a.jpg").exists());
    }

    #[test]
    fn deleting_a_folder_takes_everything_in_it_and_journals_each() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("old");
        std::fs::create_dir_all(folder.join("deep")).unwrap();
        std::fs::write(folder.join("one.txt"), b"1").unwrap();
        std::fs::write(folder.join("deep/two.txt"), b"2").unwrap();
        std::fs::write(root.path().join("keep.txt"), b"k").unwrap();
        let j = journal();

        let gone = delete_in(&[at(root.path(), "old")], &j).unwrap();
        assert_eq!(gone, 4, "two files and two folders");
        assert!(!folder.exists());
        assert!(root.path().join("keep.txt").exists());
        assert_eq!(j.recent(10).unwrap().len(), 4);
        assert!(j.incomplete().unwrap().is_empty());
    }

    #[test]
    fn the_top_of_a_place_cannot_be_deleted_or_renamed() {
        let j = journal();
        assert!(delete_in(&["/".to_string()], &j).is_err());
        assert!(rename_in("/", "x", &j).is_err());
    }
}
