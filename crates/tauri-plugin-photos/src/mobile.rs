use std::path::Path;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{Access, Asset, Filter, Removed, Result, Summary};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_photos);

/// Registers the Kotlin or Swift side.
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<Photos<R>> {
    #[cfg(target_os = "android")]
    let handle =
        api.register_android_plugin("com.ifeanyiibekie.tungstate.photos", "PhotosPlugin")?;
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_photos)?;
    Ok(Photos(handle))
}

/// The photo library, through the phone's own code.
pub struct Photos<R: Runtime>(PluginHandle<R>);

/// Kotlin and Swift answer with an object; the access is its one field.
#[derive(Deserialize)]
struct Answer {
    access: Access,
}

#[derive(Deserialize)]
struct Assets {
    assets: Vec<Asset>,
}

#[derive(Serialize)]
struct Export<'a> {
    id: &'a str,
    path: std::borrow::Cow<'a, str>,
}

#[derive(Serialize)]
struct Ids<'a> {
    ids: &'a [String],
}

#[derive(Serialize)]
struct Awake {
    on: bool,
}

impl<R: Runtime> Photos<R> {
    /// # Errors
    /// The phone's code failed.
    pub fn access(&self) -> Result<Access> {
        Ok(self.0.run_mobile_plugin::<Answer>("access", ())?.access)
    }

    /// Shows the system's own question, and answers once it is answered.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn ask(&self) -> Result<Access> {
        Ok(self.0.run_mobile_plugin::<Answer>("ask", ())?.access)
    }

    /// # Errors
    /// The phone's code failed, or access was not given.
    pub fn summary(&self, filter: Filter) -> Result<Summary> {
        Ok(self.0.run_mobile_plugin("summary", filter)?)
    }

    /// Every photo and video the filter covers, favourites included.
    ///
    /// # Errors
    /// The phone's code failed, or access was not given.
    pub fn list(&self, filter: Filter) -> Result<Vec<Asset>> {
        Ok(self.0.run_mobile_plugin::<Assets>("list", filter)?.assets)
    }

    /// Copies one photo or video, untouched, into a file at `to`.
    ///
    /// # Errors
    /// The phone's code failed: the photo is gone, unreadable, or the file
    /// could not be written.
    pub fn export(&self, id: &str, to: &Path) -> Result<()> {
        let path = to.to_string_lossy();
        self.0
            .run_mobile_plugin::<()>("export", Export { id, path })?;
        Ok(())
    }

    /// Asks the person, through the system's own prompt, to remove these.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn remove(&self, ids: &[String]) -> Result<Removed> {
        Ok(self.0.run_mobile_plugin("remove", Ids { ids })?)
    }

    /// Keeps the screen on, or lets it sleep again.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn keep_awake(&self, on: bool) -> Result<()> {
        self.0.run_mobile_plugin::<()>("keepAwake", Awake { on })?;
        Ok(())
    }
}
