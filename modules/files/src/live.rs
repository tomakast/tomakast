//! The live index: walked on its own thread, kept fresh by watching the indexed folders, and
//! re-walked when the `[files]` coverage in the config file changes.
//!
//! One thread, `files-index`, owns everything that changes: the rules, the watches and the
//! timers. It sleeps on one channel that the folder watcher, the config watcher and `stop`
//! feed, with a timeout for the next periodic re-walk, so an idle machine wakes it only every
//! 30 minutes (watched or not: with the folders too many to watch, that re-walk is the only
//! way changes come in).
//!
//! The index of the last run is kept in a snapshot file: at start it is read back at once
//! (when the coverage is the same), so files are found from the first moments, and the first
//! walk then replaces it in the background.
//!
//! The index sits behind a `RwLock`. A full walk builds a new index off the lock and swaps it
//! in, so a search during a walk answers from the old one at once. Watcher batches change the
//! index in place: the disk is read first with no write lock (a new folder's whole subtree
//! included), then the entries go in under short write locks of `INSERT_CHUNK` each.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant, SystemTime};

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tomakast_core::settings::{FilesSettings, Settings};

use crate::index::{FileIndex, Upsert};
use crate::rules::{Coverage, Rules};

/// A batch is applied at the latest this long after its first event, even while events keep
/// coming (a file being written all the time must not hold the others back).
const MAX_BATCH: Duration = Duration::from_secs(1);
/// Entries inserted per write lock: how long a search may wait behind a new folder.
const INSERT_CHUNK: usize = 64;
/// Between two write locks of one large insert, searches waiting for the index get it first,
/// for at most this long. The lock does not queue: without the wait the thread would take it
/// again before a woken search does, chunk after chunk.
const READERS_TURN: Duration = Duration::from_millis(20);
/// A folder changed this long before a walk began may have changed after the walk read it
/// (file systems with coarse modification times).
const CLOCK_SLACK: Duration = Duration::from_secs(2);
/// Never watch more folders than this, whatever the system allows.
const MOST_WATCHES: usize = 200_000;

/// How the thread works; production values by default, tests shorten or hook them.
pub(crate) struct Options {
    /// Honour the user's global gitignore (tests turn it off: it is the machine's git config).
    pub global_ignore: bool,
    /// At most this many folders are watched; over it, none is, and the index is re-walked
    /// every `poll_every` instead.
    pub watch_cap: usize,
    /// The safety-net re-walk while watching.
    pub rewalk_every: Duration,
    /// The re-walk while not watching (over the cap, out of system watches), and at the
    /// latest after the watcher lost events. As long as `rewalk_every` by default: a walk of
    /// the home folder is a burst of disk and CPU, and the ruling keeps it rare.
    pub poll_every: Duration,
    /// A batch of watcher events is applied once none has come for this long.
    pub debounce: Duration,
    /// Builds a fresh index (a test may hold it back to look at the old one meanwhile).
    pub walk: Box<dyn Fn(&Rules) -> FileIndex + Send>,
    /// Called once, on the thread, right after the first index is in place: the snapshot
    /// read back, or the first walk.
    pub on_first_walk: Option<Box<dyn FnOnce() + Send>>,
    /// The snapshot file: read at start, written after every walk. `None`: none is kept.
    pub snapshot: Option<PathBuf>,
    /// Pretends the system has watches for only this many folders.
    #[cfg(test)]
    pub fake_watch_limit: Option<usize>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            global_ignore: true,
            watch_cap: watch_cap(),
            rewalk_every: Duration::from_secs(30 * 60),
            poll_every: Duration::from_secs(30 * 60),
            debounce: Duration::from_millis(200),
            walk: Box::new(FileIndex::walk),
            on_first_walk: None,
            snapshot: None,
            #[cfg(test)]
            fake_watch_limit: None,
        }
    }
}

/// Half of the system's inotify watches (other programs share them), at most `MOST_WATCHES`.
fn watch_cap() -> usize {
    if !cfg!(target_os = "linux") {
        return MOST_WATCHES;
    }
    let max = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        // The kernel's long-standing default, when the limit cannot be read.
        .unwrap_or(8192);
    (max / 2).min(MOST_WATCHES)
}

/// What the thread is woken with.
enum Msg {
    /// Paths a watcher reported. `structural`: an entry may have appeared, gone or moved
    /// (not only been written to).
    Fs {
        paths: Vec<PathBuf>,
        structural: bool,
    },
    /// The watcher lost events.
    Rescan,
    /// The config file changed.
    Config,
    Stop,
}

/// The index of the covered folders, kept fresh by the `files-index` thread.
#[derive(Clone)]
pub struct Live {
    index: Arc<RwLock<FileIndex>>,
    /// An index is in place (read back or walked); before, searches find nothing yet.
    ready: Arc<AtomicBool>,
    /// Calls of `read` waiting for the lock.
    waiting: Arc<AtomicUsize>,
    shared: Arc<Shared>,
    #[cfg(test)]
    seen: Arc<std::sync::Mutex<Seen>>,
}

/// What the thread shows the tests of itself.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
struct Seen {
    watched: BTreeSet<PathBuf>,
    /// Folder watchers created.
    watchers: usize,
    /// Walks swapped in.
    walks: usize,
}

