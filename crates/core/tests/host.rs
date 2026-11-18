//! Module workers: every module call runs on the module's own thread and
//! comes back as a `CoreEvent`.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreEvent, CoreUpdate, Effect, Item, ItemId, ItemKind, LocalNow, RankWeights,
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

fn close() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CloseWindow])
}

fn fail() -> anyhow::Result<Vec<Effect>> {
    Err(anyhow::anyhow!("boom"))
}

/// Core over a file-backed store, plus a second handle on the same file to inspect it.
fn core_over(modules: Vec<FakeModule>) -> (Core, Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut core = Core::new(Store::open(&path).unwrap(), RankWeights::default(), now()).unwrap();
    for m in modules {
        core.register(Box::new(m));
    }
    core.start_modules();
    let inspect = Store::open(&path).unwrap();
    (core, inspect, dir)
}

/// Applies the next `n` core events (blocking on the workers) and returns the updates.
fn apply_next(core: &mut Core, n: usize) -> Vec<CoreUpdate> {
    let rx = core.events();
    (0..n).map(|_| core.apply(wait_event(&rx), now())).collect()
}

fn titles(update: &CoreUpdate) -> Vec<String> {
    match update {
        CoreUpdate::Results(snapshot) => {
            snapshot.hits.iter().map(|h| h.item.title.clone()).collect()
        }
        other => panic!("expected Results, got {other:?}"),
    }
}

#[test]
fn action_runs_off_caller_thread() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let threads = module.threads.clone();
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    let request = core
        .request_action(&ItemId::new("fake", "a"), "run", "")
        .unwrap();
    let update = apply_next(&mut core, 1).remove(0);

    match update {
        CoreUpdate::ActionDone {
            request: r,
            effects,
            ..
        } => {
            assert_eq!(r, request);
            assert_eq!(effects, vec![Effect::CloseWindow]);
        }
        other => panic!("expected ActionDone, got {other:?}"),
    }
    // `start` and `run_action` both ran on the worker.
    assert_eq!(
        *threads.lock().unwrap(),
        ["module-fake".to_string(), "module-fake".to_string()]
    );
    assert_ne!(std::thread::current().name(), Some("module-fake"));
}

#[test]
fn usage_recorded_only_on_success_reply() {
    let (mut ok_core, ok_store, _d1) = core_over(vec![FakeModule::new(
        vec![item("fake", "a", "Alpha")],
        close,
    )]);
    apply_next(&mut ok_core, 1);
    ok_core
        .request_action(&ItemId::new("fake", "a"), "run", "Al")
        .unwrap();
    // Nothing is recorded until the reply is applied.
    assert!(ok_store.load_usage(0).unwrap().is_empty());
    apply_next(&mut ok_core, 1);
    let records = ok_store.load_usage(0).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].item, ItemId::new("fake", "a"));
    assert_eq!(records[0].query, "al");
    assert_eq!(records[0].at, now());

    let (mut bad_core, bad_store, _d2) = core_over(vec![FakeModule::new(
        vec![item("fake", "a", "Alpha")],
        fail,
    )]);
    apply_next(&mut bad_core, 1);
    let request = bad_core
        .request_action(&ItemId::new("fake", "a"), "run", "")
        .unwrap();
    match apply_next(&mut bad_core, 1).remove(0) {
        CoreUpdate::ActionFailed {
            request: r,
            message,
        } => {
            assert_eq!(r, request);
            assert!(message.contains("boom"), "message: {message:?}");
        }
        other => panic!("expected ActionFailed, got {other:?}"),
    }
    assert!(bad_store.load_usage(0).unwrap().is_empty());
}

#[test]
fn unknown_item_or_provider_is_rejected_at_once() {
    let (mut core, _s, _d) = core_over(vec![FakeModule::new(
        vec![item("fake", "a", "Alpha")],
        close,
    )]);
    apply_next(&mut core, 1);
    assert!(
        core.request_action(&ItemId::new("fake", "nope"), "run", "")
            .is_err()
    );
    assert!(
        core.request_action(&ItemId::new("ghost", "x"), "run", "")
            .is_err()
    );
}

