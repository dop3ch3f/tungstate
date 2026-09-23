//! What a file looks and sounds like, as numbers that survive re-encoding.
//!
//! Slice 8b answers *are these the same bytes*. This answers *is this the same
//! photograph*, which no digest can: change one pixel and BLAKE3 changes
//! everything, which is the whole point of a digest and the whole problem here.
//!
//! Two numbers per file, for the same reason 8b has two tiers:
//!
//! 1. A **signature**, 64 bits, cheap to compare and cheap to index. Finding
//!    everything within a distance of one signature is a tree walk.
//! 2. The **detail**, which is what actually decides. Only pairs the signature
//!    brought together are ever compared this way.
//!
//! Nothing here caches, reads a journal or knows what a folder is. It is given
//! a path and hands back numbers.

use std::path::Path;

mod picture;
mod sound;
#[cfg(test)]
mod tests;
mod video;

pub use picture::THUMB_EDGE;

/// The kinds of file worth looking at this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A still picture.
    Picture,
    /// Moving pictures, with or without sound.
    Moving,
    /// Sound on its own.
    Sound,
}

/// Anything that stops a file being fingerprinted.
///
/// Every one of these is *counted and named* rather than swallowed: a finder
/// that silently skips what it cannot read tells you your drive is clean when
/// it has not looked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Trouble {
    /// The file could not be opened or read.
    #[error("could not read it: {0}")]
    Unreadable(String),
    /// The format is one nothing here can decode.
    #[error("nothing here can read a {0}")]
    Unsupported(String),
    /// A video with no sound, on a machine with no ffmpeg to look at frames.
    #[error("{0}")]
    NoDecoder(String),
    /// There was nothing to fingerprint: an empty file, or silence.
    #[error("there is nothing in it to compare")]
    Empty,
}

/// A file's likeness, as numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Print {
    /// Which algorithm produced it. Part of the stored string, and checked
    /// before any comparison: **two prints from different algorithms are
    /// never compared**, which is the mistake DESIGN §5 warns about for
    /// BLAKE3 against MD5.
    pub algo: &'static str,
    /// The 64 bits a tree indexes.
    pub signature: u64,
    /// How much of the thing there is: pixels for a picture, fingerprint
    /// items for sound. Not part of the comparison. It decides which copy
    /// leads a group, because the copy with the most in it is the one worth
    /// keeping, and it is stored with the print so that a second scan can
    /// answer that without opening the file again.
    pub weight: u64,
    /// What decides a pair the signature brought together.
    pub detail: Vec<u64>,
}

/// A difference hash of a picture, at two sizes. Version 1.
pub const PICTURE: &str = "pic1";
/// A chromaprint of sound. Version 1.
pub const SOUND: &str = "snd1";
/// A video, fingerprinted through its soundtrack. Version 1.
pub const MOVING_SOUND: &str = "vid1";
/// A video, fingerprinted from frames ffmpeg handed over. Version 1.
pub const MOVING_FRAMES: &str = "vidf1";

impl Print {
    /// The stored form: the algorithm, then the numbers, all in hex.
    ///
    /// Self-describing on purpose. A print written by a later version of an
    /// algorithm fails to match this `algo` and is recomputed, so a change to
    /// the arithmetic invalidates the cache without a migration.
    #[must_use]
    pub fn encode(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!(
            "{}:{:016x}:{:016x}:",
            self.algo, self.signature, self.weight
        );
        for word in &self.detail {
            let _ = write!(out, "{word:016x}");
        }
        out
    }

    /// Read back what [`encode`](Self::encode) wrote, for one known algorithm.
    ///
    /// Returns `None` for anything else, including a print of the same kind
    /// from a different version. That is the invalidation.
    #[must_use]
    pub fn decode(stored: &str, algo: &'static str) -> Option<Self> {
        let rest = stored.strip_prefix(algo)?.strip_prefix(':')?;
        let (signature, rest) = rest.split_once(':')?;
        let (weight, detail) = rest.split_once(':')?;
        let signature = u64::from_str_radix(signature, 16).ok()?;
        let weight = u64::from_str_radix(weight, 16).ok()?;
        if !detail.len().is_multiple_of(16) {
            return None;
        }
        let detail = detail
            .as_bytes()
            .chunks(16)
            .map(|word| u64::from_str_radix(std::str::from_utf8(word).ok()?, 16).ok())
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            algo,
            signature,
            weight,
            detail,
        })
    }

    /// Which algorithm a stored string came from, without decoding it.
    #[must_use]
    pub fn algorithm(stored: &str) -> Option<&str> {
        stored.split_once(':').map(|(algo, _)| algo)
    }
}

