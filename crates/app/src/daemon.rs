//! The daemon: wires the core, the modules, IPC, global hotkeys and the launcher UI.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context as _, anyhow, bail};
use gpui_kit::{App, BorrowAppContext as _, QuitMode};
use tomakast_core::store::Store;
use tomakast_core::{
    Core, CoreUpdate, Hotkey, ItemId, LocalNow, Paths, QueryHistory, RankWeights, Settings,
};
use tomakast_module_apps::AppsModule;
use tomakast_module_demo::DemoModule;
use tomakast_module_history::HistoryModule;
use tomakast_module_home::HomeModule;
use tomakast_platform::{HotkeyEvent, HotkeyService};
use tomakast_plugin_host::{
    HostEngine, ManagerAction, ManagerThread, PluginDb, PluginLinkPolicy, PluginManager,
    PluginsHandle, PluginsModule,
};
use tomakast_ui::LauncherController;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config_watch::watch_config;
use crate::hotkeys::{Hotkeys, LAUNCHER_ID, Outcome};
use crate::ipc::{self, Request, ServeError};

/// Runs the daemon until the process exits. Fails when another daemon is running.
pub fn run(paths: Paths) -> anyhow::Result<()> {
    init_tracing();
    if let Some(warning) = raster_helper_missing(&tomakast_ui::raster_helper()) {
        warn!("{warning}");
    }
    paths.ensure_dirs()?;

    // IPC threads only post requests here; the UI thread handles them.
    // Requests that arrive before the UI loop starts wait in the channel.
    let (requests_tx, requests) = async_channel::unbounded::<Received>();
    let ipc_tx = requests_tx.clone();
    // The socket is the single-instance lock, so it is taken before anything
    // else touches the database or starts modules. Kept alive for the whole
    // run: dropping it removes the socket.
    let _server = match ipc::serve(move |req| post(&ipc_tx, req)) {
        Ok(server) => server,
        Err(ServeError::AlreadyRunning) => bail!("another tomakast daemon is already running"),
        Err(e) => return Err(e.into()),
    };

    let (settings, settings_error) = Settings::load_or_default(&paths.config_file);
    let settings_loaded = settings_error.is_none();
    if let Some(e) = settings_error {
        error!("{e:#}; using the default settings");
    }
    let store = Store::open(&paths.db_file)
        .with_context(|| format!("opening {}", paths.db_file.display()))?;
    let mut core = Core::new(store, RankWeights::default(), LocalNow::now())?;
    // "Add to Home" writes the list into this file; a change of it reaches the core through
    // the plugin manager (`ManagerAction::Home`).
    core.set_config_file(paths.config_file.clone());
    core.set_home(settings.home.clone());
    // The search history has its own store handle; without it the in-memory default stays.
    match Store::open(&paths.db_file)
        .with_context(|| format!("opening {}", paths.db_file.display()))
        .and_then(QueryHistory::load)
    {
        Ok(history) => core.set_history(history),
        Err(e) => error!("search history is not kept: {e:#}"),
    }
    // Every built-in module, by id: they may open links, and no plugin may take their id.
    let mut native_ids = HashSet::new();
    let mut register_native = |core: &mut Core, module: Box<dyn tomakast_core::Module>| {
        native_ids.insert(module.id().to_string());
        core.register(module);
    };
    register_native(&mut core, Box::new(AppsModule::new()));
    let history = core.history().clone();
    register_native(&mut core, Box::new(HistoryModule::new(history)));
    // The query that opened Search History is not recorded.
    core.exempt_from_history(tomakast_module_history::PROVIDER);
    register_native(&mut core, Box::new(HomeModule::new()));
    if std::env::var_os("TOMAKAST_DEMO").is_some_and(|v| v == "1") {
        info!("TOMAKAST_DEMO=1: the demo module is registered");
        register_native(&mut core, Box::new(DemoModule::new()));
    }
    native_ids.insert(tomakast_plugin_host::plugins_module::PROVIDER.to_string());
    let link_policy = PluginLinkPolicy::new(native_ids.clone());
    // Without its database the review module cannot record a decision: then no plugin loads
    // (logged), and the launcher runs on.
    let plugins_db = PluginDb::open(&paths.db_file)
        .with_context(|| format!("opening {} for plugins", paths.db_file.display()))
        .inspect_err(|e| error!("plugins are not loaded: {e:#}"))
        .ok();
    let manager = plugins_db
        .as_ref()
        .and_then(|_| plugin_manager(&paths, &settings, native_ids, link_policy.clone()));
    // The review module is there whenever its database is, even when plugins cannot load.
    let plugins = plugins_db.map(|db| {
        let events = match &manager {
            Some(manager) => manager.events(),
            None => mpsc::channel().0,
        };
        PluginsModule::install(&mut core, db, events)
    });
    core.start_modules();
    // Plugins load after the built-in modules have started, off the UI thread.
    let (plugin_actions_tx, plugin_actions) = async_channel::unbounded();
    let _plugin_manager: Option<ManagerThread> = manager.and_then(|manager| {
        let spawned = manager.spawn(move |actions| {
            // Unbounded: fails only when the UI side is gone.
            let _ = plugin_actions_tx.try_send(actions);
        });
        spawned
            .inspect_err(|e| error!("plugins are not loaded: {e:#}"))
            .ok()
    });

    match autostart_plan(home_override(), settings_loaded, settings.autostart) {
        AutostartPlan::Leave(LeaveReason::HomeOverride) => {
            info!("TOMAKAST_HOME is set: autostart is left untouched");
        }
        AutostartPlan::Leave(LeaveReason::SettingsError) => {
            warn!("the config file could not be loaded: autostart is left untouched");
        }
        AutostartPlan::Set(enabled) => {
            if let Err(e) = tomakast_platform::set_autostart(enabled) {
                error!("cannot update autostart: {e:#}");
            }
        }
    }

    let core = Rc::new(RefCell::new(core));
    // The bundled Lucide icons, which theme icon names resolve to.
    gpui_kit::application()
        .with_assets(tomakast_ui::assets())
        .run(move |cx: &mut App| {
            // `tomakast_ui::init` calls `gpui_kit::init` itself.
            tomakast_ui::init(cx);
            // Names outside the bundle: the desktop's icon theme, off the UI thread.
            tomakast_ui::resolve_named_icons(cx);
            cx.set_quit_mode(QuitMode::Explicit);
            let mut controller = LauncherController::new(core.clone());
            controller.set_link_policy(Rc::new(link_policy));
            controller.prewarm(cx);
            cx.set_global(controller);
            spawn_core_events(core.clone(), cx);
            spawn_plugin_actions(core.clone(), plugins, plugin_actions, cx);
            spawn_requests(core.clone(), requests, cx);
            spawn_hotkeys(core, &paths.config_file, &settings, cx);
        });
    Ok(())
}

