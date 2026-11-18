use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreUpdate, Effect, Item, ItemId, ItemKind, LocalNow, RankWeights, ViewEvent, ViewId,
};

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

fn ok() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CloseWindow])
}

fn fail() -> anyhow::Result<Vec<Effect>> {
    Err(anyhow::anyhow!("boom"))
}

/// Core over a file-backed store, plus a second handle on the same file to inspect it.
fn core_with(
    items: Vec<Item>,
    result: fn() -> anyhow::Result<Vec<Effect>>,
) -> (Core, Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut core = Core::new(Store::open(&path).unwrap(), RankWeights::default(), now()).unwrap();
    core.register(Box::new(FakeModule::new(items, result)));
    core.start_modules();
    let inspect = Store::open(&path).unwrap();
    (core, inspect, dir)
}

/// Applies the module's start-up items, waiting for its worker to publish them.
fn drain(core: &mut Core) {
    let ev = wait_event(&core.events());
    assert!(matches!(core.apply(ev, now()), CoreUpdate::ItemsChanged));
}

/// Requests the action and applies its reply.
fn execute(core: &mut Core, item: &ItemId, query: &str) -> CoreUpdate {
    core.request_action(item, "run", query).unwrap();
    let ev = wait_event(&core.events());
    core.apply(ev, now())
}

fn hits(core: &mut Core, query: &str) -> Vec<String> {
    core.search(query, now())
        .hits
        .iter()
        .map(|h| h.item.title.clone())
        .collect()
}

fn two_items() -> Vec<Item> {
    vec![item("fake", "a", "Alpha"), item("fake", "b", "Beta")]
}

#[test]
fn module_items_become_searchable_after_apply() {
    let (mut core, _s, _d) = core_with(two_items(), ok);
    assert!(hits(&mut core, "").is_empty());
    drain(&mut core);
    assert_eq!(hits(&mut core, ""), ["Alpha", "Beta"]);
}

#[test]
fn successful_action_records_usage_in_store_and_engine() {
    let (mut core, store, _d) = core_with(two_items(), ok);
    drain(&mut core);
    assert_eq!(hits(&mut core, "")[0], "Alpha");
    let beta = ItemId::new("fake", "b");
    match execute(&mut core, &beta, "be") {
        CoreUpdate::ActionDone { effects, .. } => assert_eq!(effects, [Effect::CloseWindow]),
        other => panic!("expected ActionDone, got {other:?}"),
    }
    assert_eq!(store.load_usage(0).unwrap().len(), 1);
    assert_eq!(hits(&mut core, "")[0], "Beta");
}

#[test]
fn failed_action_does_not_record_usage() {
    let (mut core, store, _d) = core_with(two_items(), fail);
    drain(&mut core);
    let alpha = ItemId::new("fake", "a");
    assert!(matches!(
        execute(&mut core, &alpha, ""),
        CoreUpdate::ActionFailed { .. }
    ));
    assert!(store.load_usage(0).unwrap().is_empty());
}

#[test]
fn foreign_provider_items_are_dropped() {
    let items = vec![item("fake", "a", "Alpha"), item("other", "x", "Foreign")];
    let (mut core, _s, _d) = core_with(items, ok);
    drain(&mut core);
    let hits = core.search("", now()).hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.id.as_str(), "fake/a");
}

#[test]
fn request_for_unknown_item_is_error() {
    let (mut core, store, _d) = core_with(two_items(), ok);
    drain(&mut core);
    assert!(
        core.request_action(&ItemId::new("fake", "nope"), "run", "")
            .is_err()
    );
    assert!(
        core.request_action(&ItemId::new("ghost", "x"), "run", "")
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
    assert!(matches!(
        execute(&mut core, &beta, ""),
        CoreUpdate::ActionDone { .. }
    ));
    // The in-memory record is kept.
    assert_eq!(hits(&mut core, "")[0], "Beta");
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

/// Gamma for this module, plus an item of another provider and one with an
/// invalid id, which the core must drop.
fn replacement() -> Vec<Effect> {
    let bad = Item {
        id: ItemId::parse("fake/bad\nid").unwrap(),
        ..item("fake", "bad", "Bad")
    };
    vec![
        Effect::ReplaceItems(vec![
            item("fake", "c", "Gamma"),
            item("other", "x", "Foreign"),
            bad,
        ]),
        Effect::CloseWindow,
    ]
}

fn replace_and_close() -> anyhow::Result<Vec<Effect>> {
    Ok(replacement())
}

#[test]
fn replace_items_from_an_action_is_applied_by_the_core() {
    let (mut core, _s, _d) = core_with(two_items(), replace_and_close);
    drain(&mut core);

    let update = execute(&mut core, &ItemId::new("fake", "a"), "");

    match update {
        CoreUpdate::ActionDone {
            provider,
            effects,
            items_changed,
            ..
        } => {
            assert_eq!(provider, "fake");
            assert_eq!(
                effects,
                [Effect::CloseWindow],
                "the UI never sees ReplaceItems"
            );
            assert!(items_changed);
        }
        other => panic!("expected ActionDone, got {other:?}"),
    }
    assert_eq!(hits(&mut core, ""), ["Gamma"]);
}

#[test]
fn replace_items_from_a_view_event_is_applied_by_the_core() {
    let module = FakeModule::new(two_items(), ok).with_view_events(|_, _| Ok(replacement()));
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    core.register(Box::new(module));
    core.start_modules();
    drain(&mut core);

    let view = ViewId("fake/v".into());
    let event = ViewEvent::ActionChosen {
        row_id: "r".into(),
        action_id: "a".into(),
    };
    core.send_view_event("fake", &view, event).unwrap();
    let ev = wait_event(&core.events());

    match core.apply(ev, now()) {
        CoreUpdate::ViewEffects {
            provider,
            view: v,
            effects,
            items_changed,
            ..
        } => {
            assert_eq!(provider, "fake");
            assert_eq!(v, view);
            assert_eq!(effects, [Effect::CloseWindow]);
            assert!(items_changed);
        }
        other => panic!("expected ViewEffects, got {other:?}"),
    }
    assert_eq!(hits(&mut core, ""), ["Gamma"]);
}

#[test]
fn replies_without_replace_items_do_not_change_items() {
    let (mut core, _s, _d) = core_with(two_items(), ok);
    drain(&mut core);

    match execute(&mut core, &ItemId::new("fake", "a"), "") {
        CoreUpdate::ActionDone { items_changed, .. } => assert!(!items_changed),
        other => panic!("expected ActionDone, got {other:?}"),
    }
}
