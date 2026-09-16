//! What the classifier knows about one file, and what each fact costs.
//!
//! Everything here is plain data. The crate that fills it in
//! (`tungstate-attrs`) does the reading; this crate only decides.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::civil::DateTime;
use serde::{Deserialize, Serialize};

/// How much of a file has to be read to answer a policy's questions.
///
/// Ordered, so the tier a policy needs is the maximum over every attribute
/// it mentions. Over FTP that ordering is the difference between a ranged
/// read and pulling a 4 GB video across the network to learn it is a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Name, size and times. Free: a directory listing already has them.
    Stat,
    /// The first 8 KiB: enough for magic bytes.
    Head,
    /// The first 64 KiB: enough for EXIF and most container headers.
    Meta,
    /// Every byte: the content hash.
    Whole,
}

impl Tier {
    /// How many bytes [`Tier::Head`] reads.
    pub const HEAD_BYTES: u64 = 8 * 1024;
    /// How many bytes [`Tier::Meta`] reads.
    pub const META_BYTES: u64 = 64 * 1024;

    /// The prefix length this tier reads, or `None` when it reads nothing
    /// (`Stat`) or everything (`Whole`).
    #[must_use]
    pub fn prefix_len(self) -> Option<u64> {
        match self {
            Self::Stat | Self::Whole => None,
            Self::Head => Some(Self::HEAD_BYTES),
            Self::Meta => Some(Self::META_BYTES),
        }
    }

    /// The name used in traces and on the command line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stat => "stat",
            Self::Head => "head",
            Self::Meta => "meta",
            Self::Whole => "whole",
        }
    }
}

/// The cost of one attribute by name, or `None` if the name is not one the
/// vocabulary knows.
///
/// This is the whole cost model: `required_tier` is a fold of this over every
/// attribute a policy mentions.
#[must_use]
pub fn tier_of(attribute: &str) -> Option<Tier> {
    match attribute {
        "name" | "stem" | "ext" | "parent" | "path" | "size" | "mtime" | "now" | "source" => {
            Some(Tier::Stat)
        }
        "mime" => Some(Tier::Head),
        "hash" => Some(Tier::Whole),
        other
            if other
                .strip_prefix("exif.")
                .is_some_and(|tag| !tag.is_empty()) =>
        {
            Some(Tier::Meta)
        }
        _ => None,
    }
}

/// The attribute names, for an error message that lists what a rule may use.
pub const VOCABULARY: &str =
    "name, stem, ext, parent, path, size, mtime, now, source, mime, hash, exif.<Tag>";

/// Everything the classifier may ask about one file.
///
/// Fields past `mtime` are `Option`, because whether they were gathered is a
/// policy decision (see [`Tier`]), and an attribute that was never read must
/// be distinguishable from one that was read and found empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    /// The file's name, extension included.
    pub name: String,
    /// The directory the file is in, relative to the folder root, with
    /// forward slashes. Empty for a file at the root.
    pub parent: String,
    /// Size in bytes.
    pub size: u64,
    /// True for a directory.
    pub is_dir: bool,
    /// True for a symbolic link.
    pub is_symlink: bool,
    /// Last modification time, where the backend reports one.
    pub mtime: Option<Timestamp>,
    /// Media type from magic bytes, with an extension fallback. Tier `head`.
    pub mime: Option<String>,
    /// EXIF fields by tag name, as displayed. Tier `meta`.
    pub exif: BTreeMap<String, String>,
    /// BLAKE3 hex digest of the whole file. Tier `whole`.
    pub hash: Option<String>,
    /// Which link brought the file here, from the journal.
    pub source: Option<String>,
    /// The moment the decision is being made, so `age` is reproducible.
    pub now: Timestamp,
}

/// Split a root-relative path into its directory and its name, both owned.
fn split(relative_path: &str) -> (String, String) {
    let trimmed = relative_path.trim_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, name)) => (parent.to_string(), name.to_string()),
        None => (String::new(), trimmed.to_string()),
    }
}

