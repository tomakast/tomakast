//! The plugin harness shared by the `WasmPlugin` tests: a guest plugin started in a core,
//! driven as the daemon drives it. Most tests run the kitchen-sink plugin ([`harness`] and
//! its variants); [`plugin_harness`] runs any plugin of `plugins/`, over seeded storage.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use super::{compile_exe, guest, sha256};
use tomakast_core::store::Store;
use tomakast_core::strip::WidgetEnv;
use tomakast_core::testing::wait_event;
use tomakast_core::widget::{
    Frame, SearchAnswer, Slot, TextSizes, ThemeColors, TileKey, WidgetEvent, WidgetKey, WidgetSize,
    WidgetTheme, params,
};
use tomakast_core::{
    Core, CoreEvent, CoreUpdate, Effect, HomeEntry, Item, ItemId, LocalNow, Module, RankWeights,
    Toast, ToastKind, View, ViewEvent, ViewId,
};
use tomakast_plugin_host::{
    Cache, HostEngine, Package, PluginDb, PluginEvents, Responder, WasmPlugin,
};
use wasmtime::component::Component;

pub const ID: &str = "kitchen-sink";
pub const NAME: &str = "Kitchen Sink";

pub fn now() -> LocalNow {
    LocalNow::from_unix_with_offset(1_800_000_000, 0)
}

/// The engine, the kitchen-sink component and its plugin folder ([`loaded_plugin`]).
pub fn loaded() -> &'static (HostEngine, Component, PathBuf) {
    loaded_plugin(ID)
}

/// The engine, the component of `plugins/<id>` and its plugin folder, built and compiled
/// once per process and plugin, so that the timing tests do not race a compile child. The
/// map's lock is held only to find the plugin's slot, not across the build: other plugins
/// load meanwhile, and a failed build (a panic) leaves the slot empty instead of poisoning
/// the map.
pub fn loaded_plugin(id: &str) -> &'static (HostEngine, Component, PathBuf) {
    type Slot = OnceLock<(HostEngine, Component, PathBuf)>;
    static SLOTS: OnceLock<Mutex<HashMap<String, &'static Slot>>> = OnceLock::new();
    let slot: &'static Slot = SLOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(id.to_owned())
        .or_insert_with(|| Box::leak(Box::default()));
    slot.get_or_init(|| {
        let wasm_path = guest(id);
        let wasm = std::fs::read(&wasm_path).unwrap();
        let host = HostEngine::new().unwrap();
        let cache_dir = wasm_path.parent().unwrap().parent().unwrap().join("cache");
        let component = host
            .load(sha256(&wasm), &wasm, &Cache::new(cache_dir), &compile_exe())
            .unwrap();
        (host, component, wasm_path.parent().unwrap().to_owned())
    })
}

pub struct Harness {
    /// The plugin's id.
    pub id: &'static str,
    pub core: Core,
    pub rx: async_channel::Receiver<CoreEvent>,
    pub plugin_events: mpsc::Receiver<PluginEvents>,
    /// Effects the plugin sent unsolicited ([`CoreUpdate::ModuleEffects`]), in order.
    pub unsolicited: Vec<Effect>,
    /// The database the core and the plugin's storage use.
    pub db_path: PathBuf,
    /// The plugin's preference values; a reload starts with them again.
    prefs: BTreeMap<String, String>,
    /// Installed again by a reload, when the setup had one.
    responder: Option<Responder>,
    _data: tempfile::TempDir,
}

pub fn harness() -> Harness {
    harness_with(|_| {}, BTreeMap::new())
}

/// Starts the plugin in a core, with `edit` applied to its package, and waits until `init`'s
/// items are indexed.
pub fn harness_with(edit: impl FnOnce(&mut Package), prefs: BTreeMap<String, String>) -> Harness {
    harness_built(edit, prefs, |plugin| plugin)
}

/// [`harness_with`], with `build` applied to the plugin before it is registered. Waits for
/// `init`'s answer: its items when the plugin holds the `index` permission, else the denial
/// toast for its `replace-items` (kept in `unsolicited`).
pub fn harness_built(
    edit: impl FnOnce(&mut Package),
    prefs: BTreeMap<String, String>,
    build: impl FnOnce(WasmPlugin) -> WasmPlugin,
) -> Harness {
    harness_beside(edit, prefs, build, Vec::new())
}

