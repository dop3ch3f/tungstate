# Slice 12c tour: cloud storage, without a browser sign-in

What Tungstate can reach today with keys and passwords, made easy to find,
plus the cloud drives whose own apps keep a folder on this computer. Direct
sign-in to Google Drive, OneDrive and Dropbox is shown as coming soon.

---

## 1. What was decided, and why the rest waits

- **Google Drive direct needs Google's review.** Full access to someone's
  Drive is a "restricted" permission. Production use needs Google's
  security review, which takes weeks and has a yearly cost. Until then, an
  unreviewed app is limited to 100 test users.
- **OneDrive and Dropbox direct are free to register.** They still need a
  browser sign-in built into the app.
- **All three appear in the connection picker as "Soon"**, each pointing at
  what works meanwhile.

## 2. Services by name

The S3 and WebDAV connections already reached most cloud storage. What was
missing was knowing the address and where the keys are made.

`kinds.ts` now lists the services by name, from each provider's own
documentation:

- **S3:** Backblaze B2, Cloudflare R2, Wasabi, iDrive e2, Storj,
  DigitalOcean Spaces, Hetzner Object Storage, Synology C2, MEGA S4,
  Scaleway, AWS, or another.
- **WebDAV:** a NAS, Nextcloud, ownCloud, pCloud (US and EU), Koofr,
  Yandex Disk, Hetzner Storage Box, Infomaniak kDrive, Seafile, 4shared,
  OpenDrive, or another.

Choosing one in the form does two things:
- **Fills in its address**, or shows the pattern to complete, such as
  Nextcloud's `/remote.php/dav/files/USERNAME`.
- **Says where the keys are made**, and any catch. B2's master key doesn't
  work over S3. R2 shows its secret once. pCloud's WebDAV stops working
  when two-step sign-in is on. Koofr and Yandex need an app password.

Some services are deliberately left out:
- **Box:** its WebDAV shut down in 2023.
- **Proton Drive and MEGA's ordinary drive:** they have none.
- **ownCloud Infinite Scale:** it won't take a password.

Nothing in the engine changed for this part. It's the same S3 and WebDAV
code.

## 3. Cloud drives' own folders

`clouds.rs` finds the folders the drives' apps keep on this computer and
offers them in the place picker under "Cloud drives":
- **macOS:** `~/Library/CloudStorage`, where the file-provider apps put
  Google Drive (its `My Drive`), OneDrive, Dropbox and Box. Also iCloud
  Drive's own folder.
- **Windows:** `OneDrive`, `OneDrive - <organisation>`, `Dropbox`, `Box`
  and `iCloudDrive` in the home folder.

Two drives of one service, such as a personal and a work OneDrive, are told
apart by account. It is a function of a home folder, so its tests build
fake homes instead of reading yours.

## 4. Online-only files are never downloaded just to look

These apps keep many files in the cloud, with only a placeholder here.
Reading one downloads it, silently and in full. Duplicates reads to
compare, and Organize reads a file's first 64 KB to learn its kind, so
pointing either at a drive folder could pull down the whole drive.

- **`Meta.online_only`** is read off the metadata the system already
  returns, so it costs no extra call:
  - **macOS:** a file-provider placeholder carries the `SF_DATALESS` flag.
  - **Windows:** a cloud placeholder is marked "recall on data access",
    "recall on open", or offline.

  Both come from the standard library's per-platform `MetadataExt`, so no
  new dependency and no `unsafe`.
- **Nothing that only reads to look reads a placeholder:**
  - sniffing its kind or EXIF (`fill` returns early);
  - duplicate candidates;
  - the "looks alike" pass.
- **Both screens say how many files were left out**, and how to include
  them: make them available offline in the drive's app. So does
  `tungstate dedupe`.
- **A transfer still downloads a placeholder.** Copying a file somewhere
  else is the point of a transfer, so reading it there is expected.

## 5. What is checked

- The drive-folder finder against fake homes:
  - macOS file providers;
  - iCloud;
  - two OneDrives told apart;
  - the Windows folders;
  - a home with none.
- A placeholder is never read to learn its kind, and only the other file
  costs a read.
- A placeholder is never a duplicate candidate.
- An ordinary local file is not a placeholder.
- The whole suite in parallel and one at a time, and the window's checks.

## What is not verified

- **A real placeholder.** No test here can make the system mark a file as
  online only; that takes a drive's app. The flag values come from Apple's
  and Microsoft's headers, and the code reading them builds on macOS and
  (in CI) Windows.
- **Every service's address against a real account.** They come from each
  provider's own pages, and the research report notes the few that were
  from memory.
