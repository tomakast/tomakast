//! `WasmPlugin`: one plugin component behind the core's [`Module`] trait. It runs on its
//! module worker thread, in its own `Store`, with every call under an epoch deadline.
//!
//! A call can end in three ways:
//! - the guest answers `Err(message)`: an ordinary error, the plugin lives on;
//! - the answer breaks a cap or a validity rule ([`crate::caps`]): the answer is rejected as
//!   a whole with an error toast, the plugin lives on. A rejected `run` answer is the call's
//!   error (one per user action, and no usage is recorded); every other rejection is a host
//!   denial toast, at most one per plugin per 10 s;
//! - wasmtime fails the call (a trap, an exceeded deadline or limit, a blocking WASI call, a
//!   failed instantiation) or the host panics: the plugin is broken. Its store is dropped (and
//!   with it the plugin's timers), the user sees one error toast, [`PluginEvents::Broken`] asks
//!   for it to be unloaded, and every later call fails with `plugin <id> stopped: <reason>`.
//!
//! An accepted answer may still lose single effects the plugin may not send (a missing
//! permission, a rate exceeded): they are dropped with a host denial toast, the rest is kept.
//!
//! Widgets: `render-widget` runs under [`RENDER_DEADLINE`] (past it the plugin is broken); a
//! frame that breaks a cap or a rule ([`crate::widget_caps`]) is an error for that frame only,
//! logged at most once per [`RENDER_FAILURE_LOG_INTERVAL`] per widget. A widget event is a user
//! action when it is an `action`, a `key` or a completed click (a pointer `up` on the node that
//! got the last `down`); its answer then counts as one to `run`. Any other widget event's
//! answer is unsolicited, and may not copy, push or pop a view or close the launcher.
//!
//! Timer and HTTP events arrive in the plugin's inbox from host threads, which wake the worker;
//! [`Module::on_wake`] delivers them to `on-event`, and their effects are unsolicited. The time
//! their calls take, host imports included, counts against the plugin's background budget
//! ([`crate::budget`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use tomakast_core::widget::{
    Frame, NodeKind, PointerKind, WidgetAnswer, WidgetContext, WidgetDecl, WidgetEvent,
};
use tomakast_core::{
    Action, Effect, Icon, Item, ItemId, ItemKind, Module, ModuleContext, QueryReading,
    SearchAnswer, Toast, ToastKind, ViewEvent, ViewId, check_frame,
};
use wasmtime::component::Component;
use wasmtime::{Engine, Store, Trap};

use crate::bindings::{self, tomakast::plugin::types as wit};
use crate::budget::{BACKGROUND_BUDGET, BUDGET_WINDOW, Budget, SUSPENSION, Verdict};
use crate::caps::{self, CapError, MAX_INDEX_ITEMS, MAX_RETURN_BYTES, Origin, Rates};
use crate::convert;
use crate::db::PluginDb;
use crate::engine::{HostEngine, api_version, check_api_version};
use crate::epoch::Epoch;
use crate::imports::open::Opener;
use crate::imports::preferences::effective;
use crate::imports::{self, HostState, Inbox};
use crate::manifest::{Manifest, Permissions, PreferenceDecl};
use crate::package::Package;
#[cfg(any(test, feature = "testing"))]
use crate::recorder::{effect_entries, event_entries, frame_entries};
use crate::widget_caps::{self, check_image};

/// Notes the entries `$entries` makes in `$plugin`'s test recorder; nothing at all (not even
/// the entries) without the `testing` feature.
macro_rules! record {
    ($plugin:expr, $entries:expr) => {
        #[cfg(any(test, feature = "testing"))]
        $plugin.record(|| $entries);
    };
}

/// How long `init` (and instantiation before it) may run.
pub const INIT_DEADLINE: Duration = Duration::from_millis(2000);
/// How long `search` may run.
pub const SEARCH_DEADLINE: Duration = Duration::from_millis(100);
/// How long `run` and `on-event` may run.
pub const CALL_DEADLINE: Duration = Duration::from_millis(1000);
/// How long `render-widget` may run. The soft deadline is the core's
/// ([`soft_render_deadline`](tomakast_core::soft_render_deadline): 16 ms, 64 ms in a debug build),
/// measured in the worker thread's CPU time around [`Module::render_widget`] as a whole,
/// so the host's conversion and checks of the frame (strings, images, SVG parsing, `plugin-file`
/// icons) count towards it too.
pub const RENDER_DEADLINE: Duration = Duration::from_millis(100);
/// A widget's failed frames are logged at most once in this interval.
pub const RENDER_FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(10);
/// At most one host denial toast per plugin in this interval.
pub const DENIAL_TOAST_INTERVAL: Duration = Duration::from_secs(10);

/// The action id of a manifest command's item.
pub const COMMAND_ACTION: &str = "run";
/// Largest image a `plugin-file` icon may name.
pub const MAX_ICON_FILE_BYTES: u64 = 1024 * 1024;
/// Component-model handles (resources) one plugin store may hold at once.
pub const MAX_COMPONENT_HANDLES: usize = 1000;

/// Longest plugin-controlled text (an error, a rejection reason) that leaves the host in an
/// error, a toast or a log line.
pub const MAX_MESSAGE_BYTES: usize = 1024;

