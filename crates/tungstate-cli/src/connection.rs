//! The `tungstate connection` subcommands.

use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::Subcommand;
use tungstate_journal::{ConnectionSettings, Journal, NewConnection, Scheme, option, share_of};
use tungstate_secret::{SecretStore, connection_key};

use crate::fail;

/// Settings only some schemes read. Each becomes an option or part of the
/// root, so the journal needs no new columns for them.
#[derive(clap::Args, Default)]
pub struct KindFlags {
    /// S3: the bucket every path lives in.
    #[arg(long)]
    bucket: Option<String>,
    /// S3: the bucket's region. Needed for AWS unless the bucket is in
    /// us-east-1; other services are asked, and most accept any.
    #[arg(long)]
    region: Option<String>,
    /// S3: the service's address, for anything that is not AWS itself, as in
    /// `https://s3.eu-central-003.backblazeb2.com` or `http://nas.local:9000`.
    /// WebDAV: the server's address, required, as in `https://nas.local:5006`.
    // This comment is the `--help` text, where backticks would show.
    #[allow(clippy::doc_markdown)]
    #[arg(long)]
    endpoint: Option<String>,
    /// SMB: the share, which the root then continues inside. `--share media
    /// --root backups` is the same as `--root media/backups`.
    #[arg(long)]
    share: Option<String>,
    /// SFTP: sign in with this private key file instead of a password. The
    /// secret asked for is then the key's passphrase, if it has one.
    #[arg(long, value_name = "PATH")]
    key: Option<String>,
}

impl KindFlags {
    /// Fold these into options and a root. `stored` says the root came from
    /// the saved connection, so it already starts with a share to replace;
    /// a root typed alongside `--share` is the folder inside it.
    fn apply(&self, options: &mut BTreeMap<String, String>, root: &mut String, stored: bool) {
        for (key, value) in [
            (option::BUCKET, &self.bucket),
            (option::REGION, &self.region),
            (option::ENDPOINT, &self.endpoint),
            (option::KEY, &self.key),
        ] {
            if let Some(value) = value {
                options.insert(key.to_string(), value.clone());
            }
        }
        if let Some(share) = &self.share {
            *root = if stored {
                let inside = share_of(root).map_or("", |current| {
                    root.trim_start_matches(['/', '\\'])[current.len()..]
                        .trim_start_matches(['/', '\\'])
                });
                with_share(share, inside)
            } else {
                with_share(share, root)
            };
        }
    }
}

/// `share` followed by `root`, the folder inside it. A root that already
/// begins with that share is left as it is, so saying it twice is harmless.
fn with_share(share: &str, root: &str) -> String {
    let share = share.trim_matches(['/', '\\']);
    let rest = match share_of(root) {
        // Already begins with this share: keep the folder part as it is.
        Some(current) if current == share => {
            return root.trim_start_matches(['/', '\\']).to_string();
        }
        _ => root.trim_start_matches(['/', '\\']),
    };
    if rest.is_empty() {
        share.to_string()
    } else {
        format!("{share}/{rest}")
    }
}

