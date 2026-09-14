//! A rule's `match` block, compiled, and the question "does this file fit?"
//!
//! Every constraint present must hold. A rule with no `match` fits everything,
//! which is what makes a trailing catch-all rule expressible.

use std::collections::BTreeMap;

use globset::{GlobSet, GlobSetBuilder};
use regex::Regex;

use crate::attrs::Attributes;
use crate::grammar::{Interval, human_age, human_bytes};

/// A media-type pattern: `*`, `image/*`, or an exact `image/jpeg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MimePattern {
    /// Any type at all.
    Any,
    /// Any subtype of one type.
    Type(String),
    /// One exact type.
    Exact(String),
}

impl MimePattern {
    /// Parse one pattern. Rejects anything that is not `a/b`, `a/*` or `*`.
    ///
    /// # Errors
    /// A message naming what was wrong, for a diagnostic.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim().to_ascii_lowercase();
        if text == "*" || text == "*/*" {
            return Ok(Self::Any);
        }
        match text.split_once('/') {
            Some((kind, "*")) if !kind.is_empty() && !kind.contains('*') => {
                Ok(Self::Type(kind.to_string()))
            }
            Some((kind, sub)) if !kind.is_empty() && !sub.is_empty() && !text.contains('*') => {
                Ok(Self::Exact(format!("{kind}/{sub}")))
            }
            _ => Err(format!(
                "`{text}` is not a media type; expected `image/jpeg`, `image/*` or `*`"
            )),
        }
    }

    /// Whether a concrete media type fits.
    #[must_use]
    pub fn matches(&self, mime: &str) -> bool {
        let mime = mime.to_ascii_lowercase();
        match self {
            Self::Any => true,
            Self::Type(kind) => mime.split_once('/').is_some_and(|(k, _)| k == kind),
            Self::Exact(exact) => mime == *exact,
        }
    }

    /// Whether everything `other` accepts, `self` accepts too.
    #[must_use]
    pub fn covers(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Any, _) => true,
            (Self::Type(a), Self::Type(b)) | (Self::Exact(a), Self::Exact(b)) => a == b,
            (Self::Type(a), Self::Exact(b)) => b.split_once('/').is_some_and(|(k, _)| k == a),
            _ => false,
        }
    }
}

impl std::fmt::Display for MimePattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("*"),
            Self::Type(kind) => write!(f, "{kind}/*"),
            Self::Exact(exact) => f.write_str(exact),
        }
    }
}

/// A set of globs plus the text they came from, for traces.
#[derive(Debug, Clone)]
pub struct Patterns {
    set: GlobSet,
    patterns: Vec<String>,
}

impl Patterns {
    /// Compile a list of globs. `**` crosses directories; `*` does not.
    ///
    /// # Errors
    /// A message naming the pattern that is not a glob.
    pub fn compile(patterns: &[String]) -> Result<Self, String> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| format!("`{pattern}` is not a glob: {}", e.kind()))?;
            builder.add(glob);
        }
        let set = builder.build().map_err(|e| e.to_string())?;
        Ok(Self {
            set,
            patterns: patterns.to_vec(),
        })
    }

    /// Whether there is nothing to match.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// The patterns as written.
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// The first pattern the file fits, or `None`.
    ///
    /// Same rule as `.gitignore`: a pattern without a `/` is matched against
    /// the name alone, so `*.part` finds a partial download at any depth; a
    /// pattern with a `/` is matched against the whole relative path.
    #[must_use]
    pub fn first_match(&self, relative_path: &str) -> Option<&str> {
        let name = relative_path.rsplit('/').next().unwrap_or(relative_path);
        self.patterns
            .iter()
            .enumerate()
            .find(|(index, pattern)| {
                let candidate = if pattern.contains('/') {
                    relative_path
                } else {
                    name
                };
                self.set.matches(candidate).contains(index)
            })
            .map(|(_, pattern)| pattern.as_str())
    }

    /// The first pattern that fits the file or any directory above it.
    ///
    /// For `opaque`: a file inside `Photos.photoslibrary` is inside an opaque
    /// unit even though its own name matches nothing.
    #[must_use]
    pub fn first_match_including_ancestors(&self, relative_path: &str) -> Option<&str> {
        let mut candidates = vec![relative_path];
        let mut rest = relative_path;
        while let Some((ancestor, _)) = rest.rsplit_once('/') {
            candidates.push(ancestor);
            rest = ancestor;
        }
        candidates
            .into_iter()
            .find_map(|candidate| self.first_match(candidate))
    }

    /// Whether a plain glob fits `relative_path`.
    #[must_use]
    pub fn any(&self, relative_path: &str) -> bool {
        self.first_match(relative_path).is_some()
    }
}

/// A rule's `match` block, compiled.
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    /// Extensions, lower-cased and without the dot.
    pub ext: Option<Vec<String>>,
    /// Media-type patterns.
    pub mime: Option<Vec<MimePattern>>,
    /// Globs over the relative path or name.
    pub glob: Option<Patterns>,
    /// A regex over the name; named captures become variables.
    pub regex: Option<Regex>,
    /// A size interval in bytes.
    pub size: Option<Interval>,
    /// An age interval in seconds since the last modification.
    pub age: Option<Interval>,
}

/// Why a rule did or did not take a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every constraint held. Named regex captures come along.
    Matched {
        /// Named captures from `match.regex`.
        captures: BTreeMap<String, String>,
    },
    /// One constraint did not hold. The first failure stops the check.
    Failed {
        /// Which constraint.
        constraint: &'static str,
        /// What the file had instead.
        reason: String,
    },
}

