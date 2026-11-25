//! Every place the tiles want, in one store: pure, so it is tested natively; `lib.rs` does
//! the imports it asks for.
//!
//! - A tile names a place by its param `city` ([`where_of`]): a city, or, without one, the
//!   computer's location. The places wanted are those of the Home tiles, the search's tile
//!   once its query rests, and the location when one of those has no city. The search's
//!   tile (its widget and place) going off screen ends the search, and coming back on screen
//!   starts it again; an older tile of the same place does neither.
//! - A place has one forecast request and, when one of its tiles shows air quality or pollen,
//!   one air request ([`Feed`]), however many tiles name it, and however they spell it:
//!   places are kept by their coordinates. A city not in the built-in list is looked up
//!   first, in each of its [`languages`](api::languages) in turn; one geocoding does not know
//!   is asked again only when the user asks. A failed lookup backs off as a forecast does.
//! - [`Places::due`] says what to fetch: every wanted place is kept fresh (a forecast
//!   [`REFRESH_SECS`](crate::feed::REFRESH_SECS) old is fetched again, failures back off), at
//!   most [`MAX_IN_FLIGHT`] requests at once. A request refused because the host is busy
//!   waits for the next call, with no failure. [`Places::force_all`] makes every wanted
//!   place due at once (the user's refresh of everything).
//! - A storage key that may hold a kept answer (a Home place's, the location's when a Home
//!   tile has no city) goes once no tile names it by that key, when Home or the search
//!   changes; the place's forecast and air go too, unless a tile (Home, the search or the
//!   location) still names its coordinates. Places only searched, the location searched
//!   without a city included, stay in a cache trimmed past [`MAX_SPOTS`], which never trims
//!   a wanted place.
//! - Each good answer of a Home place (the location's included) is kept in storage under
//!   `forecast:<key>` or `air:<key>` ([`Where::key`], [`encode`]) and read back with
//!   [`Places::restore`], so the tiles draw at once after a restart.

use std::collections::HashMap;

use tomakast_places::cities::{self, Cities};
use tomakast_plugin::{WidgetParam, param};

use crate::api::{self, Air, Geocoded, Place, Reading, TempUnit, Units};
use crate::feed::{Feed, backoff};
use crate::location::{Location, Status};
use crate::search::{self, Ask};
use crate::tile::View;

/// Requests waiting at once: the host's limit.
pub const MAX_IN_FLIGHT: usize = 4;
/// Places and lookups kept; past it, those no tile wants and no request waits for go.
pub const MAX_SPOTS: usize = 32;
/// Longer storage keys are not kept.
const MAX_KEY_BYTES: usize = 240;
/// The tile param that names the place.
const CITY: &str = "city";

/// The place a tile names.
#[derive(Debug, Clone, PartialEq)]
pub enum Where {
    /// The computer's location.
    Here,
    /// A city: the tile's param as typed (trimmed), and what it names.
    City { typed: String, ask: Ask },
}

/// The place a tile with `params` names: its param `city`, read by [`search::city`].
pub fn where_of(params: &[WidgetParam], cities: &Cities) -> Where {
    let Some(typed) = param(params, CITY) else {
        return Where::Here;
    };
    match search::city(typed, cities) {
        Some(ask) => Where::City {
            typed: typed.trim().to_owned(),
            ask,
        },
        None => Where::Here,
    }
}

impl Where {
    /// The place's storage key: `@here`, or the [`cities::key`] of the city as typed.
    pub fn key(&self) -> String {
        match self {
            Where::Here => "@here".into(),
            Where::City { typed, .. } => cities::key(typed),
        }
    }

    /// The key of the city to look up, when it is one.
    fn lookup(&self) -> Option<String> {
        match self {
            Where::City {
                ask: Ask::Lookup(typed),
                ..
            } => Some(cities::key(typed)),
            _ => None,
        }
    }
}

/// A tile that wants a place.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    /// The tile's widget.
    pub widget: String,
    pub at: Where,
    /// The tile shows the place's air.
    pub air: bool,
}

/// A request to start.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// The geocoding of a city (its [`cities::key`]) in its `language`th language.
    Geocode {
        city: String,
        language: usize,
    },
    Forecast {
        place: Place,
    },
    Air {
        place: Place,
    },
}

impl Job {
    /// Whether `other` asks the same: one city's lookup, or one kind of request at the same
    /// coordinates, whatever the place is called.
    fn same_request(&self, other: &Job) -> bool {
        match (self, other) {
            (Job::Geocode { city: a, .. }, Job::Geocode { city: b, .. }) => a == b,
            (Job::Forecast { place: a }, Job::Forecast { place: b })
            | (Job::Air { place: a }, Job::Air { place: b }) => spot_key(a) == spot_key(b),
            _ => false,
        }
    }

    /// The URL to ask, the forecast in `units`.
    pub fn url(&self, units: Units) -> String {
        match self {
            Job::Geocode { city, language } => {
                let languages = api::languages(city);
                api::geocoding_url(city, languages[(*language).min(languages.len() - 1)])
            }
            Job::Forecast { place } => api::forecast_url(place, units),
            Job::Air { place } => api::air_url(place),
        }
    }
}

/// What storage keeps of a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Forecast,
    Air,
}

/// What an answer did.
#[derive(Debug, Clone, PartialEq)]
pub struct Answered {
    /// What the tiles show changed.
    pub changed: bool,
    /// The answer could be read: a forecast, air, or a lookup's (a city not known included).
    pub good: bool,
    /// The storage keys and values to write: one for every Home `Where` (the location's
    /// included) naming the place, after a good forecast or air answer.
    pub keep: Vec<(String, String)>,
}

/// What a lookup found.
#[derive(Debug, Clone, PartialEq)]
enum Geo {
    Found(Place),
    /// Not found in the languages tried so far; this one is next.
    Next(usize),
    Unknown,
    /// Failed at `at`, `failures` times in a row; tried again after the [`backoff`].
    Failed {
        at: i64,
        failures: u32,
    },
}

/// A place's requests, kept by its coordinates.
#[derive(Debug, Default)]
struct Spot {
    forecast: Feed<Reading>,
    air: Feed<Air>,
}

/// See the module doc.
#[derive(Debug, Default)]
pub struct Places {
    units: Units,
    /// Lookups by city key.
    geo: HashMap<String, Geo>,
    /// The lookups out: request id, city key, language.
    lookups: Vec<(u64, String, usize)>,
    /// Places by [`spot_key`].
    spots: HashMap<String, Spot>,
    home: Vec<Want>,
    /// The search's want, and whether its query rested.
    search: Option<(Want, bool)>,
    /// The search ended by its tile going off screen, as it was: it is wanted again when
    /// the tile comes back.
    hidden: Option<(Want, bool)>,
    /// The location's place.
    here: Option<Place>,
    /// The places whose storage keys may hold a kept answer: Home's (the location's when a
    /// Home tile has no city); each goes (and is returned) once no tile names its key.
    kept: Vec<Where>,
    /// The spots and the lookups the user asked to fetch at once ([`Places::force_all`]),
    /// until their request starts.
    forced_spots: Vec<(Kind, String)>,
    forced_lookups: Vec<String>,
}

/// A place's key in [`Places::spots`].
fn spot_key(place: &Place) -> String {
    format!("{},{}", place.latitude, place.longitude)
}

impl Places {
    /// Forecasts in other units are forgotten, and the answers of their requests ignored; air
    /// is kept.
    pub fn set_units(&mut self, units: Units) {
        if units != self.units {
            self.units = units;
            for spot in self.spots.values_mut() {
                spot.forecast = Feed::default();
            }
        }
    }

