//! Finding the folder a command is about, and reporting on its policy.
//!
//! `explain` and `plan` both have to answer the same three questions before
//! they can do anything: which folder is this, what governs it, and how do we
//! draw the complaint if that policy will not load. They answered them with
//! one copy of the code between them, so it lives here rather than in
//! whichever command happened to need it first.
//!
//! The only place in the workspace that draws a policy diagnostic.
//! `tungstate-core` reports a byte span as data; `miette` turns that into
//! *line 14: ...* with the line underneath, which is what DESIGN.md §7 asks
//! for and what makes a broken policy fixable without reading the parser.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use miette::{Diagnostic, LabeledSpan, NamedSource, Severity};
use tungstate_core::{Loaded, Policy, PolicyError, Warning};

/// Where a policy file is expected, relative to the folder it governs.
pub(crate) const POLICY_RELATIVE: &str = ".tungstate/policy.toml";

/// A policy problem, drawn against its source.
#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("{message}")]
pub(crate) struct PolicyDiagnostic {
    message: String,
    // Not called `source`: thiserror claims a field of that name as the
    // error's cause, and this one is the text the span points into.
    #[source_code]
    text: NamedSource<String>,
    #[label(collection)]
    labels: Vec<LabeledSpan>,
    #[diagnostic(severity)]
    severity: Severity,
    #[help]
    help: Option<String>,
}

impl PolicyDiagnostic {
    fn error(name: &str, text: &str, error: &PolicyError) -> Self {
        let mut labels = Vec::new();
        if let Some(span) = error.span() {
            labels.push(LabeledSpan::new_primary_with_span(
                Some("here".to_string()),
                span,
            ));
        }
        if let Some((label, span)) = error.related() {
            labels.push(LabeledSpan::new_with_span(Some(label.to_string()), span));
        }
        Self {
            message: error.to_string(),
            text: NamedSource::new(name, text.to_string()),
            labels,
            severity: Severity::Error,
            help: None,
        }
    }

    fn warning(name: &str, text: &str, warning: &Warning) -> Self {
        let labels = match warning {
            Warning::Shadowed { span, by_span, .. } => vec![
                LabeledSpan::new_primary_with_span(
                    Some("can never fire".to_string()),
                    span.clone(),
                ),
                LabeledSpan::new_with_span(
                    Some("takes everything it would match".to_string()),
                    by_span.clone(),
                ),
            ],
        };
        Self {
            message: warning.to_string(),
            text: NamedSource::new(name, text.to_string()),
            labels,
            severity: Severity::Warning,
            help: Some("move it above the rule that shadows it, or delete it".to_string()),
        }
    }
}

/// Everything `explain` had to find before it could ask the classifier.
pub(crate) struct Located {
    /// The folder the policy governs. Every path in a trace is relative to it.
    pub(crate) root: PathBuf,
    /// The policy file.
    pub(crate) policy: PathBuf,
    /// The policy file as a trace names it: relative to the root when it is
    /// inside it, so output does not depend on where the folder lives.
    pub(crate) policy_name: String,
}

/// Find the policy for `start`, walking up to the nearest
/// `.tungstate/policy.toml`, unless one was named on the command line.
///
/// `start` is the file being explained, or the working directory for
/// `validate`. With `--policy`, the walk still decides what the root is; if
/// nothing is found, the file's own directory stands in, so `explain` works
/// before any folder has been added.
pub(crate) fn locate(start: &Path, explicit: Option<&Path>) -> Result<Located, String> {
    let start = start
        .canonicalize()
        .map_err(|e| format!("cannot read `{}`: {e}", start.display()))?;
    let first_dir = if start.is_dir() {
        start.clone()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("`{}` has no parent directory", start.display()))?
    };

    let found = first_dir
        .ancestors()
        .map(|dir| (dir.to_path_buf(), dir.join(POLICY_RELATIVE)))
        .find(|(_, candidate)| candidate.is_file());

    let (root, policy) = match (explicit, found) {
        (Some(explicit), found) => {
            let policy = explicit
                .canonicalize()
                .map_err(|e| format!("cannot read `{}`: {e}", explicit.display()))?;
            (found.map_or(first_dir, |(root, _)| root), policy)
        }
        (None, Some(found)) => found,
        (None, None) => {
            return Err(format!(
                "no `{POLICY_RELATIVE}` above `{}`; add one, or name a policy with --policy",
                start.display()
            ));
        }
    };
    // A policy found by walking is named relative to the folder it governs,
    // so a trace does not depend on where that folder happens to live. One
    // given with --policy is named as it was typed, which is what the reader
    // will recognise.
    let policy_name = match explicit {
        Some(explicit) => explicit.display().to_string(),
        None => policy.strip_prefix(&root).map_or_else(
            |_| policy.display().to_string(),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        ),
    };
    Ok(Located {
        root,
        policy,
        policy_name,
    })
}

