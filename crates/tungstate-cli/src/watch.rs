//! `tungstate watch`: keep governed folders in order while this runs.
//!
//! The loop is `tungstate_watch`. This is the conversation around it: which
//! folders, how often to sweep, and what to print when something happens.
//!
//! There is no daemon until slice 10, so this holds the terminal. That is said
//! on the first line rather than left to be discovered.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use tungstate_backend::Backend as _;
use tungstate_backend::local::LocalBackend;
use tungstate_watch::{Noticed, Stop, Watched};

/// What was asked for on the command line.
#[derive(Debug, Clone, Default)]
pub struct Asked {
    /// Only these folders, rather than every governed one.
    pub only: Vec<String>,
    /// How often to look at everything regardless.
    pub sweep: Option<Duration>,
    /// Look once and stop, which is what a `cron` line wants.
    pub once: bool,
}

/// How often to look at everything regardless of what any watcher said.
///
/// Watchers drop events, so this is the number that decides how long a missed
/// one can leave a folder untidy.
const SWEEP_EVERY: Duration = Duration::from_secs(3600);

/// `tungstate watch [PATH...] [--sweep DUR] [--once]`, from the parsed flags.
pub fn run(paths: Vec<String>, sweep: Option<&str>, once: bool) -> ExitCode {
    // The same parser the policy's own durations use, so `5m` means one thing
    // across the whole product.
    let every = match sweep.map(tungstate_core::grammar::parse_duration) {
        None => None,
        Some(Ok(every)) => Some(every),
        Some(Err(why)) => {
            eprintln!("error: {why}");
            return ExitCode::FAILURE;
        }
    };
    watch(&Asked {
        only: paths,
        sweep: every,
        once,
    })
}

/// `tungstate watch`, from what was asked for.
pub fn watch(asked: &Asked) -> ExitCode {
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };
    let known = match journal.folders() {
        Ok(folders) => folders,
        Err(error) => return crate::fail(&error),
    };

    let wanted: Vec<PathBuf> = asked
        .only
        .iter()
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path)))
        .collect();
    let folders: Vec<Watched> = known
        .into_iter()
        .filter(|folder| {
            wanted.is_empty()
                || wanted.iter().any(|path| {
                    std::fs::canonicalize(&folder.root)
                        .unwrap_or_else(|_| PathBuf::from(&folder.root))
                        == *path
                })
        })
        .map(|folder| {
            let root = PathBuf::from(&folder.root);
            Watched {
                name: folder.name,
                // A share reports nothing about a change another machine made,
                // so it is swept rather than watched, and the first line says
                // how many are in each camp.
                networked: networked(&root),
                root,
            }
        })
        .collect();

    if folders.is_empty() {
        if asked.only.is_empty() {
            eprintln!("error: no folders are being governed");
            eprintln!("add one with `tungstate folder add <PATH>`");
        } else {
            eprintln!("error: none of those folders are being governed");
        }
        return ExitCode::FAILURE;
    }

    if asked.once {
        tungstate_watch::sweep(&folders, &journal, &mut |noticed| say(noticed));
        return ExitCode::SUCCESS;
    }

    let stop = Stop::new();
    match tungstate_watch::watch(
        &folders,
        &journal,
        asked.sweep.unwrap_or(SWEEP_EVERY),
        &stop,
        &mut |noticed| say(noticed),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Whether this folder's bytes are a network away.
fn networked(root: &Path) -> bool {
    LocalBackend::new(root.to_path_buf())
        .capabilities()
        .networked
}

/// One thing that happened, as a line.
fn say(noticed: &Noticed) {
    match noticed {
        Noticed::Started { watching, sweeping } => {
            println!("watching {watching} folder(s), checking {sweeping} on the hour");
            if *sweeping > 0 {
                println!(
                    "  the ones on a share are checked rather than watched: a share says nothing about a change another machine made"
                );
            }
            println!("  only folders set to enforce are tidied on their own");
            println!("  this runs until you stop it; there is no background service yet");
        }
        Noticed::Tidied {
            folder,
            files,
            plan,
        } => println!("{folder}: filed {files} file(s). Undo with `tungstate undo --plan {plan}`"),
        Noticed::Waiting { folder, files } => println!(
            "{folder}: {files} file(s) have arrived. `tungstate apply {folder}` files them"
        ),
        Noticed::Settled { folder } => println!("{folder}: nothing to do"),
        Noticed::Trouble { folder, why } => eprintln!("{folder}: {why}"),
    }
}
