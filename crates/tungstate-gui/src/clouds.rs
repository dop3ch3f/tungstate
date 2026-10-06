//! The folders cloud drives' own apps keep on this machine.
//!
//! Google Drive, `OneDrive`, Dropbox, Box and iCloud each install an app that
//! shows the drive as a folder. Tungstate can already work in any folder, so
//! these are offered by name in the place picker: the drive without a browser
//! sign-in, as long as its app is installed. Files the app keeps online only
//! are marked by the system and never read just to look (see
//! `tungstate_backend::Meta::online_only`).

use std::path::{Path, PathBuf};

/// One drive's folder, named as its app names the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFolder {
    /// What to call it: "Google Drive", "`OneDrive`", "iCloud Drive"…
    pub label: String,
    /// Where it is.
    pub path: PathBuf,
}

/// Every cloud drive folder under `home`, in a stable order.
///
/// Looks only where the apps put them, and only lists what exists, so the
/// same code is right on every platform: on macOS, `~/Library/CloudStorage`
/// (where the file-provider apps live) and iCloud's own folder; on Windows,
/// the apps' folders in the user's home.
#[must_use]
pub fn cloud_folders(home: &Path) -> Vec<CloudFolder> {
    let mut found = Vec::new();

    // macOS file providers: one folder each, named `<Service>-<account>`.
    if let Ok(entries) = std::fs::read_dir(home.join("Library/CloudStorage")) {
        let mut named: Vec<(String, String, PathBuf)> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| {
                let raw = entry.file_name().to_string_lossy().into_owned();
                let (service, account) = raw.split_once('-').unwrap_or((&raw, ""));
                let label = service_name(service)?;
                // Google Drive's folder holds "My Drive" and "Shared drives";
                // the files are in the first.
                let mine = entry.path().join("My Drive");
                let path = if mine.is_dir() { mine } else { entry.path() };
                Some((label.to_string(), account.to_string(), path))
            })
            .collect();
        named.sort();
        found.extend(tell_apart(named));
    }

    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    if icloud.is_dir() {
        found.push(CloudFolder {
            label: "iCloud Drive".to_string(),
            path: icloud,
        });
    }

    // Windows, and Dropbox on Linux: the apps' folders in the home folder.
    if let Ok(entries) = std::fs::read_dir(home) {
        let mut named: Vec<(String, String, PathBuf)> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| {
                let raw = entry.file_name().to_string_lossy().into_owned();
                let (label, account) = match raw.as_str() {
                    "Dropbox" => ("Dropbox", String::new()),
                    "Box" => ("Box", String::new()),
                    "iCloudDrive" => ("iCloud Drive", String::new()),
                    "OneDrive" => ("OneDrive", String::new()),
                    // A work or school OneDrive is `OneDrive - <organisation>`.
                    other => match other.strip_prefix("OneDrive - ") {
                        Some(org) => ("OneDrive", org.to_string()),
                        None => return None,
                    },
                };
                Some((label.to_string(), account, entry.path()))
            })
            .collect();
        named.sort();
        found.extend(tell_apart(named));
    }
    found
}

/// The service a file-provider folder belongs to, by the prefix its app uses.
/// `None` for one Tungstate does not know, which is left out rather than
/// shown with a raw folder name.
fn service_name(prefix: &str) -> Option<&'static str> {
    match prefix {
        "GoogleDrive" => Some("Google Drive"),
        "OneDrive" => Some("OneDrive"),
        "Dropbox" => Some("Dropbox"),
        "Box" => Some("Box"),
        "pCloudDrive" | "pCloud" => Some("pCloud"),
        "ProtonDrive" => Some("Proton Drive"),
        _ => None,
    }
}

/// Two drives of one service (a personal and a work `OneDrive`) are told apart
/// by their account; one alone needs only the service's name.
fn tell_apart(named: Vec<(String, String, PathBuf)>) -> Vec<CloudFolder> {
    let count = |label: &str| named.iter().filter(|(l, _, _)| l == label).count();
    named
        .iter()
        .map(|(label, account, path)| CloudFolder {
            label: if count(label) > 1 && !account.is_empty() {
                format!("{label} ({account})")
            } else {
                label.clone()
            },
            path: path.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with(dirs: &[&str]) -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("temp home");
        for dir in dirs {
            std::fs::create_dir_all(home.path().join(dir)).expect("dir");
        }
        home
    }

    fn labels(home: &Path) -> Vec<String> {
        cloud_folders(home).into_iter().map(|c| c.label).collect()
    }

    #[test]
    fn the_mac_file_providers_are_found_by_their_service() {
        let home = home_with(&[
            "Library/CloudStorage/GoogleDrive-me@example.com/My Drive",
            "Library/CloudStorage/OneDrive-Personal",
            "Library/CloudStorage/Dropbox",
            "Library/CloudStorage/Box-Box",
            "Library/CloudStorage/SomethingElse-x",
            "Library/Mobile Documents/com~apple~CloudDocs",
        ]);
        assert_eq!(
            labels(home.path()),
            ["Box", "Dropbox", "Google Drive", "OneDrive", "iCloud Drive"]
        );
        let google = cloud_folders(home.path())
            .into_iter()
            .find(|c| c.label == "Google Drive")
            .unwrap();
        assert!(
            google.path.ends_with("My Drive"),
            "the files are in My Drive"
        );
    }

    #[test]
    fn two_drives_of_one_service_are_told_apart_by_account() {
        let home = home_with(&[
            "Library/CloudStorage/OneDrive-Personal",
            "Library/CloudStorage/OneDrive-Contoso",
        ]);
        assert_eq!(
            labels(home.path()),
            ["OneDrive (Contoso)", "OneDrive (Personal)"]
        );
    }

    #[test]
    fn the_windows_folders_are_found_in_the_home_folder() {
        let home = home_with(&[
            "OneDrive",
            "OneDrive - Contoso",
            "Dropbox",
            "iCloudDrive",
            "Documents",
        ]);
        assert_eq!(
            labels(home.path()),
            ["Dropbox", "OneDrive", "OneDrive (Contoso)", "iCloud Drive"]
        );
    }

    #[test]
    fn a_home_with_no_drives_offers_none() {
        let home = home_with(&["Documents", "Downloads"]);
        assert_eq!(labels(home.path()), [] as [String; 0]);
    }
}
