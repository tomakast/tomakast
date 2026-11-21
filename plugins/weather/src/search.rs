//! What a query typed in the launcher search asks for, and the place a tile's param `city`
//! names: pure, so it is tested natively; `lib.rs` answers the search and
//! [`places`](crate::places) fetches.
//!
//! - [`read`] reads a query for any of the ten widgets: a word for each, in English and
//!   Russian (`wind in Berlin`, `berlin wind`, `ветер в Берлин`, `uv index`, `weather today`,
//!   `погода в Урюпинске`), with or without a city. Without one the tile shows where the
//!   computer is; a bare city of the built-in list is its `now`. A city is found in the
//!   built-in list, read as `latitude, longitude`, or else geocoded by Open-Meteo, so what
//!   else the user types is never sent anywhere. A word still being typed (`uv ind` of `uv
//!   index`, `weather tod` of `weather today`) asks for nothing, so it is never looked up as
//!   a city; after a connector (`weather in Toda`) it is the city.
//! - The tile's param `city` names what it shows ([`Ask::param`]), so a render, an action or
//!   an answer that comes after more typing finds its own place, and a Home tile finds it
//!   again from `config.toml` ([`city`]), where the user may also type a city by hand. A city
//!   is kept as typed (trimmed); [`cities::key`] is only what it is matched and looked up by.

use crate::api::{self, Place};
use tomakast_places::cities::{self, Cities};

/// What a query that ends in it has not named the city yet.
const CONNECTORS: &[&str] = &["in", "at", "for", "в", "во"];

/// The search words of each widget (its manifest id), compared on [`cities::key`]s.
const WORDS: &[(&str, &[&str])] = &[
    ("now", &["weather", "погода"]),
    ("humidity", &["humidity", "влажность"]),
    ("wind", &["wind", "ветер"]),
    ("uv", &["uv index", "uv", "уф", "ультрафиолет"]),
    ("pressure", &["pressure", "давление"]),
    (
        "precipitation",
        &["precipitation", "rain", "осадки", "дождь"],
    ),
    ("air-quality", &["air quality", "aqi", "качество воздуха"]),
    ("pollen", &["pollen", "пыльца"]),
    (
        "daylight",
        &[
            "daylight",
            "sunrise",
            "sunset",
            "световой день",
            "восход",
            "закат",
        ],
    ),
];
/// Ends a weather query to ask for the `today` widget.
const TODAY: &[&str] = &["today", "сегодня"];

/// What a query asks for: a widget and the city it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// The widget's manifest id.
    pub widget: &'static str,
    /// `None`: where the computer is.
    pub city: Option<Ask>,
}

/// What a query asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// A known place.
    Place(Place),
    /// A city to geocode, as typed (trimmed); looked up by its [`cities::key`].
    Lookup(String),
}

/// What `query` asks for, if anything (see the module doc). A bare city of the built-in
/// list is its `now`; `today` / `сегодня` at the end of a weather query makes it `today`.
pub fn read(query: &str, cities: &Cities) -> Option<Query> {
    let query = query.trim();
    let weather =
        |rest: &str| words(rest, cities).filter(|q| q.as_ref().is_none_or(|q| q.widget == "now"));
    if let Some(rest) = TODAY.iter().find_map(|t| before_words(query, t)) {
        // `weather today`; `wind today` is a wind query for a city named Today.
        if let Some(read) = weather(rest) {
            return read.map(|q| Query {
                widget: "today",
                ..q
            });
        }
    }
    // Still typing `today` after a weather query (`weather tod`): not a city named `Tod`.
    if let Some((rest, last)) = query.rsplit_once(char::is_whitespace) {
        let last = cities::key(last);
        let typing = TODAY
            .iter()
            .any(|t| t.starts_with(&last) && *t != last && last.chars().count() >= 2);
        if typing && matches!(weather(rest.trim()), Some(Some(_))) {
            return None;
        }
    }
    match words(query, cities) {
        Some(read) => read,
        None => cities.find(query).map(|place| Query {
            widget: "now",
            city: Some(Ask::Place(place)),
        }),
    }
}

