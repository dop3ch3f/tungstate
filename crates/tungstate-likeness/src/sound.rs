//! Sound, by chromaprint.
//!
//! The same recording at two bitrates has almost nothing in common byte for
//! byte and everything in common as sound. Chromaprint is what every music
//! tool uses for this: it throws away the waveform and keeps how the energy
//! moves between the twelve notes, which is what survives re-encoding.
//!
//! It produces a *sequence*, not a number, and sequences cannot be compared
//! against ten thousand other sequences one pair at a time. So the sequence is
//! the detail and a 64 bit summary of it is the signature, exactly as the
//! picture side has two sizes of the same idea.
// Every cast below turns a count of bits or samples into a score between 0 and
// 100. The precision a f32 loses at that scale is smaller than the rounding.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::fs::File;
use std::path::Path;

use rusty_chromaprint::{Configuration, Fingerprinter, match_fingerprints};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::{Print, SOUND, Shot, Trouble};

/// Listen to at most this many seconds.
///
/// Two minutes is more than enough to tell one recording from another, and it
/// puts a ceiling on a scan of a folder of hour-long videos. The cost of the
/// cap is a pair of files that are identical for two minutes and diverge
/// after, which is not a thing that happens by accident.
const LISTEN_SECONDS: u64 = 120;

pub fn look(path: &Path) -> Result<Shot, Trouble> {
    let print = print_of(path, SOUND)?;
    Ok(Shot { print, thumb: None })
}

/// Fingerprint the sound in a file, whatever the file is.
///
/// `algo` is the caller's, because the same arithmetic over the same bytes
/// means one thing for a song and another for the soundtrack of a video, and
/// the two must never be compared.
pub fn print_of(path: &Path, algo: &'static str) -> Result<Print, Trouble> {
    let raw = listen(path)?;
    if raw.len() < 16 {
        return Err(Trouble::Empty);
    }
    Ok(Print {
        algo,
        signature: summarise(&raw),
        // How long it is, near enough: chromaprint emits one item per short
        // slice of sound, so the count stands in for the duration without
        // needing the container to have been honest about it.
        weight: raw.len() as u64,
        detail: pack(&raw),
    })
}

/// Decode up to [`LISTEN_SECONDS`] and hand it to chromaprint.
fn listen(path: &Path) -> Result<Vec<u32>, Trouble> {
    let file = File::open(path).map_err(|error| Trouble::Unreadable(error.to_string()))?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|it| it.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| Trouble::Unsupported(error.to_string()))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| Trouble::Unsupported("file with no sound in it".into()))?;
    let track_id = track.id;
    let params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params.clone(),
        _ => return Err(Trouble::Unsupported("track with no codec".into())),
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|error| Trouble::Unsupported(error.to_string()))?;

    let config = Configuration::preset_test1();
    let mut printer = Fingerprinter::new(&config);
    let mut started = false;
    let mut heard = 0u64;
    let mut samples = Vec::new();

    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track_id {
            continue;
        }
        let Ok(sound) = decoder.decode(&packet) else {
            // One bad packet in the middle of a file is not a reason to give
            // up on the file; a run of them ends at the loop's own exit.
            continue;
        };
        if sound.is_empty() {
            continue;
        }
        let rate = sound.spec().rate();
        let channels = sound.spec().channels().count() as u32;
        if !started {
            if rate == 0 || channels == 0 {
                return Err(Trouble::Unsupported("sound with no sample rate".into()));
            }
            printer
                .start(rate, channels)
                .map_err(|error| Trouble::Unsupported(error.to_string()))?;
            started = true;
        }
        sound.copy_to_vec_interleaved(&mut samples);
        printer.consume(&samples);
        heard += sound.frames() as u64;
        if rate > 0 && heard / u64::from(rate) >= LISTEN_SECONDS {
            break;
        }
    }
    if !started {
        return Err(Trouble::Empty);
    }
    printer.finish();
    Ok(printer.fingerprint().to_vec())
}

/// Two 32 bit chroma words to a 64 bit word, so the detail stores and reads
/// back through the same hex as every other print.
fn pack(raw: &[u32]) -> Vec<u64> {
    raw.chunks(2)
        .map(|pair| {
            let high = u64::from(pair[0]) << 32;
            high | u64::from(pair.get(1).copied().unwrap_or_default())
        })
        .collect()
}

fn unpack(detail: &[u64]) -> Vec<u32> {
    detail
        .iter()
        .flat_map(|word| [(word >> 32) as u32, (*word & 0xffff_ffff) as u32])
        .collect()
}

/// A 64 bit summary of a whole fingerprint.
///
/// How often each of the 32 bit positions is set, over the whole recording,
/// quantised to four levels. Two encodings of one song have nearly the same
/// profile even when they align badly, which is what a candidate search needs.
///
/// The four levels are written in Gray code (00, 01, 11, 10) so that
/// neighbouring levels differ by one bit. Plain binary would make the step
/// from 1 to 2 a two bit jump and quietly distort every distance.
fn summarise(raw: &[u32]) -> u64 {
    const GRAY: [u64; 4] = [0b00, 0b01, 0b11, 0b10];
    let mut signature = 0u64;
    for bit in 0..32 {
        let set = raw.iter().filter(|word| *word >> bit & 1 == 1).count();
        let share = set as f32 / raw.len() as f32;
        let level = ((share * 4.0) as usize).min(3);
        signature |= GRAY[level] << (bit * 2);
    }
    signature
}

/// How alike two recordings are.
///
/// Chromaprint scores a matched stretch from 0 to 32, lower being closer, and
/// says nothing by itself about how much of the two files that stretch covers.
/// A four second jingle matching inside two different hour-long videos is a
/// perfect score over nothing, so the score is weighted by the share of the
/// shorter fingerprint the match accounts for.
pub fn alike(one: &[u64], other: &[u64]) -> Option<u8> {
    let (first, second) = (unpack(one), unpack(other));
    let shorter = first.len().min(second.len());
    if shorter < 16 {
        return None;
    }
    let config = Configuration::preset_test1();
    let segments = match_fingerprints(&first, &second, &config).ok()?;
    let best = segments
        .iter()
        .filter(|segment| segment.score < 10.0)
        .max_by_key(|segment| segment.items_count)?;
    let covered = (best.items_count as f32 / shorter as f32).min(1.0);
    let closeness = 1.0 - (best.score as f32 / 32.0);
    Some((covered * closeness * 100.0).round().clamp(0.0, 100.0) as u8)
}
