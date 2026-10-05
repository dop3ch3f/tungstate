//! `tungstate folder` and `tungstate init`.
//!
//! The command line does not strictly need a list of governed folders — `plan`
//! finds its own by walking up from where it was typed. It has one because the
//! window needs one, and the command line is the reference surface: it must not
//! be able to do less than the window, or the two disagree about what a folder
//! is.

use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

use tungstate_backend::Backend;
use tungstate_core::learn::Level;
use tungstate_core::templates::{TEMPLATES, template};
use tungstate_journal::{Endpoint, Journal};

/// Where a policy lives, relative to the folder it governs.
const POLICY_RELATIVE: &str = ".tungstate/policy.toml";

/// A folder named on the command line: a path here, or `connection:folder`.
struct Place {
    backend: Box<dyn Backend>,
    end: Endpoint,
    /// As output names it.
    shown: String,
    /// Its last part, for a name.
    label: String,
}

/// Open the folder `path` names. A path on this machine has to exist and be
/// a directory; one on a connection has to be reachable.
fn place(path: &str, journal: &Journal) -> Result<Place, ExitCode> {
    if let Ok(end) = tungstate_journal::ends::parse_end(path, None, journal)
        && end.connection.is_some()
    {
        let backend =
            tungstate_backend_opendal::open(&end, journal, crate::secret_store().as_ref())
                .map_err(|error| crate::fail(&error))?;
        if !backend.stat(Path::new("")).is_ok_and(|meta| meta.is_dir) {
            eprintln!("error: `{path}` is not a folder on that connection");
            return Err(ExitCode::FAILURE);
        }
        return Ok(Place {
            backend,
            end,
            shown: path.to_string(),
            label: label_of(path),
        });
    }
    let Ok(root) = Path::new(path).canonicalize() else {
        eprintln!("error: cannot read `{path}`");
        return Err(ExitCode::FAILURE);
    };
    if !root.is_dir() {
        eprintln!("error: `{path}` is not a directory");
        return Err(ExitCode::FAILURE);
    }
    let shown = root.to_string_lossy().to_string();
    Ok(Place {
        backend: Box::new(tungstate_backend::local::LocalBackend::new(root.clone())),
        end: Endpoint::local(root),
        label: label_of(&shown),
        shown,
    })
}

/// A folder's last part, or a connection's own name for its root.
fn label_of(target: &str) -> String {
    let last = target
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(target);
    match last.split_once(':') {
        Some((name, rest)) if name.len() > 1 => {
            if rest.trim().is_empty() { name } else { rest }.to_string()
        }
        _ => last.to_string(),
    }
}

impl Place {
    fn has_rules(&self) -> bool {
        self.backend
            .stat(Path::new(POLICY_RELATIVE))
            .is_ok_and(|meta| !meta.is_dir)
    }

    fn read_rules(&self) -> Result<String, tungstate_backend::BackendError> {
        let mut text = String::new();
        self.backend
            .open_read(Path::new(POLICY_RELATIVE))?
            .read_to_string(&mut text)
            .map_err(|source| tungstate_backend::BackendError::Io {
                path: POLICY_RELATIVE.into(),
                source,
            })?;
        Ok(text)
    }

    /// Write rules into the folder, wherever it is. Never over existing ones:
    /// a policy is somebody's work.
    fn write_rules(&self, text: &str) -> Result<(), ExitCode> {
        if self.has_rules() {
            eprintln!("error: `{}` already has rules", self.shown);
            eprintln!("edit its `{POLICY_RELATIVE}`, or move it aside first");
            return Err(ExitCode::FAILURE);
        }
        let policy = Path::new(POLICY_RELATIVE);
        let written = policy
            .parent()
            .map_or(Ok(()), |parent| self.backend.create_dir_all(parent))
            .and_then(|()| self.backend.create_write(policy))
            .and_then(|mut sink| {
                sink.write_all(text.as_bytes()).map_err(|source| {
                    tungstate_backend::BackendError::Io {
                        path: policy.into(),
                        source,
                    }
                })?;
                sink.finish()
            });
        written.map_err(|error| crate::fail(&error))
    }
}

