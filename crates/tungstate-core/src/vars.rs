//! `vars.<name>`: a value taken from the first attribute that resolves, then
//! optionally relabelled through `map` or `bucket`.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use serde::Serialize;

use crate::attrs::{Attributes, Tier, Value, tier_of};
use crate::grammar::Interval;

/// One compiled variable definition.
#[derive(Debug, Clone)]
pub struct VarDef {
    /// The variable's name.
    pub name: String,
    /// Attribute names tried in order; the first present wins.
    pub from: Vec<String>,
    /// Zone for formatting times.
    pub tz: TimeZone,
    /// The zone as written, for traces.
    pub tz_name: String,
    /// Text relabelling. `_` is the catch-all.
    pub map: Option<BTreeMap<String, String>>,
    /// Size relabelling: each label, already made path-safe by
    /// [`Interval::label`], with the interval it covers.
    pub bucket: Option<Vec<(String, Interval)>>,
}

/// A resolved variable, with where its value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The value, after `map` or `bucket`.
    pub value: Value,
    /// The attribute that supplied it.
    pub source: String,
    /// What reading that attribute cost.
    pub tier: Tier,
    /// The attribute's value before relabelling.
    pub raw: String,
    /// Which `map` entry or `bucket` label relabelled it, if any.
    pub via: Option<String>,
    /// Sources tried before `source` that were absent.
    pub skipped: Vec<String>,
}

/// Why a variable has no value for this file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Unresolved {
    /// None of the sources was present.
    Absent {
        /// Every source that was tried.
        tried: Vec<String>,
    },
    /// The value fell outside every bucket.
    NoBucket {
        /// Which attribute supplied the value.
        source: String,
        /// The value.
        raw: String,
    },
    /// No `map` entry fit and there was no `_`.
    NoMapping {
        /// Which attribute supplied the value.
        source: String,
        /// The value.
        raw: String,
    },
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent { tried } => match tried.as_slice() {
                [one] => write!(f, "`{one}` is not known for this file"),
                many => write!(f, "none of {} is known for this file", many.join(", ")),
            },
            Self::NoBucket { raw, .. } => write!(f, "{raw} falls in no bucket"),
            Self::NoMapping { raw, .. } => {
                write!(f, "`{raw}` has no map entry and there is no `_`")
            }
        }
    }
}

impl VarDef {
    /// The highest tier any source needs.
    #[must_use]
    pub fn tier(&self) -> Tier {
        self.from
            .iter()
            .filter_map(|s| tier_of(s))
            .max()
            .unwrap_or(Tier::Stat)
    }

    /// Resolve against a file. `captures` are this rule's regex captures,
    /// which a var may read as if they were attributes.
    ///
    /// # Errors
    /// [`Unresolved`] when no source is present, or the value fits no
    /// `bucket` or `map` entry.
    pub fn resolve(
        &self,
        attrs: &Attributes,
        captures: &BTreeMap<String, String>,
    ) -> Result<Resolved, Unresolved> {
        let mut skipped = Vec::new();
        for source in &self.from {
            let value = captures
                .get(source)
                .map(|s| Value::Text(s.clone()))
                .or_else(|| attrs.get(source));
            let Some(value) = value else {
                skipped.push(source.clone());
                continue;
            };
            let raw = value.as_text();
            let tier = tier_of(source).unwrap_or(Tier::Stat);
            let (value, via) = self.relabel(source, value)?;
            return Ok(Resolved {
                value,
                source: source.clone(),
                tier,
                raw,
                via,
                skipped,
            });
        }
        Err(Unresolved::Absent {
            tried: self.from.clone(),
        })
    }

    fn relabel(&self, source: &str, value: Value) -> Result<(Value, Option<String>), Unresolved> {
        if let Some(buckets) = &self.bucket {
            let raw = value.as_text();
            let Value::Size(n) = value else {
                // Load-time validation only lets `bucket` sit on `size`, so
                // this is unreachable in practice, but a wrong answer here
                // must not be a panic.
                return Err(Unresolved::NoBucket {
                    source: source.to_string(),
                    raw,
                });
            };
            return buckets
                .iter()
                .find(|(_, interval)| interval.contains(n))
                .map(|(label, _)| {
                    (
                        Value::Text(label.clone()),
                        Some(format!("bucket `{label}`")),
                    )
                })
                .ok_or(Unresolved::NoBucket {
                    source: source.to_string(),
                    raw,
                });
        }
        if let Some(map) = &self.map {
            let raw = value.as_text();
            return match lookup_map(map, &raw) {
                Some((key, label)) => Ok((
                    Value::Text(label.clone()),
                    Some(format!("map `{key}` = \"{label}\"")),
                )),
                None => Err(Unresolved::NoMapping {
                    source: source.to_string(),
                    raw,
                }),
            };
        }
        Ok((value, None))
    }
}

/// Find the `map` entry for `text`: an exact key first, then the wildcard
/// pattern with the most literal text, then `_`.
///
/// Specificity rather than file order, because TOML inline tables do not
/// promise an order and `image/*` next to `image/png` should not depend on
/// how the parser happened to sort them.
fn lookup_map<'m>(map: &'m BTreeMap<String, String>, text: &str) -> Option<(&'m str, &'m String)> {
    if let Some((key, value)) = map.get_key_value(text) {
        return Some((key.as_str(), value));
    }
    map.iter()
        .filter(|(key, _)| key.contains('*') && wildcard_matches(key, text))
        .max_by_key(|(key, _)| key.chars().filter(|c| *c != '*').count())
        .map(|(key, value)| (key.as_str(), value))
        .or_else(|| map.get_key_value("_").map(|(k, v)| (k.as_str(), v)))
}

/// `*` matches any run of characters, including none. Nothing else is special.
#[must_use]
pub fn wildcard_matches(pattern: &str, text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    let mut pieces = pattern.split('*');
    let Some(first) = pieces.next() else {
        return text.is_empty();
    };
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut pieces = pieces.peekable();
    while let Some(piece) = pieces.next() {
        if pieces.peek().is_none() {
            return rest.ends_with(piece);
        }
        match rest.find(piece) {
            Some(i) => rest = &rest[i + piece.len()..],
            None => return false,
        }
    }
    // The pattern had no `*` at all, so it had to match exactly.
    rest.is_empty()
}