/// Held by every clone of a `Live`; the last one dropped stops the thread.
struct Shared {
    tx: Sender<Msg>,
    stop: Arc<AtomicBool>,
    #[cfg(test)]
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Shared {
    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Stop);
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Live {
    /// Spawns the `files-index` thread: first walk, then watch + re-walks; watches
    /// `config_file` and re-walks when `[files]` roots/exclude/hidden change. `on_settings`
    /// gets the `[files]` table once at start and again on every readable change of the file.
    pub fn start(
        config_file: PathBuf,
        home: PathBuf,
        on_settings: impl Fn(&FilesSettings) + Send + 'static,
    ) -> Live {
        Live::start_with(config_file, home, Options::default(), on_settings)
    }

    /// `start` with the thread's settings given.
    pub(crate) fn start_with(
        config_file: PathBuf,
        home: PathBuf,
        opts: Options,
        on_settings: impl Fn(&FilesSettings) + Send + 'static,
    ) -> Live {
        let index = Arc::new(RwLock::new(FileIndex::default()));
        let ready = Arc::new(AtomicBool::new(false));
        let waiting = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let seen = Arc::new(std::sync::Mutex::new(Seen::default()));
        let worker = Worker {
            index: index.clone(),
            ready: ready.clone(),
            waiting: waiting.clone(),
            stop: stop.clone(),
            rx,
            tx: tx.clone(),
            config_file,
            rules: Rules::new(
                &coverage(&FilesSettings::default(), opts.global_ignore),
                &home,
            ),
            home,
            opts,
            on_settings: Box::new(on_settings),
            files: FilesSettings::default(),
            watcher: None,
            watched: BTreeSet::new(),
            watch_blocked: false,
            last_walk: Instant::now(),
            next_walk: Instant::now(),
            #[cfg(test)]
            seen: seen.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("files-index".into())
            .spawn(move || worker.run());
        if let Err(err) = &thread {
            tracing::error!(%err, "files: cannot start the index thread; no files will be found");
        }
        Live {
            index,
            ready,
            waiting,
            shared: Arc::new(Shared {
                tx,
                stop,
                #[cfg(test)]
                thread: std::sync::Mutex::new(thread.ok()),
            }),
            #[cfg(test)]
            seen,
        }
    }

    /// Runs `f` under the read lock. Never waits for a walk: a full re-walk builds a new
    /// FileIndex off the lock and swaps it in; watcher batches mutate in place under short
    /// write locks. Empty before the first walk.
    pub fn read<R>(&self, f: impl FnOnce(&FileIndex) -> R) -> R {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        self.waiting.fetch_sub(1, Ordering::SeqCst);
        f(&index)
    }

    /// Whether an index is in place yet: until then a search finds nothing because nothing
    /// is indexed, not because nothing matches.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// Has the thread look at `path` again soon, as if the watcher had reported it: gone from
    /// the disk, it leaves the index. Returns at once.
    pub fn recheck(&self, path: &Path) {
        let _ = self.shared.tx.send(Msg::Fs {
            paths: vec![path.to_path_buf()],
            structural: true,
        });
    }

    /// Tells the thread to end, and returns at once. A walk in progress is finished first and
    /// then thrown away. Also done when the last clone is dropped.
    pub fn stop(&self) {
        self.shared.stop();
    }
}

/// The `[files]` table as the rules take it.
fn coverage(files: &FilesSettings, global_ignore: bool) -> Coverage {
    Coverage {
        roots: files.roots.clone(),
        exclude: files.exclude.clone(),
        hidden: files.hidden,
        gitignore: files.gitignore,
        global_ignore,
    }
}

/// The changes a debounce window gathered.
#[derive(Default)]
struct Batch {
    /// Path to whether any of its events was structural.
    paths: HashMap<PathBuf, bool>,
    config: bool,
    rescan: bool,
}

impl Batch {
    /// False for `Stop`.
    fn add(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Fs { paths, structural } => {
                for p in paths {
                    *self.paths.entry(p).or_default() |= structural;
                }
            }
            Msg::Rescan => self.rescan = true,
            Msg::Config => self.config = true,
            Msg::Stop => return false,
        }
        true
    }
}

/// The state of the `files-index` thread.
struct Worker {
    index: Arc<RwLock<FileIndex>>,
    /// See `Live::ready`.
    ready: Arc<AtomicBool>,
    /// See `Live::waiting`.
    waiting: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    /// For the watchers the thread creates.
    tx: Sender<Msg>,
    config_file: PathBuf,
    home: PathBuf,
    opts: Options,
    on_settings: Box<dyn Fn(&FilesSettings) + Send>,
    /// The `[files]` table the rules were built from.
    files: FilesSettings,
    rules: Rules,
    /// `None` while not watching (too many folders, or no watcher could be made).
    watcher: Option<RecommendedWatcher>,
    /// The folders watched, one non-recursive watch each.
    watched: BTreeSet<PathBuf>,
    /// The system ran out of watches: no watcher again until the coverage changes (or the
    /// launcher restarts), so the last free watches stay with other programs.
    watch_blocked: bool,
    last_walk: Instant,
    next_walk: Instant,
    #[cfg(test)]
    seen: Arc<std::sync::Mutex<Seen>>,
}

impl Worker {
    fn run(mut self) {
        // Watched before it is read, so no change in between is missed.
        let _config_watch = watch_config(&self.config_file, self.tx.clone());
        let (settings, err) = Settings::load_or_default(&self.config_file);
        if let Some(err) = err {
            tracing::warn!("files: using the default coverage, the config is unreadable: {err:#}");
        }
        self.set_files(settings.files);
        self.watcher = self.folder_watcher();
        self.read_snapshot();
        self.rewalk();
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            // Checked before the channel: events that never stop must not hold the walk off.
            if Instant::now() >= self.next_walk {
                self.rewalk();
                continue;
            }
            let wait = self.next_walk.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(wait) {
                Err(RecvTimeoutError::Timeout) => self.rewalk(),
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(first) => match self.collect(first) {
                    Some(batch) => self.handle(batch),
                    None => return,
                },
            }
        }
    }

