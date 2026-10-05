# Slice 8f tour: one listing, not one per file

The rest of 8e. Its tour ended on the next saving to make: a scan listed
each folder and then asked again about every file in it. The 8e tour is
[08e-tour.md](08e-tour.md).

## The numbers

The same 5,000-file folder as 8e, behind the same fake network (20 ms a
request, 50 MB/s). The 8e row is that tour's last one.

| | requests | time | warm requests | warm time |
|---|---:|---:|---:|---:|
| After 8e | 13,737 | 3.0 min | 5,683 | 2.3 min |
| After 8f | 8,396 | 58 s | 342 | 8 s |

The listing went from 5,683 requests to 342, one per folder. A scan of a
folder whose files have not changed is now almost all listing, so the warm
scan gains the most. The samples are unchanged: 8,054 requests and 528 MB,
as before.

Direct SMB was not timed this time. The Samba container keeps its files in a
Docker volume, and copying the folder in would have written its 144 GB of
mostly empty videos out in full. The request counts do not depend on the
network, and a test against Samba checks the part this rests on (below).

## Who answered twice, and why

`survey_at` walks with `read_dir`, which already gives each entry's size,
time and kind. It then called `gather` on each entry, which starts with a
`stat`. That was harmless when every folder was local. Over a network:

- **SMB:** one round trip per file. The listing and the `stat` both come
  out of the same `meta` function, with the same three fields.
- **FTP:** much worse. `OpenDAL` answers an FTP `stat` by listing the parent
  folder and picking out one name. A folder of 1,000 files was listed 1,001
  times.
- **A mounted share:** `LocalBackend::read_dir` already calls `stat` for
  each entry, so the second one was a pure repeat.

## A question each storage answers for itself

```rust
fn listing_is_complete(&self) -> bool {
    false
}
```

This is a **defaulted trait method**, like `read_prefix` and `read_range`
before it. Every `Backend` gets it for free, and the ones that can say more
override it.

`false` is the safe default: a scan that does not trust the listing asks
again, which is slower and never wrong. That matters for wrappers such as
`Counted` and the benchmark's `Slow`. A wrapper that forgot to forward
`listing_is_complete` would fall back to the default and lose the speed, but
never the accuracy. Both forward it.

Who says yes, and who does not:

- **Local, SMB, FTP and FTPS:** yes.
- **S3:** no. Its listing gives times to the millisecond and its `HEAD` to
  the second. Taking the listing would change every file's time once, which
  would throw away the duplicate pass's remembered digests and could make a
  sync think everything changed.
- **`OpenDAL`'s `fs`:** no. Its `stat` follows links and its listing does
  not.

## A setter, not a new argument

`OpendalBackend::new` is public and already takes five arguments. Rather than
add a sixth and change every caller, FTP opts in with:

```rust
OpendalBackend::new(operator, prefix, anchor, name, networked)
    .with_complete_listing(complete_listing)
```

`with_complete_listing` takes `self` by value and returns it, the same
builder shape as `tauri::Builder` in 9f. Callers that never call it get
`false`.

## The scan

```rust
let mut attrs = if trust_listing {
    let mut attrs = described(&relative_string(&entry.path)?, entry.meta, taken);
    fill(backend, &mut attrs, tier)?;
    attrs
} else {
    gather(backend, &entry.path, tier)?
};
```

`described` is the part of `gather` that turns a `Meta` into `Attributes`,
lifted out so both paths share it. `fill` still does any reading the policy
asks for, so a tidy that sniffs file types still sniffs them. Duplicates,
tidying and sync all scan with `survey_at`, so all three get faster.

## What is checked

- **`a_complete_listing_is_taken_as_listed_and_says_what_stat_would`**
  scans one folder twice, once trusting the listing and once not. It
  counts the `stat`s (none against twelve), and checks that the two results
  are identical, EXIF included. It fails with the shortcut switched off.
- **`a_listing_says_what_a_stat_would`** against real Samba, and
  **`a_listing_over_ftp_says_what_a_stat_would`** against a real FTP server:
  every listed entry equals its own `stat`. If a server's listing ever
  drifts from its `stat`, these fail rather than the scan quietly going
  wrong.
- The whole suite: 718 tests, in parallel and one at a time. Also the full
  FTP and SMB suites against local servers.

## What is not done, and not verified

- **Not measured on the NAS**, as before.
- **FTP was not timed.** The saving there is the largest in principle,
  because each `stat` re-listed the whole folder, but no benchmark covers
  FTP yet.
- **S3 still asks per file.** Comparing listing times at the second would
  let it skip that too. It touches what counts as "changed", so it is left
  for when S3 speed matters.
