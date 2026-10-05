# Slice 13: tidy a folder on the NAS, in place

**Goal:** Organize works on a folder on any connection, not only on a
folder this computer can open. Files move on the NAS itself, nothing is
downloaded to be filed, and Put it back works there as it does locally.

Two parts, each tagged when green:
- **13a:** connections that rename on the server: SMB, WebDAV and SFTP.
- **13b:** FTP and S3, which cannot rename, so a move is a copy, a check and
  a delete.

**Runnable outcome:**

```
tungstate folder add nas:media          # a folder on a connection
tungstate folder learn nas:media        # its shape, written as rules on the NAS
tungstate plan --folder nas:media
tungstate apply --folder nas:media
tungstate undo --last 1
```

In the window, Organize's "Choose a folder" offers connections, as
Transfer, Sync and Duplicates already do.

---

## What is already there

- **The executor takes any `Backend`.** It only renames, makes and removes
  folders, and sets aside into `.tungstate-quarantine` inside the folder. It
  never needs the desktop trash for a tidy.
- **Duplicates already sets files aside on a NAS and puts them back.** It
  goes through the same executor.
- **Scans over a network are cheap since 8e and 8f.** Rules that only need
  names, sizes and dates read nothing. Rules that need a file's kind or EXIF
  read 8 KB or 64 KB of each file, which the preview will say before
  anything happens.

## What is missing, and the decisions

- **A remote folder is recorded by its connection (chosen).**
  - **Journal v15.** `folders` and `plans` gain a `connection` column, as
    `ops` already has. `folders` is rebuilt, because "one row per path"
    becomes "one row per connection and path". Existing rows are local
    (`NULL`), so nothing already recorded changes meaning.
  - **Apply, undo and recovery take an `Endpoint`**, a connection and a
    path, instead of a path string. Every operation then records its
    connection. This also fixes Duplicates, which today records `nas:folder`
    as if it were a local path.
  - **A connection that organized folders still use cannot be deleted**,
    just as one with pairs or syncs cannot.
- **The rules live on the NAS, in the folder (chosen).** That's
  `.tungstate/policy.toml`, as for a local folder, read and written through
  the connection. Another computer running Tungstate sees the same rules.
- **Every kind of connection can be tidied (chosen).**
  - SMB, WebDAV and SFTP rename on the server, so 13a needs nothing new
    from the executor.
  - FTP and S3 cannot rename. In 13b a move becomes copy, check, delete,
    journalled so a crash between the copy and the delete is finished or
    undone on the next run. S3 copies on the server; FTP passes the bytes
    through this computer. The preview says how much will be copied.
- **Remote folders are not watched.** Keeping a folder in order while the
  window is open uses file-system events, which a NAS doesn't send to this
  computer. A remote folder is tidied when asked, which also respects "no
  background work".

## Tests

- Journal: the v15 migration keeps existing folders and plans as local. The
  same path on two connections is two folders.
- Executor: a tidy and its undo on a backend that is not local record the
  connection on every operation.
- Against Samba (CI's existing SMB job): learn, plan, apply and undo through
  the binary on a share. The rules file is on the share.
- 13b: against FTP and S3, a tidy and its undo, and a crash between copy and
  delete, recovered.
- Window: Organize picking a connection, and the preview's cost line.
