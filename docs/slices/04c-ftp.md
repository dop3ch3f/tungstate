# Slice 4c: FTP

**Goal:** drain the laptop to the NAS over FTP, with the same guarantee slice 3
shipped — nothing deleted until a verified copy exists — using the seam slice 4b
built.

**Runnable outcome:**

```
tungstate connection add nas --scheme ftps --host nas.local --user me --root /volume1/media
tungstate connection test nas
tungstate link add ~/Videos nas:inbox --name laptop-to-nas --move --verify readback
tungstate link run laptop-to-nas
```

**This brief is the spec.** The walkthrough of the shipped code is in
[04c-tour.md](04c-tour.md).

---

## What FTP turned out to be, which is not what the roadmap assumed

The roadmap said "add `services-ftp`, handle `atomic_rename: false`". Reading
`opendal-service-ftp` 0.59.1 before writing any code found four things that
change the work.

**1. OpenDAL's FTP service supports neither `rename` nor `copy`.** Both return
`Unsupported`, and `rename` is absent from its capability set. This is a gap in
OpenDAL's adapter, not in the protocol: FTP has RNFR/RNTO, and `suppaftp` — the
crate OpenDAL uses underneath — exposes `rename()` on both its clients. So
tungstate's commit step, *stream to a temp name → verify → rename into place*,
cannot run over OpenDAL's FTP.

**2. But the FTP writer is internally atomic anyway.** `FtpLazyWriter` writes to
`<basename>.<8 random chars>` in the destination directory and issues RNFR/RNTO
to the target on `close()`. So writing straight to the final name *does* publish
atomically over FTP. The engine cannot assume that of every no-rename backend,
but it is why this slice's degradation is much smaller than it first looked.

**3. A ranged read costs a whole data connection.** `FtpReadStream::new` sends
`REST <offset>` then `RETR`, so every ranged read is a new FTP data connection.
The adapter's 1 MiB ranged-read loop, which was fine for `fs`, would open ~4000
data connections for a 4 GB file. This is the cost the 4b tour flagged and
deferred to here.

**4. Crashing leaks a temp file nobody will ever collect.** OpenDAL's temp name
is random, so neither it nor tungstate can find it afterwards. Over a long life
a NAS would silently accumulate `holiday.mp4.k3xq9wpz` next to every transfer
that was ever interrupted.

## Decisions

**FTP stays on OpenDAL; the engine learns to commit without rename.** The
alternative — hand-rolling FTP on `suppaftp`, as slice 4b decided for SFTP — was
weighed and rejected for now. The no-rename commit path is needed for S3 and
WebDAV in slice 12 whatever happens to FTP, and `Capabilities.atomic_rename` has
been defined but never read since slice 1, which is a small lie in the codebase
worth correcting. If real use against the NAS shows the OpenDAL path hurting,
hand-rolling stays available; DESIGN.md §6 already sanctions it.

**Publishing on a no-rename backend is `create_write` + `finish()` to the final
name.** `copy_verify_commit` branches on `capabilities().atomic_rename`. The
rename path is unchanged. The no-rename path streams to the destination name,
verifies by reading that name back, and treats `finish()` as the commit point —
because `finish()` is already the trait's documented "commit everything written
durably, then close", and what that means is the backend's business.

**Recovery follows the same branch, and sweeps.** On a rename backend, recovery
deletes `temp_name(destination, op)` as before. On a no-rename backend the
partial can only be under the real name, so recovery deletes that — *and* sweeps
the destination directory for `<basename>.<8 chars>` orphans, because OpenDAL's
random temp names mean nothing else ever will. The sweep is scoped to the
destination of this link's own interrupted operations.

**`ConflictAction::Replace` is refused on a no-rename destination.** Replace
quarantines the existing file before landing the incoming one, and it does that
by renaming. There is no honest way to do it without rename: writing the new
file first would destroy the old one, which is precisely what Replace promises
not to do. The error names `quarantine`, `rename` and `skip` as the
alternatives, and `link add` refuses the combination up front rather than at the
first clash.

**Reads become one streamed request per file.** `OpendalRead` stops issuing a
ranged read per chunk and pulls from a `BufferStream` instead, keeping a
leftover buffer across `Read::read` calls. One RETR per file. This also removes
the need to `stat` for the length before every read.

**Reachability stops listing the root once per file.** `Anchor::Store` now
`stat`s the link end's own directory, which is one command, and only falls back
to listing the connection root when that comes back missing — which happens
exactly once, before the destination folder has been created. The
once-per-file-quadratic behaviour flagged in the 4b tour is gone.

