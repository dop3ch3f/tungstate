//! Looking at files before opening them: small pictures for file lists, a
//! screen-sized one for the viewer, and a copy of a file on a connection that
//! the phone can open.
//!
//! Pictures are asked for over the `thumb:` scheme as `/f?p=<location>&s=<edge>`
//! and kept in their own cache folder, named by the file's path, size and
//! modification time, so an edited file gets a new picture.
//!
//! How each is made:
//! - **On this phone**: Android's own decoders, through the photos plugin, so
//!   HEIC, video frames, app icons and song covers all work.
//! - **On a connection**: the preview a camera stores inside the photo's first
//!   few dozen kilobytes. A photo without one is fetched whole, up to a limit,
//!   and shrunk here. Videos on a connection keep their kind's mark.
//! - **On desktop**: pictures only, decoded by `tungstate-likeness`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tungstate_backend::Backend;
use tungstate_execute::thumbs::Thumbs;
use tungstate_journal::Journal;
use tungstate_journal::ends::connection_prefix;

use crate::{App, describe, secrets};

/// The edge a list asks for, and the most a viewer may.
const LIST_EDGE: u32 = 256;
const MOST_EDGE: u32 = 1600;
/// The cache's ceiling. A 256 px JPEG is about 15 KB, so this is some twenty
/// thousand list pictures, or a few hundred viewer-sized ones.
const MOST_CACHED: u64 = 300 * 1024 * 1024;
/// Trim the cache after this many new pictures, not on every one.
const TRIM_EVERY: usize = 200;
/// The biggest photo on a connection fetched whole for want of an embedded
/// preview. A camera's original is rarely more; a 200 MB panorama is skipped.
const MOST_FETCHED: u64 = 40 * 1024 * 1024;
/// Pictures made from connections at once. Each holds a network read open,
/// and an SMB server answers requests on one connection in turn anyway.
const AT_ONCE_REMOTE: usize = 3;

const PICTURE_ENDINGS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "avif", "tif", "tiff",
];
const MOVING_ENDINGS: &[&str] = &["mp4", "mov", "m4v", "mkv", "webm", "3gp", "avi"];
const SOUND_ENDINGS: &[&str] = &["mp3", "m4a", "flac", "ogg", "opus", "wav", "aac"];

/// What the pictures need between requests.
pub struct Previews {
    cache: Option<Thumbs>,
    /// Where copies of connection files are put to be opened.
    opened: Option<PathBuf>,
    /// One open backend per connection, by name. An SMB sign-in is several
    /// round trips, and a folder of photos asks for dozens of pictures.
    backends: Mutex<HashMap<String, Arc<dyn Backend>>>,
    remote: Gate,
    made: AtomicUsize,
}

impl Previews {
    pub fn new(cache_dir: Option<&Path>) -> Self {
        let opened = cache_dir.map(|dir| dir.join("opened"));
        // Copies opened in an earlier session are not wanted any more; the
        // app that showed them has long since read them.
        if let Some(dir) = &opened {
            let _ = std::fs::remove_dir_all(dir);
        }
        Self {
            cache: cache_dir.and_then(|dir| Thumbs::at(&dir.join("pictures")).ok()),
            opened,
            backends: Mutex::new(HashMap::new()),
            remote: Gate::new(AT_ONCE_REMOTE),
            made: AtomicUsize::new(0),
        }
    }

    /// Forgets the open backend for a connection, after its password changes
    /// or it is removed.
    pub fn forget(&self, connection: &str) {
        if let Ok(mut open) = self.backends.lock() {
            open.remove(connection);
        }
    }

    fn backend(&self, name: &str, journal: &Journal) -> Result<Arc<dyn Backend>, String> {
        if let Some(open) = self
            .backends
            .lock()
            .ok()
            .and_then(|open| open.get(name).cloned())
        {
            return Ok(open);
        }
        let end = tungstate_journal::ends::parse_end(&format!("{name}:"), None, journal)
            .map_err(describe)?;
        let fresh: Arc<dyn Backend> = Arc::from(
            tungstate_backend_opendal::open(&end, journal, &secrets()).map_err(describe)?,
        );
        if let Ok(mut open) = self.backends.lock() {
            open.insert(name.to_owned(), Arc::clone(&fresh));
        }
        Ok(fresh)
    }
}

