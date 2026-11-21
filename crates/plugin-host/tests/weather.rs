//! The weather plugin (`plugins/weather`) driven through the core, as the daemon drives it,
//! with Open-Meteo faked: its ten tiles on Home with a city and without one (the computer's
//! location), the places of Home kept fresh without drawing, air asked only for the air
//! tiles, the tiles in their sizes, a city typed in the search, the search's tile going off
//! screen and back, the lookup of a city, the calm states of no network, a bad answer, an
//! unknown city and a slow answer, the readings kept in storage per place, a change of city,
//! pressure in mm Hg, and many places waiting their turn.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use support::harness::{Harness, PluginSetup, offline, plugin_harness_with, sized_home_entry};
use tomakast_core::widget::{Frame, NodeKind, WidgetEvent, WidgetSize, params};
use tomakast_core::{Effect, Icon};
use tomakast_plugin_host::{Fix, LocationSource, Locator, PreferencesHandle, Responder, Source};

const ID: &str = "weather";

const BERLIN: &str = include_str!("../../../plugins/weather/tests/fixtures/geocoding-berlin.json");
const MOSCOW: &str =
    include_str!("../../../plugins/weather/tests/fixtures/geocoding-moscow-ru.json");
const MISS: &str = include_str!("../../../plugins/weather/tests/fixtures/geocoding-miss.json");
/// A forecast without hours: `now` shows 3°, Clear.
const FORECAST: &str = include_str!("../../../plugins/weather/tests/fixtures/forecast-berlin.json");
/// A forecast with the day's hours: `now` shows 7°, Overcast, at 14:15 in Berlin.
const DAY: &str = include_str!("../../../plugins/weather/tests/fixtures/forecast-berlin-day.json");
const AIR: &str = include_str!("../../../plugins/weather/tests/fixtures/air-berlin.json");

const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search?";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast?";
const AIR_URL: &str = "https://air-quality-api.open-meteo.com/";

/// The built-in list's Berlin.
const BERLIN_AT: &str = "latitude=52.52437&longitude=13.41053";

/// Requests a responder saw, as `"<method> <url>"`.
type Seen = Arc<Mutex<Vec<String>>>;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Open-Meteo: Berlin and Москва (in Russian) are known, any other city is not; every
/// forecast is `forecast` (Berlin's), every air answer Berlin's. Answers wait while `held` is
/// true (at most 10 s); every request is recorded.
fn open_meteo(held: Arc<AtomicBool>, forecast: &'static str) -> (Responder, Seen) {
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        log.lock().unwrap().push(format!("{method} {url}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while held.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let body = if url.starts_with(FORECAST_URL) {
            forecast
        } else if url.starts_with(AIR_URL) {
            AIR
        } else if url.starts_with(&format!("{GEOCODING}name=Berlin&"))
            || url.starts_with(&format!("{GEOCODING}name=smallburg&"))
        {
            BERLIN
        } else if url.starts_with(GEOCODING) && url.contains("&language=ru&") {
            MOSCOW
        } else if url.starts_with(GEOCODING) {
            MISS
        } else {
            return Err(format!("no fake for {url}"));
        };
        Ok((200, body.into()))
    });
    (responder, seen)
}

/// Open-Meteo answering at once, its forecasts without hours.
fn served() -> (Responder, Seen) {
    open_meteo(Arc::new(AtomicBool::new(false)), FORECAST)
}

/// Open-Meteo answering at once, its forecasts with the day's hours.
fn served_day() -> (Responder, Seen) {
    open_meteo(Arc::new(AtomicBool::new(false)), DAY)
}

/// A responder that gives every request the same answer, counting them.
fn answering(status: u16, body: &'static str) -> (Responder, Seen) {
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        log.lock().unwrap().push(format!("{method} {url}"));
        Ok((status, body.into()))
    });
    (responder, seen)
}

/// A location service at Berlin that counts how often it is asked.
fn counted_location() -> (Arc<Locator>, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let here: LocationSource = Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Fix {
            latitude: 52.52437,
            longitude: 13.41053,
            accuracy_km: 3,
        })
    });
    (Locator::new(here), asked)
}

/// The weather plugin with these preferences and stored entries, its requests answered by
/// `responder`; with the handle that changes its preferences. The location service refuses
/// (the harness's). Home is empty.
fn start(
    prefs: &[(&str, &str)],
    seed: &[(&str, &str)],
    responder: Responder,
) -> (Harness, PreferencesHandle) {
    start_located(prefs, seed, responder, None)
}

/// [`start`], with `location` as the location service when given.
fn start_located(
    prefs: &[(&str, &str)],
    seed: &[(&str, &str)],
    responder: Responder,
    location: Option<Arc<Locator>>,
) -> (Harness, PreferencesHandle) {
    let handle: Arc<Mutex<Option<PreferencesHandle>>> = Arc::default();
    let keep = Arc::clone(&handle);
    let prefs: BTreeMap<String, String> = prefs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let h = plugin_harness_with(
        ID,
        PluginSetup {
            prefs,
            seed: seed
                .iter()
                .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
                .collect(),
            responder,
        },
        move |plugin| {
            *keep.lock().unwrap() = Some(plugin.preferences());
            match location {
                Some(locator) => plugin.with_locator(locator),
                None => plugin,
            }
        },
    );
    let handle = handle
        .lock()
        .unwrap()
        .take()
        .expect("the preferences handle");
    (h, handle)
}

