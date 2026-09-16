//! `tungstate apply` and `tungstate undo`.
//!
//! The circuit breaker lives here rather than in `tungstate-execute`, because
//! whether a blast radius should stop a run is a question for whoever is
//! talking to the user. The engine does what it is told; `--yes` is a
//! conversation.

use std::path::Path;
use std::process::ExitCode;

use tungstate_backend::local::LocalBackend;
use tungstate_core::plan::{Blast, Plan};
use tungstate_execute::{Applied, Resolution, resolve_interrupted};
use tungstate_journal::Journal;
use tungstate_journal::plans::PlanId;

use crate::folder::{load, locate};

/// `tungstate apply [PATH] [--yes] [--plan FILE]`.
pub fn apply(target: Option<&str>, saved: Option<&Path>, yes: bool) -> ExitCode {
    let target = target.unwrap_or(".");
    let located = match locate(Path::new(target), None) {
        Ok(located) => located,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok((_, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };

    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };
    let backend = LocalBackend::new(located.root.clone());
    let root = located.root.to_string_lossy().to_string();

    if let Err(code) = settle_the_books(&backend, &journal, &root) {
        return code;
    }

    let fresh = match tungstate_attrs::survey(&backend, &loaded.policy) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };

    // A saved plan is read back; otherwise one is made from the folder as it
    // is. Either way the executor checks the fingerprint, so the saved route
    // is refused the moment the folder has moved on.
    let plan = match saved {
        Some(path) => match read_plan(path) {
            Ok(plan) => plan,
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::FAILURE;
            }
        },
        None => loaded.policy.plan(&fresh),
    };

    if plan.ops.is_empty() {
        // "Tidy" and "not tidy yet" are the same empty list of operations and
        // very different sentences. Saying the first when the second is true
        // is how somebody concludes the thing does not work.
        if plan.waiting() > 0 {
            println!(
                "nothing to do yet: {} file(s) in `{}` were written too recently to touch.",
                plan.waiting(),
                plan.folder
            );
            println!(
                "The longest has {}s to wait. Run this again after that.",
                plan.longest_wait()
            );
        } else {
            println!(
                "nothing to do: `{}` already matches its policy",
                plan.folder
            );
        }
        return ExitCode::SUCCESS;
    }

    if let Some(refusal) = breaker(&plan, yes) {
        eprint!("{refusal}");
        return ExitCode::FAILURE;
    }

    let applied = match tungstate_execute::apply(&plan, &fresh, &backend, &journal, &root) {
        Ok(applied) => applied,
        Err(error) => return stopped(&error),
    };
    report(&applied, &plan);
    if applied.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Why this run is refused, if it is.
///
/// Both refusals are `--yes`-able, because both are judgements about someone
/// else's files: the job is to be sure they meant it, not to know better.
fn breaker(plan: &Plan, yes: bool) -> Option<String> {
    if yes {
        return None;
    }
    if !plan.settles {
        return Some(format!(
            "refusing: this policy does not settle, so applying it would leave {} file(s) \
             still wanting to move.\nA `rename` that reads {{name}} and adds to it renders \
             differently once the file is renamed.\nFix the rule, or pass --yes if you are sure.\n",
            plan.unsettled.len()
        ));
    }
    if plan.blast.over_limit {
        let Blast { files, of, .. } = plan.blast;
        return Some(format!(
            "refusing: this would move {files} of {of} file(s), past the {}-file / {}% limit.\n\
             Run `tungstate plan` and read it, then pass --yes if that is what you meant.\n",
            Blast::LIMIT_FILES,
            100 / Blast::LIMIT_SHARE,
        ));
    }
    None
}

/// Close the books on anything left in flight by a process that died.
fn settle_the_books(backend: &LocalBackend, journal: &Journal, root: &str) -> Result<(), ExitCode> {
    match resolve_interrupted(backend, journal, root) {
        Ok(resolved) if resolved.is_empty() => Ok(()),
        Ok(resolved) => {
            println!(
                "{} operation(s) were interrupted by an earlier run:",
                resolved.len()
            );
            for (path, resolution) in &resolved {
                let said = match resolution {
                    Resolution::Committed => "had already happened",
                    Resolution::Abandoned => "had not started",
                    Resolution::Unclear => "cannot be told either way, and is recorded as failed",
                };
                println!("  {path}  ({said})");
            }
            println!("Nothing was resumed; this run plans the folder as it is now.\n");
            Ok(())
        }
        Err(error) => Err(crate::fail(&error)),
    }
}