/// A counting semaphore: std has none, and this is all of one.
struct Gate {
    free: Mutex<usize>,
    freed: Condvar,
}

impl Gate {
    const fn new(n: usize) -> Self {
        Self {
            free: Mutex::new(n),
            freed: Condvar::new(),
        }
    }

    fn enter(&self) -> Pass<'_> {
        let mut free = self
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *free == 0 {
            free = self
                .freed
                .wait(free)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *free -= 1;
        Pass(self)
    }
}

struct Pass<'a>(&'a Gate);

impl Drop for Pass<'_> {
    fn drop(&mut self) {
        let mut free = self
            .0
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *free += 1;
        self.0.freed.notify_one();
    }
}

fn ending(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_picture(path: &str) -> bool {
    PICTURE_ENDINGS.contains(&ending(path).as_str())
}

/// Whether the phone's decoders can make a picture of a file of this kind.
fn has_picture(path: &str) -> bool {
    let end = ending(path);
    is_picture(path)
        || MOVING_ENDINGS.contains(&end.as_str())
        || SOUND_ENDINGS.contains(&end.as_str())
        || end == "apk"
}

/// Whether the window may have a picture of this local file.
///
/// An absolute path with no `..` in it. On a phone, also only shared storage
/// and the app's own cache, which is all a file window can show; on desktop
/// the window can already browse anywhere, so the kind check is the limit.
fn allowed_local(path: &Path, cache_dir: Option<&Path>) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    if cfg!(mobile) {
        return path.starts_with("/storage/") || cache_dir.is_some_and(|dir| path.starts_with(dir));
    }
    true
}

/// The edge asked for, held to what a list or a viewer needs.
fn edge(asked: Option<u32>) -> u32 {
    asked.unwrap_or(LIST_EDGE).clamp(64, MOST_EDGE)
}

/// `p` and `s` from a query string, percent-decoded.
fn query(raw: &str) -> (Option<String>, Option<u32>) {
    let mut path = None;
    let mut size = None;
    for pair in raw.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "p" => path = percent_decode(value),
            "s" => size = value.parse().ok(),
            _ => {}
        }
    }
    (path, size)
}

fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Answers one `thumb:` request for a file's picture. Blocking: call it off
/// the window's thread.
pub fn serve<R: Runtime>(app: &AppHandle<R>, raw_query: &str) -> tauri::http::Response<Vec<u8>> {
    let (Some(path), size) = query(raw_query) else {
        return crate::empty_picture();
    };
    let Some(state) = app.try_state::<App>() else {
        return crate::empty_picture();
    };
    match picture(app, &state, &path, edge(size)) {
        Some(bytes) => tauri::http::Response::builder()
            .header("Content-Type", "image/jpeg")
            // Named by size and mtime, so a cached answer is never stale.
            .header("Cache-Control", "max-age=86400")
            .body(bytes)
            .unwrap_or_else(|_| crate::empty_picture()),
        None => crate::empty_picture(),
    }
}

