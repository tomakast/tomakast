//! `location.request` through the host, with kitchen-sink's `locate` action and fake location
//! sources (no test reaches a real one): the permission, the rounding and the accuracy, one
//! call for many requests, the kept answer, failures, the answer that comes unasked when the
//! sources change, and an answer that comes after the plugin went.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use support::harness::*;
use tomakast_core::{Effect, View};
use tomakast_plugin_host::{Fix, LocationSource, Locator, Source};

const SINK: &str = "kitchen-sink/indexed";

/// kitchen-sink with `source` as the location service.
fn located(source: LocationSource) -> Harness {
    harness_built(
        |_| {},
        BTreeMap::new(),
        move |plugin| plugin.with_location(source),
    )
}

/// A source that answers `answer` after `delay`, counting its calls.
fn counting(delay: Duration, answer: Result<Fix, String>) -> (LocationSource, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let source: LocationSource = Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(delay);
        answer.clone()
    });
    (source, calls)
}

/// The metadata of every `location` detail view the plugin pushed unasked so far, taken out
/// of `unsolicited`.
fn location_events(h: &mut Harness) -> Vec<BTreeMap<String, String>> {
    let mut found = Vec::new();
    h.unsolicited.retain(|effect| match effect {
        Effect::PushView(View::Detail(view)) if view.title == "location" => {
            found.push(view.metadata.iter().cloned().collect());
            false
        }
        _ => true,
    });
    found
}

/// Settles until `n` location events came, for up to 5 s; returns them.
fn wait_events(h: &mut Harness, n: usize) -> Vec<BTreeMap<String, String>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while events.len() < n {
        assert!(Instant::now() < deadline, "{events:?}, expected {n}");
        h.settle(Duration::from_millis(10));
        events.extend(location_events(h));
    }
    events
}

fn locate(h: &mut Harness) {
    let effects = h.run(SINK, "locate").expect("locate runs");
    assert!(effects.is_empty(), "{effects:?}");
}

fn fix(latitude: f64, longitude: f64, accuracy_km: u32) -> Fix {
    Fix {
        latitude,
        longitude,
        accuracy_km,
    }
}

fn at(latitude: &str, longitude: &str, accuracy_km: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("latitude".to_string(), latitude.to_string()),
        ("longitude".to_string(), longitude.to_string()),
        ("accuracy-km".to_string(), accuracy_km.to_string()),
    ])
}

#[test]
fn without_the_permission_the_request_fails() {
    let (source, calls) = counting(Duration::ZERO, Ok(fix(1., 2., 11)));
    let mut h = harness_built(
        |p| p.manifest.permissions.location = false,
        BTreeMap::new(),
        move |plugin| plugin.with_location(source),
    );
    let effects = h.run(SINK, "locate").unwrap();
    let error = metadata(&effects).remove("error").unwrap_or_default();
    assert_eq!(error, "the location permission was not granted");
    h.settle(Duration::from_millis(100));
    assert!(location_events(&mut h).is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn the_answer_is_rounded_to_a_tenth_of_a_degree_and_at_least_11_km() {
    let (source, _) = counting(Duration::ZERO, Ok(fix(52.52437, 13.41053, 2)));
    let mut h = located(source);
    locate(&mut h);
    assert_eq!(wait_events(&mut h, 1), [at("52.5", "13.4", "11")]);
}

#[test]
fn many_requests_make_one_call_and_the_answer_is_kept() {
    let (source, calls) = counting(
        Duration::from_millis(200),
        Ok(fix(-33.86785, 151.20732, 30)),
    );
    let mut h = located(source);
    for _ in 0..5 {
        locate(&mut h);
    }
    // One request waits: the five are one event.
    assert_eq!(wait_events(&mut h, 1), [at("-33.9", "151.2", "30")]);
    h.settle(Duration::from_millis(100));
    assert!(location_events(&mut h).is_empty(), "one event");
    // Asked again: the kept answer, at once, without a call.
    locate(&mut h);
    assert_eq!(wait_events(&mut h, 1), [at("-33.9", "151.2", "30")]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failure_reaches_the_plugin_and_is_kept_too() {
    let (source, calls) = counting(Duration::ZERO, Err("location services are off".into()));
    let mut h = located(source);
    locate(&mut h);
    let events = wait_events(&mut h, 1);
    assert_eq!(events[0]["error"], "location: location services are off");
    locate(&mut h);
    wait_events(&mut h, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A background event: nothing toasts.
    assert!(
        h.unsolicited.iter().all(|e| !matches!(e, Effect::Toast(_))),
        "{:?}",
        h.unsolicited
    );
}

#[test]
fn the_harness_never_asks_the_real_service() {
    let mut h = harness();
    locate(&mut h);
    let events = wait_events(&mut h, 1);
    assert_eq!(
        events[0]["error"],
        "location: tests never ask the location service"
    );
}

#[test]
fn an_answer_after_a_reload_is_discarded() {
    let (source, calls) = counting(Duration::from_millis(300), Ok(fix(1., 2., 11)));
    let mut h = located(source);
    locate(&mut h);
    // The instance that asked goes before the answer comes.
    h.reload();
    h.settle(Duration::from_millis(600));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(location_events(&mut h).is_empty(), "{:?}", h.unsolicited);
}

#[test]
fn a_change_of_sources_reaches_a_plugin_that_asked_before_unasked() {
    let source = |answer: Fix| -> Vec<Source> {
        vec![Source::new(
            "the city in the settings",
            Duration::from_secs(1),
            Arc::new(move || Ok(answer)),
        )]
    };
    let locator = Locator::with_sources(source(fix(52.52, 13.40, 11)));
    let shared = Arc::clone(&locator);
    let mut h = harness_built(
        |_| {},
        BTreeMap::new(),
        move |plugin| plugin.with_locator(shared),
    );
    locate(&mut h);
    assert_eq!(wait_events(&mut h, 1), [at("52.5", "13.4", "11")]);
    // The user sets another city: the plugin hears of it without asking.
    locator.set_sources(source(fix(55.75, 37.62, 11)));
    assert_eq!(wait_events(&mut h, 1), [at("55.8", "37.6", "11")]);
    // A coarse place says so.
    locator.set_sources(source(fix(55.75, 37.62, 500)));
    assert_eq!(wait_events(&mut h, 1), [at("55.8", "37.6", "500")]);
}

#[test]
fn a_plugin_that_never_asked_hears_nothing_of_a_change() {
    let locator = Locator::new(Arc::new(|| Ok(fix(1., 2., 11))));
    let shared = Arc::clone(&locator);
    let mut h = harness_built(
        |_| {},
        BTreeMap::new(),
        move |plugin| plugin.with_locator(shared),
    );
    locator.set_sources(vec![Source::new(
        "x",
        Duration::from_secs(1),
        Arc::new(|| Ok(fix(3., 4., 11))),
    )]);
    h.settle(Duration::from_millis(200));
    assert!(location_events(&mut h).is_empty());
}
