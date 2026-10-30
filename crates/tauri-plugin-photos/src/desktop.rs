use std::path::Path;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{Access, Asset, Error, Filter, Removed, Result, Summary};

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Photos<R> {
    Photos(app.clone())
}

/// A desktop's stand-in: every call says there is no library here.
pub struct Photos<R: Runtime>(#[allow(dead_code)] AppHandle<R>);

#[allow(clippy::unused_self)] // the phone's methods take `&self`; callers must not care which
impl<R: Runtime> Photos<R> {
    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn access(&self) -> Result<Access> {
        Err(Error::NoLibrary)
    }

    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn ask(&self) -> Result<Access> {
        Err(Error::NoLibrary)
    }

    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn summary(&self, _filter: Filter) -> Result<Summary> {
        Err(Error::NoLibrary)
    }

    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn list(&self, _filter: Filter) -> Result<Vec<Asset>> {
        Err(Error::NoLibrary)
    }

    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn export(&self, _id: &str, _to: &Path) -> Result<()> {
        Err(Error::NoLibrary)
    }

    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn remove(&self, _ids: &[String]) -> Result<Removed> {
        Err(Error::NoLibrary)
    }

    /// # Errors
    /// Always [`Error::NoLibrary`].
    pub fn keep_awake(&self, _on: bool) -> Result<()> {
        Err(Error::NoLibrary)
    }
}