/// Puts weather tiles on Home: each a widget and its city, if any, in its default size.
fn home(h: &mut Harness, tiles: &[(&str, Option<&str>)]) {
    sized_home(
        h,
        &tiles
            .iter()
            .map(|(widget, city)| (*widget, None, *city))
            .collect::<Vec<_>>(),
    );
}

/// [`home`], each tile in a size of its own.
fn sized_home(h: &mut Harness, tiles: &[(&str, Option<WidgetSize>, Option<&str>)]) {
    let entries = tiles
        .iter()
        .map(|(widget, size, city)| {
            let city: Vec<(&str, &str)> = city.iter().map(|c| ("city", *c)).collect();
            sized_home_entry(ID, widget, *size, &city)
        })
        .collect();
    h.set_home(entries);
}

/// The weather plugin with one Home tile, `now` of `city`, its requests answered by
/// `responder`.
fn start_with_city(city: &str, responder: Responder) -> Harness {
    let (mut h, _) = start(&[], &[], responder);
    home(&mut h, &[("now", Some(city))]);
    h
}

/// The texts and icon names of `frame`, in order. Icons drawn on a canvas (the faded and
/// muted ones) are not among them.
fn shown(frame: &Frame) -> Vec<String> {
    frame
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Text(t) => Some(t.text.clone()),
            NodeKind::Icon(i) => match &i.icon {
                Icon::Named(name) => Some(format!("icon:{name}")),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// What `now` shows once it shows `want` in its third text (the temperature, or `—`);
/// settles and draws again for up to 5 s.
fn wait_tile(h: &mut Harness, widget: &str, want: &str) -> Vec<String> {
    assert_eq!(
        widget, "now",
        "the third text is the temperature only in `now`"
    );
    wait_until(h, widget, |texts| {
        texts.get(2).map(String::as_str) == Some(want)
    })
}

/// What `widget` shows once its second text (the condition, or why there is none) is
/// `word`; settles and draws again for up to 5 s.
fn wait_word(h: &mut Harness, widget: &str, word: &str) -> Vec<String> {
    wait_until(h, widget, |texts| {
        texts.get(1).map(String::as_str) == Some(word)
    })
}

/// What `widget` shows once it shows `value` ([`shows`]); settles and draws again for up to
/// 5 s.
fn wait_value(h: &mut Harness, widget: &str, value: &str) -> Vec<String> {
    wait_until(h, widget, |texts| shows(texts, value))
}

/// What `widget` shows once `done` holds for its texts; settles and draws again for up to
/// 5 s.
fn wait_until(h: &mut Harness, widget: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let texts = tile(h, widget);
        if done(&texts) {
            return texts;
        }
        assert!(Instant::now() < deadline, "{widget} shows {texts:?}");
        h.settle(Duration::from_millis(20));
    }
}

/// Whether `texts` show `value`: as one text ("8 h 27 min", "1"), or as a number and its
/// unit beside it ("76" "%", "1013" "hPa", "760" "mm Hg").
fn shows(texts: &[String], value: &str) -> bool {
    if texts.iter().any(|t| t == value) {
        return true;
    }
    let (number, unit) = match value.strip_suffix('%') {
        Some(number) => (number, "%"),
        None => match value.split_once(' ') {
            Some(parts) => parts,
            None => return false,
        },
    };
    texts.windows(2).any(|w| w[0] == number && w[1] == unit)
}

/// What `widget` shows now.
fn tile(h: &mut Harness, widget: &str) -> Vec<String> {
    shown(&h.render(widget).expect("a frame"))
}

/// Settles, drawing `now` again, for up to 5 s until `seen` holds `n` requests.
fn wait_requests(h: &mut Harness, seen: &Seen, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests(seen).len() < n {
        assert!(
            Instant::now() < deadline,
            "{:?}, expected {n}",
            requests(seen)
        );
        tile(h, "now");
        h.settle(Duration::from_millis(10));
    }
}

/// Settles, drawing nothing, for up to 5 s until `seen` holds `n` requests starting with
/// `url`.
fn settle_until(h: &mut Harness, seen: &Seen, url: &str, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while asked(seen, url).len() < n {
        assert!(
            Instant::now() < deadline,
            "{:?}, expected {n} of {url}",
            requests(seen)
        );
        h.settle(Duration::from_millis(10));
    }
}

fn no_toasts(h: &Harness) {
    let toasts: Vec<_> = h
        .unsolicited
        .iter()
        .filter(|e| matches!(e, Effect::Toast(_)))
        .collect();
    assert!(toasts.is_empty(), "{toasts:?}");
}

fn requests(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().clone()
}

/// The requests of `seen` starting with `url`.
fn asked(seen: &Seen, url: &str) -> Vec<String> {
    let prefix = format!("GET {url}");
    requests(seen)
        .into_iter()
        .filter(|r| r.starts_with(&prefix))
        .collect()
}

/// What storage keeps for a forecast of Berlin (the built-in list's coordinates) in metric
/// units, fetched at `fetched`: the plugin's own format, the raw answer on one line.
fn kept_berlin(name: &str, fetched: i64, body: &str) -> String {
    format!(
        "units=metric\nfetched={fetched}\nname={name}\nlat=52.52437\nlon=13.41053\nbody={}",
        body.replace(['\n', '\r'], " ")
    )
}

/// The `today` quarter of the day fixture at 14:15 in Berlin, without the icons drawn on a
/// canvas: the morning's temperature (over), the afternoon's icon, sky and temperature, the
/// evening's and the night's sky and temperature, and the place.
const TODAY_QUARTER: [&str; 9] = [
    "2°",
    "icon:sun",
    "icon:cloud-sun-rain",
    "7°",
    "icon:cloud-moon-rain",
    "5°",
    "icon:cloud-rain",
    "5°",
    "Berlin, DE",
];

#[test]
fn a_home_tile_with_a_city_draws_that_city() {
    let (responder, seen) = served_day();
    let (mut h, _) = start(&[], &[], responder);
    home(
        &mut h,
        &[("now", Some("Berlin")), ("today", Some("Berlin"))],
    );
    let now = wait_tile(&mut h, "now", "7°");
    // The name is the list's.
    assert_eq!(now, ["icon:cloud", "Overcast", "7°", "Berlin, DE"]);
    // `today` draws the four parts of the day and the place.
    assert_eq!(tile(&mut h, "today"), TODAY_QUARTER);
    // On Home a city's tile copies its weather too.
    let frame = h.frame("now").unwrap();
    let actions: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions, ["refresh", "copy"]);
    let copy = WidgetEvent::Action {
        node: None,
        action: "copy".into(),
    };
    assert_eq!(
        h.widget_event("now", copy).unwrap(),
        [Effect::CopyText(
            "Berlin, DE: 7°, Overcast, H 7° L 1°".into()
        )]
    );
    // One forecast for both tiles of the place, and no lookup for a listed city.
    let seen = requests(&seen);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0].starts_with(&format!("GET {FORECAST_URL}{BERLIN_AT}")),
        "{seen:?}"
    );
    h.settle(Duration::from_millis(50));
    no_toasts(&h);
}

