//! Weather plugin: the weather from Open-Meteo (free, no key) in ten widgets for the
//! launcher's strip: `now` (a quarter or a single), `today` (the four parts of the day, as a
//! quarter, a single or a double, see [`today`]) and the eight measures (humidity, wind, UV,
//! pressure, rain, air quality, pollen, daylight, see [`measures`]). The user puts them on
//! Home; the plugin puts nothing there.
//!
//! - A tile with the param `city` shows that city; without it, the computer's location,
//!   asked of the launcher (`location.request`) and named after a city of the built-in list
//!   near it (`Cities::nearest`). Every place lives in one store, [`places`] (with [`feed`]
//!   and [`location`]): one forecast per place, and air only while an air quality or pollen
//!   tile shows it. the preferences `temperature` (°C or °F), `wind` (km/h, m/s, mph or knots) and
//!   `pressure` (hPa or mm Hg) pick the units; a change of `pressure` only redraws.
//! - `init` reads the preferences, takes back the location's place and readings from storage
//!   (`forecast:@here`, `air:@here`), removes the old key `last` and starts a [`TICK_MS`]
//!   timer. It asks for nothing: the `home` event says what is wanted.
//! - The `home` event (the plugin's Home tiles, sent while the launcher is hidden too) names
//!   the places to keep fresh. A place no tile names any more is forgotten, its storage keys
//!   removed (when Home drops it, or when the search that still named it moves on); a
//!   place Home names anew is taken back from storage, so it draws at once. The location is
//!   asked only while a tile without a city is on Home or is the search's. Then what is due
//!   is fetched and every tile redrawn.
//! - Every tick, a tile coming on screen, an answer and a change of units fetch what is due
//!   ([`Places::due`]): a reading 15 minutes old, after a failure not before its wait, at most
//!   four requests at once. A request the host refuses because four are out waits for the
//!   next turn, with no failure. Drawing a tile never fetches.
//! - Enter on a tile fetches its place at once (the location is asked again too; a city
//!   geocoding did not know is asked again). The "Refresh Weather" command does the same for
//!   every wanted place: Home's, the location and the search's.
//! - Every answer that changes what the tiles show redraws them; each good answer of a Home
//!   place (the location's, for a Home tile without a city) is written to storage under its
//!   place's key; places only searched are not. Failures never toast: the tiles keep the
//!   last reading and say how old it is, or say calmly why there is none. They are logged
//!   once per run of failures.
//! - Search: a word for each widget, with or without a city (see [`search`]), answers that
//!   widget, with the param `city` for a city. Its place is fetched once the query rests
//!   [`SEARCH_REST_MS`], and kept fresh until the query no longer matches or its tile goes
//!   off screen (the launcher hid), and again when that tile comes back; Enter copies the
//!   weather in one line.

pub mod api;
pub mod chart;
pub mod day;
pub mod feed;
pub mod ink;
pub mod json;
pub mod location;
pub mod measures;
pub mod places;
pub mod search;
pub mod tile;
pub mod today;

use std::cell::{OnceCell, RefCell};
use std::time::{SystemTime, UNIX_EPOCH};

use tomakast_plugin::{
    Coordinates, Effect, Event, HomeTile, HttpDone, HttpRequest, Item, Plugin, QueryReading,
    SearchAnswer, TimerId, WidgetAnswer, WidgetContext, WidgetEvent, WidgetFrame, WidgetTarget,
    export_plugin, http, log, preferences, storage, timers,
};

use api::{Place, TempUnit, Units, WindUnit};
use location::{COARSE_KM, Location};
use measures::{MEASURES, needs_air};
use places::{Kind, Places, Want, Where, where_of};
use tile::{CITY, NOW, TODAY};
use tomakast_places::cities::Cities;

/// How often the plugin looks whether a fetch is due: a minute, so a wait after a failure
/// ends on time. A tick that finds nothing due costs microseconds.
pub const TICK_MS: u64 = 60_000;
/// The storage key of the plugin before places had keys of their own; removed at `init`.
const OLD_KEY: &str = "last";
/// How long a query must stay the same before its tile fetches: typing `San Francisco`
/// passes `San`, a city too.
pub const SEARCH_REST_MS: u64 = 300;
/// A city of the built-in list is named for the location when one is this near.
const NAME_WITHIN_KM: f64 = 50.;

struct WeatherPlugin;