#[test]
fn stale_generation_results_are_dropped() {
    let (mut core, _s, _d) = core_over(vec![FakeModule::new(
        vec![item("fake", "a", "Alpha")],
        close,
    )]);
    apply_next(&mut core, 1);

    let first = core.search("a", now());
    let second = core.search("ab", now());
    assert!(second.generation > first.generation);

    let stale = CoreEvent::SearchResults {
        provider: "fake".into(),
        generation: first.generation,
        items: vec![item("fake", "old", "= old")],
    };
    assert!(matches!(core.apply(stale, now()), CoreUpdate::None));

    let current = CoreEvent::SearchResults {
        provider: "fake".into(),
        generation: second.generation,
        items: vec![item("fake", "new", "= new")],
    };
    let update = core.apply(current, now());
    match &update {
        CoreUpdate::Results(snapshot) => assert_eq!(snapshot.generation, second.generation),
        other => panic!("expected Results, got {other:?}"),
    }
    // "Alpha" does not match "ab"; only the current-generation answer is listed.
    assert_eq!(titles(&update), ["= new"]);
}

#[test]
fn dynamic_items_rank_above_static_matches() {
    let calc = FakeModule::new(Vec::new(), close)
        .with_id("calc")
        .with_search(vec![item("calc", "answer", "= 4")]);
    let apps = FakeModule::new(vec![item("fake", "firefox", "Firefox")], close);
    let (mut core, _s, _d) = core_over(vec![apps, calc]);
    // Both modules publish their (possibly empty) items at start.
    apply_next(&mut core, 2);

    let snapshot = core.search("fi", now());
    // Static hits are there at once; the dynamic answer is not yet.
    let static_titles: Vec<_> = snapshot.hits.iter().map(|h| h.item.title.clone()).collect();
    assert_eq!(static_titles, ["Firefox"]);

    let update = apply_next(&mut core, 1).remove(0);
    assert_eq!(titles(&update), ["= 4", "Firefox"]);
}

#[test]
fn panicking_module_does_not_kill_core() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let panic_on_action = module.panic_on_action.clone();
    let (mut core, store, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);
    let alpha = ItemId::new("fake", "a");

    panic_on_action.store(true, Ordering::SeqCst);
    let first = core.request_action(&alpha, "run", "").unwrap();
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::ActionFailed { request, message } => {
            assert_eq!(request, first);
            assert_eq!(message, "module fake panicked");
        }
        other => panic!("expected ActionFailed, got {other:?}"),
    }
    assert!(store.load_usage(0).unwrap().is_empty());

    // The same worker keeps serving the module.
    panic_on_action.store(false, Ordering::SeqCst);
    let second = core.request_action(&alpha, "run", "").unwrap();
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::ActionDone {
            request, effects, ..
        } => {
            assert_eq!(request, second);
            assert_eq!(effects, vec![Effect::CloseWindow]);
        }
        other => panic!("expected ActionDone, got {other:?}"),
    }
}

#[test]
fn hung_module_search_misses_deadline() {
    let slow = FakeModule::new(Vec::new(), close)
        .with_id("slow")
        .with_search(vec![item("slow", "late", "= late")])
        .with_delay(Duration::from_millis(500));
    let fast = FakeModule::new(vec![item("fake", "firefox", "Firefox")], close)
        .with_search(vec![item("fake", "answer", "= fast")]);
    let (mut core, _s, _d) = core_over(vec![slow, fast]);
    apply_next(&mut core, 2);

    let started = Instant::now();
    let snapshot = core.search("fi", now());
    // The slow module does not hold up the static hits.
    assert!(started.elapsed() < Duration::from_millis(250));
    let static_titles: Vec<_> = snapshot.hits.iter().map(|h| h.item.title.clone()).collect();
    assert_eq!(static_titles, ["Firefox"]);

    // The fast module answers first; the slow one's late answer for the
    // current generation is still merged when it arrives.
    let updates = apply_next(&mut core, 2);
    assert_eq!(titles(&updates[0]), ["= fast", "Firefox"]);
    assert!(started.elapsed() >= Duration::from_millis(500));
    // Equal dynamic scores fall back to the title order.
    assert_eq!(titles(&updates[1]), ["= fast", "= late", "Firefox"]);
}

