// Async so the phone's answer is waited for off the window's own thread:
// counting a large library takes a moment, and `ask` waits on a person.

use tauri::{AppHandle, Runtime, command};

use crate::{Access, Categories, Category, Filter, PhotosExt, Result, StoredFile, Summary};

#[command]
pub(crate) async fn share<R: Runtime>(app: AppHandle<R>, paths: Vec<String>) -> Result<()> {
    app.photos().share(paths)
}

#[command]
pub(crate) async fn open<R: Runtime>(app: AppHandle<R>, path: String) -> Result<()> {
    app.photos().open(path)
}

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

#[command]
pub(crate) async fn files_access<R: Runtime>(app: AppHandle<R>) -> Result<Access> {
    app.photos().files_access()
}

#[command]
pub(crate) async fn ask_files<R: Runtime>(app: AppHandle<R>) -> Result<Access> {
    app.photos().ask_files()
}

#[command]
pub(crate) async fn categories<R: Runtime>(app: AppHandle<R>) -> Result<Categories> {
    app.photos().categories()
}

#[command]
pub(crate) async fn category_files<R: Runtime>(
    app: AppHandle<R>,
    category: Category,
) -> Result<Vec<StoredFile>> {
    app.photos().category_files(category)
}
