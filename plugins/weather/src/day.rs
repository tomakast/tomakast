//! The plugin's day and what the widgets read from it. Pure, so it is tested natively.
//!
//! The day runs from 06:00 to 06:00 in the place's own time: morning 06–12, afternoon
//! 12–18, evening 18–24, night 00–06 of the next date. At 02:00 the day is still yesterday's,
//! and the night is its current part. An hour belongs to the day when it starts inside it; its
//! slot is the number of hours since the day's start (0..24). "Now" is the place's now, from
//! the answer (see [`place_now`]), never the system clock's time zone.

use crate::api::{Air, AirHour, Hour, POLLENS, Reading};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// The place's own time at `now` (unix seconds), from the answer's `current_time`.
pub fn place_now(reading: &Reading, now: i64) -> i64 {
    reading.current_time + (now - reading.fetched)
}

/// The unix time of the local 06:00 at or before `at`, at an offset of `utc_offset` seconds.
pub fn day_start(at: i64, utc_offset: i64) -> i64 {
    let local = at + utc_offset;
    let mut start = local.div_euclid(DAY) * DAY + 6 * HOUR;
    if start > local {
        start -= DAY;
    }
    start - utc_offset
}

/// The start of the day the place is in at `now`.
fn start_of(reading: &Reading, now: i64) -> i64 {
    day_start(place_now(reading, now), reading.utc_offset)
}

/// The slot of an hour starting at `at` in the day starting at `start`.
fn slot(at: i64, start: i64) -> Option<usize> {
    (start..start + DAY)
        .contains(&at)
        .then(|| ((at - start) / HOUR) as usize)
}

/// The four parts of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Morning, Part::Afternoon, Part::Evening, Part::Night];

    pub fn name(self) -> &'static str {
        match self {
            Part::Morning => "Morning",
            Part::Afternoon => "Afternoon",
            Part::Evening => "Evening",
            Part::Night => "Night",
        }
    }

    /// The name of its icon.
    pub fn icon(self) -> &'static str {
        match self {
            Part::Morning => "sunrise",
            Part::Afternoon => "sun",
            Part::Evening => "sunset",
            Part::Night => "moon",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Where a part is from the place's now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Past,
    Now,
    Ahead,
}

/// A part of the day in a few values; unknown when no hour of it has one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartSummary {
    pub part: Part,
    pub when: When,
    /// The values of the part's middle hour.
    pub temperature: Option<f64>,
    pub feels_like: Option<f64>,
    pub wind: Option<f64>,
    /// The worst sky of the part (see [`severity`]).
    pub code: Option<u16>,
    /// The highest chance of rain, in percent.
    pub rain_chance: Option<f64>,
    /// The rain of all its hours, in mm.
    pub rain: Option<f64>,
}

/// The day's hours by slot.
fn slots(reading: &Reading, start: i64) -> [Option<&Hour>; 24] {
    let mut slots = [None; 24];
    for hour in &reading.hours {
        if let Some(i) = slot(hour.at, start) {
            slots[i] = Some(hour);
        }
    }
    slots
}

/// The day's four parts at `now`.
pub fn parts(reading: &Reading, now: i64) -> [PartSummary; 4] {
    let (here, start) = (place_now(reading, now), start_of(reading, now));
    let slots = slots(reading, start);
    Part::ALL.map(|part| {
        let first = 6 * part.index();
        let hours: Vec<&Hour> = slots[first..first + 6].iter().flatten().copied().collect();
        let middle = slots[first + 3];
        let begin = start + first as i64 * HOUR;
        let rains: Vec<f64> = hours.iter().filter_map(|h| h.precipitation).collect();
        PartSummary {
            part,
            when: if here < begin {
                When::Ahead
            } else if here >= begin + 6 * HOUR {
                When::Past
            } else {
                When::Now
            },
            temperature: middle.and_then(|h| h.temperature),
            feels_like: middle.and_then(|h| h.feels_like),
            wind: middle.and_then(|h| h.wind),
            code: hours
                .iter()
                .filter_map(|h| h.code)
                .max_by_key(|c| severity(*c)),
            rain_chance: hours
                .iter()
                .filter_map(|h| h.rain_chance)
                .max_by(f64::total_cmp),
            rain: (!rains.is_empty()).then(|| rains.iter().sum()),
        }
    })
}