#[test]
fn searches_are_coalesced_per_module() {
    let slow = FakeModule::new(Vec::new(), close)
        .with_search(vec![item("fake", "answer", "= x")])
        .with_delay(Duration::from_millis(200));
    let searches = slow.searches.clone();
    let (mut core, _s, _d) = core_over(vec![slow]);
    apply_next(&mut core, 1);

    core.search("a", now());
    core.search("ab", now());
    let last = core.search("abc", now());

    // Only the newest generation's answer reaches the UI.
    let update = apply_next(&mut core, 1).remove(0);
    match &update {
        CoreUpdate::Results(snapshot) => assert_eq!(snapshot.generation, last.generation),
        other => panic!("expected Results, got {other:?}"),
    }
    let asked = searches.lock().unwrap().clone();
    // "ab" was queued behind a busy worker while "abc" was already queued: skipped.
    assert!(!asked.contains(&"ab".to_string()), "asked: {asked:?}");
    assert_eq!(asked.last().map(String::as_str), Some("abc"));
}

#[test]
fn invalid_item_ids_are_dropped() {
    let long = format!("fake/{}", "x".repeat(300));
    let items = vec![
        item("fake", "ok", "Valid"),
        item("fake", "b\nc", "Newline"),
        item("fake", "tab\tid", "Control"),
        Item {
            id: ItemId::parse(&long).unwrap(),
            ..item("fake", "x", "Long")
        },
    ];
    let dynamic = vec![
        item("fake", "dyn\nbad", "= bad"),
        item("fake", "dyn", "= good"),
    ];
    let module = FakeModule::new(items, close).with_search(dynamic);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    let snapshot = core.search("", now());
    let static_titles: Vec<_> = snapshot.hits.iter().map(|h| h.item.title.clone()).collect();
    assert_eq!(static_titles, ["Valid"]);

    let update = apply_next(&mut core, 1).remove(0);
    assert_eq!(titles(&update), ["= good", "Valid"]);
}

#[test]
fn items_changed_keeps_the_generation_and_dynamic_results() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close)
        .with_search(vec![item("fake", "answer", "= 4")]);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);
    let asked = core.search("", now());
    apply_next(&mut core, 1);

    // A later item refresh re-ranks the current query without asking the modules again.
    let ev = CoreEvent::ReplaceItems {
        provider: "fake".into(),
        items: vec![item("fake", "b", "Beta")],
    };
    assert!(matches!(core.apply(ev, now()), CoreUpdate::ItemsChanged));
    let refreshed = core.refresh(now());
    assert_eq!(refreshed.generation, asked.generation);
    let refreshed_titles: Vec<_> = refreshed
        .hits
        .iter()
        .map(|h| h.item.title.clone())
        .collect();
    assert_eq!(refreshed_titles, ["= 4", "Beta"]);
}

#[test]
fn panicking_search_is_skipped_and_the_worker_keeps_serving() {
    let module = FakeModule::new(Vec::new(), close).with_search(vec![item("fake", "x", "= x")]);
    let panic_on_search = module.panic_on_search.clone();
    let searches = module.searches.clone();
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    panic_on_search.store(true, Ordering::SeqCst);
    core.search("a", now());
    // The module has seen "a" (and so decided to panic) before the switch goes off.
    let deadline = Instant::now() + Duration::from_secs(5);
    while searches.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "search was never called");
        std::thread::sleep(Duration::from_millis(1));
    }
    panic_on_search.store(false, Ordering::SeqCst);
    let second = core.search("b", now());

    // The panicked search sends nothing; the next one is answered.
    let update = apply_next(&mut core, 1).remove(0);
    match &update {
        CoreUpdate::Results(snapshot) => assert_eq!(snapshot.generation, second.generation),
        other => panic!("expected Results, got {other:?}"),
    }
    assert_eq!(titles(&update), ["= x"]);
    assert_eq!(*searches.lock().unwrap(), ["a", "b"]);
}
