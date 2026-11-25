//! The plugin manager: discovery, hot reload and preferences.
//!
//! The manager owns everything about plugin folders that may take time — reading and hashing a
//! package, the approval gate, compiling a component — and runs it off the UI thread. What the
//! core has to do comes out as [`ManagerAction`]s, which the daemon applies to `Core` on the UI
//! thread, then re-ranks the open window's results.
//!
//! One path loads a plugin, whatever asked for it (the startup scan, a changed folder, the
//! user's decision in the review prompt):
//!
//! 1. `plugin.toml` and `plugin.wasm` are read once ([`Package::read`]); the same bytes are
//!    hashed, compiled and run;
//! 2. a plugin whose id is that of a built-in module is refused;
//! 3. [`approval::check`] decides; only [`Gate::Run`] goes on, so nothing of an unapproved
//!    plugin is compiled or instantiated;
//! 4. the component comes from the cache, compiled by the compile child on a miss;
//! 5. the plugin is registered; a running instance of it is unregistered first.
//!
//! What the manager remembers per plugin folder:
//!
//! - the content hash of the running instance: the same content found again changes nothing;
//! - the last failure (a read, a compile, a broken instance): it is logged once, and the same
//!   content is not tried again. Only a change of the folder retries, never a timer;
//! - the pending reviews, all of which go to the `plugins` module as one list.
//!
//! A folder removed while the manager runs unloads its plugin and deletes its grant and data
//! (Rulings AG and AR), so a folder created again later is reviewed again and starts empty.
//! A scan never deletes grants or data (Ruling AR): a folder missing at startup may be on a
//! volume that is not mounted yet.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use tomakast_core::{
    CatalogPlugin, HomeEntry, Module, Paths, PluginCatalog, PluginState, Settings,
};

use crate::approval::{self, Gate, PluginLinkPolicy, ReviewState};
use crate::cache::Cache;
use crate::catalog::catalog_entry;
use crate::db::PluginDb;
use crate::engine::HostEngine;
use crate::package::Package;
use crate::plugin::{PluginEvents, PreferencesHandle, WasmPlugin};
use crate::watch::{Debouncer, Key, Watch};

/// What the daemon applies on the UI thread, in order.
pub enum ManagerAction {
    /// Register the plugin with [`tomakast_core::Core::register`].
    Register(Box<dyn Module>),
    /// Unregister the plugin with this id ([`tomakast_core::Core::unregister`]). A reload
    /// sends it before the new instance's `Register`.
    Unregister(String),
    /// The plugins now waiting for the user's decision, all of them: hand them to
    /// [`crate::PluginsHandle::set_reviews`].
    Reviews(Vec<ReviewState>),
    /// The user's Home list, read again from a changed `config.toml`: hand it to
    /// [`tomakast_core::Core::set_home`], which changes nothing when it is the same.
    Home(Vec<HomeEntry>),
    /// Every installed plugin with its state, for the settings window. Sent after a scan, a
    /// changed folder or a report whenever the list differs from the one sent last.
    Catalog(PluginCatalog),
}

impl std::fmt::Debug for ManagerAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManagerAction::Register(m) => f.debug_tuple("Register").field(&m.id()).finish(),
            ManagerAction::Unregister(id) => f.debug_tuple("Unregister").field(id).finish(),
            ManagerAction::Reviews(r) => {
                let ids: Vec<&str> = r.iter().map(|r| r.id.as_str()).collect();
                f.debug_tuple("Reviews").field(&ids).finish()
            }
            ManagerAction::Home(home) => {
                let widgets: Vec<String> = home.iter().map(|e| e.widget.to_string()).collect();
                f.debug_tuple("Home").field(&widgets).finish()
            }
            ManagerAction::Catalog(catalog) => {
                let ids: Vec<&str> = catalog.plugins.iter().map(|p| p.id.as_str()).collect();
                f.debug_tuple("Catalog").field(&ids).finish()
            }
        }
    }
}

