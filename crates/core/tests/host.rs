//! Module workers: every module call runs on the module's own thread and
//! comes back as a `CoreEvent`.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreEvent, CoreUpdate, Effect, Item, ItemId, ItemKind, LocalNow, RankWeights, ReadingKind,
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
        source: None,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
            shortcut: None,
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
        busy: false,
    };
    assert!(matches!(core.apply(stale, now()), CoreUpdate::None));

    let current = CoreEvent::SearchResults {
        provider: "fake".into(),
        generation: second.generation,
        items: vec![item("fake", "new", "= new")],
        busy: false,
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
fn a_busy_answer_keeps_the_root_list_busy_until_an_answer_is_not() {
    let (mut core, _s, _d) = core_over(vec![FakeModule::new(Vec::new(), close)]);
    apply_next(&mut core, 1);
    let answer = |generation, busy| CoreEvent::SearchResults {
        provider: "fake".into(),
        generation,
        items: Vec::new(),
        busy,
    };
    let busy = |update: CoreUpdate| match update {
        CoreUpdate::Results(snapshot) => snapshot.busy,
        other => panic!("expected Results, got {other:?}"),
    };

    let first = core.search("fir", now());
    assert!(!first.busy);
    assert!(busy(core.apply(answer(first.generation, true), now())));
    // The next key: still busy until the module answers it.
    let second = core.search("fire", now());
    assert!(second.busy, "the busy line runs on while the user types");
    assert!(!busy(core.apply(answer(second.generation, false), now())));

    let third = core.search("fires", now());
    assert!(busy(core.apply(answer(third.generation, true), now())));
    // The empty query: nobody is asked, nothing is busy.
    assert!(!core.search("", now()).busy);
}

#[test]
fn search_again_asks_the_module_the_latest_search_again() {
    let module = FakeModule::new(Vec::new(), close).with_search(vec![]);
    let searches = module.searches.clone();
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    let snapshot = core.search("fir", now());
    apply_next(&mut core, 1);
    let effects = CoreEvent::ModuleEffects {
        provider: "fake".into(),
        effects: vec![Effect::SearchAgain],
    };
    // The core applies it: nothing of it is left for the UI.
    match core.apply(effects, now()) {
        CoreUpdate::ModuleEffects { effects, .. } => assert!(effects.is_empty(), "{effects:?}"),
        other => panic!("expected ModuleEffects, got {other:?}"),
    }
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::Results(again) => assert_eq!(again.generation, snapshot.generation),
        other => panic!("expected Results, got {other:?}"),
    }
    assert_eq!(*searches.lock().unwrap(), ["fir", "fir"]);
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
fn dynamic_source_receives_readings() {
    let calc = FakeModule::new(Vec::new(), close)
        .with_id("calc")
        .with_search(vec![]);
    let readings = calc.readings.clone();
    let apps = FakeModule::new(vec![item("fake", "firefox", "Firefox")], close);
    let (mut core, _s, _d) = core_over(vec![apps, calc]);
    apply_next(&mut core, 2);

    core.search("ашкуащч", now());
    apply_next(&mut core, 1);

    let calls = readings.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .iter()
            .any(|r| r.kind == ReadingKind::Layout && r.text == "firefox"),
        "{:?}",
        calls[0]
    );
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
        .with_search(vec![item("slow", "late", "= a late")])
        .with_delay(Duration::from_millis(500));
    let fast = FakeModule::new(vec![item("fake", "firefox", "Firefox")], close)
        .with_search(vec![item("fake", "answer", "= z fast")]);
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
    assert_eq!(titles(&updates[0]), ["= z fast", "Firefox"]);
    assert!(started.elapsed() >= Duration::from_millis(500));
    // Equal dynamic scores keep the provider order, not the title order:
    // providers by id ("fake" before "slow"), each in its own list order.
    assert_eq!(titles(&updates[1]), ["= z fast", "= a late", "Firefox"]);
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

fn woke(token: u64) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CopyText(format!("woke {token}"))])
}

fn publish_old() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::ReplaceItems(vec![item("fake", "old", "Old")])])
}

/// Titles of every hit for the empty query.
fn all_titles(core: &mut Core) -> Vec<String> {
    let snapshot = core.search("", now());
    snapshot.hits.iter().map(|h| h.item.title.clone()).collect()
}

