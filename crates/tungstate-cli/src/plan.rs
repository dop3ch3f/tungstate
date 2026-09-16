//! `tungstate plan`: everything that would have to happen to a folder, and
//! nothing that does.
//!
//! The command is deliberately side-effect-free, down to not writing the plan
//! it prints. `tungstate plan --json > plan.json` is the save, and slice 7's
//! `apply` owns the reading of one. A read-only command that writes a file by
//! default is a contradiction, and staying provably harmless is what makes
//! "run it and see" the right first thing to tell someone to do.

use std::path::Path;
use std::process::ExitCode;

use serde::Serialize;
use tungstate_backend::local::LocalBackend;
use tungstate_core::plan::{Because, Blast, Op, Parked, Plan, Reason};
use tungstate_core::{Loaded, Mode};
use tungstate_journal::ends;

use crate::folder::{describe_tier, load, locate, print_warnings};

/// What `--json` emits: the plan, plus what the command line knew around it.
#[derive(Serialize)]
struct PlanJson<'a> {
    policy: &'a str,
    #[serde(flatten)]
    plan: &'a Plan,
}

/// `tungstate plan [PATH] [--policy FILE] [--json]`.
pub fn plan(target: Option<&str>, policy: Option<&Path>, json: bool) -> ExitCode {
    let target = target.unwrap_or(".");

    // A connection prefix is refused rather than half-supported. `survey`
    // takes a `&dyn Backend`, so the day a remote folder's policy can be
    // located this works unchanged -- but walking a remote tree looking for
    // `.tungstate/policy.toml` is a round trip per level, and governing a
    // remote folder in place is a later slice.
    if let Ok(journal) = crate::open_journal()
        && let Ok(endpoint) = ends::parse_end(target, None, &journal)
        && endpoint.connection.is_some()
    {
        eprintln!(
            "error: `{target}` is on a connection, and planning a remote folder \
             is not built yet\n  tungstate plan <a path on this machine>"
        );
        return ExitCode::from(2);
    }

    let located = match locate(Path::new(target), policy) {
        Ok(located) => located,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok((text, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };

    let backend = LocalBackend::new(located.root.clone());
    let snapshot = match tungstate_attrs::survey(&backend, &loaded.policy) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };
    let plan = loaded.policy.plan(&snapshot);

    if json {
        let document = PlanJson {
            policy: &located.policy_name,
            plan: &plan,
        };
        match serde_json::to_string_pretty(&document) {
            Ok(text) => println!("{text}"),
            Err(error) => return crate::fail(&error),
        }
    } else {
        print!(
            "{}",
            render(&plan, &located.root, &located.policy_name, &loaded)
        );
    }
    print_warnings(&located, &text, &loaded.warnings);
    ExitCode::SUCCESS
}

/// How many untouched files to name before pointing at `--json`.
///
/// A folder with ten thousand settled files should not print ten thousand
/// lines saying so; the ops are what the reader came for.
const UNTOUCHED_SHOWN: usize = 10;

fn render(plan: &Plan, root: &Path, policy_name: &str, loaded: &Loaded) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let _ = write!(
        out,
        "folder \"{}\" at {}\n  policy {policy_name}, {} rule(s), {}\n\n",
        plan.folder,
        root.display(),
        loaded.policy.rules.len(),
        describe_tier(loaded.policy.required_tier()),
    );

    if plan.ops.is_empty() {
        // "Tidy" and "not tidy yet" are the same empty list and very
        // different sentences. Saying the first when the second is true is how
        // somebody concludes the thing does not work.
        if plan.waiting() > 0 {
            let _ = writeln!(
                out,
                "  nothing to do yet \u{2014} {} file(s) were written too recently to touch.",
                plan.waiting(),
            );
            let _ = writeln!(
                out,
                "  the longest has {}s to wait; run this again after that.\n",
                plan.longest_wait(),
            );
        } else {
            out.push_str("  nothing to do\n\n");
        }
    }
    for op in &plan.ops {
        out.push_str(&render_op(op));
    }

    if !plan.untouched.is_empty() {
        out.push_str("\n  left alone\n");
        for item in plan.untouched.iter().take(UNTOUCHED_SHOWN) {
            let _ = writeln!(
                out,
                "    {:<44} {}",
                item.file,
                describe_reason(&item.reason)
            );
        }
        if plan.untouched.len() > UNTOUCHED_SHOWN {
            let _ = writeln!(
                out,
                "    and {} more — see --json",
                plan.untouched.len() - UNTOUCHED_SHOWN
            );
        }
    }

    let _ = write!(out, "\n{}", summary(plan));

    if !plan.settles {
        // The one thing worth interrupting for. A dry run that churns is
        // harmless; the same policy under `apply` moves files for ever.
        let _ = write!(
            out,
            "\nThis policy does not settle: carrying this out would leave {} file(s) \
             still wanting to move, {} among them.\nA `rename` that reads {{name}} and \
             adds to it renders differently once the file is renamed. Fix the rule \
             before applying anything.\n",
            plan.unsettled.len(),
            plan.unsettled.first().map_or("one", String::as_str),
        );
    }

    if plan.mode == Mode::Observe {
        out.push_str(
            "\nThis folder is in observe mode, so nothing would be applied automatically.\n",
        );
    }

    out.push_str("\nNothing has been changed.\n");
    out
}

