//! `tungstate explain` and `tungstate policy validate`: the decision for one
//! file, with every step of the working shown.
//!
//! Finding the folder and drawing a policy diagnostic live in
//! [`crate::folder`], because `plan` needs both too.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;
use tungstate_attrs::gather;
use tungstate_backend::local::LocalBackend;
use tungstate_core::{Explanation, Loaded, Outcome, RuleResult, Warning, line_of};

use crate::folder::{Located, describe_tier, load, locate, print_warnings};
use tungstate_journal::{Journal, Locator, ends};

/// `tungstate policy validate [--policy FILE]`.
pub fn validate(policy: Option<&Path>) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let located = match locate(&cwd, policy) {
        Ok(located) => located,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok((text, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };
    let tier = loaded.policy.required_tier();
    println!(
        "ok: {} — folder \"{}\", {} rule(s), {}",
        located.policy_name,
        loaded.policy.folder.name,
        loaded.policy.rules.len(),
        describe_tier(tier)
    );
    print_warnings(&located, &text, &loaded.warnings);
    ExitCode::SUCCESS
}

/// What `--json` emits: the classifier's trace plus what the command line
/// added around it.
#[derive(Serialize)]
struct ExplainJson<'a> {
    policy: &'a str,
    folder: &'a str,
    #[serde(flatten)]
    explanation: &'a Explanation,
    warnings: &'a [Warning],
}

/// `tungstate explain <TARGET> [--policy FILE] [--json]`.
///
/// `TARGET` is a path, or `connection:path` — the same spelling `link add`
/// takes, through the same parser. A remote target is where the cost tiers
/// stop being theoretical: a `head`-tier policy asks the far side for 8 KiB
/// rather than pulling the file across to decide what it is.
pub fn explain(target: &str, policy: Option<&Path>, json: bool) -> ExitCode {
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };
    let endpoint = match ends::parse_end(target, None, &journal) {
        Ok(endpoint) => endpoint,
        Err(error) => return crate::fail(&error),
    };

    match endpoint.connection {
        None => explain_local(Path::new(target), policy, json, &journal),
        Some(_) => explain_remote(&endpoint, target, policy, json, &journal),
    }
}