/// The plugin manager, or `None` (logged) when plugins cannot run in this process.
fn plugin_manager(
    paths: &Paths,
    settings: &Settings,
    native_ids: HashSet<String>,
    link_policy: PluginLinkPolicy,
) -> Option<PluginManager> {
    let setup = || -> anyhow::Result<PluginManager> {
        let engine = HostEngine::new().context("starting the plugin engine")?;
        let db = PluginDb::open(&paths.db_file)
            .with_context(|| format!("opening {} for plugins", paths.db_file.display()))?;
        Ok(
            PluginManager::new(paths, engine, db, settings.plugin_preferences.clone())
                .with_native_ids(native_ids)
                .with_link_policy(link_policy)
                .with_location(&settings.location),
        )
    };
    setup()
        .inspect_err(|e| error!("plugins are not loaded: {e:#}"))
        .ok()
}

/// Applies the plugin manager's actions to the core on the UI thread, then re-ranks the open
/// window's results: registering and unregistering do not re-rank by themselves.
fn spawn_plugin_actions(
    core: Rc<RefCell<Core>>,
    plugins: Option<PluginsHandle>,
    actions: async_channel::Receiver<Vec<ManagerAction>>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Ok(batch) = actions.recv().await {
            apply_plugin_batch(&core, plugins.as_ref(), batch, || {
                cx.update(|cx| {
                    cx.update_global::<LauncherController, _>(|c, cx| {
                        c.core_update(CoreUpdate::ItemsChanged, cx);
                    })
                });
            });
        }
    })
    .detach();
}

