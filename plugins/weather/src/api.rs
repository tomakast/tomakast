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
/// asks for are under 6 KiB.
pub const MAX_ANSWER_BYTES: usize = 32 * 1024;

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
    /// The place's own time of the answer (`current.time`), in unix seconds; `fetched` when
    /// the answer has none.
    pub current_time: i64,
    /// The place's offset from UTC, in seconds.
    pub utc_offset: i64,
    /// The hours the answer holds, oldest first; none in an answer without them.
    pub hours: Vec<Hour>,
    /// The days the answer holds, oldest first.
    pub days: Vec<Day>,
}

/// One hour of the forecast; a value the answer lacks is `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hour {
    /// The start of the hour, in unix seconds.
    pub at: i64,
    pub temperature: Option<f64>,
    pub feels_like: Option<f64>,
    pub humidity: Option<f64>,
    pub dew_point: Option<f64>,
    /// The chance of rain, in percent.
    pub rain_chance: Option<f64>,
    /// Rain and snow melted, in mm.
    pub precipitation: Option<f64>,
    /// In cm.
    pub snowfall: Option<f64>,
    /// At sea level, in hPa.
    pub pressure: Option<f64>,
    pub wind: Option<f64>,
    /// The degrees the wind blows from.
    pub wind_direction: Option<f64>,
    pub gusts: Option<f64>,
    pub uv: Option<f64>,
    /// The WMO weather interpretation code.
    pub code: Option<u16>,
}

/// One day of the forecast.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Day {
    /// Local midnight of the day, in unix seconds; 0 when the answer does not give it as one.
    pub date: i64,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub sunrise: Option<i64>,
    pub sunset: Option<i64>,
    /// In seconds.
    pub daylight: Option<f64>,
}

/// The pollens an air answer reports, in the order of [`AirHour::pollen`].
pub const POLLENS: [&str; 6] = ["alder", "birch", "grass", "mugwort", "olive", "ragweed"];

/// One hour of air quality; a value the answer lacks is `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AirHour {
    /// The start of the hour, in unix seconds.
    pub at: i64,
    /// The European air quality index, overall and by pollutant.
    pub aqi: Option<f64>,
    pub pm2_5: Option<f64>,
    pub pm10: Option<f64>,
    pub no2: Option<f64>,
    pub o3: Option<f64>,
    pub so2: Option<f64>,
    /// Grains per cubic metre, in [`POLLENS`] order.
    pub pollen: [Option<f64>; 6],
}

/// An air quality answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Air {
    pub hours: Vec<AirHour>,
    /// The place's offset from UTC, in seconds.
    pub utc_offset: i64,
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

/// The forecast at `place` in `units`: the current weather, the hours from 24 back to 26
/// ahead, and the days from yesterday to six ahead, in the place's own time zone, with times
/// as unix seconds.
pub fn forecast_url(place: &Place, units: Units) -> String {
    let mut url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m\
         &hourly=temperature_2m,apparent_temperature,relative_humidity_2m,dew_point_2m,precipitation_probability,precipitation,snowfall,weather_code,pressure_msl,wind_speed_10m,wind_direction_10m,wind_gusts_10m,uv_index\
         &daily=temperature_2m_max,temperature_2m_min,sunrise,sunset,daylight_duration\
         &timezone=auto&timeformat=unixtime&past_hours=24&forecast_hours=26&past_days=1&forecast_days=7",
        place.latitude, place.longitude
    );
    if units == Units::Imperial {
        url.push_str("&temperature_unit=fahrenheit&wind_speed_unit=mph");
    }
    url
}