/// Read and compile the policy, drawing the diagnostic if it fails.
pub(crate) fn load(located: &Located) -> Result<(String, Loaded), ExitCode> {
    let text = match std::fs::read_to_string(&located.policy) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read `{}`: {error}", located.policy.display());
            return Err(ExitCode::FAILURE);
        }
    };
    compile(&located.policy_name, text)
}

/// A folder with rules, on this machine or on a connection, opened and ready
/// to plan, apply or undo against.
pub(crate) struct Governed {
    /// The folder itself.
    pub(crate) backend: Box<dyn tungstate_backend::Backend>,
    /// What the journal records against.
    pub(crate) end: tungstate_journal::Endpoint,
    /// The folder as output names it: a path, or `nas:media`.
    pub(crate) shown: String,
    /// The policy as a diagnostic names it.
    pub(crate) policy_name: String,
    /// The policy's text, for warnings drawn against it.
    pub(crate) text: String,
    /// The policy, compiled.
    pub(crate) loaded: Loaded,
}

/// Open the folder `target` names, with its rules.
///
/// On this machine, `target` may be anywhere inside the folder: the nearest
/// `.tungstate/policy.toml` above it decides, as it always has. On a
/// connection (`nas:media`) it is the folder itself, because walking up a
/// remote tree to look for rules is a round trip per level.
pub(crate) fn governed(
    target: &str,
    explicit: Option<&Path>,
    journal: &tungstate_journal::Journal,
) -> Result<Governed, ExitCode> {
    if let Ok(end) = tungstate_journal::ends::parse_end(target, None, journal)
        && end.connection.is_some()
    {
        return remote(target, end, explicit, journal);
    }
    let located = locate(Path::new(target), explicit).map_err(|message| {
        eprintln!("error: {message}");
        ExitCode::FAILURE
    })?;
    let (text, loaded) = load(&located)?;
    Ok(Governed {
        backend: Box::new(tungstate_backend::local::LocalBackend::new(
            located.root.clone(),
        )),
        end: tungstate_journal::Endpoint::local(located.root.clone()),
        shown: located.root.display().to_string(),
        policy_name: located.policy_name,
        text,
        loaded,
    })
}

fn remote(
    target: &str,
    end: tungstate_journal::Endpoint,
    explicit: Option<&Path>,
    journal: &tungstate_journal::Journal,
) -> Result<Governed, ExitCode> {
    use std::io::Read as _;

    let backend = tungstate_backend_opendal::open(&end, journal, crate::secret_store().as_ref())
        .map_err(|error| crate::fail(&error))?;
    let (policy_name, text) = if let Some(explicit) = explicit {
        let text = std::fs::read_to_string(explicit).map_err(|error| {
            eprintln!("error: cannot read `{}`: {error}", explicit.display());
            ExitCode::FAILURE
        })?;
        (explicit.display().to_string(), text)
    } else {
        let mut text = String::new();
        backend
            .open_read(Path::new(POLICY_RELATIVE))
            .and_then(|mut reader| {
                reader.read_to_string(&mut text).map_err(|source| {
                    tungstate_backend::BackendError::Io {
                        path: POLICY_RELATIVE.into(),
                        source,
                    }
                })
            })
            .map_err(|_| {
                eprintln!(
                    "error: `{target}` has no `{POLICY_RELATIVE}`; give it rules with \
                     `tungstate init {target}` or `tungstate folder learn {target}`"
                );
                ExitCode::FAILURE
            })?;
        (format!("{target}/{POLICY_RELATIVE}"), text)
    };
    let (text, loaded) = compile(&policy_name, text)?;
    Ok(Governed {
        backend,
        end,
        shown: target.to_string(),
        policy_name,
        text,
        loaded,
    })
}

/// Compile a policy, drawing the diagnostic if it will not.
fn compile(name: &str, text: String) -> Result<(String, Loaded), ExitCode> {
    match Policy::parse(&text) {
        Ok(loaded) => Ok((text, loaded)),
        Err(error) => {
            let report = miette::Report::new(PolicyDiagnostic::error(name, &text, &error));
            eprintln!("{report:?}");
            Err(ExitCode::FAILURE)
        }
    }
}

pub(crate) fn print_warnings(policy_name: &str, text: &str, warnings: &[Warning]) {
    for warning in warnings {
        let report = miette::Report::new(PolicyDiagnostic::warning(policy_name, text, warning));
        eprintln!("{report:?}");
    }
}

pub(crate) fn describe_tier(tier: tungstate_core::Tier) -> String {
    use tungstate_core::Tier;
    match tier {
        Tier::Stat => "reads nothing but names and sizes (tier: stat)".to_string(),
        Tier::Head => "reads the first 8 KiB of each file (tier: head)".to_string(),
        Tier::Meta => "reads the first 64 KiB of each file (tier: meta)".to_string(),
        Tier::Whole => "reads every file in full (tier: whole)".to_string(),
    }
}