    /// Gathers events until none came for the debounce time (or `MAX_BATCH` passed). `None`
    /// on `Stop`.
    fn collect(&self, first: Msg) -> Option<Batch> {
        let mut batch = Batch::default();
        if !batch.add(first) {
            return None;
        }
        let end = Instant::now() + MAX_BATCH;
        loop {
            let wait = self
                .opts
                .debounce
                .min(end.saturating_duration_since(Instant::now()));
            if wait.is_zero() {
                return Some(batch);
            }
            match self.rx.recv_timeout(wait) {
                Ok(msg) => {
                    if !batch.add(msg) {
                        return None;
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Some(batch),
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn handle(&mut self, batch: Batch) {
        if batch.config && self.reload_config() {
            // The fresh walk sees every change of the batch too.
            self.rewalk();
            return;
        }
        if batch.rescan {
            // Not at once: a burst that overflows the queue may well go on for a while.
            tracing::debug!(
                every = ?self.opts.poll_every,
                "files: the watcher lost events; re-walking within this interval"
            );
            self.next_walk = self.next_walk.min(self.last_walk + self.opts.poll_every);
        }
        if !batch.paths.is_empty() {
            self.apply(batch.paths.into_iter().collect());
        }
    }

    /// Reads the config file again and passes `[files]` on; true when the coverage changed.
    /// An unreadable file (an editor half-way through saving) changes nothing.
    fn reload_config(&mut self) -> bool {
        match Settings::load(&self.config_file) {
            Ok(settings) => self.set_files(settings.files),
            Err(err) => {
                tracing::warn!("files: keeping the coverage, the config is unreadable: {err:#}");
                false
            }
        }
    }

    /// Takes the `[files]` table; true when the coverage changed (and the rules with it).
    fn set_files(&mut self, files: FilesSettings) -> bool {
        (self.on_settings)(&files);
        let changed = files.roots != self.files.roots
            || files.exclude != self.files.exclude
            || files.hidden != self.files.hidden
            || files.gitignore != self.files.gitignore;
        if changed {
            self.rules = Rules::new(&coverage(&files, self.opts.global_ignore), &self.home);
            self.watch_blocked = false;
        }
        self.files = files;
        changed
    }

    /// Walks the coverage into a new index, watches its folders, and swaps it in. Events
    /// that arrive meanwhile wait in the channel and are applied to the new index.
    fn rewalk(&mut self) {
        let started = Instant::now();
        let since = SystemTime::now() - CLOCK_SLACK;
        let mut fresh = (self.opts.walk)(&self.rules);
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        // Before the swap: once the index shows a folder, changes in it are caught.
        let newly = self.sync_watches(&fresh);
        self.catch_up(&mut fresh, newly, since);
        self.publish();
        let (entries, folders) = (fresh.len(), fresh.folders().len());
        let old = std::mem::replace(&mut *self.write(), fresh);
        // Freed after the lock is released.
        drop(old);
        self.index_ready();
        self.write_snapshot();
        self.last_walk = Instant::now();
        let every = if self.watcher.is_some() {
            self.opts.rewalk_every
        } else {
            self.opts.poll_every
        };
        self.next_walk = self.last_walk + every;
        #[cfg(test)]
        {
            self.seen.lock().unwrap().walks += 1;
        }
        tracing::info!(
            entries,
            folders,
            watched = self.watched.len(),
            took = ?started.elapsed(),
            "files: walked"
        );
    }

    /// Watches exactly the folders of `fresh`, or none when they are over the cap; returns
    /// the folders that were not watched before.
    fn sync_watches(&mut self, fresh: &FileIndex) -> Vec<PathBuf> {
        if fresh.folders().len() > self.opts.watch_cap {
            if self.stop_watching() {
                tracing::info!(
                    every = ?self.opts.poll_every,
                    "files: more folders than the watch cap; re-walking periodically instead"
                );
            }
            return Vec::new();
        }
        if self.watch_blocked {
            return Vec::new();
        }
        if self.watcher.is_none() {
            self.watcher = self.folder_watcher();
        }
        let gone: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|w| !fresh.is_folder(w))
            .cloned()
            .collect();
        self.unwatch(&gone);
        let new: Vec<PathBuf> = fresh
            .folders()
            .filter(|f| !self.watched.contains(*f))
            .map(Path::to_path_buf)
            .collect();
        if self.watch(&new) { new } else { Vec::new() }
    }

    /// Changes made in newly watched folders after the walk read them and before their watch
    /// was in place (the first walk, new roots, watching again after polling): each such folder
    /// modified since the walk began is compared with the disk, both ways. Folders found that
    /// way are watched and compared too.
    fn catch_up(&mut self, fresh: &mut FileIndex, newly: Vec<PathBuf>, since: SystemTime) {
        let mut check: Vec<PathBuf> = newly
            .into_iter()
            .filter(|f| modified_since(f, since))
            .collect();
        while !check.is_empty() {
            let dirty: Vec<PathBuf> = check
                .iter()
                .flat_map(|f| drift(fresh, &self.rules, f))
                .collect();
            for path in &dirty {
                fresh.upsert(path, &self.rules);
            }
            let new: Vec<PathBuf> = dirty
                .iter()
                .flat_map(|p| fresh.folders_under(p))
                .filter(|f| !self.watched.contains(f))
                .collect();
            // Only folders the watch took are followed: one it refused (unreadable) would come
            // round again and again.
            let Some(new) = self.watch_new(new) else {
                break;
            };
            check = new;
        }
    }

    /// Applies one batch of watcher events in place.
    fn apply(&mut self, mut paths: Vec<(PathBuf, bool)>) {
        // Hidden files and the like, written in watched folders: dropped before any lock.
        paths.retain(|(path, _)| !skipped(&self.rules, path));
        let mut look: Vec<PathBuf> = Vec::new();
        for (path, structural) in &paths {
            if *structural {
                // A moved or removed folder's watches go (notify drops them itself; the set
                // follows); the folders still indexed are watched again below.
                self.unwatch_under(path);
                look.push(path.clone());
            }
        }
        self.upsert(&paths);
        // New folders get their watch; whatever changed in them before it did is found by
        // comparing them with the disk, which may find more folders, until nothing is new.
        while !look.is_empty() {
            let new: Vec<PathBuf> = {
                let index = self.read();
                look.iter()
                    .flat_map(|p| index.folders_under(p))
                    .filter(|f| !self.watched.contains(f))
                    .collect()
            };
            let Some(new) = self.watch_new(new) else {
                break;
            };
            let dirty: Vec<(PathBuf, bool)> = {
                let index = self.read();
                new.iter()
                    .flat_map(|f| drift(&index, &self.rules, f))
                    .map(|p| (p, true))
                    .collect()
            };
            self.upsert(&dirty);
            look = dirty.into_iter().map(|(p, _)| p).collect();
        }
        self.publish();
    }

    /// `upsert` of each path: the disk is read under the read lock (searches go on), the
    /// entries go in under write locks of `INSERT_CHUNK` entries at most.
    fn upsert(&self, paths: &[(PathBuf, bool)]) {
        for (path, structural) in paths {
            let plan = {
                let index = self.read();
                // A folder written to: its contents have watches of their own, and upsert
                // would walk all of it.
                if !structural && index.is_folder(path) {
                    continue;
                }
                match index.plan(path, &self.rules) {
                    Upsert::Remove if index.id_of(path).is_none() => continue,
                    plan => plan,
                }
            };
            match plan {
                Upsert::Nothing => {}
                Upsert::Remove => {
                    self.let_readers_in();
                    self.write().remove(path);
                }
                Upsert::Insert(found) => {
                    for chunk in found.chunks(INSERT_CHUNK) {
                        self.let_readers_in();
                        self.write().insert_found(chunk);
                    }
                }
            }
        }
    }

    /// Adds a watch per folder; false when watching stopped (over the cap, or the system's
    /// limit reached).
    fn watch(&mut self, new: &[PathBuf]) -> bool {
        if new.is_empty() {
            return self.watcher.is_some();
        }
        if self.watched.len() + new.len() > self.opts.watch_cap {
            if self.stop_watching() {
                tracing::info!(
                    every = ?self.opts.poll_every,
                    "files: more folders than the watch cap; re-walking periodically instead"
                );
            }
            return false;
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return false;
        };
        let mut full = false;
        let mut paths = watcher.paths_mut();
        for folder in new {
            #[cfg(test)]
            if self
                .opts
                .fake_watch_limit
                .is_some_and(|limit| self.watched.len() >= limit)
            {
                full = true;
                break;
            }
            match paths.add(folder, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.watched.insert(folder.clone());
                }
                Err(err) if matches!(err.kind, notify::ErrorKind::MaxFilesWatch) => {
                    full = true;
                    break;
                }
                // Gone again already, or unreadable.
                Err(err) => tracing::debug!(folder = %folder.display(), %err, "files: not watched"),
            }
        }
        if let Err(err) = paths.commit() {
            tracing::debug!(%err, "files: watches not committed");
        }
        if full {
            self.stop_watching();
            self.watch_blocked = true;
            tracing::warn!(
                every = ?self.opts.poll_every,
                "files: the system has no file watches left; not watching folders until the \
                 [files] coverage changes or the launcher restarts, re-walking periodically \
                 instead"
            );
            return false;
        }
        true
    }

    /// Watches the folders and returns the ones now watched; `None` when there are none, when
    /// watching stopped, or when the thread is asked to stop (the loops that follow new
    /// folders end there).
    fn watch_new(&mut self, mut new: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        if new.is_empty() || self.stop.load(Ordering::SeqCst) || !self.watch(&new) {
            return None;
        }
        new.retain(|f| self.watched.contains(f));
        (!new.is_empty()).then_some(new)
    }

    fn unwatch_under(&mut self, path: &Path) {
        let gone: Vec<PathBuf> = self
            .watched
            .range::<Path, _>((Bound::Included(path), Bound::Unbounded))
            .take_while(|w| w.starts_with(path))
            .cloned()
            .collect();
        self.unwatch(&gone);
    }

    fn unwatch(&mut self, gone: &[PathBuf]) {
        if gone.is_empty() {
            return;
        }
        for folder in gone {
            self.watched.remove(folder);
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let mut paths = watcher.paths_mut();
        for folder in gone {
            // Fails when notify already dropped it (the folder was removed or moved away).
            let _ = paths.remove(folder);
        }
        let _ = paths.commit();
    }

    /// Drops every watch; periodic re-walks keep the index fresh instead. True when there was
    /// a watcher to drop.
    fn stop_watching(&mut self) -> bool {
        let had = self.watcher.take().is_some();
        self.watched.clear();
        self.next_walk = self.next_walk.min(self.last_walk + self.opts.poll_every);
        had
    }

    fn folder_watcher(&self) -> Option<RecommendedWatcher> {
        #[cfg(test)]
        {
            self.seen.lock().unwrap().watchers += 1;
        }
        folder_watcher(self.tx.clone())
    }

    /// Waits (briefly) until the searches that asked for the index meanwhile have it.
    fn let_readers_in(&self) {
        let until = Instant::now() + READERS_TURN;
        while self.waiting.load(Ordering::SeqCst) > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_micros(50));
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, FileIndex> {
        self.index.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, FileIndex> {
        self.index.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Shows the watched folders to the tests.
    /// An index is in place: searches may say "nothing found" from now on.
    fn index_ready(&mut self) {
        self.ready.store(true, Ordering::SeqCst);
        if let Some(first) = self.opts.on_first_walk.take() {
            first();
        }
    }

    /// The coverage the index is walked under, as one number: a snapshot of another one is
    /// not read back.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.home.hash(&mut h);
        self.files.roots.hash(&mut h);
        self.files.exclude.hash(&mut h);
        self.files.hidden.hash(&mut h);
        self.files.gitignore.hash(&mut h);
        self.opts.global_ignore.hash(&mut h);
        h.finish()
    }

    /// Puts the last run's index in place, when the snapshot file has one of this coverage.
    fn read_snapshot(&mut self) {
        let Some(file) = &self.opts.snapshot else {
            return;
        };
        let started = Instant::now();
        let bytes = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(err) => {
                tracing::warn!(file = %file.display(), %err, "files: cannot read the saved index");
                return;
            }
        };
        let Some(index) = FileIndex::from_snapshot(&bytes, self.fingerprint()) else {
            tracing::info!("files: the saved index is of another coverage or version; walking");
            return;
        };
        drop(bytes);
        let entries = index.len();
        *self.write() = index;
        self.index_ready();
        tracing::info!(entries, took = ?started.elapsed(), "files: read the saved index");
    }

    /// Writes the index to the snapshot file: built under a read lock (searches go on), then
    /// written to a temporary file that replaces the old one, readable by the user alone.
    fn write_snapshot(&self) {
        let Some(file) = &self.opts.snapshot else {
            return;
        };
        let bytes = self.read().to_snapshot(self.fingerprint());
        if let Err(err) = crate::state::write_private(file, &bytes) {
            tracing::warn!(file = %file.display(), %err, "files: cannot save the index");
        }
    }

    fn publish(&self) {
        #[cfg(test)]
        {
            self.seen.lock().unwrap().watched = self.watched.clone();
        }
    }
}

/// Whether a path is surely left out, by the names and prefixes of the rules alone (hidden,
/// a built-in folder name or an `exclude` on the way down from its root), or outside every
/// root. Only a quick check before the index is locked: `upsert` still checks everything.
/// The path's own name is checked as a file's, since it may be one.
fn skipped(rules: &Rules, path: &Path) -> bool {
    let Some(root) = rules
        .roots()
        .iter()
        .filter(|r| path.starts_with(r))
        .max_by_key(|r| r.components().count())
    else {
        return true;
    };
    path.ancestors()
        .take_while(|link| *link != root)
        .any(|link| rules.excludes(link, link != path))
}

/// The entries of `folder` the index has wrong: on disk but not indexed (and not surely left
/// out), or indexed but gone. The folder itself when it is gone; nothing when it is there but
/// cannot be read (another user's folder): asking again would get the same answer.
fn drift(index: &FileIndex, rules: &Rules, folder: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return vec![folder.to_path_buf()];
        }
        Err(_) => return Vec::new(),
    };
    let on_disk: HashSet<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    let mut dirty: Vec<PathBuf> = index
        .children_of(folder)
        .into_iter()
        .filter(|kid| !on_disk.contains(kid))
        .collect();
    dirty.extend(
        on_disk
            .into_iter()
            .filter(|p| !skipped(rules, p) && index.id_of(p).is_none()),
    );
    dirty
}

/// Whether the folder was modified at or after `since` (or cannot be looked at).
fn modified_since(folder: &Path, since: SystemTime) -> bool {
    std::fs::symlink_metadata(folder)
        .and_then(|m| m.modified())
        .map_or(true, |t| t >= since)
}

/// The watcher of the indexed folders, feeding the thread's channel. Reads are dropped here,
/// the thread's own walks included, so they never wake it.
fn folder_watcher(tx: Sender<Msg>) -> Option<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let event = match res {
            Ok(event) => event,
            Err(err) => {
                tracing::debug!(%err, "files: watcher error");
                return;
            }
        };
        if event.need_rescan() {
            let _ = tx.send(Msg::Rescan);
            return;
        }
        let structural = match event.kind {
            EventKind::Access(_) => return,
            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)) => false,
            _ => true,
        };
        if !event.paths.is_empty() {
            let _ = tx.send(Msg::Fs {
                paths: event.paths,
                structural,
            });
        }
    });
    match watcher {
        Ok(w) => Some(w),
        Err(err) => {
            tracing::warn!(%err, "files: no file watcher; the index is refreshed by re-walks only");
            None
        }
    }
}

