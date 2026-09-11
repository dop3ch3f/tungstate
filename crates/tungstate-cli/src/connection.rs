//! The `tungstate connection` subcommands.

use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::Subcommand;
use tungstate_journal::{ConnectionSettings, Journal, NewConnection, Scheme};
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
    /// Change a connection. Only the flags given are touched.
    ///
    /// There is deliberately no `--name`: the name is what link specs were
    /// written against and what the keychain entry is filed under, so a rename
    /// is two migrations wearing one hat. Remove and re-add instead.
    Update {
        /// The connection name.
        name: String,
        /// Which protocol it speaks.
        #[arg(long)]
        scheme: Option<String>,
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
        #[arg(long)]
        root: Option<String>,
        /// Set one per-scheme extra, as `key=value`. Repeatable.
        #[arg(long = "option", value_name = "KEY=VALUE")]
        options: Vec<String>,
    },
    /// Replace the stored password for a connection.
    ///
    /// The recovery path for a password that was mistyped or has since
    /// changed: `connection remove` is refused while any link points at the
    /// connection, so until now there was no way back from a refused login.
    Password {
        /// The connection name.
        name: String,
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
        ConnectionAction::Update {
            name,
            scheme,
            host,
            port,
            username,
            root,
            options,
        } => update(
            journal,
            &UpdateArgs {
                name,
                scheme,
                host,
                port,
                username,
                root,
                options,
            },
        ),
        ConnectionAction::Password { name, secret_stdin } => {
            password(journal, secrets, &name, secret_stdin)
        }
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

/// Grouped for the same reason [`AddArgs`] is.
struct UpdateArgs {
    name: String,
    scheme: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    root: Option<String>,
    options: Vec<String>,
}

/// Partial CLI semantics over a journal API that is a full replace.
///
/// Read the row, overlay the flags that were given, write the whole settings
/// struct back. The partial shape belongs here because a flag that was not
/// typed means "leave it"; the journal takes every field because a form
/// submits every field.
fn update(journal: &Journal, args: &UpdateArgs) -> ExitCode {
    let current = match journal.connection_by_name(&args.name) {
        Ok(connection) => connection,
        Err(error) => return fail(&error),
    };

    let scheme = match args.scheme.as_deref().map(Scheme::parse) {
        None => current.scheme,
        Some(Some(scheme)) => scheme,
        Some(None) => {
            eprintln!("error: --scheme must be fs, ftp or ftps");
            return ExitCode::from(2);
        }
    };

    // Merged rather than replaced: a repeatable key=value flag reads as "set
    // this one", and there is no way to spell "and drop the others" that a
    // second `--option` would not also mean. `--option k=` clears one value.
    let mut options = current.options.clone();
    for option in &args.options {
        let Some((key, value)) = option.split_once('=') else {
            eprintln!("error: --option must be written key=value, got `{option}`");
            return ExitCode::from(2);
        };
        options.insert(key.to_string(), value.to_string());
    }

    let settings = ConnectionSettings {
        scheme,
        host: args.host.clone().or(current.host),
        port: args.port.or(current.port),
        username: args.username.clone().or(current.username),
        root: args.root.clone().unwrap_or(current.root),
        options,
    };

    if let Err(error) = journal.update_connection(&args.name, &settings) {
        return fail(&error);
    }

    println!("updated connection `{}`", args.name);

    // Two notes, both about a scheme change, because that is the edit whose
    // consequences are not on the screen. Neither is an error.
    if !scheme.is_encrypted() {
        eprintln!(
            "warning: `{}` sends your password and your files across the network unencrypted.",
            scheme.as_str()
        );
    }
    if scheme.authenticates() && !current.scheme.authenticates() {
        println!(
            "note: this scheme needs a password, and none is stored yet.\n      \
             set one with: tungstate connection password {}",
            args.name
        );
    }
    println!("check it with: tungstate connection test {}", args.name);
    ExitCode::SUCCESS
}

/// Replace the stored password, leaving the row alone.
///
/// The connection is looked up first so a typo names itself rather than
/// writing a keychain entry nothing will ever read.
fn password(
    journal: &Journal,
    secrets: &dyn SecretStore,
    name: &str,
    from_stdin: bool,
) -> ExitCode {
    let connection = match journal.connection_by_name(name) {
        Ok(connection) => connection,
        Err(error) => return fail(&error),
    };
    if !connection.scheme.authenticates() {
        eprintln!(
            "error: `{name}` speaks {}, which does not authenticate, so a password would \
             never be read",
            connection.scheme.as_str()
        );
        return ExitCode::from(2);
    }

    let typed = match read_secret(name, from_stdin) {
        Ok(secret) => secret,
        Err(error) => {
            eprintln!("error: could not read a password: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = store_password(secrets, name, typed.as_deref()) {
        eprintln!("error: the password for `{name}` was not saved: {error}");
        return ExitCode::FAILURE;
    }

    match typed {
        Some(_) => println!("stored a new password for `{name}`"),
        None => println!("removed the stored password for `{name}`"),
    }
    println!("check it with: tungstate connection test {name}");
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

/// Apply a typed answer to the store.
///
/// Its own function because it is the part worth testing: `password` above is
/// a prompt and two lookups around it, and neither a prompt nor the machine's
/// real keychain belongs in a test.
///
/// A blank answer removes the stored password rather than storing an empty
/// one, which is the rule `add` already follows for a blank prompt.
fn store_password(
    secrets: &dyn SecretStore,
    name: &str,
    typed: Option<&str>,
) -> tungstate_secret::Result<()> {
    match typed {
        Some(secret) => secrets.set(&connection_key(name), secret),
        None => secrets.delete(&connection_key(name)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tungstate_secret::MemoryStore;

    /// The end-to-end suite cannot see this: `TUNGSTATE_SECRETS=memory` is
    /// per-process, so what one `tungstate` invocation stores is gone before
    /// the next one starts. In-process is where the replacement is visible.
    #[test]
    fn a_new_password_replaces_the_one_already_stored() {
        let store = MemoryStore::new();
        let key = connection_key("nas");

        store_password(&store, "nas", Some("first")).unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("first"));

        store_password(&store, "nas", Some("second")).unwrap();
        assert_eq!(
            store.get(&key).unwrap().as_deref(),
            Some("second"),
            "the recovery path: a mistyped password must be correctable"
        );
    }

    #[test]
    fn a_blank_answer_removes_the_stored_password() {
        // Not an empty string in the keychain, which would be a credential
        // that exists and always fails rather than no credential at all.
        let store = MemoryStore::new();
        let key = connection_key("nas");

        store_password(&store, "nas", Some("first")).unwrap();
        store_password(&store, "nas", None).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }
}
