//! A photo's look from its first bytes, without the rest of the file.
//!
//! Cameras and phones store a small preview picture inside a JPEG's EXIF,
//! which sits at the very start of the file. The difference hash shrinks every
//! picture to 17 by 16 pixels before comparing, so a 160 pixel preview carries
//! everything it needs. That makes a photo on a NAS cost its first few dozen
//! kilobytes rather than all of it.
//!
//! The print is the same arithmetic as a full decode's, under the same
//! algorithm name, so a preview of a camera's original compares with a full
//! decode of a smaller export of it. Two things are taken from the EXIF
//! rather than the preview, because the preview would get them wrong:
//! - the weight is the photo's real pixel count, so a group is still led by
//!   the biggest copy;
//! - black bars some cameras add to fit a preview into 160 by 120 are trimmed,
//!   or a 3:2 photo's preview would not match its own 3:2 export.

use std::io::Cursor;

use image::DynamicImage;

use crate::Shot;
use crate::picture::{jpeg, print_of};

/// How much of a file to read for its preview: EXIF lives in one APP1
/// segment, which cannot exceed 64 KiB, right after the file's first bytes.
pub const HEAD_BYTES: u64 = 66 * 1024;

/// Refuse a preview that claims to be bigger than this.
const MOST_PREVIEW_BYTES: usize = 1024 * 1024;

/// The print, and a small picture if asked, from a JPEG's embedded preview.
///
/// `None` when there is no preview to use: not a JPEG, no EXIF, no preview,
/// or one that would not decode. The caller then decides whether the whole
/// file is worth fetching.
#[must_use]
pub fn from_head(head: &[u8], want_thumb: bool) -> Option<Shot> {
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(head))
        .ok()?;
    let preview = embedded(&exif)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader =
        image::ImageReader::with_format(Cursor::new(preview), image::ImageFormat::Jpeg);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let picture = without_bars(&decoded);

    let mut print = print_of(&picture);
    if let Some(pixels) = full_size(&exif) {
        print.weight = pixels;
    }
    Some(Shot {
        print,
        thumb: want_thumb.then(|| jpeg(&picture)),
    })
}

/// The preview's bytes, where the EXIF says they are.
fn embedded(exif: &exif::Exif) -> Option<&[u8]> {
    let number = |tag| {
        exif.get_field(tag, exif::In::THUMBNAIL)
            .and_then(|field| field.value.get_uint(0))
            .and_then(|value| usize::try_from(value).ok())
    };
    let offset = number(exif::Tag::JPEGInterchangeFormat)?;
    let length = number(exif::Tag::JPEGInterchangeFormatLength)?;
    if length == 0 || length > MOST_PREVIEW_BYTES {
        return None;
    }
    exif.buf().get(offset..offset.checked_add(length)?)
}

/// The photo's own pixel count, from the EXIF.
fn full_size(exif: &exif::Exif) -> Option<u64> {
    let number = |tag| {
        exif.get_field(tag, exif::In::PRIMARY)
            .and_then(|field| field.value.get_uint(0))
            .map(u64::from)
    };
    let (width, height) = number(exif::Tag::PixelXDimension)
        .zip(number(exif::Tag::PixelYDimension))
        .or_else(|| number(exif::Tag::ImageWidth).zip(number(exif::Tag::ImageLength)))?;
    (width > 0 && height > 0).then(|| width * height)
}

/// Trim the black bars a camera adds to fit a preview into a fixed shape.
///
/// A bar is a run of rows or columns from an edge that are all nearly black.
/// Bars that together would take more than a third of the height (or the
/// width) are not bars: that is a dark photo, a night sky, and is left whole.
fn without_bars(picture: &DynamicImage) -> DynamicImage {
    const DARK: u8 = 16;
    let grey = picture.to_luma8();
    let (width, height) = grey.dimensions();
    let row_dark = |y: u32| (0..width).all(|x| grey.get_pixel(x, y).0[0] <= DARK);
    let col_dark = |x: u32| (0..height).all(|y| grey.get_pixel(x, y).0[0] <= DARK);
    let most_rows = height / 3;
    let most_cols = width / 3;
    let run = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
    let (mut top, mut bottom) = (
        run((0..most_rows).take_while(|&y| row_dark(y)).count()),
        run((0..most_rows)
            .take_while(|&y| row_dark(height - 1 - y))
            .count()),
    );
    let (mut left, mut right) = (
        run((0..most_cols).take_while(|&x| col_dark(x)).count()),
        run((0..most_cols)
            .take_while(|&x| col_dark(width - 1 - x))
            .count()),
    );
    if top + bottom > most_rows {
        (top, bottom) = (0, 0);
    }
    if left + right > most_cols {
        (left, right) = (0, 0);
    }
    if top + bottom + left + right == 0 {
        return picture.clone();
    }
    picture.crop_imm(left, top, width - left - right, height - top - bottom)
}

