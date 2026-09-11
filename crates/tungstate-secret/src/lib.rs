//! Where a connection's password lives: the machine's keychain, never the
//! journal.
//!
//! Its own crate rather than a module for two reasons. The CLI has to *store* a
//! secret at `connection add` time, and nothing else about that command needs
//! `OpenDAL`. And every test in the workspace needs a store that is guaranteed
//! not to touch the developer's real keychain, which means the abstraction has
//! to be importable on its own.
//!
//! The trait is the load-bearing part. Headless Linux CI has no D-Bus session
//! and therefore no Secret Service, so a test against [`KeyringStore`] would
//! fail on one of the three platforms tungstate promises. [`MemoryStore`] is
//! what the suite actually runs against.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Anything that can go wrong reaching the secret store.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The platform's credential store refused the request.
    #[error("the system keychain refused `{key}`")]
    Keyring {
        /// Which entry was being read or written.
        key: String,
        /// The underlying failure.
        #[source]
        source: keyring::Error,
    },
}

/// Result alias so signatures read `Result<Option<String>>`.
pub type Result<T> = std::result::Result<T, SecretError>;

/// The service name every tungstate entry is filed under.
const SERVICE: &str = "tungstate";

/// The keychain account a connection's password is stored as.
///
/// Namespaced so a later kind of secret — a node pairing token, say — cannot
/// collide with a connection that happens to share its name.
#[must_use]
pub fn connection_key(name: &str) -> String {
    format!("connection/{name}")
}

/// Somewhere a password can be kept that is not the journal.
pub trait SecretStore: Send + Sync {
    /// Read a secret, or `None` if nothing is stored under `key`.
    ///
    /// # Errors
    /// [`SecretError::Keyring`] if the store could not be reached. A missing
    /// entry is `Ok(None)`, not an error: "no password saved" is an answer.
    fn get(&self, key: &str) -> Result<Option<String>>;

    /// Store a secret, replacing anything already under `key`.
    ///
    /// # Errors
    /// [`SecretError::Keyring`] if the store refuses the write.
    fn set(&self, key: &str, secret: &str) -> Result<()>;

    /// Remove whatever is under `key`. Removing nothing is not an error.
    ///
    /// # Errors
    /// [`SecretError::Keyring`] if the store refuses the delete.
    fn delete(&self, key: &str) -> Result<()>;
}

/// The real thing: macOS Keychain, Windows Credential Manager, Secret Service.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    /// A store backed by this machine's credential manager.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).map_err(|source| SecretError::Keyring {
        key: key.to_string(),
        source,
    })
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(source) => Err(SecretError::Keyring {
                key: key.to_string(),
                source,
            }),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<()> {
        entry(key)?
            .set_password(secret)
            .map_err(|source| SecretError::Keyring {
                key: key.to_string(),
                source,
            })
    }

    fn delete(&self, key: &str) -> Result<()> {
        match entry(key)?.delete_credential() {
            // Already gone is the outcome the caller wanted.
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(source) => Err(SecretError::Keyring {
                key: key.to_string(),
                source,
            }),
        }
    }
}

/// Reads a password from the environment before asking the inner store.
///
/// `TUNGSTATE_SECRET_<NAME>`, where `<NAME>` is the connection name uppercased
/// with `-` turned into `_`. So connection `nas-attic` is `TUNGSTATE_SECRET_NAS_ATTIC`.
///
/// This exists for machines with no keychain to ask. DESIGN.md §6b puts a
/// tungstate daemon on the NAS itself, and a container or a systemd unit has
/// no Secret Service, no Keychain and nobody to type at a prompt; passing a
/// secret by environment is how such things are normally fed. It is also what
/// lets the FTP integration tests give the binary a password without touching
/// the developer's real keychain.
///
/// Writes always go to the inner store. A process cannot put a secret into a
/// future process's environment, so pretending `set` succeeded would be a lie.
#[derive(Debug, Default, Clone)]
pub struct EnvOverride<S>(pub S);

