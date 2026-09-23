//! Made-up photographs and made-up music, because a test that needs a real
//! holiday video is a test that never runs on anybody else's machine.

// Drawing a picture and writing a tune is arithmetic on pixel and sample
// counts. The same allowance the modules under test make, for the same reason.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::f32::consts::TAU;
use std::path::Path;

use image::{DynamicImage, RgbImage};

use crate::{Kind, PICTURE, Print, SOUND, Trouble, alike, kind_of, look};

/// A smooth picture with structure in it, at any size.
///
/// Smooth on purpose: a difference hash is about broad light and dark, and a
/// pattern of single pixels would be destroyed by the resize this test is
/// about rather than by any flaw in the hash.
fn picture(width: u32, height: u32, tune: f32) -> DynamicImage {
    let image = RgbImage::from_fn(width, height, |x, y| {
        let across = x as f32 / width as f32;
        let down = y as f32 / height as f32;
        let value = (across * TAU * tune).sin() * (down * TAU * (tune + 1.0)).cos();
        let level = ((value + 1.0) * 127.5) as u8;
        image::Rgb([level, level / 2, 255 - level])
    });
    DynamicImage::ImageRgb8(image)
}

fn write_picture(dir: &Path, name: &str, image: &DynamicImage) -> std::path::PathBuf {
    let path = dir.join(name);
    image.save(&path).expect("save");
    path
}

/// Ten seconds of a repeating tune, as a 16 bit mono WAV.
///
/// Hand-rolled rather than through a crate: a WAV header is 44 bytes and this
/// is the only place in the project that needs to write one.
fn write_tune(dir: &Path, name: &str, rate: u32, notes: &[f32]) -> std::path::PathBuf {
    let seconds = 10;
    let count = rate * seconds;
    let mut samples = Vec::with_capacity(count as usize);
    for index in 0..count {
        let at = index as f32 / rate as f32;
        let note = notes[(at as usize) % notes.len()];
        let value = (at * note * TAU).sin() * 0.6;
        samples.push((value * f32::from(i16::MAX)) as i16);
    }
    let bytes: Vec<u8> = samples.iter().flat_map(|one| one.to_le_bytes()).collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // one channel
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    wav.extend_from_slice(&bytes);
    let path = dir.join(name);
    std::fs::write(&path, wav).expect("write");
    path
}

#[test]
fn a_photograph_saved_smaller_is_the_same_photograph() {
    let dir = tempfile::tempdir().expect("dir");
    let full = write_picture(&dir.path().join("."), "full.png", &picture(800, 600, 3.0));
    let small = write_picture(&dir.path().join("."), "small.jpg", &picture(400, 300, 3.0));

    let one = look(&full, Kind::Picture, false).expect("full").print;
    let other = look(&small, Kind::Picture, false).expect("small").print;

    let score = alike(&one, &other).expect("comparable");
    assert!(score >= 90, "a half-size copy scored {score}");
}

#[test]
fn two_different_photographs_are_not_confused() {
    let dir = tempfile::tempdir().expect("dir");
    let one = write_picture(dir.path(), "one.png", &picture(600, 600, 2.0));
    let other = write_picture(dir.path(), "other.png", &picture(600, 600, 9.0));

    let one = look(&one, Kind::Picture, false).expect("one").print;
    let other = look(&other, Kind::Picture, false).expect("other").print;

    let score = alike(&one, &other).expect("comparable");
    assert!(score < 60, "two different pictures scored {score}");
}

#[test]
fn a_thumbnail_is_a_jpeg_and_is_small() {
    let dir = tempfile::tempdir().expect("dir");
    let path = write_picture(dir.path(), "big.png", &picture(2000, 1500, 4.0));

    let shot = look(&path, Kind::Picture, true).expect("look");
    let thumb = shot.thumb.expect("a picture has a thumbnail");

    assert_eq!(&thumb[..2], &[0xff, 0xd8], "JPEG starts with ff d8");
    assert!(thumb.len() < 120_000, "thumbnail was {} bytes", thumb.len());
}

#[test]
fn the_same_tune_recorded_at_another_rate_is_the_same_tune() {
    let dir = tempfile::tempdir().expect("dir");
    let notes = [440.0, 554.0, 659.0, 523.0, 392.0, 440.0, 330.0, 494.0];
    let one = write_tune(dir.path(), "cd.wav", 44100, &notes);
    let other = write_tune(dir.path(), "radio.wav", 32000, &notes);

    let one = look(&one, Kind::Sound, false).expect("one").print;
    let other = look(&other, Kind::Sound, false).expect("other").print;

    let score = alike(&one, &other).expect("comparable");
    assert!(score >= 70, "one tune at two rates scored {score}");
}

#[test]
fn two_different_tunes_are_not_confused() {
    let dir = tempfile::tempdir().expect("dir");
    let one = write_tune(dir.path(), "one.wav", 44100, &[440.0, 554.0, 659.0, 523.0]);
    let other = write_tune(dir.path(), "other.wav", 44100, &[110.0, 147.0, 98.0, 131.0]);

    let one = look(&one, Kind::Sound, false).expect("one").print;
    let other = look(&other, Kind::Sound, false).expect("other").print;

    // No comparable stretch at all is the usual answer here, and a weak one is
    // the other acceptable answer. What must not happen is a strong one.
    let score = alike(&one, &other).unwrap_or(0);
    assert!(score < 70, "two different tunes scored {score}");
}

#[test]
fn a_print_survives_being_written_down() {
    let print = Print {
        algo: PICTURE,
        signature: 0x0123_4567_89ab_cdef,
        detail: vec![1, u64::MAX, 0],
    };
    let written = print.encode();
    assert_eq!(Print::decode(&written, PICTURE).as_ref(), Some(&print));
    assert_eq!(Print::algorithm(&written), Some(PICTURE));
}

#[test]
fn a_print_from_another_algorithm_is_never_read_back() {
    let written = Print {
        algo: SOUND,
        signature: 7,
        detail: vec![9],
    }
    .encode();

    assert!(Print::decode(&written, PICTURE).is_none());
}

#[test]
fn prints_of_different_kinds_are_never_compared() {
    let one = Print {
        algo: PICTURE,
        signature: 0,
        detail: vec![0, 0, 0, 0],
    };
    let other = Print {
        algo: SOUND,
        signature: 0,
        detail: vec![0, 0, 0, 0],
    };

    assert_eq!(alike(&one, &other), None);
}

#[test]
fn something_that_is_not_a_picture_is_named_rather_than_skipped() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("notes.png");
    std::fs::write(&path, b"this is not a picture").expect("write");

    match look(&path, Kind::Picture, false) {
        Err(Trouble::Unreadable(_) | Trouble::Unsupported(_)) => {}
        other => panic!("expected to be told why, got {other:?}"),
    }
}

#[test]
fn a_kind_comes_from_the_media_type_first_and_the_name_second() {
    assert_eq!(
        kind_of(Some("image/jpeg"), "holiday.bin"),
        Some(Kind::Picture)
    );
    assert_eq!(kind_of(None, "IMG_4471.HEIC"), Some(Kind::Picture));
    assert_eq!(kind_of(None, "final cut.MOV"), Some(Kind::Moving));
    assert_eq!(kind_of(None, "track.flac"), Some(Kind::Sound));
    assert_eq!(kind_of(Some("application/pdf"), "invoice.pdf"), None);
    assert_eq!(kind_of(None, "notes"), None);
}
