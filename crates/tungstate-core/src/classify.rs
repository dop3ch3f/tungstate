//! Where a file belongs, and the trace that says why.
//!
//! The trace is not a debugging afterthought bolted onto the decision. The
//! decision *is* the trace: `explain` and the planner (slice 6) read the same
//! [`Explanation`], so what the command line shows is what will happen.

use std::collections::BTreeMap;
use std::ops::Range;

use jiff::tz::TimeZone;
use serde::Serialize;

use crate::attrs::{Attributes, Tier, Value, tier_of};
use crate::matcher::Verdict;
use crate::policy::{Policy, Rule, Symlinks};
use crate::template::{Rendered, Template, canonical_path};
use crate::vars::Unresolved;

/// The whole decision for one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    /// The file, relative to the folder root.
    pub file: String,
    /// The tier the policy as a whole needs.
    pub tier: Tier,
    /// Every rule in file order, and how it fared.
    pub rules: Vec<RuleTrace>,
    /// What happens to the file.
    pub outcome: Outcome,
}

/// How one rule fared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleTrace {
    /// The rule's name.
    pub name: String,
    /// Where the rule is declared, for the trace to name a line.
    pub span: Range<usize>,
    /// What happened.
    pub result: RuleResult,
}

/// Whether a rule took the file, and if not why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleResult {
    /// This rule took the file.
    Matched,
    /// This rule fits the file too, but an earlier one took it first.
    ///
    /// Rules after the winner are still evaluated so the trace can show
    /// what reordering would do: that is the whole argument for file order
    /// being the precedence.
    AlsoMatches {
        /// The rule that came first.
        taken_by: String,
    },
    /// One constraint did not hold.
    Failed {
        /// Which constraint.
        constraint: String,
        /// What the file had instead.
        reason: String,
    },
}

/// What happens to the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Left alone because `[folder].ignore` or `symlinks` says so.
    Ignored {
        /// The pattern or setting that said so.
        because: String,
    },
    /// Inside a unit `[folder].opaque` says never to look inside.
    Opaque {
        /// The pattern that named the unit.
        pattern: String,
    },
    /// A rule took the file and its destination rendered.
    Routed {
        /// The rule.
        rule: String,
        /// Every variable the templates used, with its provenance.
        vars: Vec<VarTrace>,
        /// The `path` template's working.
        path: Rendered,
        /// The `rename` template's working, if the rule has one.
        rename: Option<Rendered>,
        /// Where the file belongs, relative to the folder root.
        destination: String,
        /// True when it is already there.
        in_place: bool,
    },
    /// A rule took the file but a variable it needs has no value.
    Unresolvable {
        /// The rule.
        rule: String,
        /// Every variable the templates used, resolved or not.
        vars: Vec<VarTrace>,
        /// The variable that stopped it.
        var: String,
        /// Why.
        reason: String,
    },
    /// No rule took the file.
    Unmatched {
        /// Where it goes instead, if `[folder].inbox` is set.
        inbox: Option<String>,
    },
}

/// One variable's provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VarTrace {
    /// The variable's name as the template wrote it.
    pub name: String,
    /// Sources that were absent before one resolved, in order.
    pub skipped: Vec<String>,
    /// The attribute that supplied the value.
    pub source: Option<String>,
    /// What reading that attribute cost.
    pub tier: Option<Tier>,
    /// The attribute's value before relabelling.
    pub raw: Option<String>,
    /// Which `map` entry or `bucket` label applied.
    pub via: Option<String>,
    /// The value handed to the template.
    pub value: Option<String>,
    /// The zone times are shown in.
    pub tz: Option<String>,
    /// Why there is no value, when there is none.
    pub missing: Option<Unresolved>,
}

