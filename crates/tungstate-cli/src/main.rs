//! The `tungstate` command line interface.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tungstate", version = tungstate_api::VERSION, about = "Keep folders in the shape you declared.")]
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
    }

    eprintln!("not implemented yet");
    std::process::ExitCode::from(2)
}
