//! What the plugin knows and when it asks again: a pure state machine, so it is tested
//! natively. `lib.rs` does the imports it asks for.
//!
//! - One request at a time. [`Weather::due`] says what to fetch now, if anything; the
//!   answer goes to [`Weather::answered`], which ignores an answer to a request it no longer
//!   waits for (the city changed meanwhile).
//! - A city name is geocoded first, in each of its [`languages`](api::languages) until one
//!   finds it; `latitude, longitude` needs no lookup.
//! - The forecast is fetched when there is none, or when it is [`REFRESH_SECS`] old.
//! - A failure (no network, a timeout, a bad answer) keeps the last reading and waits
//!   before the next try: 1, 2, 4, 8 and then 15 minutes. A city geocoding does not know is
//!   asked again only when the city changes or the user refreshes.
//! - The city `auto` (or none) is the computer's location: asked of the launcher at once,
//!   then every [`LOCATE_SECS`], and [`RELOCATE_FAILED_SECS`] after it could not be told.
//!   Until one comes nothing is fetched; when none can be had the tiles say so.

use crate::api::{self, Geocoded, Place, Reading, Units};

/// A reading this old is fetched again.
pub const REFRESH_SECS: i64 = 15 * 60;
/// How much sooner than [`REFRESH_SECS`] a reading is due, so a timer a little early still
/// fetches.
const SLACK_SECS: i64 = 30;
/// How often the location is asked again once known.
pub const LOCATE_SECS: i64 = 3 * 3600;
/// How soon it is asked again when it could not be told, or only coarsely.
pub const RELOCATE_FAILED_SECS: i64 = 30 * 60;
/// A location less accurate than this is coarse: marked on the tile, and asked again sooner.
pub const COARSE_KM: u32 = 50;
/// The wait after the first failure in a row; it doubles up to [`MAX_BACKOFF_SECS`].
const FIRST_BACKOFF_SECS: i64 = 60;
const MAX_BACKOFF_SECS: i64 = REFRESH_SECS;

/// The user's preferences.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    /// Trimmed; empty when unset.
    pub city: String,
    pub units: Units,
}

impl Settings {
    /// The city is the computer's location: `auto`, or none.
    pub fn auto(&self) -> bool {
        self.city.is_empty() || self.city.eq_ignore_ascii_case("auto")
    }
}

/// What a fetch is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// The geocoding of the city in its `n`th language.
    Geocode(usize),
    Forecast,
}

/// A request to start.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetch {
    pub want: Want,
    pub url: String,
}

/// What the tile says besides the reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Fine, or waiting for the first answer.
    Ok,
    /// The city is `auto`, and the computer's location cannot be told.
    NoLocation,
    /// Geocoding knows no such city.
    Unknown,
    /// The last try failed; it is tried again later.
    Failed,
}

#[derive(Debug, Default)]
pub struct Weather {
    pub settings: Settings,
    pub place: Option<Place>,
    pub reading: Option<Reading>,
    failed: bool,
    unknown: bool,
    /// The request waiting for its answer, and what it is for.
    pending: Option<(u64, Want)>,
    /// The geocoding language to try next.
    language: usize,
    /// Failures in a row.
    failures: u32,
    /// No automatic fetch before this time (unix seconds).
    retry_at: Option<i64>,
    /// Why the computer's location cannot be told, when it cannot.
    no_location: Option<String>,
    /// When the location was last asked for and last told (unix seconds).
    location_asked: Option<i64>,
    /// The location known is coarser than [`COARSE_KM`].
    coarse: bool,
    /// The location was asked for and has not been told yet: an answer now is the answer, not
    /// news of a change of the launcher's settings.
    locating: bool,
}

impl Weather {
    /// Takes new preferences; returns whether they changed. A new city forgets the place and
    /// the reading; new units forget the reading. Either way the next fetch is due at once,
    /// and the answer of a request started before is ignored.
    pub fn set_settings(&mut self, settings: Settings) -> bool {
        if settings == self.settings {
            return false;
        }
        if settings.city != self.settings.city {
            self.place = if settings.auto() {
                None
            } else {
                api::coordinates(&settings.city)
            };
            self.language = 0;
            self.unknown = false;
            self.no_location = None;
            self.location_asked = None;
        }
        self.reading = None;
        self.pending = None;
        self.failed = false;
        self.failures = 0;
        self.retry_at = None;
        self.settings = settings;
        true
    }

