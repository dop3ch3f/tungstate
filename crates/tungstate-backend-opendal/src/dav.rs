//! `WebDAV` uploads, streamed.
//!
//! `OpenDAL`'s `WebDAV` writer is a `OneShotWriter`: it takes exactly one
//! `write` and refuses a second with `Unsupported`. The engine writes a file in
//! chunks, so through `OpenDAL` any file bigger than one chunk would fail, and
//! holding a 40 GB video in memory to make it one chunk is not an answer.
//!
//! So this one operation is ours: a `PUT` whose body is fed chunk by chunk
//! from the synchronous writer through a channel. Everything else on a `WebDAV`
//! connection, listing, reading, renaming and deleting, stays on `OpenDAL`.
//! The URL is built with `OpenDAL`'s own path helpers, so an upload lands
//! exactly where `OpenDAL` will later look for it.

use std::io::Write;
use std::path::{Path, PathBuf};

use opendal::raw::{build_rooted_abs_path, normalize_root, percent_encode_path};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tungstate_backend::{BackendError, Result, WriteFinish};

use crate::runtime::{dispatch, spawn};

/// Chunks that may wait for the network before a `write` waits too. Enough to
/// keep the connection busy, few enough that memory stays a few megabytes.
const IN_FLIGHT: usize = 4;

/// Where uploads go, and who they go as.
#[derive(Debug)]
pub(crate) struct Uploads {
    client: reqwest::Client,
    /// The server's URL with no trailing slash, as `OpenDAL` is given it.
    endpoint: String,
    /// The connection's root, normalised as `OpenDAL` normalises it.
    root: String,
    username: Option<String>,
    password: Option<String>,
    /// The connection's name, for messages.
    name: String,
}

impl Uploads {
    pub(crate) fn new(
        client: reqwest::Client,
        endpoint: &str,
        root: &str,
        username: Option<String>,
        password: Option<String>,
        name: String,
    ) -> Self {
        Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            root: normalize_root(root),
            username,
            password,
            name,
        }
    }

    /// The URL `OpenDAL` uses for `key`, which is relative to the root.
    pub(crate) fn url(&self, key: &str) -> String {
        format!(
            "{}{}",
            self.endpoint,
            percent_encode_path(&build_rooted_abs_path(&self.root, key))
        )
    }

    /// Start a `PUT` to `key`. Bytes go across as they are written; the file
    /// exists on the server only once [`WriteFinish::finish`] has its answer.
    pub(crate) fn put(&self, key: &str, path: &Path) -> DavWrite {
        let (chunks, mut queue) = mpsc::channel::<std::io::Result<Vec<u8>>>(IN_FLIGHT);
        let body = futures_util::stream::poll_fn(move |context| queue.poll_recv(context));
        let mut request = self
            .client
            .put(self.url(key))
            .body(reqwest::Body::wrap_stream(body));
        if let Some(user) = &self.username {
            request = request.basic_auth(user, self.password.as_deref());
        }
        DavWrite {
            chunks: Some(chunks),
            answer: Some(spawn(request.send())),
            path: path.to_path_buf(),
            name: self.name.clone(),
        }
    }
}

/// One upload in progress.
pub(crate) struct DavWrite {
    /// `None` once the body is closed.
    chunks: Option<mpsc::Sender<std::io::Result<Vec<u8>>>>,
    /// The request, sent while the body is still being written. `None` once
    /// its answer has been read.
    answer: Option<JoinHandle<reqwest::Result<reqwest::Response>>>,
    path: PathBuf,
    name: String,
}

impl DavWrite {
    /// Wait for the server's answer to the request.
    fn answer(&mut self) -> Result<()> {
        let Some(answer) = self.answer.take() else {
            return Err(self.failed("this upload has already finished"));
        };
        let response = dispatch(answer).map_err(|join| self.failed(&join.to_string()))?;
        let response = response.map_err(|error| self.failed(&error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(BackendError::Auth {
                endpoint: self.name.clone(),
            });
        }
        let said = match status.as_u16() {
            // 507 Insufficient Storage is WebDAV's own.
            507 => "the server has no room for it".to_string(),
            403 => "the account may not write there".to_string(),
            _ => format!("the server answered {status}"),
        };
        Err(self.failed(&said))
    }

    fn failed(&self, said: &str) -> BackendError {
        BackendError::Io {
            path: self.path.clone(),
            source: std::io::Error::other(format!("could not upload to `{}`: {said}", self.name)),
        }
    }
}

impl Write for DavWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(chunks) = self.chunks.clone() else {
            return Err(std::io::Error::other("this writer has already been closed"));
        };
        let chunk = buf.to_vec();
        if dispatch(async move { chunks.send(Ok(chunk)).await }).is_err() {
            // The request stopped reading its body, so it has already ended,
            // and its answer says why. Better than "channel closed".
            self.chunks = None;
            let why = self.answer().err().map_or_else(
                || "the server stopped reading the upload".to_string(),
                |error| error.to_string(),
            );
            return Err(std::io::Error::other(why));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // As `File::flush`: committing is what `finish` is for.
        Ok(())
    }
}

impl WriteFinish for DavWrite {
    fn finish(mut self: Box<Self>) -> Result<()> {
        // Closing the channel ends the body, which is what lets the server
        // answer at all.
        self.chunks = None;
        self.answer()
    }
}

impl Drop for DavWrite {
    fn drop(&mut self) {
        // Dropped without `finish`: abandoning the request closes the
        // connection mid-body, so the server discards it. Merely closing the
        // channel would end the body cleanly and leave a short file.
        if let Some(answer) = self.answer.take() {
            answer.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uploads(endpoint: &str, root: &str) -> Uploads {
        Uploads::new(
            // The app's own: this build installs `ring` rather than letting
            // reqwest pick, and `Client::new` panics without a provider.
            crate::https_client().expect("an HTTPS client").clone(),
            endpoint,
            root,
            None,
            None,
            "nas".to_string(),
        )
    }

    #[test]
    fn a_url_is_the_endpoint_then_the_root_then_the_key() {
        let at = uploads("https://nas.local:5006/", "media");
        assert_eq!(
            at.url("2024/a.jpg"),
            "https://nas.local:5006/media/2024/a.jpg"
        );
        // An endpoint's own path is kept, as Nextcloud's has to be.
        let cloud = uploads("https://cloud.example/remote.php/dav/files/me", "");
        assert_eq!(
            cloud.url("a.jpg"),
            "https://cloud.example/remote.php/dav/files/me/a.jpg"
        );
    }

    #[test]
    fn names_are_escaped_and_slashes_are_not() {
        let at = uploads("http://nas", "/my media/");
        assert_eq!(
            at.url("Holiday #1/café?.mp4"),
            "http://nas/my%20media/Holiday%20%231/caf%C3%A9%3F.mp4"
        );
    }
}
