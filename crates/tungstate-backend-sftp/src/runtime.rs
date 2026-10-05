//! One tokio runtime, owned by this crate, driving `russh` from tungstate's
//! synchronous engine.
//!
//! The same bridge as the `OpenDAL` adapter's, and for the same reason: every
//! call is spawned onto this runtime and the calling thread waits on a plain
//! channel, because `block_on` panics on a thread already inside a runtime,
//! which a Tauri command is. Kept as a copy rather than shared: it is twenty
//! lines, and a crate of its own for them would cost more than it saves.

use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::Runtime;

/// Built on first use, then shared by every SFTP connection in the process.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("tungstate-sftp")
        .build()
        .expect("a tokio runtime for SFTP")
});

/// Run `future` on this crate's runtime and block until it answers.
///
/// # Panics
/// If the spawned task panicked, rather than inventing an error for a bug.
pub(crate) fn dispatch<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    RUNTIME.handle().spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv()
        .expect("the SFTP runtime dropped a task without answering")
}