    /// What the tile should say about the state of things.
    pub fn status(&self) -> Status {
        if self.settings.auto() && self.place.is_none() && self.no_location.is_some() {
            Status::NoLocation
        } else if self.unknown {
            Status::Unknown
        } else if self.failed {
            Status::Failed
        } else {
            Status::Ok
        }
    }

    /// Whether a request waits for its answer.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The fetch to start at `now`, if any. `forced` (the user asked) skips the wait after a
    /// failure, asks an unknown city again and fetches a fresh reading too.
    pub fn due(&self, now: i64, forced: bool) -> Option<Fetch> {
        if self.pending.is_some() || (self.settings.auto() && self.place.is_none()) {
            return None;
        }
        if !forced && (self.unknown || self.retry_at.is_some_and(|at| now < at)) {
            return None;
        }
        let Some(place) = &self.place else {
            let languages = api::languages(&self.settings.city);
            let language = if self.unknown { 0 } else { self.language };
            return Some(Fetch {
                want: Want::Geocode(language),
                url: api::geocoding_url(&self.settings.city, languages[language]),
            });
        };
        let stale = self
            .reading
            .as_ref()
            .is_none_or(|r| now - r.fetched >= REFRESH_SECS - SLACK_SECS || r.fetched > now);
        (forced || stale).then(|| Fetch {
            want: Want::Forecast,
            url: api::forecast_url(place, self.settings.units),
        })
    }

    /// Whether to ask the launcher for the computer's location at `now` (see the module doc);
    /// `forced`: the user asked for fresh weather.
    pub fn wants_location(&self, now: i64, forced: bool) -> bool {
        let again = if self.place.is_some() && !self.coarse {
            LOCATE_SECS
        } else {
            RELOCATE_FAILED_SECS
        };
        self.settings.auto()
            && (forced
                || self
                    .location_asked
                    .is_none_or(|at| now - at >= again || at > now))
    }

    /// The location was asked for at `now`.
    pub fn location_asked(&mut self, now: i64) {
        self.location_asked = Some(now);
        self.locating = true;
    }

    /// The computer's location with its accuracy in km, or why it cannot be told. A new place
    /// forgets the reading of the old one. A failure that answers the plugin's request keeps a
    /// place known before (the service was slow or busy); one that comes unasked, news that
    /// the user changed the launcher's location settings, forgets it. Returns whether what the
    /// tiles show changed. Ignored unless the city is `auto`.
    pub fn located(&mut self, answer: Result<(Place, u32), String>) -> bool {
        if !self.settings.auto() {
            return false;
        }
        let asked = std::mem::take(&mut self.locating);
        let before = (self.status(), self.place.clone());
        match answer {
            Ok((place, accuracy_km)) => {
                self.no_location = None;
                self.coarse = accuracy_km > COARSE_KM;
                let moved = self.place.as_ref().is_none_or(|old| {
                    (old.latitude, old.longitude) != (place.latitude, place.longitude)
                });
                if moved {
                    self.reading = None;
                    self.pending = None;
                    self.failed = false;
                    self.failures = 0;
                    self.retry_at = None;
                }
                self.place = Some(place);
            }
            Err(why) => {
                if !asked {
                    self.place = None;
                    self.reading = None;
                    self.pending = None;
                }
                self.no_location = Some(why);
            }
        }
        before != (self.status(), self.place.clone())
    }

    /// Why the location cannot be told, when it cannot.
    pub fn no_location(&self) -> Option<&str> {
        self.no_location.as_deref()
    }

    /// `fetch` was started as request `id`.
    pub fn started(&mut self, fetch: &Fetch, id: u64) {
        if let Want::Geocode(language) = fetch.want {
            self.language = language;
            self.unknown = false;
        }
        self.pending = Some((id, fetch.want));
    }

    /// Starting a fetch failed at `now`.
    pub fn start_failed(&mut self, now: i64) {
        self.fail(now);
    }