/// Plugin-controlled text bounded and escaped for an error, a toast or a log line.
fn bounded(text: &str) -> String {
    imports::sanitize(text, MAX_MESSAGE_BYTES)
}

/// What a plugin instance, or the `plugins` review module, tells the plugin manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEvents {
    /// The plugin instance running the content `content_hash` is broken and must be
    /// unloaded. The hash tells the manager which instance it was: a replaced one may still
    /// break in the call it was serving.
    Broken {
        id: String,
        content_hash: [u8; 32],
        reason: String,
    },
    /// The user allowed the content `content_hash` of plugin `id`; its grant is written. The
    /// manager gates the plugin again ([`crate::approval::check`]) rather than loading it
    /// outright: the folder may hold other content by now.
    Approved { id: String, content_hash: [u8; 32] },
    /// The user denied the content `content_hash` of plugin `id`; its grant is written. The
    /// manager gates the plugin again too, and drops its review when the gate says `Denied`.
    Denied { id: String, content_hash: [u8; 32] },
}

struct Running {
    store: Store<HostState>,
    bindings: bindings::Plugin,
}

pub struct WasmPlugin {
    id: String,
    name: String,
    /// The `api` of `plugin.toml`, which the component's own version must match.
    api: String,
    /// The content hash of the package this instance runs.
    content_hash: [u8; 32],
    /// The declared preferences, to resolve new user values against.
    pref_decls: Arc<[PreferenceDecl]>,
    /// The canonical plugin folder; `None` when it cannot be resolved (no `plugin-file` icon
    /// is shown then).
    root: Option<PathBuf>,
    permissions: Permissions,
    /// The `[[widgets]]` of the manifest.
    widgets: Vec<WidgetDecl>,
    /// The widget and the node that got the last pointer `down`, until an `up`.
    pressed: Option<(String, String)>,
    /// When each widget's failed frame was last logged.
    render_failures: HashMap<String, Instant>,
    /// When the user approved this plugin's permissions and widgets (`Grant.decided_at`).
    approved_at: Option<i64>,
    component: Component,
    engine: Engine,
    /// The items of the manifest commands, owned by the host (Ruling H).
    commands: Vec<Item>,
    /// The store data until `start` builds the store.
    setup: Option<HostState>,
    running: Option<Running>,
    broken: Option<String>,
    ctx: Option<ModuleContext>,
    events: Option<mpsc::Sender<PluginEvents>>,
    last_denial_toast: Option<Instant>,
    rates: Rates,
    /// Guest time used by timer and HTTP events (Ruling AK).
    budget: Budget,
    /// Timer and HTTP events waiting for [`Module::on_wake`].
    inbox: Arc<Inbox>,
    /// Test-only: where the exports called, the events delivered and the effects returned
    /// are recorded.
    #[cfg(any(test, feature = "testing"))]
    recorder: Option<crate::recorder::Recorder>,
}

impl WasmPlugin {
    /// A plugin that instantiates `component` when its worker starts it. `prefs` are the
    /// user's values from `[plugins.<id>]` in `config.toml`.
    pub fn new(
        package: Package,
        component: Component,
        engine: &HostEngine,
        db: PluginDb,
        prefs: BTreeMap<String, String>,
    ) -> WasmPlugin {
        let Package {
            dir,
            manifest,
            content_hash,
            ..
        } = package;
        let inbox = Arc::new(Inbox::default());
        WasmPlugin {
            content_hash,
            pref_decls: manifest.preferences.clone().into(),
            commands: command_items(&manifest),
            setup: Some(HostState::new(&manifest, db, prefs, &inbox)),
            id: manifest.id,
            name: manifest.name,
            api: manifest.api,
            root: dir.canonicalize().ok(),
            permissions: manifest.permissions,
            widgets: manifest
                .widgets
                .into_iter()
                .map(|w| WidgetDecl {
                    id: w.id,
                    title: w.title,
                    size: w.size,
                })
                .collect(),
            pressed: None,
            render_failures: HashMap::new(),
            approved_at: None,
            component,
            engine: engine.engine().clone(),
            running: None,
            broken: None,
            ctx: None,
            events: None,
            last_denial_toast: None,
            rates: Rates::default(),
            budget: Budget::default(),
            inbox,
            #[cfg(any(test, feature = "testing"))]
            recorder: None,
        }
    }

    /// Test-only: records what crosses the WIT boundary into `recorder` (see
    /// [`Recorder`](crate::recorder::Recorder)).
    /// Has no effect on the imports after `start`.
    #[cfg(any(test, feature = "testing"))]
    pub fn with_recorder(mut self, recorder: crate::recorder::Recorder) -> WasmPlugin {
        if let Some(setup) = self.setup.as_mut() {
            setup.recorder = Some(recorder.clone());
        }
        self.recorder = Some(recorder);
        self
    }

    /// Notes `entry` in the test recorder, when one is attached; nothing otherwise. Called
    /// through [`record!`], which compiles the call out without the `testing` feature.
    #[cfg(any(test, feature = "testing"))]
    fn record(&self, entry: impl FnOnce() -> Vec<String>) {
        if let Some(recorder) = &self.recorder {
            entry().into_iter().for_each(|e| recorder.note(e));
        }
    }

