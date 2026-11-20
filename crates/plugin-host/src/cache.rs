//! The precompiled-component cache in `<data-dir>/cache/plugins/`.
//!
//! An entry is named `<sha256(wasm)>-<engine compat key>.cwasm`, so a new wasmtime version or
//! engine config never matches an old entry. Entries are written only by the compile child
//! ([`crate::compile`]); the daemon only reads them, without following symlinks.

use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use wasmtime::Engine;

/// Temp files older than this were left behind by a crashed daemon or compile child.
pub const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// Prefix of the compile inputs the daemon writes into the cache dir.
pub(crate) const INPUT_PREFIX: &str = ".input-";

/// Largest cache entry the daemon reads.
const MAX_ENTRY_BYTES: u64 = 8 * crate::package::MAX_WASM_BYTES;

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// Opens the cache in `dir` and removes the compile inputs (`.input-*`) and unfinished
    /// outputs (`*.tmp`) that are older than [`STALE_AFTER`]. The sweep is best effort.
    pub fn new(dir: PathBuf) -> Cache {
        sweep(&dir, STALE_AFTER);
        Cache { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The entry for `wasm_hash` compiled by an engine compatible with `engine`. Always a
    /// direct child of [`Cache::dir`].
    pub fn path_for(&self, wasm_hash: &[u8; 32], engine: &Engine) -> PathBuf {
        self.dir.join(format!(
            "{}-{}.cwasm",
            hex(wasm_hash),
            hex(&compat_key(engine))
        ))
    }
}

fn sweep(dir: &Path, older_than: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.starts_with(INPUT_PREFIX) || name.ends_with(".tmp")) {
            continue;
        }
        // `DirEntry::metadata` does not follow symlinks: a stale link is removed, not its target.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let stale = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > older_than);
        if !meta.is_dir() && stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What [`read_entry`] found.
pub(crate) enum Entry {
    Missing,
    /// Present but not usable: a symlink, not a regular file, too large or unreadable.
    Unusable(String),
    Bytes(Vec<u8>),
}

/// Reads a cache entry into memory without following a symlink at `path` (`O_NOFOLLOW`;
/// on Windows the reparse point itself is opened), so that what is checked is what is read.
/// On Unix the open is also non-blocking (`O_NONBLOCK`), so a FIFO at `path` is refused as
/// not a regular file instead of waiting for a writer.
pub(crate) fn read_entry(path: &Path) -> Entry {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Entry::Missing,
        Err(e) => return Entry::Unusable(format!("cannot open: {e}")),
    };
    match read_regular(file) {
        Ok(bytes) => Entry::Bytes(bytes),
        Err(problem) => Entry::Unusable(problem),
    }
}

fn read_regular(file: File) -> Result<Vec<u8>, String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(format!("larger than {MAX_ENTRY_BYTES} bytes"));
    }
    Ok(bytes)
}

/// A stable digest of `Engine::precompile_compatibility_hash`: equal for engines that can load
/// each other's precompiled components.
fn compat_key(engine: &Engine) -> [u8; 16] {
    struct ShaHasher(Sha256);
    impl Hasher for ShaHasher {
        fn finish(&self) -> u64 {
            unreachable!("only the digest is used")
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0.update(bytes);
        }
    }
    let mut hasher = ShaHasher(Sha256::new());
    engine.precompile_compatibility_hash().hash(&mut hasher);
    let digest = hasher.0.finalize();
    let mut key = [0u8; 16];
    key.copy_from_slice(&digest[..16]);
    key
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Parses 64 lowercase or uppercase hex digits.
pub(crate) fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::engine_config;

    #[test]
    fn path_is_inside_dir_and_keyed_by_hash() {
        let engine = Engine::new(&engine_config()).unwrap();
        let cache = Cache::new(PathBuf::from("/cache"));
        let a = cache.path_for(&[1; 32], &engine);
        let b = cache.path_for(&[2; 32], &engine);
        assert_eq!(a.parent(), Some(Path::new("/cache")));
        assert_ne!(a, b);
        assert!(a.to_str().unwrap().ends_with(".cwasm"));
        assert_eq!(
            a,
            cache.path_for(&[1; 32], &Engine::new(&engine_config()).unwrap())
        );
    }

    #[test]
    fn opening_the_cache_sweeps_stale_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        let write = |name: &str, aged: bool| {
            let path = dir.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            if aged {
                File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(old)
                    .unwrap();
            }
            path
        };
        let stale_input = write(".input-abc.wasm", true);
        let stale_tmp = write("e.cwasm.123.tmp", true);
        let fresh_input = write(".input-def.wasm", false);
        let fresh_tmp = write("e.cwasm.456.tmp", false);
        let old_entry = write("e.cwasm", true);
        let old_other = write("notes.txt", true);

        let cache = Cache::new(dir.path().to_owned());
        assert_eq!(cache.dir(), dir.path());
        assert!(!stale_input.exists());
        assert!(!stale_tmp.exists());
        for kept in [fresh_input, fresh_tmp, old_entry, old_other] {
            assert!(kept.exists(), "{} was removed", kept.display());
        }
        // A missing dir is fine.
        Cache::new(dir.path().join("missing"));
    }

    #[cfg(unix)]
    #[test]
    fn read_entry_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"bytes").unwrap();
        let link = dir.path().join("link.cwasm");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(read_entry(&target), Entry::Bytes(b) if b == b"bytes"));
        assert!(matches!(read_entry(&link), Entry::Unusable(_)));
        assert!(matches!(read_entry(dir.path()), Entry::Unusable(_)));
        assert!(matches!(
            read_entry(&dir.path().join("none")),
            Entry::Missing
        ));
    }

    /// A FIFO at the entry path must not block the reader: opening one for reading waits for
    /// a writer unless it is opened non-blocking.
    #[cfg(unix)]
    #[test]
    fn read_entry_does_not_block_on_a_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("entry.cwasm");
        let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let path = fifo.clone();
        std::thread::spawn(move || {
            let unusable = matches!(read_entry(&path), Entry::Unusable(_));
            let _ = tx.send(unusable);
        });
        let answer = rx.recv_timeout(Duration::from_secs(2));
        if answer.is_err() {
            // Unblock the stuck reader so the test process can exit.
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
        }
        assert_eq!(answer, Ok(true), "read_entry blocked on a FIFO");
    }

    #[test]
    fn hex_roundtrip() {
        let bytes: [u8; 32] = std::array::from_fn(|i| (i * 7) as u8);
        assert_eq!(parse_hex32(&hex(&bytes)), Some(bytes));
        assert_eq!(parse_hex32(&hex(&bytes).to_uppercase()), Some(bytes));
        assert_eq!(parse_hex32("00"), None);
        assert_eq!(parse_hex32(&"zz".repeat(32)), None);
        assert_eq!(parse_hex32(&"é".repeat(32)), None);
    }
}
