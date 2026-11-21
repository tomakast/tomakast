//! The weather of a city typed in the launcher search, as a condition tile (the quarter
//! widget `city`): pure, so it is tested natively; `lib.rs` does the imports.
//!
//! - [`ask`] reads a query: `weather in <city>` (also `weather <city>`, `<city> weather`,
//!   `погода в <city>`, `погода <city>`, `<city> погода`) asks for that city, found in the
//!   built-in list, as `latitude, longitude`, or else geocoded by Open-Meteo. A bare query
//!   shows a tile only when it is a city of the built-in list: what else the user types is
//!   never sent anywhere.
//! - [`Searched`] holds what searches asked for: the places looked up and the readings of
//!   places, each fresh for [`REFRESH_SECS`](crate::state::REFRESH_SECS). At most two requests
//!   wait at once; a place that failed is tried again a minute later at the earliest. Old
//!   entries are dropped past [`MAX_ENTRIES`].
//! - The tile's payload names what it shows ([`Ask::payload`]), so a render, an action or
//!   an answer that comes after more typing finds its own place.

use std::collections::HashMap;

use crate::api::{self, Geocoded, Place, Reading, Units};
use crate::state::{REFRESH_SECS, Status};
use crate::tile::View;
use tomakast_places::cities::{self, Cities};

/// The words that ask for the weather, before the city (on [`cities::key`]s).
const PREFIXES: &[&str] = &[
    "weather in ",
    "weather at ",
    "weather for ",
    "weather ",
    "погода в ",
    "погода во ",
    "погода ",
];
/// What a query that ends in it has not named the city yet.
const CONNECTORS: &[&str] = &["in", "at", "for", "в", "во"];
/// The same, after the city.
const SUFFIXES: &[&str] = &[" weather", " погода"];
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
    /// A city to geocode, as typed (its key).
    Lookup(String),
}

/// What `query` asks for, if anything (see the module doc).
pub fn ask(query: &str, cities: &Cities) -> Option<Ask> {
    let typed = cities::key(query);
    let named = PREFIXES
        .iter()
        .find_map(|p| typed.strip_prefix(p))
        .or_else(|| SUFFIXES.iter().find_map(|s| typed.strip_suffix(s)));
    let Some(city) = named else {
        return cities.find(&typed).map(Ask::Place);
    };
    let city = city.trim();
    // Still typing: `weather in` before the city, or one letter of it.
    if city.chars().count() < 2 || CONNECTORS.contains(&city) {
        return None;
    }
    Some(
        cities
            .find(city)
            .or_else(|| api::coordinates(city))
            .map_or_else(|| Ask::Lookup(city.to_owned()), Ask::Place),
    )
}

impl Ask {
    /// The tile's payload: `p\t<lat>\t<lon>\t<name>` or `q\t<city>`.
    pub fn payload(&self) -> String {
        match self {
            Ask::Place(p) => format!("p\t{}\t{}\t{}", p.latitude, p.longitude, p.name),
            Ask::Lookup(city) => format!("q\t{city}"),
        }
    }

    /// The ask a [`payload`](Self::payload) names.
    pub fn from_payload(payload: &str) -> Option<Ask> {
        let mut parts = payload.splitn(4, '\t');
        match parts.next()? {
            "p" => {
                let latitude: f64 = parts.next()?.parse().ok()?;
                let longitude: f64 = parts.next()?.parse().ok()?;
                let name = parts.next()?.to_owned();
                (latitude.is_finite() && longitude.is_finite()).then_some(Ask::Place(Place {
                    name,
                    latitude,
                    longitude,
                }))
            }
            "q" => Some(Ask::Lookup(parts.next()?.to_owned())),
            _ => None,
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
            Ask::Lookup(city) => match self.geo.get(city)? {
                Geo::Found(place) => Some(place.clone()),
                _ => None,
            },
        }
    }