/// Blocks until `rx` holds `n` unapplied events; panics after 5 s.
fn wait_queued(rx: &async_channel::Receiver<CoreEvent>, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while rx.len() < n {
        assert!(Instant::now() < deadline, "{n} events never queued");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The context the module's `start` received, once it has run.
fn started_ctx(module: &FakeModule) -> impl Fn() -> tomakast_core::ModuleContext + use<> {
    let ctx = module.ctx.clone();
    move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(ctx) = ctx.lock().unwrap().clone() {
                return ctx;
            }
            assert!(Instant::now() < deadline, "start never ran");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
fn module_with_runtime_id_runs() {
    let id: String = ["plug", "a"].join("-");
    let module = FakeModule::new(vec![item(&id, "x", "Plugged")], close).with_id(id.clone());
    let threads = module.threads.clone();
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    let request = core
        .request_action(&ItemId::new(&id, "x"), "run", "")
        .unwrap();
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::ActionDone {
            request: r,
            provider,
            effects,
            ..
        } => {
            assert_eq!(r, request);
            assert_eq!(provider, "plug-a");
            assert_eq!(effects, vec![Effect::CloseWindow]);
        }
        other => panic!("expected ActionDone, got {other:?}"),
    }
    assert_eq!(threads.lock().unwrap()[0], "module-plug-a");
}

#[test]
fn register_after_start_starts_worker() {
    let (mut core, _s, _d) = core_over(vec![]);
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let threads = module.threads.clone();
    core.register(Box::new(module));

    // No second `start_modules`: the worker starts and publishes on its own.
    assert!(matches!(
        apply_next(&mut core, 1).remove(0),
        CoreUpdate::ItemsChanged
    ));
    assert_eq!(all_titles(&mut core), ["Alpha"]);
    core.request_action(&ItemId::new("fake", "a"), "run", "")
        .unwrap();
    assert!(matches!(
        apply_next(&mut core, 1).remove(0),
        CoreUpdate::ActionDone { .. }
    ));
    assert_eq!(*threads.lock().unwrap(), ["module-fake", "module-fake"]);
}

#[test]
fn register_same_id_replaces_and_drops_items() {
    let old = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let old_calls = old.calls.clone();
    let (mut core, _s, _d) = core_over(vec![old]);
    apply_next(&mut core, 1);
    assert_eq!(all_titles(&mut core), ["Alpha"]);

    let new = FakeModule::new(vec![item("fake", "b", "Beta")], close);
    let new_threads = new.threads.clone();
    let new_calls = new.calls.clone();
    core.register(Box::new(new));
    // The old items are gone at once, before the new module publishes.
    assert!(all_titles(&mut core).is_empty());

    apply_next(&mut core, 1);
    assert_eq!(all_titles(&mut core), ["Beta"]);
    assert_eq!(*new_threads.lock().unwrap(), ["module-fake"]);

    // Actions go to the new instance only.
    assert!(
        core.request_action(&ItemId::new("fake", "a"), "run", "")
            .is_err()
    );
    core.request_action(&ItemId::new("fake", "b"), "run", "")
        .unwrap();
    apply_next(&mut core, 1);
    assert_eq!(new_calls.lock().unwrap().len(), 1);
    assert!(old_calls.lock().unwrap().is_empty());
}

#[test]
fn replaced_instance_events_are_dropped() {
    // The old instance's first publish is still queued, unapplied, when the
    // replacement is registered.
    let old = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let old_ctx = started_ctx(&old);
    let (mut core, _s, _d) = core_over(vec![old]);
    let rx = core.events();
    wait_queued(&rx, 1);
    let old_ctx = old_ctx();

    core.register(Box::new(FakeModule::new(
        vec![item("fake", "b", "Beta")],
        close,
    )));
    wait_queued(&rx, 2);
    // Applied in arrival order: the old instance's publish comes first and
    // must not write its items for the new instance.
    assert!(matches!(
        core.apply(wait_event(&rx), now()),
        CoreUpdate::None
    ));
    assert!(all_titles(&mut core).is_empty());
    apply_next(&mut core, 1);
    assert_eq!(all_titles(&mut core), ["Beta"]);

    // A context the old instance kept is dead too.
    old_ctx.replace_items(vec![item("fake", "late", "Late")]);
    old_ctx.emit(vec![Effect::ReplaceItems(vec![item(
        "fake", "late2", "Late 2",
    )])]);
    for update in apply_next(&mut core, 2) {
        assert!(matches!(update, CoreUpdate::None), "got {update:?}");
    }
    assert_eq!(all_titles(&mut core), ["Beta"]);
}

#[test]
fn reply_of_a_replaced_instance_cannot_write_items() {
    let old = FakeModule::new(vec![item("fake", "a", "Alpha")], publish_old)
        .with_delay(Duration::from_millis(200));
    let old_calls = old.calls.clone();
    let old_threads = old.threads.clone();
    let (mut core, store, _d) = core_over(vec![old]);
    apply_next(&mut core, 1);

    // The old instance is busy with the first action, the second is queued
    // behind it, when it is replaced.
    let alpha = ItemId::new("fake", "a");
    let busy = core.request_action(&alpha, "run", "").unwrap();
    let queued = core.request_action(&alpha, "run", "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    // `start` and the first `run_action` have both begun.
    while old_threads.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "the action never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    core.register(Box::new(FakeModule::new(
        vec![item("fake", "b", "Beta")],
        close,
    )));

    let updates = apply_next(&mut core, 3);
    // The old worker finished its current call and served nothing after it.
    assert_eq!(old_calls.lock().unwrap().len(), 1);
    for request in [busy, queued] {
        let failed = updates
            .iter()
            .find_map(|u| match u {
                CoreUpdate::ActionFailed {
                    request: r,
                    message,
                } if *r == request => Some(message),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no ActionFailed for {request:?} in {updates:?}"));
        assert_eq!(failed, tomakast_core::RELOADED, "message: {failed:?}");
    }
    // The busy action's `ReplaceItems` was not applied.
    assert_eq!(all_titles(&mut core), ["Beta"]);
    assert!(store.load_usage(0).unwrap().is_empty());
}

#[test]
fn unregister_removes_items_and_dynamic_answers() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close)
        .with_search(vec![item("fake", "answer", "= 4")])
        .with_on_wake(woke);
    let wakes = module.wakes.clone();
    let ctx = started_ctx(&module);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);
    core.search("", now());
    assert_eq!(titles(&apply_next(&mut core, 1)[0]), ["= 4", "Alpha"]);
    let ctx = ctx();

    core.unregister("fake");
    assert!(core.refresh(now()).hits.is_empty());
    assert!(
        core.request_action(&ItemId::new("fake", "a"), "run", "")
            .is_err()
    );
    assert!(
        core.request_action(&ItemId::new("fake", "answer"), "run", "")
            .is_err()
    );

    // The worker is stopped: a wake-up is never served.
    ctx.wake(7);
    std::thread::sleep(Duration::from_millis(100));
    assert!(wakes.lock().unwrap().is_empty());
    assert!(core.events().is_empty());
}

