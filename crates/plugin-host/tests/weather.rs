//! The weather plugin (`plugins/weather`) driven through the core, as the daemon drives it,
//! with Open-Meteo faked: its two tiles, the lookup of a city, the calm states of no network,
//! a bad answer, an unknown city and a slow answer, the reading kept in storage, and a change
//! of city.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use support::harness::{Harness, PluginSetup, offline, plugin_harness_with};
use tomakast_core::widget::{Frame, NodeKind, WidgetEvent};
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

/// The weather plugin with these preferences (`city` Berlin unless they set it) and stored
/// entries, its requests answered by `responder`; with the handle that changes its
/// preferences. The location service refuses (the harness's).
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
    let mut prefs: BTreeMap<String, String> = prefs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    prefs
        .entry("city".into())
        .or_insert_with(|| "Berlin".into());
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

/// Settles for up to 5 s until `seen` holds `n` requests.
fn wait_requests(h: &mut Harness, seen: &Seen, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests(seen).len() < n {
        assert!(
            Instant::now() < deadline,
            "{:?}, expected {n}",
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

#[test]
fn shows_two_tiles_with_the_weather_of_the_default_city() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[], responder);
    let now = wait_tile(&mut h, "now", "3°");
    // `init`'s answer, applied by now.
    assert_eq!(h.core.always_widgets(ID), ["now", "today"]);
    assert_eq!(now, ["icon:moon", "Clear", "3°", "Berlin"]);
    let today = tile(&mut h, "today");
    assert_eq!(
        today[..5],
        ["icon:moon", "Clear", "3°", "Berlin", "H 5°  L 2°"].map(String::from)
    );
    assert_eq!(today.last().unwrap(), "Updated just now");
    let seen = requests(&seen);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(
        seen[0].starts_with(&format!("GET {GEOCODING}name=Berlin&count=1&language=en")),
        "{seen:?}"
    );
    assert!(seen[1].starts_with(&format!(
        "GET {FORECAST_URL}latitude=52.52437&longitude=13.41053"
    )));
    let kept = h.stored("last").expect("the reading is kept");
    assert!(
        kept.contains("name=Berlin\n") && kept.contains("temp=2.6\n"),
        "{kept}"
    );
    h.settle(Duration::from_millis(50));
    no_toasts(&h);
}

#[test]
fn the_quarter_is_drawn_a_quarter_wide() {
    let (responder, _) = served();
    let (mut h, _) = start(&[], &[], responder);
    wait_tile(&mut h, "now", "3°");
    let strip = h.core.strip();
    let sizes: Vec<_> = strip
        .iter()
        .map(|t| (t.key.widget.as_str(), t.size))
        .collect();
    use tomakast_core::WidgetSize::{Quarter, Single};
    assert_eq!(sizes, [("now", Quarter), ("today", Single)]);
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
    let (mut h, _) = start(&[], &[], responder);
    let now = wait_word(&mut h, "now", "No weather");
    assert_eq!(
        now,
        ["icon:cloud-off", "No weather", "—", "Offline, will retry"]
    );
    // Showing the tile again does not hammer the network: the next try waits a minute.
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
        let (mut h, _) = start(&[("city", "52.52, 13.41")], &[], responder);
        wait_word(&mut h, "now", "No weather");
        assert_eq!(requests(&seen).len(), 1, "{name}");
        no_toasts(&h);
    }
}

#[test]
fn an_unknown_city_says_so_and_is_not_asked_again() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[("city", "Xyzzyqqplonk")], &[], responder);
    assert_eq!(
        wait_word(&mut h, "now", "Unknown city"),
        ["icon:map-pin-off", "Unknown city", "—", "Xyzzyqqplonk"]
    );
    h.settle(Duration::from_millis(100));
    assert_eq!(requests(&seen).len(), 1);
    // The user's refresh asks once more.
    h.run("weather/refresh", "run").unwrap();
    wait_requests(&mut h, &seen, 2);
    h.settle(Duration::from_millis(100));
    assert_eq!(requests(&seen).len(), 2);
    no_toasts(&h);
}

#[test]
fn a_cyrillic_city_is_found_in_russian() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[("city", "Москва")], &[], responder);
    let now = wait_tile(&mut h, "now", "3°");
    assert_eq!(now[3], "Москва");
    assert!(
        requests(&seen)[0]
            .contains("name=%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0&count=1&language=ru"),
        "{:?}",
        requests(&seen)
    );
}

#[test]
fn a_slow_answer_shows_loading_until_it_comes() {
    let held = Arc::new(AtomicBool::new(true));
    let (responder, _) = open_meteo(Arc::clone(&held));
    let (mut h, _) = start(&[], &[], responder);
    h.settle(Duration::from_millis(100));
    assert_eq!(
        tile(&mut h, "now"),
        ["icon:loader", "Loading", "—", "Berlin"]
    );
    held.store(false, Ordering::SeqCst);
    wait_tile(&mut h, "now", "3°");
    no_toasts(&h);
}

#[test]
fn a_kept_reading_shows_at_once_and_is_not_fetched_again() {
    let fetched = unix_now() - 60;
    let kept = format!(
        "city=Berlin\nunits=metric\nname=Berlin\nlat=52.52437\nlon=13.41053\ntemp=-4.2\n\
         feels=\nhumidity=\nwind=\ncode=71\nday=true\nhigh=\nlow=\nfetched={fetched}\n"
    );
    let (responder, seen) = served();
    let (mut h, _) = start(&[], &[("last", &kept)], responder);
    assert_eq!(
        tile(&mut h, "now"),
        ["icon:cloud-snow", "Snow", "\u{2212}4°", "Berlin"]
    );
    h.settle(Duration::from_millis(200));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
}