    /// Where [`PluginEvents`] go (the plugin manager).
    pub fn with_events(mut self, events: mpsc::Sender<PluginEvents>) -> WasmPlugin {
        self.events = Some(events);
        self
    }

    /// When the user approved this plugin, in seconds since the epoch (the grant's
    /// `decided_at`); [`Module::approved_at`] returns it.
    pub fn with_approved_at(mut self, at: i64) -> WasmPlugin {
        self.approved_at = Some(at);
        self
    }

    /// A handle that delivers new preference values to this instance (Ruling C): the plugin
    /// manager keeps it after registering the plugin.
    pub fn preferences(&self) -> PreferencesHandle {
        PreferencesHandle {
            inbox: Arc::clone(&self.inbox),
            decls: Arc::clone(&self.pref_decls),
        }
    }

    /// What `open.open-url` and `open.open-path` hand checked targets to; the platform's
    /// default applications unless set. Has no effect after `start`.
    pub fn with_opener(mut self, opener: Arc<dyn Opener>) -> WasmPlugin {
        if let Some(setup) = self.setup.as_mut() {
            setup.set_opener(opener);
        }
        self
    }

    /// Where `locale.local-time-zone` reads the computer's time zone: an IANA name, or `None`
    /// when it cannot be told (the import then answers `UTC`). Called on every import call,
    /// so the answer follows a change. The operating system's setting unless set. Has no
    /// effect after `start`.
    pub fn with_time_zone(
        mut self,
        source: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> WasmPlugin {
        if let Some(setup) = self.setup.as_mut() {
            setup.set_time_zone(source);
        }
        self
    }

    /// Where `location.request` gets the computer's location: blocks until it answers, or says
    /// why it cannot. Its own cache and one-at-a-time rule apply (see
    /// [`Locator`](crate::Locator)). The launcher's own sources unless set. Has no effect after
    /// `start`.
    pub fn with_location(self, source: crate::LocationSource) -> WasmPlugin {
        self.with_locator(crate::Locator::new(source))
    }

    /// Like [`with_location`](Self::with_location), with a locator the caller keeps: a change
    /// of its sources reaches the plugin.
    pub fn with_locator(mut self, locator: std::sync::Arc<crate::Locator>) -> WasmPlugin {
        if let Some(setup) = self.setup.as_mut() {
            setup.set_locator(locator);
        }
        self
    }

    /// Test-only: answers the requests that pass the checks instead of the network. Has no
    /// effect after `start`.
    #[cfg(any(test, feature = "testing"))]
    pub fn with_http_responder(mut self, responder: crate::imports::http::Responder) -> WasmPlugin {
        if let Some(setup) = self.setup.as_mut() {
            setup.set_http_responder(responder);
        }
        self
    }

    fn instantiate(&self, state: HostState) -> wasmtime::Result<Running> {
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        // Bounds what wasmtime lifts out of the guest per call, before the host sees it.
        store.set_hostcall_fuel(MAX_RETURN_BYTES);
        store.set_max_component_handles(MAX_COMPONENT_HANDLES);
        let linker = imports::linker(&self.engine)?;
        let deadline = Epoch::global().enter_store(&mut store, INIT_DEADLINE);
        let bindings = bindings::Plugin::instantiate(&mut store, &self.component, &linker)?;
        drop(deadline);
        Ok(Running { store, bindings })
    }

    /// Whether the component was built for the API version of `plugin.toml` ([`check_api_version`]).
    fn check_api(&self) -> Result<(), String> {
        let built_for = api_version(&self.component, &self.engine)?;
        if built_for.is_none() {
            tracing::debug!(
                target: "plugin",
                plugin = %self.id,
                api = %self.api,
                "the component records no API version; going by plugin.toml"
            );
        }
        check_api_version(built_for.as_deref(), &self.api)
    }

    /// Runs `export` under `deadline`, started `by` the user or the host. A wasmtime error or
    /// a host panic breaks the plugin.
    fn call<R>(
        &mut self,
        export: &'static str,
        deadline: Duration,
        by: StartedBy,
        f: impl FnOnce(&bindings::Plugin, &mut Store<HostState>) -> wasmtime::Result<R>,
    ) -> anyhow::Result<R> {
        if let Some(reason) = &self.broken {
            return Err(stopped(&self.id, reason));
        }
        record!(self, vec![format!("export {export}")]);
        let Some(running) = self.running.as_mut() else {
            return Err(anyhow!("plugin {} is not started", self.id));
        };
        // Rulings X and AL: what only a user action may do (open) checks this flag, and one
        // call may open once.
        running.store.data_mut().start_call(by == StartedBy::User);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _deadline = Epoch::global().enter_store(&mut running.store, deadline);
            f(&running.bindings, &mut running.store)
        }));
        let reason = match outcome {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(err)) => reason(export, deadline, running.store.data_mut(), &err),
            Err(_) => format!("the host panicked during {export}"),
        };
        Err(self.fail(reason))
    }

    /// Marks the plugin broken: drops its store, shows one error toast and asks the manager to
    /// unload it. Returns the error every call answers from now on.
    fn fail(&mut self, reason: String) -> anyhow::Error {
        self.end("stopped", reason)
    }

    /// [`Self::fail`] for a plugin refused before it was instantiated: the log line and the
    /// toast say "not loaded".
    fn refuse(&mut self, reason: String) {
        self.end("not loaded", reason);
    }

    fn end(&mut self, what: &str, reason: String) -> anyhow::Error {
        let reason = bounded(&reason);
        tracing::error!(target: "plugin", plugin = %self.id, "plugin {what}: {reason}");
        self.running = None;
        self.broken = Some(reason.clone());
        if let Some(ctx) = &self.ctx {
            ctx.emit(vec![Effect::Toast(Toast {
                kind: ToastKind::Error,
                title: format!("Plugin {} {what}", self.name),
                message: Some(reason.clone()),
            })]);
        }
        if let Some(events) = &self.events {
            let broken = PluginEvents::Broken {
                id: self.id.clone(),
                content_hash: self.content_hash,
                reason: reason.clone(),
            };
            if events.send(broken).is_err() {
                tracing::debug!(target: "plugin", plugin = %self.id, "no manager to unload the plugin");
            }
        }
        stopped(&self.id, &reason)
    }

    /// An error toast from the host about this plugin, at most one per
    /// [`DENIAL_TOAST_INTERVAL`]; the rest are only logged.
    fn denial_toast(&mut self, message: &str) {
        let message = bounded(message);
        tracing::warn!(target: "plugin", plugin = %self.id, "{message}");
        let quiet = self
            .last_denial_toast
            .is_none_or(|at| at.elapsed() >= DENIAL_TOAST_INTERVAL);
        if !quiet {
            return;
        }
        let Some(ctx) = &self.ctx else {
            return;
        };
        self.last_denial_toast = Some(Instant::now());
        ctx.emit(vec![Effect::Toast(Toast {
            kind: ToastKind::Error,
            title: format!("Plugin {}", self.name),
            message: Some(message),
        })]);
    }

    /// The checked, host-completed form of `effects`, or why they are rejected. Effects the
    /// plugin may not send are dropped with a denial toast.
    fn accept(
        &mut self,
        origin: Origin,
        effects: Vec<wit::Effect>,
    ) -> Result<Vec<Effect>, CapError> {
        record!(self, effects.iter().flat_map(effect_entries).collect());
        // Counted as returned, before any item is dropped.
        for effect in &effects {
            if let wit::Effect::ReplaceItems(items) = effect {
                caps::count("replace-items", items.len(), MAX_INDEX_ITEMS)?;
                let bare = items.iter().filter(|i| ItemId::parse(&i.id).is_none());
                self.log_bare_ids(bare.count());
            }
        }
        let effects = effects.into_iter().map(Effect::from).collect();
        let checked = caps::check_effects(
            &self.id,
            &self.permissions,
            &self.widgets,
            &mut self.rates,
            origin,
            Instant::now(),
            effects,
        )?;
        if !checked.denied.is_empty() {
            self.denial_toast(&format!("dropped: {}", checked.denied.join("; ")));
        }
        let mut effects = checked.effects;
        let mut icons = IconDrops::default();
        for effect in &mut effects {
            // Ruling AN: the user sees which plugin a toast comes from. The host's own toasts
            // about the plugin do not pass here.
            if let Effect::Toast(toast) = effect {
                toast.title = format!("{}: {}", self.name, toast.title);
            }
            caps::for_each_icon(effect, &mut |icon| self.resolve_icon(icon, &mut icons));
            if let Effect::ReplaceItems(items) = effect {
                *items = self.with_commands(std::mem::take(items));
            }
        }
        icons.log(&self.id);
        Ok(effects)
    }

    fn log_bare_ids(&self, dropped: usize) {
        if dropped > 0 {
            tracing::warn!(target: "plugin", plugin = %self.id, "dropped {dropped} items whose id has no provider");
        }
    }

    /// Ruling H: the manifest commands stay indexed whatever the plugin replaces; a plugin
    /// item cannot take a command's id.
    fn with_commands(&self, items: Vec<Item>) -> Vec<Item> {
        let taken: HashSet<&ItemId> = self.commands.iter().map(|c| &c.id).collect();
        let mut all = self.commands.clone();
        let mut dropped = 0;
        for item in items {
            if taken.contains(&item.id) {
                dropped += 1;
                continue;
            }
            all.push(self.own(item));
        }
        if dropped > 0 {
            tracing::warn!(target: "plugin", plugin = %self.id, "dropped {dropped} items that reuse a command's id");
        }
        all
    }

    /// Sets what the plugin cannot set on its item: the kind and the source.
    fn own(&self, mut item: Item) -> Item {
        item.kind = ItemKind::Command;
        item.source = Some(self.name.clone());
        item
    }

    /// Ruling U: a checked relative `plugin-file` path becomes the canonical path of a regular
    /// file of at most 1 MiB inside the plugin folder, reached without a symbolic link;
    /// otherwise the icon is dropped (`None`), and the effect is kept.
    fn resolve_icon(&self, icon: &mut Option<Icon>, drops: &mut IconDrops) {
        let Some(Icon::PluginFile(rel)) = icon else {
            return;
        };
        let resolved = match &self.root {
            Some(root) => resolve_plugin_file(root, rel),
            None => Err("the plugin folder cannot be resolved".into()),
        };
        match resolved {
            Ok(path) => *rel = path,
            Err(problem) => {
                drops.add(rel, problem);
                *icon = None;
            }
        }
    }

    /// The item as the plugin returned it: `PluginFile` icons relative to the plugin folder
    /// again.
    fn to_wit_item(&self, item: &Item) -> wit::Item {
        let mut item = item.clone();
        if let Some(Icon::PluginFile(path)) = &item.icon {
            item.icon = self
                .root
                .as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .map(|rel| Icon::PluginFile(rel.to_owned()));
        }
        wit::Item::from(item)
    }

    /// The answer of a `run` or `on-event` call. A rejected `run` answer becomes the error the
    /// launcher shows as a toast: one per user action, and the failed action records no usage.
    /// A rejected `on-event` answer (events can come with every keystroke) gets a denial
    /// toast, at most one per 10 s, and no effects.
    fn reply(
        &mut self,
        export: &'static str,
        origin: Origin,
        answer: Result<Vec<wit::Effect>, String>,
    ) -> anyhow::Result<Vec<Effect>> {
        let effects = answer.map_err(|message| {
            anyhow!(
                "plugin {}: {export} failed: {}",
                self.name,
                bounded(&message)
            )
        })?;
        match self.accept(origin, effects) {
            Ok(effects) => Ok(effects),
            Err(cap) if export == "run" => {
                let cap = bounded(&cap.to_string());
                tracing::warn!(target: "plugin", plugin = %self.id, "{export}: rejected answer: {cap}");
                Err(anyhow!(
                    "plugin {} sent an invalid answer: {cap}",
                    self.name
                ))
            }
            Err(cap) => {
                self.denial_toast(&format!("{export} sent an invalid answer: {cap}"));
                Ok(Vec::new())
            }
        }
    }
}

