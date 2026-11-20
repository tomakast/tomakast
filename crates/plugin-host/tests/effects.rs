//! Permission and rate checks on effects: `copy-text` needs `clipboard`, `replace-items` needs
//! `index` and the 1000-item quota, toasts follow 10 per 10 s and unsolicited view updates 30
//! per s. A denied effect is dropped with one host denial toast per plugin per 10 s.

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use support::harness::*;
use tomakast_core::{Effect, ToastKind, ViewEvent};

const SINK: &str = "kitchen-sink/indexed";

fn copies(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CopyText(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn toasts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Toast(_)))
        .count()
}

fn updates(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::UpdateView(_)))
        .count()
}

fn assert_denial(h: &mut Harness, needle: &str) {
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    // The host's own toast, not prefixed like the plugin's toasts (Ruling AN).
    assert_eq!(toast.title, format!("Plugin {NAME}"), "{toast:?}");
    let message = toast.message.unwrap_or_default();
    assert!(message.contains(needle), "{message}");
}

/// Task 3 (W11): `copy-text` applies even while the launcher is hidden, so a background timer
/// could overwrite the clipboard. Without the `clipboard` permission the effect is dropped.
#[test]
fn copy_text_effect_without_clipboard_permission_is_dropped() {
    let mut h = harness_with(
        |p| p.manifest.permissions.clipboard = false,
        BTreeMap::new(),
    );
    let effects = h.run(SINK, "timer-reply:20:copy:from-timer").unwrap();
    assert_eq!(metadata(&effects)["result"], "Ok(1)");
    assert_denial(&mut h, "clipboard");
    h.settle(Duration::from_millis(100));
    assert!(copies(&h.unsolicited).is_empty(), "{:?}", h.unsolicited);
    // An answer to `run` is checked the same way; the rest of the answer is kept.
    let effects = h.run(SINK, "copy:from-run").unwrap();
    assert!(effects.is_empty(), "{effects:?}");

    // The twin: with the permission a copy in answer to `run` arrives.
    let mut h = harness();
    let effects = h.run(SINK, "copy:from-run").unwrap();
    assert_eq!(copies(&effects), ["from-run"]);
}

/// Ruling X: copy-text needs a user action. From a timer it is dropped even with the
/// `clipboard` permission.
#[test]
fn copy_text_from_a_timer_is_dropped_even_with_the_permission() {
    let mut h = harness();
    let effects = h.run(SINK, "timer-reply:20:copy:from-timer").unwrap();
    assert_eq!(metadata(&effects)["result"], "Ok(1)");
    assert_denial(&mut h, "user action");
    h.settle(Duration::from_millis(200));
    assert!(copies(&h.unsolicited).is_empty(), "{:?}", h.unsolicited);
    // In answer to `run` it still copies.
    let effects = h.run(SINK, "copy:from-run").unwrap();
    assert_eq!(copies(&effects), ["from-run"]);
}

/// Ruling AL: copy-text answered to `closed` (sent when the launcher hides),
/// `query-changed` or `text-changed` is dropped, even with the `clipboard` permission; in
/// answer to a chosen action or a submitted form it copies.
#[test]
fn copy_text_needs_a_chosen_action_not_any_view_event() {
    let mut h = harness();
    let view = "reply:copy:secret";
    for event in [
        ViewEvent::Closed,
        ViewEvent::QueryChanged {
            revision: 1,
            text: "a".into(),
        },
        ViewEvent::TextChanged {
            revision: 1,
            text: "a".into(),
        },
    ] {
        let effects = h.view_event(view, event.clone()).unwrap();
        assert!(copies(&effects).is_empty(), "{event:?}: {effects:?}");
    }
    assert_denial(&mut h, "user action");
    for event in [
        ViewEvent::ActionChosen {
            row_id: view.into(),
            action_id: "copy".into(),
        },
        ViewEvent::FormSubmitted { values: Vec::new() },
    ] {
        let effects = h.view_event(view, event.clone()).unwrap();
        assert_eq!(copies(&effects), ["secret"], "{event:?}");
    }
}

/// Ruling AD: at most one unsolicited replace-items per 5 s; `init`'s counts.
#[test]
fn unsolicited_replace_items_once_per_5_s() {
    let mut h = harness();
    assert!(h.indexed("Indexed by init").is_some());
    h.run(SINK, "timer-reply:20:replace-many:3").unwrap();
    assert_denial(&mut h, "replace-items");
    h.settle(Duration::from_millis(100));
    assert!(
        h.indexed("Replaced").is_none(),
        "the timer's replace-items was applied"
    );
    assert!(h.indexed("Indexed by init").is_some());
    // In answer to `run` it is not limited.
    h.run(SINK, "replace-many:3").unwrap();
    h.settle(Duration::from_millis(100));
    assert!(h.indexed("Replaced").is_some());
}

#[test]
fn replace_items_needs_index_permission_and_quota() {
    // init's replace-items is dropped with a denial toast; the manifest command stays.
    let mut h = harness_with(|p| p.manifest.permissions.index = false, BTreeMap::new());
    assert_denial(&mut h, "index");
    assert!(
        h.indexed("Say hello").is_some(),
        "commands belong to the host"
    );
    assert!(h.indexed("Indexed by init").is_none());
    let effects = h.run("kitchen-sink/hello", "run").unwrap();
    assert_eq!(toasts(&effects), 1, "the rest of the plugin works");
    let effects = h.run("kitchen-sink/hello", "replace-many:3").unwrap();
    assert!(effects.is_empty());
    h.settle(Duration::from_millis(50));
    assert!(h.indexed("Replaced").is_none());

    // With the permission: at most 1000 items.
    let mut h = harness();
    assert!(h.indexed("Indexed by init").is_some());
    h.run(SINK, "replace-many:1000").unwrap();
    h.settle(Duration::from_millis(50));
    assert!(h.indexed("Replaced").is_some());
    // The replaced items took `indexed`'s place; the manifest command stays.
    let err = h
        .run("kitchen-sink/hello", "replace-many:1001")
        .unwrap_err();
    assert!(err.contains("1000"), "{err}");
}

#[test]
fn toast_rate_limit() {
    let mut h = harness();
    let effects = h.run(SINK, "toasts:12").unwrap();
    assert_eq!(toasts(&effects), 10);
    assert_denial(&mut h, "toast");
    // The window is full: the next toast is dropped too, and no second denial toast comes.
    let effects = h.run(SINK, "toasts:1").unwrap();
    assert_eq!(toasts(&effects), 0);
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none(), "one denial toast per 10 s");
    // Other effects of the answer are kept.
    let effects = h.run(SINK, "echo-item").unwrap();
    assert_eq!(effects.len(), 1);
}

#[test]
fn unsolicited_view_updates_are_rate_limited() {
    let mut h = harness();
    h.run(SINK, "timer-reply:20:updates:40").unwrap();
    assert_denial(&mut h, "update");
    h.settle(Duration::from_millis(50));
    assert_eq!(updates(&h.unsolicited), 30);
    // Updates in answer to `run` are not unsolicited.
    let effects = h.run(SINK, "updates:40").unwrap();
    assert_eq!(updates(&effects), 40);
}