/// Applies `batch` in order under one core borrow. When it registered or unregistered a
/// module, calls `rerank` once, after that borrow has ended: the launcher view borrows the
/// core to re-rank. Reviews go to the `plugins` module, whose items reach the core through
/// its own `ReplaceItems`. A new Home list goes to the core, which redraws the strip itself.
fn apply_plugin_batch(
    core: &RefCell<Core>,
    plugins: Option<&PluginsHandle>,
    batch: Vec<ManagerAction>,
    rerank: impl FnOnce(),
) {
    let mut changed = false;
    {
        let mut core = core.borrow_mut();
        for action in batch {
            match action {
                ManagerAction::Register(module) => {
                    core.register(module);
                    changed = true;
                }
                ManagerAction::Unregister(id) => {
                    core.unregister(&id);
                    changed = true;
                }
                ManagerAction::Reviews(reviews) => match plugins {
                    Some(plugins) => plugins.set_reviews(reviews),
                    None => tracing::debug!("no plugins module for the reviews"),
                },
                ManagerAction::Home(home) => {
                    core.home_file_changed(home);
                }
            }
        }
    }
    if changed {
        rerank();
    }
}

/// A request and the moment a hotkey activation or an IPC connection
/// delivered it: the start of the logged open-latency metric.
struct Received {
    req: Request,
    at: Instant,
}

/// Stamps `req` with the current time and queues it for the UI thread.
fn post(tx: &async_channel::Sender<Received>, req: Request) {
    // Unbounded: fails only when the UI side is gone.
    let _ = tx.try_send(Received {
        req,
        at: Instant::now(),
    });
}

#[derive(Debug, PartialEq, Eq)]
enum AutostartPlan {
    Set(bool),
    Leave(LeaveReason),
}

#[derive(Debug, PartialEq, Eq)]
enum LeaveReason {
    /// Dev and test runs never write autostart.
    HomeOverride,
    /// The defaults that replace a broken config say `autostart = true`;
    /// applying them would undo the user's `autostart = false`.
    SettingsError,
}

/// Whether to write or remove the autostart entry, or leave it as it is.
fn autostart_plan(home_override: bool, settings_loaded: bool, enabled: bool) -> AutostartPlan {
    if home_override {
        AutostartPlan::Leave(LeaveReason::HomeOverride)
    } else if !settings_loaded {
        AutostartPlan::Leave(LeaveReason::SettingsError)
    } else {
        AutostartPlan::Set(enabled)
    }
}

/// Logs to stderr; `RUST_LOG` overrides the default filter. notify logs harmless INFO lines
/// whenever a watched plugin folder is removed, so it is quieter by default.
fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,notify=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// The warning to log when the raster helper is not at `helper`: plugin images then draw
/// nothing, and nothing else says why.
fn raster_helper_missing(helper: &Path) -> Option<String> {
    (!helper.is_file()).then(|| {
        format!(
            "the raster helper is missing at {}: widget images and SVG draw nothing \
             (`just build` builds it next to tomakast)",
            helper.display()
        )
    })
}

/// Same rule as `Paths::detect`: a non-empty `TOMAKAST_HOME`.
fn home_override() -> bool {
    std::env::var_os("TOMAKAST_HOME").is_some_and(|v| !v.is_empty())
}

/// Applies the replies and events of module workers to the core and hands
/// the resulting updates to the launcher.
fn spawn_core_events(core: Rc<RefCell<Core>>, cx: &mut App) {
    let events = core.borrow().events();
    cx.spawn(async move |cx| {
        while let Ok(ev) = events.recv().await {
            let mut updates = Vec::new();
            {
                let mut core = core.borrow_mut();
                let now = LocalNow::now();
                updates.push(core.apply(ev, now));
                while let Ok(ev) = events.try_recv() {
                    updates.push(core.apply(ev, now));
                }
            }
            // The core borrow has ended: the launcher view borrows it too.
            cx.update(|cx| {
                cx.update_global::<LauncherController, _>(|c, cx| {
                    for update in coalesce(updates) {
                        c.core_update(update, cx);
                    }
                })
            });
        }
    })
    .detach();
}

/// Drops `None` updates and all but the last `ItemsChanged`: one refresh per
/// burst, after any `Results` computed before the last item change.
fn coalesce(updates: Vec<CoreUpdate>) -> Vec<CoreUpdate> {
    let last_items_changed = updates
        .iter()
        .rposition(|u| matches!(u, CoreUpdate::ItemsChanged));
    updates
        .into_iter()
        .enumerate()
        .filter(|(ix, update)| match update {
            CoreUpdate::None => false,
            CoreUpdate::ItemsChanged => Some(*ix) == last_items_changed,
            _ => true,
        })
        .map(|(_, update)| update)
        .collect()
}

