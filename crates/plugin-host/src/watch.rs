//! Watching the plugins folder and `config.toml`, and the debounce in front of the plugin
//! manager: a plugin folder is read only once it has been quiet for [`QUIET`], so a folder
//! that is still being copied is never loaded half-written.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use notify::{EventKind, RecursiveMode, Watcher as _};

/// How long a plugin folder (or `config.toml`) must have been quiet before it is read.
pub const QUIET: Duration = Duration::from_millis(500);

/// What changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// Something inside the plugin folder with this name, or the folder itself.
    Folder(String),
    /// `config.toml`.
    Config,
    /// Events were lost (the kernel queue overflowed): everything is scanned again.
    All,
}

/// Per-key quiet timers: a key is due once [`QUIET`] has passed since its last change.
#[derive(Debug, Default)]
pub struct Debouncer {
    last_change: HashMap<Key, Instant>,
}

impl Debouncer {
    /// Records a change of `key` at `now`; it restarts the key's quiet time.
    pub fn touch(&mut self, key: Key, now: Instant) {
        self.last_change.insert(key, now);
    }

    /// When the next key comes due; `None` when nothing is waiting.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.last_change.values().min().map(|at| *at + QUIET)
    }

    /// Takes the keys that have been quiet for [`QUIET`] at `now`.
    pub fn take_due(&mut self, now: Instant) -> Vec<Key> {
        let mut due: Vec<Key> = self
            .last_change
            .iter()
            .filter(|(_, at)| now.saturating_duration_since(**at) >= QUIET)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &due {
            self.last_change.remove(key);
        }
        // `All` first: the full scan makes the other keys' work moot but harmless.
        due.sort_by_key(|key| !matches!(key, Key::All));
        due
    }
}

/// The notify watcher on the plugins folder (recursive) and on the directory of
/// `config.toml` (not recursive, filtered on the file name: editors replace the file). Dropping
/// it stops watching.
pub struct Watch {
    _watcher: notify::RecommendedWatcher,
}

impl Watch {
    /// Calls `on_change` with the [`Key`] of every content change, on the watcher's thread.
    /// Access events are ignored: reading a plugin folder must not count as a change of it.
    pub fn start(
        plugins_dir: &Path,
        config_file: &Path,
        on_change: impl Fn(Key) + Send + 'static,
    ) -> anyhow::Result<Watch> {
        let filter = Filter::new(plugins_dir, config_file);
        let mut watcher = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            for key in filter.keys(ev) {
                on_change(key);
            }
        })
        .context("starting the file watcher")?;
        watcher
            .watch(plugins_dir, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", plugins_dir.display()))?;
        if let Some(dir) = config_file.parent()
            && let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive)
        {
            tracing::warn!(target: "plugin", "cannot watch {}: {e}; preference changes need a restart", dir.display());
        }
        Ok(Watch { _watcher: watcher })
    }
}

struct Filter {
    plugins_dir: PathBuf,
    config_file: PathBuf,
}

impl Filter {
    /// notify makes a relative watch path absolute against the current directory before it
    /// reports events, so the filter compares against the same absolute forms.
    fn new(plugins_dir: &Path, config_file: &Path) -> Filter {
        let absolute = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
        Filter {
            plugins_dir: absolute(plugins_dir),
            config_file: absolute(config_file),
        }
    }

    fn keys(&self, ev: notify::Result<notify::Event>) -> Vec<Key> {
        let ev = match ev {
            Ok(ev) => ev,
            Err(e) => {
                tracing::warn!(target: "plugin", "file watcher error: {e}");
                return vec![Key::All];
            }
        };
        if ev.need_rescan() {
            return vec![Key::All];
        }
        if !matches!(
            ev.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
        ) {
            return Vec::new();
        }
        let root_removed =
            matches!(ev.kind, EventKind::Remove(_)) && ev.paths.contains(&self.plugins_dir);
        if root_removed {
            // The scan finds the folder gone and says so; the watch on it is lost.
            return vec![Key::All];
        }
        ev.paths.iter().filter_map(|path| self.key(path)).collect()
    }