fn picture<R: Runtime>(
    app: &AppHandle<R>,
    state: &App,
    location: &str,
    edge: u32,
) -> Option<Vec<u8>> {
    let previews = &state.previews;
    let cache = previews.cache.as_ref()?;
    let remote = if location.starts_with("content:") {
        None
    } else {
        connection_prefix(location)
    };

    // The photo library's own address for a photo, which Free up lists by.
    // Checked first: `content:` would otherwise read as a connection's name.
    // An id is never reused for another photo, so it needs no size or mtime.
    if location.starts_with("content://media/") {
        if !cfg!(mobile) {
            return None;
        }
        let name = Thumbs::name(&format!("e{edge}"), location, 0, None);
        if let Ok(bytes) = std::fs::read(cache.at_name(&name)) {
            return Some(bytes);
        }
        let bytes = make_local(app, cache, Path::new(location), &name, edge)?;
        let _ = cache.keep(&name, &bytes);
        return Some(bytes);
    }

    let (name, bytes) = match remote {
        None => {
            let path = Path::new(location);
            if !allowed_local(path, crate::cache_dir().as_deref()) || !has_picture(location) {
                return None;
            }
            let meta = std::fs::metadata(path).ok()?;
            let mtime = modified_ms(meta.modified().ok());
            let name = Thumbs::name(&format!("e{edge}"), location, meta.len(), mtime);
            if let Ok(bytes) = std::fs::read(cache.at_name(&name)) {
                return Some(bytes);
            }
            (name.clone(), make_local(app, cache, path, &name, edge)?)
        }
        Some((connection, inside)) => {
            if !is_picture(location) {
                return None;
            }
            let backend = previews.backend(connection, &state.journal).ok()?;
            let at = Path::new(inside.trim_start_matches('/'));
            let meta = backend.stat(at).ok()?;
            let mtime = modified_ms(meta.modified);
            let name = Thumbs::name(&format!("e{edge}"), location, meta.len, mtime);
            if let Ok(bytes) = std::fs::read(cache.at_name(&name)) {
                return Some(bytes);
            }
            let _pass = previews.remote.enter();
            (
                name.clone(),
                make_remote(app, cache, backend.as_ref(), at, meta.len, &name, edge)?,
            )
        }
    };
    let _ = cache.keep(&name, &bytes);
    if previews.made.fetch_add(1, Ordering::Relaxed) % TRIM_EVERY == TRIM_EVERY - 1 {
        let dir = cache.clone();
        std::thread::spawn(move || dir.trim(MOST_CACHED));
    }
    Some(bytes)
}

fn modified_ms(at: Option<std::time::SystemTime>) -> Option<i64> {
    at.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

/// A picture of a file on this device.
#[allow(unused_variables)]
fn make_local<R: Runtime>(
    app: &AppHandle<R>,
    cache: &Thumbs,
    path: &Path,
    name: &str,
    edge: u32,
) -> Option<Vec<u8>> {
    #[cfg(mobile)]
    {
        let out = cache.at_name(&format!("{name}.part"));
        let made = tauri_plugin_photos::PhotosExt::photos(app).thumbnail(
            &path.to_string_lossy(),
            edge,
            &out,
        );
        let bytes = made.ok().and_then(|()| std::fs::read(&out).ok());
        let _ = std::fs::remove_file(&out);
        bytes
    }
    #[cfg(desktop)]
    {
        if !is_picture(&path.to_string_lossy()) {
            return None;
        }
        tungstate_likeness::look(path, tungstate_likeness::Kind::Picture, true)
            .ok()?
            .thumb
    }
}

/// A picture of a photo on a connection: its embedded preview for a list,
/// or the whole photo, shrunk, when there is none or the viewer wants more.
fn make_remote<R: Runtime>(
    app: &AppHandle<R>,
    cache: &Thumbs,
    backend: &dyn Backend,
    at: &Path,
    len: u64,
    name: &str,
    edge: u32,
) -> Option<Vec<u8>> {
    if edge <= LIST_EDGE {
        let head = backend
            .read_prefix(at, tungstate_likeness::preview::HEAD_BYTES)
            .ok()?;
        if let Some(thumb) =
            tungstate_likeness::preview::from_head(&head, true).and_then(|shot| shot.thumb)
        {
            return Some(thumb);
        }
    }
    if len > MOST_FETCHED {
        return None;
    }
    // Fetched under the cache folder with the photo's own ending, so the
    // phone's decoder knows what it is looking at.
    let whole = cache.at_name(&format!("{name}.whole.{}", ending(&at.to_string_lossy())));
    let fetched = (|| -> std::io::Result<()> {
        let mut from = backend.open_read(at).map_err(std::io::Error::other)?;
        let mut to = std::fs::File::create(&whole)?;
        std::io::copy(&mut from, &mut to)?;
        Ok(())
    })();
    let bytes = fetched
        .ok()
        .and_then(|()| make_local(app, cache, &whole, name, edge));
    let _ = std::fs::remove_file(&whole);
    bytes
}

/// How far a copy of a connection file has come.
#[derive(Debug, Clone, Serialize)]
struct Fetching<'a> {
    path: &'a str,
    done: u64,
    total: u64,
}

