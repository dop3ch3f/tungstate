//! Pictures, by difference hash.
//!
//! Shrink to a handful of pixels, throw the colour away, and record whether
//! each pixel is brighter than the one to its right. That survives resizing,
//! re-compression and a change of format, because all three leave the broad
//! light and dark of a photograph alone, and it is a handful of arithmetic
//! rather than a model.
//!
//! Two sizes from one decode: 9 by 8 gives the 64 bit signature a tree can
//! index, 17 by 16 gives the 256 bits that decide.
// Every cast below turns a count of bits or samples into a score between 0 and
// 100. The precision a f32 loses at that scale is smaller than the rounding.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, GrayImage};

use crate::{PICTURE, Print, Shot, Trouble};

/// The longest edge of the small picture kept for the window.
pub const THUMB_EDGE: u32 = 480;

/// Refuse anything that would decode to more than this many bytes.
///
/// A decoder is a parser and a parser is where somebody else's malformed file
/// does something surprising. This is the one dial that keeps a 400 megapixel
/// header from asking for all of the memory.
const MOST_BYTES: u64 = 512 * 1024 * 1024;

pub fn look(path: &Path, want_thumb: bool) -> Result<Shot, Trouble> {
    let full = decode(path)?;
    let print = print_of(&full);
    let thumb = want_thumb.then(|| jpeg(&full));
    Ok(Shot { print, thumb })
}

/// Decode one picture, with the limits on.
pub fn decode(path: &Path) -> Result<DynamicImage, Trouble> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| Trouble::Unreadable(error.to_string()))?
        .with_guessed_format()
        .map_err(|error| Trouble::Unreadable(error.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MOST_BYTES);
    reader.limits(limits);
    reader.decode().map_err(|error| match error {
        image::ImageError::Unsupported(what) => Trouble::Unsupported(what.to_string()),
        other => Trouble::Unreadable(other.to_string()),
    })
}

/// The signature and the detail, from one decoded picture.
pub fn print_of(full: &DynamicImage) -> Print {
    let grey = full.to_luma8();
    Print {
        algo: PICTURE,
        signature: gradient(&grey, 8).first().copied().unwrap_or_default(),
        detail: gradient(&grey, 16),
    }
}

/// A small JPEG for the window, longest edge [`THUMB_EDGE`].
pub fn jpeg(full: &DynamicImage) -> Vec<u8> {
    let small = full.resize(THUMB_EDGE, THUMB_EDGE, FilterType::Triangle);
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 78);
    // An encode into a Vec cannot fail for want of space, and a picture that
    // decoded will encode. Nothing useful to do with the error but drop it.
    let _ = encoder.encode_image(&small.into_rgb8());
    out
}

/// `edge` rows of `edge` "is this pixel brighter than its right neighbour"
/// bits, packed 64 to a word.
fn gradient(grey: &GrayImage, edge: u32) -> Vec<u64> {
    let small = image::imageops::resize(grey, edge + 1, edge, FilterType::Triangle);
    let mut words = vec![0u64; (edge * edge).div_ceil(64) as usize];
    let mut bit = 0usize;
    for y in 0..edge {
        for x in 0..edge {
            if small.get_pixel(x, y).0[0] > small.get_pixel(x + 1, y).0[0] {
                words[bit / 64] |= 1 << (bit % 64);
            }
            bit += 1;
        }
    }
    words
}

/// How alike two pictures are, as a percentage of the detail bits that agree.
pub fn alike(one: &Print, other: &Print) -> Option<u8> {
    if one.detail.len() != other.detail.len() || one.detail.is_empty() {
        return None;
    }
    let bits = one.detail.len() * 64;
    let differ: u32 = one
        .detail
        .iter()
        .zip(&other.detail)
        .map(|(a, b)| (a ^ b).count_ones())
        .sum();
    // Half the bits differing is what two unrelated pictures score, so the
    // scale is stretched to put that at zero rather than at fifty.
    let agree = bits as f32 - differ as f32;
    let scaled = (agree / bits as f32).mul_add(200.0, -100.0);
    Some(scaled.clamp(0.0, 100.0).round() as u8)
}
