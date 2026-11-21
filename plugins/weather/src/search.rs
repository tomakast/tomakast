//! The weather of a city typed in the launcher search (the `now` tile with the param `city`),
//! and of a Home tile with a city: pure, so it is tested natively; `lib.rs` does the imports.
//!
//! - [`ask`] reads a query: `weather in <city>` (also `weather <city>`, `<city> weather`,
//!   `погода в <city>`, `погода <city>`, `<city> погода`) asks for that city, found in the
//!   built-in list, as `latitude, longitude`, or else geocoded by Open-Meteo. A bare query
//!   shows a tile only when it is a city of the built-in list: what else the user types is
//!   never sent anywhere.
//! - [`Searched`] holds what searches asked for: the places looked up and the readings of
//!   places, each fresh for [`REFRESH_SECS`](crate::state::REFRESH_SECS). At most two requests
//!   wait at once; a place that failed is tried again a minute later at the earliest, unless
//!   the user asks (Enter on the tile), which also asks again for a city geocoding did not
//!   know. Old entries are dropped past [`MAX_ENTRIES`].
//! - The tile's param `city` names what it shows ([`Ask::param`]), so a render, an action or
//!   an answer that comes after more typing finds its own place, and a Home tile finds it
//!   again from `config.toml` ([`city`]), where the user may also type a city by hand. A city
//!   is kept as typed (trimmed); [`cities::key`] is only what it is matched and looked up by.

use std::collections::HashMap;

use crate::api::{self, Geocoded, Place, Reading, Units};
use crate::state::{REFRESH_SECS, Status};
use crate::tile::View;
use tomakast_places::cities::{self, Cities};

/// The words that ask for the weather, before the city (compared on [`cities::key`]s).
const PREFIXES: &[&str] = &[
    "weather in",
    "weather at",
    "weather for",
    "weather",
    "погода в",
    "погода во",
    "погода",
];
/// What a query that ends in it has not named the city yet.
const CONNECTORS: &[&str] = &["in", "at", "for", "в", "во"];
/// The same, after the city.
const SUFFIXES: &[&str] = &["weather", "погода"];
/// A place that failed is not asked again sooner.
const RETRY_SECS: i64 = 60;
/// Requests a search may have waiting.
const MAX_PENDING: usize = 2;
/// Places and lookups kept; past it, those not waiting for an answer are forgotten.
pub const MAX_ENTRIES: usize = 32;

/// What a query asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// A known place.
    Place(Place),
    /// A city to geocode, as typed (trimmed); looked up by its [`cities::key`].
    Lookup(String),
}

/// What `query` asks for, if anything (see the module doc).
pub fn ask(query: &str, cities: &Cities) -> Option<Ask> {
    let query = query.trim();
    let named = PREFIXES
        .iter()
        .find_map(|p| after_words(query, p))
        .or_else(|| SUFFIXES.iter().find_map(|s| before_words(query, s)));
    let Some(named) = named else {
        return cities.find(query).map(Ask::Place);
    };
    // Still typing: `weather in` before the city, or one letter of it.
    let key = cities::key(named);
    if key.chars().count() < 2 || CONNECTORS.contains(&key.as_str()) {
        return None;
    }
    // Read as a tile's param is, so the search tile and its param name the same place.
    city(named, cities)
}

/// The rest of `text` (trimmed) after its first words, when their key is `words`.
fn after_words<'a>(text: &'a str, words: &str) -> Option<&'a str> {
    text.char_indices()
        .filter(|(_, c)| c.is_whitespace())
        .find(|(i, _)| cities::key(&text[..*i]) == words)
        .map(|(i, _)| text[i..].trim())
}

/// The start of `text` (trimmed) before its last words, when their key is `words`.
fn before_words<'a>(text: &'a str, words: &str) -> Option<&'a str> {
    text.char_indices()
        .filter(|(_, c)| c.is_whitespace())
        .find(|(i, _)| cities::key(&text[*i..]) == words)
        .map(|(i, _)| text[..i].trim())
}

/// The place a tile's param `city` names: a city of the built-in list, by any of its names
/// or as `Name, CC` (how [`Ask::param`] writes it), `latitude, longitude`, or else a city to
/// geocode, kept as typed. `None` for a blank one or `auto` (in any case): the computer's
/// location.
pub fn city(text: &str, cities: &Cities) -> Option<Ask> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("auto") {
        return None;
    }
    if let Some(place) = cities.find(text).or_else(|| api::coordinates(text)) {
        return Some(Ask::Place(place));
    }
    let listed = text.rsplit_once(',').and_then(|(name, country)| {
        let place = cities.find(name)?;
        let (_, cc) = place.name.rsplit_once(", ")?;
        cc.eq_ignore_ascii_case(country.trim()).then_some(place)
    });
    Some(listed.map_or_else(|| Ask::Lookup(text.to_owned()), Ask::Place))
}

