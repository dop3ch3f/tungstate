use std::path::Path;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{
    Access, Asset, Categories, Category, CategoryArgs, Filter, Insets, Order, PathsArgs, Removed,
    Result, StoredFile, Summary,
};

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

#[derive(Deserialize)]
struct Files {
    files: Vec<StoredFile>,
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

#[derive(Serialize)]
struct Shrink<'a> {
    path: &'a str,
    size: u32,
    out: std::borrow::Cow<'a, str>,
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

    /// Whether the app may read and change all of shared storage, which a
    /// file manager needs ("All files access" on Android 11 and later).
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn files_access(&self) -> Result<Access> {
        Ok(self
            .0
            .run_mobile_plugin::<Answer>("filesAccess", ())?
            .access)
    }

    /// Opens the system's own page for All files access, and answers once
    /// the person is back from it.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn ask_files(&self) -> Result<Access> {
        Ok(self.0.run_mobile_plugin::<Answer>("askFiles", ())?.access)
    }

    /// Counts and sizes for the category tiles.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn categories(&self) -> Result<Categories> {
        Ok(self.0.run_mobile_plugin("categories", ())?)
    }

    /// Every file in one category, by its real path, or only the `first`
    /// few in `order`: a big category takes seconds to list in full, and its
    /// first screenful can be drawn long before.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn category_files(
        &self,
        category: Category,
        first: Option<u32>,
        order: Order,
    ) -> Result<Vec<StoredFile>> {
        let args = CategoryArgs {
            category,
            first,
            order,
        };
        Ok(self
            .0
            .run_mobile_plugin::<Files>("categoryFiles", args)?
            .files)
    }

    /// Offers these files to other apps through the system's share sheet.
    ///
    /// # Errors
    /// The phone's code failed, or a file is outside shared storage.
    pub fn share(&self, paths: Vec<String>) -> Result<()> {
        self.0
            .run_mobile_plugin::<()>("share", PathsArgs { paths })?;
        Ok(())
    }

    /// Opens one file in the app the phone uses for its kind.
    ///
    /// # Errors
    /// The phone's code failed, or no app opens files of this kind.
    pub fn open(&self, path: String) -> Result<()> {
        self.0
            .run_mobile_plugin::<()>("open", PathsArgs { paths: vec![path] })?;
        Ok(())
    }

    /// Holds Wi-Fi multicast open while on, so mDNS answers reach the app.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn multicast(&self, on: bool) -> Result<()> {
        self.0.run_mobile_plugin::<()>("multicast", Awake { on })?;
        Ok(())
    }

    /// The status and navigation bars' heights.
    ///
    /// # Errors
    /// The phone's code failed.
    pub fn insets(&self) -> Result<Insets> {
        Ok(self.0.run_mobile_plugin("insets", ())?)
    }

    /// Writes a JPEG of `path` no bigger than `size` on its long side to
    /// `out`, using the phone's own decoders: photos (HEIC too), a video's
    /// frame, an app installer's icon, a song's cover.
    ///
    /// # Errors
    /// The phone's code failed, or it has no picture for this kind of file.
    pub fn thumbnail(&self, path: &str, size: u32, out: &Path) -> Result<()> {
        let out = out.to_string_lossy();
        self.0
            .run_mobile_plugin::<()>("thumbnail", Shrink { path, size, out })?;
        Ok(())
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