/// The items of the manifest commands: `<id>/<command>`, kind `Command`, the plugin's name
/// as source, one action `run`.
fn command_items(manifest: &Manifest) -> Vec<Item> {
    manifest
        .commands
        .iter()
        .map(|command| Item {
            id: ItemId::new(&manifest.id, &command.id),
            title: command.title.clone(),
            subtitle: None,
            keywords: Vec::new(),
            icon: command.icon.clone().map(Icon::Named),
            kind: ItemKind::Command,
            actions: vec![Action {
                id: COMMAND_ACTION.into(),
                title: "Run".into(),
                shortcut: None,
            }],
            source: Some(manifest.name.clone()),
        })
        .collect()
}

/// Ruling U, for `rel` already checked by [`caps`] (relative, normal components only): the
/// file under `root` (canonical), reached through no symbolic link, a regular file of at most
/// [`MAX_ICON_FILE_BYTES`]. Only metadata is read, so a FIFO or a device cannot block.
fn resolve_plugin_file(root: &Path, rel: &Path) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for part in rel.components() {
        let PathComponent::Normal(part) = part else {
            continue;
        };
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("the path goes through a symbolic link".into());
        }
    }
    // With no link on the way the canonical path is the joined one; checked anyway.
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if canonical != path || !canonical.starts_with(root) {
        return Err("the path leaves the plugin folder".into());
    }
    let meta = std::fs::symlink_metadata(&canonical).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_ICON_FILE_BYTES {
        return Err(format!("larger than {MAX_ICON_FILE_BYTES} bytes"));
    }
    Ok(canonical)
}

