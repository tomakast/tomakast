//! The weather plugin (`plugins/weather`) driven through the core, as the daemon drives it,
//! with Open-Meteo faked: its tiles on Home with a city and without one (the computer's
//! location), a city typed in the search, the lookup of a city, the calm states of no network,
//! a bad answer, an unknown city and a slow answer, the reading kept in storage, and a change
//! of city.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
const FORECAST: &str = include_str!("../../../plugins/weather/tests/fixtures/forecast-berlin.json");

const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search?";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast?";

/// Requests a responder saw, as `"<method> <url>"`.
type Seen = Arc<Mutex<Vec<String>>>;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Open-Meteo: Berlin and Москва (in Russian) are known, any other city is not; every
/// forecast is Berlin's. Answers wait while `held` is true (at most 10 s); every request is
/// recorded.
fn open_meteo(held: Arc<AtomicBool>) -> (Responder, Seen) {
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        log.lock().unwrap().push(format!("{method} {url}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while held.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let body = if url.starts_with(FORECAST_URL) {
            FORECAST
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

fn served() -> (Responder, Seen) {
    open_meteo(Arc::new(AtomicBool::new(false)))
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

/// The texts and icon names of `frame`, in order.
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

/// What `widget` shows once it shows `want` in its third text (the temperature, or `—`);
/// settles and draws again for up to 5 s.
fn wait_tile(h: &mut Harness, widget: &str, want: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let frame = h.render(widget).expect("a frame");
        let texts = shown(&frame);
        if texts.get(2).map(String::as_str) == Some(want) {
            return texts;
        }
        assert!(Instant::now() < deadline, "{widget} shows {texts:?}");
        h.settle(Duration::from_millis(20));
    }
}

/// What `widget` shows once its second text (the condition, or why there is none) is
/// `word`; settles and draws again for up to 5 s.
fn wait_word(h: &mut Harness, widget: &str, word: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let texts = tile(h, widget);
        if texts.get(1).map(String::as_str) == Some(word) {
            return texts;
        }
        assert!(Instant::now() < deadline, "{widget} shows {texts:?}");
        h.settle(Duration::from_millis(20));
    }
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

#[test]
fn a_home_tile_with_a_city_draws_that_city() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    home(
        &mut h,
        &[("now", Some("Berlin")), ("today", Some("Berlin"))],
    );
    let now = wait_tile(&mut h, "now", "3°");
    // The name is the list's.
    assert_eq!(now, ["icon:moon", "Clear", "3°", "Berlin, DE"]);
    let today = tile(&mut h, "today");
    assert_eq!(
        today[..5],
        ["icon:moon", "Clear", "3°", "Berlin, DE", "H 5°  L 2°"].map(String::from)
    );
    assert_eq!(today.last().unwrap(), "Updated just now");
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
        [Effect::CopyText("Berlin, DE: 3°, Clear, H 5° L 2°".into())]
    );
    // One forecast for both tiles of the place, and no lookup for a listed city.
    let seen = requests(&seen);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0].starts_with(&format!(
            "GET {FORECAST_URL}latitude=52.52437&longitude=13.41053"
        )),
        "{seen:?}"
    );
    h.settle(Duration::from_millis(50));
    no_toasts(&h);
}

#[test]
fn a_home_tile_without_a_city_follows_the_location() {
    let (responder, seen) = served();
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
    assert_eq!(wait_tile(&mut h, "today", "3°")[3], "Berlin, DE");
    let now = wait_tile(&mut h, "now", "3°");
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
    use tomakast_core::WidgetSize::{Quarter, Single};
    assert_eq!(
        sizes,
        [("now".to_string(), Quarter), ("today".to_string(), Single)]
    );
}

/// `now` in a single draws what `today` draws.
#[test]
fn now_in_a_single_draws_the_day() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    sized_home(&mut h, &[("now", Some(WidgetSize::Single), Some("Berlin"))]);
    let now = wait_tile(&mut h, "now", "3°");
    assert_eq!(
        now[..5],
        ["icon:moon", "Clear", "3°", "Berlin, DE", "H 5°  L 2°"].map(String::from)
    );
    assert_eq!(now.last().unwrap(), "Updated just now");
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
    let (responder, _) = open_meteo(Arc::clone(&held));
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

/// The reading of the computer's location is kept: it shows at once on the next start.
#[test]
fn a_kept_reading_shows_at_once_and_is_not_fetched_again() {
    let fetched = unix_now() - 60;
    let kept = format!(
        "city=\nunits=metric\nname=Berlin\nlat=52.52437\nlon=13.41053\ntemp=-4.2\n\
         feels=\nhumidity=\nwind=\ncode=71\nday=true\nhigh=\nlow=\nfetched={fetched}\n"
    );
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[("last", &kept)], responder);
    home(&mut h, &[("now", None)]);
    assert_eq!(
        tile(&mut h, "now"),
        ["icon:cloud-snow", "Snow", "\u{2212}4°", "Berlin"]
    );
    h.settle(Duration::from_millis(200));
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
    for query in [
        "fire",
        "settings",
        "Berl",
        "weather",
        "weather in",
        "погода в",
    ] {
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