/// The air quality at `place`: the European index, overall and by pollutant, and the pollens,
/// for the hours from 24 back to 26 ahead.
pub fn air_url(place: &Place) -> String {
    format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={}&longitude={}\
         &hourly=european_aqi,european_aqi_pm2_5,european_aqi_pm10,european_aqi_nitrogen_dioxide,european_aqi_ozone,european_aqi_sulphur_dioxide,alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,olive_pollen,ragweed_pollen\
         &timezone=auto&timeformat=unixtime&past_hours=24&forecast_hours=26",
        place.latitude, place.longitude
    )
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
/// body holds at least the current temperature and weather code. Hours and days the answer
/// lacks are empty; a value in them that is no number is unknown.
pub fn parse_forecast(status: u16, body: &[u8], units: Units, now: i64) -> Option<Reading> {
    let json = answer_json(status, body)?;
    let current = json.get("current")?;
    let field = |name| current.get(name).and_then(number);
    let code = field("weather_code")?;
    if !(0. ..=99.).contains(&code) || code.fract() != 0. {
        return None;
    }
    let current_time = field("time").map_or(now, |t| t as i64);
    let utc_offset = json
        .get("utc_offset_seconds")
        .and_then(number)
        .map_or(0, |o| o as i64);
    let hours = json.get("hourly").map_or_else(Vec::new, |hourly| {
        let column = |name| numbers(hourly, name);
        let (temperature, feels_like, humidity, dew_point) = (
            column("temperature_2m"),
            column("apparent_temperature"),
            column("relative_humidity_2m"),
            column("dew_point_2m"),
        );
        let (rain_chance, precipitation, snowfall, codes) = (
            column("precipitation_probability"),
            column("precipitation"),
            column("snowfall"),
            column("weather_code"),
        );
        let (pressure, wind, wind_direction, gusts, uv) = (
            column("pressure_msl"),
            column("wind_speed_10m"),
            column("wind_direction_10m"),
            column("wind_gusts_10m"),
            column("uv_index"),
        );
        times(hourly)
            .into_iter()
            .enumerate()
            .map(|(i, at)| Hour {
                at,
                temperature: temperature[i],
                feels_like: feels_like[i],
                humidity: humidity[i],
                dew_point: dew_point[i],
                rain_chance: rain_chance[i],
                precipitation: precipitation[i],
                snowfall: snowfall[i],
                pressure: pressure[i],
                wind: wind[i],
                wind_direction: wind_direction[i],
                gusts: gusts[i],
                uv: uv[i],
                code: codes[i]
                    .filter(|c| (0. ..=99.).contains(c) && c.fract() == 0.)
                    .map(|c| c as u16),
            })
            .collect()
    });
    let days: Vec<Day> = json.get("daily").map_or_else(Vec::new, |daily| {
        let (high, low, sunrise, sunset, daylight) = (
            numbers(daily, "temperature_2m_max"),
            numbers(daily, "temperature_2m_min"),
            numbers(daily, "sunrise"),
            numbers(daily, "sunset"),
            numbers(daily, "daylight_duration"),
        );
        let dates = match daily.get("time") {
            Some(Json::Arr(dates)) => dates.iter().map(|d| number(d).map_or(0, |d| d as i64)),
            _ => return Vec::new(),
        };
        dates
            .enumerate()
            .map(|(i, date)| Day {
                date,
                high: high[i],
                low: low[i],
                sunrise: sunrise[i].map(|t| t as i64),
                sunset: sunset[i].map(|t| t as i64),
                daylight: daylight[i],
            })
            .collect()
    });
    let local_date = |t: i64| (t + utc_offset).div_euclid(86_400);
    let today = days
        .iter()
        .find(|d| local_date(d.date) == local_date(current_time))
        .or(days.first());
    Some(Reading {
        temperature: field("temperature_2m")?,
        feels_like: field("apparent_temperature"),
        humidity: field("relative_humidity_2m"),
        wind: field("wind_speed_10m"),
        code: code as u16,
        is_day: field("is_day") != Some(0.),
        high: today.and_then(|d| d.high),
        low: today.and_then(|d| d.low),
        units,
        fetched: now,
        current_time,
        utc_offset,
        hours,
        days,
    })
}