/// What the plugin knows.
#[derive(Default)]
struct State {
    places: Places,
    location: Location,
    units: Units,
    /// Pressure in mm Hg.
    mmhg: bool,
    /// The storage keys of the places Home named last.
    home: Vec<String>,
    /// A run of failures is going on: the next is logged at debug level only.
    failing: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    static TICK: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    /// The timer that marks the latest search's query as rested.
    static REST: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    static CITIES: OnceCell<Cities> = const { OnceCell::new() };
}

fn with_cities<R>(f: impl FnOnce(&Cities) -> R) -> R {
    CITIES.with(|c| f(c.get_or_init(Cities::builtin)))
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(f)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn units_preference() -> Units {
    let read = |key| preferences::get(key).unwrap_or_default();
    Units {
        temperature: TempUnit::from_preference(&read("temperature")),
        wind: WindUnit::from_preference(&read("wind")),
    }
}

fn mmhg_preference() -> bool {
    preferences::get("pressure").is_some_and(|v| v == "mmhg")
}

/// Every widget of the plugin.
fn widgets() -> impl Iterator<Item = &'static str> {
    [NOW, TODAY].into_iter().chain(MEASURES)
}

fn redraw() -> Vec<Effect> {
    widgets().map(Effect::redraw_widget).collect()
}

/// Takes back what storage kept for `at`: its forecast and its air.
fn restore(s: &mut State, at: &Where) {
    for (kind, prefix) in [(Kind::Forecast, "forecast:"), (Kind::Air, "air:")] {
        match storage::get(&format!("{prefix}{}", at.key())) {
            Ok(Some(text)) => s.places.restore(at, kind, &text),
            Ok(None) => {}
            Err(e) => log::debug(&format!("reading the kept weather failed: {e}")),
        }
    }
    // The location's place, when storage told it first.
    if *at == Where::Here && s.location.place.is_none() {
        s.location.place = s.places.here().cloned();
    }
}

/// Removes what storage kept for the place of `key`.
fn forget(key: &str) {
    for prefix in ["forecast:", "air:"] {
        if let Err(e) = storage::remove(&format!("{prefix}{key}")) {
            log::debug(&format!("forgetting the kept weather failed: {e}"));
        }
    }
}

/// Whether `why`, a refused `http::fetch`, is the host's limit of requests at once: the
/// request waits for its turn.
fn is_busy(why: &str) -> bool {
    why.starts_with("at most ") && why.ends_with(" requests may run at a time")
}

/// Starts the requests that are due (`forced`: the user asked for that place).
fn refresh(forced: Option<&Where>) {
    let now = unix_now();
    with(|s| {
        for job in s.places.due(now, forced) {
            let request = HttpRequest::get(job.url(s.units)).header("accept", "application/json");
            match http::fetch(&request) {
                Ok(id) => s.places.started(job, id),
                Err(e) => {
                    let busy = is_busy(&e);
                    if busy {
                        log::debug(&format!("weather fetch waits its turn: {e}"));
                    } else {
                        note_failure(s, &e);
                    }
                    s.places.cannot_start(&job, now, busy);
                }
            }
        }
    });
}

/// Logs a failure: the first of a run as a warning, the next ones at debug level only, so a
/// long outage does not flood the log.
fn note_failure(s: &mut State, why: &str) {
    if std::mem::replace(&mut s.failing, true) {
        log::debug(&format!("weather fetch failed again: {why}"));
    } else {
        log::warn(&format!("weather fetch failed: {why}; trying again later"));
    }
}

/// An HTTP answer: taken in, kept in storage when good, the next requests started, and the
/// tiles redrawn when what they show changed.
fn answered(done: HttpDone) -> Vec<Effect> {
    let now = unix_now();
    let (answer, why) = match &done.response {
        Ok(r) => (
            Ok((r.status, r.body.as_slice())),
            format!("status {}, {} bytes it cannot read", r.status, r.body.len()),
        ),
        Err(e) => (Err(e.clone()), e.clone()),
    };
    let changed = with(|s| {
        let answered = s.places.answered(done.request, answer, now)?;
        if answered.good {
            s.failing = false;
        } else {
            note_failure(s, &why);
        }
        for (key, text) in &answered.keep {
            if let Err(e) = storage::set(key, text) {
                log::debug(&format!("keeping the weather failed: {e}"));
            }
        }
        Some(answered.changed)
    });
    refresh(None);
    if changed == Some(true) {
        redraw()
    } else {
        Vec::new()
    }
}

