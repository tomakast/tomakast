//! The built-in list of cities (`cities.tsv`, from GeoNames, CC BY 4.0; see
//! `tools/cities.py`): about 6,300 places of at least 100,000 people, each under its name,
//! its ASCII form, its Cyrillic names and, for cities of a million or more, its other Latin
//! names. The weather plugin shows the weather of a city typed in the search without anything
//! leaving the computer first; the launcher finds the city the user set as their location.
//!
//! A query matches a name by [`key`]: lowercase, `ё` as `е`, a hyphen as a space, one space
//! between words. A key that is the main name of a city means that city, before any city
//! that has it as another name; otherwise the most populous city with that name wins.

use std::collections::HashMap;

use crate::Place;

const DATA: &str = include_str!("cities.tsv");

/// Within this distance, [`Cities::nearest`] names the biggest city, not the nearest one.
pub const AROUND_KM: f64 = 20.;

/// What a query is matched on.
pub fn key(name: &str) -> String {
    name.to_lowercase()
        .replace('ё', "е")
        .replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy)]
struct City {
    name: &'static str,
    population: u32,
    latitude: f64,
    longitude: f64,
    country: &'static str,
}

/// The cities by name key.
pub struct Cities {
    cities: Vec<City>,
    /// The city and the name as the list spells it.
    by_key: HashMap<String, (u32, &'static str)>,
}

impl Cities {
    /// The built-in list.
    pub fn builtin() -> Cities {
        Cities::parse(DATA)
    }

