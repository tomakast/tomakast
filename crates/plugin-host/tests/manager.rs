//! The plugin manager: discovery, the debounced watcher, hot reload, preferences, and what it
//! asks the daemon to apply to the core.

mod support;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use support::{compile_exe, guest, sha256};
use tomakast_core::store::Store;
use tomakast_core::testing::wait_event;
use tomakast_core::widget::{WidgetKey, WidgetSize, params};
use tomakast_core::{
    Core, CoreEvent, CoreUpdate, Effect, HomeEntry, ItemId, LinkPolicy, LocalNow, Paths, RELOADED,
    RankWeights, Toast, View, ViewEvent, ViewId,
};
use tomakast_core::{PluginCatalog, PluginState};
use tomakast_plugin_host::approval::ReviewReason;
use tomakast_plugin_host::plugins_module::MIN_READ_TIME;
use tomakast_plugin_host::{
    Cache, Grant, GrantDecision, HostEngine, ManagerAction, ManagerStats, ManagerThread, Package,
    PluginDb, PluginEvents, PluginLinkPolicy, PluginManager, PluginsHandle, PluginsModule,
    WidgetGrant,
};

const ID: &str = "kitchen-sink";
/// How long the tests wait for something the manager does on its own thread.
const PATIENCE: Duration = Duration::from_secs(20);

fn now() -> LocalNow {
    LocalNow::from_unix_with_offset(1_800_000_000, 0)
}

/// The built kitchen-sink folder (`plugin.toml`, `plugin.wasm`, `icons/`), and a cache dir
/// shared by every test of this binary in which kitchen-sink is already compiled: a compile
/// child running next to a plugin's 2 s `init` deadline would make the timing tests flaky.
struct Source {
    folder: PathBuf,
    cache: tempfile::TempDir,
}

fn source() -> &'static Source {
    static SOURCE: OnceLock<Source> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let wasm_path = guest(ID);
        let cache = tempfile::tempdir().unwrap();
        let wasm = std::fs::read(&wasm_path).unwrap();
        HostEngine::new()
            .unwrap()
            .load(
                sha256(&wasm),
                &wasm,
                &Cache::new(cache.path().to_owned()),
                &compile_exe(),
            )
            .unwrap();
        Source {
            folder: wasm_path.parent().unwrap().to_owned(),
            cache,
        }
    })
}

struct Env {
    home: tempfile::TempDir,
    paths: Paths,
    /// A handle on the database to inspect and seed grants.
    db: PluginDb,
}

fn env() -> Env {
    let home = tempfile::tempdir().unwrap();
    let mut paths = Paths::with(home.path().join("config.toml"), home.path().to_owned());
    paths.plugin_cache_dir = source().cache.path().to_owned();
    std::fs::create_dir_all(&paths.plugins_dir).unwrap();
    // The core store creates the schema.
    drop(Store::open(&paths.db_file).unwrap());
    let db = PluginDb::open(&paths.db_file).unwrap();
    Env { home, paths, db }
}

impl Env {
    fn manager(&self) -> PluginManager {
        self.manager_with(BTreeMap::new())
    }

    fn manager_with(&self, prefs: BTreeMap<String, BTreeMap<String, String>>) -> PluginManager {
        PluginManager::new(
            &self.paths,
            HostEngine::new().unwrap(),
            PluginDb::open(&self.paths.db_file).unwrap(),
            prefs,
        )
        .with_compile_exe(compile_exe())
    }

    fn folder(&self) -> PathBuf {
        self.paths.plugins_dir.join(ID)
    }

    /// Copies kitchen-sink into the plugins folder.
    fn install(&self) -> PathBuf {
        let dir = self.folder();
        copy_dir(&source().folder, &dir);
        dir
    }

    /// Stores an `allow` grant for the folder's current content, as the review would.
    fn approve(&self) -> [u8; 32] {
        self.decide(GrantDecision::Allow)
    }

    fn decide(&self, decision: GrantDecision) -> [u8; 32] {
        let package = Package::read(&self.folder()).unwrap();
        self.grant(&package.manifest, package.content_hash, decision);
        package.content_hash
    }

    /// A grant of `manifest`'s permissions and widgets.
    fn grant(
        &self,
        manifest: &tomakast_plugin_host::Manifest,
        content_hash: [u8; 32],
        decision: GrantDecision,
    ) {
        self.db
            .set_grant(&Grant {
                plugin_id: ID.into(),
                content_hash,
                permissions: manifest.permissions.clone(),
                name: "Kitchen Sink".into(),
                author: None,
                widgets: manifest
                    .widgets
                    .iter()
                    .map(|w| WidgetGrant {
                        id: w.id.clone(),
                        title: w.title.clone(),
                    })
                    .collect(),
                decision,
                decided_at: 1,
                updated_at: 1,
            })
            .unwrap();
    }

