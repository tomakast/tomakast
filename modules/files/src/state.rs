//! What the module keeps between runs, in `<data_dir>/state.json`: the screen's layout and
//! the files opened through the launcher.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::search::Opened;

/// The state file's name in the data folder.
const FILE: &str = "state.json";
/// At most this many opened paths are kept; the longest unopened go first.
pub const OPENED_CAP: usize = 2000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub layout: Layout,
    /// Paths opened through the launcher. Stored as a list, so a path that is not UTF-8
    /// survives (as its bytes).
    #[serde(default, with = "opened_list")]
    pub opened: HashMap<PathBuf, Opened>,
}

/// How the "Search Files" screen shows its files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    List,
    Grid,
}

impl Layout {
    pub fn flipped(self) -> Layout {
        match self {
            Layout::List => Layout::Grid,
            Layout::Grid => Layout::List,
        }
    }
}

impl State {
    /// The state in `dir`; a missing or unreadable file is the default state.
    pub fn load(dir: &Path) -> State {
        let path = dir.join(FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return State::default(),
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "files: cannot read the state file");
                return State::default();
            }
        };
        match serde_json::from_slice::<State>(&bytes) {
            Ok(mut state) => {
                state.trim();
                state
            }
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "files: the state file is damaged; starting afresh");
                State::default()
            }
        }
    }

    /// Writes the state to `dir` (created if missing) atomically: a temp file there, then a
    /// rename. On Unix the file is readable by its owner only (it lists opened paths).
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        write_private(&dir.join(FILE), &bytes)
    }

    /// Counts an open of `path` at `now` (Unix seconds).
    pub fn record_open(&mut self, path: &Path, now: i64) {
        let opened = self
            .opened
            .entry(path.to_path_buf())
            .or_insert(Opened { count: 0, last: 0 });
        opened.count = opened.count.saturating_add(1);
        opened.last = now;
        self.trim();
    }

    /// Drops the longest unopened paths past [`OPENED_CAP`].
    fn trim(&mut self) {
        let excess = self.opened.len().saturating_sub(OPENED_CAP);
        if excess == 0 {
            return;
        }
        let mut by_age: Vec<(i64, PathBuf)> = self
            .opened
            .iter()
            .map(|(path, o)| (o.last, path.clone()))
            .collect();
        by_age.select_nth_unstable(excess - 1);
        for (_, path) in &by_age[..excess] {
            self.opened.remove(path);
        }
    }
}

/// `opened` as a list of `{ path | path_bytes, count, last }`.
mod opened_list {
    use super::*;
    use serde::{Deserializer, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Entry {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path_bytes: Option<Vec<u8>>,
        count: u32,
        last: i64,
    }

    pub fn serialize<S: Serializer>(
        opened: &HashMap<PathBuf, Opened>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut entries: Vec<Entry> = opened
            .iter()
            .filter_map(|(path, o)| {
                let (path, path_bytes) = match path.to_str() {
                    Some(text) => (Some(text.to_string()), None),
                    None => (None, Some(raw(path)?)),
                };
                Some(Entry {
                    path,
                    path_bytes,
                    count: o.count,
                    last: o.last,
                })
            })
            .collect();
        // The same state, the same file.
        entries.sort_by(|a, b| (&a.path, &a.path_bytes).cmp(&(&b.path, &b.path_bytes)));
        entries.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<HashMap<PathBuf, Opened>, D::Error> {
        let entries = Vec::<Entry>::deserialize(d)?;
        Ok(entries
            .into_iter()
            .filter_map(|e| {
                let path = match (e.path, e.path_bytes) {
                    (Some(text), _) => PathBuf::from(text),
                    (None, Some(bytes)) => from_raw(bytes)?,
                    (None, None) => return None,
                };
                let opened = Opened {
                    count: e.count,
                    last: e.last,
                };
                Some((path, opened))
            })
            .collect())
    }

    /// The bytes of a path that is not UTF-8 (Unix); elsewhere such a path is not kept.
    fn raw(path: &Path) -> Option<Vec<u8>> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Some(path.as_os_str().as_bytes().to_vec())
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            None
        }
    }

    fn from_raw(bytes: Vec<u8>) -> Option<PathBuf> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            None
        }
    }
}

