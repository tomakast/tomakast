//! The eight measure tiles: humidity, wind, UV, pressure, precipitation, air quality, pollen
//! and daylight. Pure, so they are tested natively.
//!
//! - Quarter: a muted icon and the measure's name, its value now, a small chart of the
//!   plugin's day (see [`day`](crate::day)) and the place under it.
//! - Single: the same with a detail line in a left column 150 wide (the dew point, where the
//!   wind is from, how long the UV lasts and so on); the chart fills the rest, with the hour
//!   marks.
//!
//! The value now is the hourly value of the hour that holds the place's now, so it agrees with
//! the chart's dot. A unit is drawn smaller and muted beside the number ("1013" "hPa").
//! Daylight draws the week's day lengths instead of a day chart. The place line says how old
//! the reading is when it is old or the last refresh failed, as every tile does. A value the
//! day lacks is "—" and the chart is still drawn. Without a forecast the tile is calm, as the
//! `now` quarter is; the air quality and pollen tiles are also calm while their air is
//! loading or failed. Pollen with none in the whole day says so.

use tomakast_plugin::{
    Align, Color, ColorToken, FrameBuilder, Layout, Node, NodeRef, TextStyle, WidgetContext,
    WidgetFrame,
};

use crate::api::{Air, AirHour, Hour, POLLENS, Reading};
use crate::chart::{self, Chart, LINE, Mark};
use crate::day::{self, Series, Trend};
use crate::ink::{self, MUTED, Size};
use crate::location::Status;
use crate::tile::{self, PADDING, View, caption, degrees};

/// The measure widgets.
pub const MEASURES: [&str; 8] = [
    "humidity",
    "wind",
    "uv",
    "pressure",
    "precipitation",
    "air-quality",
    "pollen",
    "daylight",
];

/// The header icon.
const ICON: f32 = 16.;
/// The icons beside the sun's times.
const SMALL: f32 = 12.;
/// The width of the single's left column.
const LEFT: f32 = 150.;
/// The height of the quarter's chart and of the single's.
const QUARTER_CHART: f32 = 22.;
const SINGLE_CHART: f32 = 90.;
/// Millimetres of mercury in a hectopascal.
const MM_HG: f64 = 0.750062;
/// The colour of pollen bars.
const POLLEN: u32 = 0xA6D96AFF;
const DASH: &str = "—";

/// Whether `widget` reads the place's air.
pub fn needs_air(widget: &str) -> bool {
    matches!(widget, "air-quality" | "pollen")
}

/// The chart of a measure.
enum Plot {
    /// The day's hours.
    Day(Box<Series>, Mark),
    /// The lengths of the days of the week.
    Week(Vec<f64>),
    /// None: the note is shown in its place.
    Nothing,
}

/// A chart of the day's hours of `series`.
fn day_plot(series: Series, mark: Mark) -> Plot {
    Plot::Day(Box::new(series), mark)
}

/// The detail line of a single.
enum Detail {
    Text(String),
    /// Sunrise, sunset and the change against the next day.
    Sun(String, String, String),
}