    /// Rewrites `plugin.toml` with `edit`.
    fn edit_toml(&self, edit: impl FnOnce(String) -> String) {
        let path = self.folder().join("plugin.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, edit(text)).unwrap();
    }

    /// Changes `plugin.toml` without changing its permissions: an in-place update.
    fn bump_version(&self, version: &str) {
        let path = self.folder().join("plugin.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text.replacen(
            "version = \"0.1.0\"",
            &format!("version = \"{version}\""),
            1,
        );
        std::fs::write(path, text).unwrap();
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The actions as short strings: `register:<id>`, `unregister:<id>`, `reviews:<ids>`,
/// `home:<widgets>`. The catalog is left out: `catalogs` reads it.
fn summary(actions: &[ManagerAction]) -> Vec<String> {
    actions
        .iter()
        .filter(|a| !matches!(a, ManagerAction::Catalog(_)))
        .map(|a| match a {
            ManagerAction::Register(m) => format!("register:{}", m.id()),
            ManagerAction::Unregister(id) => format!("unregister:{id}"),
            ManagerAction::Reviews(reviews) => {
                let ids: Vec<&str> = reviews.iter().map(|r| r.id.as_str()).collect();
                format!("reviews:{}", ids.join(","))
            }
            ManagerAction::Home(home) => {
                let widgets: Vec<String> = home.iter().map(|e| e.widget.to_string()).collect();
                format!("home:{}", widgets.join(","))
            }
            ManagerAction::Catalog(_) => unreachable!(),
        })
        .collect()
}

/// The catalogs among the actions.
fn catalogs(actions: &[ManagerAction]) -> Vec<&PluginCatalog> {
    actions
        .iter()
        .filter_map(|a| match a {
            ManagerAction::Catalog(catalog) => Some(catalog),
            _ => None,
        })
        .collect()
}

fn count(n: &std::sync::atomic::AtomicUsize) -> usize {
    n.load(Ordering::SeqCst)
}

/// Waits until `done` holds, polling; panics after [`PATIENCE`].
fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A manager running on its own thread; its action batches arrive on `actions`.
struct Spawned {
    actions: mpsc::Receiver<(Instant, Vec<ManagerAction>)>,
    stats: std::sync::Arc<ManagerStats>,
    events: mpsc::Sender<PluginEvents>,
    _thread: ManagerThread,
}

fn spawn(manager: PluginManager) -> Spawned {
    let stats = manager.stats();
    let events = manager.events();
    let (tx, actions) = mpsc::channel();
    let thread = manager
        .spawn(move |batch| {
            let _ = tx.send((Instant::now(), batch));
        })
        .unwrap();
    Spawned {
        actions,
        stats,
        events,
        _thread: thread,
    }
}

impl Spawned {
    fn next(&self) -> (Instant, Vec<ManagerAction>) {
        self.actions
            .recv_timeout(PATIENCE)
            .expect("the manager sends actions")
    }

    fn next_actions(&self) -> Vec<ManagerAction> {
        self.next().1
    }

    /// The batch of the initial scan.
    fn scanned(&self) -> Vec<ManagerAction> {
        self.next_actions()
    }

    fn assert_quiet(&self, wait: Duration) {
        if let Ok((_, batch)) = self.actions.recv_timeout(wait) {
            panic!("unexpected actions {:?}", summary(&batch));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// A core driven as the daemon drives it: actions applied on "the UI thread" (the test thread).

struct Live {
    core: Core,
    rx: async_channel::Receiver<CoreEvent>,
    unsolicited: Vec<Effect>,
    /// The `plugins` review module, when installed: `Reviews` actions go to it.
    plugins: Option<PluginsHandle>,
    /// The module instance of the last action answer.
    last_instance: u64,
    _data: tempfile::TempDir,
}

fn live() -> Live {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(&data.path().join("core.db")).unwrap();
    let mut core = Core::new(store, RankWeights::default(), now()).unwrap();
    core.start_modules();
    let rx = core.events();
    Live {
        core,
        rx,
        unsolicited: Vec::new(),
        plugins: None,
        last_instance: 0,
        _data: data,
    }
}

impl Live {
    /// Applies `actions` as the daemon does; waits for a registered plugin's `init` answer.
    fn apply(&mut self, actions: Vec<ManagerAction>) {
        let mut registered = 0;
        for action in actions {
            match action {
                ManagerAction::Register(m) => {
                    self.core.register(m);
                    registered += 1;
                }
                ManagerAction::Unregister(id) => self.core.unregister(&id),
                ManagerAction::Reviews(reviews) => {
                    if let Some(plugins) = &self.plugins {
                        plugins.set_reviews(reviews);
                    }
                }
                ManagerAction::Home(home) => {
                    self.core.set_home(home);
                }
                // The settings window's business, not the core's.
                ManagerAction::Catalog(_) => {}
            }
        }
        self.core.refresh(now());
        for _ in 0..registered {
            self.until(|u| matches!(u, CoreUpdate::ModuleEffects { .. }));
        }
    }

    fn update(&mut self) -> CoreUpdate {
        let update = self.core.apply(wait_event(&self.rx), now());
        if let CoreUpdate::ModuleEffects { effects, .. } = &update {
            self.unsolicited.extend(effects.iter().cloned());
        }
        update
    }

    fn until(&mut self, mut done: impl FnMut(&CoreUpdate) -> bool) {
        loop {
            if done(&self.update()) {
                return;
            }
        }
    }

    /// Runs `action` of item `id`; for a view it pushes, returns the view id and the module
    /// instance that pushed it, which the UI keeps with the view.
    fn push(&mut self, id: &str, action: &str) -> (String, u64) {
        let effects = self.run(id, action).unwrap();
        (pushed_view(&effects), self.last_instance)
    }

    fn run(&mut self, id: &str, action: &str) -> Result<Vec<Effect>, String> {
        let request = self
            .core
            .request_action(&ItemId::parse(id).unwrap(), action, "")
            .map_err(|e| e.to_string())?;
        loop {
            match self.update() {
                CoreUpdate::ActionDone {
                    request: r,
                    effects,
                    module_instance,
                    ..
                } if r == request => {
                    self.last_instance = module_instance;
                    return Ok(effects);
                }
                CoreUpdate::ActionFailed {
                    request: r,
                    message,
                } if r == request => return Err(message),
                _ => {}
            }
        }
    }

    /// Sends `event` to view `view` of kitchen-sink, pushed by module instance `instance`.
    fn view_event(
        &mut self,
        (view, instance): &(String, u64),
        event: ViewEvent,
    ) -> Result<Vec<Effect>, String> {
        self.view_event_of(ID, *instance, view, event)
    }

    fn view_event_of(
        &mut self,
        provider: &str,
        instance: u64,
        view: &str,
        event: ViewEvent,
    ) -> Result<Vec<Effect>, String> {
        let request = self
            .core
            .send_view_event(provider, instance, &ViewId(view.into()), event)
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

    /// Whether the item `id` is indexed now.
    fn indexed(&mut self, id: &str) -> bool {
        let id = ItemId::parse(id).unwrap();
        self.core.primary_action(&id).is_some()
    }

    /// Applies core events until `done` holds.
    fn wait(&mut self, what: &str, mut done: impl FnMut(&mut Live) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            if let Ok(ev) = self.rx.try_recv() {
                self.core.apply(ev, now());
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    /// Waits for an unsolicited toast titled `title`.
    fn toast(&mut self, title: &str) -> Toast {
        loop {
            let found = self
                .unsolicited
                .iter()
                .position(|e| matches!(e, Effect::Toast(t) if t.title == title));
            if let Some(at) = found {
                let Effect::Toast(toast) = self.unsolicited.remove(at) else {
                    unreachable!()
                };
                return toast;
            }
            self.update();
        }
    }
}

fn query(text: &str) -> ViewEvent {
    ViewEvent::QueryChanged {
        revision: 1,
        text: text.into(),
        readings: vec![],
    }
}

fn pushed_view(effects: &[Effect]) -> String {
    match effects {
        [Effect::PushView(View::Detail(view))] => view.id.0.clone(),
        other => panic!("expected one detail view, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Discovery and the gate

#[test]
fn scan_registers_approved_plugin() {
    let env = env();
    env.install();
    env.approve();
    let mut manager = env.manager();
    let stats = manager.stats();
    assert_eq!(summary(&manager.scan()), ["register:kitchen-sink"]);
    assert_eq!(count(&stats.compiles), 1);
    // Nothing changed: the running instance stays, nothing is compiled again.
    assert!(manager.scan().is_empty());
    assert_eq!(count(&stats.compiles), 1);
}

#[test]
fn scan_reviews_new_plugin() {
    let env = env();
    let dir = env.install();
    let mut manager = env.manager();
    let stats = manager.stats();
    let actions = manager.scan();
    assert_eq!(summary(&actions), ["reviews:kitchen-sink"]);
    let ManagerAction::Reviews(reviews) = &actions[0] else {
        unreachable!()
    };
    let package = Package::read(&dir).unwrap();
    assert_eq!(reviews[0].reason, ReviewReason::New);
    assert_eq!(reviews[0].content_hash, package.content_hash);
    // Nothing of an unapproved plugin is compiled.
    assert_eq!(count(&stats.compiles), 0);
    // The pending review stays as long as the content does: no new action.
    assert!(manager.scan().is_empty());
}

#[test]
fn plugin_with_a_native_module_id_is_rejected() {
    let env = env();
    env.install();
    env.approve();
    // Not one of the manifest's reserved ids: only the manager's own check refuses it.
    let mut manager = env
        .manager()
        .with_native_ids(HashSet::from([ID.to_string(), "apps".to_string()]));
    let stats = manager.stats();
    assert!(summary(&manager.scan()).is_empty());
    assert_eq!(count(&stats.compiles), 0);
    assert_eq!(count(&stats.failures_logged), 1);
}

#[test]
fn approved_event_gates_again_and_loads() {
    let env = env();
    env.install();
    let mut manager = env.manager();
    assert_eq!(summary(&manager.scan()), ["reviews:kitchen-sink"]);
    let content_hash = env.approve();
    let actions = manager.on_event(PluginEvents::Approved {
        id: ID.into(),
        content_hash,
    });
    assert_eq!(summary(&actions), ["register:kitchen-sink", "reviews:"]);
}

#[test]
fn approval_of_content_that_changed_since_asks_again() {
    let env = env();
    env.install();
    let mut manager = env.manager();
    manager.scan();
    // The user allowed what the prompt showed; meanwhile the folder got content that asks
    // for more. The event's hash is not taken on trust: the folder is read and gated again.
    let content_hash = env.approve();
    let toml = env.folder().join("plugin.toml");
    let text = std::fs::read_to_string(&toml).unwrap();
    std::fs::write(&toml, format!("{text}http = [\"example.com\"]\n")).unwrap();
    let actions = manager.on_event(PluginEvents::Approved {
        id: ID.into(),
        content_hash,
    });
    assert_eq!(summary(&actions), ["reviews:kitchen-sink"]);
    let ManagerAction::Reviews(reviews) = &actions[0] else {
        unreachable!()
    };
    assert_eq!(reviews[0].reason, ReviewReason::MorePermissions);
    assert_ne!(reviews[0].content_hash, content_hash);
}

/// Ruling AJ: an in-place update that starts seeing what the user types (`search`), or that
/// changes the name or author, is not taken silently: the plugin is unloaded and reviewed.
#[test]
fn an_update_that_adds_search_or_renames_asks_again() {
    type Edit = fn(String) -> String;
    let edits: [(&str, Edit, ReviewReason); 3] = [
        (
            "search added",
            |t| format!("{t}search = true\n"),
            ReviewReason::MorePermissions,
        ),
        (
            "renamed",
            |t| t.replacen("name = \"Kitchen Sink\"", "name = \"Plugins\"", 1),
            ReviewReason::Renamed,
        ),
        (
            "author added",
            |t| {
                t.replacen(
                    "api = \"0.1.0\"\n",
                    "api = \"0.1.0\"\nauthor = \"tomakast\"\n",
                    1,
                )
            },
            ReviewReason::Renamed,
        ),
    ];
    for (what, edit, reason) in edits {
        let env = env();
        env.install();
        // Approved without `search`.
        env.edit_toml(|t| t.replacen("search = true\n", "", 1));
        env.approve();
        let mut manager = env.manager();
        assert_eq!(
            summary(&manager.scan()),
            ["register:kitchen-sink"],
            "{what}"
        );
        env.edit_toml(edit);
        let actions = manager.folder_changed(ID);
        assert_eq!(
            summary(&actions),
            ["unregister:kitchen-sink", "reviews:kitchen-sink"],
            "{what}"
        );
        let ManagerAction::Reviews(reviews) = &actions[1] else {
            unreachable!()
        };
        assert_eq!(reviews[0].reason, reason, "{what}");
    }
}

#[test]
fn denied_event_drops_the_review() {
    let env = env();
    env.install();
    let mut manager = env.manager();
    let stats = manager.stats();
    manager.scan();
    let content_hash = env.decide(GrantDecision::Deny);
    let actions = manager.on_event(PluginEvents::Denied {
        id: ID.into(),
        content_hash,
    });
    assert_eq!(summary(&actions), ["reviews:"]);
    assert_eq!(count(&stats.compiles), 0);
}

/// Ruling AR (replacing the startup half of Ruling AG): the startup scan never deletes a grant
/// or a plugin's data. A folder that is missing at startup may be on a volume that is not
/// mounted yet; only a removal seen while running uninstalls a plugin.
#[test]
fn startup_scan_keeps_grants_and_data_of_missing_folders() {
    let env = env();
    env.install();
    env.approve();
    env.db.kv_set(ID, "k", b"v", 1).unwrap();
    std::fs::remove_dir_all(env.folder()).unwrap();
    assert!(summary(&env.manager().scan()).is_empty());
    assert!(env.db.grant(ID).unwrap().is_some());
    assert_eq!(env.db.kv_get(ID, "k").unwrap().as_deref(), Some(&b"v"[..]));
    // An empty plugins folder (Minor 4) keeps every grant too.
    std::fs::create_dir_all(&env.paths.plugins_dir).unwrap();
    assert!(summary(&env.manager().scan()).is_empty());
    assert!(env.db.grant(ID).unwrap().is_some());
}

#[test]
fn link_policy_follows_loaded_plugins() {
    let env = env();
    env.install();
    env.approve();
    let policy = PluginLinkPolicy::new(HashSet::from(["apps".to_string()]));
    let mut manager = env.manager().with_link_policy(policy.clone());
    assert!(!policy.may_open_links(ID));
    manager.scan();
    // kitchen-sink holds the `open` permission.
    assert!(policy.may_open_links(ID));
    std::fs::remove_dir_all(env.folder()).unwrap();
    assert_eq!(
        summary(&manager.folder_changed(ID)),
        ["unregister:kitchen-sink"]
    );
    assert!(!policy.may_open_links(ID));
    assert!(policy.may_open_links("apps"));
}

#[test]
fn a_broken_report_of_a_replaced_instance_is_ignored() {
    let env = env();
    env.install();
    env.approve();
    let mut manager = env.manager();
    manager.scan();
    let actions = manager.on_event(PluginEvents::Broken {
        id: ID.into(),
        content_hash: [7; 32],
        reason: "late".into(),
    });
    assert!(actions.is_empty(), "{:?}", summary(&actions));
}

/// The whole approval path, as the daemon wires it: the review module reports to the
/// manager, the user allows the plugin in the review view, the manager gates it again and
/// loads it, and its review goes away.
#[test]
fn allowing_in_the_review_view_loads_the_plugin() {
    let env = env();
    env.install();
    let spawned = spawn(env.manager());
    let mut live = live();
    let clock = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
    let read = clock.clone();
    live.plugins = Some(PluginsModule::install_with_clock(
        &mut live.core,
        PluginDb::open(&env.paths.db_file).unwrap(),
        spawned.events.clone(),
        std::sync::Arc::new(move || *read.lock().unwrap()),
    ));
    let scanned = spawned.scanned();
    assert_eq!(summary(&scanned), ["reviews:kitchen-sink"]);
    live.apply(scanned);
    live.wait("the review item", |l| {
        l.indexed("plugins/review-kitchen-sink")
    });
    assert!(!live.indexed("kitchen-sink/hello"));

    let (view, instance) = live.push("plugins/review-kitchen-sink", "review");
    *clock.lock().unwrap() += MIN_READ_TIME + Duration::from_millis(100);
    let chosen = ViewEvent::ActionChosen {
        row_id: view.clone(),
        action_id: "allow".into(),
    };
    live.view_event_of("plugins", instance, &view, chosen)
        .unwrap();

    let actions = spawned.next_actions();
    assert_eq!(summary(&actions), ["register:kitchen-sink", "reviews:"]);
    live.apply(actions);
    live.wait("the plugin's command", |l| l.indexed("kitchen-sink/hello"));
    live.wait("the review to go", |l| {
        !l.indexed("plugins/review-kitchen-sink")
    });
    assert!(live.run("kitchen-sink/hello", "run").is_ok());
}

// ---------------------------------------------------------------------------------------------
// The watcher

/// Review Focus: a folder still being copied is read only after 500 ms of quiet.
#[test]
fn half_copied_folder_waits_for_quiet() {
    let env = env();
    let toml = std::fs::read(source().folder.join("plugin.toml")).unwrap();
    let wasm = std::fs::read(source().folder.join("plugin.wasm")).unwrap();
    let spawned = spawn(env.manager());
    assert!(summary(&spawned.scanned()).is_empty());
    // Approved beforehand (after the startup scan, which forgets grants without a folder),
    // so that the one read loads the plugin.
    let package = {
        let staging = env.home.path().join("staging").join(ID);
        copy_dir(&source().folder, &staging);
        Package::read(&staging).unwrap()
    };
    env.grant(
        &package.manifest,
        package.content_hash,
        GrantDecision::Allow,
    );
    let dir = env.folder();
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), &toml).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let half = wasm.len() / 2;
    std::fs::write(dir.join("plugin.wasm"), &wasm[..half]).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("plugin.wasm"))
            .unwrap();
        file.write_all(&wasm[half..]).unwrap();
    }
    let written = Instant::now();
    assert_eq!(
        count(&spawned.stats.reads),
        0,
        "read before the folder was quiet"
    );

    let (at, actions) = spawned.next();
    assert_eq!(summary(&actions), ["register:kitchen-sink"]);
    assert!(
        at >= written + Duration::from_millis(500),
        "loaded {:?} after the last write",
        at - written
    );
    assert_eq!(count(&spawned.stats.reads), 1);
    assert_eq!(count(&spawned.stats.compiles), 1);
}

#[test]
fn broken_wasm_logged_once_and_retried_on_change() {
    let env = env();
    let dir = env.install();
    std::fs::write(dir.join("plugin.wasm"), b"not a component").unwrap();
    env.approve();
    let spawned = spawn(env.manager());
    let stats = spawned.stats.clone();
    assert!(summary(&spawned.scanned()).is_empty());
    assert_eq!(
        (count(&stats.compiles), count(&stats.failures_logged)),
        (1, 1)
    );

    // No retry loop.
    spawned.assert_quiet(Duration::from_millis(1200));
    assert_eq!(count(&stats.compiles), 1);

    // The same bytes written again: a file event, so the same content is tried again, but
    // its failure, the one logged last, is not logged again.
    std::fs::write(dir.join("plugin.wasm"), b"not a component").unwrap();
    wait_until("the retry of the same content", || {
        count(&stats.compiles) == 2
    });

    // A change is retried, and its failure logged.
    std::fs::write(dir.join("plugin.wasm"), b"still not a component").unwrap();
    wait_until("the retry of the change", || count(&stats.compiles) == 3);
    wait_until("the second log line", || count(&stats.failures_logged) == 2);
    // The other failure text is a change of the catalog, and only that.
    let batch = spawned.next_actions();
    assert!(summary(&batch).is_empty() && catalogs(&batch).len() == 1);
    spawned.assert_quiet(Duration::from_millis(100));
    assert_eq!(count(&stats.failures_logged), 2);

    // A rescan is no change: the failed content is not tried again.
    let compiles = count(&stats.compiles);
    spawned
        .events
        .send(PluginEvents::Approved {
            id: ID.into(),
            content_hash: Package::read(&dir).unwrap().content_hash,
        })
        .unwrap();
    spawned.assert_quiet(Duration::from_millis(300));
    assert_eq!(count(&stats.compiles), compiles);

    // A fixed wasm loads (an in-place update with the same permissions).
    std::fs::copy(source().folder.join("plugin.wasm"), dir.join("plugin.wasm")).unwrap();
    assert_eq!(summary(&spawned.next_actions()), ["register:kitchen-sink"]);
}

/// A compile child that fails while `fail` exists, else runs the real one: the same content
/// can fail once and then succeed.
#[cfg(unix)]
fn flaky_compiler(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let fail = dir.join("fail");
    let script = dir.join("flaky-compile");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ -e '{}' ] && exit 1\nexec '{}' \"$@\"\n",
            fail.display(),
            compile_exe().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&fail, b"").unwrap();
    (script, fail)
}

/// A failure that was not the content's fault (here, the compile child) is retried on the
/// next file event in the folder, even with the same content.
#[cfg(unix)]
#[test]
fn same_content_is_retried_after_a_file_event() {
    let mut env = env();
    // An empty cache, so that the compile child runs.
    let cache = env.home.path().join("own-cache");
    env.paths.plugin_cache_dir = cache;
    env.install();
    env.approve();
    let (script, fail) = flaky_compiler(env.home.path());
    let spawned = spawn(env.manager().with_compile_exe(script));
    assert!(summary(&spawned.scanned()).is_empty());
    assert_eq!(
        (
            count(&spawned.stats.compiles),
            count(&spawned.stats.failures_logged)
        ),
        (1, 1)
    );

    std::fs::remove_file(fail).unwrap();
    // The same bytes again: only the event is new.
    let toml = env.folder().join("plugin.toml");
    std::fs::write(&toml, std::fs::read(&toml).unwrap()).unwrap();
    assert_eq!(summary(&spawned.next_actions()), ["register:kitchen-sink"]);
    assert_eq!(count(&spawned.stats.compiles), 2);
}

/// Ruling AI: only a folder that is not there is a removal.
#[cfg(unix)]
#[test]
fn an_unreadable_folder_unloads_but_keeps_the_grant() {
    use std::os::unix::fs::PermissionsExt;
    let env = env();
    let dir = env.install();
    env.approve();
    let mut manager = env.manager();
    assert_eq!(summary(&manager.scan()), ["register:kitchen-sink"]);

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    let actions = manager.folder_changed(ID);
    let rescan = manager.scan();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(summary(&actions), ["unregister:kitchen-sink"]);
    assert!(rescan.is_empty(), "{:?}", summary(&rescan));
    assert!(env.db.grant(ID).unwrap().is_some());

    // Readable again: it loads without a new review.
    assert_eq!(
        summary(&manager.folder_changed(ID)),
        ["register:kitchen-sink"]
    );

    // A dangling link in place of the folder is not a removal either.
    std::fs::remove_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(env.home.path().join("nowhere"), &dir).unwrap();
    assert_eq!(
        summary(&manager.folder_changed(ID)),
        ["unregister:kitchen-sink"]
    );
    manager.scan();
    assert!(env.db.grant(ID).unwrap().is_some());
}

/// Ruling AI: the plugins folder itself going away unloads everything and wipes no grant.
#[test]
fn a_vanished_plugins_folder_keeps_grants() {
    let env = env();
    env.install();
    env.approve();
    let mut manager = env.manager();
    manager.scan();
    std::fs::remove_dir_all(&env.paths.plugins_dir).unwrap();
    // The folder's own event may come first, or the rescan.
    assert_eq!(
        summary(&manager.folder_changed(ID)),
        ["unregister:kitchen-sink"]
    );
    assert!(manager.scan().is_empty());
    assert!(env.db.grant(ID).unwrap().is_some());
}

/// A rescan forgets the failures of folders that are gone, so content copied in again is
/// tried again.
#[test]
fn scan_forgets_failures_of_removed_folders() {
    let env = env();
    let dir = env.install();
    std::fs::write(dir.join("plugin.wasm"), b"not a component").unwrap();
    env.approve();
    let mut manager = env.manager();
    let stats = manager.stats();
    manager.scan();
    assert_eq!(count(&stats.compiles), 1);

    std::fs::remove_dir_all(&dir).unwrap();
    manager.scan();
    env.install();
    std::fs::write(dir.join("plugin.wasm"), b"not a component").unwrap();
    env.approve();
    manager.scan();
    assert_eq!(count(&stats.compiles), 2);
}

/// A stray file next to the plugin folders is no plugin and no failure.
#[test]
fn a_stray_file_in_the_plugins_folder_is_ignored() {
    let env = env();
    std::fs::write(env.paths.plugins_dir.join("README.md"), b"hi").unwrap();
    let mut manager = env.manager();
    let stats = manager.stats();
    assert!(summary(&manager.scan()).is_empty());
    assert!(manager.folder_changed("README.md").is_empty());
    assert_eq!(count(&stats.failures_logged), 0);
}

#[test]
fn removed_folder_unregisters() {
    let env = env();
    env.install();
    env.approve();
    let spawned = spawn(env.manager());
    assert_eq!(summary(&spawned.scanned()), ["register:kitchen-sink"]);
    env.db.kv_set(ID, "k", b"v", 1).unwrap();
    env.db.kv_set("other", "k", b"v", 1).unwrap();

    std::fs::remove_dir_all(env.folder()).unwrap();
    assert_eq!(
        summary(&spawned.next_actions()),
        ["unregister:kitchen-sink"]
    );
    // Ruling AG: the grant goes with the folder, so the same plugin copied in again is
    // reviewed again. Ruling AR: its stored data goes too, so a later plugin with the same
    // id does not inherit it.
    assert_eq!(env.db.grant(ID).unwrap(), None);
    assert_eq!(env.db.kv_usage(ID).unwrap(), 0);
    assert!(
        env.db.kv_get("other", "k").unwrap().is_some(),
        "another plugin's data"
    );
    env.install();
    assert_eq!(summary(&spawned.next_actions()), ["reviews:kitchen-sink"]);
}

/// Review Focus: an update while one of the plugin's views is open.
#[test]
fn reload_replaces_instance_and_old_view_events_error() {
    let env = env();
    env.install();
    env.approve();
    let spawned = spawn(env.manager());
    let stats = spawned.stats.clone();
    let mut live = live();
    live.apply(spawned.scanned());
    let old_view = live.push("kitchen-sink/indexed", "echo-item");
    assert!(live.view_event(&old_view, query("a")).is_ok());

    // An in-place update: same wasm (a cache hit), same permissions.
    env.bump_version("0.1.1");
    let actions = spawned.next_actions();
    assert_eq!(
        summary(&actions),
        ["unregister:kitchen-sink", "register:kitchen-sink"]
    );
    assert_eq!(count(&stats.compiles), 2);
    live.apply(actions);

    // The old view is still on the UI's stack; its events go nowhere.
    assert_eq!(
        live.view_event(&old_view, query("b")),
        Err(RELOADED.to_string())
    );
    // A view of the new instance works.
    // The new instance pushes a view with the same id (`echo`) above the old one.
    let new_view = live.push("kitchen-sink/indexed", "echo-item");
    assert_eq!(new_view.0, old_view.0);
    assert_ne!(new_view.1, old_view.1);
    match live.view_event(&new_view, query("c")).unwrap().as_slice() {
        [Effect::Toast(toast)] => assert_eq!(toast.message.as_deref(), Some("query-changed 1 c")),
        other => panic!("{other:?}"),
    }
    // The old one's events still go nowhere, also once the new one is closed.
    assert_eq!(
        live.view_event(&old_view, query("d")),
        Err(RELOADED.to_string())
    );
    assert!(live.view_event(&new_view, ViewEvent::Closed).is_ok());
    assert_eq!(
        live.view_event(&old_view, query("e")),
        Err(RELOADED.to_string())
    );
}

#[test]
fn broken_plugin_is_unloaded_until_its_folder_changes() {
    let env = env();
    env.install();
    env.approve();
    let spawned = spawn(env.manager());
    let mut live = live();
    live.apply(spawned.scanned());
    assert!(live.run("kitchen-sink/indexed", "panic").is_err());
    assert_eq!(
        summary(&spawned.next_actions()),
        ["unregister:kitchen-sink"]
    );

    // Approving it again changes nothing: the same content stays unloaded.
    spawned
        .events
        .send(PluginEvents::Approved {
            id: ID.into(),
            content_hash: Package::read(&env.folder()).unwrap().content_hash,
        })
        .unwrap();
    spawned.assert_quiet(Duration::from_millis(300));

    env.bump_version("0.1.2");
    assert_eq!(summary(&spawned.next_actions()), ["register:kitchen-sink"]);
}

// ---------------------------------------------------------------------------------------------
// Preferences

#[test]
fn preferences_change_delivers_event() {
    let env = env();
    env.install();
    env.approve();
    std::fs::write(
        &env.paths.config_file,
        "[plugins.kitchen-sink]\ngreeting = \"hi\"\n",
    )
    .unwrap();
    let settings = tomakast_core::Settings::load(&env.paths.config_file).unwrap();
    let spawned = spawn(env.manager_with(settings.plugin_preferences));
    let stats = spawned.stats.clone();
    let mut live = live();
    live.apply(spawned.scanned());

    std::fs::write(
        &env.paths.config_file,
        "[plugins.kitchen-sink]\ngreeting = \"hello there\"\nloud = true\n",
    )
    .unwrap();
    let toast = live.toast("preferences-changed");
    assert_eq!(
        toast.message.as_deref(),
        Some("greeting=Some(\"hello there\")")
    );
    assert_eq!(count(&stats.preferences_delivered), 1);
    assert_eq!(count(&stats.config_reads), 1);

    // A change to another plugin's table is not delivered to this one.
    std::fs::write(
        &env.paths.config_file,
        "[plugins.kitchen-sink]\ngreeting = \"hello there\"\nloud = true\n\
         [plugins.other]\nx = \"y\"\n",
    )
    .unwrap();
    wait_until("the second config read", || count(&stats.config_reads) == 2);
    // A broken config keeps the old values.
    std::fs::write(&env.paths.config_file, "[plugins.kitchen-sink\n").unwrap();
    wait_until("the third config read", || count(&stats.config_reads) == 3);
    assert_eq!(count(&stats.preferences_delivered), 1);
    let effects = live.run("kitchen-sink/indexed", "prefs");
    // `prefs` is not an action of the indexed item's own list, but kitchen-sink answers any
    // action id; the detail view lists what `preferences.get` returns now.
    let effects = effects.unwrap();
    let [Effect::PushView(View::Detail(view))] = effects.as_slice() else {
        panic!("{effects:?}")
    };
    let greeting = view.metadata.iter().find(|(k, _)| k == "greeting").unwrap();
    assert_eq!(greeting.1, "Some(\"hello there\")");
}

/// A change of `config.toml` sends the new Home list; a broken file sends none (the Home on
/// screen stays). Needs no plugin.
#[test]
fn a_config_change_sends_the_new_home_list() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::with(home.path().join("config.toml"), home.path().to_owned());
    std::fs::create_dir_all(&paths.plugins_dir).unwrap();
    drop(Store::open(&paths.db_file).unwrap());
    // There before the watch starts, so the change below is an edit, as a user's is.
    std::fs::write(&paths.config_file, "").unwrap();
    let manager = PluginManager::new(
        &paths,
        HostEngine::new().unwrap(),
        PluginDb::open(&paths.db_file).unwrap(),
        BTreeMap::new(),
    );
    let spawned = spawn(manager);
    let stats = spawned.stats.clone();
    // The startup scan of an empty plugins folder; it sends no Home.
    let scanned = spawned.scanned();
    assert!(
        !scanned.iter().any(|a| matches!(a, ManagerAction::Home(_))),
        "{:?}",
        summary(&scanned)
    );

    std::fs::write(
        &paths.config_file,
        "[[home.widgets]]\nwidget = \"weather/now\"\nsize = \"quarter\"\ncity = \"Barcelona\"\n\n\
         [[home.widgets]]\nwidget = \"calc/answer\"\n",
    )
    .unwrap();
    let batch = spawned.next_actions();
    let [ManagerAction::Home(entries)] = batch.as_slice() else {
        panic!("{:?}", summary(&batch));
    };
    assert_eq!(
        *entries,
        [
            HomeEntry {
                widget: WidgetKey::new("weather", "now"),
                size: Some(WidgetSize::Quarter),
                params: params([("city".to_string(), "Barcelona".to_string())]),
            },
            HomeEntry {
                widget: WidgetKey::new("calc", "answer"),
                size: None,
                params: params([]),
            },
        ]
    );

    std::fs::write(&paths.config_file, "[[home.widgets]\n").unwrap();
    wait_until("the broken config read", || count(&stats.config_reads) == 2);
    spawned.assert_quiet(Duration::from_millis(300));
}

/// Installs kitchen-sink as folder `name` (its id and name are `name`); `decision` stores a
/// grant for it as the review would.
fn install_as(env: &Env, name: &str, decision: Option<GrantDecision>) {
    let dir = env.paths.plugins_dir.join(name);
    copy_dir(&source().folder, &dir);
    let toml = std::fs::read_to_string(dir.join("plugin.toml"))
        .unwrap()
        .replacen("id = \"kitchen-sink\"", &format!("id = \"{name}\""), 1)
        .replacen("name = \"Kitchen Sink\"", &format!("name = \"{name}\""), 1);
    std::fs::write(dir.join("plugin.toml"), toml).unwrap();
    let Some(decision) = decision else { return };
    let package = Package::read(&dir).unwrap();
    env.db
        .set_grant(&Grant {
            plugin_id: name.into(),
            content_hash: package.content_hash,
            permissions: package.manifest.permissions.clone(),
            name: name.into(),
            author: None,
            widgets: package
                .manifest
                .widgets
                .iter()
                .map(|w| WidgetGrant {
                    id: w.id.clone(),
                    title: w.title.clone(),
                })
                .collect(),
            decision,
            decided_at: 1,
            updated_at: 1,
        })
        .unwrap();
}

#[test]
fn catalog_lists_running_review_and_denied() {
    let env = env();
    install_as(&env, "charlie", Some(GrantDecision::Deny));
    install_as(&env, "alpha", Some(GrantDecision::Allow));
    install_as(&env, "bravo", None);
    let mut manager = env.manager();
    let actions = manager.scan();
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{actions:?}");
    };
    assert_eq!(catalog.unavailable, None);
    let listed: Vec<(&str, &PluginState)> = catalog
        .plugins
        .iter()
        .map(|p| (p.name.as_str(), &p.state))
        .collect();
    assert_eq!(
        listed,
        [
            ("alpha", &PluginState::Running),
            ("bravo", &PluginState::Review),
            ("charlie", &PluginState::Denied),
        ]
    );
    // What the manifest declares comes along, for every state.
    let denied = &catalog.plugins[2];
    assert_eq!(denied.id, "charlie");
    assert_eq!(denied.version, "0.1.0");
    assert_eq!(denied.commands[0].id, "hello");
    assert_eq!(denied.commands[0].title, "Say hello");
    assert_eq!(denied.preferences[0].key, "greeting");
    assert_eq!(
        denied.preferences[0].kind,
        tomakast_core::settings_schema::ValueKind::Text
    );
    assert_eq!(denied.preferences[0].default.as_deref(), Some("hello"));
    assert_eq!(denied.widgets[0].id, "static");
    // The params' titles come along beside the widgets, for the settings window.
    assert_eq!(denied.param_title("echo", "text"), Some("Text"));
    assert_eq!(denied.param_title("echo", "other"), None);
}

#[test]
fn broken_manifest_is_in_the_catalog() {
    let env = env();
    let dir = env.paths.plugins_dir.join("oops");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), "id = 1\n").unwrap();
    std::fs::write(dir.join("plugin.wasm"), b"").unwrap();
    let mut manager = env.manager();
    let actions = manager.scan();
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{actions:?}");
    };
    let [plugin] = catalog.plugins.as_slice() else {
        panic!("{catalog:?}");
    };
    assert_eq!((plugin.id.as_str(), plugin.name.as_str()), ("oops", "oops"));
    assert_eq!(plugin.version, "");
    assert!(
        matches!(&plugin.state, PluginState::ManifestError(m) if !m.is_empty()),
        "{:?}",
        plugin.state
    );
}

#[test]
fn a_plugin_that_does_not_compile_is_failed_in_the_catalog() {
    let env = env();
    let dir = env.install();
    std::fs::write(dir.join("plugin.wasm"), b"not a component").unwrap();
    env.approve();
    let mut manager = env.manager();
    let actions = manager.scan();
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{actions:?}");
    };
    assert!(
        matches!(&catalog.plugins[0].state, PluginState::Failed(m) if m.contains("compile")),
        "{:?}",
        catalog.plugins[0].state
    );
}

