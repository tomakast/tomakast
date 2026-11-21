//! Hash-bound approval: the decision table, the gate the plugin manager runs before a plugin
//! may load, the `plugins` review module, and the link policy.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use support::harness::{ID, loaded, now};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreEvent, CoreUpdate, DetailView, Effect, Item, ItemId, ItemKind, LinkPolicy,
    RankWeights, ToastKind, View, ViewEvent, ViewId,
};
use tomakast_plugin_host::approval::{Decision, Gate, ReviewReason, check, decide};
use tomakast_plugin_host::plugins_module::{MIN_READ_TIME, PROVIDER, PluginsHandle, PluginsModule};
use tomakast_plugin_host::{
    Grant, GrantDecision, Manifest, Package, Permissions, PluginDb, PluginEvents, PluginLinkPolicy,
    WasmPlugin, WidgetGrant,
};

const REVIEW_ITEM: &str = "plugins/review-kitchen-sink";
const NEW_SUBTITLE: &str = "New plugin needs your approval";
const NEW_WIDGETS_SUBTITLE: &str = "Plugin update adds or renames tiles";
const MORE_SUBTITLE: &str = "Plugin update asks for more permissions";

// ---------------------------------------------------------------------------------------------
// decide: the rules as a table

const MANIFEST_HEAD: &str =
    "id = \"kitchen-sink\"\nname = \"Kitchen Sink\"\nversion = \"1\"\napi = \"0.1.0\"\n";

fn pure_package(permissions: &str, hash: u8) -> Package {
    let text = format!("{MANIFEST_HEAD}[permissions]\n{permissions}");
    Package {
        dir: PathBuf::from("/plugins/kitchen-sink"),
        manifest: Manifest::parse(&text, ID).unwrap(),
        wasm: Arc::from(Vec::new()),
        content_hash: [hash; 32],
        wasm_hash: [0; 32],
    }
}

fn authored_package(permissions: &str, hash: u8, author: &str) -> Package {
    let text = format!("{MANIFEST_HEAD}author = \"{author}\"\n[permissions]\n{permissions}");
    Package {
        manifest: Manifest::parse(&text, ID).unwrap(),
        ..pure_package(permissions, hash)
    }
}

fn perms(text: &str) -> Permissions {
    pure_package(text, 0).manifest.permissions
}

fn grant(decision: GrantDecision, hash: u8, permissions: &str) -> Grant {
    Grant {
        plugin_id: ID.into(),
        content_hash: [hash; 32],
        permissions: perms(permissions),
        name: "Kitchen Sink".into(),
        widgets: Vec::new(),
        author: None,
        decision,
        decided_at: 5,
        updated_at: 5,
    }
}

#[test]
fn decide_table() {
    use Decision::{Ask, Denied, Run};
    use GrantDecision::{Allow, Deny};
    const SOME: &str = "storage = true\nhttp = [\"*.example.org\"]\n";
    let rows: &[(&str, Option<Grant>, Package, Decision)] = &[
        ("no grant", None, pure_package(SOME, 1), Ask),
        (
            "allow, same hash",
            Some(grant(Allow, 1, SOME)),
            pure_package(SOME, 1),
            Run,
        ),
        (
            "allow grant, different hash, same permissions (in-place update)",
            Some(grant(Allow, 1, SOME)),
            pure_package(SOME, 2),
            Run,
        ),
        (
            "allow, new hash, fewer permissions",
            Some(grant(Allow, 1, SOME)),
            pure_package("storage = true\n", 2),
            Run,
        ),
        (
            "allow, new hash, narrower http",
            Some(grant(Allow, 1, SOME)),
            pure_package("http = [\"api.example.org\"]\n", 2),
            Run,
        ),
        (
            "allow, new hash, a new permission",
            Some(grant(Allow, 1, SOME)),
            pure_package(&format!("{SOME}clipboard = true\n"), 2),
            Ask,
        ),
        (
            "allow, new hash, a new http domain",
            Some(grant(Allow, 1, SOME)),
            pure_package(
                "storage = true\nhttp = [\"*.example.org\", \"example.com\"]\n",
                2,
            ),
            Ask,
        ),
        (
            "deny, same hash",
            Some(grant(Deny, 1, SOME)),
            pure_package(SOME, 1),
            Denied,
        ),
        (
            "deny, new hash",
            Some(grant(Deny, 1, SOME)),
            pure_package(SOME, 2),
            Ask,
        ),
        (
            "deny, new hash, fewer permissions",
            Some(grant(Deny, 1, SOME)),
            pure_package("", 2),
            Ask,
        ),
        (
            "allow api.example.org, update asks *.example.org",
            Some(grant(Allow, 1, "http = [\"api.example.org\"]\n")),
            pure_package("http = [\"*.example.org\"]\n", 2),
            Ask,
        ),
        (
            "allow, new hash, the update starts seeing what is typed (Ruling AJ)",
            Some(grant(Allow, 1, SOME)),
            pure_package(&format!("{SOME}search = true\n"), 2),
            Ask,
        ),
        (
            "allow with search, new hash, still search",
            Some(grant(Allow, 1, &format!("{SOME}search = true\n"))),
            pure_package(&format!("{SOME}search = true\n"), 2),
            Run,
        ),
        (
            "allow, new hash, same permissions, another name (Ruling AJ)",
            Some(Grant {
                name: "Old Name".into(),
                ..grant(Allow, 1, SOME)
            }),
            pure_package(SOME, 2),
            Ask,
        ),
        (
            "allow, new hash, same permissions, an author added (Ruling AJ)",
            Some(grant(Allow, 1, SOME)),
            authored_package(SOME, 2, "Someone"),
            Ask,
        ),
        (
            "allow, new hash, same permissions, the author removed (Ruling AJ)",
            Some(Grant {
                author: Some("Someone".into()),
                ..grant(Allow, 1, SOME)
            }),
            pure_package(SOME, 2),
            Ask,
        ),
        (
            "allow, new hash, same name and author",
            Some(Grant {
                author: Some("Someone".into()),
                ..grant(Allow, 1, SOME)
            }),
            authored_package(SOME, 2, "Someone"),
            Run,
        ),
        (
            "a grant stored before names were recorded (empty name), new hash",
            Some(Grant {
                name: String::new(),
                ..grant(Allow, 1, SOME)
            }),
            pure_package(SOME, 2),
            Ask,
        ),
        (
            "a renamed grant, same hash (the hash covers plugin.toml)",
            Some(Grant {
                name: "Old Name".into(),
                ..grant(Allow, 1, SOME)
            }),
            pure_package(SOME, 1),
            Run,
        ),
        (
            "a grant of another plugin",
            Some(Grant {
                plugin_id: "other".into(),
                ..grant(Allow, 1, SOME)
            }),
            pure_package(SOME, 1),
            Ask,
        ),
    ];
    for (name, grant, package, expected) in rows {
        assert_eq!(decide(grant.as_ref(), package), *expected, "{name}");
    }
}