/// A file on this machine: walk up for the policy, and the folder it governs
/// is the root every path in the trace is relative to.
fn explain_local(path: &Path, policy: Option<&Path>, json: bool, journal: &Journal) -> ExitCode {
    let located = match locate(path, policy) {
        Ok(located) => located,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok((text, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };

    let absolute = match path.canonicalize() {
        Ok(absolute) => absolute,
        Err(error) => {
            eprintln!("error: cannot read `{}`: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let Ok(relative) = absolute.strip_prefix(&located.root) else {
        eprintln!(
            "error: `{}` is not inside `{}`, the folder this policy governs",
            absolute.display(),
            located.root.display()
        );
        return ExitCode::FAILURE;
    };

    let backend = LocalBackend::new(located.root.clone());
    let source = source_of(journal, &absolute);
    report(&backend, relative, source, &located, &text, &loaded, json)
}

/// A file reached through a connection.
///
/// The connection's own root stands in for the governed folder, so `{parent}`
/// means what it means locally. The policy must be named: walking a remote
/// tree looking for `.tungstate/policy.toml` is a round trip per level, and
/// DESIGN §2 puts a remote folder's policy in the central config directory
/// rather than in the folder anyway.
fn explain_remote(
    endpoint: &tungstate_journal::Endpoint,
    target: &str,
    policy: Option<&Path>,
    json: bool,
    journal: &Journal,
) -> ExitCode {
    let Some(policy) = policy else {
        eprintln!(
            "error: `{target}` is on a connection, so the policy has to be named:\n  \
             tungstate explain {target} --policy <FILE>"
        );
        return ExitCode::from(2);
    };
    let Some(id) = endpoint.connection else {
        unreachable!("explain_remote is only called for a remote endpoint");
    };
    let name = match journal.connection_by_id(id) {
        Ok(connection) => connection.name,
        Err(error) => return crate::fail(&error),
    };

    let root = tungstate_journal::Endpoint::remote(id, PathBuf::new());
    let backend =
        match tungstate_backend_opendal::open(&root, journal, crate::secret_store().as_ref()) {
            Ok(backend) => backend,
            Err(error) => return crate::fail(&error),
        };

    let located = Located {
        root: PathBuf::from(format!("{name}:")),
        policy: policy.to_path_buf(),
        policy_name: policy.display().to_string(),
    };
    let Ok((text, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };
    let relative = endpoint.path.clone();
    report(
        backend.as_ref(),
        &relative,
        None,
        &located,
        &text,
        &loaded,
        json,
    )
}

/// Gather the attributes the policy's tier allows, classify, and print.
fn report(
    backend: &dyn tungstate_backend::Backend,
    relative: &Path,
    source: Option<String>,
    located: &Located,
    text: &str,
    loaded: &Loaded,
    json: bool,
) -> ExitCode {
    let tier = loaded.policy.required_tier();
    let mut attrs = match gather(backend, relative, tier) {
        Ok(attrs) => attrs,
        Err(error) => return crate::fail(&error),
    };
    attrs.source = source;

    let explanation = loaded.policy.explain(&attrs);

    if json {
        let mut value = serde_json::to_value(ExplainJson {
            policy: &located.policy_name,
            folder: &loaded.policy.folder.name,
            explanation: &explanation,
            warnings: &loaded.warnings,
        })
        .expect("a trace is plain data and always serialises");
        // Spans are what the crate carries; lines are what a reader wants.
        if let Some(rules) = value.get_mut("rules").and_then(|r| r.as_array_mut()) {
            for rule in rules {
                let start = rule["span"]["start"].as_u64().unwrap_or(0);
                let (line, _) = line_of(text, usize::try_from(start).unwrap_or(0));
                rule["line"] = serde_json::Value::from(line);
            }
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("a JSON value always prints")
        );
        return ExitCode::SUCCESS;
    }

    print!(
        "{}",
        render(&explanation, &located.policy_name, loaded, text)
    );
    print_warnings(located, text, &loaded.warnings);
    ExitCode::SUCCESS
}

/// Which link brought this file here, if the journal knows.
///
/// A missing or unreadable journal is not an error for `explain`: the
/// answer is "nobody knows", which is exactly what `source` being absent
/// means to a template.
fn source_of(journal: &Journal, absolute: &Path) -> Option<String> {
    journal
        .whereis(&Locator::Path(absolute))
        .ok()?
        .into_iter()
        .find(|op| {
            op.destination
                .as_ref()
                .is_some_and(|d| d.connection.is_none() && d.root.join(&d.path) == absolute)
        })
        .and_then(|op| op.link)
}

/// The human trace. Every rule in order, whether it matched and which
/// constraint decided it; then for the winner, each variable, where its
/// value came from and what that cost, and each template's working.
fn render(explanation: &Explanation, policy_name: &str, loaded: &Loaded, text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let policy = &loaded.policy;

    let _ = writeln!(out, "{}", explanation.file);
    let _ = writeln!(
        out,
        "  policy {policy_name} (folder \"{}\"), {}",
        policy.folder.name,
        describe_tier(explanation.tier)
    );

    let _ = writeln!(out, "\nrules, in file order");
    let width = explanation
        .rules
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0);
    for (index, rule) in explanation.rules.iter().enumerate() {
        let (line, _) = line_of(text, rule.span.start);
        let verdict = match &rule.result {
            RuleResult::Matched => "matched".to_string(),
            RuleResult::AlsoMatches { taken_by } => {
                format!("also matches, but `{taken_by}` comes first")
            }
            RuleResult::Failed { constraint, reason } => format!("no: {constraint} — {reason}"),
        };
        let at = format!("(line {line})");
        let _ = writeln!(
            out,
            "  {:>2}. {:<width$}  {at:<11} {verdict}",
            index + 1,
            rule.name,
        );
    }
    if explanation.rules.is_empty() {
        let _ = writeln!(out, "  (the policy has no rules)");
    }

    match &explanation.outcome {
        Outcome::Ignored { because } => {
            let _ = writeln!(out, "\nignored: {because}; the file is never touched");
        }
        Outcome::Opaque { pattern } => {
            let _ = writeln!(
                out,
                "\ninside an opaque unit (`{pattern}`); tungstate never looks inside it"
            );
        }
        Outcome::Unmatched { inbox } => match inbox {
            Some(inbox) => {
                let _ = writeln!(out, "\nno rule matched; it would go to the inbox `{inbox}`");
            }
            None => {
                let _ = writeln!(out, "\nno rule matched; it stays where it is");
            }
        },
        Outcome::Unresolvable {
            rule,
            vars,
            var,
            reason,
        } => {
            render_vars(&mut out, rule, vars);
            let _ = writeln!(
                out,
                "\n`{rule}` matched, but `{{{var}}}` has no value: {reason}\n\
                 the file cannot be placed by this rule; give the variable a fallback \
                 (`from = [..., \"mtime\"]`) or a `default:\"...\"` filter"
            );
        }
        Outcome::Routed {
            rule,
            vars,
            path,
            rename,
            destination,
            in_place,
        } => {
            render_vars(&mut out, rule, vars);
            render_template(&mut out, "path", path);
            if let Some(rename) = rename {
                render_template(&mut out, "rename", rename);
            }
            let _ = writeln!(out, "\ndestination  {destination}");
            if *in_place {
                let _ = writeln!(out, "already there; nothing would move");
            }
        }
    }
    out
}

fn render_vars(out: &mut String, rule: &str, vars: &[tungstate_core::VarTrace]) {
    use std::fmt::Write as _;
    if vars.is_empty() {
        return;
    }
    let _ = writeln!(out, "\nvariables for `{rule}`");
    let width = vars.iter().map(|v| v.name.len()).max().unwrap_or(0);
    for var in vars {
        let mut provenance = Vec::new();
        if !var.skipped.is_empty() && var.source.is_some() {
            provenance.push(format!("{} absent", var.skipped.join(", ")));
        }
        if let (Some(source), Some(tier)) = (&var.source, var.tier) {
            provenance.push(format!("from {source} ({})", tier.as_str()));
        }
        if let (Some(raw), Some(value)) = (&var.raw, &var.value)
            && raw != value
        {
            provenance.push(format!("= {}", shorten(raw)));
        }
        if let Some(via) = &var.via {
            provenance.push(via.clone());
        }
        if let Some(tz) = &var.tz
            && var
                .source
                .as_deref()
                .is_some_and(|s| s == "mtime" || s == "now" || s.starts_with("exif."))
        {
            provenance.push(format!("tz {tz}"));
        }
        let shown = match (&var.value, &var.missing) {
            (Some(value), _) => shorten(value),
            (None, Some(missing)) => format!("(no value: {missing})"),
            (None, None) => "(no value)".to_string(),
        };
        let line = format!(
            "  {:<width$}  = {:<24} {}",
            var.name,
            shown,
            provenance.join(", ")
        );
        let _ = writeln!(out, "{}", line.trim_end());
    }
}

fn render_template(out: &mut String, label: &str, rendered: &tungstate_core::template::Rendered) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "\n{label:<7} {}", rendered.template);
    let width = rendered
        .steps
        .iter()
        .map(|s| s.placeholder.len())
        .max()
        .unwrap_or(0);
    for step in &rendered.steps {
        let mut working = String::new();
        if !step.filters.is_empty() {
            // No raw value means a `default:` filter supplied one, so the
            // chain starts from nothing and the trace should say so.
            let mut previous = step.raw.clone().unwrap_or_else(|| "(absent)".to_string());
            let mut chain = Vec::new();
            for filter in &step.filters {
                if filter.result == previous {
                    chain.push(format!("{}: unchanged", filter.filter));
                } else {
                    chain.push(format!(
                        "{}: {} -> {}",
                        filter.filter,
                        shorten(&previous),
                        shorten(&filter.result)
                    ));
                }
                previous.clone_from(&filter.result);
            }
            working = format!("   ({})", chain.join("; "));
        }
        let _ = writeln!(
            out,
            "        {:<width$}  -> {}{working}",
            step.placeholder,
            shorten(&step.result),
        );
    }
    let _ = writeln!(out, "        = {}", rendered.text);
}

/// A hash is 64 characters and a trace is a table; keep columns readable.
fn shorten(text: &str) -> String {
    const MAX: usize = 32;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let head: String = text.chars().take(MAX - 1).collect();
        format!("{head}…")
    }
}