/// A query with a search word: `None` when it has none, `Some(None)` while its city is
/// still being typed.
fn words(query: &str, cities: &Cities) -> Option<Option<Query>> {
    // The longest word first, so `uv index` is read before `uv`.
    let mut all: Vec<(&'static str, &str)> = WORDS
        .iter()
        .flat_map(|(widget, words)| words.iter().map(move |w| (*widget, *w)))
        .collect();
    all.sort_by_key(|(_, w)| std::cmp::Reverse(w.chars().count()));
    // Still typing a word of more than one: `uv ind` of `uv index`, not `uv` of `Ind`.
    let key = cities::key(query);
    if key.contains(' ') && all.iter().any(|(_, w)| w.starts_with(&key) && *w != key) {
        return Some(None);
    }
    all.into_iter().find_map(|(widget, word)| {
        let named = if key == word {
            return Some(Some(Query { widget, city: None }));
        } else if let Some(rest) = after_words(query, word) {
            // Past a connector: `wind in Berlin`.
            let key = cities::key(rest);
            if CONNECTORS.contains(&key.as_str()) {
                return Some(None);
            }
            match rest.split_once(char::is_whitespace) {
                Some((first, after)) if CONNECTORS.contains(&cities::key(first).as_str()) => {
                    after.trim()
                }
                _ => rest,
            }
        } else {
            before_words(query, word)?
        };
        // Still typing: one letter of the city.
        if cities::key(named).chars().count() < 2 {
            return Some(None);
        }
        Some(city(named, cities).map(|city| Query {
            widget,
            city: Some(city),
        }))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cities() -> Cities {
        Cities::parse(
            "55.75204\t37.61781\tRU\t10381222\tMoscow|Москва\n\
             52.52437\t13.41053\tDE\t3426354\tBerlin|Берлин\n",
        )
    }

    /// The place of the built-in list `name` names.
    fn place(name: &str) -> Ask {
        match city(name, &cities()) {
            Some(Ask::Place(p)) => Ask::Place(p),
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn a_bare_query_asks_only_for_a_listed_city() {
        let Some(Query {
            widget: "now",
            city: Some(Ask::Place(berlin)),
        }) = read("berlin", &cities())
        else {
            panic!()
        };
        assert_eq!(berlin.name, "Berlin, DE");
        assert_eq!(read_as("  москва "), Some(("now", Some(place("москва")))));
        assert_eq!(place("москва").param(), "Москва, RU");
        for not in ["", "ber", "Tokyo", "52.5, 13.4"] {
            assert_eq!(read_as(not), None, "{not:?}");
        }
    }

    #[test]
    fn weather_in_a_city_looks_it_up_unless_it_is_listed() {
        for query in [
            "weather in Berlin",
            "Weather berlin",
            "berlin weather",
            "погода в Берлин",
        ] {
            assert!(
                matches!(read_as(query), Some(("now", Some(Ask::Place(_))))),
                "{query}"
            );
        }
        // A city looked up is kept as typed.
        for (query, typed) in [
            ("weather in Tokyo", "Tokyo"),
            ("Weather in  Saint-Rémy ", "Saint-Rémy"),
            ("Saint-Rémy  Weather", "Saint-Rémy"),
            ("погода в Урюпинске", "Урюпинске"),
            ("погода Тула", "Тула"),
        ] {
            assert_eq!(
                read_as(query),
                Some(("now", Some(Ask::Lookup(typed.into())))),
                "{query}"
            );
        }
        let Some(("now", Some(Ask::Place(at)))) = read_as("weather at 1.5, 2.5") else {
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
            assert_eq!(read_as(typing), None, "{typing:?}");
        }
    }

    #[test]
    fn params_round_trip() {
        let cities = cities();
        let at = city("1.5, 2.5", &cities).unwrap();
        let with_country = city("Berlin, DE", &cities).unwrap();
        assert_eq!(with_country, place("Berlin"));
        let elsewhere = city("Berlin, US", &cities).unwrap();
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

    fn read_as(query: &str) -> Option<(&'static str, Option<Ask>)> {
        read(query, &cities()).map(|q| (q.widget, q.city))
    }

    #[test]
    fn every_word_alone_is_the_location() {
        for (widget, words) in WORDS {
            for word in *words {
                for typed in [word.to_string(), word.to_uppercase()] {
                    assert_eq!(read_as(&typed), Some((*widget, None)), "{typed}");
                }
            }
        }
        assert_eq!(read_as("uv index"), Some(("uv", None)));
        assert_eq!(read_as("Погода"), Some(("now", None)));
    }

    #[test]
    fn words_with_a_city() {
        let berlin = Some(place("Berlin"));
        for query in ["wind in Berlin", "berlin wind", "wind Berlin"] {
            assert_eq!(read_as(query), Some(("wind", berlin.clone())), "{query}");
        }
        // The same place, named as typed.
        assert_eq!(
            read_as("ветер в Берлин"),
            Some(("wind", Some(place("берлин"))))
        );
        assert_eq!(
            read_as("air quality in Tokyo"),
            Some(("air-quality", Some(Ask::Lookup("Tokyo".into()))))
        );
        assert_eq!(read_as("uv index berlin"), Some(("uv", berlin)));
        assert_eq!(
            read_as("качество воздуха в Москва"),
            Some(("air-quality", Some(place("Москва"))))
        );
    }

    #[test]
    fn today_at_the_end() {
        for query in ["weather today", "погода сегодня"] {
            assert_eq!(read_as(query), Some(("today", None)), "{query}");
        }
        for query in ["weather in Berlin today", "berlin weather today"] {
            assert_eq!(
                read_as(query),
                Some(("today", Some(place("Berlin")))),
                "{query}"
            );
        }
        assert_eq!(
            read_as("погода в Берлин сегодня"),
            Some(("today", Some(place("берлин"))))
        );
        // Only a weather query takes it.
        assert_eq!(
            read_as("wind today"),
            Some(("wind", Some(Ask::Lookup("today".into()))))
        );
        assert_eq!(read_as("today"), None);
    }

    #[test]
    fn typing_a_word_shows_nothing_until_it_names_a_city() {
        for typing in [
            "wind i",
            "wind in",
            "погода в",
            "air quality in ",
            "uv index in b",
            "air",
            // The rest of a longer word, not a city.
            "uv ind",
            "uv inde",
            "UV  Inde",
            "air qual",
            "качество возд",
            "световой д",
            // `today` being typed after a weather query.
            "weather to",
            "weather tod",
            "Weather Toda",
            "погода се",
            "погода сегодн",
            "weather in Berlin tod",
            "berlin weather to",
        ] {
            assert_eq!(read_as(typing), None, "{typing:?}");
        }
        // After a connector it is the city: `weather in <city>` names any city.
        assert_eq!(
            read_as("weather in Toda"),
            Some(("now", Some(Ask::Lookup("Toda".into()))))
        );
        // Only a weather query takes `today`.
        assert_eq!(
            read_as("wind to"),
            Some(("wind", Some(Ask::Lookup("to".into()))))
        );
    }

    #[test]
    fn a_bare_city_is_now() {
        assert_eq!(read_as("berlin"), Some(("now", Some(place("Berlin")))));
        assert_eq!(read_as("tokyo"), None);
    }
}