/// Watches the config file's folder (editors replace the file by a rename) for changes of the
/// file. Its own reads of the file are dropped.
fn watch_config(file: &Path, tx: Sender<Msg>) -> Option<RecommendedWatcher> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return None;
    };
    let name = name.to_os_string();
    let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let Ok(event) = res else { return };
        let read =
            matches!(event.kind, EventKind::Access(a) if a != AccessKind::Close(AccessMode::Write));
        let ours = event.need_rescan()
            || event
                .paths
                .iter()
                .any(|p| p.file_name() == Some(name.as_os_str()));
        if ours && !read {
            let _ = tx.send(Msg::Config);
        }
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(err) => {
            tracing::warn!(%err, "files: config changes will not be followed");
            return None;
        }
    };
    if let Err(err) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        tracing::warn!(dir = %dir.display(), %err, "files: config changes will not be followed");
        return None;
    }
    Some(watcher)
}

#[cfg(test)]
impl Live {
    /// The folders being watched now.
    fn watched(&self) -> BTreeSet<PathBuf> {
        self.seen().watched
    }

    fn seen(&self) -> Seen {
        self.seen.lock().unwrap().clone()
    }

    /// Whether the thread has ended within `timeout`.
    fn ended_within(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let ended = self
                .shared
                .thread
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|t| t.is_finished());
            if ended {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Coverage;
    use crate::search::{Signals, search};
    use std::collections::{BTreeSet, HashMap, HashSet};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Mutex, mpsc};
    use std::time::Instant;
    use tempfile::TempDir;

