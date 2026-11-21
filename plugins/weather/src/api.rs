//! Open-Meteo, as the plugin uses it: the URLs it asks and what it reads from the answers.
//! Pure, so it is tested natively against answers saved in `tests/fixtures`.
//!
//! - Geocoding (`geocoding-api.open-meteo.com/v1/search`) turns a city name into a place. Its
//!   `language` decides which names it matches: `Москва` is found with `ru` only. The plugin
//!   tries the [`languages`] the name's script suggests, in turn. An answer without
//!   `results` is a city it does not know.
//! - The forecast (`api.open-meteo.com/v1/forecast`) gives the current weather and today's
//!   high and low at a latitude and longitude, in the place's own time zone.

use crate::json::{self, Json};

/// The longest answer body the plugin reads; longer ones are refused unread. The answers it
/// asks for are under 1 KiB.
pub const MAX_ANSWER_BYTES: usize = 16 * 1024;

/// Celsius and km/h, or Fahrenheit and mph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

/// Where the weather is asked for; its name is what the tile calls it.
pub use tomakast_places::Place;

/// What the plugin shows: the current weather and today's range at a place.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub temperature: f64,
    pub feels_like: Option<f64>,
    pub humidity: Option<f64>,
    pub wind: Option<f64>,
    /// The WMO weather interpretation code.
    pub code: u16,
    pub is_day: bool,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub units: Units,
    /// When the plugin received it, in unix seconds.
    pub fetched: i64,
}

/// A place typed as `latitude, longitude` (or with a space between): two finite numbers in
/// range. `None` for anything else, which is then a city name.
pub fn coordinates(text: &str) -> Option<Place> {
    let (lat, lon) = text
        .split_once(',')
        .or_else(|| text.trim().split_once(char::is_whitespace))?;
    let (lat, lon): (f64, f64) = (lat.trim().parse().ok()?, lon.trim().parse().ok()?);
    let ok = lat.is_finite() && lon.is_finite() && lat.abs() <= 90. && lon.abs() <= 180.;
    ok.then(|| Place {
        name: format!("{lat}, {lon}"),
        latitude: lat,
        longitude: lon,
    })
}

/// The geocoding languages to try for `city`, in order: Russian then Ukrainian for a name in
/// Cyrillic, English for any other.
pub fn languages(city: &str) -> &'static [&'static str] {
    if city.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c)) {
        &["ru", "uk"]
    } else {
        &["en"]
    }
}

/// The geocoding search for `city` in `language`: the best match only.
pub fn geocoding_url(city: &str, language: &str) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language={language}&format=json",
        percent_encode(city.trim())
    )
}

/// The forecast at `place` in `units`: the current temperature, feel, humidity, weather
/// code, day or night and wind, and today's high and low.
pub fn forecast_url(place: &Place, units: Units) -> String {
    let mut url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m\
         &daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=1",
        place.latitude, place.longitude
    );
    if units == Units::Imperial {
        url.push_str("&temperature_unit=fahrenheit&wind_speed_unit=mph");
    }
    url
}

/// `text` for a URL query value: unreserved ASCII as it is, every other byte of its UTF-8
/// as `%XX`.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// What a geocoding answer says.
#[derive(Debug, Clone, PartialEq)]
pub enum Geocoded {
    Found(Place),
    /// A valid answer with no match.
    Unknown,
}

/// A geocoding answer: the first result, or [`Geocoded::Unknown`] for a valid answer
/// without `results`. `None` for anything else: a status other than 200, a body that is no
/// JSON or too long, a result without a name or coordinates.
pub fn parse_geocoding(status: u16, body: &[u8]) -> Option<Geocoded> {
    let json = answer_json(status, body)?;
    if !matches!(json, Json::Obj(_)) {
        return None;
    }
    let first = match json.get("results") {
        None => return Some(Geocoded::Unknown),
        Some(Json::Arr(results)) => match results.first() {
            None => return Some(Geocoded::Unknown),
            Some(first) => first,
        },
        Some(_) => return None,
    };
    let Some(Json::Str(name)) = first.get("name") else {
        return None;
    };
    let place = Place {
        name: name.clone(),
        latitude: number(first.get("latitude")?)?,
        longitude: number(first.get("longitude")?)?,
    };
    (place.latitude.abs() <= 90. && place.longitude.abs() <= 180.).then_some(Geocoded::Found(place))
}