impl Ask {
    /// The tile's param `city`: the place's name (`Berlin, DE`, `52.52, 13.41`), or the city
    /// to look up as typed. [`city`] reads it back.
    pub fn param(&self) -> String {
        match self {
            Ask::Place(p) => p.name.clone(),
            Ask::Lookup(city) => city.clone(),
        }
    }
}

/// What a lookup found.
#[derive(Debug, Clone, PartialEq)]
enum Geo {
    Found(Place),
    /// Not found in the languages tried so far; this one is next.
    Next(usize),
    Unknown,
    Failed(i64),
}

#[derive(Debug, Default)]
struct Spot {
    reading: Option<Reading>,
    failed_at: Option<i64>,
}

/// A request for a search's tile.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Geocode { city: String, language: usize },
    Forecast { place: Place },
}

/// What searches asked for (see the module doc).
#[derive(Debug, Default)]
pub struct Searched {
    units: Units,
    geo: HashMap<String, Geo>,
    spots: HashMap<String, Spot>,
    pending: Vec<(u64, Job)>,
}

/// A place's key in [`Searched::spots`].
fn spot(place: &Place) -> String {
    format!("{},{}", place.latitude, place.longitude)
}

impl Searched {
    /// Readings in other units are forgotten.
    pub fn set_units(&mut self, units: Units) {
        if units != self.units {
            self.units = units;
            self.spots.clear();
            self.pending
                .retain(|(_, job)| matches!(job, Job::Geocode { .. }));
        }
    }

    /// The place `ask` names, once known.
    fn place(&self, ask: &Ask) -> Option<Place> {
        match ask {
            Ask::Place(place) => Some(place.clone()),
            Ask::Lookup(city) => match self.geo.get(&cities::key(city))? {
                Geo::Found(place) => Some(place.clone()),
                _ => None,
            },
        }
    }

    /// The request `ask` needs at `now`, with its URL, if any. `forced` (the user asked)
    /// skips the wait after a failure, asks again for a city geocoding did not know, and
    /// fetches a fresh reading too.
    pub fn due(&self, ask: &Ask, now: i64, forced: bool) -> Option<(Job, String)> {
        if self.pending.len() >= MAX_PENDING {
            return None;
        }
        let waits = |job: &Job| self.pending.iter().any(|(_, j)| j == job);
        if let Ask::Lookup(typed) = ask {
            let city = cities::key(typed);
            let language = match self.geo.get(&city) {
                None => Some(0),
                Some(Geo::Next(language)) => Some(*language),
                Some(Geo::Failed(_) | Geo::Unknown) if forced => Some(0),
                Some(Geo::Failed(at)) if now - at >= RETRY_SECS => Some(0),
                Some(Geo::Failed(_) | Geo::Unknown) => return None,
                Some(Geo::Found(_)) => None,
            };
            if let Some(language) = language {
                let url = api::geocoding_url(&city, api::languages(&city)[language]);
                let job = Job::Geocode { city, language };
                return (!waits(&job)).then_some((job, url));
            }
        }
        let place = self.place(ask)?;
        let spot = self.spots.get(&spot(&place));
        let fresh = spot
            .and_then(|s| s.reading.as_ref())
            .is_some_and(|r| (0..REFRESH_SECS - 30).contains(&(now - r.fetched)));
        let failed = spot
            .and_then(|s| s.failed_at)
            .is_some_and(|at| now - at < RETRY_SECS);
        let url = api::forecast_url(&place, self.units);
        let job = Job::Forecast { place };
        ((forced || (!fresh && !failed)) && !waits(&job)).then_some((job, url))
    }

    /// `job` was started as request `id`.
    pub fn started(&mut self, job: Job, id: u64) {
        self.pending.push((id, job));
    }

    /// Starting `job` failed at `now`.
    pub fn start_failed(&mut self, job: &Job, now: i64) {
        self.fail(job, now);
    }

    /// Whether request `id` is one of the searches'.
    pub fn waits_for(&self, id: u64) -> bool {
        self.pending.iter().any(|(p, _)| *p == id)
    }