/// Counters of what the manager did, for logs and tests.
#[derive(Debug, Default)]
pub struct ManagerStats {
    /// Plugin folders read ([`Package::read`] attempts).
    pub reads: AtomicUsize,
    /// Components loaded through the cache ([`HostEngine::load`] attempts).
    pub compiles: AtomicUsize,
    /// Failures logged as warnings (a repeated failure is not).
    pub failures_logged: AtomicUsize,
    /// `preferences-changed` deliveries to running plugins.
    pub preferences_delivered: AtomicUsize,
    /// Times `config.toml` was read after a change.
    pub config_reads: AtomicUsize,
}

/// The user's plugin preferences: `[plugins.<id>]` of `config.toml`, by plugin id.
pub type Preferences = BTreeMap<String, BTreeMap<String, String>>;

/// The running instance of a plugin.
struct Loaded {
    content_hash: [u8; 32],
    prefs: PreferencesHandle,
}

/// The last failure of a plugin folder: `content_hash` is `None` when the folder could not be
/// read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    content_hash: Option<[u8; 32]>,
    message: String,
}

pub struct PluginManager {
    plugins_dir: PathBuf,
    config_file: PathBuf,
    db_file: PathBuf,
    engine: HostEngine,
    cache: Cache,
    /// The executable whose `compile-plugin` subcommand compiles components.
    compile_exe: PathBuf,
    db: PluginDb,
    prefs: Preferences,
    /// The plugins the user turned off ([`Settings::disabled_plugins`]).
    disabled: BTreeSet<String>,
    native_ids: HashSet<String>,
    link_policy: Option<PluginLinkPolicy>,
    events_tx: mpsc::Sender<PluginEvents>,
    events_rx: Option<mpsc::Receiver<PluginEvents>>,
    loaded: HashMap<String, Loaded>,
    failed: HashMap<String, Failure>,
    reviews: BTreeMap<String, ReviewState>,
    /// Every plugin folder with its state, by folder name.
    catalog: BTreeMap<String, CatalogPlugin>,
    /// Why the plugins folder could not be listed at the last scan, when it is there.
    unlisted: Option<String>,
    /// The catalog last sent as an action; `None` before the first, which is always sent.
    catalog_sent: Option<PluginCatalog>,
    stats: Arc<ManagerStats>,
}

