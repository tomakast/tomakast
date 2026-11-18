use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::FakeModule;
use tomakast_core::{ActionOutcome, Core, Item, ItemId, ItemKind, LocalNow, RankWeights};

const NOW_UNIX: i64 = 1_700_000_000;

fn now() -> LocalNow {
    LocalNow::from_unix_with_offset(NOW_UNIX, 0)
}

fn item(provider: &str, local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new(provider, local),
        title: title.to_string(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Command,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
        }],
    }
}

fn ok() -> anyhow::Result<ActionOutcome> {
    Ok(ActionOutcome::HideWindow)
}

fn fail() -> anyhow::Result<ActionOutcome> {
    Err(anyhow::anyhow!("boom"))
}

/// Core over a file-backed store, plus a second handle on the same file to inspect it.
fn core_with(
    items: Vec<Item>,
    result: fn() -> anyhow::Result<ActionOutcome>,
) -> (Core, Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut core = Core::new(Store::open(&path).unwrap(), RankWeights::default(), now()).unwrap();
    core.register(Box::new(FakeModule::new(items, result)));
    core.start_modules();
    let inspect = Store::open(&path).unwrap();
    (core, inspect, dir)
}

fn drain(core: &mut Core) {
    let rx = core.events();
    while let Ok(ev) = rx.try_recv() {
        core.apply(ev);
    }
}

fn two_items() -> Vec<Item> {
    vec![item("fake", "a", "Alpha"), item("fake", "b", "Beta")]
}

#[test]
fn module_items_become_searchable_after_apply() {
    let (mut core, _s, _d) = core_with(two_items(), ok);
    assert!(core.search("", now()).is_empty());
    let ev = core.events().recv_blocking().unwrap();
    core.apply(ev);
    assert_eq!(core.search("", now()).len(), 2);
}

#[test]
fn execute_records_usage_in_store_and_engine() {
    let (mut core, store, _d) = core_with(two_items(), ok);
    drain(&mut core);
    assert_eq!(core.search("", now())[0].item.title, "Alpha");
    let beta = ItemId::new("fake", "b");
    let outcome = core.execute(&beta, "run", "be", now()).unwrap();
    assert_eq!(outcome, ActionOutcome::HideWindow);
    assert_eq!(store.load_usage(0).unwrap().len(), 1);
    assert_eq!(core.search("", now())[0].item.title, "Beta");
}

#[test]
fn failed_action_does_not_record_usage() {
    let (mut core, store, _d) = core_with(two_items(), fail);
    drain(&mut core);
    let alpha = ItemId::new("fake", "a");
    assert!(core.execute(&alpha, "run", "", now()).is_err());
    assert!(store.load_usage(0).unwrap().is_empty());
}

#[test]
fn foreign_provider_items_are_dropped() {
    let items = vec![item("fake", "a", "Alpha"), item("other", "x", "Foreign")];
    let (mut core, _s, _d) = core_with(items, ok);
    drain(&mut core);
    let hits = core.search("", now());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.id.as_str(), "fake/a");
}

#[test]
fn execute_unknown_item_is_error() {
    let (mut core, store, _d) = core_with(two_items(), ok);
    drain(&mut core);
    assert!(
        core.execute(&ItemId::new("fake", "nope"), "run", "", now())
            .is_err()
    );
    assert!(
        core.execute(&ItemId::new("ghost", "x"), "run", "", now())
            .is_err()
    );
    assert!(store.load_usage(0).unwrap().is_empty());
}

#[test]
fn store_failure_after_successful_action_is_not_an_error() {
    let (mut core, _s, dir) = core_with(two_items(), ok);
    drain(&mut core);
    // Break persistence: drop the table from a second connection to the same file.
    rusqlite::Connection::open(dir.path().join("db.sqlite"))
        .unwrap()
        .execute_batch("DROP TABLE usage")
        .unwrap();
    let beta = ItemId::new("fake", "b");
    let outcome = core.execute(&beta, "run", "", now()).unwrap();
    assert_eq!(outcome, ActionOutcome::HideWindow);
    // The in-memory record is kept.
    assert_eq!(core.search("", now())[0].item.title, "Beta");
}

#[test]
fn primary_action_is_the_first_action_of_an_indexed_item() {
    let mut first = item("fake", "a", "Alpha");
    first.actions.push(Action {
        id: "second".into(),
        title: "Second".into(),
    });
    let (mut core, _inspect, _dir) = core_with(vec![first], ok);
    assert_eq!(core.primary_action(&ItemId::new("fake", "a")), None);
    drain(&mut core);
    assert_eq!(
        core.primary_action(&ItemId::new("fake", "a")).as_deref(),
        Some("run")
    );
    assert_eq!(core.primary_action(&ItemId::new("fake", "missing")), None);
}