#[test]
fn catalog_is_sent_only_when_it_changes() {
    let env = env();
    env.install();
    let mut manager = env.manager();
    assert_eq!(catalogs(&manager.scan()).len(), 1);
    assert!(catalogs(&manager.folder_changed(ID)).is_empty());
    assert!(catalogs(&manager.scan()).is_empty());
    // The user's decision changes the state: a new catalog.
    let content_hash = env.approve();
    let actions = manager.on_event(PluginEvents::Approved {
        id: ID.into(),
        content_hash,
    });
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{actions:?}");
    };
    assert_eq!(catalog.plugins[0].state, PluginState::Running);
    // A removed folder leaves the catalog.
    std::fs::remove_dir_all(env.folder()).unwrap();
    let actions = manager.folder_changed(ID);
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{actions:?}");
    };
    assert!(catalog.plugins.is_empty());
}

/// What the running plugin says an unset preference stands for reaches the catalog; a report
/// of another instance, or of a key it does not declare, changes nothing.
#[test]
fn a_reported_default_reaches_the_catalog() {
    let env = env();
    env.install();
    let content_hash = env.approve();
    let mut manager = env.manager();
    manager.scan();
    let report = |hash: [u8; 32], key: &str, value: &str| PluginEvents::PreferenceDefault {
        id: ID.into(),
        content_hash: hash,
        key: key.into(),
        value: value.into(),
    };
    let actions = manager.on_event(report(content_hash, "name", "Ann"));
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{:?}", summary(&actions));
    };
    let name = catalog.plugins[0]
        .preferences
        .iter()
        .find(|p| p.key == "name")
        .unwrap();
    assert_eq!(name.reported_default.as_deref(), Some("Ann"));
    assert_eq!(name.shown_default(), Some("Ann"));
    // The same again: no new catalog.
    assert!(catalogs(&manager.on_event(report(content_hash, "name", "Ann"))).is_empty());
    // An instance that no longer runs, and an undeclared key, change nothing.
    assert!(catalogs(&manager.on_event(report([7; 32], "name", "Bob"))).is_empty());
    assert!(catalogs(&manager.on_event(report(content_hash, "nope", "x"))).is_empty());
    // An empty value takes the told default back: the manifest's shows again.
    let actions = manager.on_event(report(content_hash, "name", ""));
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{:?}", summary(&actions));
    };
    let name = catalog.plugins[0]
        .preferences
        .iter()
        .find(|p| p.key == "name")
        .unwrap();
    assert_eq!(name.reported_default, None);
    assert_eq!(name.shown_default(), name.default.as_deref());
}

