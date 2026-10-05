# Slice 12a tour: WebDAV

A sixth kind of connection, for a NAS's web folders, Nextcloud, or a hosting
service. It works everywhere SMB does: transfers, sync, duplicates and
tidying. The brief is [12-webdav-sftp.md](12-webdav-sftp.md).

---

## 1. What is ours and what is OpenDAL's

OpenDAL's WebDAV service does most of it well:
- listing;
- ranged reads;
- `MOVE` with `Overwrite: T` for rename, so WebDAV takes the engine's atomic
  commit path, as SMB does;
- deleting.

Its writer is the exception. It is a `OneShotWriter`, which takes one
`write` and refuses the next. The engine writes a file a chunk at a time, so
uploads are ours (`src/dav.rs`), and everything else stays on OpenDAL.

## 2. A body that is written while it is sent

```rust
let (chunks, mut queue) = mpsc::channel::<std::io::Result<Vec<u8>>>(IN_FLIGHT);
let body = futures_util::stream::poll_fn(move |context| queue.poll_recv(context));
let mut request = self.client.put(self.url(key)).body(reqwest::Body::wrap_stream(body));
```

- **A channel is a queue between two threads.** The engine's thread pushes
  chunks in with `write`. The request, running on the adapter's runtime,
  pulls them out as the network takes them.
- **`IN_FLIGHT` is 4, and that number is the memory limit.** The channel is
  *bounded*: when four chunks are waiting, `send` waits too. A fast disk and
  a slow NAS then cost a few megabytes, not the whole file. That waiting is
  called backpressure.
- **`poll_fn` turns "receive the next chunk" into a stream.** A stream is
  the async version of an iterator: `reqwest` asks for the next item and is
  told either "here", "not yet, I'll wake you", or "finished". `poll_recv`
  answers exactly those three.
- **Nobody knows the size in advance**, so the body goes out chunked, which
  HTTP/1.1 allows. Every server tested accepts it.

The request is started with `spawn`, which returns at once, so it is already
sending while the engine writes. `finish` closes the channel, which ends the
body, and then waits for the server's answer. A `PUT` is not committed until
the server replies, so a journal entry for an upload the server rejected is
impossible.

## 3. Drop as cancel

```rust
impl Drop for DavWrite {
    fn drop(&mut self) {
        if let Some(answer) = self.answer.take() {
            answer.abort();
        }
    }
}
```

`Drop` runs when a value goes away, however that happens: an early `return`,
a `?`, or a panic unwinding past it. Here it abandons the request, which
closes the connection mid-body, and the server throws the partial upload
away.

The obvious alternative would be wrong. Merely dropping the channel would
end the body *cleanly*, and the server would keep a short file that looks
finished. `finish` takes `answer` out of the `Option` first, so a finished
upload is never aborted.

## 4. Rooted at `/`, with the folder in the key

The first test run failed with `409 Conflict` on every upload into a fresh
connection. A `PUT` into a folder that does not exist fails, so the upload
first makes the file's parent folder, but the *connection's own* folder was
OpenDAL's root, which the key never included. OpenDAL's writer made it; ours
did not.

The fix is one decision, not a special case. The operator is rooted at the
server's `/`, and the connection's folder becomes the start of every key.
"Make the parent" then includes the connection's folder the first time it is
needed, with no extra code.

## 5. Features that switch each other on

```toml
s3 = ["opendal/services-s3", "https"]
webdav = ["opendal/services-webdav", "https", "reqwest/stream"]
https = ["dep:opendal-http-transport-reqwest", "dep:reqwest", "dep:rustls"]
```

Cargo features are switches for optional code. `https` is the HTTPS client S3
and WebDAV share, so neither has to list its parts. Code that only one of
them needs says so with `#[cfg(feature = "webdav")]`, and a build with
neither still compiles. That was checked here with clippy and
`--no-default-features`; CI only builds the defaults.

The client itself is built once, in `https_client()`, behind a `OnceLock`, a
value set on first use and then only read. OpenDAL is given a clone, and
uploads use another. A `reqwest::Client` is an `Arc` inside, so the clones
share one connection pool.

## 6. The rest of the app

- **Journal:** `Scheme::WebDav`, stored as `webdav`. The server is the
  `endpoint` option, a URL, because it may carry a path (Nextcloud's
  `/remote.php/dav/files/<you>`).
  - Saving without an endpoint, or with one that isn't a URL, is refused.
  - A plain `http://` endpoint is named as crossing the network
    unencrypted, password included.
- **Command line:**
  ```
  connection add box --scheme webdav --endpoint https://nas.local:5006 --user me --root media
  ```
  `connection list` shows the address.
- **Window:** a "Web folders" kind, with an address field and a warning
  for plain http. The kind picker is now three even rows.
- **A wrong password** is recognised from the server's `401` and reported
  as the credentials, not the network.

## 7. What is checked

- **Against rclone's WebDAV server**, locally and in a new CI job (image
  pinned by digest):
  - a 6 MB file written 64 KB at a time;
  - names with spaces, `#`, `?` and `é`;
  - a ranged read;
  - a listing equal to `stat`;
  - rename over an existing file;
  - an upload dropped halfway;
  - a fresh folder listing as empty;
  - a wrong password through `connection test`;
  - a drain with read-back verification;
  - a sync whose second preview has nothing to do.
- **Unit tests** for the URL: the endpoint's own path kept, and names
  escaped.
- **The whole suite:** in parallel and one at a time, plus the window's
  type check, CSS check and its 48 tests.

## What is not verified

- **No real NAS.** Synology and QNAP were not tried. Two things to watch:
  - a self-signed HTTPS certificate is refused, as for S3;
  - servers that answer listings with full URLs instead of paths are not
    handled by OpenDAL.
- **A server that refuses chunked uploads** would fail with its own error.
- **Speed** was not measured. Uploads are one request per file, as on every
  other protocol.
