//! The template language inside `path` and `rename`: `{var}`,
//! `{var:%Y-%m-%d}`, `{var|lower|trunc:8}`.
//!
//! A hand-written parser. The grammar is four productions and never grows
//! past `{`, `:`, `|` and `}`; a parser generator would be more code than
//! this file and would put a build step between the reader and the rules.

use std::fmt;
use std::ops::Range;

use jiff::fmt::strtime;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::attrs::Value;

/// One filter in a chain. The DECIDED v1 set, no more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// ASCII lower-case.
    Lower,
    /// ASCII upper-case.
    Upper,
    /// Lower-case ASCII letters and digits, everything else a single dash.
    Slug,
    /// Strip what no backend accepts in a name. Applied last regardless.
    Sanitize,
    /// Keep the first N characters.
    Trunc(usize),
    /// Left-pad with zeros to N characters.
    Pad(usize),
    /// The first N hex characters of the value's BLAKE3.
    Hash(usize),
    /// What to use when the variable is absent.
    Default(String),
}

impl Filter {
    /// Parse `name` or `name:arg`.
    fn parse(text: &str) -> Result<Self, String> {
        let (name, arg) = match text.split_once(':') {
            Some((name, arg)) => (name.trim(), Some(arg.trim())),
            None => (text.trim(), None),
        };
        let number = |arg: Option<&str>| -> Result<usize, String> {
            arg.ok_or_else(|| format!("`{name}` needs a number, as in `{name}:8`"))?
                .parse()
                .map_err(|_| format!("`{name}` needs a number, as in `{name}:8`"))
        };
        match name {
            "lower" | "upper" | "slug" | "sanitize" if arg.is_some() => {
                Err(format!("`{name}` takes no argument"))
            }
            "lower" => Ok(Self::Lower),
            "upper" => Ok(Self::Upper),
            "slug" => Ok(Self::Slug),
            "sanitize" => Ok(Self::Sanitize),
            "trunc" => number(arg).map(Self::Trunc),
            "pad" => number(arg).map(Self::Pad),
            "hash" => number(arg).map(Self::Hash),
            "default" => {
                let raw = arg
                    .ok_or_else(|| "`default` needs a value, as in `default:\"x\"`".to_string())?;
                let unquoted = raw
                    .strip_prefix('"')
                    .and_then(|r| r.strip_suffix('"'))
                    .ok_or_else(|| {
                        "`default` takes a quoted value, as in `default:\"x\"`".to_string()
                    })?;
                Ok(Self::Default(unquoted.to_string()))
            }
            other => Err(format!(
                "`{other}` is not a filter; the filters are lower, upper, slug, sanitize, \
                 trunc:N, pad:N, hash:N and default:\"x\""
            )),
        }
    }

    /// The filter as it was written, for traces.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Lower => "lower".into(),
            Self::Upper => "upper".into(),
            Self::Slug => "slug".into(),
            Self::Sanitize => "sanitize".into(),
            Self::Trunc(n) => format!("trunc:{n}"),
            Self::Pad(n) => format!("pad:{n}"),
            Self::Hash(n) => format!("hash:{n}"),
            Self::Default(v) => format!("default:\"{v}\""),
        }
    }

    fn apply(&self, text: &str) -> String {
        match self {
            Self::Lower => text.to_lowercase(),
            Self::Upper => text.to_uppercase(),
            Self::Slug => slug(text),
            Self::Sanitize => sanitize(text),
            Self::Trunc(n) => text.chars().take(*n).collect(),
            Self::Pad(n) => {
                let width = text.chars().count();
                if width >= *n {
                    text.to_string()
                } else {
                    format!("{}{text}", "0".repeat(n - width))
                }
            }
            Self::Hash(n) => {
                let hex = blake3::hash(text.as_bytes()).to_hex();
                hex.chars().take(*n).collect()
            }
            // Only meaningful when the value is absent; on a present value
            // it is a no-op, which is what a default should be.
            Self::Default(_) => text.to_string(),
        }
    }
}

/// One piece of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// Text copied through as written.
    Literal(String),
    /// A placeholder.
    Var {
        /// The variable or attribute name.
        name: String,
        /// The `:format`, if any. strftime for times, a length for text.
        format: Option<String>,
        /// The `|filter` chain, in order.
        filters: Vec<Filter>,
        /// Where the placeholder sits in the template text.
        span: Range<usize>,
    },
}

/// Where in a template something went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    /// What is wrong.
    pub message: String,
    /// The offending character or placeholder, as offsets into the template.
    pub span: Range<usize>,
}

/// A parsed `path` or `rename` template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// The text as written.
    pub source: String,
    /// The parsed pieces.
    pub parts: Vec<Part>,
}

