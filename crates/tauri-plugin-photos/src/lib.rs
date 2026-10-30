//! A phone's photo library, for Tungstate.
//!
//! What is there, how big and how old, and whether the person has let the
//! app look; copying a photo out into a file; and asking, through the
//! system's own prompt, to remove photos once they are safe elsewhere.
//!
//! Only counting and asking for access are open to the window. Listing,
//! copying out and removing are for the app's Rust, which decides what is
//! safe to remove; nothing the window says can widen that.
//!
//! The work is done in Kotlin (`android/`) and Swift (`ios/`); this crate
//! forwards to them. On desktop every call says there is no photo library.

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub use models::*;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

pub use error::{Error, Result};

#[cfg(desktop)]
pub use desktop::Photos;
#[cfg(mobile)]
pub use mobile::Photos;

/// `app.photos()`, from anything that can reach the app's state.
pub trait PhotosExt<R: Runtime> {
    fn photos(&self) -> &Photos<R>;
}

impl<R: Runtime, T: Manager<R>> PhotosExt<R> for T {
    fn photos(&self) -> &Photos<R> {
        self.state::<Photos<R>>().inner()
    }
}

/// The plugin, to add to the app's builder.
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("photos")
        .invoke_handler(tauri::generate_handler![
            commands::access,
            commands::ask,
            commands::summary
        ])
        .setup(|app, api| {
            #[cfg(mobile)]
            let photos = mobile::init(app, api)?;
            #[cfg(desktop)]
            let photos = desktop::init(app, api);
            app.manage(photos);
            Ok(())
        })
        .build()
}
