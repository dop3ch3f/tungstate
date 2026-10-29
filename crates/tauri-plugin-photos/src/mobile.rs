use serde::{Deserialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{Access, Filter, Result, Summary};

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
}
