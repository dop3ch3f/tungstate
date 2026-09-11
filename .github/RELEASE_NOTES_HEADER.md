## Installing

Nothing here is code-signed. That is not a sign anything is wrong with the
download — it means there is no Apple Developer certificate and no Windows
code-signing certificate behind this project yet, so both operating systems
will warn you about an unidentified developer. Every file has a `.sha256`
beside it if you want to check it arrived intact.

**Command line** — download the archive for your platform, unpack it, and put
`tungstate` somewhere on your `PATH`.

```
# macOS: clear the quarantine flag, or Gatekeeper will refuse to run it
xattr -d com.apple.quarantine tungstate

# Linux and macOS
chmod +x tungstate && ./tungstate --version
```

**Desktop app**

- **macOS** — open the `.dmg`, drag Tungstate to Applications, then
  **right-click → Open** the first time. Double-clicking will just refuse.
- **Windows** — run the installer. SmartScreen will say "Windows protected
  your PC"; choose **More info → Run anyway**.
- **Linux** — `.AppImage` (`chmod +x`, then run) or `.deb`
  (`sudo apt install ./tungstate_*.deb`).

Pick the `arm64` macOS build for Apple silicon and `x86_64` for Intel.

## What works today

A durable drain: move files from one place to another, verify each one before
the original is touched, and survive being interrupted at any point.

```
tungstate link add ~/Videos /Volumes/nas/inbox --name drain --move
tungstate link preview drain
tungstate link run drain
```

Destinations can also be reached over FTP or FTPS:

```
tungstate connection add nas --scheme ftps --host nas.local --user me --root /volume1/media
tungstate connection test nas
tungstate link add ~/Videos nas:inbox --name drain --move --verify readback
```

`tungstate log <path>` and `tungstate whereis <path|hash>` answer where a file
went. Kill a run at any moment and start it again; nothing is lost and nothing
is copied twice.

**Closing the desktop window stops the transfer.** There is no background
service yet, so the engine runs inside the app. Nothing is lost when you do:
reopen it and the Transfers tab names what was left unfinished, with a button
to finish it and a button to clear it. The command line has the same two:
`tungstate link unfinished` and `tungstate link discard <name>`.

## What does not work yet

The governance half of the design — declaring a shape for a folder and having
it reconciled — is not built. `tungstate folder add` is a stub. The desktop app
cannot add connections yet, so remote destinations are command line only. See
[`docs/SYLLABUS.md`](docs/SYLLABUS.md) for the build order.

---