// ---------------------------------------------------------------------------------------------
// The world: a core with the `plugins` module, a plugin folder, and a stand-in for the Task 9
// manager that loads a plugin only when the gate says `Run`.

const NOW: i64 = 1_800_000_000;
const FULL_PERMISSIONS: &str =
    "storage = true\nindex = true\nclipboard = true\nopen = true\ntimers = true\n";

struct World {
    core: Core,
    rx: async_channel::Receiver<CoreEvent>,
    handle: PluginsHandle,
    events: mpsc::Receiver<PluginEvents>,
    db_path: PathBuf,
    /// Holds the plugin folders.
    root: PathBuf,
    /// Effects modules sent on their own ([`CoreUpdate::ModuleEffects`]), in order.
    unsolicited: Vec<Effect>,
    /// The `plugins` module's clock; moved only by the tests.
    clock: Arc<Mutex<Instant>>,
    _data: tempfile::TempDir,
}

fn world() -> World {
    world_with(|_| {})
}

/// A world with `extra` registered next to the `plugins` module, all started.
fn world_with(extra: impl FnOnce(&mut Core)) -> World {
    let data = tempfile::tempdir().unwrap();
    let db_path = data.path().join("tomakast.db");
    let store = Store::open(&db_path).unwrap();
    let mut core = Core::new(store, RankWeights::default(), now()).unwrap();
    let rx = core.events();
    let (tx, events) = mpsc::channel();
    let clock = Arc::new(Mutex::new(Instant::now()));
    let read = clock.clone();
    let handle = PluginsModule::install_with_clock(
        &mut core,
        PluginDb::open(&db_path).unwrap(),
        tx,
        Arc::new(move || *read.lock().unwrap()),
    );
    extra(&mut core);
    core.start_modules();
    let root = data.path().join("plugins");
    std::fs::create_dir(&root).unwrap();
    let mut w = World {
        core,
        rx,
        handle,
        events,
        db_path,
        root,
        unsolicited: Vec::new(),
        clock,
        _data: data,
    };
    w.settle(Duration::from_millis(50));
    w
}

/// The kitchen-sink manifest with its `[permissions]` table replaced by `permissions`.
fn kitchen_sink_manifest(permissions: &str) -> String {
    let dir = &loaded().2;
    let text = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    let (head, _) = text.split_once("[permissions]").unwrap();
    format!("{head}[permissions]\n{permissions}")
}

/// The widgets kitchen-sink's `plugin.toml` declares, as a grant records them.
fn kitchen_sink_widgets() -> Vec<WidgetGrant> {
    Manifest::parse(&kitchen_sink_manifest(""), ID)
        .unwrap()
        .widgets
        .into_iter()
        .map(|w| WidgetGrant {
            id: w.id,
            title: w.title,
        })
        .collect()
}