fn journal() -> Result<Journal, ExitCode> {
    crate::open_journal().map_err(|error| crate::fail(&error))
}

/// `tungstate folder add <PATH> [--name NAME]`, where PATH may be
/// `connection:folder`.
pub fn add(path: &str, name: Option<&str>) -> ExitCode {
    let Ok(journal) = journal() else {
        return ExitCode::FAILURE;
    };
    let Ok(here) = place(path, &journal) else {
        return ExitCode::FAILURE;
    };
    let label = name.map_or_else(|| here.label.clone(), ToString::to_string);
    if let Err(error) = journal.add_folder_at(&here.end, &label) {
        return crate::fail(&error);
    }

    println!("governing `{}` as \"{label}\"", here.shown);
    if here.has_rules() {
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
                // A folder on a connection is listed without dialling it.
                let (shown, rules) = match folder.connection {
                    Some(id) => {
                        let connection = journal
                            .connection_by_id(id)
                            .map_or_else(|_| format!("connection {}", id.0), |c| c.name);
                        (format!("{connection}:{}", folder.root), "")
                    }
                    None if Path::new(&folder.root).join(POLICY_RELATIVE).is_file() => {
                        (folder.root.clone(), "")
                    }
                    None => (folder.root.clone(), "  (no rules yet)"),
                };
                println!("{:<20} {shown}{rules}", folder.name);
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
    // On a connection the folder is forgotten without dialling it: one that
    // has gone, or a NAS that is off, must still be forgettable.
    if let Ok(end) = tungstate_journal::ends::parse_end(path, None, &journal)
        && end.connection.is_some()
    {
        return match journal.remove_folder_at(&end) {
            Ok(()) => {
                println!("no longer governing `{path}`");
                println!("its rules and its history are untouched");
                ExitCode::SUCCESS
            }
            Err(error) => crate::fail(&error),
        };
    }
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

    let Ok(journal) = journal() else {
        return ExitCode::FAILURE;
    };
    let Ok(here) = place(path, &journal) else {
        return ExitCode::FAILURE;
    };
    // Never overwritten. A policy is somebody's work, and this command exists
    // to get them started rather than to start them over.
    if here.write_rules(chosen.body).is_err() {
        return ExitCode::FAILURE;
    }

    println!("wrote {}/{POLICY_RELATIVE}", here.shown);
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
    let Ok(journal) = journal() else {
        return ExitCode::FAILURE;
    };
    let Ok(here) = place(path, &journal) else {
        return ExitCode::FAILURE;
    };

    // `PROBE` is a constant in this workspace with a test that it parses, so a
    // failure here is our bug and not something the user can act on.
    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    let snapshot = match tungstate_attrs::survey(here.backend.as_ref(), &probe) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };

    let name = &here.label;
    let learned = tungstate_core::learn::learn(&snapshot, name);

    println!("folder \"{name}\" at {}", here.shown);
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

    // Same refusal `init` makes, for the same reason: a policy is somebody's
    // work, and this command starts them off rather than over.
    if here.write_rules(chosen).is_err() {
        return ExitCode::FAILURE;
    }
    println!("\nwrote {}/{POLICY_RELATIVE}", here.shown);
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
    let Ok(journal) = journal() else {
        return ExitCode::FAILURE;
    };
    let Ok(here) = place(path, &journal) else {
        return ExitCode::FAILURE;
    };

    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    let snapshot = match tungstate_attrs::survey(here.backend.as_ref(), &probe) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };

    // The folder's own rules first, so every other row reads as a change from
    // where it actually is rather than from nothing.
    let mut also = Vec::new();
    if here.has_rules() {
        match here.read_rules() {
            Ok(text) => also.push(("(the rules you have)".to_string(), text)),
            Err(error) => eprintln!("warning: cannot read its rules: {error}"),
        }
    }

    let outcomes = tungstate_core::compare::against(&snapshot, &also);
    let files = outcomes.first().map_or(0, |o| o.of);
    println!("folder at {}", here.shown);
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
