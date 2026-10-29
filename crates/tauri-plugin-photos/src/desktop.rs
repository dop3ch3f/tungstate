use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{Access, Error, Filter, Result, Summary};

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
}
