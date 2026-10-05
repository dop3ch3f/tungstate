# Slice 12: WebDAV and SFTP

**Goal:** two more ways to reach a NAS. Synology, QNAP and most other NAS
boxes offer both. Some people can't or won't turn on SMB or FTP, and SFTP is
the one encrypted protocol nearly every NAS has.

Two parts, each tagged when green: **12a WebDAV**, then **12b SFTP**.

**Runnable outcome:**

```
tungstate connection add box --scheme webdav --endpoint https://nas.local:5006 --user me --root /media
tungstate connection add nas --scheme sftp --host nas.local --user me --root /volume1/media
tungstate connection test nas          # shows the server's fingerprint and asks to trust it
tungstate link add ~/Videos nas:inbox --name to-nas --move
tungstate link run to-nas
```

Both also appear in the window's Connections section, and work for transfers,
sync, duplicates and tidying exactly as SMB does.

**This brief is the spec.** The tours will walk the shipped code.

---

## 12a: WebDAV

### What OpenDAL's WebDAV service turned out to be

Read in `opendal-service-webdav` 0.59.1 before writing any code:

- **Listing, reading, renaming and deleting are good.** Ranged reads use a
  real `Range` header, and rename is `MOVE` with `Overwrite: T`, so WebDAV
  takes the engine's atomic commit path (write to a temp name, verify, rename)
  as SMB does.
- **A listing says exactly what a `stat` says.** Both are the same
  `PROPFIND` parsed by the same function, to the second. So
  `listing_is_complete()` is true, and scans get 8f's saving.
- **Its HTTPS is the S3 transport.** It sends every request through the
  process-wide client the adapter already installs, rustls on `ring`, so no
  new TLS stack.
- **The blocker: its writer takes exactly one `write`.** It is a
  `OneShotWriter`, and a second `write` returns `Unsupported`. Our writer
  streams a file in chunks, so any file bigger than one chunk would fail.

### Decisions

- **Uploads are our own streaming `PUT` (chosen).** OpenDAL stays for
  everything else. The upload goes through the same reqwest client, its body
  fed chunk by chunk from the synchronous writer through a channel, so a
  40 GB video never sits in memory. `finish` closes the stream and waits for
  the server's answer, because a `PUT` isn't committed until the server
  replies. Parent folders are made first, as OpenDAL's writer did.
  - Rejected: holding the whole file in memory.
  - Rejected: dropping OpenDAL for WebDAV entirely, which means parsing
    `PROPFIND` XML ourselves for no gain.
- **The connection is a URL.** `--endpoint https://nas.local:5006` (it may
  carry a path, such as Nextcloud's `/remote.php/dav/files/me`), plus a user
  and a root folder. `http://` is allowed and is marked as crossing the
  network unencrypted, as plain FTP is.
- **Size is unknown when the upload starts**, so the body is sent chunked.
  Every server tested here accepts that. A server that insists on a length
  will fail with its own error, which the brief records rather than works
  around.

### Not in 12a

- **A NAS's self-signed HTTPS certificate is refused**, as it is for S3
  today. Trusting one by its fingerprint, the same question SFTP asks in 12b,
  is a follow-up.
- **Servers that answer with absolute `href`s** (`http://host/...`) rather
  than paths are not handled by OpenDAL. They'll need checking against real
  Synology and QNAP units.

### Tests

- Against `rclone serve webdav` in Docker, in CI, mirroring the FTP and S3
  jobs:
  - a round trip;
  - a file far bigger than one chunk;
  - a ranged read;
  - a listing that equals `stat`;
  - a rename over an existing file;
  - a missing link-end folder that lists as empty;
  - the drain and a sync through the binary.
- Unit tests for the URL building: spaces, unicode, `#` and `?` in names.

---

## 12b: SFTP

Hand-rolled on `russh` 0.64 and `russh-sftp` 3, as DESIGN.md §6 decided in
4b. It's a crate of its own, `tungstate-backend-sftp`, handed out by the
connection factory like SMB.

### Decisions

- **`ring`, not `aws-lc-rs`.** `default-features = false` with `ring`, so it
  reuses the crypto already in the build and needs no OpenSSL on any
  platform.
- **The async bridge is the adapter's own: spawn, never `block_on`.**
- **Trust: the fingerprint is shown and asked once (chosen).**
  - The first connection to an unknown server stops and reports
    `SHA256:…`. `connection test` asks; the window shows it in a dialog with
    Trust and Cancel.
  - A trusted key goes into Tungstate's own `known_hosts`, beside the
    journal, not `~/.ssh`.
  - If a server's key ever changes, every connection to it stops, and says
    so in plain words, until the user looks.
- **Sign-in: password and key file (chosen).**
  - A password, offered both as `password` and as `keyboard-interactive`,
    which several NAS boxes use instead.
  - Or a key file, whose path is a connection option and whose passphrase,
    if any, is kept where a password would be.
- **Rename over an existing file uses `posix-rename@openssh.com`.** Plain
  SFTP rename fails when the target exists. Where a server lacks the
  extension, SFTP reports no atomic rename and takes the no-rename commit
  path, as FTP does.
- **A listing is complete when `stat` doesn't follow links.** OpenSSH fills
  each listing entry from `lstat`, so `stat` uses `lstat` too, which is
  already the engine's rule: links are described, never followed.
- **A dropped connection is dialled again**, once per call. `russh` never
  reconnects on its own.
- **Throughput:**
  - a larger SSH window and `nodelay`;
  - `russh-sftp`'s pipelined reads and writes;
  - a request timeout long enough for a NAS spinning its disks up.

### Tests

- Against `atmoz/sftp` in Docker, in CI, mirroring the SMB job:
  - the backend's own suite;
  - an unknown host refused until trusted;
  - a changed key refused;
  - password and key sign-in;
  - rename over an existing file;
  - a listing that equals `stat`;
  - the drain and a sync through the binary.
- Unit tests for `known_hosts` handling and the fingerprint text.

---

## Acceptance

For each part:
- build, test (in parallel and one at a time), `clippy -D warnings` and
  `fmt --check` are green;
- CI is green on all three platforms, with the new container job;
- the window offers the new kind, and a connection made there passes its
  test;
- a tour and a tag.
