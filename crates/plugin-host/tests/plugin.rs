//! `WasmPlugin` driven through the core, as the daemon drives it: commands and init, calls
//! and conversions, deadlines and limits, the broken state, storage, preferences and caps.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::harness::*;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Action, CoreEvent, CoreUpdate, Effect, FieldValue, Icon, Item, ItemId, ItemKind, ToastKind,
    ViewEvent,
};
use tomakast_plugin_host::PluginEvents;

#[test]
fn init_commands_are_indexed_with_source() {
    let mut h = harness();
    let hello = h
        .indexed("Say hello")
        .expect("the manifest command is indexed");
    assert_eq!(hello.id.as_str(), "kitchen-sink/hello");
    assert_eq!(hello.kind, ItemKind::Command);
    assert_eq!(hello.source.as_deref(), Some(NAME));
    assert_eq!(
        hello.icon,
        Some(Icon::Named("face-slightly-smiling".into()))
    );
    assert_eq!(hello.actions[0].id, "run");
    // init's replace-items keeps the command (Ruling H) and gets the source too.
    let indexed = h.indexed("Indexed by init").expect("init's item");
    assert_eq!(indexed.source.as_deref(), Some(NAME));
    assert_eq!(indexed.kind, ItemKind::Command);
    // The command runs. Its toast says which plugin it is from (Ruling AN).
    let effects = h.run("kitchen-sink/hello", "run").unwrap();
    assert!(
        matches!(&effects[..], [Effect::Toast(t)] if t.title == "hello"),
        "{effects:?}"
    );
}

#[test]
fn replace_items_keeps_manifest_commands() {
    let mut h = harness();
    let effects = h.run("kitchen-sink/indexed", "replace").unwrap();
    assert!(effects.is_empty(), "{effects:?}");
    assert!(h.indexed("Replaced").is_some());
    assert!(h.indexed("Indexed by init").is_none(), "replaced");
    // The command survives, and a plugin item cannot take its id.
    let hello = h.indexed("Say hello").expect("the command stays");
    assert_eq!(hello.id.as_str(), "kitchen-sink/hello");
    assert!(h.indexed("Not the command").is_none());
    assert!(h.indexed("Theirs").is_none(), "a foreign id is dropped");
}

#[test]
fn search_roundtrip() {
    let mut h = harness();
    let items = h.search("echo:hi there").expect("an answer");
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.id.as_str(), "kitchen-sink/echo");
    assert_eq!(item.title, "hi there");
    assert_eq!(item.subtitle.as_deref(), Some("a subtitle"));
    assert_eq!(item.keywords, ["one", "two"]);
    // `plugin-file` resolves to the file in the (canonical) plugin folder.
    let folder = loaded().2.canonicalize().unwrap();
    assert_eq!(
        item.icon,
        Some(Icon::PluginFile(folder.join("icons/echo.png")))
    );
    assert_eq!(item.kind, ItemKind::Command);
    assert_eq!(item.source.as_deref(), Some(NAME));
    assert_eq!(item.actions[1].shortcut.as_deref(), Some("ctrl+shift+c"));
}

/// kitchen-sink implements only the SDK's `search(query)`: the core's readings reach the
/// `search` export, and the SDK default answers as before.
#[test]
fn search_with_readings_keeps_an_old_style_guest_answering() {
    // A Cyrillic title in the index, so a Latin query also gets Cyrillic layout readings.
    let files = Item {
        id: ItemId::parse("fake/files").unwrap(),
        title: "Файлы".into(),
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
    };
    let fake = FakeModule::new(vec![files], || Ok(vec![]));
    let mut h = harness_beside(|_| {}, BTreeMap::new(), |p| p, vec![Box::new(fake)]);
    while h.core.query_readings("many:3").is_empty() {
        h.update();
    }
    assert!(!h.core.query_readings("ашкуащч").is_empty());
    // kitchen-sink answers no item for a query it does not know: an answer, not a failure.
    assert_eq!(h.search("ашкуащч").map(|items| items.len()), Some(0));
    assert_eq!(h.search("many:3").map(|items| items.len()), Some(3));
    assert!(h.take_toast().is_none(), "no denial or broken toast");
}