/// Writes `<parent>/kitchen-sink/` with the built wasm and `manifest`; returns the folder.
fn write_plugin(parent: &Path, manifest: &str) -> PathBuf {
    let dir = parent.join(ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
    std::fs::copy(loaded().2.join("plugin.wasm"), dir.join("plugin.wasm")).unwrap();
    dir
}

impl World {
    fn db(&self) -> PluginDb {
        PluginDb::open(&self.db_path).unwrap()
    }

    fn plugin(&self, permissions: &str) -> PathBuf {
        write_plugin(&self.root, &kitchen_sink_manifest(permissions))
    }

    /// What the plugin manager does with a plugin folder: gate it, then load it only on `Run`,
    /// hand its review to the `plugins` module on `Review`, and skip it on `Denied`.
    fn admit(&mut self, dir: &Path) -> Gate {
        let package = Package::read(dir).unwrap();
        let db = self.db();
        let gate = check(&db, &package, NOW);
        match &gate {
            Gate::Run(at) => {
                let (host, component, _) = loaded();
                let plugin = WasmPlugin::new(package, component.clone(), host, db, BTreeMap::new())
                    .with_approved_at(*at);
                self.core.register(Box::new(plugin));
            }
            Gate::Review(review) => self.handle.set_reviews(vec![(**review).clone()]),
            Gate::Denied => {}
        }
        self.settle(Duration::from_millis(50));
        gate
    }

    /// Applies `ev`; unsolicited effects are also kept in `unsolicited`.
    fn apply(&mut self, ev: CoreEvent) -> CoreUpdate {
        let update = self.core.apply(ev, now());
        if let CoreUpdate::ModuleEffects { effects, .. } = &update {
            self.unsolicited.extend(effects.iter().cloned());
        }
        update
    }

    /// Applies every event that arrives within `wait`.
    fn settle(&mut self, wait: Duration) {
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

    /// Applies events until `cond` holds; panics after 5 s.
    fn wait_for(&mut self, what: &str, mut cond: impl FnMut(&mut World) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cond(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            if let Ok(ev) = self.rx.try_recv() {
                self.apply(ev);
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    /// Every indexed item, by id.
    fn indexed(&mut self) -> BTreeMap<String, Arc<Item>> {
        self.core
            .search("", now())
            .hits
            .into_iter()
            .filter(|h| !h.dynamic)
            .map(|h| (h.item.id.as_str().to_owned(), h.item))
            .collect()
    }

    fn review_item(&mut self) -> Option<Arc<Item>> {
        self.indexed().remove(REVIEW_ITEM)
    }

    /// Items of the kitchen-sink provider: there are some only once the plugin runs.
    fn plugin_items(&mut self) -> Vec<String> {
        self.indexed()
            .into_keys()
            .filter(|id| id.starts_with("kitchen-sink/"))
            .collect()
    }

    /// Enter on the review item: the detail view it pushes.
    fn open_review(&mut self) -> DetailView {
        let request = self
            .core
            .request_action(&ItemId::parse(REVIEW_ITEM).unwrap(), "review", "")
            .unwrap();
        loop {
            match self.apply(wait_event(&self.rx)) {
                CoreUpdate::ActionDone {
                    request: r,
                    effects,
                    ..
                } if r == request => match &effects[..] {
                    [Effect::PushView(View::Detail(view))] => return view.clone(),
                    other => panic!("expected one detail view, got {other:?}"),
                },
                CoreUpdate::ActionFailed {
                    request: r,
                    message,
                } if r == request => {
                    panic!("review failed: {message}")
                }
                _ => {}
            }
        }
    }

    fn view_event(&mut self, view: &ViewId, event: ViewEvent) -> Vec<Effect> {
        let instance = self.core.module_instance(PROVIDER).unwrap();
        let request = self
            .core
            .send_view_event(PROVIDER, instance, view, event)
            .unwrap();
        loop {
            match self.apply(wait_event(&self.rx)) {
                CoreUpdate::ViewEffects {
                    request: r,
                    effects,
                    ..
                } if r == request => return effects,
                CoreUpdate::ViewFailed {
                    request: r,
                    message,
                    ..
                } if r == request => {
                    panic!("view event failed: {message}")
                }
                _ => {}
            }
        }
    }

    /// Moves the `plugins` module's clock forward.
    fn advance(&self, by: Duration) {
        *self.clock.lock().unwrap() += by;
    }

    /// Chooses `action` in the review view `view` once the user has had time to read it.
    fn choose(&mut self, view: &DetailView, action: &str) -> Vec<Effect> {
        self.advance(MIN_READ_TIME + Duration::from_millis(100));
        self.choose_now(view, action)
    }

    /// Chooses `action` in the review view `view` without moving the clock.
    fn choose_now(&mut self, view: &DetailView, action: &str) -> Vec<Effect> {
        let event = ViewEvent::ActionChosen {
            row_id: view.id.0.clone(),
            action_id: action.into(),
        };
        self.view_event(&view.id, event)
    }

    fn plugin_event(&self) -> PluginEvents {
        self.events
            .recv_timeout(Duration::from_secs(5))
            .expect("a plugin event")
    }

    fn no_plugin_event(&self) {
        assert_eq!(
            self.events.try_recv().ok(),
            None,
            "an unexpected plugin event"
        );
    }
}

fn package_hash(dir: &Path) -> [u8; 32] {
    Package::read(dir).unwrap().content_hash
}

fn toast_kinds(effects: &[Effect]) -> Vec<ToastKind> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Toast(t) => Some(t.kind),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The review item

#[test]
fn review_item_appears_for_new_plugin() {
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    let gate = w.admit(&dir);

    let Gate::Review(review) = &gate else {
        panic!("a new plugin is reviewed, got {gate:?}");
    };
    assert_eq!(review.reason, ReviewReason::New);
    let item = w.review_item().expect("the review item is indexed");
    assert_eq!(item.title, "Review plugin: Kitchen Sink");
    assert_eq!(item.subtitle.as_deref(), Some(NEW_SUBTITLE));
    assert_eq!(item.kind, ItemKind::Command);
    assert_eq!(item.actions[0].id, "review");
    // Ruling R: nothing of the plugin is indexed or run before approval.
    assert_eq!(w.plugin_items(), Vec::<String>::new());
    w.no_plugin_event();
    // The prompt is never shown unprompted: no view, no toast, only the item.
    w.settle(Duration::from_millis(50));
    assert_eq!(w.unsolicited, []);
}

#[test]
fn reviews_set_before_start_are_published_on_start() {
    let data = tempfile::tempdir().unwrap();
    let db_path = data.path().join("tomakast.db");
    let store = Store::open(&db_path).unwrap();
    let mut core = Core::new(store, RankWeights::default(), now()).unwrap();
    let rx = core.events();
    let (tx, _events) = mpsc::channel();
    let handle = PluginsModule::install(&mut core, PluginDb::open(&db_path).unwrap(), tx);
    let dir = write_plugin(data.path(), &kitchen_sink_manifest(FULL_PERMISSIONS));
    let Gate::Review(review) = check(
        &PluginDb::open(&db_path).unwrap(),
        &Package::read(&dir).unwrap(),
        NOW,
    ) else {
        panic!("a new plugin is reviewed");
    };
    // Before the module's worker has started.
    handle.set_reviews(vec![*review]);
    core.start_modules();
    loop {
        if let CoreUpdate::ItemsChanged = core.apply(wait_event(&rx), now()) {
            break;
        }
    }
    let hits = core.search("Review plugin", now()).hits;
    assert!(
        hits.iter().any(|h| h.item.id.as_str() == REVIEW_ITEM),
        "{hits:?}"
    );
}

fn used_item() -> Item {
    Item {
        id: ItemId::new("fake", "used"),
        title: "Used app".into(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
            shortcut: None,
        }],
        source: None,
    }
}

#[test]
fn review_item_leads_empty_query_suggestions() {
    let mut w = world_with(|core| {
        core.register(Box::new(FakeModule::new(vec![used_item()], || Ok(vec![]))));
    });
    // The fake item gets usage, so it is a suggestion of its own.
    let request = w
        .core
        .request_action(&ItemId::new("fake", "used"), "run", "")
        .unwrap();
    w.wait_for("the launch", |w| {
        matches!(w.apply(wait_event(&w.rx)), CoreUpdate::ActionDone { request: r, .. } if r == request)
    });
    let dir = w.plugin(FULL_PERMISSIONS);
    w.admit(&dir);

    let hits = w.core.search("", now()).hits;
    let ids: Vec<&str> = hits.iter().map(|h| h.item.id.as_str()).collect();
    assert_eq!(ids, [REVIEW_ITEM, "fake/used"]);
    // What the launcher's Suggestions section takes: a static hit with a positive score, in
    // rank order (crates/ui/src/launcher_view.rs `merge_hits`).
    assert!(!hits[0].dynamic && hits[0].score > hits[1].score && hits[1].score > 0.);
    // A typed query ranks it like any other item.
    let typed = w.core.search("used", now()).hits;
    assert_eq!(typed[0].item.id.as_str(), "fake/used");
}

// ---------------------------------------------------------------------------------------------
// Allow and Deny

#[test]
fn allow_writes_grant_and_emits_approved() {
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    w.admit(&dir);
    let view = w.open_review();
    assert_eq!(
        view.actions
            .iter()
            .map(|a| a.title.as_str())
            .collect::<Vec<_>>(),
        ["Not now", "Allow", "Deny"]
    );

    let effects = w.choose(&view, "allow");

    let grant = w.db().grant(ID).unwrap().expect("a grant is written");
    assert_eq!(grant.decision, GrantDecision::Allow);
    assert_eq!(grant.content_hash, package_hash(&dir));
    assert_eq!(grant.permissions, perms(FULL_PERMISSIONS));
    assert_eq!(
        w.plugin_event(),
        PluginEvents::Approved {
            id: ID.into(),
            content_hash: package_hash(&dir)
        }
    );
    assert!(effects.contains(&Effect::PopView), "{effects:?}");
    assert_eq!(toast_kinds(&effects), [ToastKind::Success]);
    w.settle(Duration::from_millis(50));
    assert!(w.review_item().is_none(), "the review is done");

    // The manager re-gates the plugin on `Approved`: now it runs.
    assert!(matches!(w.admit(&dir), Gate::Run(_)));
    w.wait_for("the plugin's command", |w| {
        w.plugin_items().contains(&"kitchen-sink/hello".to_owned())
    });
}

#[test]
fn deny_writes_grant_and_plugin_never_instantiates() {
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    w.admit(&dir);
    let view = w.open_review();

    let effects = w.choose(&view, "deny");

    let grant = w.db().grant(ID).unwrap().expect("a grant is written");
    assert_eq!(grant.decision, GrantDecision::Deny);
    assert_eq!(grant.content_hash, package_hash(&dir));
    assert_eq!(
        w.plugin_event(),
        PluginEvents::Denied {
            id: ID.into(),
            content_hash: package_hash(&dir)
        }
    );
    assert!(effects.contains(&Effect::PopView), "{effects:?}");
    w.settle(Duration::from_millis(50));
    assert!(w.review_item().is_none());

    // The same content stays denied: it is neither loaded nor reviewed again.
    assert_eq!(w.admit(&dir), Gate::Denied);
    w.settle(Duration::from_millis(300));
    assert_eq!(w.plugin_items(), Vec::<String>::new(), "init never ran");
    assert!(w.review_item().is_none());
    w.no_plugin_event();
}

#[test]
fn unapproved_plugin_is_never_indexed() {
    // Ruling R, for each way a plugin can be unapproved.
    // An `allow` row for this very content whose permissions cannot be read.
    let malformed = |db: &Path, hash: [u8; 32]| {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO plugin_grants
               (plugin_id, content_hash, permissions, decision, decided_at, updated_at)
             VALUES (?1, ?2, 'storage = \"yes\"', 'allow', 1, 1)",
            rusqlite::params![ID, hash.as_slice()],
        )
        .unwrap();
    };
    type Setup = fn(&World, &Path);
    let cases: [(&str, Setup); 3] = [
        ("no grant", |_, _| {}),
        ("denied", |w, dir| {
            let mut g = grant(GrantDecision::Deny, 0, FULL_PERMISSIONS);
            g.content_hash = package_hash(dir);
            w.db().set_grant(&g).unwrap();
        }),
        ("allowed with fewer permissions", |w, _| {
            w.db()
                .set_grant(&grant(GrantDecision::Allow, 0, "storage = true\n"))
                .unwrap();
        }),
    ];
    for (name, setup) in cases {
        let mut w = world();
        let dir = w.plugin(FULL_PERMISSIONS);
        setup(&w, &dir);
        let gate = w.admit(&dir);
        assert!(!matches!(gate, Gate::Run(_)), "{name}");
        w.settle(Duration::from_millis(300));
        assert_eq!(w.plugin_items(), Vec::<String>::new(), "{name}");
    }
    // A grant row that cannot be read is a denial, not a review.
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    malformed(&w.db_path, package_hash(&dir));
    assert_eq!(w.admit(&dir), Gate::Denied);
    assert!(w.review_item().is_none());
    assert_eq!(w.plugin_items(), Vec::<String>::new());
}

// ---------------------------------------------------------------------------------------------
// Updates

#[test]
fn update_with_same_permissions_runs_silently_and_records_hash() {
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    // Approved for an older version with the same permissions and widgets.
    let older = Grant {
        widgets: kitchen_sink_widgets(),
        ..grant(GrantDecision::Allow, 9, FULL_PERMISSIONS)
    };
    w.db().set_grant(&older).unwrap();

    assert!(matches!(w.admit(&dir), Gate::Run(_)));

    let g = w.db().grant(ID).unwrap().unwrap();
    assert_eq!(
        g.content_hash,
        package_hash(&dir),
        "the new hash is recorded"
    );
    assert_eq!(g.updated_at, NOW);
    assert_eq!(g.decided_at, 5, "the decision itself is the user's old one");
    assert_eq!(g.permissions, perms(FULL_PERMISSIONS));
    assert_eq!(g.decision, GrantDecision::Allow);
    assert!(w.review_item().is_none(), "no prompt");
    w.wait_for("the plugin's command", |w| {
        w.plugin_items().contains(&"kitchen-sink/hello".to_owned())
    });

    // A smaller set keeps the approved one.
    let smaller = write_plugin(
        &w.root.join("v3"),
        &kitchen_sink_manifest("storage = true\n"),
    );
    assert_eq!(
        check(&w.db(), &Package::read(&smaller).unwrap(), NOW + 1),
        Gate::Run(5)
    );
    let g = w.db().grant(ID).unwrap().unwrap();
    assert_eq!(g.content_hash, package_hash(&smaller));
    assert_eq!(g.updated_at, NOW + 1);
    assert_eq!(g.permissions, perms(FULL_PERMISSIONS));
}

#[test]
fn update_with_more_permissions_asks_again() {
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    let old = grant(GrantDecision::Allow, 9, "storage = true\n");
    w.db().set_grant(&old).unwrap();

    let gate = w.admit(&dir);

    let Gate::Review(review) = &gate else {
        panic!("more permissions ask again, got {gate:?}");
    };
    assert_eq!(review.reason, ReviewReason::MorePermissions);
    let item = w.review_item().expect("the review item is indexed");
    assert_eq!(item.subtitle.as_deref(), Some(MORE_SUBTITLE));
    assert_eq!(
        w.db().grant(ID).unwrap(),
        Some(old),
        "the old grant is untouched"
    );
    assert_eq!(w.plugin_items(), Vec::<String>::new());
}

#[test]
fn recreated_folder_with_new_content_asks_again() {
    // Review Focus: approval is bound to the content the user saw.
    let mut w = world();
    let first = w.plugin("storage = true\n");
    let first_hash = package_hash(&first);
    w.admit(&first);
    let stale = w.open_review();
    assert!(stale.markdown.contains(&hex_prefix(&first_hash)));

    // While the review is open, the folder is deleted and re-created with other content, and
    // the manager replaces the review.
    std::fs::remove_dir_all(&first).unwrap();
    let second = w.plugin(FULL_PERMISSIONS);
    let second_hash = package_hash(&second);
    assert_ne!(second_hash, first_hash);
    let Gate::Review(_) = w.admit(&second) else {
        panic!("the new content is reviewed");
    };

    // Allow in the view of the first content is refused: that content is no longer pending.
    let effects = w.choose(&stale, "allow");
    assert!(effects.contains(&Effect::PopView), "{effects:?}");
    assert!(
        effects.iter().any(|e| matches!(e,
            Effect::Toast(t) if t.kind == ToastKind::Error
                && t.title == "This plugin changed; review it again")),
        "{effects:?}"
    );
    assert_eq!(w.db().grant(ID).unwrap(), None, "no grant is written");
    w.no_plugin_event();
    let item = w.review_item().expect("the new content still waits");
    assert_eq!(item.subtitle.as_deref(), Some(NEW_SUBTITLE));
    assert_eq!(w.plugin_items(), Vec::<String>::new());

    // The new content's own review shows its hash and binds the decision to it.
    let view = w.open_review();
    assert!(view.markdown.contains(&hex_prefix(&second_hash)));
    w.choose(&view, "deny");
    assert_eq!(w.db().grant(ID).unwrap().unwrap().content_hash, second_hash);
    assert_eq!(
        w.plugin_event(),
        PluginEvents::Denied {
            id: ID.into(),
            content_hash: second_hash
        }
    );

    // A denied plugin re-created with different content (same permissions) asks again.
    std::fs::remove_dir_all(&second).unwrap();
    let third = w.plugin(&format!("{FULL_PERMISSIONS}# rebuilt\n"));
    let gate = w.admit(&third);
    assert!(
        matches!(&gate, Gate::Review(r) if r.reason == ReviewReason::New),
        "{gate:?}"
    );
}

#[test]
fn enter_is_not_now_and_writes_nothing() {
    // Ruling AE: the primary action of the review view only closes it.
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    w.admit(&dir);
    let view = w.open_review();
    assert_eq!(view.actions[0].id, "not-now");
    assert_eq!(view.actions[0].title, "Not now");

    let effects = w.choose(&view, &view.actions[0].id.clone());

    assert_eq!(effects, [Effect::PopView]);
    assert_eq!(w.db().grant(ID).unwrap(), None);
    w.no_plugin_event();
    assert!(w.review_item().is_some(), "still waiting for a decision");
}

#[test]
fn allow_needs_a_moment_to_read() {
    // Ruling AE: "Allow" within 1 s of the view being shown is ignored.
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    w.admit(&dir);
    let view = w.open_review();

    w.advance(Duration::from_millis(500));
    let effects = w.choose_now(&view, "allow");
    assert_eq!(
        effects,
        [Effect::Toast(tomakast_core::Toast {
            kind: ToastKind::Error,
            title: "Take a moment to read the permissions".into(),
            message: None,
        })]
    );
    assert_eq!(w.db().grant(ID).unwrap(), None);
    w.no_plugin_event();

    // The same view, at 1.1 s.
    w.advance(Duration::from_millis(600));
    let effects = w.choose_now(&view, "allow");
    assert!(effects.contains(&Effect::PopView), "{effects:?}");
    let grant = w.db().grant(ID).unwrap().expect("a grant is written");
    assert_eq!(grant.decision, GrantDecision::Allow);
    assert_eq!(
        w.plugin_event(),
        PluginEvents::Approved {
            id: ID.into(),
            content_hash: package_hash(&dir)
        }
    );
}

#[test]
fn decision_in_a_closed_view_writes_nothing() {
    let mut w = world();
    let dir = w.plugin(FULL_PERMISSIONS);
    w.admit(&dir);
    let view = w.open_review();
    assert_eq!(w.view_event(&view.id, ViewEvent::Closed), []);

    let effects = w.choose(&view, "allow");

    assert_eq!(toast_kinds(&effects), [ToastKind::Error]);
    assert_eq!(w.db().grant(ID).unwrap(), None);
    w.no_plugin_event();
    assert!(w.review_item().is_some(), "still waiting for a decision");
}

// ---------------------------------------------------------------------------------------------
// The prompt text

fn hex_prefix(hash: &[u8; 32]) -> String {
    hash[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// The paragraphs and list items of `md` as plain text, parsed as the launcher's renderer
/// parses it (`pulldown_cmark::Parser::new`, no extensions). Panics on any markup other than
/// paragraphs, lists and text: a manifest string must never become emphasis, a link, an image,
/// code or HTML.
fn plain_blocks(md: &str) -> Vec<String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for event in Parser::new(md) {
        match event {
            Event::Start(Tag::Paragraph | Tag::Item) => current = Some(String::new()),
            Event::End(TagEnd::Paragraph | TagEnd::Item) => {
                if let Some(text) = current.take().filter(|t| !t.is_empty()) {
                    blocks.push(text);
                }
            }
            Event::Start(Tag::List(None)) | Event::End(TagEnd::List(false)) => {}
            Event::Text(text) => current.get_or_insert_default().push_str(&text),
            Event::SoftBreak => current.get_or_insert_default().push(' '),
            other => panic!("unexpected markup {other:?} in {md:?}"),
        }
    }
    blocks
}

#[test]
fn prompt_text_comes_from_manifest_as_plain_text() {
    let name = "**b** _i_ <b>x</b> [t](https://x.test)";
    let author = "<https://x.test> ![i](u) `c` &amp; \\* end\\";
    let version = "1 # *v* <i>";
    let manifest = kitchen_sink_manifest(&format!(
        "{FULL_PERMISSIONS}http = [\"example.com\", \"*.example.org\"]\n"
    ))
    .replace("name = \"Kitchen Sink\"", &format!("name = {name:?}"))
    .replace(
        "version = \"0.1.0\"",
        &format!("version = {version:?}\nauthor = {author:?}"),
    );
    let mut w = world();
    // The folder path is shown too; its parent is named with markup.
    let parent = w.root.join("[p](https://p.test) *q*");
    let dir = write_plugin(&parent, &manifest);
    w.admit(&dir);

    let item = w.review_item().expect("the review item");
    assert_eq!(
        item.title,
        format!("Review plugin: {name}"),
        "titles are plain text"
    );
    let view = w.open_review();
    assert_eq!(view.title, format!("Review plugin: {name}"));

    assert_eq!(
        plain_blocks(&view.markdown),
        [
            format!("Name: {name}"),
            format!("Id: {ID}"),
            format!("Version: {version}"),
            format!("Author: {author}"),
            format!("Folder: {}", dir.display()),
            "It asks to:".to_owned(),
            "Can store data on this computer (up to 1 MiB)".to_owned(),
            "Can copy text to the clipboard".to_owned(),
            "Can open links and files".to_owned(),
            "Can run timers in the background".to_owned(),
            "Can add items to search".to_owned(),
            "Can make web requests to example.com, *.example.org".to_owned(),
            "It shows tiles in the launcher: Static, Buttons, Animation, Echo, Probe, Quarter A, \
             Quarter B"
                .to_owned(),
            format!("Content hash: {}", hex_prefix(&package_hash(&dir))),
        ]
    );
}

#[test]
fn prompt_for_a_plugin_without_permissions() {
    let mut w = world();
    let dir = w.plugin("");
    w.admit(&dir);
    let view = w.open_review();
    let blocks = plain_blocks(&view.markdown);
    assert!(
        blocks.contains(&"It asks for no permissions.".to_owned()),
        "{blocks:?}"
    );
    assert!(
        !blocks.iter().any(|b| b.starts_with("Author:")),
        "{blocks:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Links

#[test]
fn link_policy_denies_plugin_without_open() {
    let native = ["apps", "plugins"].map(String::from).into_iter().collect();
    let policy = PluginLinkPolicy::new(native);
    for native in ["apps", "plugins"] {
        assert!(policy.may_open_links(native), "{native}");
    }
    // Only the listed providers are native, not every reserved id.
    assert!(!policy.may_open_links("demo"));
    // Unknown providers are not trusted: a plugin that is not (or no longer) loaded.
    assert!(!policy.may_open_links("kitchen-sink"));
    assert!(!policy.may_open_links(""));

    let with_open = perms("open = true\n");
    let without = perms("storage = true\nclipboard = true\n");
    policy.set_plugin("with-open", "With Open", &with_open);
    policy.set_plugin("without-open", "Without", &without);
    assert!(policy.may_open_links("with-open"));
    assert!(!policy.may_open_links("without-open"));

    // An update that drops `open`, and an unloaded plugin, lose links; clones share the table.
    let shared = policy.clone();
    shared.set_plugin("with-open", "With Open", &without);
    assert!(!policy.may_open_links("with-open"));
    shared.set_plugin("without-open", "Without", &with_open);
    shared.remove_plugin("without-open");
    assert!(!policy.may_open_links("without-open"));
    // A plugin cannot borrow a built-in module's trust by its id being touched.
    policy.remove_plugin("apps");
    assert!(policy.may_open_links("apps"));
}

/// Ruling AN: the policy names plugins for the footer of their views. A built-in module has
/// no plugin name; a provider that is not built in is a plugin, named by its id once it is
/// unloaded (its view may still be on the stack).
#[test]
fn link_policy_names_plugins() {
    let native = ["apps", "plugins"].map(String::from).into_iter().collect();
    let policy = PluginLinkPolicy::new(native);
    policy.set_plugin("kitchen-sink", "Kitchen Sink", &perms(""));
    assert_eq!(
        policy.plugin_name("kitchen-sink").as_deref(),
        Some("Kitchen Sink")
    );
    assert_eq!(policy.plugin_name("apps"), None);
    assert_eq!(policy.plugin_name("plugins"), None);
    policy.remove_plugin("kitchen-sink");
    assert_eq!(
        policy.plugin_name("kitchen-sink").as_deref(),
        Some("kitchen-sink")
    );
}

// ---------------------------------------------------------------------------------------------
// Widgets are approved like permissions

fn widget_toml(widgets: &[(&str, &str, &str)]) -> String {
    widgets
        .iter()
        .map(|(id, title, size)| {
            format!("[[widgets]]\nid = \"{id}\"\ntitle = \"{title}\"\nsize = \"{size}\"\n")
        })
        .collect()
}

/// A package with `widgets` ([[widgets]] must precede `[permissions]`).
fn widget_package(widgets: &[(&str, &str, &str)], hash: u8) -> Package {
    let text = format!("{MANIFEST_HEAD}{}[permissions]\n", widget_toml(widgets));
    Package {
        manifest: Manifest::parse(&text, ID).unwrap(),
        ..pure_package("", hash)
    }
}

fn widget_grants(widgets: &[(&str, &str)]) -> Vec<WidgetGrant> {
    widgets
        .iter()
        .map(|(id, title)| WidgetGrant {
            id: (*id).into(),
            title: (*title).into(),
        })
        .collect()
}

/// A temporary plugin database holding `grant`.
fn db_with(grant: &Grant) -> (tempfile::TempDir, PluginDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tomakast.db");
    let _store = Store::open(&path).unwrap();
    let db = PluginDb::open(&path).unwrap();
    db.set_grant(grant).unwrap();
    (dir, db)
}

fn widget_grant(widgets: &[(&str, &str)]) -> Grant {
    Grant {
        widgets: widget_grants(widgets),
        ..grant(GrantDecision::Allow, 1, "")
    }
}

#[test]
fn an_update_that_adds_a_widget_is_reviewed_again() {
    let (_d, db) = db_with(&widget_grant(&[("clock", "Clock")]));
    let package = widget_package(
        &[
            ("clock", "Clock", "single"),
            ("weather", "Weather", "single"),
        ],
        2,
    );
    let Gate::Review(review) = check(&db, &package, NOW) else {
        panic!("an added widget asks again");
    };
    assert_eq!(review.reason, ReviewReason::NewWidgets);
    assert_eq!(
        review.widgets,
        widget_grants(&[("clock", "Clock"), ("weather", "Weather")])
    );
    let mut w = world();
    w.handle.set_reviews(vec![*review]);
    w.settle(Duration::from_millis(50));
    assert_eq!(
        w.review_item().unwrap().subtitle.as_deref(),
        Some(NEW_WIDGETS_SUBTITLE)
    );
    assert_eq!(db.grant(ID).unwrap().unwrap().content_hash, [1; 32]);
}

#[test]
fn an_update_that_renames_a_widget_is_reviewed_again() {
    let (_d, db) = db_with(&widget_grant(&[("clock", "Clock")]));
    let package = widget_package(&[("clock", "World clock", "single")], 2);
    let Gate::Review(review) = check(&db, &package, NOW) else {
        panic!("a renamed widget asks again");
    };
    assert_eq!(review.reason, ReviewReason::NewWidgets);
}

#[test]
fn removing_a_widget_or_resizing_it_runs_silently() {
    let (_d, db) = db_with(&widget_grant(&[("clock", "Clock"), ("weather", "Weather")]));
    // One widget removed, the other resized.
    let package = widget_package(&[("clock", "Clock", "double")], 2);
    // The grant's `decided_at` (5) stays, though the hash was recorded.
    assert_eq!(check(&db, &package, NOW), Gate::Run(5));
    let stored = db.grant(ID).unwrap().unwrap();
    assert_eq!(stored.content_hash, [2; 32]);
    // The approved list is not shrunk (like permissions): `weather` stays approved.
    assert_eq!(
        stored.widgets,
        widget_grants(&[("clock", "Clock"), ("weather", "Weather")])
    );
}

#[test]
fn more_permissions_and_a_new_name_come_before_new_widgets() {
    let (_d, db) = db_with(&widget_grant(&[]));
    let text = format!(
        "{MANIFEST_HEAD}{}[permissions]\nclipboard = true\n",
        widget_toml(&[("clock", "Clock", "single")])
    );
    let package = Package {
        manifest: Manifest::parse(&text, ID).unwrap(),
        ..pure_package("", 2)
    };
    let Gate::Review(review) = check(&db, &package, NOW) else {
        panic!("asks");
    };
    assert_eq!(review.reason, ReviewReason::MorePermissions);

    let renamed = Grant {
        name: "Old Name".into(),
        ..widget_grant(&[])
    };
    let (_d2, db2) = db_with(&renamed);
    let package = widget_package(&[("clock", "Clock", "single")], 2);
    let Gate::Review(review) = check(&db2, &package, NOW) else {
        panic!("asks");
    };
    assert_eq!(review.reason, ReviewReason::Renamed);
}

#[test]
fn a_grant_from_before_widgets_reviews_a_plugin_with_widgets() {
    let (d, db) = db_with(&grant(GrantDecision::Allow, 1, ""));
    // The column is NULL, as in a row written before schema v5.
    let path = d.path().join("tomakast.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE plugin_grants SET widgets = NULL", [])
        .unwrap();
    let package = widget_package(&[("clock", "Clock", "single")], 2);
    let Gate::Review(review) = check(&db, &package, NOW) else {
        panic!("widgets of a pre-widgets grant ask");
    };
    assert_eq!(review.reason, ReviewReason::NewWidgets);
}

#[test]
fn a_grant_from_before_widgets_runs_a_plugin_without_widgets() {
    let (d, db) = db_with(&grant(GrantDecision::Allow, 1, ""));
    rusqlite::Connection::open(d.path().join("tomakast.db"))
        .unwrap()
        .execute("UPDATE plugin_grants SET widgets = NULL", [])
        .unwrap();
    assert_eq!(check(&db, &pure_package("", 2), NOW), Gate::Run(5));
}

#[test]
fn the_same_content_with_an_unapproved_widget_is_reviewed() {
    // A NULL-widgets row (a v4 database) whose hash equals the package's: the gate must
    // not rely on the hash alone.
    let (d, db) = db_with(&grant(GrantDecision::Allow, 1, ""));
    rusqlite::Connection::open(d.path().join("tomakast.db"))
        .unwrap()
        .execute("UPDATE plugin_grants SET widgets = NULL", [])
        .unwrap();
    let package = widget_package(&[("clock", "Clock", "single")], 1);
    let Gate::Review(review) = check(&db, &package, NOW) else {
        panic!("an unapproved widget asks even for the same hash");
    };
    assert_eq!(review.reason, ReviewReason::NewWidgets);
}

#[test]
fn the_prompt_lists_the_widgets_by_title() {
    let mut w = world();
    let widgets = widget_toml(&[
        ("clock", "Clock", "single"),
        ("weather", "Weather", "double"),
    ]);
    let manifest = kitchen_sink_manifest("storage = true\n");
    let (head, rest) = manifest.split_once("[permissions]").unwrap();
    let dir = write_plugin(&w.root, &format!("{head}{widgets}[permissions]{rest}"));
    let Gate::Review(review) = w.admit(&dir) else {
        panic!("a new plugin is reviewed");
    };
    // Kitchen-sink's own widgets first, then the test's.
    let mut expected = kitchen_sink_widgets();
    expected.extend(widget_grants(&[("clock", "Clock"), ("weather", "Weather")]));
    assert_eq!(review.widgets, expected);
    let view = w.open_review();
    assert!(
        view.markdown.contains(
            "It shows tiles in the launcher: Static, Buttons, Animation, Echo, Probe, Quarter A, \
             Quarter B, Clock, Weather"
        ),
        "{}",
        view.markdown
    );
    assert_eq!(
        w.review_item().unwrap().subtitle.as_deref(),
        Some(NEW_SUBTITLE)
    );
    // Approving records the widgets, so the same widgets later run silently.
    let effects = w.choose(&view, "allow");
    assert!(
        toast_kinds(&effects).iter().all(|k| *k != ToastKind::Error),
        "{effects:?}"
    );
    assert_eq!(w.db().grant(ID).unwrap().unwrap().widgets, expected);
}

#[test]
fn a_loaded_plugin_reports_when_the_user_approved_it() {
    use tomakast_core::Module;
    let (host, component, _) = loaded();
    let plugin = |at: Option<i64>| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tomakast.db");
        let _store = Store::open(&path).unwrap();
        let db = PluginDb::open(&path).unwrap();
        let p = WasmPlugin::new(
            Package {
                wasm: Arc::from(Vec::new()),
                ..pure_package("", 1)
            },
            component.clone(),
            host,
            db,
            BTreeMap::new(),
        );
        (
            dir,
            at.map_or(p.approved_at(), |at| p.with_approved_at(at).approved_at()),
        )
    };
    assert_eq!(plugin(None).1, None);
    assert_eq!(plugin(Some(5)).1, Some(5));
}
