//! The phone's storage, as the Files tab's cards show it: each volume's
//! path, size and free space.
//!
//! Answered in Rust rather than by the phone's code, because `statvfs` is
//! the same call on Android, iOS and desktop, and the volumes on Android are
//! plain directories under `/storage`.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One place files live, with its size.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Volume {
    pub label: String,
    pub path: String,
    /// Both zero where the system would not say.
    pub total: u64,
    pub free: u64,
    /// An SD card or USB drive, rather than the phone's own storage.
    pub removable: bool,
}

/// The phone's own storage first, then any cards and drives.
#[tauri::command]
pub(crate) fn volumes() -> Vec<Volume> {
    let mut out = Vec::new();
    if let Some(own) = own_storage() {
        out.push(volume(&own, own_label(), false));
    }
    #[cfg(target_os = "android")]
    for (n, card) in removable_under(Path::new("/storage")).iter().enumerate() {
        let label = if n == 0 {
            "SD card".to_string()
        } else {
            format!("SD card {}", n + 1)
        };
        out.push(volume(card, &label, true));
    }
    out
}

fn own_label() -> &'static str {
    if cfg!(target_os = "android") {
        "Internal storage"
    } else if cfg!(target_os = "ios") {
        "On this iPhone"
    } else {
        "Home"
    }
}

/// Where the person's own files start.
///
/// On Android the primary shared storage, the folder a file manager opens on
/// (`/storage/emulated/0`, where `/sdcard` leads). Elsewhere the home folder,
/// which on iOS is the app's own container, and on desktop is what the
/// harness browses.
fn own_storage() -> Option<PathBuf> {
    if cfg!(target_os = "android") {
        return ["/storage/emulated/0", "/sdcard"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_dir());
    }
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Cards and drives Android has mounted: every folder under `/storage` but
/// its own two, `emulated` (the phone's storage, per user) and `self` (a
/// link back into it). A card is named by its volume id, like `1A2B-3C4D`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))] // tested everywhere, called on Android
pub(crate) fn removable_under(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            name != "emulated" && name != "self" && e.path().is_dir()
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}

fn volume(path: &Path, label: &str, removable: bool) -> Volume {
    let (total, free) = space(path).unwrap_or((0, 0));
    Volume {
        label: label.to_string(),
        path: path.display().to_string(),
        total,
        free,
        removable,
    }
}

/// Total and free bytes on the file system holding `path`. Free is what an
/// app may still write, not counting the blocks kept back for the system.
#[cfg(unix)]
fn space(path: &Path) -> Option<(u64, u64)> {
    let stat = rustix::fs::statvfs(path).ok()?;
    Some((
        stat.f_blocks.saturating_mul(stat.f_frsize),
        stat.f_bavail.saturating_mul(stat.f_frsize),
    ))
}

#[cfg(not(unix))]
fn space(_path: &Path) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_are_every_folder_under_storage_but_the_phones_own() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["emulated", "self", "1A2B-3C4D", "0000-1111"] {
            std::fs::create_dir(root.path().join(dir)).unwrap();
        }
        std::fs::write(root.path().join("not-a-volume"), b"").unwrap();

        let names: Vec<String> = removable_under(root.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["0000-1111", "1A2B-3C4D"]);
    }

    #[test]
    fn no_storage_folder_means_no_cards() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(removable_under(&root.path().join("missing")), Vec::<PathBuf>::new());
    }

    #[test]
    fn the_own_volume_comes_first_with_its_size() {
        let all = volumes();
        let own = all.first().expect("this machine has a home folder");
        assert!(!own.removable);
        if cfg!(unix) {
            assert!(own.total > 0 && own.free <= own.total);
        }
    }
}