fn encode(picture: &DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .expect("encodes");
    out
}

/// A JPEG as a camera writes one: EXIF holding a preview and the photo's
/// size, then the photo itself. For tests and fixtures, here and in the
/// crates that read through a connection.
///
/// # Panics
/// If the EXIF block would not fit in one JPEG segment.
#[must_use]
pub fn camera_jpeg(photo: &DynamicImage, preview: &DynamicImage) -> Vec<u8> {
    use exif::experimental::Writer;
    let preview_bytes = encode(preview);
    let width = exif::Field {
        tag: exif::Tag::PixelXDimension,
        ifd_num: exif::In::PRIMARY,
        value: exif::Value::Long(vec![photo.width()]),
    };
    let height = exif::Field {
        tag: exif::Tag::PixelYDimension,
        ifd_num: exif::In::PRIMARY,
        value: exif::Value::Long(vec![photo.height()]),
    };
    let mut writer = Writer::new();
    writer.push_field(&width);
    writer.push_field(&height);
    writer.set_jpeg(&preview_bytes, exif::In::THUMBNAIL);
    let mut tiff = Cursor::new(Vec::new());
    writer.write(&mut tiff, false).expect("exif writes");
    let tiff = tiff.into_inner();

    let body = encode(photo);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    let length = u16::try_from(tiff.len() + 8).expect("fits in one segment");
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    // The photo's own segments, after its SOI.
    out.extend_from_slice(&body[2..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgb, RgbImage};

    /// A photograph-ish picture: a gradient with a bright block, so the
    /// difference hash has structure to find.
    #[allow(clippy::cast_possible_truncation)] // every shade is under 256
    fn scene(width: u32, height: u32) -> DynamicImage {
        let mut img = RgbImage::new(width, height);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            let shade = ((x * 255) / width) as u8;
            let block = x > width / 3 && x < width / 2 && y > height / 4 && y < height / 2;
            *pixel = if block {
                Rgb([250, 250, 250])
            } else {
                Rgb([shade, shade / 2, 255 - shade])
            };
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn a_preview_gives_the_same_look_as_the_whole_photo() {
        let photo = scene(1200, 800);
        let file = camera_jpeg(&photo, &photo.thumbnail(160, 120));
        let head = &file[..file
            .len()
            .min(usize::try_from(HEAD_BYTES).unwrap_or(usize::MAX))];

        let from_preview = from_head(head, false).expect("a preview is found");
        let from_photo = print_of(&photo);
        let alike = crate::picture::alike(&from_preview.print.detail, &from_photo.detail)
            .expect("comparable");
        assert!(alike >= 90, "{alike}% alike");
        assert_eq!(
            from_preview.print.weight,
            1200 * 800,
            "the photo's size, not the preview's"
        );
    }

    #[test]
    fn black_bars_around_a_preview_are_trimmed() {
        // A 3:2 photo's preview letterboxed into 160 by 120.
        let photo = scene(1500, 1000);
        let small = photo.thumbnail(160, 107);
        let mut boxed = RgbImage::new(160, 120);
        image::imageops::overlay(&mut boxed, &small.to_rgb8(), 0, 6);
        let file = camera_jpeg(&photo, &DynamicImage::ImageRgb8(boxed));

        let from_preview = from_head(&file, false).expect("a preview is found");
        let alike = crate::picture::alike(&from_preview.print.detail, &print_of(&photo).detail)
            .expect("comparable");
        assert!(alike >= 85, "{alike}% alike once the bars are gone");
    }

    #[test]
    fn trimming_takes_the_bars_and_leaves_the_picture() {
        let picture = scene(160, 107);
        let mut boxed = RgbImage::new(160, 120);
        image::imageops::overlay(&mut boxed, &picture.to_rgb8(), 0, 6);
        let trimmed = without_bars(&DynamicImage::ImageRgb8(boxed));
        assert_eq!(
            trimmed.dimensions(),
            (160, 107),
            "six rows off the top, seven off the bottom"
        );

        // A dark photo is not a bar: never more than a third from a side.
        let night = DynamicImage::ImageRgb8(RgbImage::new(160, 120));
        assert_eq!(without_bars(&night).dimensions(), (160, 120));
        // And a picture with no bars is untouched.
        assert_eq!(without_bars(&picture).dimensions(), (160, 107));
    }

    #[test]
    fn a_file_without_a_preview_has_none() {
        let photo = scene(300, 200);
        assert!(
            from_head(&encode(&photo), false).is_none(),
            "a JPEG with no EXIF"
        );
        assert!(from_head(b"not a picture at all", false).is_none());
    }
}
