//! The policy file: its serde shape, and the compiled form the classifier
//! runs.
//!
//! Two layers on purpose. The `raw` structs mirror the TOML exactly, with
//! every value that a diagnostic might point at wrapped in `Spanned`. The
//! public types hold compiled globs, regexes, intervals and templates, and
//! carry only the spans a trace still needs.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::time::Duration;

use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use toml::Spanned;

use crate::attrs::{Tier, VOCABULARY, tier_of};
use crate::error::{PolicyError, Warning};
use crate::grammar::{Interval, parse_age, parse_duration, parse_size};
use crate::matcher::{Matcher, MimePattern, Patterns};
use crate::template::{Filter, Template, validate_format};
use crate::vars::VarDef;

/// What a governed folder does about drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Report drift and change nothing.
    #[default]
    Observe,
    /// Propose a plan and wait to be told.
    Suggest,
    /// Apply the plan.
    Enforce,
}

/// What to do when a symbolic link is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Symlinks {
    /// Leave it alone and never look through it.
    #[default]
    Ignore,
    /// Classify the link itself by its name and stat.
    TreatAsFile,
    /// Follow it, but only if the target is inside the folder.
    FollowWithinFolder,
}

/// What to do with a file whose content already exists elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(missing_docs)]
pub enum OnDuplicate {
    Skip,
    #[default]
    Trash,
    Replace,
    NewerWins,
    LargerWins,
    Hardlink,
    Reflink,
    Quarantine,
    KeepBoth,
}

/// What to do when a different file already holds the destination name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(missing_docs)]
pub enum OnConflict {
    Rename,
    Skip,
    Replace,
    #[default]
    Quarantine,
}

/// `[folder]`, compiled.
#[derive(Debug, Clone)]
pub struct Folder {
    /// The folder's name.
    pub name: String,
    /// What it does about drift.
    pub mode: Mode,
    /// Where unclassified files go, if anywhere.
    pub inbox: Option<String>,
    /// Never touched, never classified.
    pub ignore: Patterns,
    /// Treated as single units, never looked inside.
    pub opaque: Patterns,
    /// What to do with a symbolic link.
    pub symlinks: Symlinks,
}

/// `[defaults]`, compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    /// What to do with a duplicate.
    pub on_duplicate: OnDuplicate,
    /// What to do with a name clash.
    pub on_conflict: OnConflict,
    /// How long a file must be untouched before it is moved.
    pub cooldown: Duration,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            on_duplicate: OnDuplicate::default(),
            on_conflict: OnConflict::default(),
            cooldown: Duration::from_secs(30),
        }
    }
}

/// One `[[rule]]`, compiled.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The rule's name.
    pub name: String,
    /// Where its name is declared, for traces and warnings.
    pub span: Range<usize>,
    /// The `match` block.
    pub matcher: Matcher,
    /// The `vars` block, in name order.
    pub vars: Vec<VarDef>,
    /// The `path` template.
    pub path: Template,
    /// The `rename` template, if any.
    pub rename: Option<Template>,
    /// The highest tier anything in this rule reads.
    pub tier: Tier,
}

/// A loaded policy.
#[derive(Debug, Clone)]
pub struct Policy {
    /// `[folder]`.
    pub folder: Folder,
    /// `[defaults]`.
    pub defaults: Defaults,
    /// Every `[[rule]]`, in file order. File order is precedence.
    pub rules: Vec<Rule>,
}

/// A policy plus what the loader noticed on the way.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The policy.
    pub policy: Policy,
    /// Anything worth saying that did not stop the load.
    pub warnings: Vec<Warning>,
}

impl Policy {
    /// Parse and compile a policy from its TOML text.
    ///
    /// # Errors
    /// [`PolicyError`] with a span into `text` for the first problem found.
    pub fn parse(text: &str) -> Result<Loaded, PolicyError> {
        let raw: raw::PolicyFile = toml::from_str(text).map_err(|error| PolicyError::Syntax {
            message: error.message().to_string(),
            span: error.span(),
        })?;
        compile(raw)
    }

    /// The most any rule needs to read, so a policy that never mentions
    /// `exif` or `hash` never pays for them.
    #[must_use]
    pub fn required_tier(&self) -> Tier {
        self.rules
            .iter()
            .map(|rule| rule.tier)
            .max()
            .unwrap_or(Tier::Stat)
    }
}