/// Ruling U end to end: a `plugin-file` that does not resolve to a small regular file in the
/// folder, reached without a link, becomes no icon; the item is kept.
#[test]
fn unresolvable_plugin_file_icon_is_dropped() {
    let mut h = harness();
    let folder = loaded().2.canonicalize().unwrap();
    let icon = |h: &mut Harness, path: &str| {
        let items = h.search(&format!("icon:{path}")).expect("an answer");
        assert_eq!(items.len(), 1, "the item is kept");
        items[0].icon.clone()
    };
    assert_eq!(
        icon(&mut h, "icons/echo.png"),
        Some(Icon::PluginFile(folder.join("icons/echo.png")))
    );
    assert_eq!(icon(&mut h, "icons/missing.png"), None);
    assert_eq!(icon(&mut h, "icons"), None, "a folder");
    #[cfg(unix)]
    {
        let link = folder.join("icons/link.png");
        if !link.exists() {
            std::os::unix::fs::symlink(folder.join("icons/echo.png"), &link).unwrap();
        }
        assert_eq!(icon(&mut h, "icons/link.png"), None, "a symbolic link");
    }
}

#[test]
fn run_receives_whole_item() {
    let mut h = harness();
    h.search("echo:dynamic").expect("an answer");
    let effects = h.run("kitchen-sink/echo", "echo-item").unwrap();
    let meta = metadata(&effects);
    assert_eq!(meta["id"], "kitchen-sink/echo");
    assert_eq!(meta["title"], "dynamic");
    assert_eq!(meta["subtitle"], "Some(\"a subtitle\")");
    assert_eq!(meta["keywords"], "one,two");
    // The plugin gets its own relative path back, not the host path.
    assert_eq!(meta["icon"], "Some(Icon::PluginFile(\"icons/echo.png\"))");
    assert_eq!(
        meta["actions"],
        "echo-item:Echo:None,copy:Copy:Some(\"ctrl+shift+c\")"
    );
}

