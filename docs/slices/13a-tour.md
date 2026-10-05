# Slice 13a tour: tidy a folder on the NAS, in place

Organize now works on a folder on a connection. Files are filed on the NAS
itself, nothing is downloaded to be moved, and Put it back works there too.
This part covers connections that rename on the server: SMB, WebDAV and
SFTP. FTP and S3 come in 13b. The brief is
[13-tidy-in-place.md](13-tidy-in-place.md).

```
tungstate folder add nas:media
tungstate folder learn nas:media --write
tungstate plan nas:media
tungstate apply nas:media
tungstate undo nas:media
```

In the window, Organize's "Choose a folder…" opens the same place picker
Transfer, Sync and Duplicates use, so connections are offered.

---

## 1. Most of it was already there

The executor never knew it was local. It takes a `&dyn Backend`, and a
tidy only renames, makes and removes folders, and sets files aside inside
the folder. What tied Organize to this computer was all around it:
- `LocalBackend::new(path)` in every command that opened a folder;
- the rules file read and written with `std::fs`;
- folders and plans recorded by a bare path string.

## 2. Journal v15: a folder is a connection and a path

```sql
CREATE UNIQUE INDEX folders_place ON folders (IFNULL(connection, 0), root);
ALTER TABLE plans ADD COLUMN connection INTEGER REFERENCES connections (id);
```

- **`folders` is rebuilt rather than altered.** Its old `UNIQUE (root)`
  has to become "one folder per connection and path", and SQLite can't drop
  a column constraint in place. The migration copies the table into a new
  one and swaps it in. Every existing row gets `connection = NULL`, which
  means this computer, so nothing already recorded changes meaning.
  `folders_and_plans_from_before_v15_read_as_this_machines` opens a journal
  written at v14 and checks exactly that.
- **`IFNULL(connection, 0)`, because a unique index treats every `NULL` as
  different from every other.** Without it, the same local folder could be
  added twice. Connection ids start at 1, so 0 can't collide with a real
  one.

The journal's Rust API grew `_at` twins that take an `Endpoint`:
- `add_folder_at`, `folder_at`, `remove_folder_at`;
- `begin_plan_at`, `recent_plans_at`.

The old functions stay, each now a one-line call to its twin with
`Endpoint::local(root)`. So no caller had to change at once, and the ones
that never see a connection never will. That's the "new contract beside
the old one" rule: here it cost five one-liners, and nothing broke in
between.

## 3. The executor records where it worked

`apply_at` and `undo_at` take an `Endpoint` and record every operation with
`Location::within(at, path)`, which carries the connection. Before, every
operation was written with `Location::new(root, path)`, which hard-codes
"this computer". That bit Duplicates: a clean-up on the NAS was recorded as
a local folder literally named `nas:folder`. Duplicates now uses `apply_at`
too, so this fixes it as well.

A tidy on storage that can't rename (FTP, S3, an SFTP server without
`posix-rename`) is refused before anything is recorded, with a sentence
that says why. Before, it would have failed on every file in turn, after
the plan was already written down as begun.

## 4. The rules live in the folder, wherever it is

`.tungstate/policy.toml` is read with `open_read` and written with
`create_dir_all` and `create_write` through the folder's own backend. A
NAS folder's rules are on the NAS, where another computer running
Tungstate will find them, as you chose. On this computer it's the same
file in the same place as before.

## 5. One way to open a folder, in both front ends

- **Window.** `govern::open(target, journal)` turns what the window names
  (a path, or `nas:media`) into a `Spot`: the target, the `Endpoint`, and a
  backend from the connection factory. Every Organize command starts there.
- **Command line.** `folder::governed` does the same for `plan`, `apply`
  and `undo`, and a small `Place` does it for the `folder` subcommands.
  - A local path is still found by walking up to the nearest rules, so you
    can still run `plan` from inside a folder.
  - A connection path must name the folder itself. Walking up a NAS one
    level at a time would cost a round trip per level.

## 6. What the window does differently for a NAS folder

- **The folder list doesn't dial the NAS.** For each folder on this
  computer, the list checks whether it has rules and whether they load. For
  a NAS folder it says nothing (`has_rules` is `null`) and asks when you
  open it. A list that dialled every NAS would hang for a NAS that is off.
- **Opening a folder without rules shows its shape and the layouts**,
  instead of failing on a preview that has nothing to preview.
- **Past tidies** send back `nas:media`, not `media`, so Put it back
  reaches the place that was tidied.
- **NAS folders are never watched.** Keeping a folder in order while the
  window is open runs on file-system events, which a NAS doesn't send to
  this computer. Sweeping one in the background would be background work on
  your NAS. The watchers now skip folders on a connection.
- **A connection with organized folders can't be deleted.** It shows in the
  "still in use" list, as pairs and syncs do.

## 7. What is checked

- **Journal:**
  - the same path on two connections is two folders, and the same place
    twice is refused;
  - plans are found by their connection;
  - a v14 journal reads as local;
  - a connection with a folder is not deleted.
- **Executor:**
  - a tidy and its undo on a connection record that connection on every
    operation;
  - a tidy on storage that can't rename is refused with nothing recorded.
- **Command line, over an `fs` connection.** That's OpenDAL pointed at a
  folder here, so it's the remote code path with no server:
  - add, list, plan, apply, undo;
  - the connection is refused removal;
  - a folder with no rules says how to get some, and `init` writes them
    through the connection.
- **Against Samba, locally and in CI's SMB job:** a folder on the share is
  planned, tidied and put back, and its files move on the share.
- **The whole suite** in parallel and one at a time, plus the window's
  checks.

## What is not verified

- **Your NAS.** Samba in Docker stands in for it.
- **Big folders over a network.** Rules that look at a file's kind or EXIF
  read 8 or 64 KB of each file. The preview doesn't yet say what that will
  cost before it runs.
- **FTP and S3** are refused until 13b.