fn compile(raw: raw::PolicyFile) -> Result<Loaded, PolicyError> {
    let folder = compile_folder(raw.folder)?;
    let defaults = compile_defaults(raw.defaults.unwrap_or_default())?;

    let mut seen: BTreeMap<String, Range<usize>> = BTreeMap::new();
    let mut rules = Vec::with_capacity(raw.rules.len());
    for rule in raw.rules {
        let span = rule.name.span();
        let name = rule.name.get_ref().clone();
        if let Some(first) = seen.get(&name) {
            return Err(PolicyError::DuplicateRule {
                rule: name,
                span,
                first: first.clone(),
            });
        }
        seen.insert(name.clone(), span.clone());
        rules.push(compile_rule(rule)?);
    }

    let warnings = shadowing(&rules);
    Ok(Loaded {
        policy: Policy {
            folder,
            defaults,
            rules,
        },
        warnings,
    })
}

fn compile_folder(raw: raw::Folder) -> Result<Folder, PolicyError> {
    let globs =
        |field: &'static str, patterns: Vec<Spanned<String>>| -> Result<Patterns, PolicyError> {
            for pattern in &patterns {
                Patterns::compile(std::slice::from_ref(pattern.get_ref())).map_err(|message| {
                    PolicyError::Folder {
                        field,
                        message,
                        span: pattern.span(),
                    }
                })?;
            }
            let texts: Vec<String> = patterns.into_iter().map(Spanned::into_inner).collect();
            Patterns::compile(&texts).map_err(|message| PolicyError::Folder {
                field,
                message,
                span: 0..0,
            })
        };
    Ok(Folder {
        name: raw.name.into_inner(),
        mode: raw.mode.map(Spanned::into_inner).unwrap_or_default(),
        inbox: raw.inbox.map(Spanned::into_inner),
        ignore: globs("ignore", raw.ignore)?,
        opaque: globs("opaque", raw.opaque)?,
        symlinks: raw.symlinks.map(Spanned::into_inner).unwrap_or_default(),
    })
}

fn compile_defaults(raw: raw::Defaults) -> Result<Defaults, PolicyError> {
    let base = Defaults::default();
    let cooldown = match raw.cooldown {
        None => base.cooldown,
        Some(text) => parse_duration(text.get_ref()).map_err(|message| PolicyError::Defaults {
            field: "cooldown",
            message,
            span: text.span(),
        })?,
    };
    Ok(Defaults {
        on_duplicate: raw
            .on_duplicate
            .map_or(base.on_duplicate, Spanned::into_inner),
        on_conflict: raw
            .on_conflict
            .map_or(base.on_conflict, Spanned::into_inner),
        cooldown,
    })
}

fn compile_rule(raw: raw::Rule) -> Result<Rule, PolicyError> {
    let name = raw.name.get_ref().clone();
    let matcher = compile_match(&name, raw.r#match)?;
    let captures = matcher.capture_names();

    let mut vars = Vec::new();
    for (var_name, def) in raw.vars {
        vars.push(compile_var(&name, &var_name, def, &captures)?);
    }

    // Everything a template may name: attributes, this rule's vars, and the
    // regex's captures. Anything else is a typo, and a typo caught at load
    // time is one that never becomes a mystery destination.
    let known: BTreeSet<String> = vars
        .iter()
        .map(|v| v.name.clone())
        .chain(captures.iter().cloned())
        .collect();
    let is_time = |var: &str| -> Option<bool> {
        if let Some(def) = vars.iter().find(|v| v.name == var) {
            if def.map.is_some() || def.bucket.is_some() {
                return Some(false);
            }
            let mut sources = def.from.iter();
            return sources.next().map(|s| is_time_attribute(s));
        }
        if captures.contains(&var.to_string()) {
            return Some(false);
        }
        tier_of(var).map(|_| is_time_attribute(var))
    };

    let path = compile_template(&name, &raw.path, &known, is_time)?;
    let rename = raw
        .rename
        .as_ref()
        .map(|t| compile_template(&name, t, &known, is_time))
        .transpose()?;

    let tier = matcher
        .attributes()
        .iter()
        .filter_map(|a| tier_of(a))
        .chain(vars.iter().map(VarDef::tier))
        .chain(
            path.placeholders()
                .chain(rename.iter().flat_map(Template::placeholders))
                .filter_map(|(var, _, _, _)| tier_of(var)),
        )
        .max()
        .unwrap_or(Tier::Stat);

    Ok(Rule {
        name,
        span: raw.name.span(),
        matcher,
        vars,
        path,
        rename,
        tier,
    })
}

/// Whether an attribute renders as a date.
fn is_time_attribute(attribute: &str) -> bool {
    matches!(attribute, "mtime" | "now")
        || attribute
            .strip_prefix("exif.")
            .is_some_and(|tag| tag.contains("Date") || tag.contains("Time"))
}

fn compile_match(rule: &str, raw: Option<raw::Match>) -> Result<Matcher, PolicyError> {
    let Some(raw) = raw else {
        return Ok(Matcher::default());
    };
    let error = |field: &'static str, span: Range<usize>| {
        move |message: String| PolicyError::Match {
            rule: rule.to_string(),
            field,
            message,
            span,
        }
    };

    let ext = raw
        .ext
        .map(|spanned| {
            let span = spanned.span();
            let items = spanned.into_inner().into_vec();
            if items.iter().any(String::is_empty) {
                return Err(error("ext", span)(
                    "an extension cannot be empty".to_string(),
                ));
            }
            Ok(items
                .into_iter()
                .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
                .collect::<Vec<_>>())
        })
        .transpose()?;

    let mime = raw
        .mime
        .map(|spanned| {
            let span = spanned.span();
            spanned
                .into_inner()
                .into_vec()
                .iter()
                .map(|p| MimePattern::parse(p))
                .collect::<Result<Vec<_>, _>>()
                .map_err(error("mime", span))
        })
        .transpose()?;

    let glob = raw
        .glob
        .map(|spanned| {
            let span = spanned.span();
            Patterns::compile(&spanned.into_inner().into_vec()).map_err(error("glob", span))
        })
        .transpose()?;

    let regex = raw
        .regex
        .map(|spanned| {
            let span = spanned.span();
            regex::Regex::new(spanned.get_ref())
                .map_err(|e| error("regex", span)(format!("not a valid regex: {e}")))
        })
        .transpose()?;

    let size = raw
        .size
        .map(|spanned| {
            let span = spanned.span();
            Interval::parse(spanned.get_ref(), parse_size).map_err(error("size", span))
        })
        .transpose()?;

    let age = raw
        .age
        .map(|spanned| {
            let span = spanned.span();
            Interval::parse(spanned.get_ref(), parse_age).map_err(error("age", span))
        })
        .transpose()?;

    Ok(Matcher {
        ext,
        mime,
        glob,
        regex,
        size,
        age,
    })
}