/// An air quality answer received at `now`: `None` unless the status is 200 and
/// `hourly.time` is an array of numbers.
pub fn parse_air(status: u16, body: &[u8], now: i64) -> Option<Air> {
    let json = answer_json(status, body)?;
    let hourly = json.get("hourly")?;
    let Some(Json::Arr(times)) = hourly.get("time") else {
        return None;
    };
    let ats = times.iter().map(number).collect::<Option<Vec<_>>>()?;
    let column = |name| numbers(hourly, name);
    let (aqi, pm2_5, pm10, no2, o3, so2) = (
        column("european_aqi"),
        column("european_aqi_pm2_5"),
        column("european_aqi_pm10"),
        column("european_aqi_nitrogen_dioxide"),
        column("european_aqi_ozone"),
        column("european_aqi_sulphur_dioxide"),
    );
    let pollens = POLLENS.map(|name| numbers(hourly, &format!("{name}_pollen")));
    let hours = ats
        .iter()
        .enumerate()
        .map(|(i, at)| AirHour {
            at: *at as i64,
            aqi: aqi[i],
            pm2_5: pm2_5[i],
            pm10: pm10[i],
            no2: no2[i],
            o3: o3[i],
            so2: so2[i],
            pollen: std::array::from_fn(|p| pollens[p][i]),
        })
        .collect();
    Some(Air {
        hours,
        utc_offset: json
            .get("utc_offset_seconds")
            .and_then(number)
            .map_or(0, |o| o as i64),
        fetched: now,
    })
}

/// The times of an `hourly` object, in unix seconds; none when `time` is no array. An entry
/// that is no number is 0.
fn times(hourly: &Json) -> Vec<i64> {
    match hourly.get("time") {
        Some(Json::Arr(times)) => times
            .iter()
            .map(|t| number(t).map_or(0, |t| t as i64))
            .collect(),
        _ => Vec::new(),
    }
}