    /// The answer to request `id` at `now`. Returns whether it was one of the searches'.
    pub fn answered(&mut self, id: u64, answer: Result<(u16, &[u8]), String>, now: i64) -> bool {
        let Some(at) = self.pending.iter().position(|(p, _)| *p == id) else {
            return false;
        };
        let (_, job) = self.pending.swap_remove(at);
        let Ok((status, body)) = answer else {
            self.fail(&job, now);
            return true;
        };
        match &job {
            Job::Geocode { city, language } => {
                let geo = match api::parse_geocoding(status, body) {
                    Some(Geocoded::Found(place)) => Geo::Found(place),
                    Some(Geocoded::Unknown) if language + 1 < api::languages(city).len() => {
                        Geo::Next(language + 1)
                    }
                    Some(Geocoded::Unknown) => Geo::Unknown,
                    None => Geo::Failed(now),
                };
                self.geo.insert(city.clone(), geo);
            }
            Job::Forecast { place } => match api::parse_forecast(status, body, self.units, now) {
                Some(reading) => {
                    self.spots.insert(
                        spot(place),
                        Spot {
                            reading: Some(reading),
                            failed_at: None,
                        },
                    );
                }
                None => self.fail(&job, now),
            },
        }
        self.trim();
        true
    }

    fn fail(&mut self, job: &Job, now: i64) {
        match job {
            Job::Geocode { city, .. } => {
                self.geo.insert(city.clone(), Geo::Failed(now));
            }
            Job::Forecast { place } => {
                self.spots.entry(spot(place)).or_default().failed_at = Some(now);
            }
        }
        self.trim();
    }

    /// Forgets what no request waits for, once there is more than [`MAX_ENTRIES`] of it.
    fn trim(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if self.geo.len() > MAX_ENTRIES {
            self.geo.retain(|city, _| {
                pending
                    .iter()
                    .any(|(_, j)| matches!(j, Job::Geocode { city: c, .. } if c == city))
            });
        }
        if self.spots.len() > MAX_ENTRIES {
            self.spots.retain(|key, _| {
                pending
                    .iter()
                    .any(|(_, j)| matches!(j, Job::Forecast { place } if spot(place) == *key))
            });
        }
        self.pending = pending;
    }