/// `plugin-file` icons dropped during one call, logged as one line.
#[derive(Default)]
struct IconDrops {
    count: usize,
    first: Option<String>,
}

impl IconDrops {
    fn add(&mut self, rel: &Path, problem: String) {
        self.count += 1;
        if self.first.is_none() {
            self.first = Some(format!("{}: {problem}", rel.display()));
        }
    }

    fn log(self, plugin: &str) {
        if let Some(first) = self.first {
            let first = bounded(&first);
            let count = self.count;
            tracing::warn!(target: "plugin", plugin, "dropped {count} plugin-file icons (first: {first})");
        }
    }
}

fn stopped(id: &str, reason: &str) -> anyhow::Error {
    anyhow!("plugin {id} stopped: {reason}")
}

/// Why wasmtime failed `export`: a host fault or limit first (they stop the guest with a
/// plain error), then the trap.
fn reason(
    export: &str,
    deadline: Duration,
    state: &mut HostState,
    err: &wasmtime::Error,
) -> String {
    if let Some(limit) = state.limits.hit.take() {
        return format!("{export} {limit}");
    }
    if let Some(fault) = state.fault.take() {
        return format!("{export} {fault}");
    }
    // wasmtime's error type for exhausted hostcall fuel is private; its message is matched
    // only to word the reason, the plugin is broken either way.
    if format!("{err:#}").contains("fuel allocated for hostcalls") {
        return format!("{export} passed more than the 4 MiB one call may pass to the host");
    }
    match err.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => {
            format!("{export} did not answer within {} ms", deadline.as_millis())
        }
        Some(trap) => format!("{export} trapped: {trap}"),
        None => format!("{export} failed: {err}"),
    }
}

impl Module for WasmPlugin {
    fn id(&self) -> &str {
        &self.id
    }

    fn approved_at(&self) -> Option<i64> {
        self.approved_at
    }

    /// The manifest's `[[widgets]]`; the guest is not called.
    fn widgets(&self) -> Vec<WidgetDecl> {
        self.widgets.clone()
    }