    /// Generous: the machine running the tests may be busy.
    const PATIENCE: Duration = Duration::from_secs(10);

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    /// A home folder and a config file outside it.
    struct Env {
        _tmp: TempDir,
        home: PathBuf,
        config: PathBuf,
    }

    impl Env {
        fn new() -> Env {
            let tmp = TempDir::new().unwrap();
            let home = tmp.path().join("home");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(tmp.path().join("cfg")).unwrap();
            let config = tmp.path().join("cfg/config.toml");
            Env {
                _tmp: tmp,
                home,
                config,
            }
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.home.join(rel)
        }

        /// Replaces the config file the way editors do: a new file renamed over it.
        fn write_config(&self, text: &str) {
            let tmp = self.config.with_extension("toml.new");
            fs::write(&tmp, text).unwrap();
            fs::rename(&tmp, &self.config).unwrap();
        }

        fn roots_config(&self, roots: &[&str]) -> String {
            let roots: Vec<String> = roots
                .iter()
                .map(|r| format!("{:?}", self.path(r).to_str().unwrap()))
                .collect();
            format!("[files]\nroots = [{}]\n", roots.join(", "))
        }

        fn start(&self, opts: Options) -> Live {
            Live::start_with(self.config.clone(), self.home.clone(), opts, |_| {})
        }
    }

    fn test_options() -> Options {
        Options {
            global_ignore: false,
            ..Options::default()
        }
    }

    fn has(live: &Live, path: &Path) -> bool {
        live.read(|i| i.id_of(path).is_some())
    }

    fn found(live: &Live, query: &str, home: &Path) -> Vec<PathBuf> {
        let (opened, recent) = (HashMap::new(), HashSet::new());
        let signals = Signals {
            opened: &opened,
            recent: &recent,
            home,
            now: 0,
        };
        live.read(|i| {
            search(i, query, 10, &signals)
                .iter()
                .map(|f| i.path(f.id))
                .collect()
        })
    }

    /// Watching or not (over the cap, out of system watches, a lost-events burst), the index
    /// is walked again at most every 30 minutes.
    #[test]
    fn rewalks_every_30_minutes_watching_or_not() {
        let opts = Options::default();
        assert_eq!(opts.rewalk_every, Duration::from_secs(30 * 60));
        assert_eq!(opts.poll_every, Duration::from_secs(30 * 60));
    }

    #[test]
    fn created_file_is_found_after_the_debounce() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let live = env.start(test_options());
        // The index is shown only once its folders are watched.
        wait_for("first walk", || has(&live, &env.path("seed.txt")));