/// A value: its text and its unit, drawn smaller beside it ("1013" "hPa"); "—" has none.
type Value = (String, Option<&'static str>);

/// A value without a unit.
fn plain(text: impl Into<String>) -> Value {
    (text.into(), None)
}

/// `value` as `show` writes it with `unit`, or a dash.
fn with_unit(value: Option<f64>, show: impl FnOnce(f64) -> String, unit: &'static str) -> Value {
    match value {
        Some(v) => (show(v), Some(unit)),
        None => plain(DASH),
    }
}

/// What a measure tile shows.
struct Measure {
    icon: &'static str,
    name: String,
    /// The value now, and its unit when it has one.
    value: Value,
    detail: Detail,
    plot: Plot,
}

/// The tile of the measure `widget` showing `v` at `now` (unix seconds), as a quarter or a
/// single by the size `context` tells; `mmhg` shows the pressure in mm Hg.
pub fn measure(
    widget: &str,
    v: &View,
    context: &WidgetContext,
    now: i64,
    mmhg: bool,
) -> Result<WidgetFrame, String> {
    if !MEASURES.contains(&widget) {
        return Err(format!("no widget {widget}"));
    }
    let Some(r) = v.reading else {
        return tile::quarter(v, context, now);
    };
    let m = match (widget, v.air) {
        ("humidity", _) => humidity(r, now),
        ("wind", _) => wind(r, now),
        ("uv", _) => uv(r, now),
        ("pressure", _) => pressure(r, now, mmhg),
        ("precipitation", _) => precipitation(r, now),
        ("air-quality", Some(air)) => air_quality(air, r, now),
        ("pollen", Some(air)) => pollen(air, r, now),
        ("daylight", _) => daylight(r, now),
        _ => return air_calm(v),
    };
    let frame = match ink::size_of(context) {
        Size::Quarter => quarter(&m, v, r, context, now),
        Size::Single | Size::Double => single(&m, v, r, context, now),
    };
    tile::with_actions(frame)
}

/// An air tile without its air: loading, or offline.
fn air_calm(v: &View) -> Result<WidgetFrame, String> {
    let (icon, word, under) = match v.air_status {
        Status::Failed => (
            "cloud-off",
            "No air data",
            "Offline, will retry".to_string(),
        ),
        status => tile::calm(&View {
            status,
            ..v.clone()
        }),
    };
    tile::calm_tile(icon, word, under)
}

/// `value` rounded to a whole number, without `-0`.
fn whole(value: f64) -> String {
    format!("{}", value.round() as i64)
}

/// `value` as `show` writes it, or a dash.
fn or_dash(value: Option<f64>, show: impl FnOnce(f64) -> String) -> String {
    value.map_or_else(|| DASH.to_string(), show)
}

/// An amount: one decimal under 10 when it is not whole, "0" for none.
fn amount(value: f64) -> String {
    let tenths = (value * 10.).round();
    if tenths <= 0. {
        "0".into()
    } else if tenths < 100. && tenths % 10. != 0. {
        format!("{:.1}", tenths / 10.)
    } else {
        whole(value)
    }
}

/// Whether an amount shows as more than "0".
fn shows(value: f64) -> bool {
    (value * 10.).round() > 0.
}

/// The measure `pick` of the hour now.
fn now_of(r: &Reading, now: i64, pick: fn(&Hour) -> Option<f64>) -> Option<f64> {
    day::hour_now(r, now).and_then(pick)
}

fn humidity(r: &Reading, now: i64) -> Measure {
    Measure {
        icon: "droplets",
        name: "Humidity".into(),
        value: with_unit(now_of(r, now, |h| h.humidity), whole, "%"),
        detail: Detail::Text(format!(
            "Dew point {}",
            or_dash(now_of(r, now, |h| h.dew_point), degrees)
        )),
        plot: day_plot(day::series(r, now, |h| h.humidity), Mark::Line),
    }
}

fn wind(r: &Reading, now: i64) -> Measure {
    let unit = tile::wind_unit(r.units);
    let mut detail = Vec::new();
    if let Some(from) = now_of(r, now, |h| h.wind_direction) {
        let (arrow, point) = day::compass(from);
        detail.push(format!("{arrow} {point}"));
    }
    if let Some(gusts) = now_of(r, now, |h| h.gusts) {
        detail.push(format!("gusts {}", whole(gusts)));
    }
    Measure {
        icon: "wind",
        name: "Wind".into(),
        value: with_unit(now_of(r, now, |h| h.wind), whole, unit),
        detail: Detail::Text(joined(detail)),
        plot: day_plot(day::series(r, now, |h| h.wind), Mark::Line),
    }
}

/// `parts` joined by a middle dot, or a dash when there are none.
fn joined(parts: Vec<String>) -> String {
    if parts.is_empty() {
        DASH.into()
    } else {
        parts.join(" · ")
    }
}

fn uv(r: &Reading, now: i64) -> Measure {
    let value = now_of(r, now, |h| h.uv);
    let detail = match value {
        Some(uv) => {
            let level = day::uv_level(uv).0.to_string();
            joined(
                [Some(level), day::uv_until(r, now)]
                    .into_iter()
                    .flatten()
                    .collect(),
            )
        }
        None => DASH.into(),
    };
    Measure {
        icon: "sun",
        name: "UV index".into(),
        value: plain(or_dash(value, whole)),
        detail: Detail::Text(detail),
        plot: day_plot(
            day::series(r, now, |h| h.uv),
            Mark::Bars(|uv| day::uv_level(uv).1),
        ),
    }
}

fn pressure(r: &Reading, now: i64, mmhg: bool) -> Measure {
    let value = now_of(r, now, |h| h.pressure);
    let trend = match day::pressure_trend(r, now) {
        Some(Trend::Rising) => "↑ rising",
        Some(Trend::Falling) => "↓ falling",
        Some(Trend::Steady) => "→ steady",
        None => DASH,
    };
    Measure {
        icon: "gauge",
        name: "Pressure".into(),
        value: if mmhg {
            with_unit(value, |p| whole(p * MM_HG), "mm Hg")
        } else {
            with_unit(value, whole, "hPa")
        },
        detail: Detail::Text(trend.into()),
        plot: day_plot(day::series(r, now, |h| h.pressure), Mark::Line),
    }
}

fn precipitation(r: &Reading, now: i64) -> Measure {
    let hour = day::hour_now(r, now);
    // Snow when it shows at one decimal, else the rain.
    let value = match hour {
        Some(Hour {
            snowfall: Some(snow),
            ..
        }) if shows(*snow) => with_unit(Some(*snow), amount, "cm"),
        Some(h) => with_unit(h.precipitation, amount, "mm"),
        None => plain(DASH),
    };
    // Without the hour now nothing is known of the rain ahead either.
    let outlook = match hour {
        Some(_) => day::rain_outlook(r, now).unwrap_or_else(|| "No rain ahead".into()),
        None => DASH.into(),
    };
    Measure {
        icon: "umbrella",
        name: "Rain".into(),
        value,
        detail: Detail::Text(outlook),
        plot: day_plot(
            day::series(r, now, |h| h.precipitation),
            Mark::Bars(|_| LINE),
        ),
    }
}

fn air_quality(air: &Air, r: &Reading, now: i64) -> Measure {
    let hour = day::air_now(air, r, now);
    let value = hour.and_then(|h| h.aqi);
    let detail = match value {
        Some(aqi) => {
            let level = day::aqi_level(aqi).0.to_string();
            joined(
                [Some(level), hour.and_then(pollutant).map(String::from)]
                    .into_iter()
                    .flatten()
                    .collect(),
            )
        }
        None => DASH.into(),
    };
    Measure {
        icon: "leaf",
        name: "Air quality".into(),
        value: plain(or_dash(value, whole)),
        detail: Detail::Text(detail),
        plot: day_plot(
            day::air_series(air, r, now, |h| h.aqi),
            Mark::Bars(|aqi| day::aqi_level(aqi).1),
        ),
    }
}

/// The pollutant with the largest sub-index of `hour`; on a tie the first of PM2.5, PM10,
/// NO₂, O₃, SO₂.
fn pollutant(hour: &AirHour) -> Option<&'static str> {
    let all = [
        ("PM2.5", hour.pm2_5),
        ("PM10", hour.pm10),
        ("NO₂", hour.no2),
        ("O₃", hour.o3),
        ("SO₂", hour.so2),
    ];
    let mut best: Option<(&str, f64)> = None;
    for (name, value) in all {
        if let Some(v) = value
            && best.is_none_or(|(_, b)| v > b)
        {
            best = Some((name, v));
        }
    }
    best.map(|(name, _)| name)
}

