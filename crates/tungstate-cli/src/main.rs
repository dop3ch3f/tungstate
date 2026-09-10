//! The `tungstate` command line interface.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use std::time::Duration;

use tungstate_backend::local::LocalBackend;
use tungstate_journal::{
    ConflictAction, Journal, Locator, Op, OpStatus, Order, SourcePolicy, VerifyLevel,
};
use tungstate_transfer::{
    ConflictResolver, FileOutcome, FixedResolver, InteractiveResolver, Progress, SkipReason,
    Transfer,
};

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
        from: PathBuf,
        /// Where files go.
        to: PathBuf,
        /// What this link is called on the command line.
        #[arg(long)]
        name: String,
        /// Delete each original once its copy is verified. Reclaims space.
        #[arg(long, conflicts_with = "copy")]
        r#move: bool,
        /// With --move, send originals to the trash instead of deleting them.
        #[arg(long, requires = "move")]
        trash: bool,
        /// Leave every original in place; the link becomes a verified mirror.
        #[arg(long)]
        copy: bool,
        /// How thoroughly to check each copy.
        #[arg(long, default_value = "hash")]
        verify: String,
        /// The order files are taken in.
        #[arg(long, default_value = "largest-first")]
        order: String,
        /// What an unattended run does with a conflicting file.
        #[arg(long, default_value = "quarantine")]
        on_conflict: String,
        /// Seconds a file must have been untouched before it is moved.
        #[arg(long, default_value_t = 30)]
        cooldown: u64,
    },
    /// Show what a run would do, without doing any of it.
    Preview {
        /// The link name.
        name: String,
    },
    /// List configured links.
    List,
    /// Run a link, resuming anything a previous run left unfinished.
    Run {
        /// The link name.
        name: String,
        /// Do not prompt on conflicts; use the link's configured action.
        #[arg(long)]
        yes: bool,
        /// Override the conflict action for this run.
        #[arg(long)]
        on_conflict: Option<String>,
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
        Command::Link { action } => return link(action),

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

/// Handle the `link` subcommands.
fn link(action: LinkAction) -> std::process::ExitCode {
    let journal = match Journal::open_default() {
        Ok(journal) => journal,
        Err(error) => return fail(&error),
    };

    match action {
        LinkAction::Add {
            from,
            to,
            name,
            r#move,
            trash,
            copy,
            verify,
            order,
            on_conflict,
            cooldown,
        } => {
            // No default. Guessing either fails to reclaim space or deletes
            // something the user wanted kept, so make them say which.
            let policy = match (r#move, trash, copy) {
                (true, false, false) => SourcePolicy::Delete,
                (true, true, false) => SourcePolicy::Trash,
                (false, false, true) => SourcePolicy::Keep,
                _ => {
                    eprintln!(
                        "error: choose exactly one of --move, --move --trash, or --copy\n  \
                         --move        delete each original once verified (reclaims space)\n  \
                         --move --trash send originals to the trash instead\n  \
                         --copy        leave originals alone"
                    );
                    return std::process::ExitCode::from(2);
                }
            };

            let Some(verify) = VerifyLevel::parse(&verify) else {
                eprintln!("error: --verify must be size, hash, or readback");
                return std::process::ExitCode::from(2);
            };
            let Some(order) = Order::parse(&order) else {
                eprintln!(
                    "error: --order must be largest-first, smallest-first, oldest-first, or discovered"
                );
                return std::process::ExitCode::from(2);
            };
            let Some(on_conflict) = ConflictAction::parse(&on_conflict) else {
                eprintln!("error: --on-conflict must be rename, skip, replace, or quarantine");
                return std::process::ExitCode::from(2);
            };

            let created = journal.create_link(&tungstate_journal::NewLink {
                name: name.clone(),
                source_root: from,
                destination_root: to,
                source_policy: policy,
                verify,
                order,
                on_conflict,
                cooldown: Duration::from_secs(cooldown),
                saved: true,
            });

            match created {
                Ok(_) => {
                    println!("created link `{name}`");
                    if verify != VerifyLevel::Readback && policy == SourcePolicy::Delete {
                        println!(
                            "note: verification is `{}`. Use --verify readback for the strongest \
                             check, at the cost of reading every file back.",
                            verify.as_str()
                        );
                    }
                    println!("run it with: tungstate link run {name}");
                    std::process::ExitCode::SUCCESS
                }
                Err(error) => fail(&error),
            }
        }

        LinkAction::List => match journal.links() {
            Ok(links) if links.is_empty() => {
                println!("no links configured");
                std::process::ExitCode::SUCCESS
            }
            Ok(links) => {
                for link in &links {
                    println!(
                        "{}  {} -> {}  [{}, verify={}, {}]",
                        link.name,
                        link.source_root.display(),
                        link.destination_root.display(),
                        link.source_policy.as_str(),
                        link.verify.as_str(),
                        link.order.as_str(),
                    );
                }
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },

        LinkAction::Preview { name } => preview_link(&journal, &name),

        LinkAction::Run {
            name,
            yes,
            on_conflict,
        } => run_link(&journal, &name, yes, on_conflict.as_deref()),
    }
}

fn preview_link(journal: &Journal, name: &str) -> std::process::ExitCode {
    let link = match journal.link_by_name(name) {
        Ok(link) => link,
        Err(error) => return fail(&error),
    };
    let source = LocalBackend::new(link.source_root.clone());
    let destination = LocalBackend::new(link.destination_root.clone());

    let preview = match tungstate_transfer::preview(&link, &source, &destination, None) {
        Ok(preview) => preview,
        Err(error) => return fail(&error),
    };

    println!(
        "{} -> {}\n",
        link.source_root.display(),
        link.destination_root.display()
    );

    for item in &preview.items {
        let (verb, detail) = match &item.prospect {
            tungstate_transfer::Prospect::Fresh => ("move", String::new()),
            tungstate_transfer::Prospect::SameSize { .. } => (
                "check",
                " (same size already there; fingerprints compared when you run)".to_string(),
            ),
            tungstate_transfer::Prospect::Clash { existing } => (
                "clash",
                format!(
                    " (a different {} file holds that name)",
                    human_bytes(*existing)
                ),
            ),
            tungstate_transfer::Prospect::TooRecent => {
                ("hold", " (written too recently)".to_string())
            }
        };
        println!(
            "  {verb:<6} {:<48} {:>10}{detail}",
            item.path.display(),
            human_bytes(item.size)
        );
    }

    println!(
        "\n{} to move ({}), {} same size, {} clash, {} held back",
        preview.fresh,
        human_bytes(preview.bytes),
        preview.same_size,
        preview.clashes,
        preview.too_recent
    );
    println!(
        "{}",
        if preview.removes_originals {
            "Originals here would be removed once each copy is verified."
        } else {
            "Originals here would be left alone."
        }
    );
    println!("\nNothing has been changed. Run it with: tungstate link run {name}");
    std::process::ExitCode::SUCCESS
}

fn run_link(
    journal: &Journal,
    name: &str,
    yes: bool,
    on_conflict: Option<&str>,
) -> std::process::ExitCode {
    let link = match journal.link_by_name(name) {
        Ok(link) => link,
        Err(error) => return fail(&error),
    };

    let unattended = match on_conflict.map(ConflictAction::parse) {
        // No override given: use whatever the link was configured with.
        None => link.on_conflict,
        Some(Some(action)) => action,
        Some(None) => {
            eprintln!("error: --on-conflict must be rename, skip, replace, or quarantine");
            return std::process::ExitCode::from(2);
        }
    };

    let source = LocalBackend::new(link.source_root.clone());
    let destination = LocalBackend::new(link.destination_root.clone());

    // Prompt only when there is actually someone there. A backgrounded or piped
    // run falls back to the link's configured action so the drain never stalls
    // waiting for an answer nobody is going to give.
    let interactive =
        !yes && on_conflict.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut prompting;
    let mut fixed;
    let resolver: &mut dyn ConflictResolver = if interactive {
        prompting = InteractiveResolver::new(
            std::io::BufReader::new(std::io::stdin()),
            std::io::stderr(),
            unattended,
        );
        &mut prompting
    } else {
        fixed = FixedResolver(unattended);
        &mut fixed
    };

    let mut progress = CliProgress;
    let outcome = Transfer::new(
        &link,
        &source,
        &destination,
        journal,
        resolver,
        &mut progress,
    )
    .run();

    match outcome {
        Ok(summary) => {
            report_summary(&summary, &link);
            std::process::ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn report_summary(summary: &tungstate_transfer::Summary, link: &tungstate_journal::Link) {
    println!(
        "\n{} transferred ({}), {} already there, {} skipped, {} quarantined",
        summary.transferred,
        human_bytes(summary.bytes),
        summary.already_present,
        summary.skipped,
        summary.quarantined,
    );
    if summary.destination_lost {
        println!(
            "\nSTOPPED: {} is no longer reachable, or is not the storage it was.\n\
             Nothing further was moved and every remaining original is untouched.\n\
             Reconnect it and run this again.",
            link.destination_root.display()
        );
    }
    if summary.recovered > 0 {
        println!(
            "{} interrupted transfer(s) were re-queued",
            summary.recovered
        );
    }
    if !summary.failures.is_empty() {
        println!(
            "\n{} file(s) failed; their originals were left alone:",
            summary.failed
        );
        for failure in &summary.failures {
            println!("  {}: {}", failure.path.display(), failure.reason);
        }
    }
    if summary.quarantined > 0 {
        println!(
            "review quarantined files at {}",
            link.destination_root
                .join(".tungstate-quarantine")
                .display()
        );
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

struct CliProgress;

impl Progress for CliProgress {
    fn starting(&mut self, path: &Path, size: u64) {
        print!("  {} ({})... ", path.display(), human_bytes(size));
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }

    fn finished(&mut self, _path: &Path, outcome: FileOutcome) {
        println!(
            "{}",
            match outcome {
                FileOutcome::Transferred => "done",
                FileOutcome::AlreadyPresent => "already there",
                FileOutcome::Skipped(SkipReason::RecentlyModified) =>
                    "skipped (written too recently; will move next run)",
                FileOutcome::Skipped(SkipReason::Conflict) => "skipped (name taken)",
                FileOutcome::Quarantined => "quarantined",
                FileOutcome::Failed => "FAILED (original left in place)",
            }
        );
    }
}