        let new = env.path("new.txt");
        touch(&new);
        wait_for("new.txt", || found(&live, "new", &env.home).contains(&new));
    }

    #[test]
    fn first_walk_hook_runs_once() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let live = env.start(Options {
            rewalk_every: Duration::from_millis(20),
            poll_every: Duration::from_millis(20),
            on_first_walk: Some(Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            })),
            ..test_options()
        });
        wait_for("three walks", || live.seen.lock().unwrap().walks >= 3);
        // Called after the swap: the index it announces is in place.
        assert!(has(&live, &env.path("seed.txt")));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn created_folder_tree_is_found_and_watched() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("seed.txt")));

        // Made at once, faster than a watch can be placed on each new folder.
        let deep = env.path("a/b/c/deep.txt");
        touch(&deep);
        wait_for("deep.txt", || has(&live, &deep));
        wait_for("the new folders watched", || {
            live.watched().contains(&env.path("a/b/c"))
        });
        let later = env.path("a/b/c/later.txt");
        touch(&later);
        wait_for("later.txt", || has(&live, &later));
    }

    #[test]
    fn removed_folder_leaves_the_index() {
        let env = Env::new();
        touch(&env.path("d/sub/x.txt"));
        touch(&env.path("keep.txt"));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("d/sub/x.txt")));
        assert!(live.watched().contains(&env.path("d/sub")));

        fs::remove_dir_all(env.path("d")).unwrap();
        wait_for("d gone", || {
            !has(&live, &env.path("d")) && !has(&live, &env.path("d/sub/x.txt"))
        });
        wait_for("d unwatched", || {
            let w = live.watched();
            !w.contains(&env.path("d")) && !w.contains(&env.path("d/sub"))
        });
        assert!(has(&live, &env.path("keep.txt")));
    }

    #[test]
    fn renamed_file_is_found_under_its_new_name() {
        let env = Env::new();
        touch(&env.path("dir/before.txt"));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("dir/before.txt")));

        fs::rename(env.path("dir/before.txt"), env.path("after.txt")).unwrap();
        wait_for("rename", || {
            has(&live, &env.path("after.txt")) && !has(&live, &env.path("dir/before.txt"))
        });
    }

    #[test]
    fn search_during_rewalk_uses_the_old_index() {
        let env = Env::new();
        touch(&env.path("a/old.txt"));
        touch(&env.path("b/fresh.txt"));
        env.write_config(&env.roots_config(&["a"]));

        let (started_tx, started) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let (started_tx, gate) = (Mutex::new(started_tx), Mutex::new(gate));
        let walks = AtomicUsize::new(0);
        let opts = Options {
            walk: Box::new(move |rules| {
                let index = FileIndex::walk(rules);
                if walks.fetch_add(1, Ordering::SeqCst) == 1 {
                    // The second walk is built; hold it back until the test has searched.
                    started_tx.lock().unwrap().send(()).unwrap();
                    let _ = gate.lock().unwrap().recv_timeout(PATIENCE);
                }
                index
            }),
            ..test_options()
        };
        let live = env.start(opts);
        wait_for("first walk", || has(&live, &env.path("a/old.txt")));

        env.write_config(&env.roots_config(&["b"]));
        started.recv_timeout(PATIENCE).expect("the re-walk started");

        let t = Instant::now();
        let during = found(&live, "old", &env.home);
        assert!(t.elapsed() < PATIENCE / 2, "read waited for the walk");
        assert_eq!(during, [env.path("a/old.txt")]);
        assert!(!has(&live, &env.path("b/fresh.txt")));

        release.send(()).unwrap();
        wait_for("the new index", || has(&live, &env.path("b/fresh.txt")));
        assert!(!has(&live, &env.path("a/old.txt")));
    }

    /// The options of a run that keeps its index in `snapshot` and whose walks wait for
    /// `gate` (a message, or the gate dropped) before they are swapped in.
    fn held_walks(snapshot: PathBuf, gate: mpsc::Receiver<()>) -> Options {
        let gate = Mutex::new(gate);
        Options {
            snapshot: Some(snapshot),
            walk: Box::new(move |rules| {
                let index = FileIndex::walk(rules);
                let _ = gate.lock().unwrap().recv_timeout(PATIENCE);
                index
            }),
            ..test_options()
        }
    }

    #[test]
    fn the_saved_index_is_searched_before_the_first_walk_ends() {
        let env = Env::new();
        touch(&env.path("a/old.txt"));
        env.write_config(&env.roots_config(&["a"]));
        let snapshot = env.path("data/index.bin");

        // The first run walks and saves.
        let live = env.start(Options {
            snapshot: Some(snapshot.clone()),
            ..test_options()
        });
        wait_for("first walk", || has(&live, &env.path("a/old.txt")));
        wait_for("the saved index", || snapshot.exists());
        live.stop();
        drop(live);
        touch(&env.path("a/new.txt"));

        // The next one finds what was saved while its walk is held back, then the new file.
        let (release, gate) = mpsc::channel();
        let live = env.start(held_walks(snapshot.clone(), gate));
        wait_for("the saved index read back", || live.is_ready());
        assert_eq!(found(&live, "old", &env.home), [env.path("a/old.txt")]);
        assert!(
            !has(&live, &env.path("a/new.txt")),
            "the walk is still held"
        );
        release.send(()).unwrap();
        wait_for("the walk", || has(&live, &env.path("a/new.txt")));
    }

    #[test]
    fn a_saved_index_of_other_roots_is_not_read() {
        let env = Env::new();
        touch(&env.path("a/old.txt"));
        touch(&env.path("b/other.txt"));
        env.write_config(&env.roots_config(&["a"]));
        let snapshot = env.path("data/index.bin");
        let live = env.start(Options {
            snapshot: Some(snapshot.clone()),
            ..test_options()
        });
        wait_for("the saved index", || snapshot.exists());
        live.stop();
        drop(live);

        env.write_config(&env.roots_config(&["b"]));
        let (release, gate) = mpsc::channel();
        let live = env.start(held_walks(snapshot, gate));
        // Not ready until the walk of the new roots is in: nothing of `a` is shown meanwhile.
        std::thread::sleep(Duration::from_millis(300));
        assert!(!live.is_ready());
        assert!(!has(&live, &env.path("a/old.txt")));
        release.send(()).unwrap();
        wait_for("the walk", || live.is_ready());
        assert!(has(&live, &env.path("b/other.txt")));
    }

    #[test]
    fn ignored_folder_is_not_watched() {
        let env = Env::new();
        touch(&env.path("proj/src/main.rs"));
        touch(&env.path("proj/target/a.o"));
        fs::create_dir_all(env.path("proj/.git")).unwrap();
        fs::write(env.path("proj/.gitignore"), "out/\n").unwrap();
        touch(&env.path("proj/out/b.o"));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("proj/src/main.rs")));

        let watched = live.watched();
        let want: BTreeSet<PathBuf> = [env.home.clone(), env.path("proj"), env.path("proj/src")]
            .into_iter()
            .collect();
        assert_eq!(watched, want);

        touch(&env.path("proj/target/new.o"));
        touch(&env.path("proj/out/new.o"));
        touch(&env.path("proj/target/sub/x.o"));
        // Written last, in a watched folder: once it is in, the writes above were handled.
        let sentinel = env.path("proj/src/sentinel.rs");
        touch(&sentinel);
        wait_for("sentinel", || has(&live, &sentinel));
        assert!(!has(&live, &env.path("proj/target/new.o")));
        assert!(!has(&live, &env.path("proj/out/new.o")));
        assert_eq!(live.watched(), want);
    }

    #[test]
    fn config_change_of_roots_rewalks() {
        let env = Env::new();
        touch(&env.path("a/one.txt"));
        touch(&env.path("b/two.txt"));
        env.write_config(&env.roots_config(&["a"]));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("a/one.txt")));
        assert!(!has(&live, &env.path("b/two.txt")));

        env.write_config(&env.roots_config(&["b"]));
        wait_for("b walked", || has(&live, &env.path("b/two.txt")));
        assert!(!has(&live, &env.path("a/one.txt")));
        let watched = live.watched();
        assert!(watched.contains(&env.path("b")));
        assert!(!watched.contains(&env.path("a")));
    }

    #[test]
    fn in_root_change_reaches_the_module() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let walks = Arc::new(AtomicUsize::new(0));
        let opts = Options {
            walk: Box::new({
                let walks = walks.clone();
                move |rules| {
                    walks.fetch_add(1, Ordering::SeqCst);
                    FileIndex::walk(rules)
                }
            }),
            ..test_options()
        };
        let live = Live::start_with(env.config.clone(), env.home.clone(), opts, {
            let seen = seen.clone();
            move |files: &FilesSettings| seen.lock().unwrap().push(files.in_root)
        });
        wait_for("first walk", || has(&live, &env.path("seed.txt")));
        assert_eq!(*seen.lock().unwrap(), [true], "called once at start");

        env.write_config("[files]\nin_root = false\n");
        wait_for("in_root = false", || {
            seen.lock().unwrap().last() == Some(&false)
        });
        // Coverage did not change: no re-walk. A file written now is picked up by the watcher.
        let sentinel = env.path("sentinel.txt");
        touch(&sentinel);
        wait_for("sentinel", || has(&live, &sentinel));
        assert_eq!(walks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn broken_config_keeps_the_coverage() {
        let env = Env::new();
        touch(&env.path("a/one.txt"));
        touch(&env.path("b/two.txt"));
        env.write_config(&env.roots_config(&["a"]));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let live = Live::start_with(env.config.clone(), env.home.clone(), test_options(), {
            let seen = seen.clone();
            move |files: &FilesSettings| seen.lock().unwrap().push(files.roots.clone())
        });
        wait_for("first walk", || has(&live, &env.path("a/one.txt")));

        // Half-saved: unreadable, so the coverage stays and nothing is passed on.
        env.write_config("[files\nroots = ");
        let sentinel = env.path("a/sentinel.txt");
        touch(&sentinel);
        wait_for("sentinel", || has(&live, &sentinel));
        assert!(has(&live, &env.path("a/one.txt")));

        let both = env.roots_config(&["a", "b"]);
        env.write_config(&both);
        wait_for("b walked", || has(&live, &env.path("b/two.txt")));
        let root = |r: &str| env.path(r).to_str().unwrap().to_string();
        // One save may reach the thread as two batches: the same table twice is fine.
        let mut seen = seen.lock().unwrap().clone();
        seen.dedup();
        assert_eq!(
            seen,
            [vec![root("a")], vec![root("a"), root("b")]],
            "a broken file is not passed on"
        );
    }

    #[test]
    fn over_the_cap_rewalks_instead_of_watching() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let opts = Options {
            watch_cap: 0,
            poll_every: Duration::from_millis(300),
            ..test_options()
        };
        let live = env.start(opts);
        wait_for("first walk", || has(&live, &env.path("seed.txt")));
        assert!(live.watched().is_empty());

        let new = env.path("new.txt");
        touch(&new);
        wait_for("new.txt from a re-walk", || has(&live, &new));
        assert!(live.watched().is_empty());
    }

    #[test]
    fn stop_ends_the_thread() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("seed.txt")));
        live.stop();
        assert!(live.ended_within(PATIENCE));
    }

    #[test]
    fn change_during_first_walk_is_found() {
        let env = Env::new();
        touch(&env.path("seed.txt"));
        touch(&env.path("gone.txt"));
        touch(&env.path("sub/also-gone.txt"));
        let (during, gone, also_gone) = (
            env.path("during.txt"),
            env.path("gone.txt"),
            env.path("sub/also-gone.txt"),
        );
        let first = AtomicUsize::new(0);
        let opts = Options {
            walk: Box::new({
                let (during, gone, also_gone) = (during.clone(), gone.clone(), also_gone.clone());
                move |rules| {
                    let index = FileIndex::walk(rules);
                    if first.fetch_add(1, Ordering::SeqCst) == 0 {
                        // After the walk passed every folder, before anything is watched.
                        touch(&during);
                        fs::remove_file(&gone).unwrap();
                        fs::remove_file(&also_gone).unwrap();
                    }
                    index
                }
            }),
            ..test_options()
        };
        let live = env.start(opts);
        wait_for("first walk", || has(&live, &env.path("seed.txt")));
        wait_for("the changes made during the walk", || {
            has(&live, &during) && !has(&live, &gone) && !has(&live, &also_gone)
        });
    }

    #[test]
    fn big_new_folder_does_not_block_readers() {
        const FILES: usize = 20_000;
        let env = Env::new();
        touch(&env.path("seed.txt"));
        // Made outside the home folder, then moved in at once.
        let staging = env.home.parent().unwrap().join("staging");
        for i in 0..FILES {
            touch(&staging.join(format!("d{}/file-{i}.txt", i % 100)));
        }
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("seed.txt")));
        let before = live.read(|i| i.len());

        let done = Arc::new(AtomicBool::new(false));
        let reader = std::thread::spawn({
            let (live, done) = (live.clone(), done.clone());
            move || {
                let mut waits = Vec::new();
                while !done.load(Ordering::SeqCst) {
                    let asked = Instant::now();
                    waits.push(live.read(|_| asked.elapsed()));
                    std::thread::sleep(Duration::from_millis(1));
                }
                waits
            }
        });
        fs::rename(&staging, env.path("big")).unwrap();
        // The folder, its 100 subfolders and the files.
        let want = before + 1 + 100 + FILES;
        wait_for("all of big indexed", || live.read(|i| i.len()) == want);
        done.store(true, Ordering::SeqCst);
        let mut waits = reader.join().unwrap();
        waits.sort_unstable();
        let slow = waits
            .iter()
            .filter(|w| **w >= Duration::from_millis(50))
            .count();
        let worst = waits.last().copied().unwrap_or_default();
        // A read held up for the whole insert waits hundreds of milliseconds. One slow read
        // is let pass: on a machine loaded far past its cores, either thread can be taken off
        // the CPU for that long, lock or not.
        assert!(
            slow <= 1 && worst < Duration::from_millis(200),
            "{slow} of {} reads waited 50 ms or more, the longest {worst:?}",
            waits.len()
        );
    }

    #[test]
    fn watch_limit_backs_off_until_the_config_changes() {
        let env = Env::new();
        touch(&env.path("sub/x.txt"));
        let opts = Options {
            // home and sub: the second watch fails as if the system's limit were reached.
            fake_watch_limit: Some(1),
            poll_every: Duration::from_millis(100),
            ..test_options()
        };
        let live = env.start(opts);
        wait_for("first walk", || has(&live, &env.path("sub/x.txt")));
        let walks = live.seen().walks;
        wait_for("two more walks", || live.seen().walks >= walks + 2);
        let seen = live.seen();
        assert_eq!(
            seen.watchers, 1,
            "a new watcher was made after the limit was hit"
        );
        assert!(seen.watched.is_empty());

        // A changed coverage tries again: one folder fits.
        env.write_config(&env.roots_config(&["sub"]));
        wait_for("watching sub", || {
            live.watched() == BTreeSet::from([env.path("sub")])
        });
        assert_eq!(live.seen().watchers, 2);
    }

    #[test]
    fn hidden_and_excluded_names_are_skipped_before_the_index() {
        let home = Path::new("/h");
        let c = Coverage {
            roots: vec!["~".into()],
            exclude: vec!["*.iso".into()],
            hidden: false,
            gitignore: true,
            global_ignore: false,
        };
        let rules = Rules::new(&c, home);
        assert!(skipped(&rules, Path::new("/h/.cache/x")));
        assert!(skipped(&rules, Path::new("/h/.bash_history")));
        assert!(skipped(&rules, Path::new("/h/p/node_modules/m.js")));
        assert!(skipped(&rules, Path::new("/h/p/target/debug/x")));
        assert!(skipped(&rules, Path::new("/h/disk.iso")));
        assert!(skipped(&rules, Path::new("/elsewhere/a.txt")));
        assert!(!skipped(&rules, Path::new("/h/p/src/main.rs")));
        // A folder named like a built-in skip may be a file; upsert decides.
        assert!(!skipped(&rules, Path::new("/h/p/target")));
        assert!(!skipped(&rules, Path::new("/h")));

        let hidden = Rules::new(&Coverage { hidden: true, ..c }, home);
        assert!(!skipped(&hidden, Path::new("/h/.cache/x")));
    }

    /// Root reads any folder; the tests of unreadable folders mean nothing then.
    fn running_as_root() -> bool {
        let probe = TempDir::new().unwrap();
        let dir = probe.path().join("locked");
        let _locked = Locked::new(&dir);
        fs::read_dir(&dir).is_ok()
    }

    /// A folder made with mode 000; readable again when dropped, so its temp dir can go.
    struct Locked(PathBuf);

    impl Locked {
        fn new(dir: &Path) -> Locked {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir_all(dir).unwrap();
            fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
            Locked(dir.to_path_buf())
        }
    }

    impl Drop for Locked {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn unreadable_folder_does_not_spin() {
        if running_as_root() {
            return;
        }
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let live = env.start(test_options());
        wait_for("first walk", || has(&live, &env.path("seed.txt")));

        // Can be looked at but neither read nor watched, like another user's 0700 folder.
        let _locked = Locked::new(&env.path("locked"));
        let after = env.path("after.txt");
        touch(&after);
        wait_for("after.txt", || has(&live, &after));
        live.stop();
        assert!(live.ended_within(Duration::from_secs(5)));
    }

    #[test]
    fn unreadable_folder_during_first_walk() {
        if running_as_root() {
            return;
        }
        let env = Env::new();
        touch(&env.path("seed.txt"));
        let locked = Arc::new(Mutex::new(None));
        let first = AtomicUsize::new(0);
        let opts = Options {
            walk: Box::new({
                let (locked, dir) = (locked.clone(), env.path("locked"));
                move |rules| {
                    let index = FileIndex::walk(rules);
                    if first.fetch_add(1, Ordering::SeqCst) == 0 {
                        *locked.lock().unwrap() = Some(Locked::new(&dir));
                    }
                    index
                }
            }),
            ..test_options()
        };
        let live = env.start(opts);
        wait_for("first walk swapped in", || {
            has(&live, &env.path("seed.txt"))
        });
        let sentinel = env.path("sentinel.txt");
        touch(&sentinel);
        wait_for("sentinel", || has(&live, &sentinel));
        live.stop();
        assert!(live.ended_within(Duration::from_secs(5)));
        drop(locked);
    }
}
