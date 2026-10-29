// Async so the phone's answer is waited for off the window's own thread:
// counting a large library takes a moment, and `ask` waits on a person.

use tauri::{AppHandle, Runtime, command};

use crate::{Access, Filter, PhotosExt, Result, Summary};

#[command]
pub(crate) async fn access<R: Runtime>(app: AppHandle<R>) -> Result<Access> {
    app.photos().access()
}

#[command]
pub(crate) async fn ask<R: Runtime>(app: AppHandle<R>) -> Result<Access> {
    app.photos().ask()
}

#[command]
pub(crate) async fn summary<R: Runtime>(app: AppHandle<R>, filter: Filter) -> Result<Summary> {
    app.photos().summary(filter)
}