#[test]
fn a_home_tile_without_a_city_follows_the_location() {
    let (responder, seen) = served_day();
    let here: LocationSource = Arc::new(|| {
        Ok(Fix {
            latitude: 52.52437,
            longitude: 13.41053,
            accuracy_km: 3,
        })
    });
    let (mut h, _) = start_located(&[], &[], responder, Some(Locator::new(here)));
    // `auto`, in any case, is the computer's location too.
    home(&mut h, &[("now", None), ("today", Some(" Auto "))]);
    let now = wait_tile(&mut h, "now", "7°");
    // `today` has the reading too: its parts, not why there is none.
    assert_eq!(tile(&mut h, "today"), TODAY_QUARTER);
    // Rounded by the launcher, named by the plugin's list.
    assert_eq!(now[3], "Berlin, DE");
    let seen = requests(&seen);
    assert_eq!(seen.len(), 1, "no lookup: {seen:?}");
    assert!(
        seen[0].starts_with(&format!("GET {FORECAST_URL}latitude=52.5&longitude=13.4&")),
        "{seen:?}"
    );
    no_toasts(&h);
}

#[test]
fn the_tiles_are_drawn_in_their_sizes() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    home(
        &mut h,
        &[("now", Some("Berlin")), ("today", Some("Berlin"))],
    );
    wait_tile(&mut h, "now", "3°");
    let sizes: Vec<_> = h
        .core
        .strip()
        .iter()
        .map(|t| (t.key.widget.widget.clone(), t.size))
        .collect();
    use tomakast_core::WidgetSize::Quarter;
    assert_eq!(
        sizes,
        [("now".to_string(), Quarter), ("today".to_string(), Quarter)]
    );
}

/// `now` in a single adds the day's high and low, the feel, the wind, the humidity and the
/// age.
#[test]
fn now_in_a_single_draws_the_day() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    sized_home(&mut h, &[("now", Some(WidgetSize::Single), Some("Berlin"))]);
    let now = wait_tile(&mut h, "now", "3°");
    assert_eq!(
        now,
        [
            "icon:moon",
            "Clear",
            "3°",
            "Berlin, DE",
            "↑ 5°  ↓ 2°",
            "\u{2212}1°",
            "6 km/h",
            "81%",
        ]
    );
}

/// The sizes of the strip's tiles, by widget.
fn sizes(h: &Harness) -> Vec<(String, WidgetSize)> {
    h.core
        .strip()
        .iter()
        .map(|t| (t.key.widget.widget.clone(), t.size))
        .collect()
}

