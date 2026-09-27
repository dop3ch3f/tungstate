## Installing

Nothing here is code-signed. This project has no Apple Developer certificate
and no Windows code-signing certificate yet, so both systems will warn you
about an unidentified developer. Each file has a `.sha256` next to it if you
want to check the download.

**Desktop app**

- **macOS:** open the `.dmg` and drag Tungstate to Applications. The first
  time, right-click it and choose **Open**, because a double-click will refuse.
  If the right-click doesn't offer Open (Sonoma and later), open it once, then
  go to **System Settings → Privacy & Security** and press **Open Anyway** next
  to the message about Tungstate.

  If macOS says the app is **damaged**, download it again from a release built
  after 2026-09-15. Earlier builds shipped without a signature of their own,
  and macOS reports that as damage.
- **Windows:** run the installer. When SmartScreen says "Windows protected your
  PC", choose **More info → Run anyway**.
- **Linux:** the `.AppImage` (`chmod +x`, then run it) or the `.deb`
  (`sudo apt install ./tungstate_*.deb`).

Pick the `arm64` macOS build for Apple silicon and `x86_64` for Intel.

**Command line:** download the archive for your platform, unpack it, and put
`tungstate` somewhere on your `PATH`.

```
# macOS: clear the quarantine flag, or Gatekeeper will refuse to run it
xattr -d com.apple.quarantine tungstate

# Linux and macOS
chmod +x tungstate && ./tungstate --version
```

**Updates.** From 0.1.0-alpha.4 the desktop app checks for a new release when
it opens and once a day, and asks before installing it. Install alpha.4 or
later by hand once; after that, Settings or the line in the sidebar does it.
The `.deb` doesn't update itself, so download the next one yourself.

## What it does

**Transfer.** Move or copy files to another folder, a mounted share, or a NAS
over FTP or FTPS. Each file is checked on arrival before the original is
touched, and a transfer that is interrupted picks up where it stopped.

```
tungstate link add ~/Videos /Volumes/nas/inbox --name drain --move
tungstate link run drain

tungstate connection add nas --scheme ftps --host nas.local --user me --root /volume1/media
tungstate link add ~/Videos nas:inbox --name drain --move --verify readback
```

**Organize.** Give a folder a layout, by picking a starting one or letting
`tungstate folder learn` read the shape it already has. Preview what would
move, tidy it, and put it all back if you don't like the result. A folder can
also keep itself tidy while the app is open.

```
tungstate init ~/Downloads
tungstate plan ~/Downloads
tungstate apply ~/Downloads
tungstate undo ~/Downloads
```

**Sync.** Keep two or more folders in step: sending one way, bringing in, or
both ways, on this machine or on the NAS. The app asks about conflicts before
anything moves, and sets deleted files aside instead of destroying them.
`tungstate sync undo` puts a run back on every member.

```
tungstate sync add capcut ~/CapCut nas:capcut --all
tungstate sync preview capcut
tungstate sync run capcut
```

**Duplicates.** Find files that are the same file, whatever they're called,
and set the extra copies aside. `tungstate dedupe` reports and changes nothing
until you add `--apply`.

**History.** `tungstate log <path>` and `tungstate whereis <path|hash>` say
what happened to a file and where it went.

## Good to know

Nothing runs in the background. Transfers, syncs and tidying happen while the
window is open or a command you typed is running. Closing the window stops a
transfer safely: reopen it and the app offers to finish what was left or clear
it. The command line has the same two: `tungstate link unfinished` and
`tungstate link discard <name>`.

A connection can't be renamed, because saved transfers and the stored password
both use its name. Remove it and add it again instead.

See [`docs/SYLLABUS.md`](docs/SYLLABUS.md) for what is built and what comes
next.

---