    /// The Home tiles' wants. Returns the storage keys that may hold a kept answer and that
    /// no tile names now (neither Home, nor the search, nor the location), each once
    /// ([`release`](Self::release)).
    pub fn set_home(&mut self, wants: Vec<Want>) -> Vec<String> {
        self.home = wants;
        for at in self.home.iter().map(|want| &want.at) {
            if !self.kept.iter().any(|k| k.key() == at.key()) {
                self.kept.push(at.clone());
            }
        }
        self.release()
    }

    /// The search tile's want, or none. It counts once [`search_rested`](Self::search_rested).
    /// Returns the storage keys no tile names now, as [`set_home`](Self::set_home) does: a
    /// Home key the search held back goes once the search moves on.
    pub fn set_search(&mut self, want: Option<Want>) -> Vec<String> {
        self.search = want.map(|want| (want, false));
        self.hidden = None;
        self.release()
    }

    /// A tile of `widget` at `at` went off screen. When it is the search's tile, the search
    /// ends (it is remembered for [`search_shown`](Self::search_shown)) and the storage keys
    /// no tile names now are returned, as [`set_search`](Self::set_search) does; another
    /// tile, an older search's of the same place included, changes nothing.
    pub fn search_hidden(&mut self, widget: &str, at: &Where) -> Option<Vec<String>> {
        if !self
            .search
            .as_ref()
            .is_some_and(|(want, _)| want.widget == widget && want.at == *at)
        {
            return None;
        }
        self.hidden = self.search.take();
        Some(self.release())
    }

    /// A tile of `widget` at `at` came on screen. When it is the search's tile that went off
    /// screen, the search is wanted again as it was; returns whether its query had rested.
    pub fn search_shown(&mut self, widget: &str, at: &Where) -> Option<bool> {
        if !self
            .hidden
            .as_ref()
            .is_some_and(|(want, _)| want.widget == widget && want.at == *at)
        {
            return None;
        }
        self.search = self.hidden.take();
        self.search.as_ref().map(|(_, rested)| *rested)
    }

    /// Forgets the places of [`kept`](Self::kept) no tile names by key any more, and returns
    /// their keys. A place's forecast and air stay while any tile names its coordinates.
    fn release(&mut self) -> Vec<String> {
        let wanted = self.wanted_keys();
        let (kept, forgotten): (Vec<Where>, Vec<Where>) = std::mem::take(&mut self.kept)
            .into_iter()
            .partition(|at| wanted.keys.contains(&at.key()));
        self.kept = kept;
        for at in &forgotten {
            if let Some(spot) = self.place_of(at).map(spot_key)
                && !wanted.spots.contains(&spot)
            {
                self.spots.remove(&spot);
            }
            if let Some(city) = at.lookup()
                && !wanted.lookups.contains(&city)
                && !self.lookups.iter().any(|(_, c, _)| *c == city)
            {
                self.geo.remove(&city);
            }
        }
        self.forced_spots
            .retain(|(_, spot)| wanted.spots.contains(spot));
        self.forced_lookups
            .retain(|city| wanted.lookups.contains(city));
        forgotten.iter().map(Where::key).collect()
    }

    /// The user asked for everything fresh: every wanted place (its forecast, its air when
    /// wanted, its lookup when it is a city not found yet) is due at once, until its request
    /// starts, though more than fit in one turn. A request already out is not asked again:
    /// its answer is as fresh.
    pub fn force_all(&mut self) {
        let wanted = self.wanted_keys();
        let out = |kind: Kind, spot: &String| {
            self.spots.get(spot).is_some_and(|s| match kind {
                Kind::Forecast => s.forecast.waiting(),
                Kind::Air => s.air.waiting(),
            })
        };
        self.forced_spots = wanted
            .spots
            .into_iter()
            .flat_map(|spot| [(Kind::Forecast, spot.clone()), (Kind::Air, spot)])
            .filter(|(kind, spot)| !out(*kind, spot))
            .collect();
        self.forced_lookups = wanted
            .lookups
            .into_iter()
            .filter(|city| !self.lookups.iter().any(|(_, c, _)| c == city))
            .collect();
    }

    /// The search's query rested: its place is wanted.
    pub fn search_rested(&mut self) {
        if let Some((_, rested)) = &mut self.search {
            *rested = true;
        }
    }

    /// The location's place, once told.
    pub fn set_here(&mut self, place: Option<Place>) {
        self.here = place;
    }

    /// The location's place as known here (told, or restored from storage).
    pub fn here(&self) -> Option<&Place> {
        self.here.as_ref()
    }

    /// Whether a tile without a city is on Home or is the search's (rested or not).
    pub fn wants_here(&self) -> bool {
        self.home.iter().any(|want| want.at == Where::Here)
            || self
                .search
                .as_ref()
                .is_some_and(|(want, _)| want.at == Where::Here)
    }

    /// The requests to start at `now`, see the module doc. `forced` (the user asked for that
    /// place) skips the freshness and the wait after a failure of its requests, and asks an
    /// unknown city again.
    pub fn due(&self, now: i64, forced: Option<&Where>) -> Vec<Job> {
        let forced_lookup = forced.and_then(Where::lookup);
        let forced_spot = forced.and_then(|at| self.place_of(at)).map(spot_key);
        let room = MAX_IN_FLIGHT.saturating_sub(self.in_flight());
        let mut jobs = Vec::new();
        let add = |job: Job, jobs: &mut Vec<Job>| {
            if jobs.len() < room && !jobs.iter().any(|j| j.same_request(&job)) {
                jobs.push(job);
            }
        };
        let rested = self.search.iter().filter(|(_, rested)| *rested);
        for want in rested.map(|(want, _)| want).chain(&self.home) {
            if let Some(city) = want.at.lookup() {
                let force =
                    forced_lookup.as_ref() == Some(&city) || self.forced_lookups.contains(&city);
                let language = match self.geo.get(&city) {
                    None => Some(0),
                    Some(Geo::Next(language)) => Some(*language),
                    Some(Geo::Found(_)) => None,
                    Some(Geo::Failed { .. } | Geo::Unknown) if force => Some(0),
                    Some(Geo::Failed { at, failures }) if now - at >= backoff(*failures) => Some(0),
                    Some(Geo::Failed { .. } | Geo::Unknown) => continue,
                };
                if let Some(language) = language {
                    if !self.lookups.iter().any(|(_, c, _)| *c == city) {
                        add(Job::Geocode { city, language }, &mut jobs);
                    }
                    continue;
                }
            }
            let Some(place) = self.place_of(&want.at) else {
                continue;
            };
            let key = spot_key(place);
            let force = forced_spot.as_ref() == Some(&key);
            let forced = |kind| force || self.forced_spots.contains(&(kind, key.clone()));
            let spot = self.spots.get(&key);
            if spot.is_none_or(|s| s.forecast.due(now, forced(Kind::Forecast))) {
                let place = place.clone();
                add(Job::Forecast { place }, &mut jobs);
            }
            if want.air && spot.is_none_or(|s| s.air.due(now, forced(Kind::Air))) {
                let place = place.clone();
                add(Job::Air { place }, &mut jobs);
            }
        }
        jobs
    }

    /// `job` was started as request `id`.
    pub fn started(&mut self, job: Job, id: u64) {
        self.unforce(&job);
        match job {
            Job::Geocode { city, language } => self.lookups.push((id, city, language)),
            Job::Forecast { place } => self.spot(&place).forecast.started(id),
            Job::Air { place } => self.spot(&place).air.started(id),
        }
    }

    /// Starting `job` failed at `now`. `busy`: the host refused at its limit of requests, so
    /// the job stays due, with no failure.
    pub fn cannot_start(&mut self, job: &Job, now: i64, busy: bool) {
        if busy {
            return;
        }
        self.unforce(job);
        match job {
            Job::Geocode { city, .. } => {
                let failed = self.lookup_failed(city, now);
                self.geo.insert(city.clone(), failed);
            }
            Job::Forecast { place } => self.spot(place).forecast.start_failed(now),
            Job::Air { place } => self.spot(place).air.start_failed(now),
        }
        self.trim();
    }

