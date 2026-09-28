//! Remote [`Backend`]s via Apache `OpenDAL`, and the factory that turns a link
//! end into one.
//!
//! Named as DESIGN.md §7 specifies. Two things live here:
//!
//! - [`OpendalBackend`], the adapter from `OpenDAL`'s async `Operator` onto
//!   tungstate's synchronous [`Backend`] trait. See `runtime` for how, and why
//!   it is not `opendal::blocking`.
//! - [`open`], which resolves an [`Endpoint`] to a `Box<dyn Backend>`. A `None`
//!   connection is the local filesystem and gets a [`LocalBackend`]; everything
//!   else gets an operator. This is the single place in the workspace that
//!   knows a link end might not be local.
//!
//! **Services registered in this slice: `fs` and `memory`.** `fs` is not a
//! curiosity: it is how the adapter is exercised on macOS, Windows and Linux
//! with no server and no network, and it answers "does this behave identically
//! to [`LocalBackend`]?" directly. FTP arrives in slice 4c as one more flag,
//! and S3 in the polish pass as another.
//!
//! SMB is the exception to the name: it is not an `OpenDAL` service, and has
//! a crate of its own, `tungstate-backend-smb`. [`open`] still hands it out,
//! because this remains the one place that turns a connection into a backend.

mod backend;
mod keys;
mod runtime;

pub use backend::{Anchor, OpendalBackend};
pub use keys::remote_key;

use std::path::PathBuf;

use opendal::Operator;
use tungstate_backend::local::LocalBackend;
use tungstate_backend::{Backend, BackendError};
use tungstate_journal::{Connection, Endpoint, Journal, JournalError, Scheme};
use tungstate_secret::SecretStore;