    /// The request `ask` needs at `now`, with its URL, if any.
    pub fn due(&self, ask: &Ask, now: i64) -> Option<(Job, String)> {
        if self.pending.len() >= MAX_PENDING {
            return None;
        }
        let waits = |job: &Job| self.pending.iter().any(|(_, j)| j == job);
        if let Ask::Lookup(city) = ask {
            let language = match self.geo.get(city) {
                None => Some(0),
                Some(Geo::Next(language)) => Some(*language),
                Some(Geo::Failed(at)) if now - at >= RETRY_SECS => Some(0),
                Some(Geo::Failed(_) | Geo::Unknown) => return None,
                Some(Geo::Found(_)) => None,
            };
            if let Some(language) = language {
                let job = Job::Geocode {
                    city: city.clone(),
                    language,
                };
                let url = api::geocoding_url(city, api::languages(city)[language]);
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
        (!fresh && !failed && !waits(&job)).then_some((job, url))
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
            Ask::Lookup(city) => self.geo.get(city),
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
        assert_eq!(
            ask("weather in Tokyo", &cities),
            Some(Ask::Lookup("tokyo".into()))
        );
        assert_eq!(
            ask("погода в Урюпинске", &cities),
            Some(Ask::Lookup("урюпинске".into()))
        );
        assert_eq!(
            ask("погода Тула", &cities),
            Some(Ask::Lookup("тула".into()))
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
    fn payloads_round_trip() {
        for ask in [place("Berlin"), Ask::Lookup("tokyo".into())] {
            assert_eq!(Ask::from_payload(&ask.payload()), Some(ask));
        }
        assert_eq!(Ask::from_payload("x\ty"), None);
        assert_eq!(Ask::from_payload("p\tnan\t1\tX"), None);
    }

    #[test]
    fn a_listed_city_fetches_its_forecast_once() {
        let mut s = Searched::default();
        let berlin = place("Berlin");
        assert_eq!(s.view(&berlin).reading, None);
        assert_eq!(s.view(&berlin).status, Status::Ok);
        let (job, url) = s.due(&berlin, 0).unwrap();
        assert!(
            url.contains("latitude=52.52437&longitude=13.41053"),
            "{url}"
        );
        s.started(job, 7);
        assert_eq!(s.due(&berlin, 0), None, "already asked");
        assert!(s.waits_for(7));
        assert!(s.answered(7, Ok((200, FORECAST)), 1));
        assert_eq!(s.view(&berlin).reading.unwrap().temperature, 2.6);
        assert_eq!(s.view(&berlin).name, "Berlin, DE");
        assert_eq!(s.due(&berlin, 60), None, "fresh");
        assert!(s.due(&berlin, 1 + REFRESH_SECS).is_some(), "stale");
        assert!(!s.answered(99, Ok((200, FORECAST)), 1), "not the searches'");
    }

    #[test]
    fn a_lookup_geocodes_then_fetches() {
        let mut s = Searched::default();
        let moscow = Ask::Lookup("москва".into());
        let (job, url) = s.due(&moscow, 0).unwrap();
        assert!(url.contains("&language=ru&"), "{url}");
        s.started(job, 1);
        s.answered(1, Ok((200, MOSCOW)), 0);
        assert_eq!(s.view(&moscow).name, "Москва");
        let (job, _) = s.due(&moscow, 0).unwrap();
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
            let (job, url) = s.due(&city, 0).unwrap();
            assert!(url.contains(&format!("&language={language}&")), "{url}");
            s.started(job, id);
            s.answered(id, Ok((200, MISS)), 0);
        }
        assert_eq!(s.view(&city).status, Status::Unknown);
        assert_eq!(s.view(&city).name, "урюпинске");
        assert_eq!(s.due(&city, 10_000), None);
    }

    #[test]
    fn a_failure_waits_a_minute_and_shows() {
        let mut s = Searched::default();
        let berlin = place("Berlin");
        let (job, _) = s.due(&berlin, 0).unwrap();
        s.started(job, 1);
        s.answered(1, Err("offline".into()), 0);
        assert_eq!(s.view(&berlin).status, Status::Failed);
        assert_eq!(s.due(&berlin, 59), None);
        assert!(s.due(&berlin, 60).is_some());
        let tokyo = Ask::Lookup("tokyo".into());
        let (job, _) = s.due(&tokyo, 0).unwrap();
        s.start_failed(&job, 0);
        assert_eq!(s.view(&tokyo).status, Status::Failed);
        assert_eq!(s.due(&tokyo, 30), None);
    }

    #[test]
    fn at_most_two_requests_wait() {
        let mut s = Searched::default();
        for (id, city) in ["a", "b"].into_iter().enumerate() {
            let (job, _) = s.due(&Ask::Lookup(city.into()), 0).unwrap();
            s.started(job, id as u64);
        }
        assert_eq!(s.due(&Ask::Lookup("c".into()), 0), None);
    }

    #[test]
    fn new_units_forget_the_readings() {
        let mut s = Searched::default();
        let berlin = place("Berlin");
        let (job, _) = s.due(&berlin, 0).unwrap();
        s.started(job, 1);
        s.answered(1, Ok((200, FORECAST)), 0);
        s.set_units(Units::Imperial);
        assert_eq!(s.view(&berlin).reading, None);
        assert!(s.due(&berlin, 0).unwrap().1.contains("fahrenheit"));
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
            let (job, _) = s.due(&ask, 0).unwrap();
            s.started(job, i);
            s.answered(i, Ok((200, FORECAST)), 0);
        }
        assert!(s.spots.len() <= MAX_ENTRIES, "{}", s.spots.len());
    }
}