fn read_plan(path: &Path) -> Result<Plan, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| {
        format!(
            "`{}` is not a plan this build understands: {e}",
            path.display()
        )
    })
}

fn report(applied: &Applied, plan: &Plan) {
    println!(
        "applied plan {} to \"{}\": {} operation(s)",
        applied.plan.0, plan.folder, applied.done
    );
    for skipped in &applied.skipped {
        println!("  skipped  {}  ({})", skipped.path, skipped.why);
    }
    for failed in &applied.failed {
        println!("  failed   {}  ({})", failed.path, failed.why);
    }
    if applied.complete() {
        println!(
            "\nTake it back with `tungstate undo --plan {}`.",
            applied.plan.0
        );
    } else {
        println!(
            "\n{} skipped, {} failed. Run `tungstate plan` to see what is left.",
            applied.skipped.len(),
            applied.failed.len()
        );
    }
}

/// A run that did not finish, said so that the next move is obvious.
///
/// The two cases need different sentences: one leaves the folder untouched,
/// and the other leaves half a reorganisation that can be taken back.
fn stopped(error: &tungstate_execute::ExecuteError) -> ExitCode {
    use tungstate_execute::ExecuteError;
    eprintln!("error: {error}");
    match error {
        ExecuteError::StoppedPartWay { plan, done, .. } if *done > 0 => eprintln!(
            "Those {done} operation(s) are recorded. Take them back with \
             `tungstate undo --plan {}`, or run `tungstate plan` to carry on from here.",
            plan.0
        ),
        ExecuteError::StoppedPartWay { .. } => {
            eprintln!("Nothing was changed. `tungstate plan` describes the folder as it is now.");
        }
        _ => eprintln!("Nothing was changed. Run `tungstate plan` to see the folder as it is now."),
    }
    ExitCode::FAILURE
}

/// `tungstate undo [--last N | --plan ID] [PATH]`.
pub fn undo(target: Option<&str>, last: Option<usize>, plan: Option<i64>) -> ExitCode {
    let target = target.unwrap_or(".");
    let located = match locate(Path::new(target), None) {
        Ok(located) => located,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok((_, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };
    let backend = LocalBackend::new(located.root.clone());
    let root = located.root.to_string_lossy().to_string();

    if let Err(code) = settle_the_books(&backend, &journal, &root) {
        return code;
    }

    let wanted = match choose(&journal, &root, last, plan) {
        Ok(wanted) => wanted,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    if wanted.is_empty() {
        println!("nothing to undo for `{root}`");
        return ExitCode::SUCCESS;
    }

    for id in wanted {
        let fresh = match tungstate_attrs::survey(&backend, &loaded.policy) {
            Ok(snapshot) => snapshot,
            Err(error) => return crate::fail(&error),
        };
        match tungstate_execute::undo(id, &fresh, &backend, &journal, &root) {
            Ok(undone) => println!("undid plan {}: {} operation(s)", id.0, undone.done),
            Err(error) => {
                eprintln!("error: plan {} could not be undone: {error}", id.0);
                eprintln!("Nothing was changed by this undo.");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// Which reorganisations to reverse, newest first.
fn choose(
    journal: &Journal,
    root: &str,
    last: Option<usize>,
    plan: Option<i64>,
) -> Result<Vec<PlanId>, String> {
    match (last, plan) {
        (Some(_), Some(_)) => Err("--last and --plan ask for different things".to_string()),
        (None, Some(id)) => Ok(vec![PlanId(id)]),
        (last, None) => {
            let want = last.unwrap_or(1);
            let recent = journal
                .recent_plans(want.saturating_mul(4).max(16))
                .map_err(|e| e.to_string())?;
            // This folder's, and still standing. `is_undoable` also skips
            // plans that are themselves undos -- without that, "undo the last
            // thing" undoes the undo and quietly redoes the work.
            Ok(recent
                .into_iter()
                .filter(|p| p.folder == root && p.is_undoable())
                .take(want)
                .map(|p| p.id)
                .collect())
        }
    }
}