fn spawn_requests(
    core: Rc<RefCell<Core>>,
    requests: async_channel::Receiver<Received>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Ok(received) = requests.recv().await {
            cx.update(|cx| handle_request(received, &core, cx));
        }
    })
    .detach();
}

fn handle_request(received: Received, core: &RefCell<Core>, cx: &mut App) {
    let Received { req, at } = received;
    match req {
        Request::Toggle => {
            cx.update_global::<LauncherController, _>(|c, cx| c.toggle_at(at, cx));
        }
        Request::Run(id) => run_item(&id, core),
        Request::Ping => {}
    }
}

/// Requests the primary action of `id` with an empty query, as a hotkey or
/// `tomakast run` does. The reply reaches the launcher through `core_update`,
/// which hides it on `CloseWindow` and logs a failure.
fn run_item(id: &ItemId, core: &RefCell<Core>) {
    let mut core = core.borrow_mut();
    let result = match core.primary_action(id) {
        Some(action) => core.request_action(id, &action, ""),
        None => Err(anyhow!("unknown item {}", id.as_str())),
    };
    if let Err(e) = result {
        error!(item = %id.as_str(), "cannot run item: {e:#}");
    }
}

/// Starts the global hotkeys on the UI thread (macOS and Windows need them there): binds the
/// sets of `settings`, follows edits of `config_file`, and takes the recorder's changes.
fn spawn_hotkeys(core: Rc<RefCell<Core>>, config_file: &Path, settings: &Settings, cx: &mut App) {
    let (events_tx, events) = async_channel::unbounded::<HotkeyEvent>();
    // `TOMAKAST_NO_KEYBOARD=1` gets the null backend: nothing real is bound.
    let service = HotkeyService::start(move |event| {
        // Unbounded: fails only when the UI side is gone.
        let _ = events_tx.try_send(event);
    });
    let hotkeys = Rc::new(RefCell::new(Hotkeys::new(
        service,
        config_file.to_path_buf(),
    )));

    let (changes_tx, changes) = async_channel::unbounded::<(ItemId, Option<Hotkey>)>();
    cx.update_global::<LauncherController, _>(|c, _| {
        c.set_hotkey_requests(Rc::new(RecorderRequests(changes_tx)));
    });
    let (reload_tx, reloads) = async_channel::unbounded::<()>();
    let watch = watch_config(config_file, move || {
        let _ = reload_tx.try_send(());
    })
    .inspect_err(|e| warn!("hotkey edits of config.toml need a restart: {e:#}"))
    .ok();

    let outcomes = hotkeys
        .borrow_mut()
        .load(settings, &|item| core.borrow().item_title(item));
    apply_outcomes(outcomes, &core, cx);

    {
        let (core, hotkeys) = (core.clone(), hotkeys.clone());
        cx.spawn(async move |cx| {
            while let Ok(event) = events.recv().await {
                let outcomes = hotkeys.borrow_mut().on_event(event);
                cx.update(|cx| apply_outcomes(outcomes, &core, cx));
            }
        })
        .detach();
    }
    {
        let (core, hotkeys) = (core.clone(), hotkeys.clone());
        cx.spawn(async move |cx| {
            while let Ok((item, hotkey)) = changes.recv().await {
                let result = hotkeys
                    .borrow_mut()
                    .request(&item, hotkey, &|item| core.borrow().item_title(item));
                let outcomes = match result {
                    Ok(outcomes) => outcomes,
                    Err(refusal) => vec![Outcome::Notice(refusal.to_string())],
                };
                cx.update(|cx| apply_outcomes(outcomes, &core, cx));
            }
        })
        .detach();
    }
    let config_file = config_file.to_path_buf();
    cx.spawn(async move |cx| {
        // Lives as long as this loop: the whole run.
        let _watch = watch;
        while reloads.recv().await.is_ok() {
            let settings = match Settings::load(&config_file) {
                Ok(settings) => settings,
                Err(e) => {
                    warn!("{e:#}; hotkeys stay as they were");
                    continue;
                }
            };
            let outcomes = hotkeys
                .borrow_mut()
                .load(&settings, &|item| core.borrow().item_title(item));
            cx.update(|cx| apply_outcomes(outcomes, &core, cx));
        }
    })
    .detach();
}

/// Hands the recorder's changes to the UI thread's hotkey loop.
struct RecorderRequests(async_channel::Sender<(ItemId, Option<Hotkey>)>);