impl Matcher {
    /// Whether this matcher has no constraints, and so takes everything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ext.is_none()
            && self.mime.is_none()
            && self.glob.is_none()
            && self.regex.is_none()
            && self.size.is_none()
            && self.age.is_none()
    }

    /// The attribute names this matcher reads, for the cost model.
    #[must_use]
    pub fn attributes(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.ext.is_some() || self.regex.is_some() || self.glob.is_some() {
            out.push("name");
        }
        if self.mime.is_some() {
            out.push("mime");
        }
        if self.size.is_some() {
            out.push("size");
        }
        if self.age.is_some() {
            out.push("mtime");
        }
        out
    }

    /// The names of the regex's named captures, which templates may use.
    #[must_use]
    pub fn capture_names(&self) -> Vec<String> {
        self.regex
            .as_ref()
            .map(|re| re.capture_names().flatten().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Test a file, stopping at the first constraint that fails.
    #[must_use]
    pub fn test(&self, attrs: &Attributes) -> Verdict {
        let fail = |constraint, reason| Verdict::Failed { constraint, reason };

        if let Some(exts) = &self.ext {
            let ext = attrs.ext().to_ascii_lowercase();
            if !exts.contains(&ext) {
                let had = if ext.is_empty() {
                    "the file has no extension".to_string()
                } else {
                    // `list` supplies its own "one of" when there is more
                    // than one, so this must not add a second.
                    format!("`{ext}` is not {}", list(exts))
                };
                return fail("ext", had);
            }
        }
        if let Some(patterns) = &self.mime {
            match &attrs.mime {
                None => return fail("mime", "the media type is not known".to_string()),
                Some(mime) if !patterns.iter().any(|p| p.matches(mime)) => {
                    return fail(
                        "mime",
                        format!(
                            "`{mime}` is not {}",
                            list(&patterns.iter().map(ToString::to_string).collect::<Vec<_>>())
                        ),
                    );
                }
                Some(_) => {}
            }
        }
        if let Some(globs) = &self.glob
            && !globs.any(&attrs.relative_path())
        {
            return fail(
                "glob",
                format!(
                    "`{}` fits none of {}",
                    attrs.relative_path(),
                    list(globs.patterns())
                ),
            );
        }
        let mut captures = BTreeMap::new();
        if let Some(regex) = &self.regex {
            match regex.captures(&attrs.name) {
                None => {
                    return fail(
                        "regex",
                        format!("`{}` does not match `{}`", attrs.name, regex.as_str()),
                    );
                }
                Some(found) => {
                    for name in regex.capture_names().flatten() {
                        if let Some(m) = found.name(name) {
                            captures.insert(name.to_string(), m.as_str().to_string());
                        }
                    }
                }
            }
        }
        if let Some(size) = &self.size
            && !size.contains(attrs.size)
        {
            return fail(
                "size",
                format!("{} is not {}", human_bytes(attrs.size), describe_size(size)),
            );
        }
        if let Some(age) = &self.age {
            let Some(mtime) = attrs.mtime else {
                return fail("age", "the modification time is not known".to_string());
            };
            let seconds = attrs
                .now
                .duration_since(mtime)
                .as_secs()
                .try_into()
                .unwrap_or(0_u64);
            if !age.contains(seconds) {
                return fail(
                    "age",
                    format!("{} old is not {}", human_age(seconds), describe_age(age)),
                );
            }
        }
        Verdict::Matched { captures }
    }

    /// Whether every file `later` would take, `self` takes first.
    ///
    /// Exact for literal sets and intervals; `false` whenever a glob or regex
    /// is involved on the earlier side, because that is undecidable cheaply
    /// and a wrong warning is worse than none.
    #[must_use]
    pub fn subsumes(&self, later: &Self) -> bool {
        if self.glob.is_some() || self.regex.is_some() {
            return false;
        }
        let ext_ok = match (&self.ext, &later.ext) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(mine), Some(theirs)) => theirs.iter().all(|e| mine.contains(e)),
        };
        let mime_ok = match (&self.mime, &later.mime) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(mine), Some(theirs)) => theirs.iter().all(|t| mine.iter().any(|m| m.covers(t))),
        };
        let size_ok = match (&self.size, &later.size) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(mine), Some(theirs)) => mine.subsumes(theirs),
        };
        let age_ok = match (&self.age, &later.age) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(mine), Some(theirs)) => mine.subsumes(theirs),
        };
        ext_ok && mime_ok && size_ok && age_ok
    }
}

fn list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|i| format!("`{i}`")).collect();
    match quoted.len() {
        0 => "nothing".to_string(),
        1 => quoted[0].clone(),
        _ => format!("one of {}", quoted.join(", ")),
    }
}

fn describe_size(interval: &Interval) -> String {
    describe(interval, human_bytes)
}

fn describe_age(interval: &Interval) -> String {
    describe(interval, human_age)
}

fn describe(interval: &Interval, unit: fn(u64) -> String) -> String {
    match (interval.low, interval.high) {
        (Some((a, true)), Some((b, true))) if a == b => format!("exactly {}", unit(a)),
        (Some((a, _)), Some((b, _))) => format!("between {} and {}", unit(a), unit(b)),
        (Some((a, true)), None) => format!("at least {}", unit(a)),
        (Some((a, false)), None) => format!("more than {}", unit(a)),
        (None, Some((b, true))) => format!("at most {}", unit(b)),
        (None, Some((b, false))) => format!("less than {}", unit(b)),
        (None, None) => "anything".to_string(),
    }
}
