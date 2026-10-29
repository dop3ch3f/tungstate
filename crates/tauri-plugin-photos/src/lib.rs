//! A phone's photo library, for Tungstate.
//!
//! What is there, how big and how old, and whether the person has let the
//! app look. Reading only, for now: copying photos out and removing them
//! after they are safe on a connection come with the rest of Free up space.
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
use desktop::Photos;
#[cfg(mobile)]
use mobile::Photos;

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
