//! What can be wrong with a policy, with byte spans as plain data.
//!
//! No `miette` here. A span is a `Range<usize>` into the policy text so the
//! CLI can draw it and the window can render it differently later, without
//! either of them needing this crate to know how.

use std::fmt;
use std::ops::Range;

use serde::Serialize;

/// A policy that cannot be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// The TOML itself, or a field's type, was wrong.
    #[error("{message}")]
    Syntax {
        /// As reported by the parser, without its own position prefix.
        message: String,
        /// Where, when the parser could say.
        span: Option<Range<usize>>,
    },

    /// A `path` or `rename` template that cannot be used.
    #[error("rule `{rule}`: {message}")]
    Template {
        /// The rule whose template it is.
        rule: String,
        /// What is wrong with it.
        message: String,
        /// The offending character or placeholder.
        span: Range<usize>,
    },

    /// A `match` constraint that cannot be compiled.
    #[error("rule `{rule}`, match.{field}: {message}")]
    Match {
        /// The rule the constraint belongs to.
        rule: String,
        /// Which constraint.
        field: &'static str,
        /// What is wrong with it.
        message: String,
        /// The constraint's value in the file.
        span: Range<usize>,
    },

    /// A `vars.<name>` definition that cannot be used.
    #[error("rule `{rule}`, vars.{var}: {message}")]
    Var {
        /// The rule the variable belongs to.
        rule: String,
        /// The variable's name.
        var: String,
        /// What is wrong with it.
        message: String,
        /// The definition, or the part of it that is wrong.
        span: Range<usize>,
    },

    /// A `[folder]` setting that cannot be used.
    #[error("[folder].{field}: {message}")]
    Folder {
        /// Which setting.
        field: &'static str,
        /// What is wrong with it.
        message: String,
        /// The setting's value in the file.
        span: Range<usize>,
    },

    /// A `[defaults]` setting that cannot be used.
    #[error("[defaults].{field}: {message}")]
    Defaults {
        /// Which setting.
        field: &'static str,
        /// What is wrong with it.
        message: String,
        /// The setting's value in the file.
        span: Range<usize>,
    },

    /// Two rules share a name, so a trace could not tell them apart.
    #[error("rule `{rule}` is declared twice")]
    DuplicateRule {
        /// The name.
        rule: String,
        /// The second declaration.
        span: Range<usize>,
        /// The first.
        first: Range<usize>,
    },
}

impl PolicyError {
    /// Where in the policy text the problem is, when known.
    #[must_use]
    pub fn span(&self) -> Option<Range<usize>> {
        match self {
            Self::Syntax { span, .. } => span.clone(),
            Self::Template { span, .. }
            | Self::Match { span, .. }
            | Self::Var { span, .. }
            | Self::Folder { span, .. }
            | Self::Defaults { span, .. }
            | Self::DuplicateRule { span, .. } => Some(span.clone()),
        }
    }

    /// A second location worth pointing at, when there is one.
    #[must_use]
    pub fn related(&self) -> Option<(&'static str, Range<usize>)> {
        match self {
            Self::DuplicateRule { first, .. } => Some(("first declared here", first.clone())),
            _ => None,
        }
    }
}

/// Something a policy loader noticed that is not an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// A rule that can never fire, because an earlier one takes everything it
    /// would match. Reported only when that is provable: literal `ext` and
    /// `mime` sets and size or age ranges. Globs and regexes stay silent,
    /// since deciding whether two regexes overlap is not a loader's job.
    Shadowed {
        /// The rule that can never fire.
        rule: String,
        /// Its name in the file.
        span: Range<usize>,
        /// The earlier rule that takes everything it would match.
        by: String,
        /// That rule's name in the file.
        by_span: Range<usize>,
    },
}

impl Warning {
    /// Where the warning points.
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        match self {
            Self::Shadowed { span, .. } => span.clone(),
        }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shadowed { rule, by, .. } => write!(
                f,
                "rule `{rule}` can never fire: everything it matches is taken by `{by}` first"
            ),
        }
    }
}

/// The 1-based line and column of a byte offset in `text`.
///
/// Lives here rather than in the CLI because a snapshot test in this crate
/// wants to say "line 14" too, and two implementations of line counting are
/// one too many.
#[must_use]
pub fn line_of(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rfind('\n').map_or(offset, |nl| offset - nl - 1) + 1;
    (line, column)
}