    /// The answer to request `id` at `now`: the status and body, or why there is none.
    /// Returns whether what the tile shows changed. An answer to a request it does not wait
    /// for changes nothing.
    pub fn answered(&mut self, id: u64, answer: Result<(u16, &[u8]), String>, now: i64) -> bool {
        let Some((pending, want)) = self.pending else {
            return false;
        };
        if pending != id {
            return false;
        }
        self.pending = None;
        let before = (self.status(), self.place.clone(), self.reading.clone());
        let Ok((status, body)) = answer else {
            self.fail(now);
            return before != (self.status(), self.place.clone(), self.reading.clone());
        };
        match want {
            Want::Geocode(language) => match api::parse_geocoding(status, body) {
                Some(Geocoded::Found(place)) => {
                    self.place = Some(place);
                    self.succeed();
                }
                Some(Geocoded::Unknown) => {
                    self.succeed();
                    if language + 1 < api::languages(&self.settings.city).len() {
                        self.language = language + 1;
                    } else {
                        self.language = 0;
                        self.unknown = true;
                    }
                }
                None => self.fail(now),
            },
            Want::Forecast => match api::parse_forecast(status, body, self.settings.units, now) {
                Some(reading) => {
                    self.reading = Some(reading);
                    self.succeed();
                }
                None => self.fail(now),
            },
        }
        before != (self.status(), self.place.clone(), self.reading.clone())
    }

    /// How many failures in a row there were.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    fn succeed(&mut self) {
        self.failed = false;
        self.failures = 0;
        self.retry_at = None;
    }

    fn fail(&mut self, now: i64) {
        self.failed = true;
        self.failures = self.failures.saturating_add(1);
        let backoff = FIRST_BACKOFF_SECS
            .saturating_mul(1 << (self.failures - 1).min(10))
            .min(MAX_BACKOFF_SECS);
        self.retry_at = Some(now + backoff);
    }

    /// The place and reading to keep in storage, for these settings: `None` without a place.
    pub fn encode(&self) -> Option<String> {
        let place = self.place.as_ref()?;
        let mut text = format!(
            "city={}\nunits={}\nname={}\nlat={}\nlon={}\n",
            one_line(&self.settings.city),
            units_word(self.settings.units),
            one_line(&place.name),
            place.latitude,
            place.longitude
        );
        if let Some(r) = &self.reading {
            let opt = |v: Option<f64>| v.map_or(String::new(), |v| v.to_string());
            text.push_str(&format!(
                "temp={}\nfeels={}\nhumidity={}\nwind={}\ncode={}\nday={}\nhigh={}\nlow={}\nfetched={}\n",
                r.temperature,
                opt(r.feels_like),
                opt(r.humidity),
                opt(r.wind),
                r.code,
                r.is_day,
                opt(r.high),
                opt(r.low),
                r.fetched
            ));
        }
        Some(text)
    }

    /// Takes back what [`encode`](Self::encode) kept, if it was kept for the current
    /// settings; anything else (another city or units, garbage) is ignored.
    pub fn restore(&mut self, text: &str) {
        let get = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        };
        if get("city") != Some(self.settings.city.as_str())
            || get("units") != Some(units_word(self.settings.units))
        {
            return;
        }
        let num = |key| {
            get(key)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
        };
        let (Some(name), Some(latitude), Some(longitude)) = (get("name"), num("lat"), num("lon"))
        else {
            return;
        };
        self.place = Some(Place {
            name: name.to_owned(),
            latitude,
            longitude,
        });
        let reading = (|| {
            Some(Reading {
                temperature: num("temp")?,
                feels_like: num("feels"),
                humidity: num("humidity"),
                wind: num("wind"),
                code: get("code")?.parse().ok().filter(|c| *c <= 99)?,
                is_day: get("day")? == "true",
                high: num("high"),
                low: num("low"),
                units: self.settings.units,
                fetched: get("fetched")?.parse().ok()?,
            })
        })();
        self.reading = reading;
    }
}

fn units_word(units: Units) -> &'static str {
    match units {
        Units::Metric => "metric",
        Units::Imperial => "imperial",
    }
}