/// The first scan publishes a catalog even when nothing is installed: the settings window
/// waits for it before it offers to remove a plugin's settings.
#[test]
fn the_first_catalog_is_sent_even_when_empty() {
    let env = env();
    let mut manager = env.manager();
    let actions = manager.scan();
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{:?}", summary(&actions));
    };
    assert_eq!(*catalog, PluginCatalog::default());
    assert!(catalogs(&manager.scan()).is_empty());
}

/// A plugins folder that is there but cannot be listed is not an empty one: the catalog says
/// why there is no list.
#[cfg(unix)]
#[test]
fn an_unlistable_plugins_folder_makes_the_catalog_unavailable() {
    use std::os::unix::fs::PermissionsExt;
    let env = env();
    env.install();
    env.approve();
    let mut manager = env.manager();
    manager.scan();
    let dir = &env.paths.plugins_dir;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    let actions = manager.scan();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(summary(&actions), ["unregister:kitchen-sink"]);
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{:?}", summary(&actions));
    };
    assert!(catalog.plugins.is_empty());
    assert!(catalog.unavailable.is_some(), "{catalog:?}");

    // Listable again: the list is back.
    let actions = manager.scan();
    let [catalog] = catalogs(&actions)[..] else {
        panic!("{:?}", summary(&actions));
    };
    assert_eq!(catalog.unavailable, None);
    assert_eq!(catalog.plugins.len(), 1);
}