impl Policy {
    /// Decide where one file belongs and say why.
    ///
    /// Pure: the same attributes and policy always give the same answer.
    #[must_use]
    pub fn explain(&self, attrs: &Attributes) -> Explanation {
        let file = attrs.relative_path();
        let tier = self.required_tier();
        let outcome_early = self.pre_rules(attrs, &file);

        let mut rules = Vec::with_capacity(self.rules.len());
        let mut winner: Option<(usize, BTreeMap<String, String>)> = None;
        for (index, rule) in self.rules.iter().enumerate() {
            let result = match rule.matcher.test(attrs) {
                Verdict::Matched { captures } => match &winner {
                    None => {
                        winner = Some((index, captures));
                        RuleResult::Matched
                    }
                    Some((first, _)) => RuleResult::AlsoMatches {
                        taken_by: self.rules[*first].name.clone(),
                    },
                },
                Verdict::Failed { constraint, reason } => RuleResult::Failed {
                    constraint: constraint.to_string(),
                    reason,
                },
            };
            rules.push(RuleTrace {
                name: rule.name.clone(),
                span: rule.span.clone(),
                result,
            });
        }

        let outcome = match outcome_early {
            Some(early) => early,
            None => match winner {
                None => Outcome::Unmatched {
                    inbox: self.folder.inbox.clone(),
                },
                Some((index, captures)) => route(&self.rules[index], attrs, &captures, &file),
            },
        };

        Explanation {
            file,
            tier,
            rules,
            outcome,
        }
    }

    /// Where one file belongs, without the working shown.
    ///
    /// The same decision [`Policy::explain`] reaches, and a property test holds
    /// the two to it. This one stops at the first rule that matches and builds
    /// no trace, because `explain` deliberately evaluates every rule so it can
    /// report what reordering would do — the right trade for one file, and
    /// millions of discarded allocations for a folder of two hundred thousand.
    #[must_use]
    pub fn place(&self, attrs: &Attributes) -> Outcome {
        let file = attrs.relative_path();
        if let Some(early) = self.pre_rules(attrs, &file) {
            return early;
        }
        for rule in &self.rules {
            if let Verdict::Matched { captures } = rule.matcher.test(attrs) {
                return route(rule, attrs, &captures, &file);
            }
        }
        Outcome::Unmatched {
            inbox: self.folder.inbox.clone(),
        }
    }

    /// The checks that come before any rule: ignore, symlinks, opaque.
    fn pre_rules(&self, attrs: &Attributes, file: &str) -> Option<Outcome> {
        if let Some(pattern) = self.folder.ignore.first_match_including_ancestors(file) {
            return Some(Outcome::Ignored {
                because: format!("ignore `{pattern}`"),
            });
        }
        if attrs.is_symlink && self.folder.symlinks == Symlinks::Ignore {
            return Some(Outcome::Ignored {
                because: "symlinks = \"ignore\"".to_string(),
            });
        }
        if let Some(pattern) = self.folder.opaque.first_match_including_ancestors(file) {
            return Some(Outcome::Opaque {
                pattern: pattern.to_string(),
            });
        }
        None
    }
}