#[test]
fn on_event_view_target_roundtrip() {
    let mut h = harness();
    let cases = [
        (
            ViewEvent::QueryChanged {
                revision: 7,
                text: "abc".into(),
                readings: vec![],
            },
            "query-changed 7 abc",
        ),
        (
            ViewEvent::ActionChosen {
                row_id: "row".into(),
                action_id: "act".into(),
            },
            "action-chosen row act",
        ),
        (
            ViewEvent::FormSubmitted {
                values: vec![
                    ("name".into(), FieldValue::Text("Ada".into())),
                    ("ok".into(), FieldValue::Bool(true)),
                    ("day".into(), FieldValue::Date(Some("2026-11-19".into()))),
                    ("none".into(), FieldValue::Date(None)),
                ],
            },
            "form-submitted name=FieldValue::Text(\"Ada\"),ok=FieldValue::Boolean(true),\
             day=FieldValue::Date(Some(\"2026-11-19\")),none=FieldValue::Date(None)",
        ),
        (
            ViewEvent::TextChanged {
                revision: 3,
                text: "1+1".into(),
            },
            "text-changed 3 1+1",
        ),
        (ViewEvent::Closed, "closed"),
    ];
    for (event, expected) in cases {
        let effects = h.view_event("my-view", event).unwrap();
        match &effects[..] {
            [Effect::Toast(t)] => {
                assert_eq!(t.title, "my-view");
                assert_eq!(t.message.as_deref(), Some(expected));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn infinite_loop_in_search_traps_and_marks_broken() {
    let mut h = harness();
    let start = Instant::now();
    h.core.search("spin", now());
    let toast = h.toast();
    let elapsed = start.elapsed();
    // The 100 ms deadline, not the 1000 or 2000 ms of the other calls.
    assert!(
        elapsed >= Duration::from_millis(100) && elapsed < Duration::from_millis(500),
        "{elapsed:?}"
    );
    h.unsolicited.insert(0, Effect::Toast(toast));
    assert_broken(&mut h, "search did not answer within 100 ms");
}

#[test]
fn oversized_return_value_breaks_the_plugin() {
    let mut h = harness();
    let effects = h.run("kitchen-sink/indexed", "big-return:3").unwrap();
    assert!(matches!(&effects[..], [Effect::CopyText(s)] if s.len() == 3 << 20));
    // Breaks rather than rejects: after a failed lift wasmtime refuses to enter the instance
    // again ("cannot enter component instance").
    let err = h.run("kitchen-sink/indexed", "big-return:5").unwrap_err();
    assert!(err.contains("more than the 4 MiB"), "{err}");
    assert_broken(&mut h, "more than the 4 MiB");
}

#[test]
fn memory_hog_traps_at_64_mib() {
    let mut h = harness();
    // Well under the limit: fine.
    assert_eq!(h.run("kitchen-sink/indexed", "hog:32"), Ok(vec![]));
    let err = h.run("kitchen-sink/indexed", "hog:96").unwrap_err();
    assert!(err.contains("64 MiB memory limit"), "{err}");
    assert_broken(&mut h, "64 MiB memory limit");
}

#[test]
fn guest_panic_is_an_error_not_a_host_panic() {
    let mut h = harness();
    let err = h.run("kitchen-sink/indexed", "panic").unwrap_err();
    assert!(err.starts_with("plugin kitchen-sink stopped: "), "{err}");
    assert!(err.contains("trap"), "{err}");
    assert_broken(&mut h, "trap");
}

#[test]
fn blocking_wasi_call_is_refused() {
    let mut h = harness();
    let start = Instant::now();
    assert_eq!(h.search("sleep"), None);
    let toast = h.toast();
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "the sleep was not cut short: {:?}",
        start.elapsed()
    );
    h.unsolicited.insert(0, Effect::Toast(toast));
    assert_broken(&mut h, "may not block");
}

#[test]
fn storage_quota_enforced() {
    let mut h = harness();
    let meta = metadata(&h.run("kitchen-sink/indexed", "storage").unwrap());
    assert_eq!(meta["set"], "Ok(())");
    assert_eq!(meta["get"], "Ok(Some(\"v\"))");
    assert_eq!(meta["remove"], "Ok(())");
    assert_eq!(meta["get-removed"], "Ok(None)");
    assert!(
        meta["long-key"].contains("longer than 256 bytes"),
        "{meta:?}"
    );
    assert!(
        meta["big-value"].contains("larger than 262144 bytes"),
        "{meta:?}"
    );
    assert_eq!(meta["fill-stored"], "5");
    // Ruling 7: a stable prefix plugins can match.
    assert!(meta["fill-error"].starts_with("quota: "), "{meta:?}");
}

/// Ruling AO: at most 20 storage writes (set and remove) per second per plugin; the rest fail
/// with an error, the plugin lives on, and a second later it may write again.
#[test]
fn storage_writes_are_rate_limited() {
    let mut h = harness();
    let meta = metadata(&h.run("kitchen-sink/indexed", "storage-flood:25").unwrap());
    assert_eq!(meta["written"], "20", "{meta:?}");
    assert!(meta["error"].contains("20 storage writes"), "{meta:?}");
    std::thread::sleep(Duration::from_millis(1100));
    let meta = metadata(&h.run("kitchen-sink/indexed", "storage-flood:2").unwrap());
    assert_eq!(meta["written"], "2", "{meta:?}");
}

#[test]
fn storage_without_permission_errors() {
    let mut h = harness_with(|p| p.manifest.permissions.storage = false, BTreeMap::new());
    let meta = metadata(&h.run("kitchen-sink/indexed", "storage").unwrap());
    for step in [
        "set",
        "get",
        "remove",
        "get-removed",
        "long-key",
        "big-value",
    ] {
        assert!(
            meta[step].starts_with("Err(") && meta[step].contains("storage permission"),
            "{step}: {}",
            meta[step]
        );
    }
    assert_eq!(meta["fill-stored"], "0");
}

#[test]
fn preferences_default_and_override() {
    let prefs = BTreeMap::from([
        ("name".to_string(), "Ada".to_string()),
        ("loud".to_string(), "true".to_string()),
        ("undeclared".to_string(), "x".to_string()),
    ]);
    let mut h = harness_with(|_| {}, prefs);
    let meta = metadata(&h.run("kitchen-sink/indexed", "prefs").unwrap());
    assert_eq!(meta["greeting"], "Some(\"hello\")", "the manifest default");
    assert_eq!(meta["name"], "Some(\"Ada\")", "set by the user");
    assert_eq!(meta["loud"], "Some(\"true\")", "overrides the default");
    assert_eq!(meta["undeclared"], "None", "only declared preferences");
}

/// `preference-default` leaves the answer and reaches the plugin manager, for a key the
/// manifest declares only.
#[test]
fn a_preference_default_reaches_the_manager_for_declared_keys_only() {
    let mut h = harness();
    let effects = h.run("kitchen-sink/indexed", "preference-default").unwrap();
    assert!(effects.is_empty(), "{effects:?}");
    let event = h
        .plugin_events
        .recv_timeout(Duration::from_secs(5))
        .expect("a report to the manager");
    let PluginEvents::PreferenceDefault { id, key, value, .. } = event else {
        panic!("{event:?}");
    };
    assert_eq!(
        (id.as_str(), key.as_str(), value.as_str()),
        ("kitchen-sink", "name", "Ann")
    );
    // The undeclared key is dropped, with no toast.
    assert!(
        h.plugin_events
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none());
}

/// `locale.languages` answers the source's languages cleaned to codes, read on every call.
#[test]
fn kitchen_sink_reads_the_languages() {
    let langs = |h: &mut Harness| {
        metadata(&h.run("kitchen-sink/indexed", "tz").unwrap())["languages"].clone()
    };
    let source = Arc::new(Mutex::new(vec![
        "en_US.UTF-8".to_string(),
        "ru".to_string(),
    ]));
    let read = Arc::clone(&source);
    let mut h = harness_built(
        |_| {},
        BTreeMap::new(),
        move |plugin| plugin.with_languages(Arc::new(move || read.lock().unwrap().clone())),
    );
    assert_eq!(langs(&mut h), "en,ru");
    *source.lock().unwrap() = vec!["de_DE".into(), "C".into(), "de".into()];
    assert_eq!(langs(&mut h), "de");
}

/// `locale.local-time-zone` answers what the zone source says, read on every call, and `UTC`
/// when it cannot tell. The harness pins the source to "cannot tell".
#[test]
fn kitchen_sink_reads_the_time_zone() {
    let zone =
        |h: &mut Harness| metadata(&h.run("kitchen-sink/indexed", "tz").unwrap())["zone"].clone();
    let source = Arc::new(Mutex::new(Some("Asia/Tokyo".to_string())));
    let read = Arc::clone(&source);
    let mut h = harness_built(
        |_| {},
        BTreeMap::new(),
        move |plugin| plugin.with_time_zone(Arc::new(move || read.lock().unwrap().clone())),
    );
    assert_eq!(zone(&mut h), "Asia/Tokyo");
    // The computer's zone changes: the next call follows it.
    *source.lock().unwrap() = Some("Europe/Moscow".into());
    assert_eq!(zone(&mut h), "Europe/Moscow");
    *source.lock().unwrap() = Some("not a zone".into());
    assert_eq!(zone(&mut h), "UTC");

    let mut h = harness_built(
        |_| {},
        BTreeMap::new(),
        |plugin| plugin.with_time_zone(Arc::new(|| None)),
    );
    assert_eq!(zone(&mut h), "UTC");
    assert_eq!(zone(&mut harness()), "UTC", "the harness's own source");
}

#[test]
fn oversized_list_view_is_rejected_with_toast() {
    let mut h = harness();
    // The ActionFailed reply is what the launcher shows as an error toast.
    let err = h.run("kitchen-sink/indexed", "big-view").unwrap_err();
    assert!(err.contains("2001 rows") && err.contains("2000"), "{err}");
    // The plugin is not broken: a cap violation only rejects the answer.
    assert!(h.run("kitchen-sink/hello", "run").is_ok());
}

/// Rejected `on-event` answers (which can come with every keystroke) get no effects and one
/// denial toast per 10 s, not one error per event.
#[test]
fn repeated_on_event_rejections_show_one_toast() {
    let mut h = harness();
    for _ in 0..3 {
        let event = ViewEvent::QueryChanged {
            revision: 1,
            text: "x".into(),
            readings: vec![],
        };
        assert_eq!(h.view_event("oversized", event), Ok(vec![]));
    }
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error);
    let message = toast.message.unwrap_or_default();
    assert!(message.contains("2001 rows"), "{message}");
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none(), "a second denial toast");
}

/// A replace-items list is counted as returned: 1 own item + 1000 foreign ones is over 1000,
/// although only one item would be kept.
#[test]
fn replace_items_counts_foreign_items_against_the_cap() {
    let mut h = harness();
    let err = h.run("kitchen-sink/indexed", "replace-flood").unwrap_err();
    assert!(err.contains("1001 entries, at most 1000"), "{err}");
    assert!(h.indexed("Flood").is_none());
    assert!(
        h.indexed("Indexed by init").is_some(),
        "nothing was replaced"
    );
}

/// Plugin-controlled text in an error or a rejection reaches the launcher bounded and on one
/// line: a guest error, a rejected run answer and a rejected search answer (denial toast).
#[test]
fn plugin_text_in_errors_is_bounded_and_one_line() {
    let short_line = |text: &str| text.len() <= 1100 && !text.contains('\n');
    let mut h = harness();
    let err = h.run("kitchen-sink/indexed", "err-flood").unwrap_err();
    assert!(short_line(&err), "{} bytes: {:.200}", err.len(), err);
    let err = h.run("kitchen-sink/indexed", "bad-shortcut").unwrap_err();
    assert!(err.contains("invalid answer"), "{err:.200}");
    assert!(short_line(&err), "{} bytes: {:.200}", err.len(), err);
    assert_eq!(h.search("bad-shortcut"), None);
    let toast = h.toast();
    let message = toast.message.unwrap_or_default();
    assert!(message.contains("invalid answer"), "{message:.200}");
    assert!(short_line(&message), "{} bytes", message.len());
}

#[test]
fn foreign_item_id_dropped() {
    let mut h = harness();
    let items = h.search("foreign-id").expect("an answer");
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["kitchen-sink/mine"]);
}