impl tomakast_ui::HotkeyRequests for RecorderRequests {
    fn assign(&self, item: &ItemId, hotkey: Option<Hotkey>) {
        let _ = self.0.try_send((item.clone(), hotkey));
    }
}

/// Shows tables and notices in the launcher, and runs presses: the launcher hotkey toggles
/// the window, an item's runs its primary action, and any of them goes to an open recorder.
fn apply_outcomes(outcomes: Vec<Outcome>, core: &RefCell<Core>, cx: &mut App) {
    for outcome in outcomes {
        match outcome {
            Outcome::Table(table) => {
                cx.update_global::<LauncherController, _>(|c, cx| c.set_hotkey_table(table, cx));
            }
            Outcome::Notice(text) => {
                cx.update_global::<LauncherController, _>(|c, cx| c.hotkey_notice(text, cx));
            }
            Outcome::Pressed { id, hotkey } => {
                let recording = cx.global::<LauncherController>().is_recording(cx);
                if recording {
                    cx.update_global::<LauncherController, _>(|c, cx| c.recorded_press(hotkey, cx));
                } else if id == LAUNCHER_ID {
                    let at = Instant::now();
                    cx.update_global::<LauncherController, _>(|c, cx| c.toggle_at(at, cx));
                } else if let Some(item) = ItemId::parse(&id) {
                    run_item(&item, core);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_raster_helper_is_named_in_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("tomakast-raster");

        let warning = raster_helper_missing(&helper).expect("a warning");
        assert!(warning.contains(&helper.display().to_string()), "{warning}");

        std::fs::write(&helper, b"").unwrap();
        assert_eq!(raster_helper_missing(&helper), None);
    }

    #[test]
    fn the_raster_helper_is_looked_for_next_to_the_executable() {
        let exe = std::env::current_exe().unwrap();
        let helper = tomakast_ui::raster_helper();
        assert_eq!(helper.parent(), exe.parent());
        assert!(
            helper
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("tomakast-raster"),
            "{helper:?}"
        );
    }

    #[test]
    fn autostart_follows_cleanly_loaded_settings_only() {
        assert_eq!(autostart_plan(false, true, true), AutostartPlan::Set(true));
        assert_eq!(
            autostart_plan(false, true, false),
            AutostartPlan::Set(false)
        );
        // A broken config yields default settings, whose `autostart = true` must not apply.
        assert_eq!(
            autostart_plan(false, false, true),
            AutostartPlan::Leave(LeaveReason::SettingsError)
        );
        assert_eq!(
            autostart_plan(true, true, true),
            AutostartPlan::Leave(LeaveReason::HomeOverride)
        );
        assert_eq!(
            autostart_plan(true, false, true),
            AutostartPlan::Leave(LeaveReason::HomeOverride)
        );
    }

    #[test]
    fn posted_requests_carry_their_receipt_time() {
        let (tx, rx) = async_channel::unbounded();
        let before = Instant::now();
        post(&tx, Request::Toggle);
        let after = Instant::now();
        // Taken later, as the UI thread would; the stamp must not move.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let received = rx.try_recv().unwrap();
        assert_eq!(received.req, Request::Toggle);
        assert!(before <= received.at && received.at <= after);
    }

    #[test]
    fn coalesce_keeps_only_the_last_items_changed() {
        let results = || {
            CoreUpdate::Results(tomakast_core::SearchSnapshot {
                generation: 1,
                hits: Vec::new(),
            })
        };
        let updates = vec![
            CoreUpdate::ItemsChanged,
            CoreUpdate::None,
            results(),
            CoreUpdate::ItemsChanged,
            CoreUpdate::None,
        ];
        let kept = coalesce(updates);
        assert_eq!(kept.len(), 2);
        assert!(matches!(kept[0], CoreUpdate::Results(_)));
        // The refresh comes after the results computed before the last item change.
        assert!(matches!(kept[1], CoreUpdate::ItemsChanged));
    }

    /// A module that publishes one item.
    struct OneItem(&'static str);

    impl tomakast_core::Module for OneItem {
        fn id(&self) -> &str {
            self.0
        }

        fn start(&mut self, ctx: tomakast_core::ModuleContext) {
            ctx.replace_items(vec![tomakast_core::Item {
                id: ItemId::new(self.0, "x"),
                title: "Plugged in".into(),
                subtitle: None,
                keywords: Vec::new(),
                icon: None,
                kind: tomakast_core::ItemKind::Command,
                actions: Vec::new(),
                source: None,
            }]);
        }

        fn run_action(
            &mut self,
            _item: &tomakast_core::Item,
            _action_id: &str,
        ) -> anyhow::Result<Vec<tomakast_core::Effect>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_home_action_rereads_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_file = dir.path().join("tomakast.db");
        let store = Store::open(&db_file).unwrap();
        let mut core = Core::new(store, RankWeights::default(), LocalNow::now()).unwrap();
        let (tx, _rx) = mpsc::channel();
        let plugins = PluginsModule::install(&mut core, PluginDb::open(&db_file).unwrap(), tx);
        core.start_modules();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "[[home.widgets]]\nwidget = \"newer/tile\"\n").unwrap();
        core.set_config_file(config);
        let core = RefCell::new(core);
        // The watcher carries an older list than the file holds now.
        let carried = vec![tomakast_core::HomeEntry {
            widget: tomakast_core::widget::WidgetKey::new("older", "tile"),
            size: None,
            params: tomakast_core::widget::params([]),
        }];
        apply_plugin_batch(
            &core,
            Some(&plugins),
            vec![ManagerAction::Home(carried)],
            || {},
        );
        let core = core.borrow();
        assert_eq!(core.home().len(), 1);
        assert_eq!(core.home()[0].widget.provider, "newer");
    }

    #[test]
    fn plugin_actions_change_the_core_and_ask_for_a_rerank() {
        use std::cell::Cell;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let db_file = dir.path().join("tomakast.db");
        let store = Store::open(&db_file).unwrap();
        let mut core = Core::new(store, RankWeights::default(), LocalNow::now()).unwrap();
        let (tx, _rx) = mpsc::channel();
        let plugins = PluginsModule::install(&mut core, PluginDb::open(&db_file).unwrap(), tx);
        core.start_modules();
        let events = core.events();
        let core = RefCell::new(core);
        // What the launcher does on `ItemsChanged`: re-rank the current query. It borrows
        // the core, so it panics if the batch still holds the borrow.
        core.borrow_mut().search("plugged", LocalNow::now());
        let ranked = |core: &RefCell<Core>| -> Vec<String> {
            let snapshot = core.borrow_mut().refresh(LocalNow::now());
            snapshot.hits.iter().map(|h| h.item.title.clone()).collect()
        };
        let reranks = Cell::new(0);
        let seen = RefCell::new(Vec::new());
        let rerank = || {
            reranks.set(reranks.get() + 1);
            *seen.borrow_mut() = ranked(&core);
        };

        // Reviews alone change no module: no re-rank.
        apply_plugin_batch(
            &core,
            Some(&plugins),
            vec![ManagerAction::Reviews(vec![])],
            rerank,
        );
        assert_eq!(reranks.get(), 0);

        // A Home list goes to the core and changes no module: no re-rank.
        let home = vec![tomakast_core::HomeEntry {
            widget: tomakast_core::widget::WidgetKey::new("plug", "tile"),
            size: None,
            params: tomakast_core::widget::params([]),
        }];
        apply_plugin_batch(
            &core,
            Some(&plugins),
            vec![ManagerAction::Home(home.clone())],
            rerank,
        );
        assert_eq!(reranks.get(), 0);
        assert_eq!(core.borrow().home(), home);

        let register = vec![ManagerAction::Register(Box::new(OneItem("plug")))];
        apply_plugin_batch(&core, Some(&plugins), register, rerank);
        assert_eq!(reranks.get(), 1);
        // The plugin publishes from its worker; applied as the core events loop does.
        let deadline = Instant::now() + Duration::from_secs(5);
        while ranked(&core).is_empty() {
            assert!(Instant::now() < deadline, "the plugin never published");
            match events.try_recv() {
                Ok(ev) => {
                    core.borrow_mut().apply(ev, LocalNow::now());
                }
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        assert_eq!(ranked(&core), ["Plugged in"]);

        // The re-rank after an unregister no longer shows the plugin's item.
        let unregister = vec![ManagerAction::Unregister("plug".into())];
        apply_plugin_batch(&core, Some(&plugins), unregister, rerank);
        assert_eq!(reranks.get(), 2);
        assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());

        // Without the plugins module, reviews are dropped and modules still change.
        let register = vec![
            ManagerAction::Reviews(vec![]),
            ManagerAction::Register(Box::new(OneItem("plug"))),
        ];
        apply_plugin_batch(&core, None, register, rerank);
        assert_eq!(reranks.get(), 3);
    }
}