    /// What the tile of `ask` shows.
    pub fn view(&self, ask: &Ask) -> View<'_> {
        let typed = match ask {
            Ask::Place(place) => place.name.clone(),
            Ask::Lookup(city) => city.clone(),
        };
        let lookup = match ask {
            Ask::Lookup(city) => self.geo.get(&cities::key(city)),
            Ask::Place(_) => None,
        };
        let status = match lookup {
            Some(Geo::Unknown) => Status::Unknown,
            Some(Geo::Failed(_)) => Status::Failed,
            _ => Status::Ok,
        };
        let Some(place) = self.place(ask) else {
            return View {
                name: typed.clone(),
                reading: None,
                status,
                city: typed,
            };
        };
        let spot = self.spots.get(&spot(&place));
        View {
            name: place.name.clone(),
            reading: spot.and_then(|s| s.reading.as_ref()),
            status: if spot.is_some_and(|s| s.failed_at.is_some()) {
                Status::Failed
            } else {
                status
            },
            city: typed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin.json");
    const MOSCOW: &[u8] = include_bytes!("../tests/fixtures/geocoding-moscow-ru.json");
    const MISS: &[u8] = include_bytes!("../tests/fixtures/geocoding-miss.json");

    fn cities() -> Cities {
        Cities::parse(
            "55.75204\t37.61781\tRU\t10381222\tMoscow|Москва\n\
             52.52437\t13.41053\tDE\t3426354\tBerlin|Берлин\n",
        )
    }

    fn place(name: &str) -> Ask {
        match ask(name, &cities()) {
            Some(Ask::Place(p)) => Ask::Place(p),
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn a_bare_query_asks_only_for_a_listed_city() {
        let cities = cities();
        let Some(Ask::Place(berlin)) = ask("berlin", &cities) else {
            panic!()
        };
        assert_eq!(berlin.name, "Berlin, DE");
        let Some(Ask::Place(moscow)) = ask("  москва ", &cities) else {
            panic!()
        };
        assert_eq!(moscow.name, "Москва, RU");
        for not in ["", "ber", "Tokyo", "weather", "погода", "52.5, 13.4"] {
            assert_eq!(ask(not, &cities), None, "{not:?}");
        }
    }

    #[test]
    fn weather_in_a_city_looks_it_up_unless_it_is_listed() {
        let cities = cities();
        for query in [
            "weather in Berlin",
            "Weather berlin",
            "berlin weather",
            "погода в Берлин",
        ] {
            assert!(
                matches!(ask(query, &cities), Some(Ask::Place(_))),
                "{query}"
            );
        }
        // A city looked up is kept as typed.
        assert_eq!(
            ask("weather in Tokyo", &cities),
            Some(Ask::Lookup("Tokyo".into()))
        );
        assert_eq!(
            ask("Weather in  Saint-Rémy ", &cities),
            Some(Ask::Lookup("Saint-Rémy".into()))
        );
        assert_eq!(
            ask("Saint-Rémy  Weather", &cities),
            Some(Ask::Lookup("Saint-Rémy".into()))
        );
        assert_eq!(
            ask("погода в Урюпинске", &cities),
            Some(Ask::Lookup("Урюпинске".into()))
        );
        assert_eq!(
            ask("погода Тула", &cities),
            Some(Ask::Lookup("Тула".into()))
        );
        let Some(Ask::Place(at)) = ask("weather at 1.5, 2.5", &cities) else {
            panic!()
        };
        assert_eq!((at.latitude, at.longitude), (1.5, 2.5));
        for typing in [
            "weather in ",
            "weather i",
            "weather in t",
            "погода в",
            "погода во",
        ] {
            assert_eq!(ask(typing, &cities), None, "{typing:?}");
        }
    }

    #[test]
    fn params_round_trip() {
        let cities = cities();
        let at = ask("weather at 1.5, 2.5", &cities).unwrap();
        let with_country = ask("weather in Berlin, DE", &cities).unwrap();
        assert_eq!(with_country, place("Berlin"));
        let elsewhere = ask("weather in Berlin, US", &cities).unwrap();
        assert_eq!(elsewhere, Ask::Lookup("Berlin, US".into()));
        for ask in [
            place("Berlin"),
            place("москва"),
            at,
            with_country,
            elsewhere,
            Ask::Lookup("Saint-Rémy".into()),
        ] {
            assert_eq!(city(&ask.param(), &cities), Some(ask));
        }
        assert_eq!(place("Berlin").param(), "Berlin, DE");
    }

    #[test]
    fn a_city_typed_by_hand_is_found() {
        let cities = cities();
        for typed in ["Berlin", " berlin ", "Berlin, DE", "berlin, de"] {
            assert_eq!(city(typed, &cities), Some(place("Berlin")), "{typed:?}");
        }
        // Another name of the list: the same place, named as typed.
        assert_eq!(city("Берлин", &cities), Some(place("берлин")));
        let Some(Ask::Place(at)) = city("52.52, 13.41", &cities) else {
            panic!()
        };
        assert_eq!((at.latitude, at.longitude), (52.52, 13.41));
        // Another country's Berlin, or a city not listed, is looked up.
        assert_eq!(
            city("Berlin, US", &cities),
            Some(Ask::Lookup("Berlin, US".into()))
        );
        assert_eq!(city(" Tokyo ", &cities), Some(Ask::Lookup("Tokyo".into())));
        // Blank or `auto`: the computer's location.
        for location in ["  ", "auto", " AUTO ", "Auto"] {
            assert_eq!(city(location, &cities), None, "{location:?}");
        }
    }

    #[test]
    fn a_listed_city_fetches_its_forecast_once() {
        let mut s = Searched::default();
        let berlin = place("Berlin");
        assert_eq!(s.view(&berlin).reading, None);
        assert_eq!(s.view(&berlin).status, Status::Ok);
        let (job, url) = s.due(&berlin, 0, false).unwrap();
        assert!(
            url.contains("latitude=52.52437&longitude=13.41053"),
            "{url}"
        );
        s.started(job, 7);
        assert_eq!(s.due(&berlin, 0, false), None, "already asked");
        assert!(s.waits_for(7));
        assert!(s.answered(7, Ok((200, FORECAST)), 1));
        assert_eq!(s.view(&berlin).reading.unwrap().temperature, 2.6);
        assert_eq!(s.view(&berlin).name, "Berlin, DE");
        assert_eq!(s.due(&berlin, 60, false), None, "fresh");
        assert!(s.due(&berlin, 1 + REFRESH_SECS, false).is_some(), "stale");
        assert!(!s.answered(99, Ok((200, FORECAST)), 1), "not the searches'");
    }

    #[test]
    fn a_lookup_geocodes_then_fetches() {
        let mut s = Searched::default();
        let moscow = Ask::Lookup("москва".into());
        let (job, url) = s.due(&moscow, 0, false).unwrap();
        assert!(url.contains("&language=ru&"), "{url}");
        s.started(job, 1);
        s.answered(1, Ok((200, MOSCOW)), 0);
        assert_eq!(s.view(&moscow).name, "Москва");
        let (job, _) = s.due(&moscow, 0, false).unwrap();
        assert!(matches!(job, Job::Forecast { .. }));
        s.started(job, 2);
        s.answered(2, Ok((200, FORECAST)), 0);
        assert!(s.view(&moscow).reading.is_some());
    }

    #[test]
    fn an_unknown_city_is_tried_in_each_language_then_left() {
        let mut s = Searched::default();
        let city = Ask::Lookup("урюпинске".into());
        for (id, language) in [(1, "ru"), (2, "uk")] {
            let (job, url) = s.due(&city, 0, false).unwrap();
            assert!(url.contains(&format!("&language={language}&")), "{url}");
            s.started(job, id);
            s.answered(id, Ok((200, MISS)), 0);
        }
        assert_eq!(s.view(&city).status, Status::Unknown);
        assert_eq!(s.view(&city).name, "урюпинске");
        assert_eq!(s.due(&city, 10_000, false), None);
    }

    #[test]
    fn a_failure_waits_a_minute_and_shows() {
        let mut s = Searched::default();
        let berlin = place("Berlin");
        let (job, _) = s.due(&berlin, 0, false).unwrap();
        s.started(job, 1);
        s.answered(1, Err("offline".into()), 0);
        assert_eq!(s.view(&berlin).status, Status::Failed);
        assert_eq!(s.due(&berlin, 59, false), None);
        assert!(s.due(&berlin, 60, false).is_some());
        let tokyo = Ask::Lookup("tokyo".into());
        let (job, _) = s.due(&tokyo, 0, false).unwrap();
        s.start_failed(&job, 0);
        assert_eq!(s.view(&tokyo).status, Status::Failed);
        assert_eq!(s.due(&tokyo, 30, false), None);
    }

    /// The user's refresh asks at once: an unknown city again, a failed place within the
    /// minute, a fresh reading again.
    #[test]
    fn a_forced_fetch_asks_at_once() {
        let mut s = Searched::default();
        let city = Ask::Lookup("Xyzzy".into());
        let (job, url) = s.due(&city, 0, false).unwrap();
        assert!(url.contains("name=xyzzy&"), "looked up by its key: {url}");
        s.started(job, 1);
        s.answered(1, Ok((200, MISS)), 0);
        assert_eq!(s.view(&city).status, Status::Unknown);
        assert_eq!(s.view(&city).name, "Xyzzy", "shown as typed");
        assert_eq!(s.due(&city, 0, false), None);
        assert!(s.due(&city, 0, true).is_some());

        let berlin = place("Berlin");
        let (job, _) = s.due(&berlin, 0, false).unwrap();
        s.started(job, 2);
        s.answered(2, Err("offline".into()), 0);
        assert_eq!(s.due(&berlin, 10, false), None);
        let (job, _) = s.due(&berlin, 10, true).unwrap();
        s.started(job, 3);
        s.answered(3, Ok((200, FORECAST)), 10);
        assert_eq!(s.due(&berlin, 20, false), None, "fresh");
        assert!(s.due(&berlin, 20, true).is_some());
    }

    #[test]
    fn at_most_two_requests_wait() {
        let mut s = Searched::default();
        for (id, city) in ["a", "b"].into_iter().enumerate() {
            let (job, _) = s.due(&Ask::Lookup(city.into()), 0, false).unwrap();
            s.started(job, id as u64);
        }
        assert_eq!(s.due(&Ask::Lookup("c".into()), 0, false), None);
    }

    #[test]
    fn new_units_forget_the_readings() {
        let mut s = Searched::default();
        let berlin = place("Berlin");
        let (job, _) = s.due(&berlin, 0, false).unwrap();
        s.started(job, 1);
        s.answered(1, Ok((200, FORECAST)), 0);
        s.set_units(Units::Imperial);
        assert_eq!(s.view(&berlin).reading, None);
        assert!(s.due(&berlin, 0, false).unwrap().1.contains("fahrenheit"));
    }

    #[test]
    fn old_entries_are_forgotten() {
        let mut s = Searched::default();
        for i in 0..=MAX_ENTRIES as u64 {
            let ask = Ask::Place(Place {
                name: format!("P{i}"),
                latitude: i as f64,
                longitude: 0.,
            });
            let (job, _) = s.due(&ask, 0, false).unwrap();
            s.started(job, i);
            s.answered(i, Ok((200, FORECAST)), 0);
        }
        assert!(s.spots.len() <= MAX_ENTRIES, "{}", s.spots.len());
    }
}
