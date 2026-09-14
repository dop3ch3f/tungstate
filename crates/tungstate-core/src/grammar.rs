//! The small grammars inside string values: sizes, ages, durations, ranges.
//!
//! One range grammar serves `match.size`, `match.age` and `bucket`, so a
//! boundary means the same thing everywhere it can be written.

use std::fmt;
use std::time::Duration;

/// Parse `10MiB`, `1.5 GB`, `4096`. Binary units are `KiB`..`TiB` (or the
/// bare `K`..`T`); decimal units are `KB`..`TB`. Case does not matter.
///
/// # Errors
/// A message naming what was wrong, for a diagnostic.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    if number.is_empty() {
        return Err(format!(
            "`{text}` is not a size; expected something like `10MiB`"
        ));
    }
    let magnitude: f64 = number
        .parse()
        .map_err(|_| format!("`{number}` is not a number"))?;
    let multiplier: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kib" => 1024.0,
        "m" | "mib" => 1024.0 * 1024.0,
        "g" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        other => {
            return Err(format!(
                "`{other}` is not a size unit; use B, KiB, MiB, GiB, TiB or KB, MB, GB, TB"
            ));
        }
    };
    // Truncation is the intent: `1.5KiB` is 1536 bytes, and a fractional
    // byte does not exist.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((magnitude * multiplier).round() as u64)
}

/// Parse `30s`, `5m`, `2h`, `30d`, `4w`. A bare number is seconds.
///
/// # Errors
/// A message naming what was wrong, for a diagnostic.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    if number.is_empty() {
        return Err(format!(
            "`{text}` is not a duration; expected something like `30d`"
        ));
    }
    let magnitude: f64 = number
        .parse()
        .map_err(|_| format!("`{number}` is not a number"))?;
    let seconds: f64 = match unit.trim() {
        "" | "s" => 1.0,
        "m" => 60.0,
        "h" => 3600.0,
        "d" => 86_400.0,
        "w" => 7.0 * 86_400.0,
        other => return Err(format!("`{other}` is not a time unit; use s, m, h, d or w")),
    };
    Ok(Duration::from_secs_f64(magnitude * seconds))
}

/// Seconds, for the age grammar, which compares against `now - mtime`.
///
/// # Errors
/// As [`parse_duration`].
pub fn parse_age(text: &str) -> Result<u64, String> {
    parse_duration(text).map(|d| d.as_secs())
}

/// A closed, half-open or open interval over a magnitude.
///
/// Written as `> 1MiB`, `>= 1MiB`, `< 10MiB`, `<= 10MiB`, `= 4096`,
/// `1MiB..1GiB` or `1MiB-1GiB`. Both ends of a range are inclusive, so the
/// three buckets `<10MiB`, `10MiB-1GiB`, `>1GiB` partition every size with
/// no gap and no overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    /// Lower bound and whether it is included.
    pub low: Option<(u64, bool)>,
    /// Upper bound and whether it is included.
    pub high: Option<(u64, bool)>,
}

impl Interval {
    /// Parse one interval, with `unit` turning `10MiB` or `30d` into a number.
    ///
    /// # Errors
    /// A message naming what was wrong, for a diagnostic.
    pub fn parse(text: &str, unit: fn(&str) -> Result<u64, String>) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty; expected something like `> 1MiB`".to_string());
        }
        for (prefix, build) in [
            (
                ">=",
                (|n| Self {
                    low: Some((n, true)),
                    high: None,
                }) as fn(u64) -> Self,
            ),
            ("<=", |n| Self {
                low: None,
                high: Some((n, true)),
            }),
            (">", |n| Self {
                low: Some((n, false)),
                high: None,
            }),
            ("<", |n| Self {
                low: None,
                high: Some((n, false)),
            }),
            ("=", |n| Self {
                low: Some((n, true)),
                high: Some((n, true)),
            }),
        ] {
            if let Some(rest) = text.strip_prefix(prefix) {
                return unit(rest).map(build);
            }
        }
        let (a, b) = text
            .split_once("..")
            .or_else(|| text.split_once('-'))
            .ok_or_else(|| format!("`{text}` is not a comparison or a range"))?;
        let (low, high) = (unit(a)?, unit(b)?);
        if low > high {
            return Err(format!("`{text}` runs backwards"));
        }
        Ok(Self {
            low: Some((low, true)),
            high: Some((high, true)),
        })
    }

    /// A label for this interval that is legal as a directory name.
    ///
    /// `<` and `>` are illegal in a name on Windows and over SMB, and
    /// sanitising both to `_` would make `<1GiB` and `>1GiB` the same
    /// folder, so a bucket is spelled in words: `under-10MiB`, `10MiB-1GiB`,
    /// `over-1GiB`.
    #[must_use]
    pub fn label(text: &str) -> String {
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        for (prefix, word) in [
            (">=", "from-"),
            ("<=", "up-to-"),
            (">", "over-"),
            ("<", "under-"),
            ("=", "exactly-"),
        ] {
            if let Some(rest) = text.strip_prefix(prefix) {
                return format!("{word}{rest}");
            }
        }
        text.replace("..", "-")
    }

    /// Whether `value` lies inside.
    #[must_use]
    pub fn contains(&self, value: u64) -> bool {
        let above = match self.low {
            None => true,
            Some((n, true)) => value >= n,
            Some((n, false)) => value > n,
        };
        let below = match self.high {
            None => true,
            Some((n, true)) => value <= n,
            Some((n, false)) => value < n,
        };
        above && below
    }

    /// Whether every value inside `other` is also inside `self`.
    #[must_use]
    pub fn subsumes(&self, other: &Self) -> bool {
        let low_ok = match (self.low, other.low) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some((a, a_in)), Some((b, b_in))) => a < b || (a == b && (a_in || !b_in)),
        };
        let high_ok = match (self.high, other.high) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some((a, a_in)), Some((b, b_in))) => a > b || (a == b && (a_in || !b_in)),
        };
        low_ok && high_ok
    }
}

/// The human spelling of a byte count, for traces.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The human spelling of an age in seconds, for traces.
#[must_use]
pub fn human_age(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.low, self.high) {
            (Some((a, true)), Some((b, true))) if a == b => write!(f, "= {a}"),
            (Some((a, _)), Some((b, _))) => write!(f, "{a}..{b}"),
            (Some((a, true)), None) => write!(f, ">= {a}"),
            (Some((a, false)), None) => write!(f, "> {a}"),
            (None, Some((b, true))) => write!(f, "<= {b}"),
            (None, Some((b, false))) => write!(f, "< {b}"),
            (None, None) => write!(f, "anything"),
        }
    }
}
