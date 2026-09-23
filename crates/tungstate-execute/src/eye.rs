//! Looking at files for the near-duplicate pass, and remembering what was seen.
//!
//! The twin of [`crate::digest`], for the other pass. Same bargain: the pure
//! side in [`tungstate_core::similar`] asks, this reads, and every answer goes
//! into the journal keyed by the path and checked against size and mtime.
//!
//! The bargain matters more here. A digest of a folder nobody touched costs
//! nothing the second time because most files are never read at all; a
//! fingerprint has to decode every picture and every video the first time, and
//! the cache is the only thing standing between the second scan and all of
//! that work again.
//!
//! **Local files only.** Decoding needs the whole file, and there is no
//! sampled shortcut the way there is for a digest, so pointing this at a
//! connection would pull every video across the network. The refusal lives
//! where the scan is started, not here.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tungstate_core::dupes::STOPPED;
use tungstate_core::similar::{Look, Mark, Sort};
use tungstate_journal::{Journal, Remembered};
use tungstate_likeness::{Kind, Print};

use crate::thumbs::Thumbs;

/// Told how far a pass has got.
type Watcher<'a> = Box<dyn FnMut(&Seen) + Send + 'a>;

/// Asked before every file, so a scan can be abandoned.
type Asked<'a> = Box<dyn Fn() -> bool + Send + 'a>;

/// How far a pass has got, for whoever is watching it.
#[derive(Debug, Clone, Default)]
pub struct Seen {
    /// Files fingerprinted so far, however the answer was arrived at.
    pub looked: usize,
    /// Files opened and decoded.
    pub decoded: usize,
    /// Files whose fingerprint came from the journal.
    pub recalled: usize,
    /// The file being handled right now.
    pub path: String,
    /// What kind of file that is, so the window can say *pictures* while it
    /// is on pictures.
    pub sort: Option<Sort>,
}

/// A [`Look`] that decodes through the filesystem and remembers what it found.
pub struct Eye<'a> {
    root: PathBuf,
    journal: &'a Journal,
    /// Where small pictures go, when anybody wants them.
    thumbs: Option<&'a Thumbs>,
    decoded: usize,
    recalled: usize,
    looked: usize,
    watcher: Option<Watcher<'a>>,
    stop: Option<Asked<'a>>,
}

impl<'a> Eye<'a> {
    /// An eye for one local folder.
    #[must_use]
    pub fn new(root: &Path, journal: &'a Journal) -> Self {
        Self {
            root: root.to_path_buf(),
            journal,
            thumbs: None,
            decoded: 0,
            recalled: 0,
            looked: 0,
            watcher: None,
            stop: None,
        }
    }

    /// Keep a small picture of everything that has one.
    #[must_use]
    pub fn keeping_pictures(mut self, thumbs: &'a Thumbs) -> Self {
        self.thumbs = Some(thumbs);
        self
    }

    /// Report progress as the pass goes.
    #[must_use]
    pub fn watched_by(mut self, watcher: Watcher<'a>) -> Self {
        self.watcher = Some(watcher);
        self
    }

    /// Stop when `stop` says so. Checked before every file.
    #[must_use]
    pub fn stopping_when(mut self, stop: Asked<'a>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// How many files were decoded rather than recalled.
    #[must_use]
    pub fn decoded(&self) -> usize {
        self.decoded
    }

    /// How many fingerprints came from the journal.
    #[must_use]
    pub fn recalled(&self) -> usize {
        self.recalled
    }

    /// Where a thumbnail for this file would be, if one was kept.
    #[must_use]
    pub fn picture_of(&self, path: &str) -> Option<PathBuf> {
        let thumbs = self.thumbs?;
        let (size, mtime) = self.stamp(path).ok()?;
        let at = thumbs.at_name(&Thumbs::name(
            &self.root.to_string_lossy(),
            path,
            size,
            mtime,
        ));
        at.exists().then_some(at)
    }

    fn stamp(&self, path: &str) -> Result<(u64, Option<i64>), String> {
        let meta = std::fs::metadata(self.root.join(path)).map_err(|error| error.to_string())?;
        let mtime = meta.modified().ok().and_then(|at| {
            at.duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|since| i64::try_from(since.as_secs()).ok())
        });
        Ok((meta.len(), mtime))
    }
}

/// The kinds, in both directions. Two enums rather than one because core must
/// not depend on a crate that decodes pictures, and the cost of that is this
/// function.
fn as_sort(kind: Kind) -> Sort {
    match kind {
        Kind::Picture => Sort::Picture,
        Kind::Moving => Sort::Moving,
        Kind::Sound => Sort::Sound,
    }
}

