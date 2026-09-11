//! The `tungstate connection` subcommands.

use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::Subcommand;
use tungstate_journal::{Journal, NewConnection, Scheme};
use tungstate_secret::{SecretStore, connection_key};

use crate::fail;

#[derive(Subcommand)]
pub enum ConnectionAction {
    /// Record a place link ends can live, other than this machine.
    Add {
        /// What this connection is called on the command line.
        name: String,
        /// Which protocol it speaks.
        #[arg(long)]
        scheme: String,
        /// Hostname, for the schemes that have one.
        #[arg(long)]
        host: Option<String>,
        /// Port, where it differs from the protocol default.
        #[arg(long)]
        port: Option<u16>,
        /// Who to connect as.
        #[arg(long = "user")]
        username: Option<String>,
        /// Absolute path on the far side that every link path is relative to.
        ///
        /// For a remote this is a path on the server, not on this machine, and
        /// it is often not the same as the directory you land in when you log
        /// in: many servers put you in `/home/you` while `/` is the whole
        /// disk. `connection test` prints what it found there, so check it.
        #[arg(long, default_value = "")]
        root: String,
        /// Per-scheme extra, as `key=value`. Repeatable.
        #[arg(long = "option", value_name = "KEY=VALUE")]
        options: Vec<String>,
        /// Read the password from stdin rather than prompting, for scripts.
        #[arg(long)]
        secret_stdin: bool,
    },
    /// List configured connections.
    List,
    /// Check a connection is reachable, without transferring anything.
    Test {
        /// The connection name.
        name: String,
    },
    /// Forget a connection. Refused while a link still points at it.
    Remove {
        /// The connection name.
        name: String,
    },
}

/// Handle the `connection` subcommands.
pub fn run(action: ConnectionAction, journal: &Journal, secrets: &dyn SecretStore) -> ExitCode {
    match action {
        ConnectionAction::Add {
            name,
            scheme,
            host,
            port,
            username,
            root,
            options,
            secret_stdin,
        } => add(
            journal,
            secrets,
            &AddArgs {
                name,
                scheme,
                host,
                port,
                username,
                root,
                options,
                secret_stdin,
            },
        ),
        ConnectionAction::List => list(journal),
        ConnectionAction::Test { name } => test(journal, secrets, &name),
        ConnectionAction::Remove { name } => remove(journal, secrets, &name),
    }
}

/// Grouped so `add` takes one argument rather than eight, which clippy is right
/// to insist on and which makes the call site readable.
struct AddArgs {
    name: String,
    scheme: String,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    root: String,
    options: Vec<String>,
    secret_stdin: bool,
}

fn add(journal: &Journal, secrets: &dyn SecretStore, args: &AddArgs) -> ExitCode {
    let Some(scheme) = Scheme::parse(&args.scheme) else {
        eprintln!("error: --scheme must be fs, ftp or ftps");
        return ExitCode::from(2);
    };

    let mut parsed = BTreeMap::new();
    for option in &args.options {
        let Some((key, value)) = option.split_once('=') else {
            eprintln!("error: --option must be written key=value, got `{option}`");
            return ExitCode::from(2);
        };
        parsed.insert(key.to_string(), value.to_string());
    }

    // Said before the password prompt, so the user can still change their mind
    // about sending it in the clear.
    if !scheme.is_encrypted() {
        eprintln!(
            "warning: `{}` sends your password and your files across the network \
             unencrypted.\n  \
             If the server offers it, use --scheme ftps instead.",
            scheme.as_str()
        );
    }

    // Read the password before writing the row. Storing a connection whose
    // password prompt was then cancelled would leave something half-made.
    let secret = if scheme.authenticates() {
        match read_secret(&args.name, args.secret_stdin) {
            Ok(secret) => secret,
            Err(error) => {
                eprintln!("error: could not read a password: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    let created = journal.create_connection(&NewConnection {
        name: args.name.clone(),
        scheme,
        host: args.host.clone(),
        port: args.port,
        username: args.username.clone(),
        root: args.root.clone(),
        options: parsed,
    });
    if let Err(error) = created {
        return fail(&error);
    }

    if let Some(secret) = secret
        && let Err(error) = secrets.set(&connection_key(&args.name), &secret)
    {
        // The row is already written, so say plainly what did and did not
        // happen rather than leaving the user to guess.
        eprintln!(
            "error: `{}` was saved, but its password was not: {error}",
            args.name
        );
        return ExitCode::FAILURE;
    }

    println!("added connection `{}`", args.name);
    println!("check it with: tungstate connection test {}", args.name);
    ExitCode::SUCCESS
}

fn list(journal: &Journal) -> ExitCode {
    match journal.connections() {
        Ok(connections) if connections.is_empty() => {
            println!("no connections configured");
            ExitCode::SUCCESS
        }
        Ok(connections) => {
            for connection in &connections {
                let where_to = match (&connection.host, connection.port) {
                    (Some(host), Some(port)) => format!("{host}:{port}"),
                    (Some(host), None) => host.clone(),
                    _ => "this machine".to_string(),
                };
                let as_who = connection
                    .username
                    .as_deref()
                    .map(|u| format!(" as {u}"))
                    .unwrap_or_default();
                // Named on every line, not just when adding. "Which of my
                // connections is still sending passwords in the clear?" should
                // be answerable by looking.
                let wire = if connection.scheme.is_encrypted() {
                    ""
                } else {
                    "  [UNENCRYPTED]"
                };
                println!(
                    "{}  {} {}{}  root={}{}",
                    connection.name,
                    connection.scheme.as_str(),
                    where_to,
                    as_who,
                    if connection.root.is_empty() {
                        "/"
                    } else {
                        &connection.root
                    },
                    wire,
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn test(journal: &Journal, secrets: &dyn SecretStore, name: &str) -> ExitCode {
    let connection = match journal.connection_by_name(name) {
        Ok(connection) => connection,
        Err(error) => return fail(&error),
    };

    // The root is named, not just the count. "reachable, 18 entries" is
    // reassuring and useless when the 18 entries are the server's own `/bin`
    // and `/etc` because `--root` was left at the default.
    let root = if connection.root.is_empty() {
        "/"
    } else {
        &connection.root
    };
    match tungstate_backend_opendal::probe(&connection, journal, secrets) {
        Ok(count) => {
            println!("`{name}` is reachable");
            println!("  {root} holds {count} entries");
            if !connection.scheme.is_encrypted() {
                println!("  (this connection is not encrypted)");
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn remove(journal: &Journal, secrets: &dyn SecretStore, name: &str) -> ExitCode {
    // The row first: if a link still points at it the foreign key refuses, and
    // deleting the password before finding that out would break a live link.
    if let Err(error) = journal.delete_connection(name) {
        return fail(&error);
    }
    if let Err(error) = secrets.delete(&connection_key(name)) {
        eprintln!("warning: `{name}` was removed, but its saved password was not: {error}");
    }
    println!("removed connection `{name}`");
    ExitCode::SUCCESS
}

/// Read a password without it ever becoming a command-line argument.
///
/// An argument would land in shell history and be visible in `ps` to every
/// other user on the machine, which is why there is no `--password` flag.
fn read_secret(name: &str, from_stdin: bool) -> std::io::Result<Option<String>> {
    if from_stdin {
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        return Ok((!trimmed.is_empty()).then(|| trimmed.to_string()));
    }
    let typed = rpassword::prompt_password(format!("password for {name} (blank for none): "))?;
    Ok((!typed.is_empty()).then_some(typed))
}
