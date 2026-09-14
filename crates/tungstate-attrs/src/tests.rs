//! The gatherer, against a temp directory and a counting backend.
//!
//! The important tests are the counting ones: they prove the tiers are a
//! real read budget rather than decoration.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use tungstate_backend::local::LocalBackend;
use tungstate_backend::{Backend, Entry, Meta, Result as BackendResult, RootToken, WriteFinish};
use tungstate_core::Policy;

use super::*;

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

/// A backend that leaves `read_prefix` to the trait's default, so the
/// default implementation is exercised on its own.
struct Defaulted(LocalBackend);

impl Backend for Defaulted {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        self.0.capabilities()
    }
    fn root_token(&self) -> BackendResult<RootToken> {
        self.0.root_token()
    }
    fn stat(&self, path: &Path) -> BackendResult<Meta> {
        self.0.stat(path)
    }
    fn read_dir(&self, path: &Path) -> BackendResult<Vec<Entry>> {
        self.0.read_dir(path)
    }
    fn open_read(&self, path: &Path) -> BackendResult<Box<dyn Read + Send>> {
        self.0.open_read(path)
    }
    fn create_write(&self, path: &Path) -> BackendResult<Box<dyn WriteFinish>> {
        self.0.create_write(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> BackendResult<()> {
        self.0.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> BackendResult<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> BackendResult<()> {
        self.0.remove_dir(path)
    }
    fn create_dir_all(&self, path: &Path) -> BackendResult<()> {
        self.0.create_dir_all(path)
    }
}

/// Records every read: which prefix lengths were asked for, and how many
/// times the whole file was opened.
struct Counting {
    inner: LocalBackend,
    prefixes: Mutex<Vec<u64>>,
    opens: Mutex<usize>,
}

impl Counting {
    fn new(root: &Path) -> Self {
        Self {
            inner: LocalBackend::new(root.to_path_buf()),
            prefixes: Mutex::new(Vec::new()),
            opens: Mutex::new(0),
        }
    }
    fn prefixes(&self) -> Vec<u64> {
        self.prefixes.lock().unwrap().clone()
    }
    fn opens(&self) -> usize {
        *self.opens.lock().unwrap()
    }
}

impl Backend for Counting {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        self.inner.capabilities()
    }
    fn root_token(&self) -> BackendResult<RootToken> {
        self.inner.root_token()
    }
    fn stat(&self, path: &Path) -> BackendResult<Meta> {
        self.inner.stat(path)
    }
    fn read_dir(&self, path: &Path) -> BackendResult<Vec<Entry>> {
        self.inner.read_dir(path)
    }
    fn open_read(&self, path: &Path) -> BackendResult<Box<dyn Read + Send>> {
        *self.opens.lock().unwrap() += 1;
        self.inner.open_read(path)
    }
    fn read_prefix(&self, path: &Path, len: u64) -> BackendResult<Vec<u8>> {
        self.prefixes.lock().unwrap().push(len);
        self.inner.read_prefix(path, len)
    }
    fn create_write(&self, path: &Path) -> BackendResult<Box<dyn WriteFinish>> {
        self.inner.create_write(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> BackendResult<()> {
        self.inner.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> BackendResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> BackendResult<()> {
        self.inner.remove_dir(path)
    }
    fn create_dir_all(&self, path: &Path) -> BackendResult<()> {
        self.inner.create_dir_all(path)
    }
}

/// An `OpenDAL` backend over `dir`, through the `fs` service and the factory.
fn over_opendal(dir: &Path) -> Box<dyn Backend> {
    use tungstate_journal::{Endpoint, Journal, NewConnection, Scheme};
    let journal = Journal::open_in_memory().unwrap();
    let id = journal
        .create_connection(&NewConnection {
            name: "scratch".to_string(),
            scheme: Scheme::Fs,
            host: None,
            port: None,
            username: None,
            root: dir.to_string_lossy().into_owned(),
            options: BTreeMap::new(),
        })
        .unwrap();
    tungstate_backend_opendal::open(
        &Endpoint::remote(id, std::path::PathBuf::new()),
        &journal,
        &tungstate_secret::MemoryStore::new(),
    )
    .unwrap()
}

fn policy(text: &str) -> Policy {
    Policy::parse(text).expect("policy should load").policy
}

#[test]
fn the_default_read_prefix_and_both_overrides_agree() {
    let dir = dir();
    let body: Vec<u8> = (0..100_000_u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.path().join("blob.bin"), &body).unwrap();

    let local = LocalBackend::new(dir.path().to_path_buf());
    let defaulted = Defaulted(LocalBackend::new(dir.path().to_path_buf()));
    let remote = over_opendal(dir.path());

    for len in [0_u64, 1, 8 * 1024, 64 * 1024, 100_000, 1 << 20] {
        let expected = &body[..usize::try_from(len).unwrap().min(body.len())];
        for (name, backend) in [
            ("local", &local as &dyn Backend),
            ("default", &defaulted),
            ("opendal", remote.as_ref()),
        ] {
            let got = backend.read_prefix(Path::new("blob.bin"), len).unwrap();
            assert_eq!(got, expected, "{name} disagreed at len {len}");
        }
    }
}

#[test]
fn a_stat_only_policy_never_opens_the_file() {
    let dir = dir();
    std::fs::write(dir.path().join("big.zip"), vec![0_u8; 1 << 20]).unwrap();
    let counting = Counting::new(dir.path());

    let policy = policy(
        "[folder]\nname = \"x\"\n[[rule]]\nname = \"z\"\npath = \"Z/{ext}\"\nmatch = { ext = \"zip\", size = \"> 1KiB\" }\n",
    );
    assert_eq!(policy.required_tier(), Tier::Stat);

    let attrs = gather(&counting, Path::new("big.zip"), policy.required_tier()).unwrap();
    assert_eq!(attrs.size, 1 << 20);
    assert_eq!(attrs.mime, None);
    assert_eq!(counting.opens(), 0, "stat tier must not open the file");
    assert!(
        counting.prefixes().is_empty(),
        "stat tier must not read a prefix"
    );
}

#[test]
fn a_mime_only_policy_reads_eight_kib_and_no_more() {
    let dir = dir();
    let mut body = vec![0_u8; 4 << 20];
    body[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x18]);
    body[4..12].copy_from_slice(b"ftypmp42");
    std::fs::write(dir.path().join("clip.mp4"), &body).unwrap();
    let counting = Counting::new(dir.path());

    let policy = policy(
        "[folder]\nname = \"x\"\n[[rule]]\nname = \"v\"\npath = \"Video\"\nmatch = { mime = \"video/*\" }\n",
    );
    assert_eq!(policy.required_tier(), Tier::Head);

    let attrs = gather(&counting, Path::new("clip.mp4"), policy.required_tier()).unwrap();
    assert_eq!(attrs.mime.as_deref(), Some("video/mp4"));
    assert_eq!(counting.prefixes(), vec![Tier::HEAD_BYTES]);
    assert_eq!(counting.opens(), 0, "head tier must not stream the file");
    assert!(attrs.exif.is_empty());
    assert_eq!(attrs.hash, None);
}

#[test]
fn an_exif_policy_reads_sixty_four_kib_and_a_hash_policy_reads_it_all() {
    let dir = dir();
    let mut jpeg = jpeg_with_exif_date("2023:12:25 08:30:00");
    jpeg.extend(std::iter::repeat_n(0_u8, 200_000));
    std::fs::write(dir.path().join("IMG_0001.JPG"), &jpeg).unwrap();

    let counting = Counting::new(dir.path());
    let attrs = gather(&counting, Path::new("IMG_0001.JPG"), Tier::Meta).unwrap();
    assert_eq!(attrs.mime.as_deref(), Some("image/jpeg"));
    assert_eq!(
        attrs.exif.get("DateTimeOriginal").map(String::as_str),
        Some("2023-12-25 08:30:00")
    );
    assert_eq!(counting.prefixes(), vec![Tier::META_BYTES]);
    assert_eq!(counting.opens(), 0);

    let counting = Counting::new(dir.path());
    let attrs = gather(&counting, Path::new("IMG_0001.JPG"), Tier::Whole).unwrap();
    assert_eq!(counting.opens(), 1);
    assert_eq!(
        attrs.hash.as_deref(),
        Some(blake3::hash(&jpeg).to_hex().as_str())
    );
    assert_eq!(attrs.mime.as_deref(), Some("image/jpeg"));
    assert!(attrs.exif.contains_key("DateTimeOriginal"));
}

#[test]
fn the_camera_date_reaches_the_classifier_as_a_date() {
    let dir = dir();
    std::fs::write(
        dir.path().join("IMG_0001.JPG"),
        jpeg_with_exif_date("2023:12:25 08:30:00"),
    )
    .unwrap();
    let backend = LocalBackend::new(dir.path().to_path_buf());
    let policy = policy(
        "[folder]\nname = \"x\"\n[[rule]]\nname = \"p\"\npath = \"{d:%Y/%m}\"\n\
         vars.d = { from = [\"exif.DateTimeOriginal\", \"mtime\"], tz = \"utc\" }\n",
    );
    let attrs = gather(&backend, Path::new("IMG_0001.JPG"), policy.required_tier()).unwrap();
    match policy.explain(&attrs).outcome {
        tungstate_core::Outcome::Routed { destination, .. } => {
            assert_eq!(destination, "2023/12/IMG_0001.JPG");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn mime_falls_back_to_the_extension_and_then_to_a_text_heuristic() {
    assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0], "bin"), "image/jpeg");
    assert_eq!(sniff_mime(b"hello\n", "txt"), "text/plain");
    assert_eq!(sniff_mime(b"# Title\n", "md"), "text/markdown");
    assert_eq!(sniff_mime(b"just words, no extension", ""), "text/plain");
    assert_eq!(
        sniff_mime(&[0, 1, 2, 3, 4], "xyz"),
        "application/octet-stream"
    );
    assert_eq!(sniff_mime(&[], "xyz"), "application/octet-stream");
}

#[test]
fn a_directory_or_a_link_is_described_but_never_read() {
    let dir = dir();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let counting = Counting::new(dir.path());
    let attrs = gather(&counting, Path::new("sub"), Tier::Whole).unwrap();
    assert!(attrs.is_dir);
    assert_eq!(counting.opens(), 0);
    assert!(counting.prefixes().is_empty());
}

#[test]
fn paths_are_relative_with_forward_slashes() {
    let dir = dir();
    std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
    std::fs::write(dir.path().join("a/b/c.txt"), b"x").unwrap();
    let backend = LocalBackend::new(dir.path().to_path_buf());
    let attrs = gather(
        &backend,
        &Path::new("a").join("b").join("c.txt"),
        Tier::Stat,
    )
    .unwrap();
    assert_eq!(attrs.parent, "a/b");
    assert_eq!(attrs.name, "c.txt");
    assert_eq!(attrs.relative_path(), "a/b/c.txt");
    assert!(attrs.mtime.is_some());

    let error = gather(&backend, Path::new("../c.txt"), Tier::Stat).unwrap_err();
    assert!(matches!(error, GatherError::BadPath { .. }));
    let error = gather(&backend, Path::new("missing.txt"), Tier::Stat).unwrap_err();
    assert!(matches!(error, GatherError::Backend { .. }));
}