#[test]
fn a_new_city_is_fetched_at_once() {
    let (responder, seen) = served();
    let (mut h, prefs) = start(&[], &[], responder);
    wait_tile(&mut h, "now", "3°");
    prefs.set(BTreeMap::from([("city".into(), "Москва".into())]));
    // A geocoding and a forecast more; the Loading tile names the city too, so wait for
    // both requests, then for the drawn reading.
    wait_requests(&mut h, &seen, 4);
    let now = wait_tile(&mut h, "now", "3°");
    assert_eq!(now[3], "Москва");
    h.settle(Duration::from_millis(50));
    assert_eq!(requests(&seen).len(), 4, "{:?}", requests(&seen));
    // °F asks the forecast again in Fahrenheit, without a lookup.
    prefs.set(BTreeMap::from([
        ("city".into(), "Москва".into()),
        ("fahrenheit".into(), "true".into()),
    ]));
    wait_requests(&mut h, &seen, 5);
    assert!(requests(&seen)[4].contains("temperature_unit=fahrenheit"));
    no_toasts(&h);
}

/// The `city` tile `query` answers: its payload, once drawn with `want` as its temperature.
fn searched(h: &mut Harness, query: &str, want: &str) -> (String, Vec<String>) {
    let answer = h.search_answer(query);
    let [tile] = &answer.widgets[..] else {
        panic!("{query}: one tile: {answer:?}");
    };
    assert_eq!(tile.widget, "city");
    assert!(answer.items.is_empty());
    let shown = wait_tile(h, "city", want);
    (tile.payload.clone(), shown)
}

#[test]
fn a_city_typed_in_the_search_shows_its_weather() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[("city", "52.52, 13.41")], &[], responder);
    wait_tile(&mut h, "now", "3°");
    let before = requests(&seen).len();

    let (_, shown) = searched(&mut h, "Moscow", "3°");
    // The fixture's forecast is Berlin's; the name is the list's.
    assert_eq!(shown, ["icon:moon", "Clear", "3°", "Moscow, RU"]);
    let asked = &requests(&seen)[before..];
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].starts_with(&format!(
            "GET {FORECAST_URL}latitude=55.75204&longitude=37.61781"
        )),
        "no lookup for a listed city: {asked:?}"
    );
    let (_, shown) = searched(&mut h, "москва", "3°");
    assert_eq!(shown[3], "Москва, RU");
    assert_eq!(requests(&seen).len(), before + 1, "the reading is reused");
    no_toasts(&h);
}

#[test]
fn other_queries_send_nothing() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[("city", "52.52, 13.41")], &[], responder);
    wait_tile(&mut h, "now", "3°");
    let before = requests(&seen).len();
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
    assert_eq!(requests(&seen).len(), before, "{:?}", requests(&seen));
}

#[test]
fn weather_in_a_city_not_listed_looks_it_up() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[("city", "52.52, 13.41")], &[], responder);
    wait_tile(&mut h, "now", "3°");
    let (_, shown) = searched(&mut h, "weather in Smallburg", "3°");
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
    let shown = wait_word(&mut h, "city", "Unknown city");
    assert_eq!(shown[3], "xyzzyqqplonk");
    no_toasts(&h);
}

#[test]
fn enter_on_the_city_tile_copies_its_weather() {
    let (responder, _) = served();
    let (mut h, _) = start(&[("city", "52.52, 13.41")], &[], responder);
    searched(&mut h, "berlin", "3°");
    let effects = h
        .widget_event(
            "city",
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
fn auto_shows_the_weather_where_the_computer_is() {
    let (responder, seen) = served();
    let here: LocationSource = Arc::new(|| {
        Ok(Fix {
            latitude: 52.52437,
            longitude: 13.41053,
            accuracy_km: 3,
        })
    });
    let (mut h, _) = start_located(
        &[("city", "auto")],
        &[],
        responder,
        Some(Locator::new(here)),
    );
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
fn auto_marks_a_coarse_location() {
    let (responder, _) = served();
    // The time zone's place: Moscow, to 500 km.
    let zone: LocationSource = Arc::new(|| {
        Ok(Fix {
            latitude: 55.7558,
            longitude: 37.6178,
            accuracy_km: 500,
        })
    });
    let (mut h, _) = start_located(
        &[("city", "auto")],
        &[],
        responder,
        Some(Locator::new(zone)),
    );
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
    let (mut h, _) = start_located(
        &[("city", "auto")],
        &[],
        responder,
        Some(Arc::clone(&locator)),
    );
    assert_eq!(wait_tile(&mut h, "now", "3°")[3], "Berlin, DE");
    // The user types a city that is not on the list: no quiet fall back to the old place.
    locator.set_sources(city(Err(
        "`Xyzzy` is not a city of the built-in list".into()
    )));
    wait_word(&mut h, "now", "No location");
    no_toasts(&h);
}

#[test]
fn auto_without_a_location_is_calm() {
    let (responder, seen) = served();
    let (mut h, _) = start(&[("city", "auto")], &[], responder);
    assert_eq!(
        wait_word(&mut h, "now", "No location"),
        [
            "icon:map-pin-off",
            "No location",
            "—",
            "Set a city in preferences"
        ]
    );
    h.settle(Duration::from_millis(100));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
    no_toasts(&h);
}