/// How bad a sky is: its class rank (thunder, snow, rain, drizzle, fog, overcast, partly
/// cloudy, clear; higher is worse) and its code, which orders within a class (showers 80 are
/// worse than rain 61).
pub fn severity(code: u16) -> (u8, u16) {
    let rank = match code {
        95..=99 => 7,
        71..=77 | 85 | 86 => 6,
        61..=67 | 80..=82 => 5,
        51..=57 => 4,
        45 | 48 => 3,
        3 => 2,
        1 | 2 => 1,
        _ => 0,
    };
    (rank, code)
}

/// The day's high and low, from its hours.
pub fn range(reading: &Reading, now: i64) -> Option<(f64, f64)> {
    let start = start_of(reading, now);
    let temperatures = reading
        .hours
        .iter()
        .filter(|h| slot(h.at, start).is_some())
        .filter_map(|h| h.temperature);
    temperatures.fold(None, |range, t| match range {
        None => Some((t, t)),
        Some((high, low)) => Some((high.max(t), low.min(t))),
    })
}

/// The day's hours from 06:00 to 05:00 in one measure, for a chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Series {
    pub values: [Option<f64>; 24],
    /// The place's now, in hours since the day's start (0..24).
    pub now: f32,
}

fn now_hours(here: i64, start: i64) -> f32 {
    ((here - start) as f32 / HOUR as f32).clamp(0., 24.)
}

/// The measure `pick` takes from each hour of the day.
pub fn series(reading: &Reading, now: i64, pick: fn(&Hour) -> Option<f64>) -> Series {
    let (here, start) = (place_now(reading, now), start_of(reading, now));
    let mut values = [None; 24];
    for (value, hour) in values.iter_mut().zip(slots(reading, start)) {
        *value = hour.and_then(pick);
    }
    Series {
        values,
        now: now_hours(here, start),
    }
}

/// Like [`series`] for the air's hours, over the day of the forecast's place time.
pub fn air_series(
    air: &Air,
    reading: &Reading,
    now: i64,
    pick: fn(&AirHour) -> Option<f64>,
) -> Series {
    let (here, start) = (place_now(reading, now), start_of(reading, now));
    let mut values = [None; 24];
    for hour in &air.hours {
        if let Some(i) = slot(hour.at, start) {
            values[i] = pick(hour);
        }
    }
    Series {
        values,
        now: now_hours(here, start),
    }
}

/// The hour that holds the place's now.
pub fn hour_now(reading: &Reading, now: i64) -> Option<&Hour> {
    let here = place_now(reading, now);
    reading
        .hours
        .iter()
        .find(|h| (h.at..h.at + HOUR).contains(&here))
}

/// The air's hour that holds the place's now.
pub fn air_now<'a>(air: &'a Air, reading: &Reading, now: i64) -> Option<&'a AirHour> {
    let here = place_now(reading, now);
    air.hours
        .iter()
        .find(|h| (h.at..h.at + HOUR).contains(&here))
}

/// The UV band of an index, with its colour.
pub fn uv_level(uv: f64) -> (&'static str, u32) {
    match uv.round() {
        u if u < 3. => ("Low", 0x6FCF6FFF),
        u if u < 6. => ("Moderate", 0xF2D64BFF),
        u if u < 8. => ("High", 0xF29A3BFF),
        u if u < 11. => ("Very high", 0xE5534BFF),
        _ => ("Extreme", 0xB07CE8FF),
    }
}

/// The band of the European air quality index, with its colour.
pub fn aqi_level(aqi: f64) -> (&'static str, u32) {
    match aqi.round() {
        a if a < 20. => ("Good", 0x50F0E6FF),
        a if a < 40. => ("Fair", 0x50CCAAFF),
        a if a < 60. => ("Moderate", 0xF0E641FF),
        a if a < 80. => ("Poor", 0xFF5050FF),
        a if a < 100. => ("Very poor", 0x960032FF),
        _ => ("Extremely poor", 0x7D2181FF),
    }
}