    /// The answer to request `id` at `now`: the status and body, or why there is none. `None`
    /// when no request of these places waits for it.
    pub fn answered(
        &mut self,
        id: u64,
        answer: Result<(u16, &[u8]), String>,
        now: i64,
    ) -> Option<Answered> {
        let answer = answer.ok();
        if let Some(at) = self.lookups.iter().position(|(p, ..)| *p == id) {
            let (_, city, language) = self.lookups.swap_remove(at);
            let geo = match answer.and_then(|(status, body)| api::parse_geocoding(status, body)) {
                Some(Geocoded::Found(place)) => Geo::Found(place),
                Some(Geocoded::Unknown) if language + 1 < api::languages(&city).len() => {
                    Geo::Next(language + 1)
                }
                Some(Geocoded::Unknown) => Geo::Unknown,
                None => self.lookup_failed(&city, now),
            };
            let changed = self.geo.get(&city) != Some(&geo);
            let good = !matches!(geo, Geo::Failed { .. });
            self.geo.insert(city, geo);
            self.trim();
            return Some(Answered {
                changed,
                good,
                keep: Vec::new(),
            });
        }
        let (key, spot) = self
            .spots
            .iter_mut()
            .find(|(_, s)| s.forecast.waits_for(id) || s.air.waits_for(id))?;
        let key = key.clone();
        let (kind, changed, fetched) = if spot.forecast.waits_for(id) {
            let reading = answer
                .and_then(|(status, body)| api::parse_forecast(status, body, self.units, now));
            let fetched = reading.as_ref().map(|r| r.fetched);
            (
                Kind::Forecast,
                spot.forecast.answered(reading, now),
                fetched,
            )
        } else {
            let air = answer.and_then(|(status, body)| api::parse_air(status, body, now));
            let fetched = air.as_ref().map(|a| a.fetched);
            (Kind::Air, spot.air.answered(air, now), fetched)
        };
        let keep = match (fetched, answer) {
            (Some(fetched), Some((_, body))) => self.keep(&key, kind, fetched, body),
            _ => Vec::new(),
        };
        self.trim();
        Some(Answered {
            changed,
            good: fetched.is_some(),
            keep,
        })
    }

    /// Takes back what storage kept for `at` ([`encode`]): its place, so a city needs no
    /// lookup, and its forecast or air. Ignored when it was kept in other units (a forecast),
    /// names another place than `at` does, or is garbage; never overwrites a newer value.
    pub fn restore(&mut self, at: &Where, kind: Kind, text: &str) {
        let Some((word, fetched, place, body)) = decode(text) else {
            return;
        };
        if self
            .place_of(at)
            .is_some_and(|known| spot_key(known) != spot_key(&place))
        {
            return;
        }
        let bytes = body.as_bytes();
        match kind {
            Kind::Forecast => {
                if word != units_word(self.units) {
                    return;
                }
                let Some(reading) = api::parse_forecast(200, bytes, self.units, fetched) else {
                    return;
                };
                let feed = &mut self.spot(&place).forecast;
                if feed
                    .value
                    .as_ref()
                    .is_none_or(|v| v.fetched < reading.fetched)
                {
                    feed.value = Some(reading);
                }
            }
            Kind::Air => {
                let Some(air) = api::parse_air(200, bytes, fetched) else {
                    return;
                };
                let feed = &mut self.spot(&place).air;
                if feed.value.as_ref().is_none_or(|v| v.fetched < air.fetched) {
                    feed.value = Some(air);
                }
            }
        }
        match at {
            Where::Here if self.here.is_none() => self.here = Some(place),
            Where::City {
                ask: Ask::Lookup(typed),
                ..
            } => {
                self.geo.insert(cities::key(typed), Geo::Found(place));
            }
            _ => {}
        }
    }

    /// What the tiles of `at` show; `here` is the location, for its status.
    pub fn view(&self, at: &Where, here: &Location) -> View<'_> {
        let place = self.place_of(at);
        let spot = place.and_then(|p| self.spots.get(&spot_key(p)));
        let (name, city) = match at {
            Where::Here => (
                place.map_or_else(|| "Your location".into(), |p| p.name.clone()),
                String::new(),
            ),
            Where::City { typed, .. } => (
                place.map_or_else(|| typed.clone(), |p| p.name.clone()),
                typed.clone(),
            ),
        };
        let lookup = at.lookup().and_then(|city| self.geo.get(&city));
        let status = match (at, lookup) {
            (Where::Here, _) if here.status() != Status::Ok => here.status(),
            (_, Some(Geo::Unknown)) => Status::Unknown,
            (_, Some(Geo::Failed { .. })) => Status::Failed,
            _ if spot.is_some_and(|s| s.forecast.failed()) => Status::Failed,
            _ => Status::Ok,
        };
        View {
            name,
            reading: spot.and_then(|s| s.forecast.value.as_ref()),
            status,
            city,
            air: spot.and_then(|s| s.air.value.as_ref()),
            air_status: if spot.is_some_and(|s| s.air.failed()) {
                Status::Failed
            } else {
                Status::Ok
            },
        }
    }

    /// The places wanted, rested or not: the search's, Home's, and the location.
    fn wanted(&self) -> Vec<&Where> {
        let mut wanted: Vec<&Where> = self.search.iter().map(|(want, _)| &want.at).collect();
        wanted.extend(self.home.iter().map(|want| &want.at));
        if self.wants_here() {
            wanted.push(&Where::Here);
        }
        wanted
    }

    /// The keys of the places wanted, rested or not.
    fn wanted_keys(&self) -> Wanted {
        let wanted = self.wanted();
        Wanted {
            keys: wanted.iter().map(|at| at.key()).collect(),
            spots: wanted
                .iter()
                .filter_map(|at| self.place_of(at))
                .map(spot_key)
                .collect(),
            lookups: wanted.iter().filter_map(|at| at.lookup()).collect(),
        }
    }

    /// The place `at` names, once known.
    fn place_of<'a>(&'a self, at: &'a Where) -> Option<&'a Place> {
        match at {
            Where::Here => self.here.as_ref(),
            Where::City {
                ask: Ask::Place(place),
                ..
            } => Some(place),
            Where::City {
                ask: Ask::Lookup(typed),
                ..
            } => match self.geo.get(&cities::key(typed))? {
                Geo::Found(place) => Some(place),
                _ => None,
            },
        }
    }

    /// `job` started (or failed to): no longer forced.
    fn unforce(&mut self, job: &Job) {
        let (kind, place) = match job {
            Job::Geocode { city, .. } => {
                self.forced_lookups.retain(|c| c != city);
                return;
            }
            Job::Forecast { place } => (Kind::Forecast, place),
            Job::Air { place } => (Kind::Air, place),
        };
        let key = spot_key(place);
        self.forced_spots
            .retain(|(k, spot)| !(*k == kind && *spot == key));
    }

    /// What a lookup of `city` failing at `now` makes it: one more failure in a row.
    fn lookup_failed(&self, city: &str, now: i64) -> Geo {
        let before = match self.geo.get(city) {
            Some(Geo::Failed { failures, .. }) => *failures,
            _ => 0,
        };
        Geo::Failed {
            at: now,
            failures: before.saturating_add(1),
        }
    }

    /// The spot of `place`, made when there is none.
    fn spot(&mut self, place: &Place) -> &mut Spot {
        self.spots.entry(spot_key(place)).or_default()
    }

    /// The requests out.
    fn in_flight(&self) -> usize {
        let feeds = self
            .spots
            .values()
            .map(|s| usize::from(s.forecast.waiting()) + usize::from(s.air.waiting()));
        self.lookups.len() + feeds.sum::<usize>()
    }

    /// What to keep in storage after a good answer of `kind` at the spot `key`, fetched at
    /// `fetched`: the text for each Home `Where` naming it. The location wanted only by the
    /// search is a searched place, and not kept.
    fn keep(&self, key: &str, kind: Kind, fetched: i64, body: &[u8]) -> Vec<(String, String)> {
        let body = String::from_utf8_lossy(body);
        let prefix = match kind {
            Kind::Forecast => "forecast:",
            Kind::Air => "air:",
        };
        let mut keep: Vec<(String, String)> = Vec::new();
        for at in self.home.iter().map(|want| &want.at) {
            let Some(place) = self.place_of(at).filter(|p| spot_key(p) == key) else {
                continue;
            };
            let name = format!("{prefix}{}", at.key());
            if name.len() <= MAX_KEY_BYTES && !keep.iter().any(|(k, _)| *k == name) {
                keep.push((name, encode(self.units, place, fetched, &body)));
            }
        }
        keep
    }

    /// Forgets what no tile wants and no request waits for, once there is more than
    /// [`MAX_SPOTS`] of it.
    fn trim(&mut self) {
        let wanted = self.wanted_keys();
        if self.spots.len() > MAX_SPOTS {
            self.spots.retain(|key, s| {
                wanted.spots.contains(key) || s.forecast.waiting() || s.air.waiting()
            });
        }
        if self.geo.len() > MAX_SPOTS {
            let out = &self.lookups;
            self.geo.retain(|city, _| {
                wanted.lookups.contains(city) || out.iter().any(|(_, c, _)| c == city)
            });
        }
    }
}