/// Air quality and pollen as a quarter and as a single, drawn through the host: each frame is
/// accepted and shows the value of Berlin's air. Each size has a launcher of its own, so no
/// frame of another size is seen.
#[test]
fn air_quality_and_pollen_draw_in_their_sizes() {
    for size in [WidgetSize::Quarter, WidgetSize::Single] {
        let (responder, _) = served_day();
        let (mut h, _) = start(&[], &[], responder);
        sized_home(
            &mut h,
            &[
                ("air-quality", Some(size), Some("Berlin")),
                ("pollen", Some(size), Some("Berlin")),
            ],
        );
        let air = wait_value(&mut h, "air-quality", "19");
        assert_eq!(air[0], "Air quality", "{size:?}: {air:?}");
        assert_eq!(air.last().unwrap(), "Berlin, DE", "{size:?}: {air:?}");
        assert_eq!(
            air.contains(&"Good · PM2.5".to_string()),
            size == WidgetSize::Single,
            "{size:?}: {air:?}"
        );
        let pollen = wait_value(&mut h, "pollen", "No pollen data here");
        assert_eq!(pollen[..2], ["Pollen", "—"], "{size:?}: {pollen:?}");
        assert_eq!(
            sizes(&h),
            [
                ("air-quality".to_string(), size),
                ("pollen".to_string(), size)
            ]
        );
        no_toasts(&h);
    }
}

/// `today` as a single (the parts named, the place under them) and as a double (the place
/// first, then the day's range), drawn through the host.
#[test]
fn today_draws_as_a_single_and_a_double() {
    let (responder, _) = served_day();
    let (mut h, _) = start(&[], &[], responder);
    sized_home(
        &mut h,
        &[("today", Some(WidgetSize::Single), Some("Berlin"))],
    );
    let single = wait_value(&mut h, "today", "Afternoon");
    assert_eq!(single.last().unwrap(), "Berlin, DE", "{single:?}");
    assert_eq!(sizes(&h), [("today".to_string(), WidgetSize::Single)]);

    let (responder, _) = served_day();
    let (mut h, _) = start(&[], &[], responder);
    sized_home(
        &mut h,
        &[("today", Some(WidgetSize::Double), Some("Berlin"))],
    );
    let double = wait_value(&mut h, "today", "Today · ↑ 7° ↓ 1°");
    assert_eq!(
        double[..2],
        ["Berlin, DE", "Today · ↑ 7° ↓ 1°"],
        "{double:?}"
    );
    assert!(double.contains(&"Afternoon".to_string()), "{double:?}");
    assert_eq!(sizes(&h), [("today".to_string(), WidgetSize::Double)]);
    no_toasts(&h);
}

#[test]
fn no_network_is_calm_and_tries_again_later() {
    let (responder, seen) = {
        let seen = Seen::default();
        let log = Arc::clone(&seen);
        let off = offline();
        let responder: Responder = Arc::new(move |method: &str, url: &str| {
            log.lock().unwrap().push(format!("{method} {url}"));
            off(method, url)
        });
        (responder, seen)
    };
    let mut h = start_with_city("Berlin", responder);
    let now = wait_word(&mut h, "now", "No weather");
    assert_eq!(
        now,
        ["icon:cloud-off", "No weather", "—", "Offline, will retry"]
    );
    // Drawing the tile again does not hammer the network: the next try waits a minute.
    for _ in 0..5 {
        tile(&mut h, "now");
        h.settle(Duration::from_millis(20));
    }
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
    no_toasts(&h);
}

