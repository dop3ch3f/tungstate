//! `tungstate folder` and `tungstate init`.
//!
//! The command line does not strictly need a list of governed folders — `plan`
//! finds its own by walking up from where it was typed. It has one because the
//! window needs one, and the command line is the reference surface: it must not
//! be able to do less than the window, or the two disagree about what a folder
//! is.

use std::path::Path;
use std::process::ExitCode;

use tungstate_core::learn::Level;
use tungstate_core::templates::{TEMPLATES, template};

/// Where a policy lives, relative to the folder it governs.
const POLICY_RELATIVE: &str = ".tungstate/policy.toml";

/// `tungstate folder add <PATH> [--name NAME]`.
pub fn add(path: &str, name: Option<&str>) -> ExitCode {
    let Ok(root) = Path::new(path).canonicalize() else {
        eprintln!("error: cannot read `{path}`");
        return ExitCode::FAILURE;
    };
    if !root.is_dir() {
        eprintln!("error: `{path}` is not a directory");
        return ExitCode::FAILURE;
    }
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };

    let display = root.to_string_lossy().to_string();
    let label = name.map_or_else(
        || {
            root.file_name()
                .map_or_else(|| display.clone(), |n| n.to_string_lossy().to_string())
        },
        ToString::to_string,
    );
    if let Err(error) = journal.add_folder(&display, &label) {
        return crate::fail(&error);
    }

    println!("governing `{display}` as \"{label}\"");
    if root.join(POLICY_RELATIVE).is_file() {
        println!("it already has rules; `tungstate plan {path}` says what they would do");
    } else {
        // Named rather than left to be discovered: a governed folder with no
        // rules does nothing at all, and the reason is not guessable.
        println!("it has no rules yet, so nothing will happen to it.");
        println!("give it some with `tungstate init --template <NAME> {path}`:");
        for entry in TEMPLATES {
            println!("  {:<12} {}", entry.name, entry.summary);
        }
    }
    ExitCode::SUCCESS
}

/// `tungstate folder list`.
pub fn list() -> ExitCode {
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };
    match journal.folders() {
        Ok(folders) if folders.is_empty() => {
            println!("no folders are being governed");
            println!("add one with `tungstate folder add <PATH>`");
            ExitCode::SUCCESS
        }
        Ok(folders) => {
            for folder in &folders {
                let rules = if Path::new(&folder.root).join(POLICY_RELATIVE).is_file() {
                    ""
                } else {
                    "  (no rules yet)"
                };
                println!("{:<20} {}{rules}", folder.name, folder.root);
            }
            ExitCode::SUCCESS
        }
        Err(error) => crate::fail(&error),
    }
}

/// `tungstate folder remove <PATH>`.
pub fn remove(path: &str) -> ExitCode {
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };
    // Canonicalised if it can be, so `.` and a trailing slash find the row --
    // but a folder that has since been deleted must still be forgettable, so a
    // path that cannot be resolved is tried as it was typed.
    let display = Path::new(path)
        .canonicalize()
        .map_or_else(|_| path.to_string(), |p| p.to_string_lossy().to_string());
    match journal.remove_folder(&display) {
        Ok(()) => {
            println!("no longer governing `{display}`");
            println!("its rules and its history are untouched");
            ExitCode::SUCCESS
        }
        Err(error) => crate::fail(&error),
    }
}

/// `tungstate init [--template NAME] [PATH]`.
pub fn init(path: Option<&str>, name: Option<&str>) -> ExitCode {
    let path = path.unwrap_or(".");
    let Ok(root) = Path::new(path).canonicalize() else {
        eprintln!("error: cannot read `{path}`");
        return ExitCode::FAILURE;
    };

    let Some(name) = name else {
        println!("give a folder some rules by choosing a starting layout:");
        for entry in TEMPLATES {
            println!("\n  {}  — {}", entry.name, entry.summary);
            println!("      {}", entry.detail);
        }
        println!("\nthen: tungstate init --template <NAME> {path}");
        return ExitCode::SUCCESS;
    };

    let Some(chosen) = template(name) else {
        eprintln!(
            "error: no starting layout called `{name}`. There is: {}",
            tungstate_core::templates::names().join(", ")
        );
        return ExitCode::FAILURE;
    };

    let policy = root.join(POLICY_RELATIVE);
    if policy.exists() {
        // Never overwritten. A policy is somebody's work, and this command
        // exists to get them started rather than to start them over.
        eprintln!("error: `{}` already has rules", root.display());
        eprintln!("edit `{}`, or move it aside first", policy.display());
        return ExitCode::FAILURE;
    }
    if let Some(parent) = policy.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        eprintln!("error: cannot make `{}`: {error}", parent.display());
        return ExitCode::FAILURE;
    }
    if let Err(error) = std::fs::write(&policy, chosen.body) {
        eprintln!("error: cannot write `{}`: {error}", policy.display());
        return ExitCode::FAILURE;
    }

    println!("wrote {}", policy.display());
    println!("{}", chosen.detail);
    println!("\nnothing has moved. See what it would do:\n  tungstate plan {path}");
    ExitCode::SUCCESS
}

