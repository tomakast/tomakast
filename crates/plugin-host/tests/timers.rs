//! Plugin timers: `timers.start/cancel`, the quota and minimum intervals, and the firing that
//! reaches `on-event` and sends its effects unsolicited.

mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use support::harness::*;

const SINK: &str = "kitchen-sink/indexed";
/// How long a due event may take to arrive: generous, for a loaded CI machine. Only waits
/// for something that must come use it; checks that nothing comes wait a fixed time.
const ARRIVE: Duration = Duration::from_secs(10);

/// Starts a timer through the kitchen-sink and returns `timers.start`'s answer as the guest
/// printed it (`Ok(<id>)` or `Err("…")`).
fn start(h: &mut Harness, action: &str) -> String {
    let effects = h.run(SINK, action).expect("run answers");
    metadata(&effects)["result"].clone()
}

fn fired(h: &mut Harness, wait: Duration) -> Option<String> {
    next_detail(h, "timer-fired", wait).map(|m| m["id"].clone())
}

#[test]
fn one_shot_timer_fires_on_event_and_effects_arrive() {
    let mut h = harness();
    let started = Instant::now();
    assert_eq!(start(&mut h, "timer:50:once"), "Ok(1)");
    let id = fired(&mut h, ARRIVE).expect("the timer fires");
    assert_eq!(id, "1");
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "{:?}",
        started.elapsed()
    );
    // A one-shot timer fires once.
    assert_eq!(fired(&mut h, Duration::from_millis(300)), None);
    // Ids are not reused.
    assert_eq!(start(&mut h, "timer:10:once"), "Ok(2)");
    assert_eq!(fired(&mut h, ARRIVE).as_deref(), Some("2"));
}

/// Ruling AK: timer and HTTP events get at most 1 s of guest time per 10 s. A repeating timer
/// whose firings each spin 300 ms goes over budget at its third or fourth firing (900 ms plus
/// call overhead); then its timers are suspended (60 s), so nothing fires for a while, and the
/// plugin itself still runs.
#[test]
fn background_work_over_budget_suspends_timers() {
    let mut h = harness();
    assert_eq!(start(&mut h, "busy-timer:250:300"), "Ok(1)");
    next_detail(&mut h, "busy-fired", ARRIVE).expect("the busy timer fires");
    // Firings until the timer falls silent for 2 s, where a 250 ms timer would otherwise
    // fire about 8 times. Without a budget it never does: the loop is capped.
    let mut firings = 1;
    while next_detail(&mut h, "busy-fired", Duration::from_millis(2000)).is_some() {
        firings += 1;
        assert!(firings <= 6, "the timer kept firing over budget");
    }
    assert!(
        (3..=4).contains(&firings),
        "suspended after {firings} firings"
    );
    let alive = h
        .run("kitchen-sink/hello", "run")
        .expect("the plugin still runs");
    assert!(!alive.is_empty());
}

#[test]
fn repeating_timer_minimum_interval() {
    let mut h = harness();
    let refused = start(&mut h, "timer:249:repeat");
    assert!(
        refused.starts_with("Err") && refused.contains("250"),
        "{refused}"
    );
    let refused = start(&mut h, "timer:9:once");
    assert!(
        refused.starts_with("Err") && refused.contains("10"),
        "{refused}"
    );
    assert_eq!(start(&mut h, "timer:10:once"), "Ok(1)");
    assert_eq!(fired(&mut h, ARRIVE).as_deref(), Some("1"));

    assert_eq!(start(&mut h, "timer:250:repeat"), "Ok(2)");
    let window = Duration::from_millis(2600);
    let begin = Instant::now();
    let mut fires = 0;
    while begin.elapsed() < window {
        if let Some(id) = fired(&mut h, Duration::from_millis(50)) {
            assert_eq!(id, "2");
            fires += 1;
        }
    }
    // At most one fire per 250 ms (a load spike can only delay or skip fires, never add
    // any), and it does repeat.
    let most = (window.as_millis() / 250) as usize;
    assert!((2..=most).contains(&fires), "{fires} fires in {window:?}");
}

#[test]
fn timer_quota() {
    let mut h = harness();
    for i in 1..=16 {
        assert_eq!(start(&mut h, "timer:60000:once"), format!("Ok({i})"));
    }
    let refused = start(&mut h, "timer:60000:repeat");
    assert!(
        refused.starts_with("Err") && refused.contains("16"),
        "{refused}"
    );
    // Cancelling one frees a slot.
    h.run(SINK, "cancel:3").unwrap();
    assert_eq!(start(&mut h, "timer:60000:once"), "Ok(17)");
    assert!(start(&mut h, "timer:60000:once").starts_with("Err"));

    // A one-shot timer that fired no longer counts.
    let mut h = harness();
    for _ in 0..15 {
        start(&mut h, "timer:60000:once");
    }
    assert_eq!(start(&mut h, "timer:10:once"), "Ok(16)");
    assert_eq!(fired(&mut h, ARRIVE).as_deref(), Some("16"));
    assert_eq!(start(&mut h, "timer:60000:once"), "Ok(17)");
}

#[test]
fn timers_need_the_timers_permission() {
    let mut h = harness_with(|p| p.manifest.permissions.timers = false, BTreeMap::new());
    let refused = start(&mut h, "timer:10:once");
    assert!(
        refused.starts_with("Err") && refused.contains("permission"),
        "{refused}"
    );
    assert_eq!(fired(&mut h, Duration::from_millis(200)), None);
}

#[test]
fn cancel_stops_timer() {
    let mut h = harness();
    assert_eq!(start(&mut h, "timer:250:repeat"), "Ok(1)");
    assert_eq!(fired(&mut h, ARRIVE).as_deref(), Some("1"));
    h.run(SINK, "cancel:1").unwrap();
    assert_eq!(fired(&mut h, Duration::from_millis(700)), None);

    // A one-shot cancelled before it is due never fires; cancelling an unknown id is a no-op.
    assert_eq!(start(&mut h, "timer:200:once"), "Ok(2)");
    h.run(SINK, "cancel:2").unwrap();
    h.run(SINK, "cancel:99").unwrap();
    assert_eq!(fired(&mut h, Duration::from_millis(500)), None);
}

/// Timer 1's event cancels timer 2, whose event was already taken from the inbox with it (one
/// wake-up): timer 2 must not fire.
#[test]
fn a_timer_cancelled_by_an_earlier_event_does_not_fire() {
    let mut h = harness();
    assert_eq!(start(&mut h, "cancel-race"), "1 2");
    assert_eq!(fired(&mut h, Duration::from_millis(500)), None);
    // Timer 1 was delivered and timer 2 cancelled: both slots are free again.
    for _ in 0..16 {
        assert!(start(&mut h, "timer:60000:once").starts_with("Ok"));
    }
}

#[test]
fn huge_delays_are_accepted_and_never_fire() {
    let mut h = harness();
    // `Instant + u64::MAX ms` does not fit; the timer is kept but never comes due.
    let result = start(&mut h, &format!("timer:{}:once", u64::MAX));
    assert_eq!(result, "Ok(1)");
    let result = start(&mut h, &format!("timer:{}:repeat", u64::MAX));
    assert_eq!(result, "Ok(2)");
    // The timer thread survived: a normal timer still fires.
    assert_eq!(start(&mut h, "timer:10:once"), "Ok(3)");
    assert_eq!(fired(&mut h, ARRIVE).as_deref(), Some("3"));
}