#[test]
fn bad_answers_are_calm() {
    for (name, status, body) in [
        ("status 500", 500, FORECAST),
        ("html", 200, "<html>"),
        ("an error", 400, r#"{"error":true,"reason":"bad"}"#),
    ] {
        let (responder, seen) = answering(status, body);
        let mut h = start_with_city("52.52, 13.41", responder);
        wait_word(&mut h, "now", "No weather");
        assert_eq!(requests(&seen).len(), 1, "{name}");
        no_toasts(&h);
    }
}

#[test]
fn an_unknown_city_says_so_and_is_not_asked_again() {
    let (responder, seen) = served();
    let mut h = start_with_city("Xyzzyqqplonk", responder);
    // The city as the user typed it.
    assert_eq!(
        wait_word(&mut h, "now", "Unknown city"),
        ["icon:map-pin-off", "Unknown city", "—", "Xyzzyqqplonk"]
    );
    for _ in 0..5 {
        tile(&mut h, "now");
        h.settle(Duration::from_millis(20));
    }
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
    // The user's refresh (Enter) asks once more.
    let refresh = WidgetEvent::Action {
        node: None,
        action: "refresh".into(),
    };
    h.widget_event("now", refresh).unwrap();
    wait_requests(&mut h, &seen, 2);
    for _ in 0..5 {
        tile(&mut h, "now");
        h.settle(Duration::from_millis(20));
    }
    assert_eq!(requests(&seen).len(), 2, "{:?}", requests(&seen));
    no_toasts(&h);
}

#[test]
fn a_cyrillic_city_is_looked_up_in_russian() {
    let (responder, seen) = served();
    let mut h = start_with_city("Небывальск", responder);
    let now = wait_tile(&mut h, "now", "3°");
    // The fixture's answer: Москва.
    assert_eq!(now[3], "Москва");
    assert!(
        requests(&seen)[0].contains(
            "name=%D0%BD%D0%B5%D0%B1%D1%8B%D0%B2%D0%B0%D0%BB%D1%8C%D1%81%D0%BA&count=1&language=ru"
        ),
        "{:?}",
        requests(&seen)
    );
}

#[test]
fn a_slow_answer_shows_loading_until_it_comes() {
    let held = Arc::new(AtomicBool::new(true));
    let (responder, _) = open_meteo(Arc::clone(&held), FORECAST);
    let (mut h, _) = start(&[], &[], responder);
    // A listed city by the list's name; one to look up as the user typed it.
    home(
        &mut h,
        &[("now", Some("Berlin")), ("today", Some(" Smallburg "))],
    );
    h.settle(Duration::from_millis(100));
    assert_eq!(
        tile(&mut h, "now"),
        ["icon:loader", "Loading", "—", "Berlin, DE"]
    );
    assert_eq!(
        tile(&mut h, "today"),
        ["icon:loader", "Loading", "—", "Smallburg"]
    );
    held.store(false, Ordering::SeqCst);
    wait_tile(&mut h, "now", "3°");
    no_toasts(&h);
}

/// The reading of the computer's location is kept: it shows at once on the next start. The
/// old key `last` goes.
#[test]
fn a_kept_reading_shows_at_once_and_is_not_fetched_again() {
    let kept = kept_berlin("Berlin", unix_now() - 60, FORECAST);
    let (responder, seen) = served();
    let (mut h, _) = start(
        &[],
        &[("forecast:@here", &kept), ("last", "city=\nunits=metric\n")],
        responder,
    );
    home(&mut h, &[("now", None)]);
    assert_eq!(tile(&mut h, "now"), ["icon:moon", "Clear", "3°", "Berlin"]);
    h.settle(Duration::from_millis(200));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
    assert_eq!(h.stored("last"), None);
}

/// A Home city's reading is kept under its own key: it draws at once after a restart, with
/// no request.
#[test]
fn a_kept_home_reading_draws_at_once_after_a_restart() {
    let kept = kept_berlin("Berlin, DE", unix_now(), DAY);
    let (responder, seen) = served_day();
    let (mut h, _) = start(&[], &[("forecast:berlin", &kept)], responder);
    home(&mut h, &[("now", Some("Berlin"))]);
    // The core draws a new Home tile before it tells the plugin its Home, so the first frame
    // may be the one before; the next is the kept reading's, and nothing was asked for it.
    wait_tile(&mut h, "now", "7°");
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
    h.settle(Duration::from_millis(300));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
}

#[test]
fn a_new_city_on_home_is_fetched_at_once() {
    let (responder, seen) = served();
    let (mut h, prefs) = start(&[], &[], responder);
    home(&mut h, &[("now", Some("Berlin"))]);
    wait_tile(&mut h, "now", "3°");
    home(&mut h, &[("now", Some("Небывальск"))]);
    // A geocoding and a forecast more.
    let now = wait_tile(&mut h, "now", "3°");
    assert_eq!(now[3], "Москва");
    h.settle(Duration::from_millis(50));
    assert_eq!(requests(&seen).len(), 3, "{:?}", requests(&seen));
    // °F asks the forecast again in Fahrenheit, without a lookup.
    prefs.set(BTreeMap::from([("fahrenheit".into(), "true".into())]));
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests(&seen).len() < 4 {
        assert!(Instant::now() < deadline, "{:?}", requests(&seen));
        tile(&mut h, "now");
        h.settle(Duration::from_millis(10));
    }
    assert!(requests(&seen)[3].contains("temperature_unit=fahrenheit"));
    no_toasts(&h);
}

/// The places of Home are fetched from the `home` event, though no tile is drawn; the
/// location is not asked, as no tile wants it.
#[test]
fn home_places_are_fetched_without_drawing() {
    let (responder, seen) = served();
    let (locator, located) = counted_location();
    let (mut h, _) = start_located(&[], &[], responder, Some(locator));
    home(
        &mut h,
        &[("now", Some("Berlin")), ("today", Some("Moscow"))],
    );
    settle_until(&mut h, &seen, FORECAST_URL, 2);
    let forecasts = asked(&seen, FORECAST_URL);
    assert!(
        forecasts.iter().any(|r| r.contains(BERLIN_AT)),
        "{forecasts:?}"
    );
    assert!(
        forecasts
            .iter()
            .any(|r| r.contains("latitude=55.75204&longitude=37.61781")),
        "{forecasts:?}"
    );
    h.settle(Duration::from_millis(100));
    assert_eq!(requests(&seen).len(), 2, "{:?}", requests(&seen));
    assert_eq!(
        located.load(Ordering::SeqCst),
        0,
        "the location is not asked"
    );
}

/// With nothing on Home and no search tile the plugin sends nothing and asks nothing.
#[test]
fn an_empty_home_sends_nothing() {
    let (responder, seen) = served();
    let (locator, located) = counted_location();
    let (mut h, _) = start_located(&[], &[], responder, Some(locator));
    h.settle(Duration::from_millis(300));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
    assert_eq!(
        located.load(Ordering::SeqCst),
        0,
        "the location is not asked"
    );
}

#[test]
fn air_is_asked_only_for_air_tiles() {
    let (responder, seen) = served_day();
    let (mut h, _) = start(&[], &[], responder);
    home(&mut h, &[("humidity", Some("Berlin"))]);
    settle_until(&mut h, &seen, FORECAST_URL, 1);
    h.settle(Duration::from_millis(200));
    assert!(asked(&seen, AIR_URL).is_empty(), "{:?}", requests(&seen));
    home(
        &mut h,
        &[
            ("humidity", Some("Berlin")),
            ("air-quality", Some("Berlin")),
            ("pollen", Some("Berlin")),
        ],
    );
    settle_until(&mut h, &seen, AIR_URL, 1);
    h.settle(Duration::from_millis(200));
    assert_eq!(asked(&seen, AIR_URL).len(), 1, "{:?}", requests(&seen));
    assert_eq!(asked(&seen, FORECAST_URL).len(), 1, "{:?}", requests(&seen));
    no_toasts(&h);
}

/// Six measures of one place: one forecast, and each tile shows its value from it.
#[test]
fn the_measures_draw_from_one_reading() {
    let (responder, seen) = served_day();
    let (mut h, _) = start(&[], &[], responder);
    let measures = [
        ("humidity", "76%"),
        ("wind", "18 km/h"),
        ("uv", "1"),
        ("pressure", "1013 hPa"),
        ("precipitation", "0 mm"),
        ("daylight", "8 h 27 min"),
    ];
    let tiles: Vec<(&str, Option<&str>)> =
        measures.iter().map(|(w, _)| (*w, Some("Berlin"))).collect();
    home(&mut h, &tiles);
    for (widget, value) in measures {
        let texts = wait_value(&mut h, widget, value);
        assert_eq!(texts.last().unwrap(), "Berlin, DE", "{widget}: {texts:?}");
    }
    h.settle(Duration::from_millis(100));
    let seen = requests(&seen);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].starts_with(&format!("GET {FORECAST_URL}{BERLIN_AT}")));
    no_toasts(&h);
}