/// `text` without line breaks, for one line of the stored text.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin.json");
    const BERLIN: &[u8] = include_bytes!("../tests/fixtures/geocoding-berlin.json");
    const MOSCOW: &[u8] = include_bytes!("../tests/fixtures/geocoding-moscow-ru.json");
    const MISS: &[u8] = include_bytes!("../tests/fixtures/geocoding-miss.json");

    fn weather(city: &str) -> Weather {
        let mut w = Weather::default();
        w.set_settings(Settings {
            city: city.into(),
            units: Units::Metric,
        });
        w
    }

    /// Starts what is due at `now` as request `id`; panics when nothing is.
    fn start(w: &mut Weather, now: i64, id: u64) -> Fetch {
        let fetch = w.due(now, false).expect("a fetch is due");
        w.started(&fetch, id);
        fetch
    }

    #[test]
    fn a_city_is_geocoded_then_its_forecast_fetched() {
        let mut w = weather("Berlin");
        assert_eq!(w.status(), Status::Ok);
        let geo = start(&mut w, 0, 1);
        assert_eq!(geo.want, Want::Geocode(0));
        assert!(geo.url.contains("name=Berlin&"), "{}", geo.url);
        assert_eq!(w.due(0, false), None, "one request at a time");

        assert!(w.answered(1, Ok((200, BERLIN)), 1));
        assert_eq!(w.place.as_ref().unwrap().name, "Berlin");
        let forecast = start(&mut w, 1, 2);
        assert_eq!(forecast.want, Want::Forecast);
        assert!(w.answered(2, Ok((200, FORECAST)), 2));
        assert_eq!(w.reading.as_ref().unwrap().temperature, 2.6);
        assert_eq!(w.status(), Status::Ok);

        // Fresh: nothing is due until it is 15 minutes old (less the slack).
        assert_eq!(w.due(2 + 60, false), None);
        assert_eq!(w.due(2 + REFRESH_SECS - 31, false), None);
        assert_eq!(
            w.due(2 + REFRESH_SECS - 30, false).unwrap().want,
            Want::Forecast
        );
        // The user's refresh fetches at once.
        assert_eq!(w.due(3, true).unwrap().want, Want::Forecast);
    }

    #[test]
    fn coordinates_need_no_geocoding() {
        let w = weather("55.75, 37.62");
        assert_eq!(w.due(0, false).unwrap().want, Want::Forecast);
    }

    #[test]
    fn a_cyrillic_name_is_tried_in_each_language() {
        let mut w = weather("Москва");
        let first = start(&mut w, 0, 1);
        assert!(first.url.contains("&language=ru&"), "{}", first.url);
        // Not in Russian (as if): Ukrainian next, at once.
        assert!(!w.answered(1, Ok((200, MISS)), 0));
        let second = start(&mut w, 0, 2);
        assert_eq!(second.want, Want::Geocode(1));
        assert!(second.url.contains("&language=uk&"), "{}", second.url);
        assert!(w.answered(2, Ok((200, MOSCOW)), 0));
        assert_eq!(w.place.as_ref().unwrap().name, "Москва");
    }

    #[test]
    fn an_unknown_city_is_asked_again_only_when_it_changes_or_on_refresh() {
        let mut w = weather("Xyzzyqqplonk");
        start(&mut w, 0, 1);
        assert!(w.answered(1, Ok((200, MISS)), 0));
        assert_eq!(w.status(), Status::Unknown);
        assert_eq!(w.due(10_000, false), None);
        assert_eq!(w.due(10_000, true).unwrap().want, Want::Geocode(0));

        assert!(w.set_settings(Settings {
            city: "Berlin".into(),
            units: Units::Metric,
        }));
        assert_eq!(w.status(), Status::Ok);
        assert_eq!(w.due(0, false).unwrap().want, Want::Geocode(0));
    }

    #[test]
    fn failures_keep_the_reading_and_back_off() {
        let mut w = weather("52.52, 13.41");
        start(&mut w, 0, 1);
        w.answered(1, Ok((200, FORECAST)), 0);
        let now = REFRESH_SECS;
        start(&mut w, now, 2);
        assert!(
            w.answered(2, Err("offline".into()), now),
            "the status changed"
        );
        assert_eq!(w.status(), Status::Failed);
        assert!(w.reading.is_some(), "the last reading stays");
        // 60 s, then 120, 240, 480, 900 and 900 again.
        let mut at = now;
        for (id, wait) in (3..).zip([60, 120, 240, 480, 900, 900]) {
            assert_eq!(w.due(at + wait - 1, false), None, "waits {wait} s");
            at += wait;
            start(&mut w, at, id);
            // A failure on a failure changes nothing on screen.
            assert!(!w.answered(id, Ok((503, b"")), at));
        }
        assert_eq!(w.failures(), 7);
        // A good answer ends the run.
        start(&mut w, at + 900, 99);
        assert!(w.answered(99, Ok((200, FORECAST)), at + 900));
        assert_eq!((w.status(), w.failures()), (Status::Ok, 0));
    }

    #[test]
    fn a_failed_start_backs_off_too() {
        let mut w = weather("52.52, 13.41");
        w.due(0, false).unwrap();
        w.start_failed(0);
        assert_eq!(w.status(), Status::Failed);
        assert_eq!(w.due(59, false), None);
        assert!(w.due(60, false).is_some());
    }

    #[test]
    fn an_answer_for_the_old_city_is_ignored() {
        let mut w = weather("Berlin");
        start(&mut w, 0, 1);
        w.set_settings(Settings {
            city: "52.52, 13.41".into(),
            units: Units::Metric,
        });
        // Due at once, though the old request is still out.
        start(&mut w, 0, 2);
        assert!(!w.answered(1, Ok((200, BERLIN)), 0));
        assert_eq!(w.place.as_ref().unwrap().name, "52.52, 13.41");
        assert!(w.answered(2, Ok((200, FORECAST)), 0));
    }

    #[test]
    fn new_units_fetch_again_without_a_lookup() {
        let mut w = weather("Berlin");
        start(&mut w, 0, 1);
        w.answered(1, Ok((200, BERLIN)), 0);
        start(&mut w, 0, 2);
        w.answered(2, Ok((200, FORECAST)), 0);
        assert!(w.set_settings(Settings {
            city: "Berlin".into(),
            units: Units::Imperial,
        }));
        assert_eq!(w.reading, None);
        let fetch = w.due(1, false).unwrap();
        assert!(fetch.url.contains("fahrenheit"), "{}", fetch.url);
        assert!(!w.set_settings(w.settings.clone()));
    }

    fn here(latitude: f64) -> Place {
        Place {
            name: "Here".into(),
            latitude,
            longitude: 13.4,
        }
    }

    #[test]
    fn auto_waits_for_the_location_then_fetches_there() {
        for city in ["", "auto", "Auto"] {
            let mut w = weather(city);
            assert!(w.settings.auto(), "{city:?}");
            assert_eq!(w.status(), Status::Ok);
            assert_eq!(w.due(0, true), None, "nothing before a location");
            assert!(w.wants_location(0, false));
            w.location_asked(0);
            assert!(!w.wants_location(10, false), "asked once");
            assert!(w.located(Ok((here(52.5), 11))));
            let fetch = w.due(0, false).unwrap();
            assert_eq!(fetch.want, Want::Forecast);
            assert!(fetch.url.contains("latitude=52.5&"), "{}", fetch.url);
        }
    }

    #[test]
    fn auto_asks_again_in_three_hours_or_half_an_hour_after_a_failure() {
        let mut w = weather("auto");
        w.location_asked(0);
        assert!(w.located(Err("location services are off".into())));
        assert_eq!(w.status(), Status::NoLocation);
        assert_eq!(w.no_location(), Some("location services are off"));
        assert!(!w.wants_location(RELOCATE_FAILED_SECS - 1, false));
        assert!(w.wants_location(RELOCATE_FAILED_SECS, false));
        assert!(w.wants_location(1, true), "the user's refresh asks at once");
        w.location_asked(100);
        w.located(Ok((here(52.5), 11)));
        assert_eq!(w.status(), Status::Ok);
        assert!(!w.wants_location(100 + LOCATE_SECS - 1, false));
        assert!(w.wants_location(100 + LOCATE_SECS, false));
        // A failure of a later request keeps the known place.
        w.location_asked(200);
        assert!(!w.located(Err("timed out".into())));
        assert_eq!(w.place.as_ref().unwrap().latitude, 52.5);
        assert_eq!(w.status(), Status::Ok);
    }

    #[test]
    fn a_new_place_forgets_the_old_reading() {
        let mut w = weather("auto");
        w.located(Ok((here(52.5), 11)));
        let fetch = w.due(0, false).unwrap();
        w.started(&fetch, 1);
        w.answered(1, Ok((200, FORECAST)), 0);
        assert!(w.reading.is_some());
        assert!(
            !w.located(Ok((here(52.5), 11))),
            "the same place changes nothing"
        );
        assert!(w.reading.is_some());
        assert!(w.located(Ok((here(48.1), 11))));
        assert_eq!(w.reading, None);
    }

    #[test]
    fn a_named_city_ignores_locations() {
        let mut w = weather("Berlin");
        assert!(!w.wants_location(0, true));
        assert!(!w.located(Ok((here(1.), 11))));
        assert_eq!(w.place, None);
    }

    #[test]
    fn a_reading_from_the_future_is_stale() {
        let mut w = weather("52.52, 13.41");
        start(&mut w, 1_000_000, 1);
        w.answered(1, Ok((200, FORECAST)), 1_000_000);
        assert!(w.due(1_000_000 - 3600, false).is_some());
    }

    #[test]
    fn storage_round_trips_for_the_same_settings_only() {
        let mut w = weather("Berlin");
        assert_eq!(w.encode(), None);
        start(&mut w, 0, 1);
        w.answered(1, Ok((200, BERLIN)), 0);
        start(&mut w, 0, 2);
        w.answered(2, Ok((200, FORECAST)), 42);
        let text = w.encode().unwrap();

        let mut again = weather("Berlin");
        again.restore(&text);
        assert_eq!(again.place, w.place);
        assert_eq!(again.reading, w.reading);
        assert_eq!(again.due(42 + 60, false), None, "fresh after a reload");

        let mut other = weather("Paris");
        other.restore(&text);
        assert_eq!((other.place, other.reading), (None, None));
        let mut fahrenheit = Weather::default();
        fahrenheit.set_settings(Settings {
            city: "Berlin".into(),
            units: Units::Imperial,
        });
        fahrenheit.restore(&text);
        assert_eq!(fahrenheit.place, None);
        let mut garbage = weather("Berlin");
        garbage.restore("city=Berlin\nunits=metric\nname=B\nlat=x\n");
        assert_eq!(garbage.place, None);
        // A place without a reading is kept: no lookup after a reload.
        let mut place_only = weather("Berlin");
        place_only.restore("city=Berlin\nunits=metric\nname=Berlin\nlat=1\nlon=2\n");
        assert_eq!(place_only.reading, None);
        assert_eq!(place_only.due(0, false).unwrap().want, Want::Forecast);
    }

    #[test]
    fn a_coarse_location_is_asked_again_sooner() {
        let mut w = weather("auto");
        w.location_asked(100);
        w.located(Ok((here(55.8), 500)));
        assert_eq!(w.status(), Status::Ok);
        assert!(!w.wants_location(100 + RELOCATE_FAILED_SECS - 1, false));
        assert!(w.wants_location(100 + RELOCATE_FAILED_SECS, false));
        w.location_asked(200);
        w.located(Ok((here(55.8), COARSE_KM)));
        assert!(
            !w.wants_location(200 + RELOCATE_FAILED_SECS, false),
            "fine enough"
        );
    }

    #[test]
    fn a_failure_that_comes_unasked_forgets_the_place() {
        let mut w = weather("auto");
        w.location_asked(100);
        w.located(Ok((here(52.5), 11)));
        // Asked, and failed: the place known stays (a slow or busy service).
        w.location_asked(200);
        assert!(!w.located(Err("timed out".into())));
        assert_eq!(w.status(), Status::Ok);
        // Unasked: the user changed the launcher's location settings to a wrong city.
        assert!(w.located(Err("not a city of the built-in list".into())));
        assert_eq!(w.place, None);
        assert_eq!(w.status(), Status::NoLocation);
        // A place that comes unasked is taken as usual.
        assert!(w.located(Ok((here(48.1), 11))));
        assert_eq!(w.status(), Status::Ok);
    }
}