    /// Calls `render-widget` under [`RENDER_DEADLINE`] and checks the frame: its structure and
    /// numbers ([`check_frame`]), its strings, its images (which then count towards the image
    /// budget) and its `plugin-file` icons, resolved as an item's are. A frame that fails is
    /// this call's error; the plugin lives on. The core times this whole call against the
    /// 16 ms soft deadline, so these checks count towards it.
    fn render_widget(&mut self, widget: &str, ctx: &WidgetContext) -> anyhow::Result<Frame> {
        let context = wit::WidgetContext::from(ctx.clone());
        let answer = self.call("render-widget", RENDER_DEADLINE, StartedBy::Host, |p, s| {
            p.call_render_widget(s, widget, &context)
        })?;
        let checked = match answer {
            Ok(frame) => {
                record!(self, frame_entries(&frame));
                self.checked_frame(Frame::from(frame))
                    .map_err(|cap| format!("render-widget sent an invalid frame: {cap}"))
            }
            Err(message) => Err(format!("render-widget failed: {}", bounded(&message))),
        };
        checked.map_err(|reason| {
            let reason = bounded(&reason);
            self.log_render_failure(widget, &reason);
            anyhow!(reason)
        })
    }

    /// `action`, `key` and a completed click are user actions, answered as `run` is; every
    /// other widget event is the host's, and its answer is unsolicited.
    fn on_widget_event(
        &mut self,
        widget: &str,
        payload: Option<&str>,
        event: WidgetEvent,
    ) -> anyhow::Result<Vec<Effect>> {
        let (by, origin) = if self.is_user_action(widget, &event) {
            (StartedBy::User, Origin::Answer)
        } else {
            (StartedBy::Host, Origin::WidgetEvent)
        };
        let event = wit::Event::Widget(wit::WidgetTarget {
            widget_id: widget.to_string(),
            payload: payload.map(str::to_string),
            event: event.into(),
        });
        record!(self, event_entries(&event));
        let answer = self.call("on-event", CALL_DEADLINE, by, |p, s| {
            p.call_on_event(s, &event)
        })?;
        self.reply("on-event", origin, answer)
    }

    /// Instantiates the component, indexes the manifest commands, then calls `init`.
    fn start(&mut self, ctx: ModuleContext) {
        self.ctx = Some(ctx.clone());
        // Before instantiation: `init` may already start a timer or a request.
        self.inbox.connect(ctx.clone());
        let Some(state) = self.setup.take() else {
            tracing::error!(target: "plugin", plugin = %self.id, "started twice");
            return;
        };
        if let Err(reason) = self.check_api() {
            self.refuse(reason);
            return;
        }
        match catch_unwind(AssertUnwindSafe(|| self.instantiate(state))) {
            Ok(Ok(running)) => self.running = Some(running),
            Ok(Err(err)) => {
                let mut fault = format!("failed: {err:#}");
                if let Some(trap) = err.downcast_ref::<Trap>() {
                    fault = match trap {
                        Trap::Interrupt => {
                            format!("did not finish within {} ms", INIT_DEADLINE.as_millis())
                        }
                        trap => format!("trapped: {trap}"),
                    };
                }
                self.fail(format!("instantiation {fault}"));
                return;
            }
            Err(_) => {
                self.fail("the host panicked during instantiation".into());
                return;
            }
        }
        ctx.replace_items(self.commands.clone());
        let answer = match self.call("init", INIT_DEADLINE, StartedBy::Host, |p, s| {
            p.call_init(s)
        }) {
            Ok(answer) => answer,
            Err(_) => return,
        };
        match answer {
            Ok(effects) => match self.accept(Origin::Unsolicited, effects) {
                Ok(effects) => ctx.emit(effects),
                Err(cap) => self.denial_toast(&format!("init sent an invalid answer: {cap}")),
            },
            Err(message) => self.denial_toast(&format!("init failed: {message}")),
        }
    }

    /// Only with the `search` permission does the plugin see what is typed (Ruling AJ).
    fn is_dynamic_source(&self) -> bool {
        self.permissions.search
    }

    fn search(&mut self, query: &str) -> anyhow::Result<SearchAnswer> {
        self.search_with(query, &[])
    }

    fn search_with(
        &mut self,
        query: &str,
        readings: &[QueryReading],
    ) -> anyhow::Result<SearchAnswer> {
        let readings = convert::query_readings(readings);
        let answer = self.call("search", SEARCH_DEADLINE, StartedBy::Host, |p, s| {
            p.call_search(s, query, &readings)
        })?;
        let answer = answer.map_err(|message| anyhow!("search failed: {}", bounded(&message)))?;
        let wit::SearchAnswer { items, widgets } = answer;
        let answered = items.len();
        let (items, bare) = convert::items(items);
        self.log_bare_ids(bare);
        let widgets: Vec<WidgetAnswer> = widgets.into_iter().map(WidgetAnswer::from).collect();
        // A bad widget answer rejects the items too.
        let checked = widget_caps::check_answers(&self.widgets, &widgets)
            .and_then(|()| caps::check_search(&self.id, answered, items));
        match checked {
            Ok(items) => {
                let mut icons = IconDrops::default();
                let items: Vec<Item> = items
                    .into_iter()
                    .map(|mut item| {
                        self.resolve_icon(&mut item.icon, &mut icons);
                        self.own(item)
                    })
                    .collect();
                icons.log(&self.id);
                Ok(SearchAnswer { items, widgets })
            }
            Err(cap) => {
                let message = bounded(&format!("search sent an invalid answer: {cap}"));
                self.denial_toast(&message);
                Err(anyhow!(message))
            }
        }
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        let item = self.to_wit_item(item);
        let answer = self.call("run", CALL_DEADLINE, StartedBy::User, |p, s| {
            p.call_run(s, &item, action_id)
        })?;
        self.reply("run", Origin::Answer, answer)
    }