/// The numbers of the array `name` in `table`, as many as `table.time` has: a value that is
/// no number, or a missing array, gives `None`.
fn numbers(table: &Json, name: &str) -> Vec<Option<f64>> {
    let len = match table.get("time") {
        Some(Json::Arr(times)) => times.len(),
        _ => 0,
    };
    let values = match table.get(name) {
        Some(Json::Arr(values)) => values.as_slice(),
        _ => &[],
    };
    (0..len).map(|i| values.get(i).and_then(number)).collect()
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
    const BERLIN_DAY: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin-day.json");
    const MOSCOW_DAY_F: &[u8] =
        include_bytes!("../tests/fixtures/forecast-moscow-day-fahrenheit.json");
    const AIR_BERLIN: &[u8] = include_bytes!("../tests/fixtures/air-berlin.json");
    const AIR_NEW_YORK: &[u8] = include_bytes!("../tests/fixtures/air-new-york.json");

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
                current_time: 1_000,
                utc_offset: 3600,
                hours: vec![],
                days: vec![Day {
                    date: 0,
                    high: Some(5.4),
                    low: Some(1.6),
                    sunrise: None,
                    sunset: None,
                    daylight: None,
                }],
            }
        );
        let f = parse_forecast(200, FORECAST_F, Units::Imperial, 0).unwrap();
        assert_eq!((f.temperature, f.code, f.units), (40.4, 3, Units::Imperial));
    }

    #[test]
    fn the_day_answer_gives_hours_and_days() {
        let r = parse_forecast(200, BERLIN_DAY, Units::Metric, 7).unwrap();
        assert_eq!((r.hours.len(), r.days.len()), (50, 8));
        assert_eq!((r.utc_offset, r.current_time), (3600, 1795266900));
        let h = &r.hours[25];
        assert_eq!(h.at, 1795266000 + 3600);
        assert_eq!(h.temperature, Some(6.5));
        assert_eq!(h.rain_chance, Some(18.));
        assert_eq!(h.pressure, Some(1012.8));
        assert_eq!(h.wind_direction, Some(266.));
        assert_eq!(h.uv, Some(0.55));
        assert_eq!(h.code, Some(3));
        assert_eq!(
            r.days[1],
            Day {
                date: 1795215600,
                high: Some(6.6),
                low: Some(0.5),
                sunrise: Some(1795243100),
                sunset: Some(1795273527),
                daylight: Some(30420.66),
            }
        );
        assert_eq!((r.high, r.low), (Some(6.6), Some(0.5)));
    }

    #[test]
    fn an_imperial_day_answer_parses() {
        let r = parse_forecast(200, MOSCOW_DAY_F, Units::Imperial, 0).unwrap();
        assert_eq!(r.temperature, 38.7);
        assert_eq!((r.utc_offset, r.hours.len()), (10800, 50));
    }

    #[test]
    fn the_old_answer_still_parses() {
        let r = parse_forecast(200, FORECAST, Units::Metric, 9).unwrap();
        assert!(r.hours.is_empty());
        assert_eq!(r.days.len(), 1);
        assert_eq!(r.current_time, 9);
        assert_eq!(r.high, Some(5.4));
    }

    #[test]
    fn a_bad_value_in_an_hour_is_unknown() {
        let body = br#"{"current":{"temperature_2m":1,"weather_code":0},
            "hourly":{"time":[10,20,30],"temperature_2m":[1,null,"x"]}}"#;
        let r = parse_forecast(200, body, Units::Metric, 0).unwrap();
        let temps: Vec<_> = r.hours.iter().map(|h| h.temperature).collect();
        assert_eq!(temps, [Some(1.), None, None]);
        assert!(r.hours.iter().all(|h| h.uv.is_none() && h.code.is_none()));
        let no_time = br#"{"current":{"temperature_2m":1,"weather_code":0},
            "hourly":{"temperature_2m":[1,2]}}"#;
        let r = parse_forecast(200, no_time, Units::Metric, 0).unwrap();
        assert!(r.hours.is_empty());
    }

    #[test]
    fn air_answers_give_hours_and_pollen() {
        let a = parse_air(200, AIR_BERLIN, 5).unwrap();
        assert_eq!((a.hours.len(), a.utc_offset, a.fetched), (50, 3600, 5));
        let h = &a.hours[24];
        assert_eq!(h.aqi, Some(19.));
        assert_eq!((h.pm2_5, h.pm10, h.no2), (Some(19.), Some(13.), Some(18.)));
        assert_eq!((h.o3, h.so2), (Some(17.), Some(1.)));
        assert_eq!(h.pollen, [Some(0.); 6]);
        let ny = parse_air(200, AIR_NEW_YORK, 5).unwrap();
        assert_eq!(ny.hours[0].pollen, [None; 6]);
        assert_eq!(ny.hours[0].aqi, Some(65.));
    }

    #[test]
    fn a_bad_air_answer_is_none() {
        assert_eq!(parse_air(500, AIR_BERLIN, 0), None);
        assert_eq!(parse_air(200, b"<html>", 0), None);
        assert_eq!(parse_air(200, b"{}", 0), None);
    }

    #[test]
    fn urls_ask_for_the_day() {
        let place = Place {
            name: "Berlin".into(),
            latitude: 52.52437,
            longitude: 13.41053,
        };
        let url = forecast_url(&place, Units::Metric);
        assert!(
            url.contains(
                "&hourly=temperature_2m,apparent_temperature,relative_humidity_2m,dew_point_2m,precipitation_probability,precipitation,snowfall,weather_code,pressure_msl,wind_speed_10m,wind_direction_10m,wind_gusts_10m,uv_index&daily=temperature_2m_max,temperature_2m_min,sunrise,sunset,daylight_duration"
            ),
            "{url}"
        );
        assert!(
            url.ends_with(
                "&timezone=auto&timeformat=unixtime&past_hours=24&forecast_hours=26&past_days=1&forecast_days=7"
            ),
            "{url}"
        );
        let air = air_url(&place);
        assert!(
            air.starts_with("https://air-quality-api.open-meteo.com/v1/air-quality?latitude=52.52437&longitude=13.41053&hourly=european_aqi,"),
            "{air}"
        );
        assert!(air.contains(
            "alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,olive_pollen,ragweed_pollen"
        ));
        assert!(
            air.ends_with("&timezone=auto&timeformat=unixtime&past_hours=24&forecast_hours=26")
        );
        assert!(!air.contains("unit"), "{air}");
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