/// Asks the launcher for the computer's location when a tile wants it and it is due
/// (`forced`: the user asked). A request that cannot be made is taken as the answer.
fn locate(forced: bool) {
    let now = unix_now();
    let failed = with(|s| {
        if !s.places.wants_here() || !s.location.wants_location(now, forced) {
            return None;
        }
        s.location.location_asked(now);
        tomakast_plugin::location::request().err()
    });
    if let Some(why) = failed {
        log::warn(&format!("asking for the location failed: {why}"));
        with(|s| take_location(s, Err(why)));
    }
}

/// Takes the location's answer into both stores; returns whether what the tiles show
/// changed.
fn take_location(s: &mut State, answer: Result<(Place, u32), String>) -> bool {
    let changed = s.location.located(answer);
    s.places.set_here(s.location.place.clone());
    changed
}

/// The launcher's answer to `location.request` (or its news of a change): the place, named
/// (`≈` when coarse), or why there is none. The location's kept place follows with its next
/// good answer.
fn located(answer: Result<Coordinates, String>) -> Vec<Effect> {
    let answer = answer.map(|at| {
        let name = with_cities(|c| c.nearest(at.latitude, at.longitude, NAME_WITHIN_KM))
            .unwrap_or_else(|| format!("{}, {}", at.latitude, at.longitude));
        let place = Place {
            name: if at.accuracy_km > COARSE_KM {
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
    let changed = with(|s| take_location(s, answer));
    refresh(None);
    if changed { redraw() } else { Vec::new() }
}

/// The plugin's Home tiles: their places are wanted from now on, those dropped forgotten,
/// those new taken back from storage.
fn home(tiles: Vec<HomeTile>) -> Vec<Effect> {
    let wants: Vec<Want> = with_cities(|cities| {
        tiles
            .iter()
            .map(|t| Want {
                widget: t.widget_id.clone(),
                at: where_of(&t.params, cities),
                air: needs_air(&t.widget_id),
            })
            .collect()
    });
    let forgotten = with(|s| {
        let mut named: Vec<Where> = Vec::new();
        for want in &wants {
            let key = want.at.key();
            if !s.home.contains(&key) && !named.iter().any(|at| at.key() == key) {
                named.push(want.at.clone());
            }
        }
        s.home = wants.iter().map(|want| want.at.key()).collect();
        let forgotten = s.places.set_home(wants);
        for at in &named {
            restore(s, at);
        }
        forgotten
    });
    for key in &forgotten {
        forget(key);
    }
    locate(false);
    refresh(None);
    redraw()
}

/// Remembers `want` as the search tile's and starts its rest timer in place of the last one;
/// `None` forgets it. A storage key no tile names any more goes.
fn searched(want: Option<Want>) {
    stop_rest();
    let some = want.is_some();
    for key in with(|s| s.places.set_search(want)) {
        forget(&key);
    }
    if some {
        start_rest();
    }
}

/// Starts the timer that marks the search's query as rested.
fn start_rest() {
    let timer = timers::start(SEARCH_REST_MS, false)
        .map_err(|e| log::debug(&format!("starting the search timer failed: {e}")))
        .ok();
    REST.set(timer);
}

/// Cancels the search's rest timer, if it runs.
fn stop_rest() {
    if let Some(timer) = REST.take() {
        timers::cancel(timer);
    }
}

/// The search's query rested: its place is fetched.
fn rested() {
    with(|s| s.places.search_rested());
    locate(false);
    refresh(None);
}

/// An event of a tile of `widget` showing `at`: `copy` copies its weather in one line,
/// `refresh` fetches it at once, and coming on screen fetches what went stale meanwhile
/// (timers may sleep while the launcher is hidden). The search's tile (its widget and place)
/// going off screen ends the search: its place is no longer kept fresh (a Home tile of the
/// same place keeps it); coming back on screen starts the search again, rested as it was.
/// An older tile of the same place, replaced by a newer answer, does neither.
fn tile_event(widget: &str, at: &Where, event: WidgetEvent) -> Vec<Effect> {
    match event {
        WidgetEvent::Shown => {
            match with(|s| s.places.search_shown(widget, at)) {
                Some(true) => locate(false),
                Some(false) => start_rest(),
                None => {}
            }
            refresh(None);
            Vec::new()
        }
        WidgetEvent::Hidden => {
            if let Some(keys) = with(|s| s.places.search_hidden(widget, at)) {
                stop_rest();
                for key in keys {
                    forget(&key);
                }
            }
            Vec::new()
        }
        WidgetEvent::Action(a) if a.action_id == "copy" => {
            with(|s| tile::summary(&s.places.view(at, &s.location)))
                .map(|text| vec![Effect::copy_text(text)])
                .unwrap_or_default()
        }
        WidgetEvent::Action(a) if a.action_id == "refresh" => {
            if *at == Where::Here {
                locate(true);
            }
            refresh(Some(at));
            redraw()
        }
        _ => Vec::new(),
    }
}

impl Plugin for WeatherPlugin {
    fn init() -> Result<Vec<Effect>, String> {
        // The built-in list is read here, where there are 2 s, not in the first search.
        with_cities(|_| {});
        with(|s| {
            s.units = units_preference();
            s.mmhg = mmhg_preference();
            s.places.set_units(s.units);
            restore(s, &Where::Here);
        });
        if let Err(e) = storage::remove(OLD_KEY) {
            log::debug(&format!("removing the old weather failed: {e}"));
        }
        match timers::start(TICK_MS, true) {
            Ok(tick) => TICK.set(Some(tick)),
            Err(e) => log::warn(&format!("starting the weather timer failed: {e}")),
        }
        Ok(Vec::new())
    }

    fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
        // The query as typed, then as read on another keyboard layout.
        let read = with_cities(|cities| {
            std::iter::once(query.as_str())
                .chain(readings.iter().map(|r| r.text.as_str()))
                .find_map(|text| search::read(text, cities))
        });
        let Some(read) = read else {
            searched(None);
            return Ok(Vec::new().into());
        };
        let mut tile = WidgetAnswer::new(read.widget)?;
        let at = match read.city {
            Some(ask) => {
                tile = tile.param(CITY, ask.param());
                Where::City {
                    typed: ask.param(),
                    ask,
                }
            }
            None => Where::Here,
        };
        let tile = tile.action("copy", "Copy Weather")?;
        searched(Some(Want {
            widget: read.widget.into(),
            at,
            air: needs_air(read.widget),
        }));
        Ok(SearchAnswer {
            items: Vec::new(),
            widgets: vec![tile],
        })
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        if item.id == "weather/refresh" && action_id == "run" {
            locate(true);
            with(|s| s.places.force_all());
            refresh(None);
            return Ok(redraw());
        }
        Ok(Vec::new())
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        Ok(match event {
            Event::TimerFired(timer) if TICK.with_borrow(|t| *t == Some(timer)) => {
                locate(false);
                refresh(None);
                Vec::new()
            }
            Event::TimerFired(timer) if REST.with_borrow(|t| *t == Some(timer)) => {
                REST.set(None);
                rested();
                Vec::new()
            }
            Event::HttpDone(done) => answered(done),
            Event::Location(answer) => located(answer),
            Event::Home(tiles) => home(tiles),
            Event::PreferencesChanged => {
                let units = units_preference();
                let mmhg = mmhg_preference();
                let (units_changed, mmhg_changed) = with(|s| {
                    let changed = (s.units != units, s.mmhg != mmhg);
                    s.units = units;
                    s.mmhg = mmhg;
                    s.places.set_units(units);
                    changed
                });
                if units_changed {
                    refresh(None);
                }
                if units_changed || mmhg_changed {
                    redraw()
                } else {
                    Vec::new()
                }
            }
            Event::Widget(WidgetTarget {
                widget_id,
                params,
                event,
            }) => {
                let at = with_cities(|cities| where_of(&params, cities));
                tile_event(&widget_id, &at, event)
            }
            _ => Vec::new(),
        })
    }

    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        let now = unix_now();
        let at = with_cities(|cities| where_of(&context.params, cities));
        let copy = matches!(at, Where::City { .. });
        with(|s| {
            let view = s.places.view(&at, &s.location);
            tile::tile(&id, &view, &context, now, copy, s.mmhg)
        })
    }
}

export_plugin!(WeatherPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    /// The host's refusal at its limit of requests at once
    /// (`crates/plugin-host/src/imports/http.rs`), and others that are not it.
    #[test]
    fn the_hosts_limit_is_told_from_other_refusals() {
        assert!(is_busy("at most 4 requests may run at a time"));
        for other in [
            "the plugin has no http permission",
            "not a valid URL: relative URL without a base",
            "at most 4 requests",
        ] {
            assert!(!is_busy(other), "{other}");
        }
    }
}