fn as_kind(sort: Sort) -> Kind {
    match sort {
        Sort::Picture => Kind::Picture,
        Sort::Moving => Kind::Moving,
        Sort::Sound => Kind::Sound,
    }
}

impl Look for Eye<'_> {
    fn sort(&self, path: &str, mime: Option<&str>) -> Option<Sort> {
        let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
        tungstate_likeness::kind_of(mime, name).map(as_sort)
    }

    fn print(&mut self, path: &str) -> Result<Mark, String> {
        if self.stop.as_ref().is_some_and(|asked| asked()) {
            return Err(STOPPED.to_string());
        }
        let sort = self
            .sort(path, None)
            .ok_or_else(|| "not a kind this looks at".to_string())?;
        self.looked += 1;
        if let Some(watcher) = self.watcher.as_mut() {
            watcher(&Seen {
                looked: self.looked,
                decoded: self.decoded,
                recalled: self.recalled,
                path: path.to_string(),
                sort: Some(sort),
            });
        }

        let (size, mtime) = self.stamp(path)?;
        let root = self.root.to_string_lossy().to_string();
        let kept = self
            .journal
            .remembered(&root, path, size, mtime)
            .map_err(|error| error.to_string())?
            .and_then(|kept| kept.print);
        // A stored print names its own algorithm, so one written by an older
        // version of the arithmetic simply fails to parse and is taken again.
        // That is the whole of the invalidation for a change to the maths.
        if let Some(mark) = kept.as_deref().and_then(|stored| from_stored(stored, sort)) {
            self.recalled += 1;
            // A thumbnail is not in the journal, so a cache hit still has to
            // check the picture is there: a scan with pictures turned on after
            // a scan without one would otherwise show nothing.
            if self.thumbs.is_some() && self.picture_of(path).is_none() {
                let _ = self.keep_picture(path, sort, size, mtime);
            }
            return Ok(mark);
        }

        let want_thumb = self.thumbs.is_some();
        let shot = tungstate_likeness::look(&self.root.join(path), as_kind(sort), want_thumb)
            .map_err(|trouble| trouble.to_string())?;
        self.decoded += 1;
        self.journal
            .remember(
                &root,
                path,
                size,
                mtime,
                &Remembered {
                    print: Some(shot.print.encode()),
                    ..Remembered::default()
                },
            )
            .map_err(|error| error.to_string())?;
        if let (Some(thumbs), Some(bytes)) = (self.thumbs, shot.thumb.as_deref()) {
            let _ = thumbs.keep(&Thumbs::name(&root, path, size, mtime), bytes);
        }
        Ok(as_mark(&shot.print))
    }

    fn alike(&self, one: &Mark, other: &Mark) -> Option<u8> {
        if one.algo != other.algo {
            return None;
        }
        tungstate_likeness::alike_parts(&one.algo, &one.detail, &other.detail)
    }
}

impl Eye<'_> {
    /// Decode one file purely to draw it, when its numbers were already known.
    fn keep_picture(
        &mut self,
        path: &str,
        sort: Sort,
        size: u64,
        mtime: Option<i64>,
    ) -> Result<(), String> {
        let Some(thumbs) = self.thumbs else {
            return Ok(());
        };
        let shot = tungstate_likeness::look(&self.root.join(path), as_kind(sort), true)
            .map_err(|trouble| trouble.to_string())?;
        if let Some(bytes) = shot.thumb.as_deref() {
            let root = self.root.to_string_lossy().to_string();
            thumbs
                .keep(&Thumbs::name(&root, path, size, mtime), bytes)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn as_mark(print: &Print) -> Mark {
    Mark {
        algo: print.algo.to_string(),
        signature: print.signature,
        detail: print.detail.clone(),
        weight: print.weight,
    }
}

/// Read a stored print back, but only as one of the algorithms this kind of
/// file is allowed to have produced.
fn from_stored(stored: &str, sort: Sort) -> Option<Mark> {
    let allowed: &[&'static str] = match sort {
        Sort::Picture => &[tungstate_likeness::PICTURE],
        Sort::Moving => &[
            tungstate_likeness::MOVING_SOUND,
            tungstate_likeness::MOVING_FRAMES,
        ],
        Sort::Sound => &[tungstate_likeness::SOUND],
    };
    allowed
        .iter()
        .find_map(|algo| Print::decode(stored, algo))
        .map(|print| as_mark(&print))
}
