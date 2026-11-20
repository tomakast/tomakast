//! Theme-named icons (`Icon::Named`, such as the `accessories-calculator` of a plugin
//! command) that are not in the bundled Lucide set ([`crate::icons`]): drawn as the image
//! file the name stands for in the desktop's icon theme, resolved once per name, off the UI
//! thread. Until a name resolves, and when it resolves to nothing, the row shows its
//! monogram.
//!
//! A row asks [`cached`] while it draws. The first time a name is asked it is unresolved, and the
//! name goes to one worker thread, which resolves it (`tomakast_platform::icon_theme`: the
//! theme's index files and a walk of its icon directories) and wakes the UI to draw again.
//! After that the answer comes from the cache, a map lookup. Opening the launcher never
//! waits for a lookup. At most [`MAX_NAMES`] names are resolved: a plugin sending ever new
//! names cannot grow the cache or keep the worker busy; later names stay unresolved.
//!
//! On macOS and Windows the platform resolves no theme name, so these icons stay monograms (see
//! `tomakast_platform::icon_theme` for where a fallback goes). Nothing is resolved before
//! the app calls [`crate::resolve_named_icons`], so headless tests never read the
//! machine's icon theme.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, mpsc};

use gpui_kit::App;

/// The most names resolved per process.
pub const MAX_NAMES: usize = 1024;

/// The image file for icon `name`, if it has one and it is resolved yet; asks for it
/// otherwise.
pub(crate) fn cached(name: &str) -> Option<PathBuf> {
    ICONS.get()?.get(name)
}

static ICONS: OnceLock<NamedIcons> = OnceLock::new();

/// Starts resolving names, once per process: the platform's resolver, and a redraw of
/// every window after the names resolved meanwhile.
pub(crate) fn start(cx: &mut App) {
    if ICONS.get().is_some() {
        return;
    }
    let (wake, woken) = async_channel::unbounded();
    let wake = Arc::new(move || {
        let _ = wake.try_send(());
    });
    if ICONS
        .set(NamedIcons::new(
            tomakast_platform::icon_theme::resolve,
            wake,
        ))
        .is_err()
    {
        return;
    }
    cx.spawn(async move |cx| {
        while woken.recv().await.is_ok() {
            while woken.try_recv().is_ok() {}
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

enum Slot {
    /// With the worker.
    Pending,
    Resolved(Option<PathBuf>),
}

/// The names asked for and what they resolved to, with the worker that resolves them.
pub(crate) struct NamedIcons {
    slots: Arc<Mutex<HashMap<String, Slot>>>,
    requests: mpsc::Sender<String>,
}

impl NamedIcons {
    /// Starts the worker: it runs `resolve` on each name asked for, then `wake`.
    pub(crate) fn new(
        resolve: impl Fn(&str) -> Option<PathBuf> + Send + 'static,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> NamedIcons {
        let slots = Arc::new(Mutex::new(HashMap::new()));
        let (requests, names) = mpsc::channel::<String>();
        let resolved = slots.clone();
        let spawned = std::thread::Builder::new()
            .name("named-icons".into())
            .spawn(move || {
                for name in names {
                    let path = resolve(&name);
                    lock(&resolved).insert(name, Slot::Resolved(path));
                    wake();
                }
            });
        if let Err(e) = spawned {
            // The requests then fail, and every theme name outside the bundle stays a monogram.
            tracing::warn!(error = %e, "named icons will not be resolved");
        }
        NamedIcons { slots, requests }
    }

    /// What `name` resolved to; `None` while it is not resolved yet (asking the worker the
    /// first time) or past [`MAX_NAMES`].
    pub(crate) fn get(&self, name: &str) -> Option<PathBuf> {
        let mut slots = lock(&self.slots);
        match slots.get(name) {
            Some(Slot::Resolved(path)) => return path.clone(),
            Some(Slot::Pending) => return None,
            None if slots.len() >= MAX_NAMES => return None,
            None => {}
        }
        if self.requests.send(name.to_owned()).is_ok() {
            slots.insert(name.to_owned(), Slot::Pending);
        }
        None
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A cache whose resolver knows `known` (counting its calls) and resolves a name only
    /// once the returned sender lets it, and the wake-ups it sends.
    #[allow(clippy::type_complexity)]
    fn icons(
        known: &'static str,
    ) -> (
        NamedIcons,
        Arc<AtomicUsize>,
        mpsc::Sender<()>,
        mpsc::Receiver<()>,
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let (release, released) = mpsc::channel::<()>();
        let (woke, wakes) = mpsc::channel();
        let woke = Mutex::new(woke);
        let icons = NamedIcons::new(
            move |name: &str| {
                released.recv().unwrap();
                counted.fetch_add(1, Ordering::SeqCst);
                (name == known).then(|| PathBuf::from(format!("/icons/{name}.svg")))
            },
            Arc::new(move || lock(&woke).send(()).unwrap()),
        );
        (icons, calls, release, wakes)
    }

    fn wait(wakes: &mpsc::Receiver<()>) {
        wakes
            .recv_timeout(Duration::from_secs(5))
            .expect("a wake-up");
    }

    #[test]
    fn a_name_is_blank_until_resolved_then_cached() {
        let (icons, calls, release, wakes) = icons("accessories-calculator");
        assert_eq!(
            icons.get("accessories-calculator"),
            None,
            "not resolved yet"
        );
        assert_eq!(icons.get("accessories-calculator"), None, "asked once");
        release.send(()).unwrap();
        wait(&wakes);
        let path = Some(PathBuf::from("/icons/accessories-calculator.svg"));
        for _ in 0..3 {
            assert_eq!(icons.get("accessories-calculator"), path);
        }
        // A name with no file is cached too.
        assert_eq!(icons.get("no-such-icon"), None);
        release.send(()).unwrap();
        wait(&wakes);
        assert_eq!(icons.get("no-such-icon"), None);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(wakes.try_recv().is_err());
    }

    #[test]
    fn at_most_max_names_are_resolved() {
        let (icons, calls, release, wakes) = icons("");
        for n in 0..MAX_NAMES + 10 {
            icons.get(&format!("icon-{n}"));
        }
        for _ in 0..MAX_NAMES + 10 {
            release.send(()).unwrap();
        }
        for _ in 0..MAX_NAMES {
            wait(&wakes);
        }
        assert!(wakes.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), MAX_NAMES);
    }
}