impl Template {
    /// Parse a template. `{{` and `}}` are literal braces.
    ///
    /// # Errors
    /// [`TemplateError`] pointing at the first bad character or placeholder.
    pub fn parse(source: &str) -> Result<Self, TemplateError> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = source.char_indices().peekable();

        while let Some((at, c)) = chars.next() {
            match c {
                '{' if chars.peek().is_some_and(|(_, n)| *n == '{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek().is_some_and(|(_, n)| *n == '}') => {
                    chars.next();
                    literal.push('}');
                }
                '}' => {
                    return Err(TemplateError {
                        message: "`}` without a matching `{`; write `}}` for a literal brace"
                            .to_string(),
                        span: at..at + 1,
                    });
                }
                '{' => {
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    let Some(close) = source[at..].find('}').map(|i| at + i) else {
                        return Err(TemplateError {
                            message: "`{` is never closed".to_string(),
                            span: at..at + 1,
                        });
                    };
                    let body = &source[at + 1..close];
                    // A `{` inside the body means this placeholder was never
                    // closed and the `}` found belongs to the next one.
                    if body.contains('{') {
                        return Err(TemplateError {
                            message: "`{` is never closed".to_string(),
                            span: at..at + 1,
                        });
                    }
                    parts.push(parse_placeholder(body, at..close + 1)?);
                    // Skip past the body and the closing brace.
                    while chars.next_if(|(i, _)| *i < close + 1).is_some() {}
                }
                other => literal.push(other),
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        Ok(Self {
            source: source.to_string(),
            parts,
        })
    }

    /// Every placeholder, in order of appearance.
    pub fn placeholders(
        &self,
    ) -> impl Iterator<Item = (&str, Option<&str>, &[Filter], &Range<usize>)> {
        self.parts.iter().filter_map(|part| match part {
            Part::Var {
                name,
                format,
                filters,
                span,
            } => Some((name.as_str(), format.as_deref(), filters.as_slice(), span)),
            Part::Literal(_) => None,
        })
    }

    /// Render against `lookup`, which answers a name with its value and the
    /// zone its times should be shown in.
    ///
    /// # Errors
    /// [`RenderError`] for the first placeholder with no value and no
    /// `default`, or whose `:format` does not fit its value.
    pub fn render(
        &self,
        mut lookup: impl FnMut(&str) -> Option<(Value, TimeZone)>,
    ) -> Result<Rendered, RenderError> {
        let mut text = String::new();
        let mut steps = Vec::new();
        for part in &self.parts {
            match part {
                Part::Literal(s) => text.push_str(s),
                Part::Var {
                    name,
                    format,
                    filters,
                    span,
                } => {
                    let placeholder = self.source[span.clone()].to_string();
                    let found = lookup(name);
                    let raw = found.as_ref().map(|(value, _)| value.as_text());
                    let mut current = match found {
                        Some((value, tz)) => format_value(name, &value, format.as_deref(), &tz)?,
                        None => match filters.iter().find_map(|f| match f {
                            Filter::Default(d) => Some(d.clone()),
                            _ => None,
                        }) {
                            Some(default) => default,
                            None => {
                                return Err(RenderError {
                                    var: name.clone(),
                                    message: format!("`{name}` has no value for this file"),
                                });
                            }
                        },
                    };
                    let mut applied = Vec::new();
                    for filter in filters {
                        current = filter.apply(&current);
                        applied.push(FilterStep {
                            filter: filter.name(),
                            result: current.clone(),
                        });
                    }
                    text.push_str(&current);
                    steps.push(Step {
                        placeholder,
                        variable: name.clone(),
                        raw,
                        filters: applied,
                        result: current,
                    });
                }
            }
        }
        Ok(Rendered {
            template: self.source.clone(),
            steps,
            text,
        })
    }
}