/// Anything that can stop a link end from becoming a usable backend.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The connection could not be read out of the journal.
    #[error("could not look up the connection for this link end")]
    Journal(#[from] JournalError),

    /// A local root is missing, so building an operator on it would create it.
    ///
    /// The most important refusal in this file. `OpenDAL`'s `fs` builder creates
    /// its root directory if it does not exist, which is precisely how an
    /// unmounted NAS becomes an empty folder on the boot disk that a drain then
    /// fills while deleting the originals.
    #[error("`{root}` is not reachable, or is not a directory")]
    RootUnreachable {
        /// The directory that was expected to be there.
        root: PathBuf,
    },

    /// `OpenDAL` refused to build an operator for this connection.
    #[error("could not open connection `{name}`")]
    Opendal {
        /// The connection that could not be opened.
        name: String,
        /// The underlying failure.
        #[source]
        source: opendal::Error,
    },

    /// The link end's path could not be expressed as a key.
    #[error("`{path}` is not a usable path inside a connection")]
    EndPath {
        /// The offending path.
        path: PathBuf,
        /// Why it was refused.
        #[source]
        source: tungstate_backend::BackendError,
    },

    /// The connection was opened, but talking to it failed.
    #[error("could not read from this connection")]
    Backend(#[from] BackendError),

    /// The machine's keychain could not be reached.
    #[error("could not read the saved password for `{name}`")]
    Secret {
        /// The connection whose password was wanted.
        name: String,
        /// The underlying failure.
        #[source]
        source: tungstate_secret::SecretError,
    },

    /// A networked connection was recorded without a host to connect to.
    #[error("connection `{name}` has no host; set one with --host")]
    MissingHost {
        /// The connection that is missing one.
        name: String,
    },

    /// An S3 connection was recorded without a bucket.
    #[error("connection `{name}` has no bucket; set one with --bucket")]
    MissingBucket {
        /// The connection that is missing one.
        name: String,
    },

    /// The connection names a scheme this build has no service for.
    #[error("connection `{name}` speaks `{scheme}`, which this build cannot open")]
    SchemeNotCompiled {
        /// The connection.
        name: String,
        /// The scheme it asked for.
        scheme: &'static str,
    },
}

/// Result alias so signatures read `Result<Box<dyn Backend>>`.
pub type Result<T> = std::result::Result<T, OpenError>;

/// Turn one end of a link into something the transfer engine can drive.
///
/// This replaces the five `LocalBackend::new(...)` sites the CLI and GUI used
/// to have, and is fallible where they were not: reaching a remote can fail,
/// and pretending otherwise would defer the failure to the first file.
///
/// # Errors
/// [`OpenError`] if the connection cannot be read, its password cannot be
/// found, its root is not there, or `OpenDAL` refuses to build an operator.
pub fn open(
    endpoint: &Endpoint,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<Box<dyn Backend>> {
    let Some(id) = endpoint.connection else {
        return Ok(Box::new(LocalBackend::new(endpoint.path.clone())));
    };

    let connection = journal.connection_by_id(id)?;
    #[cfg(feature = "smb")]
    if connection.scheme == Scheme::Smb {
        return smb_backend(&connection, &endpoint.path, secrets);
    }
    let (operator, anchor) = operator_for(&connection, secrets)?;
    let prefix = remote_key(&endpoint.path).map_err(|source| OpenError::EndPath {
        path: endpoint.path.clone(),
        source,
    })?;
    // A folder reached through the local-filesystem adapter is on this
    // machine; everything else is a network away.
    let networked = !matches!(connection.scheme, Scheme::Fs);
    Ok(Box::new(OpendalBackend::new(
        operator,
        prefix,
        anchor,
        connection.name,
        networked,
    )))
}

/// Confirm a connection is usable, without transferring anything.
///
/// What `tungstate connection test` runs. Deliberately a listing of the root
/// rather than a write: proving we can reach a NAS must not leave a file in it.
///
/// Returns what is in the root rather than just how much, because a count on
/// its own cannot answer the question people actually have. "reachable, 10
/// entries" reads as success whether those ten are the folders you meant or
/// the server's own `/bin` and `/etc` because the connection has no root.
///
/// # Errors
/// [`OpenError`] as [`open`], or if the root cannot be listed.
pub fn probe(
    connection: &Connection,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<Vec<tungstate_backend::Entry>> {
    let endpoint = Endpoint::remote(connection.id, PathBuf::new());
    let backend = open(&endpoint, journal, secrets)?;
    match backend.read_dir(std::path::Path::new("")) {
        Ok(entries) => Ok(entries),
        Err(error) => Err(OpenError::Backend(classify(connection, error))),
    }
}

/// Whether the connection's root will accept a file, answered by writing one
/// and taking it away again.
///
/// [`probe`] deliberately only lists, on the principle that proving you can
/// reach a NAS must not leave anything in it. This is the other half of the
/// question and it cannot be answered by reading, because listing a directory
/// and writing to it are different permissions — and on a Synology the root
/// is a virtual list of shared folders that lists perfectly and accepts
/// nothing.
///
/// It has to be the *root* rather than the folder being drained into, because
/// `OpenDAL`'s FTP writer puts its temporary file at the root whatever the
/// destination is (see `create_write`). A root that refuses writes fails every
/// file in a run, one at a time, with an error that names neither the root nor
/// the reason.
///
/// The probe file is named so a leftover is obviously ours and obviously junk,
/// and it is removed whether or not the write succeeded.
///
/// # Errors
/// [`OpenError`] if the connection cannot be opened at all. A refused write is
/// `Ok(false)`, not an error: it is an answer.
pub fn probe_writable(
    connection: &Connection,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<bool> {
    use std::io::Write as _;

    let endpoint = Endpoint::remote(connection.id, PathBuf::new());
    let backend = open(&endpoint, journal, secrets)?;
    let name = PathBuf::from(format!(".tungstate-writable-{}", std::process::id()));

    let wrote = backend
        .create_write(&name)
        .and_then(|mut sink| {
            sink.write_all(b"tungstate")
                .map_err(|source| tungstate_backend::BackendError::Io {
                    path: name.clone(),
                    source,
                })?;
            sink.finish()
        })
        .is_ok();

    // Unconditional: a write that failed at `finish` may still have left
    // something behind, and this must never be the thing that litters.
    let _ = backend.remove_file(&name);
    Ok(wrote)
}

/// Turn a failed first contact into the most useful error we can justify.
///
/// `OpenDAL` does not classify a rejected FTP login: `format_ftp_error` maps
/// everything that is not 421 or 550 to `ErrorKind::Unexpected`, so the only
/// signal that the password was wrong is the server's own reply text.
///
/// So this reads the reply for FTP's "not logged in" status, which is fixed by
/// RFC 959 and is not an `OpenDAL` detail. It is still string-matching, and it is
/// contained here for that reason. The failure mode is benign: if the wording
/// ever changes we fall through to the unclassified error, which is exactly
/// what the user would have got anyway. The real fix is upstream — `OpenDAL`
/// reporting `PermissionDenied` — and is worth a patch.
fn classify(connection: &Connection, error: tungstate_backend::BackendError) -> BackendError {
    const FTP_NOT_LOGGED_IN: &str = "530";
    // S3 names a wrong key id or a wrong secret in its error code. These two
    // are about the credentials alone; `AccessDenied` is not, because a right
    // key can lack permission on one bucket.
    const S3_BAD_KEY: [&str; 2] = ["InvalidAccessKeyId", "SignatureDoesNotMatch"];

    if !connection.scheme.authenticates() {
        return error;
    }

    let mut rendered = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }

    let refused = match connection.scheme {
        Scheme::S3 => S3_BAD_KEY.iter().any(|code| rendered.contains(code)),
        _ => rendered.contains(FTP_NOT_LOGGED_IN),
    };
    if refused {
        return BackendError::Auth {
            endpoint: connection.name.clone(),
        };
    }
    error
}

fn operator_for(connection: &Connection, secrets: &dyn SecretStore) -> Result<(Operator, Anchor)> {
    match connection.scheme {
        Scheme::Fs => filesystem_operator(connection),

        #[cfg(feature = "ftp")]
        Scheme::Ftp | Scheme::Ftps => ftp_operator(connection, secrets),

        #[cfg(feature = "s3")]
        Scheme::S3 => s3_operator(connection, secrets),

        // Named rather than silently unmatched, so a build without the feature
        // says what is wrong instead of failing somewhere obscure.
        #[cfg(not(feature = "ftp"))]
        Scheme::Ftp | Scheme::Ftps => not_compiled(connection, secrets),
        #[cfg(not(feature = "s3"))]
        Scheme::S3 => not_compiled(connection, secrets),

        // Not an OpenDAL service: `open` hands SMB to its own crate before
        // asking for an operator, so arriving here is a caller's mistake.
        Scheme::Smb => not_compiled(connection, secrets),
    }
}

/// A share reached directly, signed in to before this returns.
#[cfg(feature = "smb")]
fn smb_backend(
    connection: &Connection,
    link_end: &std::path::Path,
    secrets: &dyn SecretStore,
) -> Result<Box<dyn Backend>> {
    let Some(host) = connection.host.as_deref().filter(|h| !h.trim().is_empty()) else {
        return Err(OpenError::MissingHost {
            name: connection.name.clone(),
        });
    };
    let settings = tungstate_backend_smb::Settings {
        host: host.trim().to_string(),
        port: connection.port,
        root: connection.root.clone(),
        username: connection.username.clone().unwrap_or_default(),
        password: secret_for(connection, secrets)?.unwrap_or_default(),
        require_encryption: connection
            .options
            .get(tungstate_journal::option::ENCRYPTION)
            .is_some_and(|value| value.trim() == "required"),
    };
    let backend =
        tungstate_backend_smb::SmbBackend::connect(&settings, link_end, &connection.name)?;
    Ok(Box::new(backend))
}

/// A connection whose scheme this build has no operator for.
#[allow(clippy::unnecessary_wraps)]
fn not_compiled(connection: &Connection, _secrets: &dyn SecretStore) -> Result<(Operator, Anchor)> {
    Err(OpenError::SchemeNotCompiled {
        name: connection.name.clone(),
        scheme: connection.scheme.as_str(),
    })
}

/// Give `OpenDAL` an HTTPS client, once per process.
///
/// Its own before-`main` hook for this is behind default features the
/// workspace turns off, and would bring a second cryptography library. Both
/// installs are first-wins, so later calls, and a provider some other part of
/// the app installed first, are fine.
#[cfg(feature = "s3")]
fn install_https() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        match reqwest::Client::builder().build() {
            Ok(client) => opendal::HttpTransporter::install_default(
                opendal_http_transport_reqwest::ReqwestTransport::new(client),
            ),
            // Left uninstalled, the first request says so in its own error.
            Err(error) => tracing::warn!(%error, "could not build an HTTPS client"),
        }
    });
}