/// The band of `grains` per cubic metre of the pollen at index `pollen` of [`POLLENS`]; none
/// under 1 grain.
pub fn pollen_level(pollen: usize, grains: f64) -> Option<&'static str> {
    if grains < 1. {
        return None;
    }
    // The upper bounds of Low, Moderate and High; the first is for trees, then grass, weeds.
    let [low, moderate, high] = match POLLENS.get(pollen).copied() {
        Some("grass") => [5., 20., 50.],
        Some("mugwort" | "ragweed") => [5., 15., 50.],
        _ => [10., 50., 200.],
    };
    Some(match grains {
        g if g < low => "Low",
        g if g < moderate => "Moderate",
        g if g < high => "High",
        _ => "Very high",
    })
}

/// How long the UV stays at 3 or more (rounded, as it is shown and its level read): "till
/// 15:00" while it is, "from 11:00" when it will be later today, else none.
pub fn uv_until(reading: &Reading, now: i64) -> Option<String> {
    let start = start_of(reading, now);
    let current = hour_now(reading, now)?;
    let mut later: Vec<&Hour> = reading
        .hours
        .iter()
        .filter(|h| h.at > current.at && slot(h.at, start).is_some())
        .collect();
    later.sort_by_key(|h| h.at);
    let strong = |h: &Hour| h.uv.is_some_and(|uv| uv.round() >= 3.);
    if strong(current) {
        let end = later
            .iter()
            .find(|h| h.uv.is_some_and(|uv| uv.round() < 3.))?;
        Some(format!("till {}", clock(end.at, reading.utc_offset)))
    } else {
        let begin = later.iter().find(|h| strong(h))?;
        Some(format!("from {}", clock(begin.at, reading.utc_offset)))
    }
}

/// Where the pressure is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Rising,
    Falling,
    Steady,
}

/// The pressure now against 3 hours earlier: 1 hPa or more is a change.
pub fn pressure_trend(reading: &Reading, now: i64) -> Option<Trend> {
    let current = hour_now(reading, now)?;
    let before = reading
        .hours
        .iter()
        .find(|h| h.at == current.at - 3 * HOUR)?;
    let change = current.pressure? - before.pressure?;
    Some(if change >= 1. {
        Trend::Rising
    } else if change <= -1. {
        Trend::Falling
    } else {
        Trend::Steady
    })
}

/// The arrow a wind from `from_degrees` blows to, and the point it blows from, in 8 points.
pub fn compass(from_degrees: f64) -> (&'static str, &'static str) {
    const POINTS: [(&str, &str); 8] = [
        ("↓", "N"),
        ("↙", "NE"),
        ("←", "E"),
        ("↖", "SE"),
        ("↑", "S"),
        ("↗", "SW"),
        ("→", "W"),
        ("↘", "NW"),
    ];
    POINTS[((from_degrees / 45.).round() as i64).rem_euclid(8) as usize]
}

/// The first part from the current one on whose chance of rain is 40% or more: "70% now",
/// or "40% after 18:00" for a later one.
pub fn rain_outlook(reading: &Reading, now: i64) -> Option<String> {
    let start = start_of(reading, now);
    parts(reading, now)
        .into_iter()
        .filter(|p| p.when != When::Past)
        .find_map(|p| {
            let chance = p.rain_chance.filter(|c| *c >= 40.)?;
            Some(match p.when {
                When::Now => format!("{chance:.0}% now"),
                _ => {
                    let begin = start + 6 * p.part.index() as i64 * HOUR;
                    format!("{chance:.0}% after {}", clock(begin, reading.utc_offset))
                }
            })
        })
}