    fn key(&self, path: &Path) -> Option<Key> {
        if path == self.config_file {
            return Some(Key::Config);
        }
        let rel = path.strip_prefix(&self.plugins_dir).ok()?;
        match rel.components().next()? {
            Component::Normal(name) => Some(Key::Folder(name.to_str()?.to_owned())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_due_after_quiet_time_since_its_last_change() {
        let t0 = Instant::now();
        let mut d = Debouncer::default();
        assert_eq!(d.next_deadline(), None);
        d.touch(Key::Folder("a".into()), t0);
        d.touch(Key::Folder("a".into()), t0 + Duration::from_millis(300));
        assert_eq!(d.next_deadline(), Some(t0 + Duration::from_millis(800)));
        assert!(d.take_due(t0 + Duration::from_millis(799)).is_empty());
        assert_eq!(
            d.take_due(t0 + Duration::from_millis(800)),
            [Key::Folder("a".into())]
        );
        assert_eq!(d.next_deadline(), None);
    }

    #[test]
    fn keys_wait_independently_and_all_comes_first() {
        let t0 = Instant::now();
        let mut d = Debouncer::default();
        d.touch(Key::Folder("a".into()), t0);
        d.touch(Key::Folder("b".into()), t0 + Duration::from_millis(400));
        d.touch(Key::All, t0 + Duration::from_millis(100));
        assert_eq!(
            d.take_due(t0 + Duration::from_millis(600)),
            [Key::All, Key::Folder("a".into())]
        );
        assert_eq!(d.next_deadline(), Some(t0 + Duration::from_millis(900)));
    }

    #[test]
    fn events_map_to_folders_and_the_config_file() {
        let f = Filter {
            plugins_dir: "/d/plugins".into(),
            config_file: "/c/config.toml".into(),
        };
        let event = |kind, path: &str| Ok(notify::Event::new(kind).add_path(path.into()));
        let modify = EventKind::Modify(notify::event::ModifyKind::Any);
        assert_eq!(
            f.keys(event(modify, "/d/plugins/ks/plugin.wasm")),
            [Key::Folder("ks".into())]
        );
        assert_eq!(
            f.keys(event(modify, "/d/plugins/ks")),
            [Key::Folder("ks".into())]
        );
        assert_eq!(f.keys(event(modify, "/c/config.toml")), [Key::Config]);
        // Other files next to config.toml (the database, its WAL) and the plugins folder
        // itself are not changes of anything the manager reads.
        assert!(f.keys(event(modify, "/c/tomakast.db-wal")).is_empty());
        assert!(f.keys(event(modify, "/d/plugins")).is_empty());
        // Reading a folder is not a change.
        let access = EventKind::Access(notify::event::AccessKind::Any);
        assert!(
            f.keys(event(access, "/d/plugins/ks/plugin.wasm"))
                .is_empty()
        );
        let rescan = notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        assert_eq!(f.keys(Ok(rescan)), [Key::All]);
        // The plugins folder itself removed: a rescan, which reports it.
        let remove = EventKind::Remove(notify::event::RemoveKind::Folder);
        assert_eq!(f.keys(event(remove, "/d/plugins")), [Key::All]);
    }

    /// notify reports absolute paths for a relative watch path (a relative `TOMAKAST_HOME`).
    #[test]
    fn relative_paths_match_the_absolute_paths_notify_reports() {
        let f = Filter::new(Path::new("home/plugins"), Path::new("home/config.toml"));
        let cwd = std::env::current_dir().unwrap();
        let modify = EventKind::Modify(notify::event::ModifyKind::Any);
        let event = |path: PathBuf| Ok(notify::Event::new(modify).add_path(path));
        assert_eq!(
            f.keys(event(cwd.join("home/plugins/ks/plugin.toml"))),
            [Key::Folder("ks".into())]
        );
        assert_eq!(f.keys(event(cwd.join("home/config.toml"))), [Key::Config]);
    }
}