fn compile_var(
    rule: &str,
    var: &str,
    raw: Spanned<raw::Var>,
    captures: &[String],
) -> Result<VarDef, PolicyError> {
    let whole = raw.span();
    let raw = raw.into_inner();
    let error = |span: Range<usize>| {
        move |message: String| PolicyError::Var {
            rule: rule.to_string(),
            var: var.to_string(),
            message,
            span,
        }
    };

    let from_span = raw.from.span();
    let from = raw.from.into_inner().into_vec();
    if from.is_empty() {
        return Err(error(from_span)("`from` names no attribute".to_string()));
    }
    for source in &from {
        if tier_of(source).is_none() && !captures.contains(source) {
            return Err(error(from_span.clone())(format!(
                "`{source}` is not an attribute; the attributes are {VOCABULARY}, plus this \
                 rule's regex captures"
            )));
        }
    }

    let (tz, tz_name) = match raw.tz {
        None => (TimeZone::system(), "local".to_string()),
        Some(spanned) => {
            let span = spanned.span();
            let name = spanned.into_inner();
            let zone = match name.as_str() {
                "local" => TimeZone::system(),
                "utc" | "UTC" => TimeZone::UTC,
                other => TimeZone::get(other).map_err(|_| {
                    error(span)(format!(
                        "`{other}` is not a time zone; use `local`, `utc` or an IANA name \
                         such as `Africa/Lagos`"
                    ))
                })?,
            };
            (zone, name)
        }
    };

    if raw.map.is_some() && raw.bucket.is_some() {
        return Err(error(whole)(
            "a var takes `map` or `bucket`, not both".to_string(),
        ));
    }

    let map = raw.map.map(|spanned| {
        let span = spanned.span();
        let map = spanned.into_inner();
        if map.is_empty() {
            return Err(error(span)("`map` is empty".to_string()));
        }
        Ok(map)
    });
    let map = map.transpose()?;

    let bucket = raw
        .bucket
        .map(|spanned| {
            let span = spanned.span();
            if from.iter().any(|s| s != "size") {
                return Err(error(span)(
                    "`bucket` only applies to `size`; use `map` for text".to_string(),
                ));
            }
            let labels = spanned.into_inner();
            if labels.is_empty() {
                return Err(error(span)("`bucket` is empty".to_string()));
            }
            labels
                .into_iter()
                .map(|label| {
                    Interval::parse(&label, parse_size)
                        .map(|interval| (Interval::label(&label), interval))
                        .map_err(|message| {
                            error(span.clone())(format!("bucket `{label}`: {message}"))
                        })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;

    Ok(VarDef {
        name: var.to_string(),
        from,
        tz,
        tz_name,
        map,
        bucket,
    })
}

fn compile_template(
    rule: &str,
    raw: &Spanned<String>,
    known: &BTreeSet<String>,
    is_time: impl Fn(&str) -> Option<bool>,
) -> Result<Template, PolicyError> {
    // A TOML basic string starts one byte after its span, at the opening
    // quote. Escapes shift later characters, so the pointer is exact for the
    // strings people write in templates and approximate for ones with `\`.
    let inner = raw.span().start + 1;
    let at = |span: &Range<usize>| (inner + span.start)..(inner + span.end).min(raw.span().end);
    let error = |message: String, span: &Range<usize>| PolicyError::Template {
        rule: rule.to_string(),
        message,
        span: at(span),
    };

    let template = Template::parse(raw.get_ref()).map_err(|e| error(e.message, &e.span))?;

    for (name, format, filters, span) in template.placeholders() {
        if !known.contains(name) && tier_of(name).is_none() {
            let mut names: Vec<&str> = known.iter().map(String::as_str).collect();
            names.sort_unstable();
            let extra = if names.is_empty() {
                String::new()
            } else {
                format!(", plus this rule's own: {}", names.join(", "))
            };
            return Err(error(
                format!("unknown variable `{name}`; the attributes are {VOCABULARY}{extra}"),
                span,
            ));
        }
        if let Some(format) = format {
            validate_format(name, format, is_time(name)).map_err(|m| error(m, span))?;
        }
        // `sanitize` is applied to every segment regardless, so writing it
        // anywhere but last is a no-op that reads as if it mattered.
        if let Some(index) = filters.iter().position(|f| *f == Filter::Sanitize)
            && index + 1 != filters.len()
        {
            return Err(error(
                "`sanitize` must be the last filter; it is applied last anyway".to_string(),
                span,
            ));
        }
    }
    Ok(template)
}

/// The shadowing check: a rule that can never fire, because an earlier one
/// provably takes everything it would match.
fn shadowing(rules: &[Rule]) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for (j, later) in rules.iter().enumerate() {
        if let Some(earlier) = rules[..j]
            .iter()
            .find(|earlier| earlier.matcher.subsumes(&later.matcher))
        {
            warnings.push(Warning::Shadowed {
                rule: later.name.clone(),
                span: later.span.clone(),
                by: earlier.name.clone(),
                by_span: earlier.span.clone(),
            });
        }
    }
    warnings
}

/// The TOML shape, one struct per table, every pointable value in `Spanned`.
mod raw {
    use std::collections::BTreeMap;

    use serde::Deserialize;
    use toml::Spanned;

    use super::{Mode, OnConflict, OnDuplicate, Symlinks};

    /// A string or a list of them, so `ext = "zip"` and `ext = ["zip", "7z"]`
    /// both read.
    #[derive(Debug, Clone, Deserialize)]
    #[serde(untagged)]
    pub enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    impl OneOrMany {
        pub fn into_vec(self) -> Vec<String> {
            match self {
                Self::One(one) => vec![one],
                Self::Many(many) => many,
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct PolicyFile {
        pub folder: Folder,
        pub defaults: Option<Defaults>,
        #[serde(default, rename = "rule")]
        pub rules: Vec<Rule>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Folder {
        pub name: Spanned<String>,
        pub mode: Option<Spanned<Mode>>,
        pub inbox: Option<Spanned<String>>,
        #[serde(default)]
        pub ignore: Vec<Spanned<String>>,
        #[serde(default)]
        pub opaque: Vec<Spanned<String>>,
        pub symlinks: Option<Spanned<Symlinks>>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Defaults {
        pub on_duplicate: Option<Spanned<OnDuplicate>>,
        pub on_conflict: Option<Spanned<OnConflict>>,
        pub cooldown: Option<Spanned<String>>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Rule {
        pub name: Spanned<String>,
        pub path: Spanned<String>,
        pub r#match: Option<Match>,
        #[serde(default)]
        pub vars: BTreeMap<String, Spanned<Var>>,
        pub rename: Option<Spanned<String>>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Match {
        pub ext: Option<Spanned<OneOrMany>>,
        pub mime: Option<Spanned<OneOrMany>>,
        pub glob: Option<Spanned<OneOrMany>>,
        pub regex: Option<Spanned<String>>,
        pub size: Option<Spanned<String>>,
        pub age: Option<Spanned<String>>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Var {
        pub from: Spanned<OneOrMany>,
        pub tz: Option<Spanned<String>>,
        pub map: Option<Spanned<BTreeMap<String, String>>>,
        pub bucket: Option<Spanned<Vec<String>>>,
    }
}