/// The pollens of the current hour with a grain or more as `(index in POLLENS, grains)`,
/// strongest band first, then most grains.
pub fn strongest_pollen(air: &Air, reading: &Reading, now: i64) -> Vec<(usize, f64)> {
    let Some(hour) = air_now(air, reading, now) else {
        return Vec::new();
    };
    let band = |(pollen, grains): &(usize, f64)| match pollen_level(*pollen, *grains) {
        Some("Low") => 1,
        Some("Moderate") => 2,
        Some("High") => 3,
        Some(_) => 4,
        None => 0,
    };
    let mut found: Vec<(usize, f64)> = hour
        .pollen
        .iter()
        .enumerate()
        .filter_map(|(i, grains)| Some((i, (*grains)?)))
        .filter(|(_, grains)| *grains >= 1.)
        .collect();
    found.sort_by(|a, b| band(b).cmp(&band(a)).then(b.1.total_cmp(&a.1)));
    found
}

/// The length of the day and the sun's times.
#[derive(Debug, Clone, PartialEq)]
pub struct DaylightInfo {
    /// In seconds.
    pub length: f64,
    pub sunrise: i64,
    pub sunset: i64,
    /// Against the next day, in seconds.
    pub change: f64,
    /// The lengths of up to 7 days from this one on.
    pub week: Vec<f64>,
}

/// The daylight of the plugin day's date; none when the answer lacks the day, its sun times
/// or the next day's length.
pub fn daylight(reading: &Reading, now: i64) -> Option<DaylightInfo> {
    let date = |at: i64| (at + reading.utc_offset).div_euclid(DAY);
    let today = date(start_of(reading, now));
    let first = reading.days.iter().position(|d| date(d.date) == today)?;
    let days = &reading.days[first..];
    let length = days[0].daylight?;
    Some(DaylightInfo {
        length,
        sunrise: days[0].sunrise?,
        sunset: days[0].sunset?,
        change: days.get(1)?.daylight? - length,
        week: days.iter().filter_map(|d| d.daylight).take(7).collect(),
    })
}