impl PluginManager {
    /// A manager for the plugins in `paths.plugins_dir`, compiling into
    /// `paths.plugin_cache_dir` (whose stale temp files are swept now). `db` is the manager's
    /// own handle; each plugin gets its own. `prefs` are the user's current preferences
    /// ([`Settings::plugin_preferences`]).
    pub fn new(paths: &Paths, engine: HostEngine, db: PluginDb, prefs: Preferences) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        PluginManager {
            plugins_dir: paths.plugins_dir.clone(),
            config_file: paths.config_file.clone(),
            db_file: paths.db_file.clone(),
            engine,
            cache: Cache::new(paths.plugin_cache_dir.clone()),
            compile_exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tomakast")),
            db,
            prefs,
            disabled: BTreeSet::new(),
            native_ids: HashSet::new(),
            link_policy: None,
            events_tx,
            events_rx: Some(events_rx),
            loaded: HashMap::new(),
            failed: HashMap::new(),
            reviews: BTreeMap::new(),
            catalog: BTreeMap::new(),
            unlisted: None,
            catalog_sent: None,
            stats: Arc::default(),
        }
    }

    /// The executable to run as `<exe> compile-plugin`; the current executable by default.
    pub fn with_compile_exe(mut self, exe: PathBuf) -> Self {
        self.compile_exe = exe;
        self
    }

    /// The plugins the user turned off ([`Settings::disabled_plugins`]): none of them is
    /// loaded, from the startup scan on. Read again whenever `config.toml` changes.
    pub fn with_disabled(mut self, disabled: BTreeSet<String>) -> Self {
        self.disabled = disabled;
        self
    }

    /// The ids of the built-in modules the daemon registers: a plugin with one of them is
    /// refused (registering it would replace the built-in module).
    pub fn with_native_ids(mut self, ids: HashSet<String>) -> Self {
        self.native_ids = ids;
        self
    }

    /// Where plugins' `location.request` finds the location, and the time zone
    /// `locale.local-time-zone` answers: the user's `[location]` settings. Read again whenever
    /// `config.toml` changes.
    pub fn with_location(self, settings: &tomakast_core::LocationSettings) -> Self {
        crate::location_sources::configure(settings);
        crate::imports::locale::configure_zone(&settings.timezone);
        self
    }

    /// The link policy the launcher asks; the manager records every load and unload in it.
    pub fn with_link_policy(mut self, policy: PluginLinkPolicy) -> Self {
        self.link_policy = Some(policy);
        self
    }

    /// Where plugin instances and the `plugins` review module report to the manager.
    pub fn events(&self) -> mpsc::Sender<PluginEvents> {
        self.events_tx.clone()
    }

    pub fn stats(&self) -> Arc<ManagerStats> {
        Arc::clone(&self.stats)
    }

    /// Reads every plugin folder and syncs the plugins with them. A loaded or reviewed plugin
    /// whose folder is gone is removed (Rulings AG and AR); a grant without a loaded or
    /// reviewed plugin is left alone, also at startup (Ruling AR).
    ///
    /// Content that failed before is not tried again here: a rescan is not a change.
    ///
    /// When the plugins folder itself cannot be listed, every plugin is unloaded and no grant
    /// is touched (Ruling AI): a vanished `plugins` folder is not the removal of each plugin.
    /// A folder that is there but cannot be listed makes the catalog unavailable, with why.
    pub fn scan(&mut self) -> Vec<ManagerAction> {
        let before = self.reviews.clone();
        let mut actions = Vec::new();
        match self.folder_names() {
            Ok(names) => {
                self.unlisted = None;
                for name in &names {
                    self.sync(name, Retry::No, &mut actions);
                }
                let gone: Vec<String> = self
                    .loaded
                    .keys()
                    .chain(self.reviews.keys())
                    .filter(|id| !names.contains(*id))
                    .cloned()
                    .collect();
                for id in gone {
                    self.removed(&id, &mut actions);
                }
                self.failed.retain(|name, _| names.contains(name));
                self.catalog.retain(|name, _| names.contains(name));
                // Ruling AR: no grant or data is deleted for a folder that is merely missing
                // here (at startup it may be on a volume not mounted yet).
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    tracing::warn!(target: "plugin", "the plugins folder {} was removed: plugins are unloaded, and new ones are found only after a restart", self.plugins_dir.display());
                    self.unlisted = None;
                } else {
                    tracing::error!(target: "plugin", "cannot list {}: {e}; plugins are unloaded", self.plugins_dir.display());
                    self.unlisted =
                        Some(format!("cannot list {}: {e}", self.plugins_dir.display()));
                }
                self.unload_all(&mut actions);
            }
        }
        self.push_reviews(before, &mut actions);
        self.push_catalog(&mut actions);
        actions
    }

    /// Something in the plugin folder `name` changed (and has been quiet since): it is read
    /// and gated again, and content that failed before is tried again; a removed folder is
    /// unloaded.
    pub fn folder_changed(&mut self, name: &str) -> Vec<ManagerAction> {
        let before = self.reviews.clone();
        let mut actions = Vec::new();
        self.sync(name, Retry::Yes, &mut actions);
        self.push_reviews(before, &mut actions);
        self.push_catalog(&mut actions);
        actions
    }

    /// Handles a report of a plugin instance or of the review module.
    ///
    /// - `Broken`: the instance is unloaded, and its content is not loaded again until the
    ///   folder changes. A report of an instance that is no longer the running one is ignored.
    /// - `Approved` and `Denied`: the folder is read and gated again; nothing is loaded on the
    ///   event's word alone.
    /// - `PreferenceDefault`: the catalog shows the value as what the unset preference stands
    ///   for, until the plugin says otherwise or is loaded again; an empty value takes it
    ///   back. A report of an instance that is no longer the running one is ignored.
    pub fn on_event(&mut self, event: PluginEvents) -> Vec<ManagerAction> {
        let before = self.reviews.clone();
        let mut actions = Vec::new();
        match event {
            PluginEvents::Broken {
                id,
                content_hash,
                reason,
            } => {
                let running = self
                    .loaded
                    .get(&id)
                    .is_some_and(|l| l.content_hash == content_hash);
                if running {
                    self.unload(&id, &mut actions);
                    // Logged by the plugin already.
                    self.set_state(&id, PluginState::Failed(reason.clone()));
                    self.failed.insert(
                        id,
                        Failure {
                            content_hash: Some(content_hash),
                            message: reason,
                        },
                    );
                }
            }
            PluginEvents::Approved { id, .. } | PluginEvents::Denied { id, .. } => {
                if is_folder_name(&id) {
                    self.sync(&id, Retry::No, &mut actions);
                }
            }
            PluginEvents::PreferenceDefault {
                id,
                content_hash,
                key,
                value,
            } => {
                let running = self
                    .loaded
                    .get(&id)
                    .is_some_and(|l| l.content_hash == content_hash);
                let pref = self
                    .catalog
                    .get_mut(&id)
                    .and_then(|p| p.preferences.iter_mut().find(|p| p.key == key));
                if let (true, Some(pref)) = (running, pref) {
                    pref.reported_default = (!value.is_empty()).then_some(value);
                }
            }
        }
        self.push_reviews(before, &mut actions);
        self.push_catalog(&mut actions);
        actions
    }

    /// The user's preferences changed: each running plugin whose `[plugins.<id>]` table
    /// differs gets the new values and a `preferences-changed` event. Plugins loaded later
    /// start with these values.
    pub fn set_preferences(&mut self, prefs: Preferences) {
        for (id, loaded) in &self.loaded {
            let new = prefs.get(id);
            if new != self.prefs.get(id) {
                loaded.prefs.set(new.cloned().unwrap_or_default());
                self.stats
                    .preferences_delivered
                    .fetch_add(1, Ordering::SeqCst);
                tracing::info!(target: "plugin", plugin = %id, "preferences changed");
            }
        }
        self.prefs = prefs;
    }

    /// The time zone changed: every running plugin gets a `preferences-changed` event (with
    /// its preferences as they are), so what it shows of the time is redone.
    fn zone_changed(&mut self) {
        for (id, loaded) in &self.loaded {
            loaded
                .prefs
                .set(self.prefs.get(id).cloned().unwrap_or_default());
        }
    }

    /// Runs the manager on its own thread: the startup scan first (its actions are always
    /// passed to `sink`, even when empty), then the watcher on the plugins folder and on
    /// `config.toml`, debounced to [`crate::watch::QUIET`] per folder, and the reports of
    /// plugins and of the review module. `sink` gets every later non-empty batch of actions.
    ///
    /// The thread blocks while nothing happens. It stops when the returned handle is dropped.
    pub fn spawn(
        mut self,
        mut sink: impl FnMut(Vec<ManagerAction>) + Send + 'static,
    ) -> anyhow::Result<ManagerThread> {
        let (input, rx) = mpsc::channel::<Input>();
        if let Err(e) = std::fs::create_dir_all(&self.plugins_dir) {
            tracing::error!(target: "plugin", "cannot create {}: {e}", self.plugins_dir.display());
        }
        let changes = input.clone();
        let watch = Watch::start(&self.plugins_dir, &self.config_file, move |key| {
            // Fails only when the manager thread is gone.
            let _ = changes.send(Input::Changed(key));
        });
        let watch = match watch {
            Ok(watch) => Some(watch),
            Err(e) => {
                tracing::error!(target: "plugin", "plugins are not reloaded on change: {e:#}");
                None
            }
        };
        let events = self
            .events_rx
            .take()
            .context("the manager was spawned twice")?;
        let forward = input.clone();
        std::thread::Builder::new()
            .name("plugin-events".into())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    if forward.send(Input::Plugin(event)).is_err() {
                        return;
                    }
                }
            })
            .context("starting the plugin events thread")?;
        std::thread::Builder::new()
            .name("plugin-manager".into())
            .spawn(move || {
                let _watch = watch;
                sink(self.scan());
                self.run(&rx, sink);
            })
            .context("starting the plugin manager thread")?;
        Ok(ManagerThread { input })
    }

    fn run(&mut self, rx: &mpsc::Receiver<Input>, mut sink: impl FnMut(Vec<ManagerAction>)) {
        let mut debouncer = Debouncer::default();
        loop {
            let input = match debouncer.next_deadline() {
                Some(deadline) => {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(input) => Some(input),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match rx.recv() {
                    Ok(input) => Some(input),
                    Err(_) => return,
                },
            };
            let mut actions = Vec::new();
            match input {
                Some(Input::Changed(key)) => debouncer.touch(key, Instant::now()),
                Some(Input::Plugin(event)) => actions.extend(self.on_event(event)),
                Some(Input::Stop) => return,
                None => {}
            }
            for key in debouncer.take_due(Instant::now()) {
                match key {
                    Key::All => actions.extend(self.scan()),
                    Key::Folder(name) => actions.extend(self.folder_changed(&name)),
                    Key::Config => actions.extend(self.reload_config()),
                }
            }
            if !actions.is_empty() {
                sink(actions);
            }
        }
    }

    /// Reads `config.toml` again: hands the plugins their new preferences and the location
    /// settings to the location sources, turns plugins on and off as the switches say, and
    /// returns the new Home list for the core (after what the switches change). A file that
    /// does not load changes nothing.
    pub fn reload_config(&mut self) -> Vec<ManagerAction> {
        self.stats.config_reads.fetch_add(1, Ordering::SeqCst);
        match Settings::load(&self.config_file) {
            Ok(settings) => {
                crate::location_sources::configure(&settings.location);
                if crate::imports::locale::configure_zone(&settings.location.timezone) {
                    self.zone_changed();
                }
                // Preferences first: a plugin turned on in the same edit starts with them.
                self.set_preferences(settings.plugin_preferences);
                let mut actions = self.set_disabled(settings.disabled_plugins);
                actions.push(ManagerAction::Home(settings.home));
                actions
            }
            Err(e) => {
                tracing::warn!(target: "plugin", "{e:#}; plugin preferences, switches, location settings and Home stay as they were");
                Vec::new()
            }
        }
    }

    /// The user's switches changed: each plugin turned off is unloaded (and its review
    /// dropped), each turned on is read and gated again, failed content tried again. Only
    /// plugins in the catalog are synced: a switch of a plugin that is not installed does
    /// nothing (a sync of a missing folder would be its removal).
    fn set_disabled(&mut self, disabled: BTreeSet<String>) -> Vec<ManagerAction> {
        if disabled == self.disabled {
            return Vec::new();
        }
        let changed: Vec<String> = self
            .disabled
            .symmetric_difference(&disabled)
            .filter(|id| self.catalog.contains_key(*id))
            .cloned()
            .collect();
        self.disabled = disabled;
        let before = self.reviews.clone();
        let mut actions = Vec::new();
        for id in changed {
            let on = !self.disabled.contains(&id);
            tracing::info!(target: "plugin", plugin = %id, "turned {}", if on { "on" } else { "off" });
            self.sync(&id, if on { Retry::Yes } else { Retry::No }, &mut actions);
        }
        self.push_reviews(before, &mut actions);
        self.push_catalog(&mut actions);
        actions
    }

    fn folder_names(&self) -> std::io::Result<Vec<String>> {
        plugin_folder_names(&self.plugins_dir)
    }

    /// Brings the plugin of folder `name` in line with what the folder holds now.
    ///
    /// Only a folder that is not there (`NotFound`, in a plugins folder that is) is a removal
    /// (Ruling AI). Any other problem — no permission, a dangling link, a file in its place —
    /// is a failed read: the plugin is unloaded and its grant kept.
    fn sync(&mut self, name: &str, retry: Retry, actions: &mut Vec<ManagerAction>) {
        if !is_folder_name(name) {
            return;
        }
        let dir = self.plugins_dir.join(name);
        match std::fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&self.plugins_dir).is_ok() {
                    self.removed(name, actions);
                } else {
                    // The plugins folder itself went away; `scan` handles that.
                    self.unload(name, actions);
                    self.reviews.remove(name);
                    self.catalog.remove(name);
                }
                return;
            }
            Err(e) => {
                self.unload(name, actions);
                self.reviews.remove(name);
                let message = format!("cannot read the plugin folder: {e}");
                self.set_unreadable(name, &message);
                self.fail(name, None, message);
                return;
            }
            // A stray file (a README) is no plugin, unless one was known by that name.
            Ok(meta)
                if meta.is_file()
                    && !self.loaded.contains_key(name)
                    && !self.reviews.contains_key(name) =>
            {
                return;
            }
            Ok(_) => {}
        }
        self.stats.reads.fetch_add(1, Ordering::SeqCst);
        let package = match Package::read(&dir) {
            Ok(package) => package,
            Err(e) => {
                self.unload(name, actions);
                self.reviews.remove(name);
                let message = format!("cannot read the plugin: {e}");
                self.set_unreadable(name, &message);
                self.fail(name, None, message);
                return;
            }
        };
        // `Package::read` checked that the manifest id is the folder name.
        let id = package.manifest.id.clone();
        let content_hash = package.content_hash;
        if self.native_ids.contains(&id) {
            self.unload(&id, actions);
            self.reviews.remove(&id);
            let message = "its id is the id of a built-in module".to_owned();
            self.set_package_state(&package, PluginState::Failed(message.clone()));
            self.fail(&id, Some(content_hash), message);
            return;
        }
        if self.disabled.contains(&id) {
            // Turned off: not gated, so no review is asked and no approval changes; nothing
            // of it is compiled or run.
            self.unload(&id, actions);
            self.reviews.remove(&id);
            self.set_package_state(&package, PluginState::Off);
            return;
        }
        match approval::check(&self.db, &package, unix_now()) {
            Gate::Run(approved_at) => {
                self.reviews.remove(&id);
                self.load(package, retry, approved_at, actions);
            }
            Gate::Review(review) => {
                self.unload(&id, actions);
                self.failed.remove(&id);
                self.set_package_state(&package, PluginState::Review);
                self.reviews.insert(id, *review);
            }
            Gate::Denied => {
                self.unload(&id, actions);
                self.reviews.remove(&id);
                self.set_package_state(&package, PluginState::Denied);
            }
        }
    }

    /// Loads an approved package, unless it is the running content, or content that failed
    /// and `retry` is `No`.
    fn load(
        &mut self,
        package: Package,
        retry: Retry,
        approved_at: i64,
        actions: &mut Vec<ManagerAction>,
    ) {
        let id = package.manifest.id.clone();
        let content_hash = package.content_hash;
        if self
            .loaded
            .get(&id)
            .is_some_and(|l| l.content_hash == content_hash)
        {
            return;
        }
        if retry == Retry::No
            && self
                .failed
                .get(&id)
                .is_some_and(|f| f.content_hash == Some(content_hash))
        {
            // A rescan or a decision is no change of the folder: what failed is not tried.
            self.unload(&id, actions);
            if let Some(failure) = self.failed.get(&id) {
                let state = PluginState::Failed(failure.message.clone());
                self.set_package_state(&package, state);
            }
            return;
        }
        self.stats.compiles.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        let component = self.engine.load(
            package.wasm_hash,
            &package.wasm,
            &self.cache,
            &self.compile_exe,
        );
        let component = match component {
            Ok(component) => component,
            Err(e) => {
                self.unload(&id, actions);
                let message = format!("cannot compile: {e:#}");
                self.set_package_state(&package, PluginState::Failed(message.clone()));
                self.fail(&id, Some(content_hash), message);
                return;
            }
        };
        let db = match PluginDb::open(&self.db_file) {
            Ok(db) => db,
            Err(e) => {
                self.unload(&id, actions);
                let message = format!("{e:#}");
                self.set_package_state(&package, PluginState::Failed(message.clone()));
                self.fail(&id, Some(content_hash), message);
                return;
            }
        };
        let permissions = package.manifest.permissions.clone();
        let name = package.manifest.name.clone();
        let version = package.manifest.version.clone();
        self.set_package_state(&package, PluginState::Running);
        let prefs = self.prefs.get(&id).cloned().unwrap_or_default();
        let plugin = WasmPlugin::new(package, component, &self.engine, db, prefs)
            .with_events(self.events_tx.clone())
            .with_approved_at(approved_at);
        let handle = plugin.preferences();
        // A reload: the old instance goes first.
        self.unload(&id, actions);
        if let Some(policy) = &self.link_policy {
            policy.set_plugin(&id, &name, &permissions);
        }
        self.failed.remove(&id);
        self.loaded.insert(
            id.clone(),
            Loaded {
                content_hash,
                prefs: handle,
            },
        );
        tracing::info!(target: "plugin", plugin = %id, %version, elapsed = ?started.elapsed(), "plugin loaded");
        actions.push(ManagerAction::Register(Box::new(plugin)));
    }

    /// Unloads every plugin and drops every review; grants stay.
    fn unload_all(&mut self, actions: &mut Vec<ManagerAction>) {
        let ids: Vec<String> = self.loaded.keys().cloned().collect();
        for id in ids {
            self.unload(&id, actions);
        }
        self.reviews.clear();
        self.catalog.clear();
    }

    /// Unloads the running instance of `id`, if any.
    fn unload(&mut self, id: &str, actions: &mut Vec<ManagerAction>) {
        if self.loaded.remove(id).is_some() {
            if let Some(policy) = &self.link_policy {
                policy.remove_plugin(id);
            }
            tracing::info!(target: "plugin", plugin = %id, "plugin unloaded");
            actions.push(ManagerAction::Unregister(id.to_owned()));
        }
    }

    /// The folder `name` is gone while the manager runs: its plugin is unloaded, its review
    /// dropped, and its grant and data deleted (Rulings AG and AR).
    fn removed(&mut self, name: &str, actions: &mut Vec<ManagerAction>) {
        self.unload(name, actions);
        self.reviews.remove(name);
        self.failed.remove(name);
        self.catalog.remove(name);
        match self.db.forget_plugin(name) {
            Ok(_) => {
                tracing::info!(target: "plugin", plugin = %name, "plugin folder removed; its approval and data are deleted");
            }
            Err(e) => {
                tracing::warn!(target: "plugin", plugin = %name, "cannot delete the plugin's approval and data: {e:#}");
            }
        }
    }

    /// Records a failure of folder `name`; logs it unless it is the one logged last.
    fn fail(&mut self, name: &str, content_hash: Option<[u8; 32]>, message: String) {
        let failure = Failure {
            content_hash,
            message,
        };
        if self.failed.get(name) == Some(&failure) {
            tracing::debug!(target: "plugin", plugin = %name, "still failing: {}", failure.message);
            return;
        }
        tracing::warn!(target: "plugin", plugin = %name, "plugin not loaded: {}; it is tried again when its folder changes", failure.message);
        self.stats.failures_logged.fetch_add(1, Ordering::SeqCst);
        self.failed.insert(name.to_owned(), failure);
    }

    /// Records `state` for the plugin of `package`, with what its manifest declares.
    fn set_package_state(&mut self, package: &Package, state: PluginState) {
        let plugin = catalog_entry(&package.manifest, state);
        self.catalog.insert(plugin.id.clone(), plugin);
    }

    /// Changes the state of a plugin already in the catalog.
    fn set_state(&mut self, id: &str, state: PluginState) {
        if let Some(plugin) = self.catalog.get_mut(id) {
            plugin.state = state;
        }
    }

    /// A folder that cannot be read as a plugin: the folder name stands for id and name.
    fn set_unreadable(&mut self, folder: &str, message: &str) {
        self.catalog
            .insert(folder.to_owned(), unreadable_entry(folder, message));
    }

    /// Adds a [`ManagerAction::Catalog`] when the catalog differs from the one sent last, or
    /// none was sent yet.
    fn push_catalog(&mut self, actions: &mut Vec<ManagerAction>) {
        let mut plugins: Vec<CatalogPlugin> = self.catalog.values().cloned().collect();
        sort_catalog(&mut plugins);
        let catalog = PluginCatalog {
            plugins,
            unavailable: self.unlisted.clone(),
        };
        if self.catalog_sent.as_ref() != Some(&catalog) {
            self.catalog_sent = Some(catalog.clone());
            actions.push(ManagerAction::Catalog(catalog));
        }
    }

    /// Adds a [`ManagerAction::Reviews`] when the pending reviews differ from `before`.
    fn push_reviews(
        &self,
        before: BTreeMap<String, ReviewState>,
        actions: &mut Vec<ManagerAction>,
    ) {
        if before != self.reviews {
            actions.push(ManagerAction::Reviews(
                self.reviews.values().cloned().collect(),
            ));
        }
    }
}