/// Writes `bytes` to `file` atomically (its folder created if missing): a temp file beside
/// it, then a rename. On Unix the file is readable by its owner only (it lists paths).
pub(crate) fn write_private(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    // The mode applies to a new file only: one a crash left behind goes first.
    match fs::remove_file(&tmp) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        _ => {}
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&tmp)
        .and_then(|mut file| file.write_all(bytes));
    written
        .and_then(|()| fs::rename(&tmp, file))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn state_round_trips() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("data/files");
        let mut state = State {
            layout: Layout::Grid,
            ..State::default()
        };
        state.record_open(Path::new("/h/a.txt"), 100);
        state.record_open(Path::new("/h/a.txt"), 200);
        state.record_open(Path::new("/h/b.txt"), 150);
        state.save(&dir).unwrap();
        let loaded = State::load(&dir);
        assert_eq!(loaded, state);
        assert_eq!(
            loaded.opened[Path::new("/h/a.txt")],
            Opened {
                count: 2,
                last: 200
            }
        );
        // Nothing but the state file is left behind.
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
    }

    /// The opened paths are private: the file is readable by its owner only, written fresh or
    /// over an older one that was not, and over a temp file a crash left behind.
    #[cfg(unix)]
    #[test]
    fn state_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let file = tmp.path().join("state.json");
        State::default().save(tmp.path()).unwrap();
        assert_eq!(mode(&file), 0o600);

        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let stale = tmp
            .path()
            .join(format!(".state.json.tmp-{}", std::process::id()));
        fs::write(&stale, b"half").unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();
        State::default().save(tmp.path()).unwrap();
        assert_eq!(mode(&file), 0o600);
        assert!(!stale.exists());
    }

    #[test]
    fn state_of_a_missing_file_is_the_default() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(State::load(&tmp.path().join("nowhere")), State::default());
    }

    #[test]
    fn state_survives_a_corrupt_file() {
        let tmp = TempDir::new().unwrap();
        for garbage in [
            &b"{not json"[..],
            b"",
            b"\xff\xfe\x00",
            b"[1, 2, 3]",
            br#"{"layout": "Diagonal"}"#,
        ] {
            fs::write(tmp.path().join("state.json"), garbage).unwrap();
            assert_eq!(State::load(tmp.path()), State::default());
        }
        // A file with only some fields keeps the rest at their defaults.
        fs::write(tmp.path().join("state.json"), r#"{"layout": "Grid"}"#).unwrap();
        assert_eq!(State::load(tmp.path()).layout, Layout::Grid);
    }

    #[test]
    fn opened_is_capped_at_2000() {
        let mut state = State::default();
        let path = |n: usize| PathBuf::from(format!("/h/{n}"));
        for n in 0..OPENED_CAP {
            state.record_open(&path(n), 1000 + n as i64);
        }
        // Opened again: now the latest, so not the next to go.
        state.record_open(&path(0), 10_000);
        state.record_open(&path(OPENED_CAP), 10_001);
        assert_eq!(state.opened.len(), OPENED_CAP);
        assert!(state.opened.contains_key(&path(0)));
        assert!(!state.opened.contains_key(&path(1)));
        assert!(state.opened.contains_key(&path(2)));
        assert!(state.opened.contains_key(&path(OPENED_CAP)));
    }

    #[test]
    fn an_oversized_file_is_capped_on_load() {
        let tmp = TempDir::new().unwrap();
        let mut state = State::default();
        for n in 0..OPENED_CAP + 5 {
            state.opened.insert(
                PathBuf::from(format!("/h/{n}")),
                Opened {
                    count: 1,
                    last: n as i64,
                },
            );
        }
        state.save(tmp.path()).unwrap();
        let loaded = State::load(tmp.path());
        assert_eq!(loaded.opened.len(), OPENED_CAP);
        assert!(!loaded.opened.contains_key(Path::new("/h/4")));
        assert!(loaded.opened.contains_key(Path::new("/h/5")));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_survives() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let tmp = TempDir::new().unwrap();
        let odd = PathBuf::from(OsStr::from_bytes(b"/h/caf\xe9.txt"));
        let mut state = State::default();
        state.record_open(&odd, 5);
        state.record_open(Path::new("/h/plain.txt"), 6);
        state.save(tmp.path()).unwrap();
        assert_eq!(State::load(tmp.path()), state);
    }
}
