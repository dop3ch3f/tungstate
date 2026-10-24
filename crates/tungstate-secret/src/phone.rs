//! The phone's own credential store, chosen by hand.
//!
//! `keyring`'s all-in-one `Entry` picks a store on desktop only, so on iOS and
//! Android the store is installed as `keyring-core`'s default here, once. The
//! wrapper has the same shape as `keyring::Entry`, so `lib.rs` reads the same.

use std::sync::{Arc, LazyLock};

use keyring_core::{CredentialStore, Error, Result};

/// Whether the platform store opened. Tried once, on first use.
static READY: LazyLock<bool> = LazyLock::new(|| match open() {
    Ok(store) => {
        keyring_core::set_default_store(store);
        true
    }
    Err(_) => false,
});

// iOS's data-protection keychain ("protected"), the only one an app may use there.
#[cfg(target_os = "ios")]
fn open() -> Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::protected::Store::new()?)
}

// Needs ndk-context initialised, which Tauri's Android runtime does at startup.
#[cfg(target_os = "android")]
fn open() -> Result<Arc<CredentialStore>> {
    Ok(android_native_keyring_store::Store::new()?)
}

/// One named secret in the phone's store.
pub struct Entry(keyring_core::Entry);

impl Entry {
    /// As `keyring::Entry::new`: a store that would not open is `NoDefaultStore`.
    pub fn new(service: &str, user: &str) -> Result<Self> {
        if !*READY {
            return Err(Error::NoDefaultStore);
        }
        keyring_core::Entry::new(service, user).map(Self)
    }

    pub fn get_password(&self) -> Result<String> {
        self.0.get_password()
    }

    pub fn set_password(&self, password: &str) -> Result<()> {
        self.0.set_password(password)
    }

    pub fn delete_credential(&self) -> Result<()> {
        self.0.delete_credential()
    }
}
