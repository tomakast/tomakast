//! Weather plugin: the current weather from Open-Meteo (free, no key) in two widgets for the
//! launcher's strip, `now` (a quarter, or a single) and `today` (a single). The user puts
//! them on Home; the plugin puts nothing there.
//!
//! - A tile with the param `city` shows that city (see [`search`]). It is fetched while the
//!   tile is drawn on Home, and once the query rests [`SEARCH_REST_MS`] for a search tile.
//! - A tile without it shows the computer's location, asked of the launcher
//!   (`location.request`) and named after the biggest city of the built-in list near it.
//!   `fahrenheit` switches to °F and mph.
//! - `init` restores the last place and reading of the location from storage (when they were
//!   kept for the same units) and starts a [`TICK_MS`] timer. Every tick, a tile coming on
//!   screen and a change of preferences fetch what is due (see [`state`]): a reading 15
//!   minutes old, after a failure not before its wait. The "Refresh Weather" command and
//!   Enter on a tile without a city fetch the location's weather at once; Enter on a tile with
//!   a city fetches that city's at once (a city geocoding did not know is asked again).
//! - Every answer that changes what the tiles show redraws them; the location's is written
//!   to storage. Failures never toast: the tiles keep the last reading and say how old it is,
//!   or say calmly why there is none. They are logged once per run of failures.
//! - Search: a city of the built-in list (`tomakast_places::cities`, read at `init`), or
//!   `weather in <city>`, answers `now` with the param `city` (see [`search`]); Enter copies
//!   the weather in one line.
//! - The `home` event (the plugin's Home tiles) is not used yet.

pub mod api;
pub mod json;
pub mod search;
pub mod state;
pub mod tile;

use std::cell::{OnceCell, RefCell};
use std::time::{SystemTime, UNIX_EPOCH};

use tomakast_plugin::{
    Coordinates, Effect, Event, HttpDone, HttpRequest, Item, Plugin, QueryReading, SearchAnswer,
    TimerId, WidgetAnswer, WidgetContext, WidgetEvent, WidgetFrame, WidgetParam, WidgetTarget,
    export_plugin, http, location, log, param, preferences, storage, timers,
};

use api::{Place, Units};
use search::{Ask, Searched};
use state::{Settings, Weather};
use tile::{CITY, NOW, TODAY};
use tomakast_places::cities::Cities;

/// How often the plugin looks whether a fetch is due: a minute, so a wait after a failure
/// ends on time. A tick that finds nothing due costs microseconds.
pub const TICK_MS: u64 = 60_000;
/// The storage key of the last place and reading.
const KEY: &str = "last";
/// How long a query must stay the same before its tile fetches: typing `San Francisco`
/// passes `San`, a city too.
pub const SEARCH_REST_MS: u64 = 300;

struct WeatherPlugin;

thread_local! {
    static STATE: RefCell<Weather> = RefCell::default();
    static TICK: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    static CITIES: OnceCell<Cities> = const { OnceCell::new() };
    static SEARCHED: RefCell<Searched> = RefCell::default();
    /// The ask of the latest search, and the timer that fetches for it once the query rests.
    static ASKED: RefCell<Option<(Option<TimerId>, Ask)>> = const { RefCell::new(None) };
}

fn with_cities<R>(f: impl FnOnce(&Cities) -> R) -> R {
    CITIES.with(|c| f(c.get_or_init(Cities::builtin)))
}

fn with_searched<R>(f: impl FnOnce(&mut Searched) -> R) -> R {
    SEARCHED.with_borrow_mut(f)
}