#[test]
fn unregistered_instance_events_are_dropped() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let (mut core, _s, _d) = core_over(vec![module]);
    let rx = core.events();
    wait_queued(&rx, 1);

    core.unregister("fake");
    assert!(matches!(
        core.apply(wait_event(&rx), now()),
        CoreUpdate::None
    ));
    assert!(all_titles(&mut core).is_empty());
}

#[test]
fn wake_calls_on_wake_on_the_worker_thread() {
    let module = FakeModule::new(Vec::new(), close).with_on_wake(woke);
    let wakes = module.wakes.clone();
    let ctx = started_ctx(&module);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    ctx().wake(42);
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::ModuleEffects {
            provider,
            effects,
            items_changed,
            ..
        } => {
            assert_eq!(provider, "fake");
            assert_eq!(effects, vec![Effect::CopyText("woke 42".into())]);
            assert!(!items_changed);
        }
        other => panic!("expected ModuleEffects, got {other:?}"),
    }
    assert_eq!(*wakes.lock().unwrap(), [(42, "module-fake".to_string())]);
}

#[test]
fn emit_from_other_thread_arrives_as_module_effects() {
    let module = FakeModule::new(Vec::new(), close);
    let ctx = started_ctx(&module);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    let ctx = ctx();
    std::thread::Builder::new()
        .name("elsewhere".into())
        .spawn(move || ctx.emit(vec![Effect::CloseWindow]))
        .unwrap()
        .join()
        .unwrap();
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::ModuleEffects {
            provider,
            effects,
            items_changed,
            ..
        } => {
            assert_eq!(provider, "fake");
            assert_eq!(effects, vec![Effect::CloseWindow]);
            assert!(!items_changed);
        }
        other => panic!("expected ModuleEffects, got {other:?}"),
    }
}

#[test]
fn module_effects_apply_replace_items() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close);
    let ctx = started_ctx(&module);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    ctx().emit(vec![
        Effect::ReplaceItems(vec![
            item("fake", "b", "Beta"),
            item("other", "x", "Foreign"),
        ]),
        Effect::CopyText("done".into()),
    ]);
    match apply_next(&mut core, 1).remove(0) {
        CoreUpdate::ModuleEffects {
            provider,
            effects,
            items_changed,
            ..
        } => {
            assert_eq!(provider, "fake");
            assert_eq!(effects, vec![Effect::CopyText("done".into())]);
            assert!(items_changed);
        }
        other => panic!("expected ModuleEffects, got {other:?}"),
    }
    assert_eq!(all_titles(&mut core), ["Beta"]);
}