/// A forecast answer in `units`, received at `now`: `None` unless the status is 200 and the
/// body holds at least the current temperature and weather code.
pub fn parse_forecast(status: u16, body: &[u8], units: Units, now: i64) -> Option<Reading> {
    let json = answer_json(status, body)?;
    let current = json.get("current")?;
    let field = |name| current.get(name).and_then(number);
    let code = field("weather_code")?;
    if !(0. ..=99.).contains(&code) || code.fract() != 0. {
        return None;
    }
    let today = |name| match json.get("daily")?.get(name)? {
        Json::Arr(values) => number(values.first()?),
        _ => None,
    };
    Some(Reading {
        temperature: field("temperature_2m")?,
        feels_like: field("apparent_temperature"),
        humidity: field("relative_humidity_2m"),
        wind: field("wind_speed_10m"),
        code: code as u16,
        is_day: field("is_day") != Some(0.),
        high: today("temperature_2m_max"),
        low: today("temperature_2m_min"),
        units,
        fetched: now,
    })
}

/// The JSON of an answer with status 200 and at most [`MAX_ANSWER_BYTES`] of body.
fn answer_json(status: u16, body: &[u8]) -> Option<Json> {
    if status != 200 || body.len() > MAX_ANSWER_BYTES {
        return None;
    }
    json::parse(std::str::from_utf8(body).ok()?)
}