/// An operator over an S3 bucket, or anything that speaks S3.
///
/// Configuration comes from the connection and the keychain only. `OpenDAL`
/// would otherwise also read `~/.aws`, the environment and the EC2 metadata
/// service, and a connection that quietly worked because of some other
/// program's credentials would stop working on the next machine.
#[cfg(feature = "s3")]
fn s3_operator(connection: &Connection, secrets: &dyn SecretStore) -> Result<(Operator, Anchor)> {
    use tungstate_journal::option;

    install_https();

    let setting = |key: &str| {
        connection
            .options
            .get(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let Some(bucket) = setting(option::BUCKET) else {
        return Err(OpenError::MissingBucket {
            name: connection.name.clone(),
        });
    };
    let endpoint = setting(option::ENDPOINT);
    // AWS itself needs the bucket's real region, which `--region` gives. Any
    // other service is asked, and most accept anything when it cannot say.
    let region = match (setting(option::REGION), &endpoint) {
        (Some(region), _) => region,
        (None, Some(endpoint)) => {
            let (endpoint, bucket) = (endpoint.clone(), bucket.clone());
            runtime::dispatch(async move {
                opendal::services::S3::detect_region(&endpoint, &bucket).await
            })
            .unwrap_or_else(|| "us-east-1".to_string())
        }
        (None, None) => "us-east-1".to_string(),
    };

    let mut builder = opendal::services::S3::default()
        .bucket(&bucket)
        .region(&region)
        .root(&connection.root)
        .disable_config_load()
        .disable_ec2_metadata();
    if let Some(endpoint) = &endpoint {
        builder = builder.endpoint(endpoint);
    }
    if let Some(key) = connection.username.as_deref() {
        builder = builder.access_key_id(key);
    }
    if let Some(secret) = secret_for(connection, secrets)? {
        builder = builder.secret_access_key(&secret);
    }

    let operator = Operator::new(builder).map_err(|source| OpenError::Opendal {
        name: connection.name.clone(),
        source,
    })?;
    Ok((operator, Anchor::Store))
}

/// An operator over an FTP or FTPS server.
///
/// The password is read from the keychain here and handed straight to `OpenDAL`;
/// it is never written down on the way past.
#[cfg(feature = "ftp")]
fn ftp_operator(connection: &Connection, secrets: &dyn SecretStore) -> Result<(Operator, Anchor)> {
    let Some(host) = connection.host.as_deref().filter(|h| !h.is_empty()) else {
        return Err(OpenError::MissingHost {
            name: connection.name.clone(),
        });
    };
    let port = connection
        .port
        .or_else(|| connection.scheme.default_port())
        .unwrap_or(21);

    // OpenDAL picks TLS off the endpoint's scheme rather than from a flag, so
    // this one string is the whole of the `ftp` versus `ftps` difference.
    let endpoint = format!("{}://{host}:{port}", connection.scheme.as_str());

    let mut builder = opendal::services::Ftp::default()
        .endpoint(&endpoint)
        .root(&connection.root);
    if let Some(user) = connection.username.as_deref() {
        builder = builder.user(user);
    }
    if let Some(secret) = secret_for(connection, secrets)? {
        builder = builder.password(&secret);
    }

    let operator = Operator::new(builder).map_err(|source| OpenError::Opendal {
        name: connection.name.clone(),
        source,
    })?;
    Ok((operator, Anchor::Store))
}

/// A connection's saved password, if it has one.
///
/// `None` is not an error: an anonymous FTP server is a real thing, and a
/// rejected login is something the server reports rather than something to
/// guess at here.
#[cfg(any(feature = "ftp", feature = "s3", feature = "smb"))]
fn secret_for(connection: &Connection, secrets: &dyn SecretStore) -> Result<Option<String>> {
    secrets
        .get(&tungstate_secret::connection_key(&connection.name))
        .map_err(|source| OpenError::Secret {
            name: connection.name.clone(),
            source,
        })
}

/// An operator over a directory this machine can already reach.
///
/// Rooted at the connection, never at the link end. A link end inside it may
/// not exist yet — creating `inbox/` on first write is ordinary — but the
/// connection itself must be there, and that is the safety rail.
fn filesystem_operator(connection: &Connection) -> Result<(Operator, Anchor)> {
    let root = PathBuf::from(&connection.root);

    // Checked before OpenDAL sees it. Its `fs` builder creates a missing root,
    // and "the mount point is gone" must never become "here is a new empty
    // folder on the boot disk" that a drain then fills.
    if !root.is_dir() {
        return Err(OpenError::RootUnreachable { root });
    }

    let builder = opendal::services::Fs::default().root(&root.to_string_lossy());
    let operator = Operator::new(builder).map_err(|source| OpenError::Opendal {
        name: connection.name.clone(),
        source,
    })?;
    // This connection is a directory on this machine, so reachability is one
    // `stat` rather than a listing.
    Ok((operator, Anchor::LocalDir(root)))
}

#[cfg(test)]
mod tests;
