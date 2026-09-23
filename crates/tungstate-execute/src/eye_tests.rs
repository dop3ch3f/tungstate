//! The eye, against real files on a real disk.
//!
//! This is the layer where the decoders, the journal and the pure pass meet,
//! so the tests are about the meeting: that a second look costs nothing, that
//! an edited file is looked at again, and that a stop is a stop.

use std::f32::consts::TAU;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use image::{DynamicImage, RgbImage};
use tungstate_core::dupes::STOPPED;
use tungstate_core::similar::{Look, Sort};
use tungstate_journal::Journal;

use crate::eye::Eye;
use crate::thumbs::Thumbs;

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn draw(dir: &Path, name: &str, tune: f32) {
    let image = RgbImage::from_fn(240, 180, |x, y| {
        let value = ((x as f32 / 240.0) * TAU * tune).sin() * ((y as f32 / 180.0) * TAU).cos();
        let level = ((value + 1.0) * 127.5) as u8;
        image::Rgb([level, 120, 255 - level])
    });
    DynamicImage::ImageRgb8(image)
        .save(dir.join(name))
        .expect("save");
}

#[test]
fn the_second_look_at_a_folder_decodes_nothing() {
    let dir = tempfile::tempdir().expect("dir");
    draw(dir.path(), "one.png", 3.0);
    let journal = Journal::open_in_memory().expect("journal");

    let mut eye = Eye::new(dir.path(), &journal);
    let first = eye.print("one.png").expect("a fingerprint");
    assert_eq!(eye.decoded(), 1);
    assert_eq!(eye.recalled(), 0);

    let mut again = Eye::new(dir.path(), &journal);
    let second = again.print("one.png").expect("a fingerprint");

    assert_eq!(again.decoded(), 0, "the second look decoded nothing");
    assert_eq!(again.recalled(), 1);
    assert_eq!(first, second, "and got the same answer");
}

#[test]
fn a_picture_that_changed_is_looked_at_again() {
    let dir = tempfile::tempdir().expect("dir");
    draw(dir.path(), "one.png", 3.0);
    let journal = Journal::open_in_memory().expect("journal");
    let mut eye = Eye::new(dir.path(), &journal);
    let first = eye.print("one.png").expect("a fingerprint");

    // Same name, different picture, which is what an edit in place looks like.
    draw(dir.path(), "one.png", 11.0);
    let mut again = Eye::new(dir.path(), &journal);
    let second = again.print("one.png").expect("a fingerprint");

    assert_eq!(again.decoded(), 1, "a changed file is not recalled");
    assert_ne!(first.signature, second.signature);
}

#[test]
fn a_kept_picture_lands_where_the_window_can_find_it() {
    let dir = tempfile::tempdir().expect("dir");
    draw(dir.path(), "one.png", 4.0);
    let journal = Journal::open_in_memory().expect("journal");
    let thumbs = Thumbs::at(&dir.path().join("cache")).expect("cache");

    let mut eye = Eye::new(dir.path(), &journal).keeping_pictures(&thumbs);
    eye.print("one.png").expect("a fingerprint");

    let picture = eye.picture_of("one.png").expect("a picture was kept");
    let bytes = std::fs::read(&picture).expect("read it back");
    assert_eq!(&bytes[..2], &[0xff, 0xd8], "a JPEG");
}

#[test]
fn a_recalled_fingerprint_still_gets_its_picture() {
    // A scan without pictures, then one with. The numbers are in the journal
    // and the picture is not, so the second scan has to notice that.
    let dir = tempfile::tempdir().expect("dir");
    draw(dir.path(), "one.png", 5.0);
    let journal = Journal::open_in_memory().expect("journal");
    Eye::new(dir.path(), &journal)
        .print("one.png")
        .expect("a fingerprint");

    let thumbs = Thumbs::at(&dir.path().join("cache")).expect("cache");
    let mut eye = Eye::new(dir.path(), &journal).keeping_pictures(&thumbs);
    eye.print("one.png").expect("a fingerprint");

    assert_eq!(eye.recalled(), 1, "the numbers came from the journal");
    assert!(
        eye.picture_of("one.png").is_some(),
        "and the picture was drawn anyway"
    );
}

#[test]
fn a_stop_is_answered_before_anything_is_decoded() {
    let dir = tempfile::tempdir().expect("dir");
    draw(dir.path(), "one.png", 6.0);
    let journal = Journal::open_in_memory().expect("journal");
    let stop = Arc::new(AtomicBool::new(true));
    let asked = Arc::clone(&stop);

    let mut eye = Eye::new(dir.path(), &journal)
        .stopping_when(Box::new(move || asked.load(Ordering::Relaxed)));
    let answer = eye.print("one.png");

    assert_eq!(answer, Err(STOPPED.to_string()));
    assert_eq!(eye.decoded(), 0);
}

#[test]
fn a_file_of_no_interest_is_never_opened() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::write(dir.path().join("notes.txt"), "words").expect("write");
    let journal = Journal::open_in_memory().expect("journal");
    let eye = Eye::new(dir.path(), &journal);

    assert_eq!(eye.sort("notes.txt", None), None);
    assert_eq!(eye.sort("holiday/IMG_1.jpg", None), Some(Sort::Picture));
    assert_eq!(eye.sort("clip.mov", None), Some(Sort::Moving));
}
