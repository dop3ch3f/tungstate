//! What the add-connection form asks of the network: which servers are here,
//! and which shares an SMB server offers once signed in to.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Runtime};

use crate::describe;
use crate::discover::{self, Found};

/// One look at a time: a second tap while looking would only double the
/// knocking. Held for the look's few seconds.
static LOOKING: Mutex<()> = Mutex::new(());

/// Looks for file servers on this network for a few seconds.
///
/// Each server is sent as `discover://found` the moment it is seen (and again
/// when it turns out to offer more), so the list fills in as it goes. Answers
/// with everything found, or `None` if a look was already under way, whose
/// events are still coming.
#[tauri::command(async)]
pub fn discover<R: Runtime>(app: AppHandle<R>) -> Option<Vec<Found>> {
    let _looking = LOOKING.try_lock().ok()?;
    // Android drops multicast packets addressed to other apps unless a lock is
    // held, and mDNS answers are multicast.
    #[cfg(mobile)]
    let _ = tauri_plugin_photos::PhotosExt::photos(&app).multicast(true);
    let found = discover::look(|row| {
        let _ = app.emit("discover://found", row);
    });
    #[cfg(mobile)]
    let _ = tauri_plugin_photos::PhotosExt::photos(&app).multicast(false);
    Some(found)
}

/// The shares an SMB server offers to this user, for picking one rather
/// than typing its name. Nothing is saved.
#[tauri::command(async)]
pub fn connection_shares(
    host: String,
    port: Option<u16>,
    username: String,
    secret: String,
) -> Result<Vec<String>, String> {
    tungstate_backend_opendal::smb_shares(&host, port, &username, &secret).map_err(describe)
}