/// [`harness_built`], with `modules` registered in the same core before the plugin.
pub fn harness_beside(
    edit: impl FnOnce(&mut Package),
    prefs: BTreeMap<String, String>,
    build: impl FnOnce(WasmPlugin) -> WasmPlugin,
    modules: Vec<Box<dyn Module>>,
) -> Harness {
    let (_, component, _) = loaded();
    let mut index = false;
    let edit = |package: &mut Package| {
        edit(package);
        index = package.manifest.permissions.index;
    };
    let mut h = spawn(ID, component, edit, prefs, build, modules, &[]);
    // Commands first (ReplaceItems), then init's effects (ModuleEffects).
    loop {
        if let CoreUpdate::ModuleEffects { items_changed, .. } = h.update() {
            assert_eq!(
                items_changed, index,
                "init replaces the plugin's items only with the index permission"
            );
            break;
        }
    }
    h
}

/// The kitchen-sink package, with `edit` applied, running `component` in a started core.
/// Does not wait for anything: for a plugin that may be refused before `init`.
pub fn started(component: &Component, edit: impl FnOnce(&mut Package)) -> Harness {
    spawn(
        ID,
        component,
        edit,
        BTreeMap::new(),
        |plugin| plugin,
        Vec::new(),
        &[],
    )
}

/// What a plugin of `plugins/` starts with: `seed` in its storage and `prefs` as the user's
/// preference values.
pub struct PluginSetup {
    pub seed: Vec<(String, Vec<u8>)>,
    pub prefs: BTreeMap<String, String>,
    /// Answers the plugin's checked HTTP requests; the default reaches no network.
    pub responder: Responder,
}

impl Default for PluginSetup {
    fn default() -> PluginSetup {
        PluginSetup {
            seed: Vec::new(),
            prefs: BTreeMap::new(),
            responder: offline(),
        }
    }
}

/// A responder that refuses every request: tests never reach the network.
pub fn offline() -> Responder {
    Arc::new(|_, _| Err("tests never reach the network".into()))
}

/// `plugins/<id>`, as its folder has it, started in a core whose plugin storage holds `seed`
/// first. Waits until its commands are indexed (not for `init`'s answer, which may be empty).
pub fn plugin_harness(id: &'static str, seed: &[(&str, &[u8])]) -> Harness {
    plugin_harness_built(id, seed, |plugin| plugin)
}

/// [`plugin_harness`], with `build` applied to the plugin before it is registered.
pub fn plugin_harness_built(
    id: &'static str,
    seed: &[(&str, &[u8])],
    build: impl FnOnce(WasmPlugin) -> WasmPlugin,
) -> Harness {
    let seed = seed
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_vec()))
        .collect();
    plugin_harness_with(
        id,
        PluginSetup {
            seed,
            ..PluginSetup::default()
        },
        build,
    )
}

/// [`plugin_harness_built`], over a whole [`PluginSetup`].
pub fn plugin_harness_with(
    id: &'static str,
    setup: PluginSetup,
    build: impl FnOnce(WasmPlugin) -> WasmPlugin,
) -> Harness {
    let (_, component, _) = loaded_plugin(id);
    let seed: Vec<(&str, &[u8])> = setup
        .seed
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let responder = setup.responder;
    let installed = Arc::clone(&responder);
    let mut h = spawn(
        id,
        component,
        |_| {},
        setup.prefs,
        |plugin| build(plugin.with_http_responder(installed)),
        Vec::new(),
        &seed,
    );
    h.responder = Some(responder);
    h.wait_indexed();
    h
}

