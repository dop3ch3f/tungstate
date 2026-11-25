use serde::{Deserialize, Serialize};

/// Whether the app may look at the photo library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Every photo and video.
    Full,
    /// Only the ones the person picked (iOS's limited access, Android 14's
    /// "select photos").
    Limited,
    /// Refused; only the system's Settings can change it now.
    Denied,
    /// Not asked yet.
    Ask,
}

/// Which photos and videos to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    All,
    Videos,
    Screenshots,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub kind: Kind,
    /// Only what is older than this many days; zero means everything.
    pub older_than_days: u32,
}

/// How many, and how many bytes.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Tally {
    pub count: u64,
    pub bytes: u64,
}

/// One year of the library.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Year {
    pub year: i32,
    pub count: u64,
    pub bytes: u64,
}

/// The library as the filter sees it. Favourites are counted apart from the
/// years, because by default they stay on the phone.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Summary {
    pub years: Vec<Year>,
    pub favourites: Tally,
}

/// One photo or video.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// The library's own address for it. Opaque outside the plugin.
    pub id: String,
    pub name: String,
    pub size: u64,
    /// When it was taken, or failing that when its file last changed.
    pub taken_ms: i64,
    /// The year of `taken_ms` in the phone's own time zone, as the summary
    /// counts it, so a photo lands in the year it was counted under.
    pub year: i32,
    pub kind: AssetKind,
    pub favourite: bool,
}

/// What a photo or video is, as far as where it is filed goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Photo,
    Video,
    Screenshot,
}

/// What came of asking to remove photos from the library.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Removed {
    /// Gone from the library, counted afterwards.
    pub removed: u64,
    /// False where the phone cannot let an app remove photos (Android 10
    /// and earlier); the person deletes them in their gallery instead.
    pub supported: bool,
}

/// What a file manager's category tiles count. One tally each; a file can
/// be in two (a PDF in Download is a document and a download).
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Categories {
    pub images: Tally,
    pub videos: Tally,
    pub audio: Tally,
    pub documents: Tally,
    pub downloads: Tally,
    /// Installer files (`.apk`), not the apps installed on the phone.
    pub apps: Tally,
}

/// One category tile, as the window names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Images,
    Videos,
    Audio,
    Documents,
    Downloads,
    Apps,
}

/// How much of the screen the system's bars cover, in CSS pixels: the web
/// view draws under them on Android and is told nothing about them.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Insets {
    pub top: f64,
    pub bottom: f64,
}

/// Files on shared storage, for sharing or opening in another app.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PathsArgs {
    pub paths: Vec<String>,
}

/// The order a category's first few files are picked in, matching the
/// window's sort so the first screenful is the one the full list starts with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    #[default]
    Newest,
    Oldest,
    Name,
    Largest,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct CategoryArgs {
    pub category: Category,
    /// Only this many, picked in `order`; every file when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<u32>,
    pub order: Order,
}

/// A file on the phone's shared storage, by its real path, which the
/// engine reads and moves like any other local file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified_ms: i64,
}