#[test]
fn search_over_50_items_is_rejected_with_one_toast() {
    let mut h = harness();
    assert_eq!(h.search("many:50").map(|items| items.len()), Some(50));
    assert_eq!(h.search("many:51"), None, "rejected as a whole");
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error);
    assert!(
        toast.message.as_deref().unwrap_or_default().contains("50"),
        "{toast:?}"
    );
    // Denial toasts are rate-limited: a second rejection within 10 s shows none.
    assert_eq!(h.search("many:51"), None);
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none(), "a second denial toast");
    // Not broken.
    assert!(h.search("many:1").is_some());
}

/// Review Focus: a plugin that answers every keystroke in 90 ms (under the 100 ms deadline).
/// Typing never waits for it, at most one answer per generation is applied while current,
/// fewer answers than keystrokes arrive, and the last keystroke gets its answer.
///
/// How stale answers are dropped is covered by the core tests in `crates/core/tests/host.rs`:
/// `searches_are_coalesced_per_module` (queued searches superseded by a newer one are skipped)
/// and `stale_generation_results_are_dropped` (an answer for an old generation is not
/// applied). Here the worker already skips them, so none reaches the core.
#[test]
fn slow_search_does_not_block_typing() {
    let mut h = harness();
    let mut slowest_keystroke = Duration::ZERO;
    let mut applied: BTreeMap<u64, usize> = BTreeMap::new();
    let mut answers = 0;
    let mut last = None;
    let mut handle = |h: &mut Harness, ev: CoreEvent, applied: &mut BTreeMap<u64, usize>| {
        if let CoreEvent::Instance { event, .. } = &ev
            && let CoreEvent::SearchResults { generation, .. } = **event
        {
            answers += 1;
            match h.apply(ev) {
                CoreUpdate::Results(snapshot) => {
                    assert_eq!(snapshot.generation, generation);
                    *applied.entry(generation).or_default() += 1;
                    let titles: Vec<String> = snapshot
                        .hits
                        .iter()
                        .filter(|hit| hit.dynamic)
                        .map(|hit| hit.item.title.clone())
                        .collect();
                    return Some((generation, titles));
                }
                // Stale for the core: possible in principle, dropped there.
                CoreUpdate::None => {}
                other => panic!("{other:?}"),
            }
        } else {
            h.apply(ev);
        }
        None
    };
    let mut generation = 0;
    for key in 1..=10 {
        let start = Instant::now();
        generation = h.core.search(&format!("slow:{key}"), now()).generation;
        slowest_keystroke = slowest_keystroke.max(start.elapsed());
        let typed = Instant::now();
        while typed.elapsed() < Duration::from_millis(15) {
            match h.rx.try_recv() {
                Ok(ev) => {
                    if let Some(answer) = handle(&mut h, ev, &mut applied) {
                        last = Some(answer);
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }
    // The answer to the last keystroke.
    let deadline = Instant::now() + Duration::from_secs(2);
    while last.as_ref().is_none_or(|(g, _)| *g != generation) {
        assert!(
            Instant::now() < deadline,
            "no answer for the last keystroke"
        );
        let ev = wait_event(&h.rx);
        if let Some(answer) = handle(&mut h, ev, &mut applied) {
            last = Some(answer);
        }
    }
    assert_eq!(last.unwrap().1, ["10"]);
    assert!(
        slowest_keystroke < Duration::from_millis(20),
        "typing waited for the plugin: {slowest_keystroke:?}"
    );
    assert!(applied.values().all(|&n| n == 1), "{applied:?}");
    assert!(
        answers < 10,
        "superseded searches were not skipped: {answers}"
    );
    // Under the deadline: the plugin is not broken.
    assert!(h.run("kitchen-sink/hello", "run").is_ok());
}
