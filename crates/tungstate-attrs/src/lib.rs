//! Gathering a file's attributes, tier by tier.
//!
//! The I/O half of classification. `tungstate-core` decides; this crate
//! reads, and reads no more than the policy's [`Tier`] asks for. A policy that
//! never mentions `exif` or `hash` never opens the file at all, and over FTP
//! a `head` read is one ranged request rather than the whole transfer.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::Path;

use jiff::Timestamp;
use tungstate_backend::walk::Walk;
use tungstate_backend::{Backend, BackendError};
pub use tungstate_core::{Attributes, Snapshot, Tier};
use tungstate_core::{Policy, snapshot};

/// Anything that can stop attributes being gathered.
#[derive(Debug, thiserror::Error)]
pub enum GatherError {
    /// The backend could not answer.
    #[error("could not read `{path}`")]
    Backend {
        /// The file.
        path: String,
        /// Why.
        #[source]
        source: BackendError,
    },

    /// The path is not a file this crate can describe.
    #[error("`{path}` is not a relative path with a file name")]
    BadPath {
        /// The path as given.
        path: String,
    },
}

/// Result alias so signatures read `Result<Attributes>`.
pub type Result<T> = std::result::Result<T, GatherError>;

/// Gather what `tier` allows about the file at `path`, relative to the
/// backend's root.
///
/// `Stat` costs one `stat`. `Head` and `Meta` cost one prefix read of 8 KiB
/// or 64 KiB. `Whole` streams the file once for its BLAKE3. The
/// modification time and the current time are both recorded so `age` is a
/// function of the returned data rather than of when the caller looks.
///
/// # Errors
/// [`GatherError::Backend`] if the backend cannot stat or read the file.
pub fn gather(backend: &dyn Backend, path: &Path, tier: Tier) -> Result<Attributes> {
    let relative = relative_string(path)?;
    let failed = |source: BackendError| GatherError::Backend {
        path: relative.clone(),
        source,
    };

    let meta = backend.stat(path).map_err(failed)?;
    let mut attrs = Attributes::new(&relative, meta.len, Timestamp::now());
    attrs.is_dir = meta.is_dir;
    attrs.is_symlink = meta.is_symlink;
    attrs.mtime = meta
        .modified
        .and_then(|time| Timestamp::try_from(time).ok());
    attrs.identity = meta.identity;

    // A directory or a link has no content worth sniffing, whatever the tier.
    if meta.is_dir || meta.is_symlink {
        return Ok(attrs);
    }

    if let Some(len) = tier.prefix_len() {
        let head = backend.read_prefix(path, len).map_err(failed)?;
        attrs.mime = Some(sniff_mime(&head, attrs.ext()));
        if tier >= Tier::Meta {
            attrs.exif = read_exif(&head);
        }
    }

    if tier == Tier::Whole {
        let mut reader = backend.open_read(path).map_err(failed)?;
        let hash = digest(&mut *reader).map_err(|source| {
            failed(BackendError::Io {
                path: path.to_path_buf(),
                source,
            })
        })?;
        attrs.hash = Some(hash);
        // The whole file passed through, so the head is a free by-product.
        if attrs.mime.is_none() {
            let head = backend
                .read_prefix(path, Tier::META_BYTES)
                .map_err(failed)?;
            attrs.mime = Some(sniff_mime(&head, attrs.ext()));
            attrs.exif = read_exif(&head);
        }
    }

    Ok(attrs)
}

/// Walk a whole folder and gather what `policy` needs about everything in it.
///
/// One pass and one `now`: every entry is stamped with the instant the walk
/// began, so a plan built from this classifies every file as of one moment
/// rather than as of whenever the walk happened to reach it. The tier is read
/// from the policy once, so a policy that never mentions `hash` costs a
/// directory listing and nothing else.
///
/// Directories matching `ignore` or `opaque`, and `.tungstate` itself, are
/// recorded but never descended. Recorded rather than dropped, deliberately:
/// a directory that vanished from the snapshot would look *empty* to the
/// planner, which would then offer to remove it.
///
/// # Errors
/// [`GatherError::Backend`] if the walk or any file cannot be read.
pub fn survey(backend: &dyn Backend, policy: &Policy) -> Result<Snapshot> {
    let taken = Timestamp::now();
    let tier = policy.required_tier();
    let failed = |path: &str| {
        let path = path.to_string();
        move |source: BackendError| GatherError::Backend { path, source }
    };

    let walk = Walk::new(backend, Path::new("")).prune(|entry| {
        let path = entry.path.to_string_lossy().replace('\\', "/");
        !snapshot::is_reserved(&path)
            && policy
                .folder
                .ignore
                .first_match_including_ancestors(&path)
                .is_none()
            && policy
                .folder
                .opaque
                .first_match_including_ancestors(&path)
                .is_none()
    });

    let mut entries = Vec::new();
    let mut directories = BTreeSet::new();
    for entry in walk {
        let entry = entry.map_err(failed("the folder"))?;
        let path = entry.path.to_string_lossy().replace('\\', "/");
        if snapshot::is_reserved(&path) {
            continue;
        }
        if entry.meta.is_dir {
            directories.insert(path.clone());
        }
        // Directories go through `gather` too: it returns early for one at any
        // tier, so this costs a `stat` and keeps one description of what an
        // entry is.
        let mut attrs = gather(backend, &entry.path, tier)?;
        // `gather` stamps its own `now`, which would leave two files read a
        // microsecond apart answering `age` differently for no reason.
        attrs.now = taken;
        entries.push(attrs);
    }

    Ok(Snapshot::new(
        taken,
        entries,
        directories,
        backend.capabilities().case_sensitive,
    ))
}