/// The tiles every harness core draws: a quarter tile 173.5x120, a single one 355x120, a
/// double one 718x120, scale 2
/// and a dark theme.
pub fn widget_env() -> WidgetEnv {
    WidgetEnv {
        quarter: (173.5, 120.),
        single: (355., 120.),
        double: (718., 120.),
        scale: 2.,
        theme: WidgetTheme {
            dark: true,
            colors: ThemeColors {
                text: 0xffffffff,
                muted: 0x999999ff,
                panel: 0x202020ff,
                border: 0x404040ff,
                selected: 0x303060ff,
                error: 0xff4040ff,
                success: 0x40ff40ff,
            },
            ui_font: "Inter".into(),
            mono_font: "Mono".into(),
            sizes: TextSizes {
                title: 16.,
                value: 28.,
                body: 14.,
                caption: 12.,
            },
        },
    }
}

/// The event a module instance sent, out of its [`CoreEvent::Instance`] envelope.
fn unwrapped(ev: &CoreEvent) -> &CoreEvent {
    match ev {
        CoreEvent::Instance { event, .. } => unwrapped(event),
        other => other,
    }
}

/// A location source that refuses, so no test reaches the operating system's location
/// service (which may show a dialog on the desktop). Every harness plugin starts with it (a
/// reload too); a test that wants a location sets its own source through `build`.
pub fn no_location() -> tomakast_plugin_host::LocationSource {
    Arc::new(|| Err("tests never ask the location service".into()))
}

/// A time zone source that cannot tell, so `locale.local-time-zone` answers `UTC` whatever
/// the machine's zone is. Every harness plugin starts with it (a reload too); a test that
/// wants a zone sets its own source through `build`.
/// A languages source that cannot tell: tests never read the computer's languages.
pub fn no_languages() -> Arc<dyn Fn() -> Vec<String> + Send + Sync> {
    Arc::new(Vec::new)
}

pub fn no_zone() -> Arc<dyn Fn() -> Option<String> + Send + Sync> {
    Arc::new(|| None)
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    id: &'static str,
    component: &Component,
    edit: impl FnOnce(&mut Package),
    prefs: BTreeMap<String, String>,
    build: impl FnOnce(WasmPlugin) -> WasmPlugin,
    modules: Vec<Box<dyn Module>>,
    seed: &[(&str, &[u8])],
) -> Harness {
    let (host, _, dir) = loaded_plugin(id);
    let mut package = Package::read(dir).unwrap();
    edit(&mut package);
    let data = tempfile::tempdir().unwrap();
    let db_path = data.path().join("tomakast.db");
    let store = Store::open(&db_path).unwrap();
    let db = PluginDb::open(&db_path).unwrap();
    for (key, value) in seed {
        db.kv_set(id, key, value, now().unix).unwrap();
    }
    let (tx, plugin_events) = mpsc::channel();
    let plugin = WasmPlugin::new(package, component.clone(), host, db, prefs.clone())
        .with_events(tx)
        .with_time_zone(no_zone())
        .with_languages(no_languages())
        .with_location(no_location());
    let plugin = build(plugin);
    let mut core = Core::new(store, RankWeights::default(), now()).unwrap();
    core.set_widget_env(widget_env());
    let rx = core.events();
    for module in modules {
        core.register(module);
    }
    core.register(Box::new(plugin));
    core.start_modules();
    Harness {
        id,
        core,
        rx,
        plugin_events,
        unsolicited: Vec::new(),
        db_path,
        prefs,
        responder: None,
        _data: data,
    }
}

impl Harness {
    /// Waits until the plugin's commands are indexed.
    pub fn wait_indexed(&mut self) {
        while !matches!(self.update(), CoreUpdate::ItemsChanged) {}
    }

    /// Unloads the plugin and starts a fresh instance of the same package over the same
    /// database, as a reload does; waits until its commands are indexed.
    pub fn reload(&mut self) {
        self.core.unregister(self.id);
        let (host, component, dir) = loaded_plugin(self.id);
        let package = Package::read(dir).unwrap();
        let db = PluginDb::open(&self.db_path).unwrap();
        let mut plugin = WasmPlugin::new(package, component.clone(), host, db, self.prefs.clone())
            .with_time_zone(no_zone())
            .with_languages(no_languages())
            .with_location(no_location());
        if let Some(responder) = &self.responder {
            plugin = plugin.with_http_responder(Arc::clone(responder));
        }
        self.core.register(Box::new(plugin));
        self.wait_indexed();
    }

