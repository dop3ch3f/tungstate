//! Video, through its soundtrack where there is one.
//!
//! Decoding video frames means a decoder written in C, and requiring one would
//! make this feature missing on most machines. A re-encoded video keeps its
//! sound recognisable, and `symphonia` reads the audio track out of an mp4 or
//! an mkv in pure Rust, so the cheapest honest fingerprint of a video is the
//! fingerprint of what it sounds like.
//!
//! Frames are the fallback, not the plan: a silent video, or one whose audio
//! codec is not among the ones compiled in, is handed to `ffmpeg` if the
//! machine has it. ffmpeg is found, never installed and never bundled, and its
//! absence costs one case rather than the feature.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use image::DynamicImage;

use crate::{MOVING_FRAMES, MOVING_SOUND, Print, Shot, Trouble, picture, sound};

/// Where in a video the frames are taken from, as fractions of its length.
///
/// Fractions rather than seconds, so a re-encode that changes the framerate
/// still lines up. Not the very start or the very end, which are titles and
/// black on most things.
const AT: [f32; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];

pub fn look(path: &Path, want_thumb: bool) -> Result<Shot, Trouble> {
    let thumb = want_thumb
        .then(|| still(path).ok().map(|frame| shrink(&frame)))
        .flatten();
    match sound::print_of(path, MOVING_SOUND) {
        Ok(print) => Ok(Shot { print, thumb }),
        Err(quiet) => match from_frames(path) {
            Ok(print) => Ok(Shot { print, thumb }),
            // The sound side's complaint is the more useful of the two: it says
            // what the file is, where the frame side only says ffmpeg is absent.
            Err(Trouble::NoDecoder(missing)) => Err(Trouble::NoDecoder(format!(
                "{quiet}, and {missing} to look at the picture instead"
            ))),
            Err(other) => Err(other),
        },
    }
}

/// Whether ffmpeg is on this machine. Asked once.
pub fn ffmpeg() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// One picture out of a file ffmpeg can read and `image` cannot.
///
/// Used for the thumbnail of a video, and for the pictures the pure Rust
/// decoders do not cover: an iPhone's HEIC is the one everybody has.
pub fn still(path: &Path) -> Result<DynamicImage, Trouble> {
    frame_at(path, None)
}

/// Fingerprint a video from frames, in the same arithmetic the picture side
/// uses, so a frame hash means the same thing a photograph's hash means.
fn from_frames(path: &Path) -> Result<Print, Trouble> {
    let length = length(path)?;
    let mut detail = Vec::with_capacity(AT.len());
    for fraction in AT {
        let frame = frame_at(path, Some(length * fraction))?;
        detail.push(picture::print_of(&frame).signature);
    }
    Ok(Print {
        algo: MOVING_FRAMES,
        // The middle of a video is the least likely part of it to be a title
        // card, which makes it the most useful thing to index.
        signature: detail[AT.len() / 2],
        detail,
    })
}

/// How long the video is, in seconds, according to ffprobe.
fn length(path: &Path) -> Result<f32, Trouble> {
    let out = run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nokey=1",
            &path.to_string_lossy(),
        ],
    )?;
    String::from_utf8_lossy(&out)
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|seconds| *seconds > 0.0)
        .ok_or(Trouble::Empty)
}

/// One frame, decoded. `at` is seconds in; `None` takes an early one.
fn frame_at(path: &Path, at: Option<f32>) -> Result<DynamicImage, Trouble> {
    let seek = format!("{:.3}", at.unwrap_or(0.0));
    let png = run(
        "ffmpeg",
        &[
            // -ss before -i seeks rather than decoding up to the point, which
            // is the difference between milliseconds and minutes on a long file.
            "-ss",
            &seek,
            "-i",
            &path.to_string_lossy(),
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-",
        ],
    )?;
    if png.is_empty() {
        return Err(Trouble::Empty);
    }
    image::load_from_memory(&png).map_err(|error| Trouble::Unreadable(error.to_string()))
}

/// Run one of ffmpeg's tools and hand back what it wrote.
fn run(tool: &str, args: &[&str]) -> Result<Vec<u8>, Trouble> {
    if !ffmpeg() {
        return Err(Trouble::NoDecoder(
            "ffmpeg is not on this machine".to_string(),
        ));
    }
    let out = Command::new(tool)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .map_err(|error| Trouble::NoDecoder(error.to_string()))?;
    if !out.status.success() {
        return Err(Trouble::Unsupported(format!("{tool} could not read it")));
    }
    Ok(out.stdout)
}

/// A frame, at the size the window wants it.
fn shrink(frame: &DynamicImage) -> Vec<u8> {
    picture::jpeg(frame)
}