/// The state of plugin `id` in the last catalog among `actions`.
fn state_in(actions: &[ManagerAction], id: &str) -> Option<PluginState> {
    let catalog = catalogs(actions).pop()?;
    let plugin = catalog.plugins.iter().find(|p| p.id == id)?;
    Some(plugin.state.clone())
}

/// Writes `config.toml` with kitchen-sink's switch on or off.
fn switch(env: &Env, on: bool) {
    let text = if on {
        ""
    } else {
        "[enabled_plugins]\nkitchen-sink = false\n"
    };
    std::fs::write(&env.paths.config_file, text).unwrap();
}

#[test]
fn a_plugin_turned_off_at_start_is_neither_loaded_nor_reviewed() {
    let env = env();
    env.install();
    env.approve();
    install_as(&env, "bravo", None);
    let mut manager = env
        .manager()
        .with_disabled([ID.to_string(), "bravo".to_string()].into());
    let stats = manager.stats();
    let actions = manager.scan();
    // No register, and no review for the unapproved one either.
    assert!(summary(&actions).is_empty(), "{:?}", summary(&actions));
    assert_eq!(count(&stats.compiles), 0);
    assert_eq!(state_in(&actions, ID), Some(PluginState::Off));
    assert_eq!(state_in(&actions, "bravo"), Some(PluginState::Off));
    // What the manifest declares still reaches the settings window.
    let catalog = catalogs(&actions)[0];
    let plugin = catalog.plugins.iter().find(|p| p.id == ID).unwrap();
    assert_eq!(plugin.commands[0].id, "hello");
}