    /// Ruling AL: only a chosen action and a submitted form are user actions; `closed` (also
    /// sent when the window hides), `query-changed` and `text-changed` are not.
    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        let (by, origin) = match event {
            ViewEvent::ActionChosen { .. } | ViewEvent::FormSubmitted { .. } => {
                (StartedBy::User, Origin::Answer)
            }
            ViewEvent::QueryChanged { .. } | ViewEvent::TextChanged { .. } | ViewEvent::Closed => {
                (StartedBy::Host, Origin::Reply)
            }
        };
        let event = convert::view_event(view, event);
        record!(self, event_entries(&event));
        let answer = self.call("on-event", CALL_DEADLINE, by, |p, s| {
            p.call_on_event(s, &event)
        })?;
        self.reply("on-event", origin, answer)
    }

    /// Delivers the timer and HTTP events in the inbox to `on-event`, in order; the effects of
    /// each answer are sent on their own, unsolicited. Only the events queued when the wake-up
    /// is served are delivered: later ones come with the next wake-up, so other calls are
    /// served in between.
    fn on_wake(&mut self, _token: u64) -> anyhow::Result<Vec<Effect>> {
        let events = self.inbox.take();
        if let Some(reason) = &self.broken {
            return Err(stopped(&self.id, reason));
        }
        for event in events {
            if matches!(event, wit::Event::PreferencesChanged)
                && let (Some(prefs), Some(running)) =
                    (self.inbox.take_preferences(), self.running.as_mut())
            {
                running.store.data_mut().set_prefs(prefs);
            }
            let deliver = self
                .running
                .as_mut()
                .is_some_and(|running| running.store.data_mut().delivering(&event));
            if !deliver {
                continue;
            }
            record!(self, event_entries(&event));
            let background = matches!(
                event,
                wit::Event::TimerFired(_) | wit::Event::HttpDone(_) | wit::Event::Location(_)
            );
            let started = Instant::now();
            let answer = self.call("on-event", CALL_DEADLINE, StartedBy::Host, |p, s| {
                p.call_on_event(s, &event)
            })?;
            if background {
                self.charge(started.elapsed())?;
            }
            match answer {
                Ok(effects) => match self.accept(Origin::Unsolicited, effects) {
                    Ok(effects) => {
                        if let Some(ctx) = &self.ctx {
                            ctx.emit(effects);
                        }
                    }
                    Err(cap) => {
                        self.denial_toast(&format!("on-event sent an invalid answer: {cap}"));
                    }
                },
                Err(message) => self.denial_toast(&format!("on-event failed: {message}")),
            }
        }
        Ok(Vec::new())
    }
}

impl WasmPlugin {
    /// `frame` as the core may take it: checked, its images counted towards the budget (all
    /// or none), its `plugin-file` icons resolved. An icon that does not resolve refuses the
    /// frame.
    fn checked_frame(&mut self, mut frame: Frame) -> Result<Frame, CapError> {
        check_frame(&frame).map_err(|e| CapError(e.to_string()))?;
        widget_caps::check_frame_text(&frame)?;
        frame.images.iter().try_for_each(check_image)?;
        self.resolve_frame_icons(&mut frame)?;
        // Last: nothing after it may refuse the frame, so the budget counts what the core
        // stores.
        self.rates.images.admit_all(&frame.images)?;
        Ok(frame)
    }