#[test]
fn mmhg_shows_pressure_in_mm_hg_without_a_request() {
    let (responder, seen) = served_day();
    let (mut h, prefs) = start(&[], &[], responder);
    home(&mut h, &[("pressure", Some("Berlin"))]);
    wait_value(&mut h, "pressure", "1013 hPa");
    let before = requests(&seen).len();
    prefs.set(BTreeMap::from([("mmhg".into(), "true".into())]));
    wait_value(&mut h, "pressure", "760 mm Hg");
    h.settle(Duration::from_millis(200));
    assert_eq!(requests(&seen).len(), before, "{:?}", requests(&seen));
}

/// A place Home drops is forgotten: its kept reading goes from storage.
#[test]
fn a_removed_place_is_forgotten() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    home(&mut h, &[("now", Some("Berlin"))]);
    wait_tile(&mut h, "now", "3°");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.stored("forecast:berlin").is_none() {
        assert!(Instant::now() < deadline, "the reading is kept");
        h.settle(Duration::from_millis(10));
    }
    home(&mut h, &[]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.stored("forecast:berlin").is_some() {
        assert!(Instant::now() < deadline, "the reading is forgotten");
        h.settle(Duration::from_millis(10));
    }
}

/// After a reload the plugin is told its Home again and takes the place back from what it
/// kept: the tile draws with no new request.
#[test]
fn a_reloaded_plugin_draws_its_kept_place() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    home(&mut h, &[("now", Some("Berlin"))]);
    wait_tile(&mut h, "now", "3°");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.stored("forecast:berlin").is_none() {
        assert!(Instant::now() < deadline, "the reading is kept");
        h.settle(Duration::from_millis(10));
    }
    h.reload();
    wait_tile(&mut h, "now", "3°");
    h.settle(Duration::from_millis(300));
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
}

/// A search cleared before its query rests fetches nothing.
#[test]
fn a_cleared_search_sends_nothing() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    assert_eq!(h.search_answer("Moscow").widgets.len(), 1);
    h.core.search("", support::harness::now());
    h.settle(Duration::from_millis(500));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
}

/// A search tile that goes off screen (the launcher hid) is no longer kept fresh: a change of
/// units refetches nothing for it.
#[test]
fn a_hidden_search_tile_is_not_kept_fresh() {
    let (responder, seen) = served();
    let (mut h, prefs) = start(&[], &[], responder);
    searched(&mut h, "Moscow", "3°");
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
    // The core tells every tile on screen `Hidden` when the launcher hides; the harness runs
    // no strip schedule, so the event is sent to the search tile as the core sends it.
    let key = h.tile("now");
    h.core.send_widget_event(&key, WidgetEvent::Hidden).unwrap();
    h.settle(Duration::from_millis(100));
    prefs.set(BTreeMap::from([("fahrenheit".into(), "true".into())]));
    h.settle(Duration::from_millis(500));
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
}