#[test]
fn dynamic_id_equal_to_static_id_is_dropped() {
    let module = FakeModule::new(vec![item("fake", "a", "Alpha")], close).with_search(vec![
        item("fake", "a", "= shadow"),
        item("fake", "b", "= ok"),
    ]);
    let (mut core, _s, _d) = core_over(vec![module]);
    apply_next(&mut core, 1);

    core.search("", now());
    assert_eq!(titles(&apply_next(&mut core, 1)[0]), ["= ok", "Alpha"]);
    // The static item keeps its own action target.
    core.request_action(&ItemId::new("fake", "a"), "run", "")
        .unwrap();
    apply_next(&mut core, 1);

    // An index change after the answer arrived still wins over the answer.
    let ev = CoreEvent::ReplaceItems {
        provider: "fake".into(),
        items: vec![item("fake", "a", "Alpha"), item("fake", "b", "Beta")],
    };
    core.apply(ev, now());
    let refreshed = core.refresh(now());
    let refreshed: Vec<_> = refreshed
        .hits
        .iter()
        .map(|h| h.item.title.clone())
        .collect();
    assert_eq!(refreshed, ["Alpha", "Beta"]);
}

#[test]
fn invalid_module_ids_are_refused() {
    let refused: Vec<FakeModule> = ["", "a/b", "bell\u{7}"]
        .into_iter()
        .map(|id| FakeModule::new(Vec::new(), close).with_id(id))
        .collect();
    let refused_threads: Vec<_> = refused.iter().map(|m| m.threads.clone()).collect();
    let mut all = refused;
    all.push(FakeModule::new(vec![item("fake", "a", "Alpha")], close));
    let (mut core, _s, _d) = core_over(all);

    // Only the valid module starts and publishes.
    assert!(matches!(
        apply_next(&mut core, 1).remove(0),
        CoreUpdate::ItemsChanged
    ));
    std::thread::sleep(Duration::from_millis(100));
    assert!(core.events().is_empty());
    for threads in refused_threads {
        assert!(threads.lock().unwrap().is_empty());
    }
    assert_eq!(all_titles(&mut core), ["Alpha"]);
}

fn push_detail(id: &str) -> Effect {
    Effect::PushView(tomakast_core::View::Detail(tomakast_core::DetailView {
        id: tomakast_core::ViewId(id.into()),
        title: id.into(),
        markdown: String::new(),
        metadata: Vec::new(),
        actions: Vec::new(),
    }))
}

fn toast_reply(
    view: &tomakast_core::ViewId,
    _: tomakast_core::ViewEvent,
) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::Toast(tomakast_core::Toast {
        kind: tomakast_core::ToastKind::Success,
        title: view.0.clone(),
        message: None,
    })])
}

fn query(text: &str) -> tomakast_core::ViewEvent {
    tomakast_core::ViewEvent::QueryChanged {
        revision: 1,
        text: text.into(),
        readings: vec![],
    }
}