#[derive(Subcommand)]
pub enum ConnectionAction {
    /// Record a place link ends can live, other than this machine.
    Add {
        /// What this connection is called on the command line.
        name: String,
        /// Which protocol it speaks: fs (a folder this machine can reach),
        /// smb, sftp, webdav, ftps, ftp or s3.
        #[arg(long)]
        scheme: String,
        /// Hostname, for the schemes that have one.
        #[arg(long)]
        host: Option<String>,
        /// Port, where it differs from the protocol default.
        #[arg(long)]
        port: Option<u16>,
        /// Who to connect as. For S3, the access key id.
        #[arg(long = "user")]
        username: Option<String>,
        /// Absolute path on the far side that every link path is relative to.
        ///
        /// For a remote this is a path on the server, not on this machine, and
        /// it is often not the same as the directory you land in when you log
        /// in: many servers put you in `/home/you` while `/` is the whole
        /// disk. For SMB it starts with the share; for S3 it is a folder
        /// inside the bucket. `connection test` prints what it found there,
        /// so check it.
        #[arg(long, default_value = "")]
        root: String,
        #[command(flatten)]
        kind: KindFlags,
        /// Per-scheme extra, as `key=value`. Repeatable. SMB reads
        /// `encryption=required`, to refuse a server that will not encrypt.
        #[arg(long = "option", value_name = "KEY=VALUE")]
        options: Vec<String>,
        /// Read the password, or S3 secret key, from stdin rather than
        /// prompting, for scripts.
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
        /// Who to connect as. For S3, the access key id.
        #[arg(long = "user")]
        username: Option<String>,
        /// Absolute path on the far side that every link path is relative to.
        #[arg(long)]
        root: Option<String>,
        #[command(flatten)]
        kind: KindFlags,
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
    /// Trust an SFTP server: show its fingerprint and, once you agree, keep
    /// its key with the connection. Needed the first time, and again if the
    /// server's key ever changes.
    Trust {
        /// The connection name.
        name: String,
        /// Trust without asking, but only if the server's fingerprint is
        /// this one (`SHA256:…`, as `ssh-keygen -lf` prints it). For scripts.
        #[arg(long)]
        fingerprint: Option<String>,
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
            kind,
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
                kind,
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
            kind,
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
                kind,
                options,
            },
        ),
        ConnectionAction::Password { name, secret_stdin } => {
            password(journal, secrets, &name, secret_stdin)
        }
        ConnectionAction::List => list(journal),
        ConnectionAction::Test { name } => test(journal, secrets, &name),
        ConnectionAction::Trust { name, fingerprint } => {
            trust(journal, secrets, &name, fingerprint.as_deref())
        }
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
    kind: KindFlags,
    options: Vec<String>,
    secret_stdin: bool,
}

