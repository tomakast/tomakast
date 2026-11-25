//! Watching the application folders: a rescan once they have been quiet a moment.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// How long the folders must be quiet before a rescan.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(500);

/// Blocks the calling thread: watches every existing directory in `dirs` and
/// calls `on_change` once the filesystem has been quiet for `debounce`.
/// Returns when `on_change` returns `false` or the watcher dies.
///
/// Only content changes (create/modify/remove) count. notify also reports
/// `Access` events (including opens), and the scan itself opens every file,
/// so reacting to those would make the watcher rescan forever.
pub(crate) fn watch(dirs: &[PathBuf], debounce: Duration, mut on_change: impl FnMut() -> bool) {
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(err) => {
            tracing::warn!(%err, "apps: file watcher unavailable, list will not auto-refresh");
            return;
        }
    };
    let mut watched = 0;
    for dir in dirs.iter().filter(|d| d.is_dir()) {
        match watcher.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => watched += 1,
            Err(err) => tracing::warn!(dir = %dir.display(), %err, "apps: cannot watch directory"),
        }
    }
    if watched == 0 {
        return;
    }
    let is_change = |ev: &notify::Result<notify::Event>| match ev {
        Ok(ev) => matches!(
            ev.kind,
            notify::EventKind::Create(_)
                | notify::EventKind::Modify(_)
                | notify::EventKind::Remove(_)
                | notify::EventKind::Any
        ),
        Err(err) => {
            tracing::debug!(%err, "apps: watcher error");
            false
        }
    };
    loop {
        // Wait for the first real change.
        loop {
            match rx.recv() {
                Ok(ev) if is_change(&ev) => break,
                Ok(_) => {}
                Err(_) => return,
            }
        }
        // Debounce: keep draining until no change arrives for `debounce`.
        loop {
            match rx.recv_timeout(debounce) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        if !on_change() {
            return;
        }
    }
}

// macOS reports events from just before a watch begins, which this test counts as changes.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn watcher_fires_once_per_change_and_ignores_its_own_reads() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().to_path_buf();
        std::fs::write(apps.join("a.desktop"), "[Desktop Entry]\n").unwrap();

        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (apps, count, stop) = (apps.clone(), count.clone(), stop.clone());
            std::thread::spawn(move || {
                let dirs = [apps.clone()];
                watch(&dirs, Duration::from_millis(100), || {
                    // Mimic a scan: list the directory and open every file.
                    for e in std::fs::read_dir(&apps).unwrap().flatten() {
                        let _ = std::fs::read(e.path());
                    }
                    count.fetch_add(1, Ordering::SeqCst);
                    !stop.load(Ordering::SeqCst)
                })
            })
        };
        // Let the watcher register; nothing changed yet, so nothing may fire.
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(count.load(Ordering::SeqCst), 0);

        std::fs::write(apps.join("b.desktop"), "[Desktop Entry]\n").unwrap();
        wait_for("first change", || count.load(Ordering::SeqCst) >= 1);
        // The reads done by the callback must not retrigger it.
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "watcher re-fired on its own reads"
        );

        std::fs::write(apps.join("c.desktop"), "[Desktop Entry]\n").unwrap();
        wait_for("second change", || count.load(Ordering::SeqCst) >= 2);

        stop.store(true, Ordering::SeqCst);
        std::fs::write(apps.join("d.desktop"), "[Desktop Entry]\n").unwrap();
        handle.join().unwrap();
    }
}
