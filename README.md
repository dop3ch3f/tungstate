<p align="center">
  <img src="docs/brand/wordmark.png" alt="tungstate" width="620">
</p>

A reconciliation controller for filesystems.

You declare the shape a folder should have. Tungstate watches it, detects drift
(misplaced files, naming violations, duplicates) and reconciles it safely and
continuously. It also moves files between machines durably: copy one, verify it,
delete the source, reclaim the space, next file, surviving sleep and network loss.

**Downloads:** [latest release](https://github.com/dop3ch3f/tungstate/releases)
— command line binaries and desktop installers for macOS, Windows and Linux.
Nothing is code-signed yet, so both macOS and Windows will warn you about an
unidentified developer; the release notes say how to get past it.

For whatever is on `main` right now, there is a rolling
[**Latest from main**](https://github.com/dop3ch3f/tungstate/releases/tag/rolling)
prerelease, rebuilt from the same pipeline every time the test suite passes on
the branch. It moves without warning — use a numbered release for anything you
want to stay put.

Status: early construction, and two halves work.

**The durable drain.** `tungstate link add <from> <to> --name x --move` then
`tungstate link run x`, over a mounted volume or over FTP, resumable after a
crash and drivable from the desktop app.

**The policy model.** Declare the shape a folder should have in
`.tungstate/policy.toml`, then `tungstate explain <file>` says where that file
belongs and why — every rule in order, every variable's provenance, and what
reading it cost. Nothing is moved yet; the planner and the executor are the
next two slices.

See `docs/DESIGN.md` for the full design and `docs/SYLLABUS.md` for the build
order.

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

## The name

Tungsten is element 74, `W`. A **tungstate** is the WO₄²⁻ anion, and *scheelite*
is calcium tungstate, CaWO₄: a dense grey mineral that fluoresces bright
blue-white under ultraviolet light.

That is where the identity comes from. The mark is a periodic tile carrying the
anion, and the interface is mineral grey with a single fluorescent accent
reserved for the file being verified right now. See [docs/brand](docs/brand).
