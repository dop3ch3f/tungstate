//! The `tungstate` command line interface.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use tungstate_journal::{Journal, Locator, Op, OpStatus};

#[derive(Parser)]
// bin_name is pinned so the usage line reads "tungstate" everywhere. Without
// it clap takes argv[0], which is "tungstate.exe" on Windows.
#[command(
    name = "tungstate",
    bin_name = "tungstate",
    version = tungstate_api::VERSION,
    about = "Keep folders in the shape you declared."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage governed folders.
    Folder {
        #[command(subcommand)]
        action: FolderAction,
    },
    /// Manage transfer links between folders.
    Link {
        #[command(subcommand)]
        action: LinkAction,
    },
    /// Show everything that ever happened to a path.
    Log {
        /// The file or directory to report on.
        path: PathBuf,
    },
    /// Show where a file ended up.
    Whereis {
        /// A path, or a BLAKE3 hash if you have one.
        target: String,
    },
}

#[derive(Subcommand)]
enum FolderAction {
    /// Start governing a folder.
    Add {
        /// Path to the folder to govern.
        path: String,
    },
}

#[derive(Subcommand)]
enum LinkAction {
    /// Create a transfer link from a source to a destination.
    Add {
        /// Where files come from.
        from: String,
        /// Where files go.
        to: String,
    },
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();

    // Every subcommand is a stub until its slice lands. Exit code 2 keeps
    // "not built yet" distinguishable from a real runtime failure (1).
    match cli.command {
        Command::Folder { action } => match action {
            FolderAction::Add { path } => {
                tracing::debug!(%path, "folder add requested");
            }
        },
        Command::Link { action } => match action {
            LinkAction::Add { from, to } => {
                tracing::debug!(%from, %to, "link add requested");
            }
        },

        Command::Log { path } => return report(|journal| journal.history(&path)),
        Command::Whereis { target } => {
            return report(|journal| {
                // A BLAKE3 digest is 64 hex characters, and no real path looks
                // like one, so the shape of the argument decides how to read it.
                let locator = if is_hash(&target) {
                    Locator::Hash(&target)
                } else {
                    Locator::Path(Path::new(&target))
                };
                journal.whereis(&locator)
            });
        }
    }

    eprintln!("not implemented yet");
    std::process::ExitCode::from(2)
}

fn is_hash(target: &str) -> bool {
    target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit())
}

/// Open the machine journal, run a query, and print the rows.
fn report(
    query: impl FnOnce(&Journal) -> tungstate_journal::Result<Vec<Op>>,
) -> std::process::ExitCode {
    let journal = match Journal::open_default() {
        Ok(journal) => journal,
        Err(error) => return fail(&error),
    };
    match query(&journal) {
        Ok(ops) if ops.is_empty() => {
            println!("no matching operations");
            std::process::ExitCode::SUCCESS
        }
        Ok(ops) => {
            for op in &ops {
                println!("{}", render(op));
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn fail(error: &dyn std::error::Error) -> std::process::ExitCode {
    eprintln!("error: {error}");
    // The chain is where the real cause lives; printing only the top line loses it.
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    std::process::ExitCode::FAILURE
}

fn render(op: &Op) -> String {
    let status = match op.status {
        OpStatus::Intended => "interrupted",
        OpStatus::Committed => "ok",
        OpStatus::Failed => "failed",
        OpStatus::Skipped => "skipped",
    };
    let source = op.source.as_ref().map(|l| l.full().display().to_string());
    let destination = op
        .destination
        .as_ref()
        .map(|l| l.full().display().to_string());

    let movement = match (source, destination) {
        (Some(from), Some(to)) => format!("{from} -> {to}"),
        (Some(from), None) => from,
        (None, Some(to)) => to,
        (None, None) => String::from("(no location recorded)"),
    };

    let note = op
        .note
        .as_deref()
        .map(|n| format!(" ({n})"))
        .unwrap_or_default();
    format!("{:>4} {status:<11} {movement}{note}", op.id.0)
}
