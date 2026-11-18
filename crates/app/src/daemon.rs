//! The daemon: wires the core, the modules, IPC, global hotkeys and the launcher UI.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context as _, anyhow, bail};
use gpui_kit::{App, BorrowAppContext as _, QuitMode};
use tomakast_core::store::Store;
use tomakast_core::{Core, CoreUpdate, ItemId, LocalNow, Paths, RankWeights, Settings};
use tomakast_module_apps::AppsModule;
use tomakast_module_demo::DemoModule;
use tomakast_platform::{HotkeyBackend, HotkeyBinding};
use tomakast_ui::LauncherController;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::ipc::{self, Request, ServeError};

/// Portal shortcut id of the launcher hotkey; app hotkeys use their `ItemId`.
const LAUNCHER_ID: &str = "launcher";
const LAUNCHER_DESCRIPTION: &str = "Open tomakast";

/// Runs the daemon until the process exits. Fails when another daemon is running.
pub fn run(paths: Paths) -> anyhow::Result<()> {
    init_tracing();
    paths.ensure_dirs()?;

    // IPC and hotkey threads only post requests here; the UI thread handles them.
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
    core.register(Box::new(AppsModule::new()));
    if std::env::var_os("TOMAKAST_DEMO").is_some_and(|v| v == "1") {
        info!("TOMAKAST_DEMO=1: the demo module is registered");
        core.register(Box::new(DemoModule::new()));
    }
    core.start_modules();

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
    gpui_kit::application().run(move |cx: &mut App| {
        // `tomakast_ui::init` calls `gpui_kit::init` itself.
        tomakast_ui::init(cx);
        cx.set_quit_mode(QuitMode::Explicit);
        let mut controller = LauncherController::new(core.clone());
        controller.prewarm(cx);
        cx.set_global(controller);
        spawn_core_events(core.clone(), cx);
        spawn_requests(core, requests, cx);
        register_hotkeys(hotkey_bindings(&settings), requests_tx);
    });
    Ok(())
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

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Same rule as `Paths::detect`: a non-empty `TOMAKAST_HOME`.
fn home_override() -> bool {
    std::env::var_os("TOMAKAST_HOME").is_some_and(|v| !v.is_empty())
}

/// `TOMAKAST_NO_KEYBOARD=1` (development runs on a live session) also leaves
/// the portal alone, so such a run never binds real shortcuts.
fn hotkeys_allowed() -> bool {
    std::env::var_os("TOMAKAST_NO_KEYBOARD").is_none_or(|v| v != "1")
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

/// The launcher hotkey and the usable `[hotkeys]`. Dropped with a warning:
/// a hotkey without a modifier, the launcher hotkey, a hotkey already bound,
/// and a second hotkey for the same item (portal shortcut ids must be unique).
/// The first binding wins.
fn hotkey_bindings(settings: &Settings) -> Vec<HotkeyBinding> {
    let mut bindings = vec![HotkeyBinding {
        id: LAUNCHER_ID.into(),
        description: LAUNCHER_DESCRIPTION.into(),
        hotkey: settings.launcher_hotkey.clone(),
    }];
    for (hotkey, item) in &settings.hotkeys {
        let item = item.as_str();
        let problem = if hotkey.mods.is_empty() {
            Some("it has no modifier")
        } else if *hotkey == settings.launcher_hotkey {
            Some("it is the launcher hotkey")
        } else if bindings.iter().any(|b| b.hotkey == *hotkey) {
            Some("it is already bound to another item")
        } else if bindings.iter().any(|b| b.id == item) {
            Some("the item already has a hotkey")
        } else {
            None
        };
        match problem {
            Some(problem) => warn!(%hotkey, item, "ignoring hotkey: {problem}"),
            // The index is still empty here, so the description is the item id, not its title.
            None => bindings.push(HotkeyBinding {
                id: item.to_string(),
                description: item.to_string(),
                hotkey: hotkey.clone(),
            }),
        }
    }
    bindings
}

/// Maps a portal shortcut id back to a request.
fn activation_request(id: &str) -> Option<Request> {
    if id == LAUNCHER_ID {
        return Some(Request::Toggle);
    }
    ItemId::parse(id).map(Request::Run)
}

/// Registers the hotkeys off the UI thread: the portal call blocks until it
/// answers, on KDE possibly until the user confirms a dialog.
fn register_hotkeys(bindings: Vec<HotkeyBinding>, requests: async_channel::Sender<Received>) {
    if !hotkeys_allowed() {
        info!("TOMAKAST_NO_KEYBOARD=1: global hotkeys are not registered");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("tomakast-hotkey-setup".into())
        .spawn(move || {
            let backend = tomakast_platform::register_global_hotkeys(bindings, move |id| {
                match activation_request(&id) {
                    Some(req) => post(&requests, req),
                    None => warn!(id, "unknown hotkey activation"),
                }
            });
            match &backend {
                HotkeyBackend::Portal => info!(?backend, "global hotkeys registered"),
                HotkeyBackend::Unsupported { reason } => warn!(
                    ?backend,
                    "global hotkeys are unavailable ({reason}); \
                     bind the `tomakast toggle` command in your system settings"
                ),
            }
        });
    if let Err(e) = spawned {
        error!("cannot start the hotkey registration thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use tomakast_core::Hotkey;

    use super::*;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn id(s: &str) -> ItemId {
        ItemId::parse(s).unwrap()
    }

    fn settings(hotkeys: &[(&str, &str)]) -> Settings {
        Settings {
            hotkeys: hotkeys.iter().map(|(k, i)| (hk(k), id(i))).collect(),
            ..Settings::default()
        }
    }

    fn summary(bindings: &[HotkeyBinding]) -> Vec<(String, String, String)> {
        bindings
            .iter()
            .map(|b| (b.id.clone(), b.description.clone(), b.hotkey.to_string()))
            .collect()
    }

    fn row(id: &str, description: &str, hotkey: &str) -> (String, String, String) {
        (id.into(), description.into(), hotkey.into())
    }

    #[test]
    fn launcher_binding_comes_first_and_items_are_described_by_id() {
        let s = settings(&[("Ctrl+Alt+T", "apps/konsole.desktop")]);
        assert_eq!(
            summary(&hotkey_bindings(&s)),
            vec![
                row("launcher", "Open tomakast", "Alt+Space"),
                row("apps/konsole.desktop", "apps/konsole.desktop", "Ctrl+Alt+T"),
            ]
        );
    }

    #[test]
    fn unusable_hotkeys_are_dropped_and_the_first_binding_wins() {
        let s = settings(&[
            // Equal to the launcher hotkey after normalisation.
            ("alt+SPACE", "apps/a.desktop"),
            ("F1", "apps/b.desktop"),
            ("Ctrl+Alt+T", "apps/konsole.desktop"),
            // Same hotkey as above, spelled differently.
            ("ctrl+alt+t", "apps/c.desktop"),
            // Second hotkey for the same item.
            ("Ctrl+Alt+K", "apps/konsole.desktop"),
            ("Super+F", "apps/firefox.desktop"),
        ]);
        assert_eq!(
            summary(&hotkey_bindings(&s)),
            vec![
                row("launcher", "Open tomakast", "Alt+Space"),
                row("apps/konsole.desktop", "apps/konsole.desktop", "Ctrl+Alt+T"),
                row("apps/firefox.desktop", "apps/firefox.desktop", "Super+F"),
            ]
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

    #[test]
    fn activations_map_to_requests() {
        assert_eq!(activation_request("launcher"), Some(Request::Toggle));
        assert_eq!(
            activation_request("apps/firefox.desktop"),
            Some(Request::Run(id("apps/firefox.desktop")))
        );
        assert_eq!(activation_request("garbage"), None);
    }
}