fn render_op(op: &Op) -> String {
    match op {
        Op::MkDir { path } => format!("  mkdir       {path}\n"),
        Op::RmDir { path } => format!("  rmdir       {path}  (this plan empties it)\n"),
        Op::Move { from, to, because } => {
            // "rename" and "move" are one op and two words: which one a reader
            // wants depends only on whether the directory changed.
            let verb = if parent_of(from) == parent_of(to) {
                "rename"
            } else {
                "move"
            };
            format!("  {verb:<11} {from} -> {to}{}\n", describe_because(because))
        }
        Op::Quarantine { from, to, because } => {
            format!("  quarantine  {from} -> {to}{}\n", describe_parked(because))
        }
    }
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn describe_because(because: &Because) -> String {
    match because {
        Because::Rule { name } => format!("  ({name})"),
        Because::Inbox => "  (no rule matched, so the inbox)".to_string(),
        Because::Numbered { wanted } => format!("  (numbered; something else wanted {wanted})"),
        Because::MakeWay => "  (out of the way, so two files can trade places)".to_string(),
    }
}

fn describe_parked(parked: &Parked) -> String {
    match parked {
        Parked::Conflict { holder } => format!("  (a different file, {holder}, holds that name)"),
        Parked::Replaced { by } => format!("  (set aside, not deleted, so {by} can have the name)"),
    }
}

fn describe_reason(reason: &Reason) -> String {
    match reason {
        Reason::AlreadyThere => "already where it belongs".to_string(),
        Reason::Ignored { because } => because.clone(),
        Reason::Opaque { pattern } => format!("`{pattern}` is opaque, so it stays one thing"),
        Reason::Unresolvable { rule, var, reason } => {
            format!("`{rule}` needs {{{var}}}, and {reason}")
        }
        Reason::Unmatched => "no rule matched, and there is no inbox".to_string(),
        Reason::Cooling { seconds_left } => {
            format!("written too recently; {seconds_left}s of cooldown left")
        }
        Reason::Conflicted { holder } => format!("`{holder}` holds the name it wants"),
        Reason::Blocked { by, detail } => format!("{detail} (`{by}`)"),
    }
}

/// The blast radius, and whether it is past what rail 3 allows.
///
/// The limit is named rather than implied, because the moment this saves you
/// from a mistyped policy is when you read it here, not when `apply` refuses.
fn summary(plan: &Plan) -> String {
    use std::fmt::Write as _;
    let Blast {
        files,
        of,
        bytes,
        created,
        removed,
        over_limit,
    } = plan.blast;
    let share = (files * 100).checked_div(of).unwrap_or(0);
    let seen = if of == 1 {
        "1 file".to_string()
    } else {
        format!("{of} files")
    };
    let mut out = format!(
        "{files} file(s) would move ({}), {created} director{} created, \
         {removed} removed, {} left alone.\n",
        crate::human_bytes(bytes),
        if created == 1 { "y" } else { "ies" },
        plan.untouched.len(),
    );
    if over_limit {
        let _ = write!(
            out,
            "That is {share}% of {seen}, past the {}-file / {}% limit, \
             so applying it will need --yes.\nCheck the policy before you agree to this.\n",
            Blast::LIMIT_FILES,
            100 / Blast::LIMIT_SHARE,
        );
    } else if of > 0 {
        let _ = writeln!(
            out,
            "That is {share}% of {seen} — under the {}-file / {}% limit.",
            Blast::LIMIT_FILES,
            100 / Blast::LIMIT_SHARE,
        );
    }
    out
}