/// Render the winning rule's templates for the file.
fn route(
    rule: &Rule,
    attrs: &Attributes,
    captures: &BTreeMap<String, String>,
    file: &str,
) -> Outcome {
    let mut vars: Vec<VarTrace> = Vec::new();
    let mut values: BTreeMap<String, (Value, TimeZone)> = BTreeMap::new();
    let mut failure: Option<(String, String)> = None;

    // Resolve every placeholder once, in order of first appearance, so the
    // trace reads top to bottom the way the template does.
    let placeholders = rule
        .path
        .placeholders()
        .chain(rule.rename.iter().flat_map(Template::placeholders))
        .map(|(name, _, _, _)| name.to_string());
    for name in placeholders {
        if vars.iter().any(|v| v.name == name) {
            continue;
        }
        let (trace, typed) = resolve_one(rule, &name, attrs, captures);
        if let Some(pair) = typed {
            values.insert(name, pair);
        }
        vars.push(trace);
    }

    let lookup = |name: &str| values.get(name).cloned();
    let path = match rule.path.render(lookup) {
        Ok(rendered) => rendered,
        Err(error) => {
            failure.get_or_insert((error.var.clone(), error.message.clone()));
            Rendered {
                template: rule.path.source.clone(),
                steps: Vec::new(),
                text: String::new(),
            }
        }
    };
    let rename = rule
        .rename
        .as_ref()
        .map(|template| match template.render(lookup) {
            Ok(rendered) => Some(rendered),
            Err(error) => {
                failure.get_or_insert((error.var.clone(), error.message.clone()));
                None
            }
        });

    if let Some((var, reason)) = failure {
        return Outcome::Unresolvable {
            rule: rule.name.clone(),
            vars,
            var,
            reason,
        };
    }

    let rename = rename.flatten();
    let filename = rename
        .as_ref()
        .map_or_else(|| attrs.name.clone(), |r| r.text.clone());
    // The name is sanitised on its own so a rename can never introduce a
    // directory level; the path part may, since `{parent}` is meant to.
    let directory = canonical_path(&path.text);
    let name = canonical_path(&filename.replace(['/', '\\'], "_"));
    let name = if name.is_empty() {
        "_".to_string()
    } else {
        name
    };
    let destination = if directory.is_empty() {
        name
    } else {
        format!("{directory}/{name}")
    };
    Outcome::Routed {
        rule: rule.name.clone(),
        vars,
        path,
        rename,
        in_place: destination == file,
        destination,
    }
}

/// A placeholder that names no `vars` entry: a capture or an attribute.
fn bare_value(
    name: &str,
    attrs: &Attributes,
    captures: &BTreeMap<String, String>,
) -> Option<Value> {
    captures
        .get(name)
        .map(|s| Value::Text(s.clone()))
        .or_else(|| attrs.get(name))
}

/// One placeholder's value and its trace. A `vars` entry is resolved through
/// its `from` chain; anything else is a regex capture or a bare attribute.
fn resolve_one(
    rule: &Rule,
    name: &str,
    attrs: &Attributes,
    captures: &BTreeMap<String, String>,
) -> (VarTrace, Option<(Value, TimeZone)>) {
    if let Some(def) = rule.vars.iter().find(|v| v.name == name) {
        return match def.resolve(attrs, captures) {
            Ok(resolved) => (
                VarTrace {
                    name: name.to_string(),
                    skipped: resolved.skipped,
                    source: Some(resolved.source),
                    tier: Some(resolved.tier),
                    raw: Some(resolved.raw),
                    via: resolved.via,
                    value: Some(resolved.value.as_text()),
                    tz: Some(def.tz_name.clone()),
                    missing: None,
                },
                Some((resolved.value, def.tz.clone())),
            ),
            Err(missing) => (
                VarTrace {
                    name: name.to_string(),
                    skipped: def.from.clone(),
                    source: None,
                    tier: None,
                    raw: None,
                    via: None,
                    value: None,
                    tz: Some(def.tz_name.clone()),
                    missing: Some(missing),
                },
                None,
            ),
        };
    }
    match bare_value(name, attrs, captures) {
        Some(value) => (
            VarTrace {
                name: name.to_string(),
                skipped: Vec::new(),
                source: Some(if captures.contains_key(name) {
                    "match.regex".to_string()
                } else {
                    name.to_string()
                }),
                tier: Some(tier_of(name).unwrap_or(Tier::Stat)),
                raw: Some(value.as_text()),
                via: None,
                value: Some(value.as_text()),
                tz: Some("local".to_string()),
                missing: None,
            },
            Some((value, TimeZone::system())),
        ),
        None => (
            VarTrace {
                name: name.to_string(),
                skipped: vec![name.to_string()],
                source: None,
                tier: None,
                raw: None,
                via: None,
                value: None,
                tz: None,
                missing: Some(Unresolved::Absent {
                    tried: vec![name.to_string()],
                }),
            },
            None,
        ),
    }
}