/// What the places wanted are known by.
struct Wanted {
    /// Their storage keys.
    keys: Vec<String>,
    /// Their spots, once their places are known.
    spots: Vec<String>,
    /// The keys of the cities among them to look up.
    lookups: Vec<String>,
}

/// The text storage keeps for an answer in `units` at `place` (Decision 11): a line each for
/// the units, the time it was fetched, the place's name and coordinates, then the raw answer
/// on one line.
pub fn encode(units: Units, place: &Place, fetched: i64, body: &str) -> String {
    format!(
        "units={}\nfetched={fetched}\nname={}\nlat={}\nlon={}\nbody={}",
        units_word(units),
        one_line(&place.name),
        place.latitude,
        place.longitude,
        one_line(body)
    )
}

/// What [`encode`] wrote: the units word, the time fetched, the place and the answer.
fn decode(text: &str) -> Option<(&str, i64, Place, &str)> {
    let (head, body) = text.split_once("\nbody=")?;
    let get = |key: &str| {
        head.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    };
    let num = |key| get(key)?.parse::<f64>().ok().filter(|v| v.is_finite());
    let place = Place {
        name: get("name")?.to_owned(),
        latitude: num("lat")?,
        longitude: num("lon")?,
    };
    Some((get("units")?, get("fetched")?.parse().ok()?, place, body))
}

fn units_word(units: Units) -> String {
    let temperature = match units.temperature {
        TempUnit::Celsius => "c",
        TempUnit::Fahrenheit => "f",
    };
    format!("{temperature}-{}", units.wind.label().replace('/', ""))
}