fn with<R>(f: impl FnOnce(&mut Weather) -> R) -> R {
    STATE.with_borrow_mut(f)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The preferences now. The tiles without a city show the computer's location: the state's
/// city is always `auto` (a tile's own city is its param).
fn settings() -> Settings {
    let fahrenheit = preferences::get("fahrenheit").is_some_and(|v| v == "true");
    Settings {
        city: String::new(),
        units: if fahrenheit {
            Units::Imperial
        } else {
            Units::Metric
        },
    }
}

fn redraw() -> Vec<Effect> {
    vec![Effect::redraw_widget(NOW), Effect::redraw_widget(TODAY)]
}

/// Starts the fetch that is due, if any (`forced`: the user asked). A fetch that cannot
/// start counts as a failure.
fn refresh(forced: bool) {
    let now = unix_now();
    with(|w| {
        let Some(fetch) = w.due(now, forced) else {
            return;
        };
        let request = HttpRequest::get(fetch.url.clone()).header("accept", "application/json");
        match http::fetch(&request) {
            Ok(id) => w.started(&fetch, id),
            Err(e) => {
                w.start_failed(now);
                note_failure(w, &e);
            }
        }
    });
}

/// Logs a failure: the first of a run as a warning, the next ones at debug level only, so a
/// long outage does not flood the log.
fn note_failure(w: &Weather, why: &str) {
    if w.failures() == 1 {
        log::warn(&format!("weather fetch failed: {why}; trying again later"));
    } else {
        log::debug(&format!("weather fetch failed again: {why}"));
    }
}

/// An HTTP answer: a city's (a search's or a Home tile's), or the location's.
fn answered(done: HttpDone) -> Vec<Effect> {
    if with_searched(|s| s.waits_for(done.request)) {
        return search_answered(done);
    }
    let now = unix_now();
    let changed = with(|w| {
        let failures = w.failures();
        let (answer, why) = match &done.response {
            Ok(r) => (
                Ok((r.status, r.body.as_slice())),
                format!("status {}, {} bytes it cannot read", r.status, r.body.len()),
            ),
            Err(e) => (Err(e.clone()), e.clone()),
        };
        let changed = w.answered(done.request, answer, now);
        if w.failures() > failures {
            note_failure(w, &why);
        }
        if changed
            && let Some(text) = w.encode()
            && let Err(e) = storage::set(KEY, &text)
        {
            log::debug(&format!("keeping the weather failed: {e}"));
        }
        changed
    });
    refresh(false);
    if changed { redraw() } else { Vec::new() }
}

/// An answer to a city's request: taken in, the next fetch of the latest search's ask
/// started (a geocoded city's forecast), and the tiles redrawn (a Home tile with a city
/// starts its own next fetch when drawn).
fn search_answered(done: HttpDone) -> Vec<Effect> {
    let now = unix_now();
    with_searched(|s| {
        let answer = match &done.response {
            Ok(r) => Ok((r.status, r.body.as_slice())),
            Err(e) => {
                log::debug(&format!("weather of a searched city failed: {e}"));
                Err(e.clone())
            }
        };
        s.answered(done.request, answer, now);
    });
    if let Some(ask) = ASKED.with_borrow(|a| a.as_ref().map(|(_, ask)| ask.clone())) {
        fetch_for(&ask, false);
    }
    redraw()
}

/// Starts the request `ask` needs, if any (`forced`: the user asked).
fn fetch_for(ask: &Ask, forced: bool) {
    let now = unix_now();
    with_searched(|s| {
        let Some((job, url)) = s.due(ask, now, forced) else {
            return;
        };
        let request = HttpRequest::get(url).header("accept", "application/json");
        match http::fetch(&request) {
            Ok(id) => s.started(job, id),
            Err(e) => {
                log::debug(&format!("weather of a searched city failed: {e}"));
                s.start_failed(&job, now);
            }
        }
    });
}

/// A city of the built-in list is named for the location when one is this near.
const NAME_WITHIN_KM: f64 = 50.;

/// Asks the launcher for the computer's location when the city is `auto` and it is due
/// (`forced`: the user asked). A request that cannot be made is taken as the answer.
fn locate(forced: bool) {
    let now = unix_now();
    let failed = with(|w| {
        if !w.wants_location(now, forced) {
            return None;
        }
        w.location_asked(now);
        location::request().err()
    });
    if let Some(why) = failed {
        log::warn(&format!("asking for the location failed: {why}"));
        with(|w| w.located(Err(why)));
    }
}

/// The launcher's answer to `location.request` (or its news of a change): the place, named
/// (`≈` when coarse), or why there is none.
fn located(answer: Result<Coordinates, String>) -> Vec<Effect> {
    let answer = answer.map(|at| {
        let name = with_cities(|c| c.nearest(at.latitude, at.longitude, NAME_WITHIN_KM))
            .unwrap_or_else(|| format!("{}, {}", at.latitude, at.longitude));
        let place = Place {
            name: if at.accuracy_km > state::COARSE_KM {
                format!("≈ {name}")
            } else {
                name
            },
            latitude: at.latitude,
            longitude: at.longitude,
        };
        (place, at.accuracy_km)
    });
    if let Err(why) = &answer {
        log::info(&format!("the location is not known: {why}"));
    }
    let changed = with(|w| {
        let changed = w.located(answer);
        if changed
            && let Some(text) = w.encode()
            && let Err(e) = storage::set(KEY, &text)
        {
            log::debug(&format!("keeping the weather failed: {e}"));
        }
        changed
    });
    refresh(false);
    if changed { redraw() } else { Vec::new() }
}

/// Remembers `ask` as the latest search's, and starts its rest timer in place of the last
/// one; `None` forgets it.
fn asked(ask: Option<Ask>) {
    let old = ASKED.take();
    if let Some((Some(timer), _)) = old {
        timers::cancel(timer);
    }
    let Some(ask) = ask else {
        return;
    };
    let timer = timers::start(SEARCH_REST_MS, false)
        .map_err(|e| log::debug(&format!("starting the search timer failed: {e}")))
        .ok();
    ASKED.set(Some((timer, ask)));
}

/// The ask of the latest search, if `timer` is its rest timer.
fn rested(timer: TimerId) -> Option<Ask> {
    ASKED.with_borrow(|a| {
        a.as_ref()
            .filter(|(t, _)| *t == Some(timer))
            .map(|(_, ask)| ask.clone())
    })
}

/// The place a tile's param `city` names, if it has one.
fn tile_city(params: &[WidgetParam]) -> Option<Ask> {
    let city = param(params, CITY)?;
    with_cities(|cities| search::city(city, cities))
}

/// An event of a tile with a city: `copy` copies its weather in one line, `refresh` fetches
/// it at once.
fn city_event(ask: &Ask, event: WidgetEvent) -> Vec<Effect> {
    match event {
        WidgetEvent::Action(a) if a.action_id == "copy" => {
            with_searched(|s| tile::summary(&s.view(ask)))
                .map(|text| vec![Effect::copy_text(text)])
                .unwrap_or_default()
        }
        WidgetEvent::Action(a) if a.action_id == "refresh" => {
            fetch_for(ask, true);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// An event of a tile without a city, the location's.
fn location_event(event: WidgetEvent) -> Vec<Effect> {
    match event {
        // A tile coming on screen fetches a reading that went stale meanwhile
        // (timers may sleep while the launcher is hidden).
        WidgetEvent::Shown => {
            refresh(false);
            Vec::new()
        }
        WidgetEvent::Action(a) if a.action_id == "refresh" => {
            locate(true);
            refresh(true);
            redraw()
        }
        _ => Vec::new(),
    }
}

impl Plugin for WeatherPlugin {
    fn init() -> Result<Vec<Effect>, String> {
        // The built-in list is read here, where there are 2 s, not in the first search.
        with_cities(|_| {});
        with(|w| {
            w.set_settings(settings());
            with_searched(|s| s.set_units(w.settings.units));
            match storage::get(KEY) {
                Ok(Some(text)) => w.restore(&text),
                Ok(None) => {}
                Err(e) => log::debug(&format!("reading the last weather failed: {e}")),
            }
        });
        locate(false);
        match timers::start(TICK_MS, true) {
            Ok(tick) => TICK.set(Some(tick)),
            Err(e) => log::warn(&format!("starting the weather timer failed: {e}")),
        }
        refresh(false);
        Ok(Vec::new())
    }

    fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
        // The query as typed, then as read on another keyboard layout.
        let ask = with_cities(|cities| {
            std::iter::once(query.as_str())
                .chain(readings.iter().map(|r| r.text.as_str()))
                .find_map(|text| search::ask(text, cities))
        });
        let Some(ask) = ask else {
            asked(None);
            return Ok(Vec::new().into());
        };
        let tile = WidgetAnswer::new(NOW)?
            .param(CITY, ask.param())
            .action("copy", "Copy Weather")?;
        asked(Some(ask));
        Ok(SearchAnswer {
            items: Vec::new(),
            widgets: vec![tile],
        })
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        if item.id == "weather/refresh" && action_id == "run" {
            locate(true);
            refresh(true);
            return Ok(redraw());
        }
        Ok(Vec::new())
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        Ok(match event {
            Event::TimerFired(timer) if TICK.with_borrow(|t| *t == Some(timer)) => {
                locate(false);
                refresh(false);
                Vec::new()
            }
            Event::TimerFired(timer) => {
                if let Some(ask) = rested(timer) {
                    fetch_for(&ask, false);
                }
                Vec::new()
            }
            Event::HttpDone(done) => answered(done),
            Event::Location(answer) => located(answer),
            Event::PreferencesChanged => {
                let settings = settings();
                with_searched(|s| s.set_units(settings.units));
                if with(|w| w.set_settings(settings)) {
                    locate(false);
                    refresh(false);
                    redraw()
                } else {
                    Vec::new()
                }
            }
            Event::Widget(WidgetTarget { params, event, .. }) => match tile_city(&params) {
                Some(ask) => city_event(&ask, event),
                None => location_event(event),
            },
            // Part 3 keeps the places of the Home tiles fresh with it.
            Event::Home(_) => Vec::new(),
            _ => Vec::new(),
        })
    }

    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        let now = unix_now();
        match tile_city(&context.params) {
            Some(ask) => {
                // A Home tile fetches its city while it is drawn; a search tile waits for the
                // query to rest.
                if context.query.is_none() {
                    fetch_for(&ask, false);
                }
                with_searched(|s| tile::tile(&id, &s.view(&ask), &context, now, true))
            }
            None => with(|w| tile::tile(&id, &w.view(), &context, now, false)),
        }
    }
}

export_plugin!(WeatherPlugin);