/// Whether content that failed before is tried again: yes after a file event in its folder,
/// no for a rescan or a decision in the review prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    Yes,
    No,
}

enum Input {
    Changed(Key),
    Plugin(PluginEvents),
    Stop,
}

/// The running manager thread; dropping it stops the thread (after the work in progress).
pub struct ManagerThread {
    input: mpsc::Sender<Input>,
}

impl Drop for ManagerThread {
    fn drop(&mut self) {
        let _ = self.input.send(Input::Stop);
    }
}

/// The names of the entries of the plugins folder that may be plugin folders: everything
/// not starting with `.`. An entry that is not a readable folder fails to read in `sync`;
/// it is still there, so it is not a removal.
pub(crate) fn plugin_folder_names(plugins_dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(plugins_dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_folder_name(&name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// The catalog entry of a folder that cannot be read as a plugin: the folder name stands for
/// id and name.
pub(crate) fn unreadable_entry(folder: &str, message: &str) -> CatalogPlugin {
    CatalogPlugin {
        id: folder.to_owned(),
        name: folder.to_owned(),
        version: String::new(),
        author: None,
        state: PluginState::ManifestError(message.to_owned()),
        preferences: Vec::new(),
        commands: Vec::new(),
        widgets: Vec::new(),
        param_titles: Vec::new(),
    }
}

/// Sorts the catalog by name, then id.
pub(crate) fn sort_catalog(plugins: &mut [CatalogPlugin]) {
    plugins.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
}

/// A plugin folder name the manager looks at: not empty, not hidden, one path component.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && Path::new(name).components().count() == 1
        && !name.contains(['/', '\\'])
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_core::store::Store;
    use tomakast_core::widget::{WidgetKey, WidgetSize, params};

    #[test]
    fn a_config_read_sends_the_home_list_unless_broken() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::with(home.path().join("config.toml"), home.path().to_owned());
        drop(Store::open(&paths.db_file).unwrap());
        let mut manager = PluginManager::new(
            &paths,
            HostEngine::new().unwrap(),
            PluginDb::open(&paths.db_file).unwrap(),
            Preferences::new(),
        );
        std::fs::write(
            &paths.config_file,
            "[[home.widgets]]\nwidget = \"weather/now\"\nsize = \"quarter\"\ncity = \"Oslo\"\n",
        )
        .unwrap();
        let actions = manager.reload_config();
        let [ManagerAction::Home(entries)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(
            *entries,
            [HomeEntry {
                widget: WidgetKey::new("weather", "now"),
                size: Some(WidgetSize::Quarter),
                params: params([("city".to_string(), "Oslo".to_string())]),
            }]
        );
        // No list: an empty Home.
        std::fs::write(&paths.config_file, "autostart = false\n").unwrap();
        let actions = manager.reload_config();
        assert!(
            matches!(actions.as_slice(), [ManagerAction::Home(e)] if e.is_empty()),
            "{actions:?}"
        );
        // A broken file changes nothing.
        std::fs::write(&paths.config_file, "[[home.widgets]\n").unwrap();
        assert!(manager.reload_config().is_empty());
    }
}