impl Attributes {
    /// Stat-tier attributes for the file at `relative_path` (forward slashes).
    #[must_use]
    pub fn new(relative_path: &str, size: u64, now: Timestamp) -> Self {
        let (parent, name) = split(relative_path);
        Self {
            name,
            parent,
            size,
            is_dir: false,
            is_symlink: false,
            mtime: None,
            mime: None,
            exif: BTreeMap::new(),
            hash: None,
            source: None,
            now,
        }
    }

    /// Move these attributes to a new root-relative path.
    ///
    /// `name` and `parent` follow the move; everything read off the file —
    /// size, mime, EXIF, hash — does not, because none of it changes when a
    /// file is renamed. That is what lets a plan be applied to a snapshot on
    /// paper and the result classified again, with no filesystem involved.
    pub fn relocate(&mut self, relative_path: &str) {
        let (parent, name) = split(relative_path);
        self.parent = parent;
        self.name = name;
    }

    /// The name without its last extension. `archive.tar.gz` gives
    /// `archive.tar`; a dotfile like `.env` is all stem.
    #[must_use]
    pub fn stem(&self) -> &str {
        match self.name.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem,
            _ => &self.name,
        }
    }

    /// The last extension, without the dot and in its original case.
    /// Empty when there is none.
    #[must_use]
    pub fn ext(&self) -> &str {
        match self.name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => ext,
            _ => "",
        }
    }

    /// The file's path relative to the folder root, forward slashes.
    #[must_use]
    pub fn relative_path(&self) -> String {
        if self.parent.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.parent, self.name)
        }
    }

    /// The value of one attribute, or `None` if it is unknown or absent.
    #[must_use]
    pub fn get(&self, attribute: &str) -> Option<Value> {
        let text = |s: &str| Some(Value::Text(s.to_string()));
        match attribute {
            "name" => text(&self.name),
            "stem" => text(self.stem()),
            "ext" => text(self.ext()),
            "parent" => text(&self.parent),
            "path" => text(&self.relative_path()),
            "size" => Some(Value::Size(self.size)),
            "mtime" => self.mtime.map(Value::Instant),
            "now" => Some(Value::Instant(self.now)),
            "mime" => self.mime.as_deref().and_then(text),
            "hash" => self.hash.as_deref().and_then(text),
            "source" => self.source.as_deref().and_then(text),
            other => {
                let tag = other.strip_prefix("exif.")?;
                let raw = self.exif.get(tag)?;
                // A camera's clock is a wall-clock reading with no zone, so it
                // stays civil rather than being pinned to an instant.
                Some(
                    parse_exif_datetime(raw).map_or_else(|| Value::Text(raw.clone()), Value::Civil),
                )
            }
        }
    }
}

/// A camera timestamp. EXIF writes `2024:06:01 12:34:56`; `kamadak-exif`
/// displays it as `2024-06-01 12:34:56`. Both are accepted.
#[must_use]
pub fn parse_exif_datetime(raw: &str) -> Option<DateTime> {
    let raw = raw.trim();
    DateTime::strptime("%Y:%m:%d %H:%M:%S", raw)
        .or_else(|_| DateTime::strptime("%Y-%m-%d %H:%M:%S", raw))
        .ok()
}

/// One attribute's value, typed so a template knows how to format it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Free text: names, media types, hashes, captures.
    Text(String),
    /// A byte count.
    Size(u64),
    /// A moment in time, not yet placed in a zone.
    Instant(Timestamp),
    /// A wall-clock reading with no zone, as a camera records it.
    Civil(DateTime),
}

impl Value {
    /// The value as text, for `map`, for traces and for filters.
    ///
    /// An instant is shown to the second. Templates format dates themselves
    /// through strftime, so this is only ever read by a person or by `map`,
    /// and nine digits of nanoseconds help neither.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Size(n) => n.to_string(),
            Self::Instant(ts) => jiff::fmt::strtime::format("%Y-%m-%dT%H:%M:%SZ", *ts)
                .unwrap_or_else(|_| ts.to_string()),
            Self::Civil(dt) => dt.to_string(),
        }
    }
}
