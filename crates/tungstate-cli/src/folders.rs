//! `tungstate folder` and `tungstate init`.
//!
//! The command line does not strictly need a list of governed folders — `plan`
//! finds its own by walking up from where it was typed. It has one because the
//! window needs one, and the command line is the reference surface: it must not
//! be able to do less than the window, or the two disagree about what a folder
//! is.

use std::path::Path;
use std::process::ExitCode;

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