/// `text` without line breaks, for one line of the stored text.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::REFRESH_SECS;
    use crate::measures::needs_air;

    const FORECAST: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin.json");
    const AIR: &[u8] = include_bytes!("../tests/fixtures/air-berlin.json");
    const MOSCOW: &[u8] = include_bytes!("../tests/fixtures/geocoding-moscow-ru.json");
    const MISS: &[u8] = include_bytes!("../tests/fixtures/geocoding-miss.json");

    fn cities() -> Cities {
        Cities::parse(
            "55.75204\t37.61781\tRU\t10381222\tMoscow|Москва\n\
             52.52437\t13.41053\tDE\t3426354\tBerlin|Берлин\n",
        )
    }

    /// The place a tile with the param `city` names; `""` for none.
    fn at(city: &str) -> Where {
        let params = if city.is_empty() {
            Vec::new()
        } else {
            vec![WidgetParam {
                key: "city".into(),
                value: city.into(),
            }]
        };
        where_of(&params, &cities())
    }

    fn want(widget: &str, city: &str) -> Want {
        Want {
            widget: widget.into(),
            at: at(city),
            air: needs_air(widget),
        }
    }

    fn home(p: &mut Places, tiles: &[(&str, &str)]) -> Vec<String> {
        p.set_home(tiles.iter().map(|(w, c)| want(w, c)).collect())
    }

    fn kind(job: &Job) -> &'static str {
        match job {
            Job::Geocode { .. } => "geocode",
            Job::Forecast { .. } => "forecast",
            Job::Air { .. } => "air",
        }
    }

    fn kinds(jobs: &[Job]) -> Vec<&'static str> {
        jobs.iter().map(kind).collect()
    }

    /// Starts every job due at `now`, with ids from `next` on; returns them.
    fn start_all(p: &mut Places, now: i64, next: &mut u64) -> Vec<(u64, Job)> {
        p.due(now, None)
            .into_iter()
            .map(|job| {
                *next += 1;
                p.started(job.clone(), *next);
                (*next, job)
            })
            .collect()
    }

    /// Answers each started job: forecasts with [`FORECAST`], air with [`AIR`].
    fn answer_all(p: &mut Places, started: &[(u64, Job)], now: i64) {
        for (id, job) in started {
            let body = match job {
                Job::Air { .. } => AIR,
                _ => FORECAST,
            };
            assert!(p.answered(*id, Ok((200, body)), now).is_some(), "{job:?}");
        }
    }

    fn here() -> Location {
        Location::default()
    }

    fn berlin() -> Place {
        Place {
            name: "Berlin, DE".into(),
            latitude: 52.52437,
            longitude: 13.41053,
        }
    }

    fn body(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).unwrap()
    }

    #[test]
    fn a_tile_names_its_place() {
        assert_eq!(at(""), Where::Here);
        assert_eq!(at(" auto "), Where::Here);
        assert_eq!(at("").key(), "@here");
        let Where::City { typed, ask } = at(" Berlin ") else {
            panic!()
        };
        assert_eq!(typed, "Berlin");
        assert_eq!(ask, Ask::Place(berlin()));
        assert_eq!(at("Berlin").key(), "berlin");
        assert_eq!(at("berlin, de").key(), "berlin, de");
        assert_eq!(at("Saint-Rémy").key(), "saint rémy");
        let job = Job::Forecast { place: berlin() };
        assert!(
            job.url(Units::default())
                .contains("api.open-meteo.com/v1/forecast?latitude=52.52437&")
        );
        assert!(job.url(Units::imperial()).contains("fahrenheit"));
        let air = Job::Air { place: berlin() }.url(Units::imperial());
        assert!(
            air.contains("air-quality-api") && !air.contains("fahrenheit"),
            "{air}"
        );
        let geo = Job::Geocode {
            city: "москва".into(),
            language: 1,
        };
        assert!(geo.url(Units::default()).contains("&language=uk&"));
    }

    #[test]
    fn two_tiles_of_a_place_make_one_forecast_request() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Berlin"), ("humidity", "Berlin")]);
        let jobs = p.due(0, None);
        assert_eq!(jobs, [Job::Forecast { place: berlin() }]);
        p.started(jobs[0].clone(), 1);
        assert_eq!(p.due(0, None), []);
        assert_eq!(p.due(0, Some(&at("Berlin"))), [], "never a second request");
    }

    #[test]
    fn air_only_for_air_tiles() {
        let mut p = Places::default();
        home(&mut p, &[("humidity", "Berlin")]);
        assert_eq!(kinds(&p.due(0, None)), ["forecast"]);
        home(
            &mut p,
            &[
                ("humidity", "Berlin"),
                ("air-quality", "Berlin"),
                ("pollen", "Berlin"),
            ],
        );
        let jobs = p.due(0, None);
        assert_eq!(kinds(&jobs), ["forecast", "air"]);
        assert_eq!(jobs[1], Job::Air { place: berlin() });
    }

    #[test]
    fn home_places_are_due_every_fifteen_minutes() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Berlin")]);
        let t = 1_000_000;
        let mut id = 0;
        let started = start_all(&mut p, t, &mut id);
        answer_all(&mut p, &started, t);
        assert!(p.view(&at("Berlin"), &here()).reading.is_some());
        assert_eq!(p.due(t + REFRESH_SECS - 31, None), []);
        assert_eq!(kinds(&p.due(t + REFRESH_SECS - 30, None)), ["forecast"]);
        // The user's refresh fetches at once.
        assert_eq!(kinds(&p.due(t + 1, Some(&at("berlin")))), ["forecast"]);
    }

    #[test]
    fn failures_back_off_per_place() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Berlin")]);
        let mut id = 0;
        let started = start_all(&mut p, 0, &mut id);
        let changed = p.answered(started[0].0, Err("offline".into()), 0);
        assert_eq!(changed.map(|a| a.changed), Some(true));
        home(&mut p, &[("now", "Berlin"), ("now", "Moscow")]);
        let jobs = p.due(1, None);
        let [Job::Forecast { place }] = &jobs[..] else {
            panic!("{jobs:?}")
        };
        assert_eq!(place.name, "Moscow, RU", "due at once");
        assert_eq!(p.due(59, None).len(), 1, "Berlin waits");
        assert_eq!(p.due(60, None).len(), 2);
        assert_eq!(p.due(1, Some(&at("Berlin"))).len(), 2, "unless forced");
    }

    #[test]
    fn at_most_four_requests_wait_and_the_rest_wait_their_turn() {
        let mut p = Places::default();
        home(
            &mut p,
            &[
                ("now", "1, 1"),
                ("now", "2, 2"),
                ("now", "3, 3"),
                ("now", "4, 4"),
                ("now", "5, 5"),
            ],
        );
        let jobs = p.due(0, None);
        assert_eq!(jobs.len(), MAX_IN_FLIGHT);
        for (id, job) in (1..).zip(&jobs[..3]) {
            p.started(job.clone(), id);
        }
        // The host was busy: no failure, due again on the next call.
        p.cannot_start(&jobs[3], 0, true);
        assert_eq!(p.due(0, None), [jobs[3].clone()]);
        assert_eq!(p.spots.get("4,4").map_or(0, |s| s.forecast.failures()), 0);
        // One answer frees a place for the fifth.
        p.answered(1, Ok((200, FORECAST)), 0);
        let next = p.due(0, None);
        assert_eq!(next.len(), 2);
        assert_eq!(next[0], jobs[3]);
        assert!(matches!(&next[1], Job::Forecast { place } if place.latitude == 5.));
        // Another refusal is a failure, and backs off.
        p.cannot_start(&next[1], 0, false);
        assert_eq!(p.spots["5,5"].forecast.failures(), 1);
        assert_eq!(p.due(0, None), [jobs[3].clone()]);
        assert_eq!(p.view(&at("5, 5"), &here()).status, Status::Failed);
    }

    #[test]
    fn a_place_named_two_ways_is_one_spot() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Berlin"), ("today", "berlin, de")]);
        let jobs = p.due(0, None);
        assert_eq!(kinds(&jobs), ["forecast"]);
        p.started(jobs[0].clone(), 1);
        let answered = p.answered(1, Ok((200, FORECAST)), 0).unwrap();
        let text = encode(Units::default(), &berlin(), 0, body(FORECAST));
        assert_eq!(
            answered.keep,
            [
                ("forecast:berlin".to_string(), text.clone()),
                ("forecast:berlin, de".to_string(), text),
            ]
        );
        for city in ["Berlin", "berlin, de", "Берлин"] {
            assert!(p.view(&at(city), &here()).reading.is_some(), "{city}");
        }
        // Dropping one spelling forgets its storage, not the place.
        assert_eq!(home(&mut p, &[("now", "Berlin")]), ["berlin, de"]);
        assert!(p.view(&at("Berlin"), &here()).reading.is_some());
    }

    #[test]
    fn a_dropped_place_is_forgotten_and_its_storage_removed() {
        let mut p = Places::default();
        let mut id = 0;
        home(
            &mut p,
            &[("now", "Berlin"), ("now", "Moscow"), ("uv", "Moscow")],
        );
        let started = start_all(&mut p, 0, &mut id);
        answer_all(&mut p, &started, 0);
        assert_eq!(home(&mut p, &[("now", "Berlin")]), ["moscow"], "once");
        assert_eq!(
            home(&mut p, &[("now", "Berlin")]),
            [] as [&str; 0],
            "only once"
        );
        assert_eq!(p.view(&at("Moscow"), &here()).reading, None);
        assert!(p.view(&at("Berlin"), &here()).reading.is_some());
        assert_eq!(
            kinds(&p.due(1, None)),
            [] as [&str; 0],
            "Moscow is not fetched"
        );

        // The search tile on Moscow keeps it.
        home(&mut p, &[("now", "Berlin"), ("now", "Moscow")]);
        let started = start_all(&mut p, 1, &mut id);
        answer_all(&mut p, &started, 1);
        p.set_search(Some(want("now", "Moscow")));
        p.search_rested();
        assert_eq!(home(&mut p, &[("now", "Berlin")]), [] as [&str; 0]);
        assert!(p.view(&at("Moscow"), &here()).reading.is_some());
    }

    /// A storage key goes when no tile names it by that key, even while the place itself is
    /// still wanted by the search or the location; the place's readings stay.
    #[test]
    fn a_dropped_key_goes_though_its_place_stays() {
        let mut p = Places::default();
        let mut id = 0;
        p.set_here(Some(berlin()));
        home(&mut p, &[("now", "Berlin"), ("now", "")]);
        let started = start_all(&mut p, 0, &mut id);
        answer_all(&mut p, &started, 0);
        assert_eq!(home(&mut p, &[("now", "")]), ["berlin"]);
        assert!(p.view(&Where::Here, &here()).reading.is_some());

        home(&mut p, &[("now", "Moscow")]);
        let started = start_all(&mut p, 0, &mut id);
        answer_all(&mut p, &started, 0);
        p.set_search(Some(want("now", "Moscow, RU")));
        p.search_rested();
        assert_eq!(home(&mut p, &[]), ["moscow"]);
        assert!(p.view(&at("Moscow, RU"), &here()).reading.is_some());
    }

    #[test]
    fn one_place_by_any_name_is_one_request() {
        for other in ["Берлин", "52.52437, 13.41053"] {
            let mut p = Places::default();
            home(&mut p, &[("now", "Berlin"), ("now", other)]);
            let jobs = p.due(0, None);
            assert_eq!(kinds(&jobs), ["forecast"], "{other}");
            p.started(jobs[0].clone(), 1);
            assert!(p.answered(1, Ok((200, FORECAST)), 0).is_some());
            for city in ["Berlin", other] {
                assert!(p.view(&at(city), &here()).reading.is_some(), "{city}");
            }
        }
        // The location at a listed city's coordinates.
        let mut p = Places::default();
        p.set_here(Some(Place {
            name: "Near Berlin".into(),
            ..berlin()
        }));
        home(&mut p, &[("now", ""), ("pollen", "Berlin")]);
        assert_eq!(kinds(&p.due(0, None)), ["forecast", "air"]);
    }

    #[test]
    fn a_lookup_out_keeps_its_city() {
        let mut p = Places::default();
        let mut id = 0;
        home(&mut p, &[("now", "Тула")]);
        let started = start_all(&mut p, 0, &mut id);
        p.answered(started[0].0, Ok((200, MISS)), 0);
        let started = start_all(&mut p, 0, &mut id);
        home(&mut p, &[]);
        assert_eq!(p.geo.get("тула"), Some(&Geo::Next(1)), "kept while asked");
        p.answered(started[0].0, Ok((200, MOSCOW)), 0);
        home(&mut p, &[("now", "Тула")]);
        assert_eq!(kinds(&p.due(0, None)), ["forecast"], "no second lookup");
    }

    #[test]
    fn a_dropped_location_is_forgotten_unless_the_search_names_it() {
        let mut p = Places::default();
        p.set_here(Some(berlin()));
        home(&mut p, &[("now", "")]);
        p.set_search(Some(want("now", "")));
        assert_eq!(home(&mut p, &[]), [] as [&str; 0]);
        home(&mut p, &[("now", "")]);
        p.set_search(None);
        assert_eq!(home(&mut p, &[]), ["@here"]);
        // Home's location key the search held goes when the search moves on.
        home(&mut p, &[("now", "")]);
        p.set_search(Some(want("now", "")));
        assert_eq!(home(&mut p, &[]), [] as [&str; 0]);
        assert_eq!(p.set_search(None), ["@here"]);
        // A location only searched never had a key.
        p.set_search(Some(want("now", "")));
        assert_eq!(p.set_search(None), [] as [&str; 0]);
    }

    /// A Home key the search held back goes once the search moves on, once.
    #[test]
    fn a_key_the_search_held_goes_when_the_search_moves_on() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Moscow")]);
        p.set_search(Some(want("now", "Moscow")));
        p.search_rested();
        assert_eq!(home(&mut p, &[]), [] as [&str; 0]);
        assert_eq!(p.set_search(None), ["moscow"]);
        assert_eq!(p.set_search(None), [] as [&str; 0], "once");
        // A search without a city keeps no key of the location, so none goes.
        p.set_search(Some(want("now", "")));
        assert_eq!(p.set_search(Some(want("now", "Berlin"))), [] as [&str; 0]);
        // A key Home still names stays.
        home(&mut p, &[("now", "Berlin")]);
        assert_eq!(p.set_search(None), [] as [&str; 0]);
    }

    /// The location wanted by the search alone is a searched place: its answers are not
    /// kept in storage, and its reading stays cached when the search ends.
    #[test]
    fn a_located_search_keeps_its_reading_cached_and_no_key() {
        let mut p = Places::default();
        let mut id = 0;
        p.set_here(Some(berlin()));
        p.set_search(Some(want("now", "")));
        p.search_rested();
        let started = start_all(&mut p, 0, &mut id);
        assert_eq!(started.len(), 1);
        let answered = p.answered(started[0].0, Ok((200, FORECAST)), 0).unwrap();
        assert_eq!(answered.keep, [], "no forecast:@here");
        assert_eq!(p.set_search(Some(want("now", "Berlin"))), [] as [&str; 0]);
        assert_eq!(p.set_search(None), [] as [&str; 0]);
        assert!(p.view(&Where::Here, &here()).reading.is_some(), "cached");
        // The next search of the location draws at once, with nothing to fetch.
        p.set_search(Some(want("now", "")));
        p.search_rested();
        assert_eq!(p.due(1, None), []);
    }

    /// The user's refresh of everything fetches every wanted place at once, more than fit
    /// in one turn included.
    #[test]
    fn a_forced_refresh_of_everything_fetches_every_wanted_place() {
        let mut p = Places::default();
        let mut id = 0;
        p.set_here(Some(berlin()));
        let cities: Vec<String> = (0..5).map(|i| format!("{i}.5, 1")).collect();
        let mut tiles: Vec<(&str, &str)> = cities.iter().map(|c| ("now", c.as_str())).collect();
        tiles.push(("now", ""));
        home(&mut p, &tiles);
        p.set_search(Some(want("now", "Moscow")));
        p.search_rested();
        let t = 1_000;
        while !p.due(t, None).is_empty() {
            let started = start_all(&mut p, t, &mut id);
            answer_all(&mut p, &started, t);
        }
        p.force_all();
        let mut forecasts = 0;
        for _ in 0..3 {
            let started = start_all(&mut p, t + 1, &mut id);
            assert!(started.len() <= MAX_IN_FLIGHT);
            forecasts += started.len();
            answer_all(&mut p, &started, t + 1);
        }
        assert_eq!(forecasts, 7, "five cities, the location and the search");
        assert_eq!(p.due(t + 1, None), [], "each once");
        // A busy start stays forced; a failed one does not.
        p.force_all();
        let jobs = p.due(t + 2, None);
        p.cannot_start(&jobs[0], t + 2, true);
        assert!(p.due(t + 2, None).contains(&jobs[0]));
        p.cannot_start(&jobs[0], t + 2, false);
        assert!(!p.due(t + 2, None).contains(&jobs[0]));
    }

    /// A refresh of everything while a request is out does not ask it again once it answers.
    #[test]
    fn a_forced_refresh_skips_what_is_already_asked() {
        let mut p = Places::default();
        let mut id = 0;
        home(&mut p, &[("now", "Berlin"), ("now", "Xyzzy")]);
        let started = start_all(&mut p, 0, &mut id);
        assert_eq!(started.len(), 2);
        p.force_all();
        assert_eq!(p.due(0, None), []);
        answer_all(&mut p, &started[..1], 0);
        p.answered(started[1].0, Ok((200, MISS)), 0);
        assert_eq!(p.view(&at("Xyzzy"), &here()).status, Status::Unknown);
        assert_eq!(
            p.due(1, None),
            [],
            "neither the forecast nor the lookup again"
        );
    }

    #[test]
    fn answers_say_whether_they_were_good() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Berlin"), ("now", "Тула")]);
        let mut id = 0;
        let started = start_all(&mut p, 0, &mut id);
        assert_eq!(
            kinds(&started.iter().map(|s| s.1.clone()).collect::<Vec<_>>()),
            ["forecast", "geocode"]
        );
        let good = |a: Option<Answered>| a.map(|a| a.good);
        assert_eq!(
            good(p.answered(started[0].0, Ok((200, b"<html>")), 0)),
            Some(false)
        );
        assert_eq!(
            good(p.answered(started[1].0, Ok((200, MISS)), 0)),
            Some(true),
            "a city not known is an answer"
        );
        let jobs = p.due(0, Some(&at("Berlin")));
        let forecast = jobs.iter().find(|j| kind(j) == "forecast").unwrap().clone();
        p.started(forecast.clone(), 10);
        assert_eq!(good(p.answered(10, Err("offline".into()), 0)), Some(false));
        p.started(forecast, 11);
        assert_eq!(good(p.answered(11, Ok((200, FORECAST)), 0)), Some(true));
    }

    #[test]
    fn searched_places_are_cached_but_home_is_never_trimmed() {
        let mut p = Places::default();
        let mut id = 0;
        home(&mut p, &[("now", "Berlin")]);
        let started = start_all(&mut p, 0, &mut id);
        answer_all(&mut p, &started, 0);
        for i in 0..40 {
            p.set_search(Some(want("now", &format!("{i}.5, 1"))));
            p.search_rested();
            let started = start_all(&mut p, 0, &mut id);
            assert_eq!(
                kinds(&started.iter().map(|s| s.1.clone()).collect::<Vec<_>>()),
                ["forecast"]
            );
            answer_all(&mut p, &started, 0);
        }
        assert!(p.spots.len() <= MAX_SPOTS, "{}", p.spots.len());
        assert!(p.view(&at("Berlin"), &here()).reading.is_some());
        assert!(
            p.view(&at("39.5, 1"), &here()).reading.is_some(),
            "the search's own"
        );
    }

    #[test]
    fn the_search_waits_until_the_query_rests() {
        let mut p = Places::default();
        p.set_search(Some(want("now", "Berlin")));
        assert_eq!(p.due(0, None), []);
        p.search_rested();
        assert_eq!(p.due(0, None), [Job::Forecast { place: berlin() }]);
        // A new query rests again.
        p.set_search(Some(want("now", "Moscow")));
        assert_eq!(p.due(0, None), []);
        p.set_search(None);
        p.search_rested();
        assert_eq!(p.due(0, None), []);
    }

    /// Only the search's own tile going off screen ends the search, and its coming back
    /// wants the place again as it was.
    #[test]
    fn the_search_ends_with_its_own_tile_and_comes_back_with_it() {
        let mut p = Places::default();
        let berlin_forecast = [Job::Forecast { place: berlin() }];
        // As the search answers it: the param is the place's name.
        p.set_search(Some(want("today", "Berlin, DE")));
        p.search_rested();
        // An older tile of another widget at the same place.
        assert_eq!(p.search_hidden("now", &at("Berlin, DE")), None);
        assert_eq!(p.search_hidden("today", &at("Moscow")), None);
        assert_eq!(p.due(0, None), berlin_forecast);
        // The search's tile.
        assert_eq!(
            p.search_hidden("today", &at("Berlin, DE")),
            Some(Vec::new())
        );
        assert_eq!(p.due(0, None), []);
        assert!(!p.wants_here());
        assert_eq!(p.search_shown("now", &at("Berlin, DE")), None);
        assert_eq!(p.search_shown("today", &at("Berlin, DE")), Some(true));
        assert_eq!(p.due(0, None), berlin_forecast);
        assert_eq!(p.search_shown("today", &at("Berlin, DE")), None, "once");
        // Hidden before its query rested, it comes back still waiting for the rest.
        p.set_search(Some(want("now", "")));
        assert_eq!(p.search_hidden("now", &at("")), Some(Vec::new()));
        assert!(!p.wants_here());
        assert_eq!(p.search_shown("now", &at("")), Some(false));
        assert!(p.wants_here());
        // A new query forgets the hidden one.
        p.search_hidden("now", &at(""));
        p.set_search(None);
        assert_eq!(p.search_shown("now", &at("")), None);
        assert!(!p.wants_here());
    }

    #[test]
    fn here_follows_the_location() {
        let mut p = Places::default();
        assert!(!p.wants_here());
        home(&mut p, &[("now", "")]);
        assert!(p.wants_here());
        assert_eq!(p.due(0, None), [], "nothing before a location");
        let place = Place {
            name: "Here".into(),
            latitude: 52.5,
            longitude: 13.4,
        };
        p.set_here(Some(place.clone()));
        let jobs = p.due(0, None);
        assert_eq!(jobs, [Job::Forecast { place }]);
        p.started(jobs[0].clone(), 1);
        p.answered(1, Ok((200, FORECAST)), 0);
        assert!(p.view(&Where::Here, &here()).reading.is_some());
        // A move: the new place has no reading yet, and is fetched at once.
        let moved = Place {
            name: "Elsewhere".into(),
            latitude: 48.1,
            longitude: 11.6,
        };
        p.set_here(Some(moved.clone()));
        let view = p.view(&Where::Here, &here());
        assert_eq!((view.name, view.reading), ("Elsewhere".into(), None));
        assert_eq!(
            p.due(1, None),
            [Job::Forecast {
                place: moved.clone()
            }]
        );
        // The location lost: nothing to show or fetch.
        p.set_here(None);
        let view = p.view(&Where::Here, &here());
        assert_eq!((view.name, view.reading), ("Your location".into(), None));
        assert_eq!(p.due(1, None), []);
        p.set_here(Some(moved));
        home(&mut p, &[("now", "Berlin")]);
        assert!(!p.wants_here());
        assert_eq!(p.due(0, None), [Job::Forecast { place: berlin() }]);
        // The search's tile without a city wants the location while its query rests.
        p.set_search(Some(want("now", "")));
        assert!(p.wants_here());
        assert_eq!(p.due(0, None), [Job::Forecast { place: berlin() }]);
        p.search_rested();
        assert_eq!(kinds(&p.due(0, None)), ["forecast", "forecast"]);
    }

    #[test]
    fn new_units_forget_forecasts_not_air() {
        let mut p = Places::default();
        let mut id = 0;
        home(&mut p, &[("air-quality", "Berlin")]);
        let started = start_all(&mut p, 0, &mut id);
        assert_eq!(started.len(), 2);
        answer_all(&mut p, &started, 0);
        p.set_units(Units::imperial());
        let view = p.view(&at("Berlin"), &here());
        assert_eq!(view.reading, None);
        assert!(view.air.is_some());
        let jobs = p.due(1, None);
        assert_eq!(kinds(&jobs), ["forecast"]);
        assert!(jobs[0].url(Units::imperial()).contains("fahrenheit"));
        // An answer in the old units is not taken.
        p.started(jobs[0].clone(), 50);
        p.set_units(Units::default());
        assert_eq!(p.answered(50, Ok((200, FORECAST)), 1), None);
        assert_eq!(p.view(&at("Berlin"), &here()).reading, None);
        assert_eq!(kinds(&p.due(1, None)), ["forecast"]);
    }

    #[test]
    fn a_lookup_comes_first_then_the_forecast() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Тула"), ("air-quality", "Тула")]);
        let jobs = p.due(0, None);
        assert_eq!(
            jobs,
            [Job::Geocode {
                city: "тула".into(),
                language: 0
            }]
        );
        assert!(jobs[0].url(Units::default()).contains("&language=ru&"));
        p.started(jobs[0].clone(), 1);
        assert_eq!(p.due(0, None), [], "the lookup is out");
        assert_eq!(
            p.view(&at("Тула"), &here()).name,
            "Тула",
            "as typed meanwhile"
        );
        // Not in Russian (as if): Ukrainian next, at once.
        let answered = p.answered(1, Ok((200, MISS)), 0).unwrap();
        assert!(answered.keep.is_empty());
        let jobs = p.due(0, None);
        assert_eq!(
            jobs,
            [Job::Geocode {
                city: "тула".into(),
                language: 1
            }]
        );
        p.started(jobs[0].clone(), 2);
        p.answered(2, Ok((200, MOSCOW)), 0).unwrap();
        assert_eq!(p.view(&at("Тула"), &here()).name, "Москва");
        assert_eq!(kinds(&p.due(0, None)), ["forecast", "air"]);

        // A city geocoding does not know is asked again only when the user asks.
        home(&mut p, &[("now", "Xyzzy")]);
        let jobs = p.due(0, None);
        assert!(
            jobs[0].url(Units::default()).contains("name=xyzzy&"),
            "by its key"
        );
        p.started(jobs[0].clone(), 3);
        p.answered(3, Ok((200, MISS)), 0);
        let view = p.view(&at("Xyzzy"), &here());
        assert_eq!((view.status, view.name), (Status::Unknown, "Xyzzy".into()));
        assert_eq!(p.due(10_000, None), []);
        assert_eq!(
            p.due(10_000, Some(&at("xyzzy"))),
            [Job::Geocode {
                city: "xyzzy".into(),
                language: 0
            }]
        );

        // A failed lookup backs off as a forecast does: 1, 2, 4, 8, then 15 minutes.
        home(&mut p, &[("now", "Tokyo")]);
        let mut at_ = 0;
        for (id, wait) in (10..).zip([60, 120, 240, 480, 900, 900]) {
            let jobs = p.due(at_, None);
            assert_eq!(kinds(&jobs), ["geocode"], "at {at_}");
            p.started(jobs[0].clone(), id);
            p.answered(id, Err("offline".into()), at_);
            assert_eq!(p.view(&at("Tokyo"), &here()).status, Status::Failed);
            assert_eq!(p.due(at_ + wait - 1, None), [], "waits {wait} s");
            assert_eq!(
                kinds(&p.due(at_ + 1, Some(&at("Tokyo")))),
                ["geocode"],
                "unless forced"
            );
            at_ += wait;
        }
        // A start that fails counts too.
        let jobs = p.due(at_, None);
        p.cannot_start(&jobs[0], at_, false);
        assert_eq!(p.due(at_ + 899, None), []);
        assert_eq!(kinds(&p.due(at_ + 900, None)), ["geocode"]);
    }

    #[test]
    fn answers_are_kept_for_home_places() {
        let mut p = Places::default();
        home(&mut p, &[("now", "Berlin"), ("humidity", "Berlin")]);
        let jobs = p.due(0, None);
        p.started(jobs[0].clone(), 1);
        let answered = p.answered(1, Ok((200, FORECAST)), 42).unwrap();
        assert!(answered.changed);
        let text = encode(Units::default(), &berlin(), 42, body(FORECAST));
        assert_eq!(answered.keep, [("forecast:berlin".into(), text.clone())]);
        assert!(
            text.starts_with(
                "units=c-kmh\nfetched=42\nname=Berlin, DE\nlat=52.52437\nlon=13.41053\nbody={"
            ),
            "{text}"
        );
        assert_eq!(text.lines().count(), 6);

        let mut again = Places::default();
        home(&mut again, &[("now", "Berlin")]);
        again.restore(&at("Berlin"), Kind::Forecast, &text);
        assert_eq!(
            again.view(&at("Berlin"), &here()).reading,
            p.view(&at("Berlin"), &here()).reading
        );
        assert_eq!(again.due(42 + REFRESH_SECS - 31, None), []);
        assert_eq!(
            kinds(&again.due(42 + REFRESH_SECS - 30, None)),
            ["forecast"]
        );
        // An older text never replaces a newer reading.
        let jobs = again.due(1000, None);
        again.started(jobs[0].clone(), 9);
        again.answered(9, Ok((200, FORECAST)), 1000);
        again.restore(&at("Berlin"), Kind::Forecast, &text);
        let view = again.view(&at("Berlin"), &here());
        assert_eq!(view.reading.map(|r| r.fetched), Some(1000));

        // Other units, another place, garbage: nothing.
        let mut imperial = Places::default();
        imperial.set_units(Units::imperial());
        home(&mut imperial, &[("now", "Berlin")]);
        imperial.restore(&at("Berlin"), Kind::Forecast, &text);
        assert_eq!(imperial.view(&at("Berlin"), &here()).reading, None);
        let mut other = Places::default();
        other.restore(&at("Moscow"), Kind::Forecast, &text);
        assert_eq!(other.view(&at("Moscow"), &here()).reading, None);
        other.restore(
            &at("Berlin"),
            Kind::Forecast,
            "units=c-kmh\nfetched=1\nname=B\nlat=x",
        );
        assert_eq!(other.view(&at("Berlin"), &here()).reading, None);

        // A looked-up city needs no lookup after a restart.
        let tula = Place {
            name: "Тула".into(),
            latitude: 54.2,
            longitude: 37.6,
        };
        let mut looked = Places::default();
        home(&mut looked, &[("now", "Тула")]);
        looked.restore(
            &at("Тула"),
            Kind::Forecast,
            &encode(Units::default(), &tula, 0, body(FORECAST)),
        );
        assert_eq!(looked.due(60, None), []);
        assert_eq!(looked.view(&at("Тула"), &here()).name, "Тула");

        // The location's place comes back too.
        let mut located = Places::default();
        located.restore(&Where::Here, Kind::Forecast, &text);
        assert_eq!(located.here(), Some(&berlin()));

        // Air, kept and restored whatever the units.
        let mut air = Places::default();
        home(&mut air, &[("pollen", "Berlin")]);
        let jobs = air.due(0, None);
        air.started(jobs[1].clone(), 2);
        let answered = air.answered(2, Ok((200, AIR)), 7).unwrap();
        let text = encode(Units::default(), &berlin(), 7, body(AIR));
        assert_eq!(answered.keep, [("air:berlin".into(), text.clone())]);
        let mut again = Places::default();
        again.set_units(Units::imperial());
        again.restore(&at("Berlin"), Kind::Air, &text);
        assert!(again.view(&at("Berlin"), &here()).air.is_some());

        // A place only searched is not kept.
        let mut searched = Places::default();
        searched.set_search(Some(want("now", "Moscow")));
        searched.search_rested();
        let jobs = searched.due(0, None);
        searched.started(jobs[0].clone(), 3);
        assert_eq!(
            searched.answered(3, Ok((200, FORECAST)), 0).unwrap().keep,
            []
        );
        // A key too long for storage is not kept.
        let long = "x".repeat(300);
        let mut far = Places::default();
        far.set_home(vec![Want {
            widget: "now".into(),
            at: Where::City {
                typed: long,
                ask: Ask::Place(berlin()),
            },
            air: false,
        }]);
        let jobs = far.due(0, None);
        far.started(jobs[0].clone(), 4);
        assert_eq!(far.answered(4, Ok((200, FORECAST)), 0).unwrap().keep, []);
    }

    #[test]
    fn the_location_is_kept_under_its_own_key() {
        let mut p = Places::default();
        p.set_here(Some(berlin()));
        home(&mut p, &[("now", ""), ("now", "Berlin")]);
        let jobs = p.due(0, None);
        assert_eq!(kinds(&jobs), ["forecast"]);
        p.started(jobs[0].clone(), 1);
        let keys: Vec<String> = p
            .answered(1, Ok((200, FORECAST)), 0)
            .unwrap()
            .keep
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(keys, ["forecast:@here", "forecast:berlin"]);
    }

    #[test]
    fn views_say_why() {
        let mut p = Places::default();
        // An unknown city.
        home(&mut p, &[("now", "Xyzzy")]);
        let jobs = p.due(0, None);
        p.started(jobs[0].clone(), 1);
        p.answered(1, Ok((200, MISS)), 0);
        let view = p.view(&at("Xyzzy"), &here());
        assert_eq!((view.status, view.city), (Status::Unknown, "Xyzzy".into()));
        // A failure with no reading.
        home(&mut p, &[("now", "Berlin")]);
        let jobs = p.due(0, None);
        p.started(jobs[0].clone(), 2);
        p.answered(2, Err("offline".into()), 0);
        let view = p.view(&at("Berlin"), &here());
        assert_eq!((view.status, view.reading), (Status::Failed, None));
        assert_eq!(view.name, "Berlin, DE");
        // The location, not known yet, or not to be told.
        let view = p.view(&Where::Here, &here());
        assert_eq!(
            (view.status, view.name),
            (Status::Ok, "Your location".into())
        );
        let mut nowhere = Location::default();
        nowhere.located(Err("location services are off".into()));
        let view = p.view(&Where::Here, &nowhere);
        assert_eq!(
            (view.status, view.name),
            (Status::NoLocation, "Your location".into())
        );
        // The location's forecast failed.
        p.set_here(Some(berlin()));
        assert_eq!(p.view(&Where::Here, &here()).status, Status::Failed);
        // The air failed, the forecast came.
        let mut air = Places::default();
        home(&mut air, &[("air-quality", "Moscow")]);
        let jobs = air.due(0, None);
        air.started(jobs[0].clone(), 1);
        air.started(jobs[1].clone(), 2);
        air.answered(1, Ok((200, FORECAST)), 0);
        air.answered(2, Ok((503, b"")), 0);
        let view = air.view(&at("Moscow"), &here());
        assert_eq!((view.status, view.air_status), (Status::Ok, Status::Failed));
        assert!(view.reading.is_some() && view.air.is_none());
    }
}