fn parse_placeholder(body: &str, span: Range<usize>) -> Result<Part, TemplateError> {
    let at = |offset: usize| span.start + 1 + offset;
    let mut segments = body.split('|');
    let head = segments.next().unwrap_or_default();
    let (name, format) = match head.split_once(':') {
        Some((name, format)) => (name.trim(), Some(format.to_string())),
        None => (head.trim(), None),
    };
    if name.is_empty() {
        return Err(TemplateError {
            message: "a placeholder needs a name, as in `{name}`".to_string(),
            span: span.clone(),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(TemplateError {
            message: format!("`{name}` is not a variable name"),
            span: span.clone(),
        });
    }
    if format.as_deref().is_some_and(str::is_empty) {
        return Err(TemplateError {
            message: format!("`{name}:` has an empty format"),
            span: span.clone(),
        });
    }

    let mut filters = Vec::new();
    let mut offset = head.len() + 1;
    for segment in segments {
        let filter = Filter::parse(segment).map_err(|message| TemplateError {
            message,
            span: at(offset)..at(offset + segment.len()),
        })?;
        filters.push(filter);
        offset += segment.len() + 1;
    }
    Ok(Part::Var {
        name: name.to_string(),
        format,
        filters,
        span,
    })
}

/// Turn a value into text, honouring the `:format`.
fn format_value(
    name: &str,
    value: &Value,
    format: Option<&str>,
    tz: &TimeZone,
) -> Result<String, RenderError> {
    let bad = |message: String| RenderError {
        var: name.to_string(),
        message,
    };
    match (value, format) {
        (Value::Instant(ts), format) => {
            let zoned = ts.to_zoned(tz.clone());
            strtime::format(format.unwrap_or(DEFAULT_DATE_FORMAT), &zoned)
                .map_err(|e| bad(format!("`{name}` cannot be formatted: {e}")))
        }
        (Value::Civil(dt), format) => strtime::format(format.unwrap_or(DEFAULT_DATE_FORMAT), *dt)
            .map_err(|e| bad(format!("`{name}` cannot be formatted: {e}"))),
        (value, None) => Ok(value.as_text()),
        // `{hash:8}` is the sketch's spelling for "the first eight", so a
        // number after a text value is a length.
        (Value::Text(s), Some(len)) => match len.parse::<usize>() {
            Ok(n) => Ok(s.chars().take(n).collect()),
            Err(_) => Err(bad(format!(
                "`{name}` is text, so `:{len}` must be a length; use `|` for filters"
            ))),
        },
        (Value::Size(_), Some(fmt)) => Err(bad(format!(
            "`{name}` is a size and takes no `:{fmt}` format; use `bucket` in a var"
        ))),
    }
}

/// What a date renders as when no `:format` is given.
pub const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d";

/// Check a `:format` at load time, before any file exists to render.
///
/// `known_time` says whether the variable is known to be a date; when it is,
/// the format must be a strftime string. A text variable takes a length.
///
/// # Errors
/// A message naming what was wrong, for a diagnostic.
pub fn validate_format(name: &str, format: &str, known_time: Option<bool>) -> Result<(), String> {
    let looks_like_strftime = format.contains('%');
    let looks_like_length = format.parse::<usize>().is_ok();
    match known_time {
        Some(true) if !looks_like_strftime => Err(format!(
            "`{name}` is a date, so `:{format}` must be a strftime format such as `%Y-%m-%d`"
        )),
        Some(false) if !looks_like_length => Err(format!(
            "`{name}` is text, so `:{format}` must be a length; use `|` for filters"
        )),
        _ if looks_like_strftime => {
            let sample = jiff::civil::date(2001, 2, 3).at(4, 5, 6, 0);
            strtime::format(format, sample)
                .map(drop)
                .map_err(|e| format!("`:{format}` is not a valid date format: {e}"))
        }
        _ if looks_like_length => Ok(()),
        _ => Err(format!("`:{format}` is neither a date format nor a length")),
    }
}

/// A variable that could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError {
    /// The variable.
    pub var: String,
    /// Why.
    pub message: String,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A rendered template, with each placeholder's working shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rendered {
    /// The template as written.
    pub template: String,
    /// One entry per placeholder, in order.
    pub steps: Vec<Step>,
    /// The result, before path sanitising.
    pub text: String,
}

/// One placeholder's journey from value to text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// The placeholder as written, braces included.
    pub placeholder: String,
    /// The variable it names.
    pub variable: String,
    /// The value before formatting, or `None` if a default was used.
    pub raw: Option<String>,
    /// Each filter and what it produced.
    pub filters: Vec<FilterStep>,
    /// What went into the path.
    pub result: String,
}

/// One filter's effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FilterStep {
    /// The filter as written.
    pub filter: String,
    /// The text after it.
    pub result: String,
}

/// Lower-case ASCII letters and digits; every other run becomes one dash.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Make one path segment legal on every backend tungstate can write to.
///
/// The union of the platforms' rules, because a NAS reached over SMB serves
/// Windows clients whatever it runs itself: no `<>:"/\|?*`, no control
/// characters, no trailing dot or space, and no `CON`-style device names.
#[must_use]
pub fn sanitize(segment: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let mut out: String = segment
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    let base = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&base.as_str()) {
        out.insert(0, '_');
    }
    out
}

/// Turn rendered text into a canonical relative path.
///
/// This is the security boundary of the template language. A filename is
/// attacker-controlled input and it flows into a path, so every segment is
/// sanitised, and `.`, `..` and empty segments are dropped rather than
/// escaped. The result is always relative and can never leave the root.
#[must_use]
pub fn canonical_path(rendered: &str) -> String {
    rendered
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(sanitize)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}