/// A search tile that comes back on screen is kept fresh again: a change of units made while
/// it was off screen is fetched once it shows.
#[test]
fn a_search_tile_shown_again_is_kept_fresh_again() {
    let (responder, seen) = served();
    let (mut h, prefs) = start(&[], &[], responder);
    searched(&mut h, "Moscow", "3°");
    let key = h.tile("now");
    h.core.send_widget_event(&key, WidgetEvent::Hidden).unwrap();
    h.settle(Duration::from_millis(100));
    prefs.set(BTreeMap::from([("fahrenheit".into(), "true".into())]));
    h.settle(Duration::from_millis(300));
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
    h.core.send_widget_event(&key, WidgetEvent::Shown).unwrap();
    settle_until(&mut h, &seen, FORECAST_URL, 2);
    assert!(
        requests(&seen)[1].contains("temperature_unit=fahrenheit"),
        "{:?}",
        requests(&seen)
    );
}

/// An older search tile going off screen after a newer search answered (`weather berlin`,
/// then `weather berlin today`) does not end the newer search, though it shows the same
/// place.
#[test]
fn an_older_tile_going_off_screen_keeps_the_newer_search() {
    let (responder, seen) = served();
    let (mut h, prefs) = start(&[], &[], responder);
    searched(&mut h, "weather berlin", "3°");
    let older = h.tile("now");
    let answer = h.search_answer("weather berlin today");
    let [tile] = &answer.widgets[..] else {
        panic!("one tile: {answer:?}");
    };
    assert_eq!(tile.widget, "today");
    // The core hides the tile the new answer replaced after that answer.
    h.core
        .send_widget_event(&older, WidgetEvent::Hidden)
        .unwrap();
    h.settle(Duration::from_millis(500));
    assert_eq!(requests(&seen).len(), 1, "{:?}", requests(&seen));
    // Still the search's place: a change of units fetches it again.
    prefs.set(BTreeMap::from([("fahrenheit".into(), "true".into())]));
    settle_until(&mut h, &seen, FORECAST_URL, 2);
    assert!(
        requests(&seen)[1].contains("temperature_unit=fahrenheit"),
        "{:?}",
        requests(&seen)
    );
}

/// "Refresh Weather" fetches every wanted place at once, a fresh Home city included.
#[test]
fn the_refresh_command_fetches_every_place_at_once() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    home(
        &mut h,
        &[("now", Some("Berlin")), ("today", Some("Moscow"))],
    );
    settle_until(&mut h, &seen, FORECAST_URL, 2);
    h.settle(Duration::from_millis(100));
    h.run("weather/refresh", "run").unwrap();
    settle_until(&mut h, &seen, FORECAST_URL, 4);
    let forecasts = asked(&seen, FORECAST_URL);
    assert!(
        forecasts[2..].iter().any(|r| r.contains(BERLIN_AT)),
        "{forecasts:?}"
    );
    no_toasts(&h);
}

/// Six places at once: never more than four requests wait, and every place is asked once
/// the first answers come.
#[test]
fn many_places_wait_their_turn() {
    let held = Arc::new(AtomicBool::new(true));
    let (inner, seen) = open_meteo(Arc::clone(&held), FORECAST);
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (now_running, high) = (Arc::clone(&running), Arc::clone(&peak));
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        let n = now_running.fetch_add(1, Ordering::SeqCst) + 1;
        high.fetch_max(n, Ordering::SeqCst);
        let answer = inner(method, url);
        now_running.fetch_sub(1, Ordering::SeqCst);
        answer
    });
    let (mut h, _) = start(&[], &[], responder);
    let cities = ["Berlin", "Moscow", "Paris", "London", "Madrid", "Rome"];
    let tiles: Vec<(&str, Option<&str>)> = cities.iter().map(|c| ("now", Some(*c))).collect();
    home(&mut h, &tiles);
    settle_until(&mut h, &seen, FORECAST_URL, 1);
    h.settle(Duration::from_millis(300));
    assert!(requests(&seen).len() <= 4, "{:?}", requests(&seen));
    held.store(false, Ordering::SeqCst);
    settle_until(&mut h, &seen, FORECAST_URL, cities.len());
    h.settle(Duration::from_millis(100));
    assert_eq!(asked(&seen, FORECAST_URL).len(), cities.len());
    assert!(
        peak.load(Ordering::SeqCst) <= 4,
        "{}",
        peak.load(Ordering::SeqCst)
    );
    no_toasts(&h);
}

/// The tile `query` answers: `now`, with its params, once drawn with `want` as its
/// temperature.
fn searched(
    h: &mut Harness,
    query: &str,
    want: &str,
) -> (tomakast_core::widget::Params, Vec<String>) {
    let answer = h.search_answer(query);
    let [tile] = &answer.widgets[..] else {
        panic!("{query}: one tile: {answer:?}");
    };
    assert_eq!(tile.widget, "now");
    assert!(answer.items.is_empty());
    let shown = wait_tile(h, "now", want);
    (tile.params.clone(), shown)
}

#[test]
fn a_city_search_answers_now_with_the_city_param() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    let answer = h.search_answer("weather in Berlin");
    let [tile] = &answer.widgets[..] else {
        panic!("one tile: {answer:?}");
    };
    assert_eq!(tile.widget, "now");
    assert_eq!(tile.params, params([("city".into(), "Berlin, DE".into())]));
    assert_eq!(tile.size, None);
    let actions: Vec<&str> = tile.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions, ["copy"]);
    assert!(answer.items.is_empty());
    // Added to Home, the city comes back as the same place.
    home(&mut h, &[("now", Some("Berlin, DE"))]);
    h.core.search("", support::harness::now());
    assert_eq!(wait_tile(&mut h, "now", "3°")[3], "Berlin, DE");
}