/// The grains of the pollen at index `pollen` of [`POLLENS`], for a chart.
fn grains(pollen: usize) -> fn(&AirHour) -> Option<f64> {
    match pollen {
        0 => |h| h.pollen[0],
        1 => |h| h.pollen[1],
        2 => |h| h.pollen[2],
        3 => |h| h.pollen[3],
        4 => |h| h.pollen[4],
        _ => |h| h.pollen[5],
    }
}

/// A pollen's name, capitalised: "Birch".
fn pollen_name(pollen: usize) -> String {
    let name = POLLENS[pollen];
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn pollen(air: &Air, r: &Reading, now: i64) -> Measure {
    let day: Vec<Series> = (0..POLLENS.len())
        .map(|i| day::air_series(air, r, now, grains(i)))
        .collect();
    let peak = |s: &Series| s.values.iter().flatten().copied().fold(0., f64::max);
    let strongest = day::strongest_pollen(air, r, now);
    // The chart is of the strongest pollen now, or of the day's when there is none now.
    let charted = strongest.first().map(|(i, _)| *i).or_else(|| {
        (0..day.len())
            .filter(|i| peak(&day[*i]) >= 1.)
            .max_by(|a, b| peak(&day[*a]).total_cmp(&peak(&day[*b])))
    });
    let Some(charted) = charted else {
        return Measure {
            icon: "flower-2",
            name: "Pollen".into(),
            value: plain(DASH),
            detail: Detail::Text("No pollen data here".into()),
            plot: Plot::Nothing,
        };
    };
    let level = |(i, grains): &(usize, f64)| day::pollen_level(*i, *grains).unwrap_or(DASH);
    let others: Vec<String> = strongest
        .iter()
        .skip(1)
        .take(2)
        .map(|p| format!("{} {}", pollen_name(p.0), level(p)))
        .collect();
    Measure {
        icon: "flower-2",
        name: format!("{} pollen", pollen_name(charted)),
        value: plain(strongest.first().map_or(DASH, level)),
        detail: Detail::Text(joined(others)),
        plot: day_plot(day[charted], Mark::Bars(|_| POLLEN)),
    }
}

fn daylight(r: &Reading, now: i64) -> Measure {
    let info = day::daylight(r, now);
    let time =
        |at: Option<i64>| at.map_or_else(|| DASH.to_string(), |at| day::clock(at, r.utc_offset));
    Measure {
        icon: "sunrise",
        name: "Daylight".into(),
        value: plain(or_dash(info.as_ref().map(|i| i.length), day::length)),
        detail: Detail::Sun(
            time(info.as_ref().map(|i| i.sunrise)),
            time(info.as_ref().map(|i| i.sunset)),
            or_dash(info.as_ref().map(|i| i.change), day::signed_minutes),
        ),
        plot: Plot::Week(info.map(|i| i.week).unwrap_or_default()),
    }
}

fn put(frame: FrameBuilder, parent: NodeRef, node: Node) -> FrameBuilder {
    frame.child(parent, node).0
}

fn row(frame: FrameBuilder, parent: NodeRef, gap: f32) -> (FrameBuilder, NodeRef) {
    frame.child(
        parent,
        Node::row(Layout::default().gap(gap).align(Align::Center)),
    )
}

/// The muted icon and the name, then the value, under `col`.
fn head(frame: FrameBuilder, col: NodeRef, m: &Measure, context: &WidgetContext) -> FrameBuilder {
    let (frame, top) = row(frame, col, 6.);
    let frame = ink::icon(frame, top, m.icon, ICON, MUTED, context);
    let frame = put(frame, top, caption(m.name.clone()).grow(1.));
    let frame = put(frame, col, Node::spacer().grow(1.));
    let (text, unit) = &m.value;
    let (frame, at) = frame.child(col, Node::row(Layout::default().gap(3.).align(Align::End)));
    let frame = put(
        frame,
        at,
        Node::text(text.clone(), TextStyle::Value).max_lines(1),
    );
    match unit {
        Some(unit) => put(
            frame,
            at,
            Node::text(*unit, TextStyle::Body)
                .color(Color::token(ColorToken::Muted))
                .max_lines(1),
        ),
        None => frame,
    }
}

/// The chart of `m`, `chart` in size; none for [`Plot::Nothing`].
fn plot(m: &Measure, chart: &Chart, context: &WidgetContext) -> Option<Node> {
    match &m.plot {
        Plot::Day(series, mark) => Some(chart::day_chart(chart, series, *mark, context)),
        Plot::Week(lengths) => Some(chart::week_chart(chart, lengths, context)),
        Plot::Nothing => None,
    }
}

/// The quarter: the header, the value, the day's chart and the place.
fn quarter(m: &Measure, v: &View, r: &Reading, context: &WidgetContext, now: i64) -> FrameBuilder {
    let frame = WidgetFrame::new(Node::column(Layout::default().padding(PADDING).gap(2.)));
    let root = frame.root();
    let frame = head(frame, root, m, context);
    let chart = Chart {
        width: context.width - 2. * PADDING,
        height: QUARTER_CHART,
        marks: false,
    };
    let frame = match (plot(m, &chart, context), &m.detail) {
        (Some(node), _) => put(frame, root, node),
        // Without a chart its note takes its place.
        (None, Detail::Text(note)) => put(frame, root, caption(note.clone())),
        (None, Detail::Sun(..)) => frame,
    };
    put(frame, root, caption(tile::place(v, r, now)))
}

/// The single: the header, the value, the detail and the place on the left; the chart, with
/// the hour marks, on the right.
fn single(m: &Measure, v: &View, r: &Reading, context: &WidgetContext, now: i64) -> FrameBuilder {
    let frame = WidgetFrame::new(Node::row(Layout::default().padding(PADDING).gap(12.)));
    let root = frame.root();
    let left = Node::column(Layout::default().gap(2.));
    // Without a chart (no pollen here) the left column takes the whole tile.
    let left = match m.plot {
        Plot::Nothing => left.grow(1.),
        _ => left.width(LEFT),
    };
    let (frame, left) = frame.child(root, left);
    let frame = head(frame, left, m, context);
    let frame = match &m.detail {
        Detail::Text(text) => put(frame, left, caption(text.clone())),
        // One line, so the column fits the tile.
        Detail::Sun(sunrise, sunset, change) => {
            let (frame, sun) = row(frame, left, 6.);
            let frame = sun_time(frame, sun, "sunrise", sunrise, context);
            let frame = sun_time(frame, sun, "sunset", sunset, context);
            put(frame, sun, caption(change.clone()))
        }
    };
    let frame = put(frame, left, caption(tile::place(v, r, now)));
    let chart = Chart {
        width: (context.width - 2. * PADDING - LEFT - 12.).max(0.),
        height: SINGLE_CHART,
        marks: true,
    };
    match plot(m, &chart, context) {
        Some(node) => put(frame, root, node),
        None => frame,
    }
}

/// A muted sun icon and a time beside it, under `parent`.
fn sun_time(
    frame: FrameBuilder,
    parent: NodeRef,
    icon: &str,
    time: &str,
    context: &WidgetContext,
) -> FrameBuilder {
    let (frame, at) = row(frame, parent, 3.);
    let frame = ink::icon(frame, at, icon, SMALL, MUTED, context);
    put(frame, at, caption(time))
}

#[cfg(test)]
mod tests {
    use tomakast_plugin::{Align, Color, ColorToken, Draw, NodeKind, Paint, TextStyle};

    use super::*;
    use crate::api::{self, Air, AirHour, Reading, Units};
    use crate::ink::{context, shown};
    use crate::location::Status;

    const BERLIN_DAY: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin-day.json");
    const MOSCOW_DAY: &[u8] =
        include_bytes!("../tests/fixtures/forecast-moscow-day-fahrenheit.json");
    const AIR_BERLIN: &[u8] = include_bytes!("../tests/fixtures/air-berlin.json");
    const AIR_NEW_YORK: &[u8] = include_bytes!("../tests/fixtures/air-new-york.json");
    /// Fetched, and the `now` of the tests: the place's now is 2026-11-21 14:15 local.
    const F: i64 = 1_800_000_000;
    /// 2026-11-21 06:00 local in Berlin.
    const START: i64 = 1795237200;
    /// The hour that holds the place's now, 14:00 local.
    const HOUR_NOW: i64 = 1795266000;
    const QUARTER: f32 = 173.5;
    const SINGLE: f32 = 355.;

    fn berlin() -> Reading {
        api::parse_forecast(200, BERLIN_DAY, Units::Metric, F).unwrap()
    }

    fn air(body: &[u8]) -> Air {
        api::parse_air(200, body, F).unwrap()
    }

    fn view<'a>(reading: Option<&'a Reading>, air: Option<&'a Air>) -> View<'a> {
        View {
            name: "Berlin, DE".into(),
            reading,
            status: Status::Ok,
            city: "Berlin".into(),
            air,
            air_status: Status::Ok,
        }
    }

    fn draw_mm(widget: &str, v: &View, width: f32, mmhg: bool) -> WidgetFrame {
        measure(widget, v, &context(width), F, mmhg).unwrap()
    }

    fn draw(widget: &str, v: &View, width: f32) -> Vec<String> {
        shown(&draw_mm(widget, v, width, false))
    }

    /// The bars of the canvas `id` of `frame`, by colour.
    fn bars(frame: &WidgetFrame, id: &str) -> Vec<u32> {
        frame
            .nodes
            .iter()
            .filter(|n| n.id == id)
            .filter_map(|n| match &n.kind {
                NodeKind::Canvas(c) => Some(c),
                _ => None,
            })
            .flat_map(|c| &c.commands)
            .filter_map(|d| match d {
                Draw::Rect(r) => match r.style.paint {
                    Paint::Solid(Color::Rgba(c)) => Some(c),
                    _ => Some(0),
                },
                _ => None,
            })
            .collect()
    }

    /// The index of the text node `text` in `frame`.
    fn text_at(frame: &WidgetFrame, text: &str) -> usize {
        frame
            .nodes
            .iter()
            .position(|n| matches!(&n.kind, NodeKind::Text(t) if t.text == text))
            .unwrap_or_else(|| panic!("no text {text}"))
    }

    /// The parent of the node at `at`.
    fn parent(frame: &WidgetFrame, at: usize) -> usize {
        frame.nodes[at].parent.unwrap() as usize
    }

    /// The number and its unit share a bottom-aligned row; the unit is muted body text.
    fn assert_unit(frame: &WidgetFrame, number: &str, unit: &str) {
        let (n, u) = (text_at(frame, number), text_at(frame, unit));
        assert_eq!(u, n + 1);
        assert_eq!(parent(frame, n), parent(frame, u));
        let NodeKind::Row(layout) = &frame.nodes[parent(frame, n)].kind else {
            panic!("not in a row");
        };
        assert_eq!(layout.align, Align::End);
        let NodeKind::Text(number) = &frame.nodes[n].kind else {
            unreachable!()
        };
        assert_eq!(number.style, TextStyle::Value);
        let NodeKind::Text(unit) = &frame.nodes[u].kind else {
            unreachable!()
        };
        assert_eq!(unit.style, TextStyle::Body);
        assert!(matches!(unit.color, Color::Token(ColorToken::Muted)));
    }

    #[test]
    fn humidity_quarter_and_single() {
        let r = berlin();
        let v = view(Some(&r), None);
        assert_eq!(
            draw("humidity", &v, QUARTER),
            [
                "faint:droplets",
                "Humidity",
                "76",
                "%",
                "chart",
                "Berlin, DE"
            ]
        );
        assert_eq!(
            draw("humidity", &v, SINGLE),
            [
                "faint:droplets",
                "Humidity",
                "76",
                "%",
                "Dew point 3°",
                "Berlin, DE",
                "chart",
                "06",
                "12",
                "18",
                "00"
            ]
        );
        // The single's left column is 150 wide; the chart takes the rest.
        let frame = draw_mm("humidity", &v, SINGLE, false);
        assert_unit(&frame, "76", "%");
        assert!(frame.nodes.iter().any(|n| n.width == Some(150.)));
        let chart = frame.nodes.iter().find(|n| n.id == "chart").unwrap();
        assert_eq!(chart.width, Some(169.));
        // The tile dispatches the measures, with Enter to refresh.
        let frame = crate::tile::tile("humidity", &v, &context(QUARTER), F, false, false).unwrap();
        assert_eq!(shown(&frame)[2..4], ["76", "%"]);
        let actions: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["refresh"]);
        // An old reading says its age, as every tile does.
        let frame = measure("humidity", &v, &context(QUARTER), F + 2 * 3600, false).unwrap();
        assert_eq!(shown(&frame).last().unwrap(), "Berlin, DE · 2 h ago");
    }

    #[test]
    fn wind_says_where_from_and_gusts() {
        let r = berlin();
        let v = view(Some(&r), None);
        let single = draw("wind", &v, SINGLE);
        assert_eq!(
            single[..5],
            ["faint:wind", "Wind", "18", "km/h", "→ W · gusts 43"]
        );
        assert_eq!(draw("wind", &v, QUARTER)[2..4], ["18", "km/h"]);
        assert_unit(&draw_mm("wind", &v, QUARTER, false), "18", "km/h");
        let moscow = api::parse_forecast(200, MOSCOW_DAY, Units::Imperial, F).unwrap();
        assert_eq!(draw("wind", &view(Some(&moscow), None), QUARTER)[3], "mph");
    }

    #[test]
    fn uv_level_and_window() {
        let mut r = berlin();
        let v = view(Some(&r), None);
        assert_eq!(
            draw("uv", &v, SINGLE)[..4],
            ["faint:sun", "UV index", "1", "Low"]
        );
        // Synthetic hours: 6.4 from 11:00, so at the current hour, until 15:00.
        let strong = START + 5 * 3600..START + 9 * 3600;
        for hour in &mut r.hours {
            hour.uv = Some(if strong.contains(&hour.at) { 6.4 } else { 0.5 });
        }
        let v = view(Some(&r), None);
        let frame = draw_mm("uv", &v, SINGLE, false);
        assert_eq!(shown(&frame)[2..4], ["6", "High · till 15:00"]);
        let expected: Vec<u32> = (0..24)
            .map(|i| {
                if (5..9).contains(&i) {
                    0xF29A3BFF
                } else {
                    0x6FCF6FFF
                }
            })
            .collect();
        assert_eq!(bars(&frame, "chart"), expected);
    }

    #[test]
    fn pressure_in_hpa_or_mm_hg() {
        let r = berlin();
        let v = view(Some(&r), None);
        assert_eq!(
            draw("pressure", &v, SINGLE)[..5],
            ["faint:gauge", "Pressure", "1013", "hPa", "→ steady"]
        );
        assert_unit(&draw_mm("pressure", &v, SINGLE, false), "1013", "hPa");
        let frame = draw_mm("pressure", &v, QUARTER, true);
        assert_eq!(shown(&frame)[2..4], ["760", "mm Hg"]);
        assert_unit(&frame, "760", "mm Hg");
    }

    #[test]
    fn precipitation_now_and_outlook() {
        let mut r = berlin();
        let v = view(Some(&r), None);
        let frame = draw_mm("precipitation", &v, SINGLE, false);
        assert_eq!(
            shown(&frame)[..5],
            ["faint:umbrella", "Rain", "0", "mm", "40% after 18:00"]
        );
        let blue = bars(&frame, "chart");
        assert!(
            !blue.is_empty() && blue.iter().all(|c| *c == 0x6EA8FEFF),
            "{blue:?}"
        );
        let hour = r.hours.iter_mut().find(|h| h.at == HOUR_NOW).unwrap();
        hour.snowfall = Some(1.5);
        assert_eq!(
            draw("precipitation", &view(Some(&r), None), QUARTER)[2..4],
            ["1.5", "cm"]
        );
        // Snow too little to show at one decimal leaves the rain.
        let hour = r.hours.iter_mut().find(|h| h.at == HOUR_NOW).unwrap();
        hour.snowfall = Some(0.03);
        hour.precipitation = Some(2.);
        assert_eq!(
            draw("precipitation", &view(Some(&r), None), QUARTER)[2..4],
            ["2", "mm"]
        );
        let hour = r.hours.iter_mut().find(|h| h.at == HOUR_NOW).unwrap();
        hour.snowfall = Some(0.);
        hour.precipitation = Some(2.);
        assert_eq!(
            draw("precipitation", &view(Some(&r), None), QUARTER)[2..4],
            ["2", "mm"]
        );
        let hour = r.hours.iter_mut().find(|h| h.at == HOUR_NOW).unwrap();
        hour.precipitation = Some(0.25);
        assert_eq!(
            draw("precipitation", &view(Some(&r), None), QUARTER)[2..4],
            ["0.3", "mm"]
        );
        // No chance of rain anywhere: it says so calmly.
        for hour in &mut r.hours {
            hour.rain_chance = Some(10.);
        }
        assert_eq!(
            draw("precipitation", &view(Some(&r), None), SINGLE)[4],
            "No rain ahead"
        );
    }

    #[test]
    fn air_quality_level_and_pollutant() {
        let r = berlin();
        let a = air(AIR_BERLIN);
        let v = view(Some(&r), Some(&a));
        let frame = draw_mm("air-quality", &v, SINGLE, false);
        assert_eq!(
            shown(&frame)[..4],
            ["faint:leaf", "Air quality", "19", "Good · PM2.5"]
        );
        let colours = bars(&frame, "chart");
        assert!(colours.contains(&0x50F0E6FF), "{colours:?}");
        assert!(needs_air("air-quality") && needs_air("pollen") && !needs_air("humidity"));
        // Without air: calm, loading, or offline.
        let v = view(Some(&r), None);
        assert_eq!(
            draw("air-quality", &v, QUARTER),
            ["icon:loader", "Loading", "—", "Berlin, DE"]
        );
        let failed = View {
            air_status: Status::Failed,
            ..view(Some(&r), None)
        };
        for width in [QUARTER, SINGLE] {
            assert_eq!(
                draw("air-quality", &failed, width),
                ["icon:cloud-off", "No air data", "—", "Offline, will retry"]
            );
        }
    }

    #[test]
    fn pollen_says_calmly_when_there_is_none() {
        let r = berlin();
        for body in [AIR_BERLIN, AIR_NEW_YORK] {
            let a = air(body);
            let v = view(Some(&r), Some(&a));
            for width in [QUARTER, SINGLE] {
                let shown = draw("pollen", &v, width);
                assert_eq!(shown[..3], ["faint:flower-2", "Pollen", "—"]);
                assert!(
                    shown.contains(&"No pollen data here".to_string()),
                    "{shown:?}"
                );
                assert!(!shown.contains(&"chart".to_string()), "{shown:?}");
                // With no chart beside it, the note has the whole tile.
                let frame = draw_mm("pollen", &v, width, false);
                assert!(frame.nodes.iter().all(|n| n.width != Some(LEFT)));
            }
        }
        // Synthetic, hand-written in-season air (no fixture has pollen): birch 60, grass 30,
        // alder 5 at the current hour, birch 20 the hour before.
        let a = Air {
            hours: vec![
                AirHour {
                    at: HOUR_NOW - 3600,
                    pollen: [Some(0.), Some(20.), Some(10.), None, None, None],
                    ..AirHour::default()
                },
                AirHour {
                    at: HOUR_NOW,
                    pollen: [Some(5.), Some(60.), Some(30.), None, None, None],
                    ..AirHour::default()
                },
            ],
            utc_offset: 3600,
            fetched: F,
        };
        let v = view(Some(&r), Some(&a));
        let frame = draw_mm("pollen", &v, SINGLE, false);
        assert_eq!(
            shown(&frame)[..5],
            [
                "faint:flower-2",
                "Birch pollen",
                "High",
                "Grass High · Alder Low",
                "Berlin, DE"
            ]
        );
        assert_eq!(bars(&frame, "chart"), [0xA6D96AFF, 0xA6D96AFF]);
        assert_eq!(draw("pollen", &v, QUARTER)[1..3], ["Birch pollen", "High"]);
    }

    #[test]
    fn daylight_length_sun_and_week() {
        let r = berlin();
        let v = view(Some(&r), None);
        let quarter = draw_mm("daylight", &v, QUARTER, false);
        assert_eq!(
            shown(&quarter)[..3],
            ["faint:sunrise", "Daylight", "8 h 27 min"]
        );
        assert_eq!(bars(&quarter, "week").len(), 7);
        let frame = draw_mm("daylight", &v, SINGLE, false);
        let shown = shown(&frame);
        assert_eq!(shown[2], "8 h 27 min");
        let sun = [
            "faint:sunrise",
            "07:38",
            "faint:sunset",
            "16:05",
            "\u{2212}3 min",
        ];
        assert_eq!(shown[3..8], sun, "{shown:?}");
        // The change is on the sun row, so the column fits the tile.
        assert_eq!(
            parent(&frame, parent(&frame, text_at(&frame, "07:38"))),
            parent(&frame, text_at(&frame, "\u{2212}3 min"))
        );
        assert_eq!(bars(&frame, "week").len(), 7);
        // Without the next day's length the length is not told, and nothing panics.
        let mut short = berlin();
        // The days are yesterday and today, so no next day.
        short.days.truncate(2);
        let shown = draw("daylight", &view(Some(&short), None), SINGLE);
        assert_eq!(shown[2], "—", "{shown:?}");
    }

    #[test]
    fn every_measure_in_both_sizes_without_hours() {
        let mut r = berlin();
        r.hours.clear();
        r.days.clear();
        let a = Air {
            hours: Vec::new(),
            utc_offset: 3600,
            fetched: F,
        };
        let v = view(Some(&r), Some(&a));
        for widget in MEASURES {
            for width in [QUARTER, SINGLE] {
                let frame = draw_mm(widget, &v, width, false);
                assert_eq!(shown(&frame)[2], "—", "{widget} {width}");
                assert!(frame.nodes.len() < 256, "{widget}: {}", frame.nodes.len());
            }
        }
        // Without a forecast every measure is calm.
        for widget in MEASURES {
            assert_eq!(
                draw(widget, &view(None, None), QUARTER),
                ["icon:loader", "Loading", "—", "Berlin, DE"]
            );
        }
        assert!(measure("nope", &v, &context(QUARTER), F, false).is_err());
        // Only the hours empty, the days and the air real: every measure still draws.
        let mut r = berlin();
        r.hours.clear();
        let a = air(AIR_BERLIN);
        let v = view(Some(&r), Some(&a));
        for widget in MEASURES {
            for width in [QUARTER, SINGLE] {
                let frame = draw_mm(widget, &v, width, false);
                assert!(frame.nodes.len() < 256, "{widget}: {}", frame.nodes.len());
                let reads_hours = !needs_air(widget) && widget != "daylight";
                if reads_hours {
                    assert_eq!(shown(&frame)[2], "—", "{widget} {width}");
                }
            }
        }
    }
}
