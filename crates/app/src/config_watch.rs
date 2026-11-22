//! Watching `config.toml` for the daemon's hotkeys, independent of the plugin manager (whose
//! watcher does not run when plugins cannot load).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context as _;
use notify::{EventKind, RecursiveMode, Watcher as _};

/// How long the file must be quiet before `on_change` runs: editors write in bursts.
const QUIET: Duration = Duration::from_millis(300);

/// Watches `path`; dropping it stops.
pub(crate) struct ConfigWatch {
    _watcher: notify::RecommendedWatcher,
}

/// Calls `on_change` (on a thread of its own) once a change of `path` has been quiet for
/// [`QUIET`]. Watches the file's directory, not the file: editors replace the file.
pub(crate) fn watch_config(
    path: &Path,
    on_change: impl Fn() + Send + 'static,
) -> anyhow::Result<ConfigWatch> {
    // The file itself, and the file a symlink points to (dotfiles managers link it): edits
    // and our own writes land on the target, in its directory.
    let mut watched: Vec<PathBuf> = vec![path.to_path_buf()];
    if let Ok(target) = std::fs::canonicalize(path)
        && target != path
    {
        watched.push(target);
    }
    let names: Vec<(PathBuf, std::ffi::OsString)> = watched
        .iter()
        .filter_map(|p| {
            let dir = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Some((dir.to_path_buf(), p.file_name()?.to_os_string()))
        })
        .collect();
    let wanted = names.clone();
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
        let Ok(ev) = ev else {
            return;
        };
        if matches!(ev.kind, EventKind::Access(_)) {
            return;
        }
        let ours = ev.paths.iter().any(|p: &PathBuf| {
            wanted
                .iter()
                .any(|(_, name)| p.file_name() == Some(name.as_os_str()))
        });
        if ours {
            let _ = tx.send(());
        }
    })
    .context("starting the config watcher")?;
    let mut dirs: Vec<&PathBuf> = names.iter().map(|(dir, _)| dir).collect();
    dirs.dedup();
    for dir in dirs {
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", dir.display()))?;
    }
    std::thread::Builder::new()
        .name("tomakast-config-watch".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                // Wait out the burst.
                while rx.recv_timeout(QUIET).is_ok() {}
                on_change();
            }
        })
        .context("starting the config watch thread")?;
    Ok(ConfigWatch { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn count_after(dir: &tempfile::TempDir, writes: impl FnOnce()) -> usize {
        count_for(&dir.path().join("config.toml"), writes)
    }

    fn count_for(path: &std::path::Path, writes: impl FnOnce()) -> usize {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let _watch = watch_config(path, move || {
            seen.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        writes();
        let until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        calls.load(Ordering::SeqCst)
    }

    #[test]
    fn config_watch_fires_once_per_burst() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let n = count_after(&dir, || {
            for i in 0..3 {
                std::fs::write(&file, format!("# {i}\n")).unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn config_watch_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("tomakast.db");
        let n = count_after(&dir, || std::fs::write(&other, "x").unwrap());
        assert_eq!(n, 0);
    }

    #[cfg(unix)]
    #[test]
    fn config_watch_follows_a_symlink_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let dotfiles = tempfile::tempdir().unwrap();
        let target = dotfiles.path().join("tomakast.toml");
        std::fs::write(&target, "# 0\n").unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let n = count_for(&link, || std::fs::write(&target, "# 1\n").unwrap());
        assert_eq!(n, 1);
    }
}