    /// Reads `data` in the format of `cities.tsv`; lines it cannot read are skipped.
    pub fn parse(data: &'static str) -> Cities {
        let mut cities = Vec::new();
        let mut names: Vec<(u32, &'static str)> = Vec::new();
        let mut alternates: Vec<(u32, &'static str)> = Vec::new();
        for line in data.lines().filter(|l| !l.starts_with('#')) {
            let mut cols = line.split('\t');
            let (Some(lat), Some(lon), Some(country), Some(population), Some(all)) = (
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
            ) else {
                continue;
            };
            let (Ok(latitude), Ok(longitude)) = (lat.parse(), lon.parse()) else {
                continue;
            };
            let ix = cities.len() as u32;
            let mut all = all.split('|');
            let Some(name) = all.next() else {
                continue;
            };
            cities.push(City {
                name,
                population: population.parse().unwrap_or(0),
                latitude,
                longitude,
                country,
            });
            names.push((ix, name));
            alternates.extend(all.map(|name| (ix, name)));
        }
        let mut by_key = HashMap::with_capacity(names.len() + alternates.len());
        // Main names first, then the others; within each, the list's order: by population.
        for (ix, name) in names.into_iter().chain(alternates) {
            by_key.entry(key(name)).or_insert((ix, name));
        }
        Cities { cities, by_key }
    }

    /// The city `text` names, as the list spells that name, with its country.
    pub fn find(&self, text: &str) -> Option<Place> {
        let (ix, name) = *self.by_key.get(&key(text))?;
        let city = self.cities[ix as usize];
        Some(Place {
            name: format!("{name}, {}", city.country),
            latitude: city.latitude,
            longitude: city.longitude,
        })
    }

    /// What to call the place at `latitude`, `longitude`, as `Name, CC`: the most populous
    /// city within [`AROUND_KM`] (a city's districts are on the list too, and a position
    /// rounded to 0.1° may fall in one), else the nearest within `max_km`.
    pub fn nearest(&self, latitude: f64, longitude: f64, max_km: f64) -> Option<String> {
        let near: Vec<(f64, &City)> = self
            .cities
            .iter()
            .map(|c| (km(latitude, longitude, c.latitude, c.longitude), c))
            .filter(|(d, _)| *d <= max_km)
            .collect();
        let around = near
            .iter()
            .filter(|(d, _)| *d <= AROUND_KM)
            .max_by_key(|(_, c)| c.population);
        around
            .or_else(|| near.iter().min_by(|a, b| a.0.total_cmp(&b.0)))
            .map(|(_, c)| format!("{}, {}", c.name, c.country))
    }

    /// The main name of every city, in the list's order (by population, so the first of
    /// two cities with one name is the one [`find`](Self::find) names). A name may repeat.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.cities.iter().map(|c| c.name)
    }

    pub fn len(&self) -> usize {
        self.cities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cities.is_empty()
    }
}

/// The great-circle distance between two places, in km.
fn km(lat_a: f64, lon_a: f64, lat_b: f64, lon_b: f64) -> f64 {
    let (a, b) = (lat_a.to_radians(), lat_b.to_radians());
    let dlat = b - a;
    let dlon = (lon_b - lon_a).to_radians();
    let h = (dlat / 2.).sin().powi(2) + a.cos() * b.cos() * (dlon / 2.).sin().powi(2);
    2. * 6371. * h.sqrt().min(1.).asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_case_yo_hyphens_and_spaces() {
        assert_eq!(key("  Санкт-Петербург "), "санкт петербург");
        assert_eq!(key("Орёл"), "орел");
        assert_eq!(key("New   York"), "new york");
    }

    #[test]
    fn the_builtin_list_finds_cities_in_english_russian_and_their_own_names() {
        let cities = Cities::builtin();
        assert!(cities.len() > 6000, "{}", cities.len());
        let moscow = cities.find("moscow").unwrap();
        assert_eq!(moscow.name, "Moscow, RU");
        assert_eq!((moscow.latitude, moscow.longitude), (55.75204, 37.61781));
        assert_eq!(cities.find("Москва").unwrap().name, "Москва, RU");
        assert_eq!(cities.find("München").unwrap().name, "München, DE");
        assert_eq!(cities.find("munich").unwrap().name, "Munich, DE");
        assert_eq!(cities.find("санкт петербург").unwrap().latitude, 59.93863);
        assert_eq!(cities.find("Київ").unwrap().name, "Київ, UA");
        assert_eq!(cities.find("paris").unwrap().name, "Paris, FR");
        for not in [
            "",
            "fire",
            "settings",
            "calculator",
            "SHA",
            "заметки",
            "Berl",
        ] {
            assert_eq!(cities.find(not), None, "{not:?}");
        }
    }

    #[test]
    fn the_nearest_city_is_named_within_the_distance() {
        let cities = Cities::builtin();
        assert_eq!(cities.nearest(52.5, 13.4, 50.).unwrap(), "Berlin, DE");
        assert_eq!(cities.nearest(55.8, 37.6, 50.).unwrap(), "Moscow, RU");
        // The middle of the Atlantic.
        assert_eq!(cities.nearest(30., -40., 50.), None);
        assert!((km(52.52, 13.41, 55.75, 37.62) - 1609.).abs() < 5.);
    }

    #[test]
    fn a_main_name_wins_over_another_name_then_the_population() {
        let cities = Cities::parse(
            "# comment\n\
             1\t2\tAA\t900\tBigtown|Smallville\n\
             3\t4\tBB\t500\tSmallville\n\
             5\t6\tCC\t400\tBigtown\n\
             bad line\n",
        );
        assert_eq!(cities.len(), 3);
        assert_eq!(cities.find("smallville").unwrap().name, "Smallville, BB");
        assert_eq!(cities.find("bigtown").unwrap().name, "Bigtown, AA");
        assert_eq!(
            cities.names().collect::<Vec<_>>(),
            ["Bigtown", "Smallville", "Bigtown"]
        );
    }

    /// Building the map runs once, at `init`, which has 2 s. Natively it takes a few ms.
    #[test]
    fn the_list_builds_quickly() {
        let started = std::time::Instant::now();
        let cities = Cities::builtin();
        assert!(!cities.is_empty());
        let took = started.elapsed();
        assert!(
            took < std::time::Duration::from_millis(if cfg!(debug_assertions) {
                2000
            } else {
                200
            }),
            "{took:?}"
        );
    }
}