#[test]
fn a_switch_turns_a_plugin_off_and_on_without_a_new_review() {
    let env = env();
    env.install();
    env.approve();
    let mut manager = env.manager();
    let stats = manager.stats();
    assert_eq!(summary(&manager.scan()), ["register:kitchen-sink"]);

    switch(&env, false);
    let actions = manager.reload_config();
    assert_eq!(summary(&actions), ["unregister:kitchen-sink", "home:"]);
    assert_eq!(state_in(&actions, ID), Some(PluginState::Off));
    // Off is not denied: the approval stays as it was.
    let grant = env.db.grant(ID).unwrap().unwrap();
    assert_eq!(grant.decision, GrantDecision::Allow);
    // A rescan keeps it off.
    assert!(summary(&manager.scan()).is_empty());

    switch(&env, true);
    let actions = manager.reload_config();
    assert_eq!(summary(&actions), ["register:kitchen-sink", "home:"]);
    assert_eq!(state_in(&actions, ID), Some(PluginState::Running));
    assert_eq!(count(&stats.compiles), 2);
    // The same switches again change nothing.
    assert_eq!(summary(&manager.reload_config()), ["home:"]);
}

#[test]
fn an_off_plugin_does_not_start_on_approval_nor_a_denied_one_when_turned_on() {
    let env = env();
    env.install();
    install_as(&env, "charlie", Some(GrantDecision::Deny));
    let mut manager = env.manager();
    assert_eq!(summary(&manager.scan()), ["reviews:kitchen-sink"]);
    // Turned off while waiting for review: the launcher stops asking.
    switch(&env, false);
    assert_eq!(summary(&manager.reload_config()), ["reviews:", "home:"]);
    let content_hash = env.approve();
    let actions = manager.on_event(PluginEvents::Approved {
        id: ID.into(),
        content_hash,
    });
    assert!(summary(&actions).is_empty(), "{:?}", summary(&actions));
    // Approved meanwhile: on, it runs.
    switch(&env, true);
    assert_eq!(
        summary(&manager.reload_config()),
        ["register:kitchen-sink", "home:"]
    );

    // A denied plugin turned off and on again is still denied.
    std::fs::write(
        &env.paths.config_file,
        "[enabled_plugins]\ncharlie = false\n",
    )
    .unwrap();
    let actions = manager.reload_config();
    assert_eq!(state_in(&actions, "charlie"), Some(PluginState::Off));
    std::fs::write(&env.paths.config_file, "").unwrap();
    let actions = manager.reload_config();
    assert_eq!(summary(&actions), ["home:"]);
    assert_eq!(state_in(&actions, "charlie"), Some(PluginState::Denied));
}

#[test]
fn a_switch_of_a_plugin_that_is_not_installed_keeps_its_approval() {
    let env = env();
    install_as(&env, "ghost", Some(GrantDecision::Allow));
    std::fs::remove_dir_all(env.paths.plugins_dir.join("ghost")).unwrap();
    let mut manager = env.manager();
    manager.scan();
    for text in ["[enabled_plugins]\nghost = false\n", ""] {
        std::fs::write(&env.paths.config_file, text).unwrap();
        assert_eq!(summary(&manager.reload_config()), ["home:"]);
    }
    assert!(env.db.grant("ghost").unwrap().is_some());
}