    /// The plugin's storage, opened on the harness's database.
    pub fn db(&self) -> PluginDb {
        PluginDb::open(&self.db_path).unwrap()
    }

    /// The plugin's stored value of `key`, as text.
    pub fn stored(&self, key: &str) -> Option<String> {
        let value = self.db().kv_get(self.id, key).unwrap()?;
        Some(String::from_utf8(value).unwrap())
    }

    /// Applies the next core event; unsolicited effects are also kept in `unsolicited`.
    pub fn update(&mut self) -> CoreUpdate {
        let ev = wait_event(&self.rx);
        self.apply(ev)
    }

    pub fn apply(&mut self, ev: CoreEvent) -> CoreUpdate {
        let update = self.core.apply(ev, now());
        if let CoreUpdate::ModuleEffects { effects, .. } = &update {
            self.unsolicited.extend(effects.iter().cloned());
        }
        update
    }

    /// The indexed item titled `title`.
    pub fn indexed(&mut self, title: &str) -> Option<Arc<Item>> {
        let snapshot = self.core.search(title, now());
        snapshot
            .hits
            .into_iter()
            .filter(|hit| !hit.dynamic)
            .map(|hit| hit.item)
            .find(|item| item.title == title)
    }

    /// Searches `query` and returns the plugin's answer, or `None` when none arrives in 1 s.
    pub fn search(&mut self, query: &str) -> Option<Vec<Arc<Item>>> {
        let generation = self.core.search(query, now()).generation;
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            let Ok(ev) = self.rx.try_recv() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            if let CoreUpdate::Results(snapshot) = self.apply(ev)
                && snapshot.generation == generation
            {
                let hits = snapshot.hits.into_iter().filter(|h| h.dynamic);
                return Some(hits.map(|h| h.item).collect());
            }
        }
        None
    }

    pub fn run(&mut self, id: &str, action: &str) -> Result<Vec<Effect>, String> {
        let request = self
            .core
            .request_action(&ItemId::parse(id).unwrap(), action, "")
            .unwrap();
        loop {
            match self.update() {
                CoreUpdate::ActionDone {
                    request: r,
                    effects,
                    ..
                } if r == request => return Ok(effects),
                CoreUpdate::ActionFailed {
                    request: r,
                    message,
                } if r == request => return Err(message),
                _ => {}
            }
        }
    }

    /// Searches `query` and returns the plugin's whole answer: its items (empty when the
    /// answer was rejected) and its widget answers, as the core received them.
    pub fn search_answer(&mut self, query: &str) -> SearchAnswer {
        let generation = self.core.search(query, now()).generation;
        let mut items = Vec::new();
        loop {
            let ev = wait_event(&self.rx);
            let mut widgets = None;
            match unwrapped(&ev) {
                CoreEvent::SearchResults {
                    provider,
                    generation: g,
                    items: answered,
                    ..
                } if provider == self.id && *g == generation => items = answered.clone(),
                CoreEvent::WidgetAnswers {
                    provider,
                    generation: g,
                    answers,
                } if provider == self.id && *g == generation => {
                    widgets = Some(answers.iter().map(|a| a.answer.clone()).collect());
                }
                _ => {}
            }
            self.apply(ev);
            if let Some(widgets) = widgets {
                return SearchAnswer {
                    items,
                    widgets,
                    busy: false,
                };
            }
        }
    }

    /// Puts `home` on the user's Home, as a change of `config.toml` does: the core shows it
    /// at the empty query and tells each plugin its own tiles.
    pub fn set_home(&mut self, home: Vec<HomeEntry>) {
        self.core.set_home(home);
    }

    /// Puts the plugin's `widgets` on Home, without params, in their default sizes.
    pub fn home_widgets(&mut self, widgets: &[&str]) {
        let home = widgets
            .iter()
            .map(|w| home_entry(self.id, w, &[]))
            .collect();
        self.set_home(home);
    }

    /// The tile of the plugin's `widget`: the one on the strip (a search tile at a typed
    /// query, a Home tile at the empty one), else its Home tile. Panics when there is none, or
    /// when two tiles of it could be meant.
    pub fn tile(&self, widget: &str) -> TileKey {
        let key = WidgetKey::new(self.id, widget);
        let on_strip: Vec<TileKey> = self
            .core
            .strip()
            .into_iter()
            .map(|tile| tile.key)
            .filter(|k| k.widget == key)
            .collect();
        match &on_strip[..] {
            [one] => return one.clone(),
            [] => {}
            more => panic!("{key} has {} tiles on the strip: {more:?}", more.len()),
        }
        let home: Vec<TileKey> = self
            .core
            .home()
            .iter()
            .filter(|entry| entry.widget == key)
            .map(|entry| TileKey {
                widget: key.clone(),
                params: entry.params.clone(),
                slot: Slot::Home(0),
            })
            .collect();
        match &home[..] {
            [one] => one.clone(),
            [] => panic!("{key} has no tile on the strip nor on Home"),
            more => panic!("{key} has {} tiles on Home: {more:?}", more.len()),
        }
    }

    /// The frame the tile of `widget` ([`tile`](Self::tile)) shows.
    pub fn frame(&self, widget: &str) -> Option<Arc<Frame>> {
        self.core.widget_frame(&self.tile(widget))
    }

    /// Renders the tile of the plugin's `widget` ([`tile`](Self::tile)) once and returns
    /// the frame the core shows after it: the new one, or the last one when the new one was
    /// dropped.
    pub fn render(&mut self, widget: &str) -> Option<Arc<Frame>> {
        let key = self.tile(widget);
        self.render_tile(&key)
    }

    /// [`render`](Self::render) of the tile `key`.
    pub fn render_tile(&mut self, key: &TileKey) -> Option<Arc<Frame>> {
        // At most 60 renders per second per tile: wait for the next slot.
        let deadline = Instant::now() + Duration::from_secs(1);
        while !self.core.render_widget(key, Instant::now()) {
            assert!(
                Instant::now() < deadline,
                "the render of {key} is not issued"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        loop {
            let ev = wait_event(&self.rx);
            let done = matches!(
                unwrapped(&ev),
                CoreEvent::WidgetRendered { provider, tile, .. }
                    if provider == self.id && tile == key
            );
            self.apply(ev);
            if done {
                return self.core.widget_frame(key);
            }
        }
    }

    /// Sends `event` to the tile of the plugin's `widget` ([`tile`](Self::tile)) and returns
    /// the effects of its answer. A Home tile takes events only at the empty query: for one,
    /// the query is emptied first, as the user does to see Home.
    pub fn widget_event(
        &mut self,
        widget: &str,
        event: WidgetEvent,
    ) -> Result<Vec<Effect>, String> {
        let key = self.tile(widget);
        self.tile_event(&key, event)
    }

    /// [`widget_event`](Self::widget_event) to the tile `key`.
    pub fn tile_event(&mut self, key: &TileKey, event: WidgetEvent) -> Result<Vec<Effect>, String> {
        let on_strip = self.core.strip().iter().any(|tile| tile.key == *key);
        if matches!(key.slot, Slot::Home(_)) && !on_strip {
            self.core.search("", now());
        }
        let request = self
            .core
            .send_widget_event(key, event)
            .map_err(|e| e.to_string())?;
        loop {
            match self.update() {
                CoreUpdate::WidgetEffects {
                    request: r,
                    effects,
                    ..
                } if r == request => return Ok(effects),
                CoreUpdate::ActionFailed {
                    request: r,
                    message,
                } if Some(r) == request => return Err(message),
                _ => {}
            }
        }
    }

    /// The running instance of the plugin (views it pushed carry it).
    pub fn instance(&self) -> u64 {
        self.core.module_instance(self.id).unwrap_or_default()
    }

    pub fn view_event(&mut self, view: &str, event: ViewEvent) -> Result<Vec<Effect>, String> {
        let request = self
            .core
            .send_view_event(self.id, self.instance(), &ViewId(view.into()), event)
            .unwrap();
        loop {
            match self.update() {
                CoreUpdate::ViewEffects {
                    request: r,
                    effects,
                    ..
                } if r == request => return Ok(effects),
                CoreUpdate::ViewFailed {
                    request: r,
                    message,
                    ..
                } if r == request => return Err(message),
                _ => {}
            }
        }
    }

    /// Waits for the next unsolicited toast.
    pub fn toast(&mut self) -> Toast {
        loop {
            if let Some(toast) = self.take_toast() {
                return toast;
            }
            self.update();
        }
    }

    pub fn take_toast(&mut self) -> Option<Toast> {
        let at = self
            .unsolicited
            .iter()
            .position(|e| matches!(e, Effect::Toast(_)))?;
        match self.unsolicited.remove(at) {
            Effect::Toast(toast) => Some(toast),
            _ => unreachable!(),
        }
    }

    /// Applies every event that arrives within `wait`.
    pub fn settle(&mut self, wait: Duration) {
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            match self.rx.try_recv() {
                Ok(ev) => {
                    self.apply(ev);
                }
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    pub fn broken_event(&self) -> PluginEvents {
        self.plugin_events
            .recv_timeout(Duration::from_secs(5))
            .expect("the plugin asks to be unloaded")
    }
}

/// A Home entry of `provider`'s `widget` with `params`, in the widget's default size.
pub fn home_entry(provider: &str, widget: &str, pairs: &[(&str, &str)]) -> HomeEntry {
    sized_home_entry(provider, widget, None, pairs)
}

/// [`home_entry`] in `size`.
pub fn sized_home_entry(
    provider: &str,
    widget: &str,
    size: Option<WidgetSize>,
    pairs: &[(&str, &str)],
) -> HomeEntry {
    HomeEntry {
        widget: WidgetKey::new(provider, widget),
        size,
        params: params(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string()))),
    }
}

/// The metadata of the detail view `effects` push, as `(key, value)` pairs.
pub fn metadata(effects: &[Effect]) -> BTreeMap<String, String> {
    match effects {
        [Effect::PushView(View::Detail(view))] => view.metadata.iter().cloned().collect(),
        other => panic!("expected one detail view, got {other:?}"),
    }
}

/// Asserts that the plugin is broken for `reason`: one error toast, one unload request, and
/// every later call fails with `plugin kitchen-sink stopped: <reason…>`.
pub fn assert_broken(h: &mut Harness, reason: &str) {
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    let message = toast.message.clone().unwrap_or_default();
    assert!(
        toast.title.contains(NAME) && message.contains(reason),
        "{toast:?}"
    );
    match h.broken_event() {
        PluginEvents::Broken { id, reason: r, .. } => {
            assert_eq!(id, ID);
            assert!(r.contains(reason), "{r}");
        }
        #[allow(unreachable_patterns)]
        other => panic!("unexpected {other:?}"),
    }
    let err = h.run("kitchen-sink/hello", "run").unwrap_err();
    assert!(
        err.starts_with("plugin kitchen-sink stopped: ") && err.contains(reason),
        "{err}"
    );
    assert!(h.view_event("v", ViewEvent::Closed).is_err());
    assert_eq!(
        h.search("echo:x"),
        None,
        "a broken plugin answers no search"
    );
    // Exactly one toast and one unload request.
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none(), "a second toast");
    assert!(
        h.plugin_events.try_recv().is_err(),
        "a second unload request"
    );
}

/// The metadata of the next unsolicited detail view `view` pushes, or `None` when none comes
/// within `wait`. Earlier unsolicited effects stay in `unsolicited`.
pub fn next_detail(
    h: &mut Harness,
    view: &str,
    wait: Duration,
) -> Option<BTreeMap<String, String>> {
    let deadline = Instant::now() + wait;
    loop {
        let at = h
            .unsolicited
            .iter()
            .position(|e| matches!(e, Effect::PushView(View::Detail(d)) if d.id.0 == view));
        if let Some(at) = at {
            let Effect::PushView(View::Detail(d)) = h.unsolicited.remove(at) else {
                unreachable!()
            };
            return Some(d.metadata.into_iter().collect());
        }
        match h.rx.try_recv() {
            Ok(ev) => {
                h.apply(ev);
            }
            Err(_) if Instant::now() >= deadline => return None,
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}