/// The clock time of `at` at `utc_offset`: "07:38".
pub fn clock(at: i64, utc_offset: i64) -> String {
    let minutes = (at + utc_offset).rem_euclid(DAY) / 60;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A span in hours and minutes: "8 h 27 min".
pub fn length(secs: f64) -> String {
    let minutes = (secs / 60.).round() as i64;
    format!("{} h {} min", minutes / 60, minutes % 60)
}

/// A change in whole minutes: "\u{2212}3 min", "+2 min", "±0 min".
pub fn signed_minutes(secs: f64) -> String {
    match (secs / 60.).round() as i64 {
        0 => "±0 min".to_string(),
        m if m < 0 => format!("\u{2212}{} min", -m),
        m => format!("+{m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{self, Air, AirHour, Hour, Reading, Units};

    const BERLIN_DAY: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin-day.json");
    const AIR_BERLIN: &[u8] = include_bytes!("../tests/fixtures/air-berlin.json");
    const AIR_NEW_YORK: &[u8] = include_bytes!("../tests/fixtures/air-new-york.json");

    /// Fetched, and the `now` of the tests: the place's now is 2026-11-21 14:15 local.
    const F: i64 = 1_800_000_000;
    /// 2026-11-21 06:00 local in Berlin.
    const START: i64 = 1795237200;

    fn berlin() -> Reading {
        api::parse_forecast(200, BERLIN_DAY, Units::Metric, F).unwrap()
    }

    /// 24 hours from `start`, the temperature of each its slot; the place's now at `now`.
    fn synthetic(start: i64, now: i64) -> Reading {
        let mut reading = berlin();
        reading.hours = (0..24)
            .map(|i| Hour {
                at: start + i * 3600,
                temperature: Some(i as f64),
                ..Hour::default()
            })
            .collect();
        reading.current_time = now;
        reading
    }

    /// A day of hours with uv 3 from 11:00 to 14:00 local, at `local_hour` o'clock.
    fn uv_day(local_hour: i64) -> Reading {
        let mut reading = synthetic(START, START + (local_hour - 6) * 3600);
        for (i, hour) in reading.hours.iter_mut().enumerate() {
            let local = 6 + i as i64;
            hour.uv = Some(if (11..=14).contains(&local) { 3. } else { 0. });
        }
        reading
    }

    #[test]
    fn the_day_starts_at_six_local() {
        assert_eq!(day_start(1795266900, 3600), START);
        assert_eq!(day_start(START - 4 * 3600, 3600), START - 86_400);
        assert_eq!(day_start(START, 3600), START);
        assert_eq!(day_start(START - 1, 3600), START - 86_400);
        let start = day_start(1795266900, 19_800);
        assert_eq!((start + 19_800).rem_euclid(86_400), 6 * 3600);
        assert!(start <= 1795266900 && 1795266900 - start < 86_400);
    }

    #[test]
    fn parts_of_berlin_at_a_quarter_past_two() {
        let parts = parts(&berlin(), F);
        let [morning, afternoon, evening, night] = &parts;
        assert_eq!(
            parts.map(|p| (p.part, p.when)),
            [
                (Part::Morning, When::Past),
                (Part::Afternoon, When::Now),
                (Part::Evening, When::Ahead),
                (Part::Night, When::Ahead),
            ]
        );
        assert_eq!(morning.temperature, Some(1.6));
        assert_eq!(morning.code, Some(3));
        assert_eq!(morning.rain_chance, Some(0.));
        assert_eq!(morning.rain, Some(0.));
        assert_eq!(afternoon.temperature, Some(6.5));
        assert_eq!(afternoon.feels_like, Some(2.2));
        assert_eq!(afternoon.wind, Some(17.7));
        assert_eq!(afternoon.code, Some(80));
        assert_eq!(afternoon.rain_chance, Some(25.));
        assert!((afternoon.rain.unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(evening.temperature, Some(5.1));
        assert_eq!(evening.code, Some(80));
        assert_eq!(evening.rain_chance, Some(40.));
        assert!((evening.rain.unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(night.temperature, Some(4.5));
        assert_eq!(night.feels_like, Some(0.8));
        assert_eq!(night.code, Some(61));
        assert_eq!(night.rain_chance, Some(45.));
        assert!((night.rain.unwrap() - 1.1).abs() < 1e-9);
    }

    #[test]
    fn the_part_names_and_icons() {
        let names = Part::ALL.map(Part::name);
        assert_eq!(names, ["Morning", "Afternoon", "Evening", "Night"]);
        let icons = Part::ALL.map(Part::icon);
        assert_eq!(icons, ["sunrise", "sun", "sunset", "moon"]);
    }

    #[test]
    fn at_two_at_night_the_day_is_yesterdays() {
        let start = START - 86_400;
        let reading = synthetic(start, START - 4 * 3600);
        let parts = parts(&reading, F);
        assert_eq!(
            parts.map(|p| p.when),
            [When::Past, When::Past, When::Past, When::Now]
        );
        // The middle hours are 09, 15, 21 and 03.
        assert_eq!(
            parts.map(|p| p.temperature),
            [Some(3.), Some(9.), Some(15.), Some(21.)]
        );
    }

    #[test]
    fn a_part_without_hours_is_unknown() {
        let mut reading = berlin();
        reading
            .hours
            .retain(|h| (START..START + 6 * 3600).contains(&h.at));
        let parts = parts(&reading, F);
        assert_eq!(parts[0].temperature, Some(1.6));
        for p in &parts[1..] {
            assert_eq!(
                (
                    p.temperature,
                    p.feels_like,
                    p.wind,
                    p.code,
                    p.rain_chance,
                    p.rain
                ),
                (None, None, None, None, None, None)
            );
        }
        reading.hours.clear();
        for p in parts_of(&reading) {
            assert_eq!(
                (
                    p.temperature,
                    p.feels_like,
                    p.wind,
                    p.code,
                    p.rain_chance,
                    p.rain
                ),
                (None, None, None, None, None, None)
            );
        }
    }

    fn parts_of(reading: &Reading) -> [PartSummary; 4] {
        parts(reading, F)
    }

    #[test]
    fn an_hour_that_is_no_time_falls_outside_every_day() {
        let mut reading = berlin();
        reading.hours.insert(
            0,
            Hour {
                at: 0,
                temperature: Some(99.),
                ..Hour::default()
            },
        );
        assert_eq!(range(&reading, F), Some((6.5, 1.4)));
    }

    #[test]
    fn severity_orders_the_sky() {
        let order = [95, 71, 61, 51, 45, 3, 2, 0];
        for pair in order.windows(2) {
            assert!(severity(pair[0]) > severity(pair[1]), "{pair:?}");
        }
        assert!(severity(80) > severity(61));
        assert_eq!(severity(86).0, severity(71).0);
        assert!(severity(1).0 == severity(2).0 && severity(2).0 > severity(0).0);
    }

    #[test]
    fn the_series_is_the_day_from_six() {
        let s = series(&berlin(), F, |h| h.humidity);
        assert_eq!(s.values[0], Some(93.));
        assert_eq!(s.values[23], Some(88.));
        assert_eq!(s.now, 8.25);
        assert!(s.values.iter().all(Option::is_some));
        let s = series(
            &Reading {
                hours: vec![],
                ..berlin()
            },
            F,
            |h| h.humidity,
        );
        assert_eq!(s.values, [None; 24]);
    }

    #[test]
    fn the_air_series_is_the_day_by_hour_start() {
        let air = api::parse_air(200, AIR_BERLIN, F).unwrap();
        let s = air_series(&air, &berlin(), F, |h| h.aqi);
        assert_eq!(s.now, 8.25);
        let slot = |at: i64| ((at - START) / 3600) as usize;
        for h in air
            .hours
            .iter()
            .filter(|h| (START..START + 86_400).contains(&h.at))
        {
            assert_eq!(s.values[slot(h.at)], h.aqi);
        }
        assert!(s.values.iter().any(Option::is_some));
    }

    #[test]
    fn the_range_is_the_days_hours() {
        assert_eq!(range(&berlin(), F), Some((6.5, 1.4)));
        assert_eq!(
            range(
                &Reading {
                    hours: vec![],
                    ..berlin()
                },
                F
            ),
            None
        );
    }

    #[test]
    fn the_hour_now_holds_the_places_now() {
        let reading = berlin();
        assert_eq!(hour_now(&reading, F).map(|h| h.at), Some(1795266000));
        let air = api::parse_air(200, AIR_BERLIN, F).unwrap();
        assert_eq!(air_now(&air, &reading, F).map(|h| h.at), Some(1795266000));
        assert_eq!(
            hour_now(
                &Reading {
                    hours: vec![],
                    ..berlin()
                },
                F
            ),
            None
        );
    }

    #[test]
    fn levels_and_bands() {
        assert_eq!(uv_level(2.4), ("Low", 0x6FCF6FFF));
        assert_eq!(uv_level(2.6), ("Moderate", 0xF2D64BFF));
        assert_eq!(uv_level(5.4).0, "Moderate");
        assert_eq!(uv_level(5.6).0, "High");
        assert_eq!(uv_level(7.4), ("High", 0xF29A3BFF));
        assert_eq!(uv_level(7.6), ("Very high", 0xE5534BFF));
        assert_eq!(uv_level(10.4).0, "Very high");
        assert_eq!(uv_level(11.), ("Extreme", 0xB07CE8FF));
        assert_eq!(aqi_level(19.), ("Good", 0x50F0E6FF));
        assert_eq!(aqi_level(20.), ("Fair", 0x50CCAAFF));
        assert_eq!(aqi_level(40.), ("Moderate", 0xF0E641FF));
        assert_eq!(aqi_level(60.), ("Poor", 0xFF5050FF));
        assert_eq!(aqi_level(80.), ("Very poor", 0x960032FF));
        assert_eq!(aqi_level(100.), ("Extremely poor", 0x7D2181FF));
        // Alder 0, birch 1, grass 2, mugwort 3, olive 4, ragweed 5.
        assert_eq!(pollen_level(1, 9.), Some("Low"));
        assert_eq!(pollen_level(1, 10.), Some("Moderate"));
        assert_eq!(pollen_level(1, 50.), Some("High"));
        assert_eq!(pollen_level(0, 200.), Some("Very high"));
        assert_eq!(pollen_level(2, 4.), Some("Low"));
        assert_eq!(pollen_level(2, 20.), Some("High"));
        assert_eq!(pollen_level(2, 50.), Some("Very high"));
        assert_eq!(pollen_level(3, 14.), Some("Moderate"));
        assert_eq!(pollen_level(5, 15.), Some("High"));
        assert_eq!(pollen_level(5, 0.5), None);
    }

    #[test]
    fn uv_until_and_from() {
        assert_eq!(uv_until(&uv_day(12), F).as_deref(), Some("till 15:00"));
        assert_eq!(uv_until(&uv_day(9), F).as_deref(), Some("from 11:00"));
        assert_eq!(uv_until(&uv_day(16), F), None);
        // Rounded, as the value and its level are: 2.6 shows as 3, Moderate.
        let mut reading = uv_day(12);
        reading.hours[6].uv = Some(2.6);
        reading.hours[9].uv = Some(2.6);
        assert_eq!(uv_until(&reading, F).as_deref(), Some("till 16:00"));
        reading.hours[6].uv = Some(2.4);
        assert_eq!(uv_until(&reading, F).as_deref(), Some("from 13:00"));
    }

    #[test]
    fn pressure_is_steady_in_berlin() {
        let mut reading = berlin();
        assert_eq!(pressure_trend(&reading, F), Some(Trend::Steady));
        reading.hours[21].pressure = Some(1000.);
        reading.hours[24].pressure = Some(1001.);
        assert_eq!(pressure_trend(&reading, F), Some(Trend::Rising));
        reading.hours[24].pressure = Some(999.);
        assert_eq!(pressure_trend(&reading, F), Some(Trend::Falling));
        reading.hours[21].pressure = None;
        assert_eq!(pressure_trend(&reading, F), None);
    }

    #[test]
    fn compass_points() {
        assert_eq!(compass(266.), ("→", "W"));
        assert_eq!(compass(0.), ("↓", "N"));
        assert_eq!(compass(45.), ("↙", "NE"));
        assert_eq!(compass(359.), ("↓", "N"));
        assert_eq!(compass(180.), ("↑", "S"));
    }

    #[test]
    fn rain_outlook_in_berlin() {
        let mut reading = berlin();
        assert_eq!(
            rain_outlook(&reading, F).as_deref(),
            Some("40% after 18:00")
        );
        reading.hours[25].rain_chance = Some(70.);
        assert_eq!(rain_outlook(&reading, F).as_deref(), Some("70% now"));
        for hour in &mut reading.hours {
            hour.rain_chance = Some(10.);
        }
        assert_eq!(rain_outlook(&reading, F), None);
    }

    #[test]
    fn daylight_in_berlin() {
        let info = daylight(&berlin(), F).unwrap();
        assert_eq!(info.length, 30420.66);
        assert_eq!(info.sunrise, 1795243100);
        assert_eq!(info.sunset, 1795273527);
        assert!((info.change + 169.69).abs() < 0.01, "{}", info.change);
        assert_eq!(info.week.len(), 7);
        assert_eq!(info.week[0], 30420.66);
        assert_eq!(clock(1795243100, 3600), "07:38");
        assert_eq!(length(30420.66), "8 h 27 min");
        assert_eq!(signed_minutes(-169.69), "\u{2212}3 min");
        assert_eq!(signed_minutes(100.), "+2 min");
        assert_eq!(signed_minutes(10.), "±0 min");
    }

    #[test]
    fn pollen_strongest_first() {
        let reading = berlin();
        let air = Air {
            hours: vec![AirHour {
                at: 1795266000,
                pollen: [Some(0.5), Some(60.), Some(30.), None, None, None],
                ..AirHour::default()
            }],
            utc_offset: 3600,
            fetched: F,
        };
        assert_eq!(
            strongest_pollen(&air, &reading, F),
            vec![(1, 60.), (2, 30.)]
        );
        let ny = api::parse_air(200, AIR_NEW_YORK, F).unwrap();
        assert_eq!(strongest_pollen(&ny, &reading, F), vec![]);
    }
}
