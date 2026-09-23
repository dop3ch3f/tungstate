//! The `tungstate` command line interface.

mod apply;
mod connection;
mod dedupe;
mod explain;
mod folder;
mod folders;
mod plan;

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use std::time::Duration;

use tungstate_backend::Backend;
// `ends` lives in the journal crate: it owns `Endpoint` and `Connection`,
// `parse_end` needs a `Journal` anyway, and the window is now a third caller
// of the same rules.
use tungstate_journal::{
    ConflictAction, Journal, Locator, Op, OpStatus, Order, SourcePolicy, VerifyLevel, ends,
};
use tungstate_secret::{EnvOverride, KeyringStore, MemoryStore, SecretStore};
use tungstate_transfer::{
    ConflictResolver, FileOutcome, FixedResolver, InteractiveResolver, Original, Progress,
    SkipReason, Transfer,
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
    /// Give a folder a starting set of rules.
    ///
    /// Writes `.tungstate/policy.toml`. Never overwrites an existing one.
    Init {
        /// The folder. Defaults to here.
        path: Option<String>,
        /// Which starting layout to use. Omit to see the choices.
        #[arg(long)]
        template: Option<String>,
    },

    /// Manage transfer links between folders.
    Link {
        #[command(subcommand)]
        action: LinkAction,
    },
    /// Manage connections to places other than this machine.
    Connection {
        #[command(subcommand)]
        action: connection::ConnectionAction,
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
    /// Show where a policy would put a file, and why.
    Explain {
        /// The file to explain: a path, or `connection:path`.
        target: String,
        /// The policy to use, instead of the nearest `.tungstate/policy.toml`.
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Emit the trace as JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Work out what would have to happen for a folder to match its policy.
    ///
    /// Changes nothing. Redirect `--json` to keep the plan.
    Plan {
        /// The folder, or anything inside it. Defaults to here.
        target: Option<String>,
        /// The policy to use, instead of the nearest `.tungstate/policy.toml`.
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Emit the plan as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Carry out a plan: make the folder match its policy.
    ///
    /// Refuses a large reorganisation without --yes. Everything it does can be
    /// taken back with `tungstate undo`.
    Apply {
        /// The folder, or anything inside it. Defaults to here.
        target: Option<String>,
        /// Apply a plan saved earlier with `tungstate plan --json`.
        #[arg(long = "plan", value_name = "FILE")]
        saved: Option<PathBuf>,
        /// Go ahead past the blast-radius and settling refusals.
        #[arg(long)]
        yes: bool,
    },

    /// Find files that are the same file, and deal with the extra copies.
    ///
    /// Reports and changes nothing unless you add --apply. Names are never
    /// compared: two copies of one video are found however they are called.
    Dedupe {
        /// The folder, or anything inside it. Defaults to here.
        target: Option<String>,
        /// Deal with the extra copies, rather than only reporting them.
        #[arg(long)]
        apply: bool,
        /// What happens to the copies that are not kept. Asked once if omitted.
        #[arg(long, value_name = "WHAT")]
        extras: Option<String>,
        /// Keep this copy, whichever way the tie-break would have gone.
        #[arg(long, value_name = "PATH")]
        keep: Vec<String>,
        /// Do not group whole folders, only files.
        #[arg(long = "files-only")]
        files_only: bool,
        /// Also look for files that are nearly the same: a photo re-exported
        /// smaller, a video re-encoded, one song at two bitrates.
        #[arg(long)]
        similar: bool,
        /// Emit what was found as JSON.
        #[arg(long)]
        json: bool,
        /// Do not stop to ask; sets extra copies aside, which loses nothing.
        #[arg(long)]
        yes: bool,
    },

    /// Put back what `tungstate apply` did.
    Undo {
        /// The folder, or anything inside it. Defaults to here.
        target: Option<String>,
        /// Reverse the last N reorganisations of this folder. Defaults to 1.
        #[arg(long, value_name = "N")]
        last: Option<usize>,
        /// Reverse one reorganisation by id, as `apply` reported it.
        #[arg(long = "plan", value_name = "ID")]
        plan: Option<i64>,
    },

    /// Keep, clear and bring back what tungstate remembers.
    Storage {
        #[command(subcommand)]
        action: StorageAction,
    },
    /// Check and inspect policies.
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
}

#[derive(Subcommand)]
enum StorageAction {
    /// Write everything out as a JSON document.
    Export {
        /// Where to write it. Omit for standard output.
        path: Option<PathBuf>,
    },
    /// Read a document back into an empty journal.
    Import {
        /// The document to read.
        path: PathBuf,
    },
    /// Archive everything and start again empty.
    Reset,
    /// List what has been archived.
    Archives,
    /// Bring an archive back, archiving what is here first.
    Restore {
        /// The archive's name, as `storage archives` lists it.
        name: String,
    },
    /// Delete an archive for good. The only thing here that loses anything.
    Forget {
        /// The archive's name.
        name: String,
    },
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Load a policy and report every problem with it.
    Validate {
        /// The policy to check, instead of the nearest `.tungstate/policy.toml`.
        #[arg(long)]
        policy: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum FolderAction {
    /// Start governing a folder.
    Add {
        /// Path to the folder to govern.
        path: String,
        /// What to call it on screen. Defaults to the directory's own name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Show every folder being governed.
    List,
    /// Read the shape a folder already has, and write it down as rules.
    ///
    /// Changes nothing unless `--write` is given.
    Learn {
        /// The folder to read.
        path: String,
        /// Save the rules as the folder's own `.tungstate/policy.toml`.
        #[arg(long)]
        write: bool,
        /// Use the suggested improvements rather than the shape as it stands.
        #[arg(long)]
        improved: bool,
    },
    /// Show what every starting layout would do to a folder, side by side.
    ///
    /// Changes nothing.
    Compare {
        /// The folder to try them against.
        path: String,
    },
    /// Stop governing a folder. Its rules and its history are untouched.
    Remove {
        /// Path to the folder to forget.
        path: String,
    },
}

/// The arguments to `link add`.
///
/// Its own struct rather than inline variant fields so the handler can take
/// one value; clap renders it identically either way.
#[derive(Args)]
struct AddLink {
    /// Where files come from: a path, or `connection:path`.
    from: String,
    /// Where files go: a path, or `connection:path`.
    to: String,
    /// Name the source's connection explicitly, leaving `from` a path.
    #[arg(long)]
    from_connection: Option<String>,
    /// Name the destination's connection explicitly, leaving `to` a path.
    #[arg(long)]
    to_connection: Option<String>,
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
}

#[derive(Subcommand)]
enum LinkAction {
    /// Create a transfer link from a source to a destination.
    Add(AddLink),
    /// Show what a run would do, without doing any of it.
    Preview {
        /// The link name.
        name: String,
    },
    /// List configured links.
    List,
    /// List work a previous run left unfinished, across every link.
    Unfinished,
    /// Abandon a link's unfinished work and clear what it left behind.
    Discard {
        /// The link name.
        name: String,
    },
    /// Remove a link. Its history is kept.
    Remove {
        /// The link name.
        name: String,
    },
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
        /// Most files to move at once.
        ///
        /// Normally decided by asking the destination how many connections it
        /// will accept. Raising this raises the ceiling that is climbed
        /// towards; it does not skip the asking, and it does not stop
        /// tungstate backing off if the far side objects.
        #[arg(long)]
        parallel: Option<usize>,
    },
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            // Ours at info, everybody else's at warn. A media decoder narrates
            // every atom of a container it does not like, and a corrupt mp4
            // would otherwise fill somebody's terminal with a library's
            // internal monologue. RUST_LOG still overrides all of it.
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,tungstate=info".into()),
        )
        .init();
    let cli = Cli::parse();

    // Every subcommand does something now. The "not implemented yet"
    // fallthrough that stood here since slice 0 became unreachable the moment
    // `folder` stopped being a stub, and the compiler said so -- which is also
    // why every arm is an expression rather than a `return`. Exit code 2 still
    // means "not built yet" where a command refuses part of its own surface,
    // such as planning a folder on a connection.
    match cli.command {
        Command::Folder { action } => match action {
            FolderAction::Add { path, name } => folders::add(&path, name.as_deref()),
            FolderAction::List => folders::list(),
            FolderAction::Learn {
                path,
                write,
                improved,
            } => folders::learn(&path, write, improved),
            FolderAction::Compare { path } => folders::compare(&path),
            FolderAction::Remove { path } => folders::remove(&path),
        },
        Command::Init { path, template } => folders::init(path.as_deref(), template.as_deref()),
        Command::Link { action } => link(action),
        Command::Connection { action } => match open_journal() {
            Ok(journal) => connection::run(action, &journal, secret_store().as_ref()),
            Err(error) => fail(&error),
        },
        Command::Log { path } => report(|journal| journal.history(&path)),
        Command::Whereis { target } => report(|journal| {
            // A BLAKE3 digest is 64 hex characters, and no real path looks
            // like one, so the shape of the argument decides how to read it.
            let locator = if is_hash(&target) {
                Locator::Hash(&target)
            } else {
                Locator::Path(Path::new(&target))
            };
            journal.whereis(&locator)
        }),
        Command::Explain {
            target,
            policy,
            json,
        } => explain::explain(&target, policy.as_deref(), json),
        Command::Storage { action } => storage(action),
        Command::Plan {
            target,
            policy,
            json,
        } => plan::plan(target.as_deref(), policy.as_deref(), json),
        Command::Apply { target, saved, yes } => {
            apply::apply(target.as_deref(), saved.as_deref(), yes)
        }
        Command::Dedupe {
            target,
            apply,
            extras,
            keep,
            files_only,
            similar,
            json,
            yes,
        } => {
            let chosen = match extras.as_deref() {
                None => None,
                Some("set-aside") => Some(tungstate_core::dupes::Extras::SetAside),
                Some("trash") => Some(tungstate_core::dupes::Extras::Trash),
                Some(other) => {
                    eprintln!("error: `{other}` is not a choice: use set-aside or trash");
                    return std::process::ExitCode::FAILURE;
                }
            };
            dedupe::dedupe(
                target.as_deref(),
                &dedupe::Asked {
                    apply,
                    extras: chosen,
                    keep,
                    files_only,
                    similar,
                    json,
                    yes,
                },
            )
        }
        Command::Undo { target, last, plan } => apply::undo(target.as_deref(), last, plan),
        Command::Policy { action } => match action {
            PolicyAction::Validate { policy } => explain::validate(policy.as_deref()),
        },
    }
}

/// Open the machine journal, honouring the test override.
///
/// `TUNGSTATE_JOURNAL` exists so the integration tests get their own database
/// and stay parallel-safe. Without it every CLI test would write to the real
/// one on the developer's machine.
fn open_journal() -> tungstate_journal::Result<Journal> {
    match std::env::var_os("TUNGSTATE_JOURNAL") {
        Some(path) => Journal::open(Path::new(&path)),
        None => Journal::open_default(),
    }
}

/// Where passwords are kept.
///
/// Always wrapped in [`EnvOverride`], so `TUNGSTATE_SECRET_<NAME>` works
/// everywhere: on a NAS or in a container there is no keychain to ask, and it
/// is how the FTP integration tests hand the binary a password without going
/// near the developer's real one.
///
/// `TUNGSTATE_SECRETS=memory` is the sibling of `TUNGSTATE_JOURNAL`, for tests
/// that must not write to the real credential store. It is per-process and
/// therefore forgets between commands, which is exactly why the environment
/// override exists alongside it.
pub(crate) fn secret_store() -> Box<dyn SecretStore> {
    match std::env::var("TUNGSTATE_SECRETS").as_deref() {
        Ok("memory") => Box::new(EnvOverride(MemoryStore::new())),
        _ => Box::new(EnvOverride(KeyringStore::new())),
    }
}

fn is_hash(target: &str) -> bool {
    target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit())
}

/// Handle the `storage` subcommands.
fn storage(action: StorageAction) -> std::process::ExitCode {
    let journal = match open_journal() {
        Ok(journal) => journal,
        Err(error) => return fail(&error),
    };

    match action {
        StorageAction::Export { path } => {
            let document = match journal.export() {
                Ok(document) => document,
                Err(error) => return fail(&error),
            };
            let text = serde_json::to_string_pretty(&document)
                .expect("an export is plain data and always prints");
            match path {
                None => println!("{text}"),
                Some(path) => {
                    if let Err(error) = std::fs::write(&path, text) {
                        eprintln!("error: could not write `{}`: {error}", path.display());
                        return std::process::ExitCode::FAILURE;
                    }
                    println!("wrote {}", path.display());
                    println!("it carries no passwords; those stay in the keychain");
                }
            }
            std::process::ExitCode::SUCCESS
        }

        StorageAction::Import { path } => {
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(error) => {
                    eprintln!("error: could not read `{}`: {error}", path.display());
                    return std::process::ExitCode::FAILURE;
                }
            };
            let document = match serde_json::from_str(&raw) {
                Ok(document) => document,
                Err(error) => {
                    eprintln!("error: `{}` is not readable JSON: {error}", path.display());
                    return std::process::ExitCode::FAILURE;
                }
            };
            match journal.import(&document) {
                Ok(rows) => {
                    println!("read {rows} row(s) from {}", path.display());
                    println!(
                        "re-enter any connection passwords with: tungstate connection password <NAME>"
                    );
                    std::process::ExitCode::SUCCESS
                }
                Err(error) => fail(&error),
            }
        }

        StorageAction::Reset => match journal.reset() {
            Ok(name) => {
                println!("archived everything as `{name}` and started again empty");
                println!("bring it back with: tungstate storage restore {name}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },

        StorageAction::Archives => match journal.archives() {
            Ok(archives) if archives.is_empty() => {
                println!("nothing has been archived");
                std::process::ExitCode::SUCCESS
            }
            Ok(archives) => {
                for archive in &archives {
                    println!("{}  {}", archive.name, human_bytes(archive.size));
                    match &archive.summary {
                        Some(summary) => println!(
                            "  {} link(s), {} connection(s), {} operation(s), {} plan(s)",
                            summary.links, summary.connections, summary.operations, summary.plans
                        ),
                        // Shown rather than hidden: an archive that will not
                        // open is exactly what somebody needs to know about.
                        None => println!("  (this one cannot be read)"),
                    }
                }
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },

        StorageAction::Restore { name } => match journal.restore(&name) {
            Ok(put_aside) => {
                println!("restored `{name}`");
                println!("what was here is archived as `{put_aside}`, so this is reversible");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },

        StorageAction::Forget { name } => match journal.forget_archive(&name) {
            Ok(()) => {
                println!("deleted archive `{name}`");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
    }
}

/// Open the machine journal, run a query, and print the rows.
fn report(
    query: impl FnOnce(&Journal) -> tungstate_journal::Result<Vec<Op>>,
) -> std::process::ExitCode {
    let journal = match open_journal() {
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
                println!("{}", render(op, &journal));
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

pub(crate) fn fail(error: &dyn std::error::Error) -> std::process::ExitCode {
    eprintln!("error: {error}");
    // The chain is where the real cause lives; printing only the top line loses it.
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    std::process::ExitCode::FAILURE
}

fn render(op: &Op, journal: &Journal) -> String {
    let status = match op.status {
        OpStatus::Intended => "interrupted",
        OpStatus::Committed => "ok",
        OpStatus::Failed => "failed",
        OpStatus::Skipped => "skipped",
    };
    // Naming the connection matters here more than anywhere: "inbox/a.mp4" is
    // not an answer to "where did this file go?".
    let source = op.source.as_ref().map(|l| ends::place(l, journal));
    let destination = op.destination.as_ref().map(|l| ends::place(l, journal));

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

/// Create one link, resolving both ends first.
fn add_link(journal: &Journal, args: &AddLink) -> std::process::ExitCode {
    // No default. Guessing either fails to reclaim space or deletes something
    // the user wanted kept, so make them say which.
    let policy = match (args.r#move, args.trash, args.copy) {
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

    let Some(verify) = VerifyLevel::parse(&args.verify) else {
        eprintln!("error: --verify must be size, hash, or readback");
        return std::process::ExitCode::from(2);
    };
    let Some(order) = Order::parse(&args.order) else {
        eprintln!(
            "error: --order must be largest-first, smallest-first, oldest-first, or discovered"
        );
        return std::process::ExitCode::from(2);
    };
    let Some(on_conflict) = ConflictAction::parse(&args.on_conflict) else {
        eprintln!("error: --on-conflict must be rename, skip, replace, or quarantine");
        return std::process::ExitCode::from(2);
    };

    let source = match ends::parse_end(&args.from, args.from_connection.as_deref(), journal) {
        Ok(end) => end,
        Err(error) => return fail(&error),
    };
    let destination = match ends::parse_end(&args.to, args.to_connection.as_deref(), journal) {
        Ok(end) => end,
        Err(error) => return fail(&error),
    };

    // `trash::delete` drives this desktop's trash and knows nothing about a
    // remote. Remote trash (`.tungstate-trash/`, DESIGN.md §4) is a later
    // slice, so say so now rather than at the first file. The message names
    // the two alternatives rather than guessing which one was meant.
    if policy == SourcePolicy::Trash && source.is_remote() {
        eprintln!(
            "error: --move --trash needs a source on this machine, and `{}` is remote\n  \
             --move   delete each original once verified\n  \
             --copy   leave every original alone",
            args.from
        );
        return std::process::ExitCode::from(2);
    }

    // Both refusals below are about the destination's protocol, so they need
    // the connection rather than just the endpoint.
    let destination_scheme = match destination.connection {
        None => None,
        Some(id) => match journal.connection_by_id(id) {
            Ok(connection) => Some(connection.scheme),
            Err(error) => return fail(&error),
        },
    };

    // `replace` keeps the existing file by moving it aside first, and moving
    // needs rename, which FTP does not give us. Refused here rather than at
    // the first clash, halfway through a drain.
    if on_conflict == ConflictAction::Replace && destination_scheme.is_some_and(|s| !s.can_rename())
    {
        eprintln!(
            "error: --on-conflict replace needs a destination that can rename, so the \
             file already there can be moved aside first.\n  \
             `{}` cannot. Use quarantine, rename or skip.",
            args.to
        );
        return std::process::ExitCode::from(2);
    }

    let created = journal.create_link(&tungstate_journal::NewLink {
        name: args.name.clone(),
        source,
        destination,
        source_policy: policy,
        verify,
        order,
        on_conflict,
        cooldown: Duration::from_secs(args.cooldown),
        saved: true,
    });

    match created {
        Ok(_) => {
            let name = &args.name;
            println!("created link `{name}`");
            // A destination with no server-side checksum gets the stronger
            // nudge, because there `hash` only ever means "the bytes we sent
            // hashed to this", never "the bytes on the far disk do".
            if verify != VerifyLevel::Readback
                && destination_scheme.is_some_and(|s| !s.has_native_checksum())
            {
                println!(
                    "note: this destination cannot checksum a file for us, so `--verify {}` \n      \
                     only proves what was sent, not what landed. Over a network, \
                     --verify readback\n      is the one that proves it, at the cost of \
                     reading every file back.",
                    verify.as_str()
                );
            } else if verify != VerifyLevel::Readback && policy == SourcePolicy::Delete {
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

/// Handle the `link` subcommands.
fn link(action: LinkAction) -> std::process::ExitCode {
    let journal = match open_journal() {
        Ok(journal) => journal,
        Err(error) => return fail(&error),
    };

    match action {
        LinkAction::Add(args) => add_link(&journal, &args),

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
                        ends::describe(&link.source, &journal),
                        ends::describe(&link.destination, &journal),
                        link.source_policy.as_str(),
                        link.verify.as_str(),
                        link.order.as_str(),
                    );
                }
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },

        LinkAction::Unfinished => unfinished(&journal),

        LinkAction::Discard { name } => discard_link(&journal, &name),

        LinkAction::Remove { name } => match journal.remove_link(&name) {
            Ok(tungstate_journal::Removal::Deleted) => {
                println!("removed link `{name}`");
                std::process::ExitCode::SUCCESS
            }
            Ok(tungstate_journal::Removal::Retired) => {
                // Said plainly rather than reported as a plain delete: the row
                // is still there, and somebody reading `log` later should not
                // be surprised that it still names this link.
                println!("removed link `{name}`");
                println!(
                    "it had run before, so its record is kept and `tungstate log` still \n                       resolves it. The name is free to use again."
                );
                std::process::ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },

        LinkAction::Preview { name } => preview_link(&journal, &name),

        LinkAction::Run {
            name,
            yes,
            on_conflict,
            parallel,
        } => run_link(&journal, &name, yes, on_conflict.as_deref(), parallel),
    }
}

/// Report every link with work begun and never finished.
///
/// Deliberately not filtered by whether the link was saved. A one-off transfer
/// from the browser is unsaved, and it is exactly the case that would
/// otherwise leave a part-copied file with nothing able to find it again.
fn unfinished(journal: &Journal) -> std::process::ExitCode {
    let runs = match journal.interrupted() {
        Ok(runs) => runs,
        Err(error) => return fail(&error),
    };
    if runs.is_empty() {
        println!("nothing was left unfinished");
        return std::process::ExitCode::SUCCESS;
    }

    for run in &runs {
        println!(
            "{}  {} -> {}",
            run.link.name,
            ends::describe(&run.link.source, journal),
            ends::describe(&run.link.destination, journal),
        );
        println!(
            "  {} file(s), {} — every original is still in place",
            run.ops.len(),
            human_bytes(run.bytes())
        );
        for op in run.ops.iter().take(5) {
            if let Some(source) = &op.source {
                println!("    {}", source.path.display());
            }
        }
        if run.ops.len() > 5 {
            println!("    and {} more", run.ops.len() - 5);
        }
        println!(
            "  finish it with:  tungstate link run {}\n  \
             or clear it with: tungstate link discard {}\n",
            run.link.name, run.link.name
        );
    }
    std::process::ExitCode::SUCCESS
}

/// Clear what an interrupted run left behind, without copying anything.
fn discard_link(journal: &Journal, name: &str) -> std::process::ExitCode {
    let link = match journal.link_by_name(name) {
        Ok(link) => link,
        Err(error) => return fail(&error),
    };
    let destination = match tungstate_backend_opendal::open(
        &link.destination,
        journal,
        secret_store().as_ref(),
    ) {
        Ok(backend) => backend,
        Err(error) => return fail(&error),
    };

    match tungstate_transfer::discard(&link, destination.as_ref(), journal) {
        Ok(removed) if removed.operations == 0 => {
            println!("`{name}` had nothing unfinished");
            std::process::ExitCode::SUCCESS
        }
        Ok(removed) => {
            println!(
                "cleared {} unfinished operation(s) for `{name}`, covering {}",
                removed.operations,
                human_bytes(removed.bytes)
            );
            println!("every original is untouched; nothing was copied");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn preview_link(journal: &Journal, name: &str) -> std::process::ExitCode {
    let link = match journal.link_by_name(name) {
        Ok(link) => link,
        Err(error) => return fail(&error),
    };
    let (source, destination) = match ends_of(&link, journal) {
        Ok(pair) => pair,
        Err(error) => return fail(error.as_ref()),
    };

    let preview =
        match tungstate_transfer::preview(&link, source.as_ref(), destination.as_ref(), None) {
            Ok(preview) => preview,
            Err(error) => return fail(&error),
        };

    println!(
        "{} -> {}\n",
        ends::describe(&link.source, journal),
        ends::describe(&link.destination, journal)
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
    parallel: Option<usize>,
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

    let (source, destination) = match ends_of(&link, journal) {
        Ok(pair) => pair,
        Err(error) => return fail(error.as_ref()),
    };

    // Prompt only when there is someone there *and* the question is open. A
    // backgrounded or piped run falls back to the configured action so the
    // drain never stalls waiting for an answer nobody will give; and a link
    // whose rule is `rename` or `skip` has already answered, so asking would
    // hold a worker over a settled question. `wants_asking` is the same rule
    // the window reads, so the two cannot disagree about one setting.
    let interactive = !yes
        && on_conflict.is_none()
        && unattended.wants_asking()
        && std::io::IsTerminal::is_terminal(&std::io::stdin());
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

    let mut progress = CliProgress::new();
    let mut transfer = Transfer::new(
        &link,
        source.as_ref(),
        destination.as_ref(),
        journal,
        resolver,
        &mut progress,
    );
    if let Some(at_once) = parallel {
        transfer = transfer.parallel(at_once);
    }
    let outcome = transfer.run();

    match outcome {
        Ok(summary) => {
            report_summary(&summary, &ends::describe(&link.destination, journal));
            std::process::ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

/// The two backends a run needs, in source-then-destination order.
type Ends = (Box<dyn Backend>, Box<dyn Backend>);

/// Anything the factory can fail with, flattened for the error printer.
type BoxedError = Box<dyn std::error::Error>;

/// Both ends of a link as backends, through the factory rather than by
/// assuming the local filesystem.
///
/// Fallible where `LocalBackend::new` was not: reaching a remote can fail, and
/// pretending otherwise would defer the failure to the first file.
fn ends_of(link: &tungstate_journal::Link, journal: &Journal) -> Result<Ends, BoxedError> {
    let secrets = secret_store();
    let source = tungstate_backend_opendal::open(&link.source, journal, secrets.as_ref())?;
    let destination =
        tungstate_backend_opendal::open(&link.destination, journal, secrets.as_ref())?;
    Ok((source, destination))
}

/// `destination` is the end written the way the user typed it, so a remote is
/// named by its connection rather than by a path that means nothing on its own.
fn report_summary(summary: &tungstate_transfer::Summary, destination: &str) {
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
            "\nSTOPPED: {destination} is no longer reachable, or is not the storage it was.\n\
             Nothing further was moved and every remaining original is untouched.\n\
             Reconnect it and run this again."
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
        println!("review quarantined files under {destination}/.tungstate-quarantine");
    }
}

pub(crate) fn human_bytes(bytes: u64) -> String {
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

/// Progress on one line per file, rewritten in place while it copies.
#[derive(Default)]
struct CliProgress {
    /// The file in flight, so a progress line can name it.
    current: Option<(String, u64)>,
    /// Longest line drawn for this file, so the next one can cover it.
    width: usize,
    /// Rewriting in place only makes sense for a person watching. Piped to a
    /// file, carriage returns turn a log into one unreadable line.
    interactive: bool,
}

impl CliProgress {
    fn new() -> Self {
        Self {
            interactive: std::io::IsTerminal::is_terminal(&std::io::stdout()),
            ..Self::default()
        }
    }

    /// Draw `line` over whatever was there, padded to cover it.
    fn redraw(&mut self, line: &str) {
        let pad = self.width.saturating_sub(line.len());
        print!("\r{line}{:pad$}", "");
        self.width = line.len();
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

impl Progress for CliProgress {
    fn planned(&mut self, files: &[tungstate_transfer::Planned]) {
        let total: u64 = files.iter().map(|f| f.size).sum();
        println!("{} file(s), {} to move:", files.len(), human_bytes(total));
        for file in files.iter().take(10) {
            println!(
                "  {:<52} {:>10}",
                file.path.display(),
                human_bytes(file.size)
            );
        }
        if files.len() > 10 {
            println!("  and {} more", files.len() - 10);
        }
        println!();
    }

    fn starting(&mut self, path: &Path, size: u64) {
        self.current = Some((path.display().to_string(), size));
        self.width = 0;
        let line = format!("  {} ({})... ", path.display(), human_bytes(size));
        if self.interactive {
            self.redraw(&line);
        } else {
            print!("{line}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }

    fn checking(&mut self, _path: &Path, done: u64, total: u64) {
        if !self.interactive || total < 8 * 1024 * 1024 {
            return;
        }
        let Some((name, _)) = self.current.clone() else {
            return;
        };
        // Named differently from `advanced` on purpose: nothing is being sent,
        // and a progress line that says otherwise is why this phase exists.
        self.redraw(&format!(
            "  {name} (comparing, {} of {})... ",
            human_bytes(done),
            human_bytes(total)
        ));
    }

    fn advanced(&mut self, _path: &Path, done: u64, total: u64) {
        // Nothing to watch on a file that finishes inside one report.
        if !self.interactive || total < 8 * 1024 * 1024 || done == total {
            return;
        }
        let Some((name, _)) = self.current.clone() else {
            return;
        };
        let line = format!(
            "  {name} ({} of {})... ",
            human_bytes(done),
            human_bytes(total)
        );
        self.redraw(&line);
    }

    fn finished(&mut self, path: &Path, outcome: FileOutcome) {
        // Restore the settled line before the verdict, so a file that showed
        // "2.1 GiB of 3.8 GiB" while copying does not keep saying it.
        if self.interactive
            && let Some((name, size)) = self.current.take()
        {
            let _ = path;
            self.redraw(&format!("  {name} ({})... ", human_bytes(size)));
        }
        println!(
            "{}",
            match outcome {
                FileOutcome::Transferred => "done",
                FileOutcome::AlreadyPresent(Original::Removed) =>
                    "identical copy already there; original removed",
                FileOutcome::AlreadyPresent(Original::Kept) =>
                    "identical copy already there; original kept",
                FileOutcome::Skipped(SkipReason::RecentlyModified) =>
                    "skipped (written too recently; will move next run)",
                FileOutcome::Skipped(SkipReason::Conflict) => "skipped (name taken)",
                FileOutcome::Quarantined => "quarantined",
                FileOutcome::Failed => "FAILED (original left in place)",
            }
        );
    }
}