fn add(journal: &Journal, secrets: &dyn SecretStore, args: &AddArgs) -> ExitCode {
    let Some(scheme) = Scheme::parse(&args.scheme) else {
        eprintln!("error: --scheme must be {}", schemes());
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
    let mut root = args.root.clone();
    args.kind.apply(&mut parsed, &mut root, false);
    let settings = ConnectionSettings {
        scheme,
        host: args.host.clone(),
        port: args.port,
        username: args.username.clone(),
        root,
        options: parsed,
    };
    // Refused before the password prompt, so nobody types a secret into a
    // connection that was never going to work.
    if refused(&settings) {
        return ExitCode::from(2);
    }

    // Said before the password prompt, so the user can still change their mind
    // about sending it in the clear.
    if let Some(warning) = settings.in_the_clear() {
        eprintln!("warning: this connection {warning}");
    }

    // Read the password before writing the row. Storing a connection whose
    // password prompt was then cancelled would leave something half-made.
    let keyed = settings.options.contains_key(option::KEY);
    let secret = if scheme.authenticates() {
        match read_secret(&args.name, scheme, keyed, args.secret_stdin) {
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
        host: settings.host.clone(),
        port: settings.port,
        username: settings.username.clone(),
        root: settings.root.clone(),
        options: settings.options.clone(),
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
    if let Some(warning) = scheme.rootless_warning(&settings.root) {
        println!("note: {warning}");
    }
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
    kind: KindFlags,
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
            eprintln!("error: --scheme must be {}", schemes());
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

    let stored = args.root.is_none();
    let mut root = args.root.clone().unwrap_or(current.root);
    args.kind.apply(&mut options, &mut root, stored);
    let settings = ConnectionSettings {
        scheme,
        host: args.host.clone().or(current.host),
        port: args.port.or(current.port),
        username: args.username.clone().or(current.username),
        root,
        options,
    };
    if refused(&settings) {
        return ExitCode::from(2);
    }

    if let Err(error) = journal.update_connection(&args.name, &settings) {
        return fail(&error);
    }

    println!("updated connection `{}`", args.name);
    if let Some(warning) = settings.scheme.rootless_warning(&settings.root) {
        println!("note: {warning}");
    }

    // Two notes, both about a scheme change, because that is the edit whose
    // consequences are not on the screen. Neither is an error.
    if let Some(warning) = settings.in_the_clear() {
        eprintln!("warning: this connection {warning}");
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

    let keyed = connection.options.contains_key(option::KEY);
    let typed = match read_secret(name, connection.scheme, keyed, from_stdin) {
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
                let bucket = connection.options.get(option::BUCKET);
                let where_to = match (&connection.host, connection.port, bucket) {
                    (_, _, Some(bucket)) if connection.scheme == Scheme::S3 => {
                        match connection.options.get(option::ENDPOINT) {
                            Some(endpoint) => format!("bucket {bucket} at {endpoint}"),
                            None => format!("bucket {bucket}"),
                        }
                    }
                    _ if connection.scheme == Scheme::WebDav => connection
                        .options
                        .get(option::ENDPOINT)
                        .cloned()
                        .unwrap_or_else(|| "no endpoint".to_string()),
                    (Some(host), Some(port), _) => format!("{host}:{port}"),
                    (Some(host), None, _) => host.clone(),
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
                let wire = if ConnectionSettings::from(connection)
                    .in_the_clear()
                    .is_none()
                {
                    ""
                } else {
                    "  [UNENCRYPTED]"
                };
                let checked = connection.last_check.as_ref().map(|check| {
                    format!(
                        "\n    checked {}: {}",
                        when(check.at),
                        if check.ok { &check.note } else { "failed" }
                    )
                });
                println!(
                    "{}  {} {}{}  root={}{}{}",
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
                    checked.unwrap_or_default(),
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
    // Written down whichever way it went, so the window's list and `list`
    // here can say when it was last checked without asking again.
    let (code, ok, note) = check(&connection, secrets);
    if let Err(error) = journal.record_check(name, ok, &note) {
        eprintln!("warning: the result was not saved: {error}");
    }
    code
}

/// An error and its causes as one line, for a check's note.
fn sentence(error: &dyn std::error::Error) -> String {
    let mut line = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        line.push_str(": ");
        line.push_str(&next.to_string());
        cause = next.source();
    }
    line
}

/// Check a connection and say what was found. The code to exit with, whether
/// it worked, and a note to remember it by.
fn check(
    connection: &tungstate_journal::Connection,
    secrets: &dyn SecretStore,
) -> (ExitCode, bool, String) {
    let name = connection.name.as_str();

    // The root is named, not just the count. "reachable, 18 entries" is
    // reassuring and useless when the 18 entries are the server's own `/bin`
    // and `/etc` because `--root` was left at the default.
    let root = if connection.root.is_empty() {
        "/"
    } else {
        &connection.root
    };
    match tungstate_backend_opendal::probe(connection, secrets) {
        Ok(entries) => {
            println!("`{name}` is reachable");
            println!("  {root} holds {} entries", entries.len());
            // Named, not just counted. Whether those entries are your folders
            // or the server's own `/bin` is the whole question, and a number
            // cannot answer it.
            // Twenty, not a handful: a connection root is the shared-folder
            // list on most NAS boxes, and the entry you are looking for is as
            // likely to be last as first.
            for entry in entries.iter().take(20) {
                let name = entry
                    .path
                    .file_name()
                    .unwrap_or(entry.path.as_os_str())
                    .to_string_lossy();
                let mark = if entry.meta.is_dir { "/" } else { "" };
                println!("    {name}{mark}");
            }
            if entries.len() > 20 {
                println!("    and {} more", entries.len() - 20);
            }
            if let Some(warning) = connection.scheme.rootless_warning(&connection.root) {
                println!("  note: {warning}");
            }
            // Listing and writing are different permissions, and only one of
            // them is what a drain needs. Asked here so it is answered once,
            // rather than by every file in a run failing separately.
            if connection.scheme.is_networked() {
                match tungstate_backend_opendal::probe_writable(connection, secrets) {
                    Ok(true) => println!("  {root} accepts files"),
                    Ok(false) => {
                        println!("  {root} REFUSES files, so every transfer here would fail");
                        println!(
                            "  set the root to a directory this account can write to:\n    \
                             tungstate connection update {name} --root /<folder>\n  \
                             and make the link's path relative to it."
                        );
                        if matches!(connection.scheme, Scheme::Ftp | Scheme::Ftps) {
                            println!(
                                "  The root has to accept files even when nothing is meant to\n  \
                                 land there, because the FTP library writes each temporary file\n  \
                                 at the root before moving it."
                            );
                        }
                        return (
                            ExitCode::FAILURE,
                            false,
                            format!("{root} lists but refuses files"),
                        );
                    }
                    Err(error) => return (fail(&error), false, sentence(&error)),
                }
            }
            if ConnectionSettings::from(connection)
                .in_the_clear()
                .is_some()
            {
                println!("  (this connection is not encrypted)");
            }
            let things = entries.len();
            (
                ExitCode::SUCCESS,
                true,
                format!(
                    "{things} {} in {root}",
                    if things == 1 { "thing" } else { "things" }
                ),
            )
        }
        Err(error) => {
            let code = fail(&error);
            if let tungstate_backend_opendal::OpenError::Backend(
                tungstate_backend::BackendError::HostUnknown { .. }
                | tungstate_backend::BackendError::HostKeyChanged { .. },
            ) = &error
            {
                eprintln!(
                    "  check the fingerprint on the server, then: tungstate connection trust {name}"
                );
            }
            (code, false, sentence(&error))
        }
    }
}

/// Learn an SFTP server's key and keep it with the connection, once the
/// person agrees or `--fingerprint` matches.
fn trust(
    journal: &Journal,
    secrets: &dyn SecretStore,
    name: &str,
    expected: Option<&str>,
) -> ExitCode {
    use tungstate_backend::BackendError;
    use tungstate_backend_opendal::OpenError;

    let connection = match journal.connection_by_name(name) {
        Ok(connection) => connection,
        Err(error) => return fail(&error),
    };
    if connection.scheme != Scheme::Sftp {
        eprintln!(
            "error: only SFTP servers are trusted this way; `{name}` is {}",
            connection.scheme.as_str()
        );
        return ExitCode::from(2);
    }
    // With the key already kept, if any: a server that still matches it is
    // trusted, and one that does not answers with what it shows now.
    let (fingerprint, key) = match tungstate_backend_opendal::probe(&connection, secrets) {
        Err(OpenError::Backend(
            BackendError::HostUnknown {
                fingerprint, key, ..
            }
            | BackendError::HostKeyChanged {
                fingerprint, key, ..
            },
        )) => (fingerprint, key),
        Ok(_) => {
            println!("`{name}` is already trusted");
            return ExitCode::SUCCESS;
        }
        Err(error) => return fail(&error),
    };

    let was = connection.options.contains_key(option::HOST_KEY);
    let agreed = match expected {
        Some(wanted) if wanted.trim() == fingerprint => true,
        Some(wanted) => {
            eprintln!(
                "error: `{name}` shows {fingerprint}, not {}; nothing was trusted",
                wanted.trim()
            );
            return ExitCode::FAILURE;
        }
        None => ask_to_trust(name, &connection, &fingerprint, was),
    };
    if !agreed {
        println!("nothing was trusted");
        return ExitCode::FAILURE;
    }

    let mut settings = ConnectionSettings::from(&connection);
    settings.options.insert(option::HOST_KEY.to_string(), key);
    if let Err(error) = journal.update_connection(name, &settings) {
        return fail(&error);
    }
    println!("trusted `{name}` ({fingerprint})");
    println!("check it with: tungstate connection test {name}");
    ExitCode::SUCCESS
}

/// Show the fingerprint and ask. No terminal means no answer, so a script has
/// to say which fingerprint it expects instead.
fn ask_to_trust(
    name: &str,
    connection: &tungstate_journal::Connection,
    fingerprint: &str,
    replacing: bool,
) -> bool {
    use std::io::{IsTerminal, Write};

    if !std::io::stdin().is_terminal() {
        eprintln!(
            "error: `{name}` shows {fingerprint}. Check that on the server, then re-run with\n  \
             --fingerprint {fingerprint}"
        );
        return false;
    }
    let host = connection.host.as_deref().unwrap_or("the server");
    if replacing {
        println!("{host} has a different key from the one trusted before.");
        println!("That happens when a server is reset or replaced, and also when something");
        println!("is pretending to be it. Only trust it if you know which.");
    } else {
        println!("This is the first time `{name}` has reached {host}.");
    }
    println!("Its fingerprint is {fingerprint}");
    println!("On the server, `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` shows its own.");
    print!("Trust it? [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}

fn remove(journal: &Journal, secrets: &dyn SecretStore, name: &str) -> ExitCode {
    let connection = match journal.connection_by_name(name) {
        Ok(connection) => connection,
        Err(error) => return fail(&error),
    };
    // Named before anything is touched: "it is in use" is a guessing game,
    // "these two saved pairs" is something to act on.
    let uses = match journal.connection_uses(connection.id) {
        Ok(uses) => uses,
        Err(error) => return fail(&error),
    };
    if uses.blocks_removal() {
        eprintln!("error: `{name}` is still used, so it was not removed:");
        for (what, names) in [
            ("saved pairs", &uses.pairs),
            ("syncs", &uses.syncs),
            ("transfers that stopped part-way", &uses.unfinished),
        ] {
            if !names.is_empty() {
                eprintln!("  {what}: {}", names.join(", "));
            }
        }
        eprintln!("  remove those first, or finish or clear the unfinished transfers");
        return ExitCode::FAILURE;
    }

    // The row first: if something else refused, deleting the password before
    // finding that out would break a connection that is still there.
    let removal = match journal.remove_connection(name) {
        Ok(removal) => removal,
        Err(error) => return fail(&error),
    };
    if let Err(error) = secrets.delete(&connection_key(name)) {
        eprintln!("warning: `{name}` was removed, but its saved password was not: {error}");
    }
    println!("removed connection `{name}`");
    if removal == tungstate_journal::Removal::Retired {
        println!(
            "  History still names it in {} past {}, so those keep saying where\n  \
             their files went. The name is free to use again.",
            uses.history,
            if uses.history == 1 {
                "operation"
            } else {
                "operations"
            },
        );
    }
    ExitCode::SUCCESS
}

/// A moment as a date and time on this machine's clock.
fn when(millis: i64) -> String {
    jiff::Timestamp::from_millisecond(millis).map_or_else(
        |_| "at an unknown time".to_string(),
        |at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

/// Every scheme's spelling, for an error that has to list them.
fn schemes() -> String {
    let names: Vec<&str> = Scheme::ALL.iter().map(|scheme| scheme.as_str()).collect();
    names.join(", ")
}

/// Print why these settings will not work, if they will not. `true` when
/// something was printed and the command should stop.
fn refused(settings: &ConnectionSettings) -> bool {
    let problems = settings.problems();
    for problem in &problems {
        eprintln!("error: {problem}");
    }
    !problems.is_empty()
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
fn read_secret(
    name: &str,
    scheme: Scheme,
    keyed: bool,
    from_stdin: bool,
) -> std::io::Result<Option<String>> {
    if from_stdin {
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        return Ok((!trimmed.is_empty()).then(|| trimmed.to_string()));
    }
    let what = if scheme == Scheme::S3 {
        "secret key"
    } else if keyed {
        "the key's passphrase"
    } else {
        "password"
    };
    let typed = rpassword::prompt_password(format!("{what} for {name} (blank for none): "))?;
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
    fn a_share_and_a_root_make_one_path_whichever_way_they_are_given() {
        assert_eq!(with_share("media", ""), "media");
        assert_eq!(with_share("media", "backups"), "media/backups");
        assert_eq!(with_share("/media/", "/backups/2026"), "media/backups/2026");
        // Given twice, it is not doubled.
        assert_eq!(with_share("media", "media/backups"), "media/backups");
        assert_eq!(with_share("media", "media\\backups"), "media\\backups");
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