**FTPS is its own scheme value, not an option.** `--scheme ftps` rather than
`--scheme ftp --option tls=true`. Plain FTP sends the password in clear text on
the wire, so the secure choice must be as easy to type as the insecure one and
must be visible in `connection list`. `connection add --scheme ftp` prints a
warning saying so. OpenDAL selects TLS from the endpoint scheme, so this is a
one-line mapping.

**`link add` nudges towards `--verify readback` on an FTP destination.** FTP has
no server-side checksum, so `hash` means "the bytes we sent hashed to this",
never "the bytes on the far disk hash to this". Over a network that gap is worth
a sentence.

**Integration tests run against a real FTP server in Docker, behind a cargo
feature, on Linux CI only.** `--features ftp-integration`, pointed at a server
by `TUNGSTATE_FTP_*` environment variables. Everything else — the no-rename
commit path, the sweep, the streaming reader — is tested on all three platforms
with a fake backend that reports `atomic_rename: false`, so the engine changes
are not hostage to Docker.

## Shape

- **`tungstate-transfer`.** `copy_verify_commit` and `recover` branch on
  `atomic_rename`. New `TransferError::ReplaceNeedsRename`. The sweep lives
  beside `recover`.
- **`tungstate-backend-opendal`.** New cargo features `ftp` (default) and
  `ftp-integration` (dev). `Scheme::Ftp`/`Ftps` build an operator from
  `{scheme}://{host}:{port}` with the password read from the keychain.
  `OpendalRead` becomes a streaming reader. `Anchor::Store` gets the cheap
  path. `BackendError::Auth` finally has a production site: OpenDAL reports a
  rejected FTP login as `PermissionDenied` at connect time.
- **`tungstate-journal`.** `Scheme` gains `Ftps`. No migration: the column is
  already `TEXT`, and `string_enum!` means the new spelling is one the user
  could have typed.
- **`tungstate-cli`.** `--scheme ftps`; a plaintext warning; the readback nudge;
  `--on-conflict replace` refused against a destination that cannot rename.
- **CI.** One extra Linux-only job that starts an FTP container and runs the
  gated tests.

## Tests

Cross-platform, no Docker:

- A fake backend reporting `atomic_rename: false` drains correctly, writes no
  `.part` file, and lands every byte.
- Interrupt-and-resume against that backend: the partial under the real name is
  removed, the operation is re-queued, and the result is complete and correct.
- The orphan sweep removes `a.mp4.k3xq9wpz` and leaves `a.mp4`, `a.mp4.txt` and
  `b.mp4.k3xq9wpz` alone.
- Verification failure against a no-rename backend still leaves the source
  alone and removes the bad destination.
- `ConflictAction::Replace` against a no-rename backend fails with
  `ReplaceNeedsRename`, source untouched, existing file untouched.
- The streaming reader returns identical bytes to the ranged one over `fs`,
  including across chunk boundaries and for an empty file.
- Reachability: a link end whose folder does not exist yet still reports the
  connection reachable; a connection that has gone away does not.
- `--scheme ftps` round-trips; `connection list` shows which is encrypted.

Linux CI only, behind `ftp-integration`:

- Round-trip a file through a real FTP server: write, stat, list, read back,
  delete.
- A whole drain over FTP, sources reclaimed, hashes matching, with
  `--verify readback`.
- Interrupt-and-resume over FTP, including the orphan sweep.
- A rejected login surfaces as `BackendError::Auth`, not as a protocol error.

## Acceptance criteria

- [x] `cargo build --locked` clean, `cargo test --locked` green in parallel and
      single-threaded runs
- [x] `cargo clippy --all-targets --locked -- -D warnings` silent, with and
      without `ftp-integration`
- [x] `cargo fmt --all --check` silent
- [x] The gated suite green against a real vsftpd in Docker, run both in
      parallel and single-threaded
- [x] A 3 GB drain over FTP killed mid-file with `kill -9` and resumed: the
      operation was re-queued, both files landed, the hash matched and the
      sources were reclaimed
- [ ] CI green on Linux, macOS and Windows
- [ ] A real drain to the actual NAS — **yours to run; I cannot reach it**

## Out of scope, deliberately

Byte-offset resume, even though FTP's `APPE`/`REST` now make it possible —
per-file resume is still the rule and changing it deserves its own slice.
Remote trash. SFTP, WebDAV, S3. The desktop app's connection screens (4d).
Concurrency, bandwidth caps and reachability pausing are still slice 4.
Contributing `rename` upstream to OpenDAL is worth doing and is not this slice.
