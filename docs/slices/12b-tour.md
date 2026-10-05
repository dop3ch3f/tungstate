# Slice 12b tour: SFTP

A seventh kind of connection: files over SSH, encrypted, and on nearly every
NAS. It signs in with a password or a key file. The first time it reaches a
server it asks you to trust that server's fingerprint, and it refuses a
server whose key later changes. The brief is
[12-webdav-sftp.md](12-webdav-sftp.md); WebDAV is
[12a-tour.md](12a-tour.md).

---

## 1. A crate of its own

`tungstate-backend-sftp` is built on `russh` and `russh-sftp`, as DESIGN.md
§6 decided in 4b. OpenDAL's SFTP service is Unix-only, refuses passwords,
and leaves the server's identity to the system's `ssh`. The connection
factory hands this crate's backend out the same way it hands out SMB's.

The async bridge is a copy of the OpenDAL adapter's: spawn onto a runtime
this crate owns, then wait on a plain channel. It is twenty lines, and a
shared crate for them would cost more than it saves.

## 2. Which `russh`: when release candidates collide

The newest `russh` is 0.64, and this slice ships 0.55. The reason is a
Cargo rule worth knowing.

- **A pinned version.** Since 0.60, `russh` pins its cryptography crates to
  exact *release candidates*, such as `curve25519-dalek = "=5.0.0-rc.1"`.
  smb-rs's `sspi` pins them too, to different candidates.
- **Cargo's rule.** Cargo keeps one copy of a crate per semver-compatible
  line. 5.0.0-rc.1 and 5.0.0 count as the same line, so two different `=`
  pins on it cannot both be met, and nothing builds.
- **Two lines can live side by side.** 0.55 uses the stable 4.x and 0.13
  lines of the same crates. Those are *different* semver lines from SMB's
  5.x and 0.14 candidates, so Cargo builds both.

Every version in between was tried. 0.56 resolves, but its RSA support
fails to compile beside SMB's: Cargo unified the two RSA requirements onto
one candidate, whose API moved. The pin and the reason sit in the crate's
`Cargo.toml`, with a note to move on once smb-rs moves.

It still uses `ring`, the crypto library already in the build, and no
OpenSSL. `cargo tree` confirms a single copy of `ring`.

## 3. Trust: the key is kept on the connection

```rust
async fn check_server_key(&mut self, server: &PublicKey) -> Result<bool, Self::Error> {
    let key = server.clone();
    let matches = self.trusted.as_ref().is_some_and(|t| t.key_data() == key.key_data());
    *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(key);
    Ok(matches)
}
```

- **During the handshake**, `russh` calls this with the server's key. Saying
  `false` ends the connection.
- **A failed connection still says what the server showed.** The handler
  can't stop and ask a person mid-handshake. So it keeps the key it was
  shown in `seen`, an `Arc<Mutex<_>>` shared with the code that dialled,
  and that code turns the failure into one of two new errors:
  - `HostUnknown`: nothing trusted yet;
  - `HostKeyChanged`: a different key from the one trusted.

  Both carry the fingerprint (`SHA256:…`) to show and the key to keep.
- **The trusted key is a connection option, `host_key`, not a
  `known_hosts` file.** That changed from the brief while building.
  - Checking is an exact comparison, so there is no file format to parse.
    `russh`'s parser has gaps: tabs, `@cert-authority`.
  - It travels with the connection through export and import.
  - It goes when the connection goes.

`BackendError` gained the two variants. Every `match` on it elsewhere already
ends in a catch-all, so adding them broke no caller. That was checked before
adding them.

## 4. Signing in

- **A password** is tried as `password`. If the server refuses but offers
  `keyboard-interactive`, which several NAS boxes prefer, the same password
  answers each prompt. That's at most four rounds: a server that keeps
  asking isn't asking for a password.
- **A key file**, with its passphrase kept where a password would be. RSA
  keys sign with SHA-2 where the server allows it.

## 5. Many requests in flight

SFTP numbers its requests, so one connection can have many outstanding.

- **Writes:** each megabyte the engine writes becomes 32 pieces of 32 KiB,
  sent together with `join_all` and answered by number.