/// The environment variable a connection's password can be passed in.
#[must_use]
pub fn env_var_for(key: &str) -> String {
    let name = key.strip_prefix("connection/").unwrap_or(key);
    format!("TUNGSTATE_SECRET_{}", name.to_uppercase().replace('-', "_"))
}

impl<S: SecretStore> SecretStore for EnvOverride<S> {
    fn get(&self, key: &str) -> Result<Option<String>> {
        // An empty variable means "no password", not "fall through". Someone
        // who sets it to empty on purpose gets an anonymous login rather than
        // a surprise credential from the keychain.
        if let Ok(secret) = std::env::var(env_var_for(key)) {
            return Ok(Some(secret));
        }
        self.0.get(key)
    }

    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.0.set(key, secret)
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.0.delete(key)
    }
}

/// A store that lives and dies with the process, for tests.
#[derive(Debug, Default)]
pub struct MemoryStore {
    // `&self` methods on a shared store, so the map needs interior mutability.
    // A poisoned lock only means some other test panicked; recovering keeps one
    // failure from cascading into every later assertion.
    entries: Mutex<BTreeMap<String, String>>,
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.entries().get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.entries().insert(key.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.entries().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_store_round_trips_a_secret() {
        let store = MemoryStore::new();
        let key = connection_key("nas");

        assert_eq!(store.get(&key).unwrap(), None, "nothing stored yet");
        store.set(&key, "hunter2").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("hunter2"));

        store.set(&key, "hunter3").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("hunter3"));

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();
    }

    #[test]
    fn connection_keys_are_namespaced() {
        assert_eq!(connection_key("nas"), "connection/nas");
    }

    #[test]
    fn a_store_can_be_shared_across_threads() {
        // `&dyn SecretStore` has to be `Sync` for the backend factory to take
        // one; this is that bound exercised rather than merely declared.
        let store = MemoryStore::new();
        let shared: &dyn SecretStore = &store;
        std::thread::scope(|scope| {
            for n in 0..4 {
                scope.spawn(move || {
                    let key = format!("connection/c{n}");
                    shared.set(&key, "s").unwrap();
                    assert_eq!(shared.get(&key).unwrap().as_deref(), Some("s"));
                });
            }
        });
    }

    #[test]
    fn an_environment_variable_names_its_connection() {
        assert_eq!(env_var_for("connection/nas"), "TUNGSTATE_SECRET_NAS");
        // A hyphen is legal in a connection name and illegal in most shells'
        // variable names, so it has to become an underscore.
        assert_eq!(
            env_var_for("connection/nas-attic"),
            "TUNGSTATE_SECRET_NAS_ATTIC"
        );
    }

    #[test]
    fn an_override_with_nothing_set_behaves_exactly_like_the_inner_store() {
        // The other half — that the environment wins — cannot be tested here:
        // `std::env::set_var` is `unsafe`, and the workspace forbids `unsafe`
        // outright. It is proved end to end instead, in the FTP integration
        // tests, where the parent sets the variable on the child process.
        // That is the more honest test anyway, since the feature exists for
        // exactly that shape: one process configuring another.
        let store = EnvOverride(MemoryStore::new());
        let key = connection_key("envtest-delegation");

        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "hunter2").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("hunter2"));
        assert_eq!(
            store.0.get(&key).unwrap().as_deref(),
            Some("hunter2"),
            "a write must reach the inner store, not be swallowed"
        );
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }

    /// Run by hand on a real desktop: `cargo test -p tungstate-secret -- --ignored`.
    ///
    /// Not in the suite. Headless Linux CI has no Secret Service, so this would
    /// fail on one of the three platforms we promise, and on a developer's Mac
    /// it prompts for keychain access.
    #[test]
    #[ignore = "touches the machine's real keychain"]
    fn a_keyring_store_round_trips_a_secret() {
        let store = KeyringStore::new();
        let key = connection_key("tungstate-selftest");

        store.set(&key, "hunter2").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("hunter2"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }
}