/// Copies a file from a connection into the app's cache, for opening it in
/// the viewer or another app, and answers with the copy's path. Sends
/// `fetch://progress` as it goes. A copy already made this session is used
/// again when the file has not changed size.
#[tauri::command(async)]
pub fn fetch_copy<R: Runtime>(app: AppHandle<R>, path: String) -> Result<String, String> {
    let state = app.state::<App>();
    let Some((connection, inside)) = connection_prefix(&path) else {
        return Err(format!("{path} is not on a connection"));
    };
    let dir = state
        .previews
        .opened
        .clone()
        .ok_or("this phone gave the app no cache folder")?;
    let backend = state.previews.backend(connection, &state.journal)?;
    let at = Path::new(inside.trim_start_matches('/'));
    let meta = backend.stat(at).map_err(describe)?;
    let file_name = at.file_name().ok_or("that is a connection, not a file")?;
    // A folder per connection file, so two `cover.jpg`s don't collide, and
    // the copy keeps its own name for whatever app opens it.
    let key = Thumbs::name("opened", &path, meta.len, modified_ms(meta.modified));
    let into = dir.join(key.trim_end_matches(".jpg"));
    let copy = into.join(file_name);
    if std::fs::metadata(&copy).is_ok_and(|m| m.len() == meta.len) {
        return Ok(copy.display().to_string());
    }
    std::fs::create_dir_all(&into).map_err(describe)?;
    let part = into.join(".part");
    let mut from = backend.open_read(at).map_err(describe)?;
    let mut to = std::fs::File::create(&part).map_err(describe)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = std::io::Read::read(&mut from, &mut buf).map_err(describe)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut to, &buf[..n]).map_err(describe)?;
        done += n as u64;
        let _ = app.emit(
            "fetch://progress",
            Fetching {
                path: &path,
                done,
                total: meta.len,
            },
        );
    }
    drop(to);
    std::fs::rename(&part, &copy).map_err(describe)?;
    Ok(copy.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_is_decoded() {
        let (path, size) = query("p=%2Fstorage%2Femulated%2F0%2FDCIM%2Fa%20b.jpg&s=1600");
        assert_eq!(path.as_deref(), Some("/storage/emulated/0/DCIM/a b.jpg"));
        assert_eq!(size, Some(1600));
    }

    #[test]
    fn a_broken_escape_is_no_path() {
        assert_eq!(query("p=%zz").0, None);
        assert_eq!(query("p=%2").0, None);
    }

    #[test]
    fn the_edge_is_held_to_what_a_screen_needs() {
        assert_eq!(edge(None), LIST_EDGE);
        assert_eq!(edge(Some(20_000)), MOST_EDGE);
        assert_eq!(edge(Some(1)), 64);
    }

    #[test]
    fn dot_dot_and_relative_paths_are_refused() {
        assert!(!allowed_local(
            Path::new("/storage/emulated/0/../../data/x.jpg"),
            None
        ));
        assert!(!allowed_local(Path::new("DCIM/x.jpg"), None));
    }

    #[test]
    fn kinds_without_a_picture_are_not_asked_for() {
        assert!(has_picture("/a/IMG_1.HEIC"));
        assert!(has_picture("/a/song.mp3"));
        assert!(has_picture("/a/app.apk"));
        assert!(!has_picture("/a/notes.txt"));
        assert!(!is_picture("/a/clip.mp4"));
    }

    #[test]
    fn the_gate_lets_only_so_many_through() {
        let gate = Arc::new(Gate::new(2));
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let (gate, inside, most) =
                    (Arc::clone(&gate), Arc::clone(&inside), Arc::clone(&most));
                std::thread::spawn(move || {
                    let _pass = gate.enter();
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for w in workers {
            w.join().expect("worker");
        }
        assert!(most.load(Ordering::SeqCst) <= 2);
    }
}