/// The path with forward slashes, as the policy language spells paths.
fn relative_string(path: &Path) -> Result<String> {
    let bad = || GatherError::BadPath {
        path: path.display().to_string(),
    };
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                parts.push(part.to_str().ok_or_else(bad)?.to_string());
            }
            std::path::Component::CurDir => {}
            _ => return Err(bad()),
        }
    }
    if parts.is_empty() {
        return Err(bad());
    }
    Ok(parts.join("/"))
}

/// Magic bytes first, the extension as a fallback.
///
/// `infer` knows binary formats and nothing about text, so `notes.txt` and
/// `README.md` come from the table below rather than from their bytes.
#[must_use]
pub fn sniff_mime(head: &[u8], ext: &str) -> String {
    if let Some(kind) = infer::get(head) {
        return kind.mime_type().to_string();
    }
    match ext.to_ascii_lowercase().as_str() {
        "txt" | "log" | "ini" | "cfg" | "rs" | "py" | "sh" | "c" | "h" | "go" | "ts" => {
            "text/plain"
        }
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        _ if looks_like_text(head) => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Whether a prefix is plausibly text: no NUL, and nearly all printable or
/// whitespace. Good enough for a fallback, and never used when magic bytes
/// or an extension already answered.
fn looks_like_text(head: &[u8]) -> bool {
    if head.is_empty() || head.contains(&0) {
        return false;
    }
    let printable = head
        .iter()
        .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace() || **b >= 0x80)
        .count();
    printable * 100 / head.len() >= 95
}

/// EXIF fields by tag name, from whatever container the prefix holds.
///
/// A truncated container makes `kamadak-exif` fail rather than return what
/// it found, and a failure here is "no EXIF", which is the honest answer
/// for a file whose metadata sits past 64 KiB.
#[must_use]
pub fn read_exif(head: &[u8]) -> BTreeMap<String, String> {
    let mut cursor = Cursor::new(head);
    let Ok(exif) = exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut cursor)
    else {
        return BTreeMap::new();
    };
    exif.fields()
        .filter(|field| field.ifd_num == exif::In::PRIMARY)
        .map(|field| (field.tag.to_string(), field.display_value().to_string()))
        .collect()
}

/// Stream a reader through BLAKE3, one megabyte at a time.
fn digest(reader: &mut dyn Read) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// A JPEG carrying one EXIF `DateTimeOriginal`, for tests and demonstrations.
///
/// Public because the CLI tests and the by-hand walkthrough need a photo
/// with a known camera date, and shipping a binary fixture in the repository
/// is worse than forty lines that build one.
///
/// # Panics
/// Never in practice: the only fallible steps are writes into memory.
#[must_use]
pub fn jpeg_with_exif_date(date_time_original: &str) -> Vec<u8> {
    use exif::experimental::Writer;

    let field = exif::Field {
        tag: exif::Tag::DateTimeOriginal,
        ifd_num: exif::In::PRIMARY,
        value: exif::Value::Ascii(vec![date_time_original.as_bytes().to_vec()]),
    };
    let mut tiff = Cursor::new(Vec::new());
    let mut writer = Writer::new();
    writer.push_field(&field);
    writer
        .write(&mut tiff, false)
        .expect("writing one ASCII field to memory cannot fail");
    let tiff = tiff.into_inner();

    // SOI, then APP1 carrying "Exif\0\0" + the TIFF, then EOI. No image data:
    // `infer` only needs the first three bytes to call it a JPEG, and
    // `kamadak-exif` only needs the APP1 segment.
    let mut jpeg = vec![0xFF, 0xD8];
    let payload_len = u16::try_from(tiff.len() + 8).expect("a test fixture fits in one segment");
    jpeg.extend_from_slice(&[0xFF, 0xE1]);
    jpeg.extend_from_slice(&payload_len.to_be_bytes());
    jpeg.extend_from_slice(b"Exif\0\0");
    jpeg.extend_from_slice(&tiff);
    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

#[cfg(test)]
mod tests;
