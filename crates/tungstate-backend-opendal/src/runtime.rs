//! One tokio runtime, owned by this crate, driving `OpenDAL`'s async API from
//! tungstate's synchronous engine.
//!
//! `OpenDAL` has been async-only since 0.54 (RFC-6189). It ships an
//! `opendal::blocking::Operator`, and this crate deliberately does not use it:
//! that wrapper calls `Handle::block_on`, which **panics when the calling
//! thread is already inside a tokio runtime**. The GUI's Tauri commands are
//! exactly such a thread, so the panic would be a user-visible crash rather
//! than a test failure.
//!
//! `Handle::spawn` has no such restriction — it is legal from any thread,
//! inside a runtime or not. So every call is spawned onto this crate's own
//! runtime and the calling thread waits on a plain `std::sync::mpsc` channel.
//! This is DESIGN.md §7's "adapter at the boundary": the async blast radius
//! stops at this file.

use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::Runtime;

/// Built on first use, then shared by every operator in the process.
///
/// Two worker threads: the engine is the thing doing the work, and these only
/// ever drive I/O futures on its behalf.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("tungstate-opendal")
        .build()
        .expect("a tokio runtime for the OpenDAL adapter")
});

/// Run `future` on this crate's runtime and block until it answers.
///
/// `'static` and `Send` on the future are `spawn`'s requirements, not ours;
/// they are why callers clone the `Operator` (an `Arc` inside) and move owned
/// paths in, rather than borrowing from `&self`.
///
/// # Panics
/// If the spawned task panicked. The alternative — inventing an error value —
/// would turn a bug in the adapter into a silent mid-drain failure.
pub(crate) fn dispatch<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    RUNTIME.handle().spawn(async move {
        // Send can only fail if the caller gave up waiting, which it never does.
        let _ = tx.send(future.await);
    });
    rx.recv()
        .expect("the OpenDAL adapter's runtime dropped a task without answering")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_comes_back_from_the_runtime() {
        assert_eq!(dispatch(async { 1 + 1 }), 2);
    }

    #[test]
    fn dispatching_from_inside_a_runtime_does_not_panic() {
        // The case `opendal::blocking` gets wrong, and the reason this module
        // exists. A Tauri command runs on exactly such a thread.
        let outer = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let answer = outer.block_on(async { dispatch(async { 42 }) });
        assert_eq!(answer, 42);
    }
}