/// `tungstate folder learn <PATH> [--write] [--improved]`.
///
/// Reads the shape a folder already has and writes it down as rules. The
/// inverse of everything else here, and the reason it exists: somebody who has
/// already organised a folder by hand should not have to describe it again in
/// a language they have just met.
pub fn learn(path: &str, write: bool, improved: bool) -> ExitCode {
    let Ok(root) = Path::new(path).canonicalize() else {
        eprintln!("error: cannot read `{path}`");
        return ExitCode::FAILURE;
    };

    // `PROBE` is a constant in this workspace with a test that it parses, so a
    // failure here is our bug and not something the user can act on.
    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    let backend = tungstate_backend::local::LocalBackend::new(root.clone());
    let snapshot = match tungstate_attrs::survey(&backend, &probe) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };

    let name = root
        .file_name()
        .map_or_else(|| "folder".to_string(), |n| n.to_string_lossy().to_string());
    let learned = tungstate_core::learn::learn(&snapshot, &name);

    println!("folder \"{name}\" at {}", root.display());
    if learned.found_a_shape() {
        let words: Vec<&str> = learned.levels.iter().map(Level::word).collect();
        println!(
            "  {} of {} file(s) sit in a shape {} level(s) deep",
            learned.explains,
            learned.of,
            learned.levels.len()
        );
        println!("  shape: {}", words.join(" / "));
    } else {
        println!(
            "  no shape found: {} file(s), none in a directory",
            learned.of
        );
    }
    if learned.loose > 0 {
        println!("  {} file(s) sit loose at the top", learned.loose);
    }

    if !learned.suggestions.is_empty() {
        println!("\nwhat would be better:");
        for s in &learned.suggestions {
            println!("  - {}", s.headline);
            println!("    {}", s.why);
        }
    }

    // Both are printed, because choosing between them is the point: the rules
    // that keep what you have, and the same rules improved.
    let chosen = if improved {
        let Some(text) = learned.improved.as_deref() else {
            eprintln!("\nerror: there is nothing to improve here");
            return ExitCode::FAILURE;
        };
        text
    } else {
        &learned.as_is
    };

    if !write {
        println!(
            "\n--- rules that keep this shape {}---",
            if improved { "(improved) " } else { "" }
        );
        print!("{chosen}");
        if learned.improved.is_some() && !improved {
            println!("\n(`--improved` shows the same rules with the suggestions applied)");
        }
        println!("\nnothing has been written. `--write` saves this as the folder's rules.");
        return ExitCode::SUCCESS;
    }

    let policy = root.join(POLICY_RELATIVE);
    if policy.exists() {
        // Same refusal `init` makes, for the same reason: a policy is
        // somebody's work, and this command starts them off rather than over.
        eprintln!("error: `{}` already has rules", root.display());
        eprintln!("edit `{}`, or move it aside first", policy.display());
        return ExitCode::FAILURE;
    }
    if let Some(parent) = policy.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return crate::fail(&error);
    }
    if let Err(error) = std::fs::write(&policy, chosen) {
        return crate::fail(&error);
    }
    println!("\nwrote {}", policy.display());
    println!("nothing has moved. `tungstate plan {path}` says what these rules would do.");
    ExitCode::SUCCESS
}

/// `tungstate folder compare <PATH>`.
///
/// What each starting layout would do to this folder, side by side, including
/// the rules it already has. Changes nothing.
///
/// The comparing itself lives in `tungstate_core::compare` so the window asks
/// the same question and gets the same answer, rather than each surface
/// computing its own and drifting.
pub fn compare(path: &str) -> ExitCode {
    let Ok(root) = Path::new(path).canonicalize() else {
        eprintln!("error: cannot read `{path}`");
        return ExitCode::FAILURE;
    };

    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    let backend = tungstate_backend::local::LocalBackend::new(root.clone());
    let snapshot = match tungstate_attrs::survey(&backend, &probe) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };

    // The folder's own rules first, so every other row reads as a change from
    // where it actually is rather than from nothing.
    let mut also = Vec::new();
    let existing = root.join(POLICY_RELATIVE);
    if existing.is_file() {
        match std::fs::read_to_string(&existing) {
            Ok(text) => also.push(("(the rules you have)".to_string(), text)),
            Err(error) => eprintln!("warning: cannot read {}: {error}", existing.display()),
        }
    }

    let outcomes = tungstate_core::compare::against(&snapshot, &also);
    let files = outcomes.first().map_or(0, |o| o.of);
    println!("folder at {}", root.display());
    println!("  {files} file(s), walked once and shown against each layout\n");

    for outcome in &outcomes {
        let name = &outcome.name;
        if !outcome.loads {
            println!("  {name:<22} these rules will not load");
            continue;
        }
        if !outcome.settles {
            println!("  {name:<22} never settles — it would keep moving the same files");
            continue;
        }
        println!(
            "  {name:<22} {} of {files} would move, {} dir(s) made, {} removed",
            outcome.files, outcome.created, outcome.removed
        );
        match &outcome.example {
            Some(m) => println!("  {:<22}   e.g. {} → {}", "", m.from, m.to),
            None => println!("  {:<22}   nothing would change", ""),
        }
    }
    println!("\nnothing has moved. `tungstate init --template <NAME> {path}` picks one.");
    ExitCode::SUCCESS
}