/// The answer to the view event `request`, after applying events until it arrives.
fn view_answer(core: &mut Core, request: tomakast_core::RequestId) -> Result<Vec<Effect>, String> {
    let rx = core.events();
    loop {
        match core.apply(wait_event(&rx), now()) {
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

/// Runs `fake/a`'s action and returns the module instance that answered.
fn run_a(core: &mut Core) -> u64 {
    core.request_action(&ItemId::new("fake", "a"), "run", "")
        .unwrap();
    match apply_next(core, 1).remove(0) {
        CoreUpdate::ActionDone {
            module_instance, ..
        } => module_instance,
        other => panic!("expected ActionDone, got {other:?}"),
    }
}

fn push_echo() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![push_detail("echo")])
}

/// Review Focus: the old instance's view stays on the stack, its events go nowhere, also
/// when the new instance pushes a view with the same id (the views are told apart by the
/// instance that pushed them, which the UI keeps per stack entry).
#[test]
fn events_of_a_view_of_a_replaced_instance_answer_plugin_reloaded() {
    let old =
        FakeModule::new(vec![item("fake", "a", "Alpha")], push_echo).with_view_events(toast_reply);
    let old_views = old.view_events.clone();
    let (mut core, _s, _d) = core_over(vec![old]);
    apply_next(&mut core, 1);
    let old_instance = run_a(&mut core);
    assert_eq!(core.module_instance("fake"), Some(old_instance));
    let echo = tomakast_core::ViewId("echo".into());
    // While the instance runs, its view's events reach it.
    let r = core
        .send_view_event("fake", old_instance, &echo, query("a"))
        .unwrap();
    assert!(view_answer(&mut core, r).is_ok());
    assert_eq!(old_views.lock().unwrap().len(), 1);

    let new =
        FakeModule::new(vec![item("fake", "a", "Alpha")], push_echo).with_view_events(toast_reply);
    let new_views = new.view_events.clone();
    core.register(Box::new(new));
    apply_next(&mut core, 1);
    // (a) The new instance pushes its own `echo` above the old one.
    let new_instance = run_a(&mut core);
    assert_ne!(new_instance, old_instance);

    let r = core
        .send_view_event("fake", old_instance, &echo, query("b"))
        .unwrap();
    assert_eq!(
        view_answer(&mut core, r),
        Err(tomakast_core::RELOADED.to_string())
    );
    let r = core
        .send_view_event("fake", new_instance, &echo, query("c"))
        .unwrap();
    assert!(view_answer(&mut core, r).is_ok());
    assert_eq!(old_views.lock().unwrap().len(), 1);
    assert_eq!(new_views.lock().unwrap().len(), 1);

    // (b) The new `echo` is closed; the old one's events still go nowhere.
    let r = core
        .send_view_event(
            "fake",
            new_instance,
            &echo,
            tomakast_core::ViewEvent::Closed,
        )
        .unwrap();
    assert!(view_answer(&mut core, r).is_ok());
    let r = core
        .send_view_event("fake", old_instance, &echo, query("d"))
        .unwrap();
    assert_eq!(
        view_answer(&mut core, r),
        Err(tomakast_core::RELOADED.to_string())
    );
    assert_eq!(old_views.lock().unwrap().len(), 1);
    assert_eq!(new_views.lock().unwrap().len(), 2);

    // Once the provider is gone, the message says so.
    core.unregister("fake");
    let r = core
        .send_view_event("fake", new_instance, &echo, query("e"))
        .unwrap();
    assert_eq!(
        view_answer(&mut core, r),
        Err("module fake was stopped".to_string())
    );
}

/// (c) However many views one module pushes, another module's reload check is unaffected.
#[test]
fn many_pushes_by_one_module_do_not_affect_another_modules_check() {
    let flood = FakeModule::new(Vec::new(), close).with_id("flood");
    let flood_ctx = started_ctx(&flood);
    let fake =
        FakeModule::new(vec![item("fake", "a", "Alpha")], push_echo).with_view_events(toast_reply);
    let (mut core, _s, _d) = core_over(vec![flood, fake]);
    // Both modules publish their items on start.
    apply_next(&mut core, 2);
    let old_instance = run_a(&mut core);
    let flood_ctx = flood_ctx();
    for i in 0..400 {
        flood_ctx.emit(vec![push_detail(&format!("flood-{i}"))]);
    }
    apply_next(&mut core, 400);

    core.register(Box::new(
        FakeModule::new(vec![item("fake", "a", "Alpha")], push_echo).with_view_events(toast_reply),
    ));
    apply_next(&mut core, 1);
    let echo = tomakast_core::ViewId("echo".into());
    let r = core
        .send_view_event("fake", old_instance, &echo, query("x"))
        .unwrap();
    assert_eq!(
        view_answer(&mut core, r),
        Err(tomakast_core::RELOADED.to_string())
    );
    let flood_instance = core.module_instance("flood").unwrap();
    let flood_view = tomakast_core::ViewId("flood-0".into());
    let r = core
        .send_view_event("flood", flood_instance, &flood_view, query("y"))
        .unwrap();
    assert!(view_answer(&mut core, r).is_ok());
}

#[test]
fn in_flight_reply_of_a_replaced_instance_says_plugin_reloaded() {
    let old = FakeModule::new(vec![item("fake", "a", "Alpha")], close)
        .with_delay(Duration::from_millis(200));
    let old_threads = old.threads.clone();
    let (mut core, _s, _d) = core_over(vec![old]);
    apply_next(&mut core, 1);
    let request = core
        .request_action(&ItemId::new("fake", "a"), "run", "")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while old_threads.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "the action never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    core.register(Box::new(FakeModule::new(Vec::new(), close)));
    let failed = apply_next(&mut core, 2).into_iter().find_map(|u| match u {
        CoreUpdate::ActionFailed {
            request: r,
            message,
        } if r == request => Some(message),
        _ => None,
    });
    assert_eq!(failed.as_deref(), Some(tomakast_core::RELOADED));
}