/// How alike two files are, from 0 to 100, or `None` when the two prints
/// cannot be compared at all.
///
/// The two video algorithms are deliberately not comparable with each other:
/// a fingerprint of a soundtrack and a fingerprint of frames describe
/// different things, and a machine that gains ffmpeg halfway through a library
/// must not start comparing them.
#[must_use]
pub fn alike(one: &Print, other: &Print) -> Option<u8> {
    if one.algo != other.algo {
        return None;
    }
    alike_parts(one.algo, &one.detail, &other.detail)
}

/// [`alike`], for a caller holding the pieces rather than a [`Print`].
///
/// This exists so the pass in `tungstate-core` can compare its own plain data
/// without building a `Print` first. Comparison happens once per candidate
/// pair, and a clone of a fingerprint per pair would be the most expensive
/// thing in the whole search.
#[must_use]
pub fn alike_parts(algo: &str, one: &[u64], other: &[u64]) -> Option<u8> {
    match algo {
        PICTURE | MOVING_FRAMES => picture::alike(one, other),
        SOUND | MOVING_SOUND => sound::alike(one, other),
        _ => None,
    }
}

/// Bits that differ between two signatures.
#[must_use]
pub fn apart(one: u64, other: u64) -> u32 {
    (one ^ other).count_ones()
}

/// What one look at a file produced.
#[derive(Debug, Clone)]
pub struct Shot {
    /// The numbers.
    pub print: Print,
    /// A small JPEG of it, where there was a picture to make one from. The
    /// caller decides where that goes; this crate keeps no cache.
    pub thumb: Option<Vec<u8>>,
}

/// Fingerprint one file.
///
/// `want_thumb` is honoured where a picture exists. It costs one resize on top
/// of a decode that has already happened, which is why the window can afford
/// a picture beside every row.
///
/// # Errors
/// [`Trouble`], which the caller is expected to count and show rather than
/// treat as the end of the scan.
pub fn look(path: &Path, kind: Kind, want_thumb: bool) -> Result<Shot, Trouble> {
    match kind {
        // A picture nothing here can decode is still a picture: an iPhone's
        // HEIC is the one everybody has. ffmpeg turns it into pixels, and the
        // pixels are hashed by the same arithmetic, so the answer is
        // comparable with an ordinary JPEG of the same photograph.
        Kind::Picture => match picture::look(path, want_thumb) {
            Err(Trouble::Unsupported(what)) => {
                let frame = video::still(path).map_err(|_| Trouble::Unsupported(what))?;
                Ok(Shot {
                    print: picture::print_of(&frame),
                    thumb: want_thumb.then(|| picture::jpeg(&frame)),
                })
            }
            other => other,
        },
        Kind::Sound => sound::look(path),
        Kind::Moving => video::look(path, want_thumb),
    }
}

/// Which kind of file this is, from its media type and its name.
///
/// The media type is what `tungstate-attrs` already read from the first bytes;
/// the extension is the fallback for the formats `infer` does not know.
#[must_use]
pub fn kind_of(mime: Option<&str>, name: &str) -> Option<Kind> {
    if let Some(mime) = mime {
        let kind = match mime.split('/').next() {
            Some("image") => Some(Kind::Picture),
            Some("video") => Some(Kind::Moving),
            Some("audio") => Some(Kind::Sound),
            _ => None,
        };
        if kind.is_some() {
            return kind;
        }
    }
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "tif" | "tiff" | "bmp" | "ico" | "heic"
        | "heif" | "avif" | "dng" | "cr2" | "nef" | "arw" | "raf" | "orf" => Some(Kind::Picture),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "mpg" | "mpeg"
        | "m2ts" | "3gp" => Some(Kind::Moving),
        "mp3" | "m4a" | "aac" | "flac" | "ogg" | "oga" | "opus" | "wav" | "aiff" | "aif"
        | "wma" | "alac" | "ape" => Some(Kind::Sound),
        _ => None,
    }
}