/// Each widget's word answers that widget: with the city's param, or alone for where the
/// computer is.
#[test]
fn search_words_answer_their_widget() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    for (query, widget, city) in [
        ("humidity in Berlin", "humidity", Some("Berlin, DE")),
        ("weather today", "today", None),
        ("pollen", "pollen", None),
        ("weather", "now", None),
        ("погода", "now", None),
    ] {
        let answer = h.search_answer(query);
        let [tile] = &answer.widgets[..] else {
            panic!("{query}: one tile: {answer:?}");
        };
        assert_eq!(tile.widget, widget, "{query}");
        let want = params(city.map(|c| ("city".to_string(), c.to_string())));
        assert_eq!(tile.params, want, "{query}");
    }
}

#[test]
fn a_city_typed_in_the_search_shows_its_weather() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);

    let (city, shown) = searched(&mut h, "Moscow", "3°");
    assert_eq!(city, params([("city".into(), "Moscow, RU".into())]));
    // The fixture's forecast is Berlin's; the name is the list's.
    assert_eq!(shown, ["icon:moon", "Clear", "3°", "Moscow, RU"]);
    let asked = requests(&seen);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].starts_with(&format!(
            "GET {FORECAST_URL}latitude=55.75204&longitude=37.61781"
        )),
        "no lookup for a listed city: {asked:?}"
    );
    let (_, shown) = searched(&mut h, "москва", "3°");
    assert_eq!(shown[3], "Москва, RU");
    assert_eq!(requests(&seen).len(), 1, "the reading is reused");
    no_toasts(&h);
}

#[test]
fn other_queries_send_nothing() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    for query in ["fire", "settings", "Berl", "weather in", "погода в"] {
        let answer = h.search_answer(query);
        assert!(answer.widgets.is_empty(), "{query}: {answer:?}");
    }
    h.settle(Duration::from_millis(500));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
}

#[test]
fn weather_in_a_city_not_listed_looks_it_up() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    let (city, shown) = searched(&mut h, "weather in Smallburg", "3°");
    // Saved as typed; looked up by its key.
    assert_eq!(city, params([("city".into(), "Smallburg".into())]));
    assert_eq!(shown[3], "Berlin");
    assert!(
        requests(&seen)
            .iter()
            .any(|r| r.contains("name=smallburg&count=1&language=en")),
        "{:?}",
        requests(&seen)
    );
    let answer = h.search_answer("погода в Xyzzyqqplonk");
    assert_eq!(answer.widgets.len(), 1);
    let shown = wait_word(&mut h, "now", "Unknown city");
    assert_eq!(shown[3], "Xyzzyqqplonk");
    no_toasts(&h);
}

#[test]
fn enter_on_a_searched_city_copies_its_weather() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    searched(&mut h, "berlin", "3°");
    let effects = h
        .widget_event(
            "now",
            WidgetEvent::Action {
                node: None,
                action: "copy".into(),
            },
        )
        .unwrap();
    assert_eq!(
        effects,
        [Effect::CopyText("Berlin, DE: 3°, Clear, H 5° L 2°".into())]
    );
}

#[test]
fn a_coarse_location_is_marked() {
    let (responder, _) = served();
    // The time zone's place: Moscow, to 500 km.
    let zone: LocationSource = Arc::new(|| {
        Ok(Fix {
            latitude: 55.7558,
            longitude: 37.6178,
            accuracy_km: 500,
        })
    });
    let (mut h, _) = start_located(&[], &[], responder, Some(Locator::new(zone)));
    home(&mut h, &[("now", None)]);
    let now = wait_tile(&mut h, "now", "3°");
    assert_eq!(now[3], "≈ Moscow, RU");
    no_toasts(&h);
}

#[test]
fn a_wrong_city_in_the_launchers_settings_says_no_location() {
    let (responder, _) = served();
    let city = |answer: Result<Fix, String>| {
        vec![Source::new(
            "the city in the settings",
            Duration::from_secs(1),
            Arc::new(move || answer.clone()),
        )]
    };
    let locator = Locator::with_sources(city(Ok(Fix {
        latitude: 52.52,
        longitude: 13.40,
        accuracy_km: 11,
    })));
    let (mut h, _) = start_located(&[], &[], responder, Some(Arc::clone(&locator)));
    home(&mut h, &[("now", None)]);
    assert_eq!(wait_tile(&mut h, "now", "3°")[3], "Berlin, DE");
    // The user types a city that is not on the list: no quiet fall back to the old place.
    locator.set_sources(city(Err(
        "`Xyzzy` is not a city of the built-in list".into()
    )));
    wait_word(&mut h, "now", "No location");
    no_toasts(&h);
}

#[test]
fn no_location_is_calm() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    home(&mut h, &[("now", None)]);
    assert_eq!(
        wait_word(&mut h, "now", "No location"),
        [
            "icon:map-pin-off",
            "No location",
            "—",
            "Set a city in launcher settings"
        ]
    );
    h.settle(Duration::from_millis(100));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
    no_toasts(&h);
}