- **Reads:** the same, 32 pieces ahead of where the engine has got to.
- **A short answer** can be the end of the file or just a server's limit.
  Either way, the pieces asked for after it start in the wrong place, so
  they are dropped and asked for again.

## 6. What the server can and cannot do

- **Rename over an existing file** is OpenSSH's `posix-rename@openssh.com`.
  Plain SFTP rename refuses an existing target.
  - Where the server offers the extension, SFTP takes the atomic commit
    path.
  - Where it doesn't, `atomic_rename` is false and the engine uses the
    no-rename path, as for FTP.
  - The extension's wire format is the same two strings as the hardlink
    extension, so `russh-sftp`'s `HardlinkExtension` struct serialises it.
- **`fsync@openssh.com`**, where offered, puts a file's bytes on the
  server's disk before `finish` says it arrived.
- **A dropped connection is dialled again** at the next call. `russh`
  never reconnects by itself.
- **Up to two retries if the server drops us before showing its key.**
  OpenSSH drops new connections at random once ten are mid-handshake
  (`MaxStartups`), and a NAS keeps that default. The test suite hit it
  straight away.

## 7. A bug the tests found

When a drain ran with two workers, one file failed with `create_dir failed:
Failure`. Both workers needed the same new folder:

1. Both looked and found it missing.
2. Both sent `mkdir`.
3. The second was refused, because the folder now existed.

SFTP says only "Failure" for that. `make_folder` now looks again after a
refused `mkdir`, and a folder that is there counts as made.
`workers_making_the_same_folders_at_once_all_succeed` races six threads at
one folder chain. It fails three runs out of three without the fix and
passes with it.

## 8. The command line and the window

- **Command line:**
  ```
  connection add nas --scheme sftp --host nas.local --user me --root /volume1/media
  connection add nas ... --key ~/.ssh/id_ed25519   # then the secret is the passphrase
  connection test nas      # an unknown server: its fingerprint, and what to run next
  connection trust nas     # shows the fingerprint and asks
  connection trust nas --fingerprint SHA256:…   # for scripts: only if it matches
  ```
  `connection test` stays read-only. `trust` with no terminal and no
  `--fingerprint` trusts nothing, so a script can never accept a server
  unseen.
- **Window:**
  - **The form.** "Files over SSH" offers a password or a key file. When
    Check meets an unknown server it shows the fingerprint with "Trust it",
    and the key is saved with the connection.
  - **The list.** Checking a saved connection that needs trusting asks in a
    dialog. A changed key gets the stronger warning and a red button.
  - **Under the hood.** The check returns a `server` field instead of an
    error string, so the window can put buttons on it.

## 9. What is checked

- **Against OpenSSH** (`atmoz/sftp`, pinned by digest), locally and in a new
  CI job:
  - an unknown server refused with its fingerprint;
  - a changed key refused;
  - a wrong password;
  - an unlocked key and a passphrase-locked key;
  - 5 MB round trips;
  - ranged reads, including one past the end;
  - a listing equal to `stat`;
  - rename over an existing file;
  - removing files and folders;
  - setting a time;
  - a fresh link end listing as empty;
  - a missing folder as unreachable;
  - a path that tries to climb out;
  - the folder race.
- **Through the binary:**
  - trust refused for a wrong fingerprint;
  - trust refused with no terminal;
  - trust accepted for the right fingerprint;
  - a wrong password;
  - a passphrase-locked key;
  - a drain with read-back;
  - a sync back the other way.
- **Unit tests:** the path rules, the journal's settings for SFTP, and the
  window's check, type check and 48 tests.

## What is not verified

- **No real NAS.** Synology's and QNAP's SSH servers are OpenSSH, so the
  extensions should be there, but nobody has checked.
- **Speed** was not measured against a slow network.
- **`russh` 0.55 is older than current** for the reason in section 2. It is
  worth revisiting when smb-rs updates.
- **The screenshots** show the Add button mid-fade after Trust. The headless
  browser freezes CSS transitions at their first frame, and in the app the
  fade is over in a moment.