    /// Ruling U for the icons of icon and button nodes.
    fn resolve_frame_icons(&self, frame: &mut Frame) -> Result<(), CapError> {
        for node in &mut frame.nodes {
            let icon = match &mut node.kind {
                NodeKind::Icon(node) => Some(&mut node.icon),
                NodeKind::Button(button) => button.icon.as_mut(),
                _ => None,
            };
            let Some(Icon::PluginFile(rel)) = icon else {
                continue;
            };
            let resolved = match &self.root {
                Some(root) => resolve_plugin_file(root, rel),
                None => Err("the plugin folder cannot be resolved".into()),
            };
            match resolved {
                Ok(path) => *rel = path,
                Err(problem) => {
                    return Err(CapError(format!(
                        "icon file {:?}: {problem}",
                        rel.display()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Logs why a frame of `widget` failed, at most once per [`RENDER_FAILURE_LOG_INTERVAL`].
    fn log_render_failure(&mut self, widget: &str, reason: &str) {
        let now = Instant::now();
        let quiet = self
            .render_failures
            .get(widget)
            .is_none_or(|at| now.duration_since(*at) >= RENDER_FAILURE_LOG_INTERVAL);
        if quiet {
            self.render_failures.insert(widget.to_owned(), now);
            tracing::warn!(target: "plugin", plugin = %self.id, widget, "{reason}");
        }
    }

    /// Whether `event` of `widget` is a user action: an `action`, a `key`, or a pointer `up`
    /// on the node of the same widget that got the last `down`. A `down` is remembered; any
    /// `up` ends the press.
    fn is_user_action(&mut self, widget: &str, event: &WidgetEvent) -> bool {
        match event {
            WidgetEvent::Action { .. } | WidgetEvent::Key(_) => true,
            WidgetEvent::Pointer(pointer) => match pointer.kind {
                PointerKind::Down => {
                    self.pressed = Some((widget.to_owned(), pointer.node.clone()));
                    false
                }
                PointerKind::Up => self
                    .pressed
                    .take()
                    .is_some_and(|(w, node)| w == widget && node == pointer.node),
                PointerKind::Move | PointerKind::Enter | PointerKind::Leave => false,
            },
            WidgetEvent::Shown | WidgetEvent::Hidden => false,
        }
    }

    /// Charges `took`, the time of a timer or HTTP event's call, to the background budget and
    /// acts on the verdict: over budget the plugin's timers are suspended (logged); the third
    /// time within 10 minutes the plugin is stopped, as broken.
    fn charge(&mut self, took: Duration) -> anyhow::Result<()> {
        let now = Instant::now();
        match self.budget.charge(now, took) {
            Verdict::Ok => Ok(()),
            Verdict::Suspend => {
                tracing::warn!(
                    target: "plugin",
                    plugin = %self.id,
                    "background work used more than {} ms in {} s; timers suspended for {} s",
                    BACKGROUND_BUDGET.as_millis(),
                    BUDGET_WINDOW.as_secs(),
                    SUSPENSION.as_secs()
                );
                if let Some(running) = self.running.as_mut() {
                    running.store.data_mut().suspend_timers(now + SUSPENSION);
                }
                Ok(())
            }
            Verdict::Stop => Err(self.fail(format!(
                "used more than {} ms of background time per {} s three times in 10 minutes",
                BACKGROUND_BUDGET.as_millis(),
                BUDGET_WINDOW.as_secs()
            ))),
        }
    }
}

/// Delivers the user's new `[plugins.<id>]` values to a running plugin: its `preferences`
/// import answers with them from the next `preferences-changed` event on, which the plugin
/// receives in `on-event`. Cheap to clone, `Send`.
#[derive(Clone)]
pub struct PreferencesHandle {
    inbox: Arc<Inbox>,
    decls: Arc<[PreferenceDecl]>,
}

impl PreferencesHandle {
    /// `user` are the plugin's values from `config.toml`; only declared keys are kept, and
    /// manifest defaults fill the rest.
    pub fn set(&self, user: BTreeMap<String, String>) {
        self.inbox.set_preferences(effective(&self.decls, user));
    }
}

/// Who started a guest call: the user (`run`, `action-chosen`, `form-submitted`, a widget's
/// `action`, `key` or completed click) or the host (`init`, `search`, `render-widget`, timer,
/// HTTP and preference events, and the view and widget events that are not user actions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartedBy {
    User,
    Host,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin folder (canonical) with `icons/ok.png` and a file outside it.
    fn folder() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("plugin");
        std::fs::create_dir_all(root.join("icons")).unwrap();
        std::fs::write(root.join("icons/ok.png"), b"png").unwrap();
        std::fs::write(tmp.path().join("outside.png"), b"png").unwrap();
        (tmp, root)
    }

    #[test]
    fn plugin_file_resolves_to_a_regular_file_in_the_folder() {
        let (_tmp, root) = folder();
        let resolved = resolve_plugin_file(&root, Path::new("icons/ok.png")).unwrap();
        assert_eq!(resolved, root.join("icons/ok.png"));
        assert_eq!(
            resolve_plugin_file(&root, Path::new("./icons/./ok.png")).unwrap(),
            resolved
        );
        // At the size limit.
        std::fs::write(
            root.join("max.png"),
            vec![0u8; MAX_ICON_FILE_BYTES as usize],
        )
        .unwrap();
        assert!(resolve_plugin_file(&root, Path::new("max.png")).is_ok());
    }

    #[test]
    fn plugin_file_refuses_what_is_not_a_small_regular_file_in_the_folder() {
        let (tmp, root) = folder();
        let refused = |rel: &str| resolve_plugin_file(&root, Path::new(rel)).unwrap_err();
        std::fs::write(root.join("big.png"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        assert!(refused("big.png").contains("larger than"));
        refused("missing.png");
        assert!(refused("icons").contains("not a regular file"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("/dev/zero", root.join("zero.png")).unwrap();
            assert!(refused("zero.png").contains("symbolic link"));
            symlink(tmp.path().join("outside.png"), root.join("out.png")).unwrap();
            assert!(refused("out.png").contains("symbolic link"));
            // Even a link that stays inside the folder, to a file or as a directory.
            symlink(root.join("icons/ok.png"), root.join("inside.png")).unwrap();
            assert!(refused("inside.png").contains("symbolic link"));
            symlink(root.join("icons"), root.join("linked")).unwrap();
            assert!(refused("linked/ok.png").contains("symbolic link"));
            let fifo = std::ffi::CString::new(root.join("fifo.png").to_str().unwrap()).unwrap();
            // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert!(refused("fifo.png").contains("not a regular file"));
        }
    }
}