/// A finite number.
fn number(value: &Json) -> Option<f64> {
    match value {
        Json::Num(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin.json");
    const FORECAST_F: &[u8] = include_bytes!("../tests/fixtures/forecast-moscow-fahrenheit.json");
    const FORECAST_ERROR: &[u8] = include_bytes!("../tests/fixtures/forecast-error-400.json");
    const BERLIN: &[u8] = include_bytes!("../tests/fixtures/geocoding-berlin.json");
    const MOSCOW: &[u8] = include_bytes!("../tests/fixtures/geocoding-moscow-ru.json");
    const MISS: &[u8] = include_bytes!("../tests/fixtures/geocoding-miss.json");

    #[test]
    fn coordinates_are_two_numbers_in_range() {
        let place = coordinates("52.52, 13.41").unwrap();
        assert_eq!((place.latitude, place.longitude), (52.52, 13.41));
        assert_eq!(place.name, "52.52, 13.41");
        assert_eq!(coordinates(" -33.9 18.4 ").unwrap().longitude, 18.4);
        for not in [
            "Berlin", "52.52", "91, 0", "0, 181", "NaN, 1", "1, inf", "", "a, b",
        ] {
            assert_eq!(coordinates(not), None, "{not:?}");
        }
    }

    #[test]
    fn cyrillic_names_are_looked_up_in_russian_then_ukrainian() {
        assert_eq!(languages("Москва"), ["ru", "uk"]);
        assert_eq!(languages("Київ"), ["ru", "uk"]);
        assert_eq!(languages("München"), ["en"]);
        assert_eq!(languages("東京"), ["en"]);
    }

    #[test]
    fn urls_encode_the_name_and_ask_for_the_units() {
        assert_eq!(
            geocoding_url(" New York ", "en"),
            "https://geocoding-api.open-meteo.com/v1/search?name=New%20York&count=1&language=en&format=json"
        );
        assert!(
            geocoding_url("Москва", "ru").contains("name=%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0&")
        );
        assert_eq!(percent_encode("a&b=c/d?é"), "a%26b%3Dc%2Fd%3F%C3%A9");
        let place = Place {
            name: "Berlin".into(),
            latitude: 52.52437,
            longitude: 13.41053,
        };
        let metric = forecast_url(&place, Units::Metric);
        assert!(
            metric.starts_with(
                "https://api.open-meteo.com/v1/forecast?latitude=52.52437&longitude=13.41053&current="
            ),
            "{metric}"
        );
        assert!(!metric.contains("fahrenheit"), "{metric}");
        assert!(
            forecast_url(&place, Units::Imperial)
                .ends_with("&temperature_unit=fahrenheit&wind_speed_unit=mph")
        );
    }

    #[test]
    fn geocoding_finds_the_first_result_or_knows_no_city() {
        let berlin = Place {
            name: "Berlin".into(),
            latitude: 52.52437,
            longitude: 13.41053,
        };
        assert_eq!(parse_geocoding(200, BERLIN), Some(Geocoded::Found(berlin)));
        let Some(Geocoded::Found(moscow)) = parse_geocoding(200, MOSCOW) else {
            panic!("Москва found");
        };
        assert_eq!(moscow.name, "Москва");
        assert_eq!(parse_geocoding(200, MISS), Some(Geocoded::Unknown));
        assert_eq!(
            parse_geocoding(200, br#"{"results":[]}"#),
            Some(Geocoded::Unknown)
        );
    }

    #[test]
    fn a_bad_geocoding_answer_is_none() {
        assert_eq!(parse_geocoding(500, BERLIN), None);
        assert_eq!(parse_geocoding(200, b"<html>"), None);
        assert_eq!(parse_geocoding(200, b"[1]"), None);
        assert_eq!(parse_geocoding(200, br#"{"results":[{"name":"X"}]}"#), None);
        assert_eq!(
            parse_geocoding(
                200,
                br#"{"results":[{"name":"X","latitude":120,"longitude":0}]}"#
            ),
            None
        );
        let long = format!(r#"{{"x":"{}"}}"#, "a".repeat(MAX_ANSWER_BYTES));
        assert_eq!(parse_geocoding(200, long.as_bytes()), None);
    }

    #[test]
    fn a_forecast_gives_now_and_todays_range() {
        let reading = parse_forecast(200, FORECAST, Units::Metric, 1_000).unwrap();
        assert_eq!(
            reading,
            Reading {
                temperature: 2.6,
                feels_like: Some(-0.5),
                humidity: Some(81.),
                wind: Some(5.9),
                code: 0,
                is_day: false,
                high: Some(5.4),
                low: Some(1.6),
                units: Units::Metric,
                fetched: 1_000,
            }
        );
        let f = parse_forecast(200, FORECAST_F, Units::Imperial, 0).unwrap();
        assert_eq!((f.temperature, f.code, f.units), (40.4, 3, Units::Imperial));
    }

    #[test]
    fn a_bad_forecast_is_none() {
        assert_eq!(parse_forecast(400, FORECAST_ERROR, Units::Metric, 0), None);
        assert_eq!(parse_forecast(200, FORECAST_ERROR, Units::Metric, 0), None);
        assert_eq!(parse_forecast(503, FORECAST, Units::Metric, 0), None);
        assert_eq!(parse_forecast(200, b"", Units::Metric, 0), None);
        let no_code = br#"{"current":{"temperature_2m":1}}"#;
        assert_eq!(parse_forecast(200, no_code, Units::Metric, 0), None);
        let odd_code = br#"{"current":{"temperature_2m":1,"weather_code":2.5}}"#;
        assert_eq!(parse_forecast(200, odd_code, Units::Metric, 0), None);
        // Only the current weather: the rest is unknown, not an error.
        let bare = br#"{"current":{"temperature_2m":-3,"weather_code":71}}"#;
        let reading = parse_forecast(200, bare, Units::Metric, 0).unwrap();
        assert_eq!(
            (reading.high, reading.feels_like, reading.is_day),
            (None, None, true)
        );
    }
}
