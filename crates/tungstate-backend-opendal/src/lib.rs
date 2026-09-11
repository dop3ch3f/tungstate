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
//! to [`LocalBackend`]?" directly. FTP arrives in slice 4c as one more flag.

mod backend;
mod keys;
mod runtime;

pub use backend::{Anchor, OpendalBackend};
pub use keys::remote_key;

use std::path::PathBuf;

use opendal::Operator;
use tungstate_backend::Backend;
use tungstate_backend::local::LocalBackend;
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
    let (operator, anchor) = operator_for(&connection, secrets)?;
    let prefix = remote_key(&endpoint.path).map_err(|source| OpenError::EndPath {
        path: endpoint.path.clone(),
        source,
    })?;
    Ok(Box::new(OpendalBackend::new(
        operator,
        prefix,
        anchor,
        connection.name,
    )))
}

/// Confirm a connection is usable, without transferring anything.
///
/// What `tungstate connection test` runs. Deliberately a listing of the root
/// rather than a write: proving we can reach a NAS must not leave a file in it.
///
/// # Errors
/// [`OpenError`] as [`open`], or if the root cannot be listed.
pub fn probe(
    connection: &Connection,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<usize> {
    let endpoint = Endpoint::remote(connection.id, PathBuf::new());
    let backend = open(&endpoint, journal, secrets)?;
    backend
        .read_dir(std::path::Path::new(""))
        .map(|entries| entries.len())
        .map_err(|source| OpenError::Opendal {
            name: connection.name.clone(),
            source: opendal::Error::new(opendal::ErrorKind::Unexpected, source.to_string()),
        })
}

fn operator_for(connection: &Connection, secrets: &dyn SecretStore) -> Result<(Operator, Anchor)> {
    match connection.scheme {
        Scheme::Fs => filesystem_operator(connection),
        // Registered in slice 4c. Named rather than silently unmatched so the
        // error tells the user what to do about it. `secrets` is already
        // threaded this far so that registering the service, and reading the
        // password it needs, is the only change that arm will need.
        Scheme::Ftp => {
            let _ = secrets;
            Err(OpenError::SchemeNotCompiled {
                name: connection.name.clone(),
                scheme: connection.scheme.as_str(),
            })
        }
    }
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
