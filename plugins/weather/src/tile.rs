//! The tiles, drawn from a [`View`]: pure, so they are tested natively. Every widget takes the
//! param `city` (a city typed in the search, or one on Home; see [`search`](crate::search));
//! without it they show where the computer is.
//!
//! - `now` as a quarter: the condition's icon and word, the temperature large, and the place.
//! - `now` as a single: the same on the left; on the right today's high and low (↑ ↓), then
//!   the feel, the wind and the humidity, each after its icon.
//! - `today`: the four parts of the day, in [`today`](crate::today).
//! - The eight measures (humidity, wind, UV, pressure, precipitation, air quality, pollen,
//!   daylight): a value now and a chart of the day, in [`measures`](crate::measures).
//!
//! Without a reading a tile says why, calmly, in the same place: loading, no city set, a city
//! geocoding does not know, or no connection. With a reading and a failed refresh every tile
//! in every size keeps the reading and says how old it is.

use tomakast_plugin::{
    Action, Align, Color, ColorToken, Icon, Justify, Layout, Node, NodeRef, TextAlign, TextStyle,
    WidgetFrame,
};
use tomakast_plugin::{FrameBuilder, WidgetContext};

use crate::api::{Air, Reading, Units};
use crate::location::Status;
use crate::measures::{self, MEASURES};

pub const NOW: &str = "now";
pub const TODAY: &str = "today";
/// The param every widget takes: the place a tile shows.
pub const CITY: &str = "city";

/// What a tile shows.
#[derive(Debug, Clone, PartialEq)]
pub struct View<'a> {
    /// The place's name, or the city as typed while it is looked up.
    pub name: String,
    pub reading: Option<&'a Reading>,
    pub status: Status,
    /// The city as the user typed it.
    pub city: String,
    /// The place's air, for the air quality and pollen tiles.
    pub air: Option<&'a Air>,
    /// How the place's air is fetched, as `status` is for the forecast.
    pub air_status: Status,
}

/// The tile's padding.
pub(crate) const PADDING: f32 = 12.;
const ICON: f32 = 20.;
/// A reading older than this shows its age on the quarter tile too.
const OLD_SECS: i64 = 30 * 60;

/// The Lucide icon and the words for WMO weather code `code`, by day or night.
pub fn condition(code: u16, day: bool) -> (&'static str, &'static str) {
    let lit = |sun, moon| if day { sun } else { moon };
    match code {
        0 => (lit("sun", "moon"), "Clear"),
        1 => (lit("sun", "moon"), "Mostly clear"),
        2 => (lit("cloud-sun", "cloud-moon"), "Partly cloudy"),
        3 => ("cloud", "Overcast"),
        45 | 48 => ("cloud-fog", "Fog"),
        51..=57 => ("cloud-drizzle", "Drizzle"),
        61..=67 => ("cloud-rain", "Rain"),
        71..=77 => ("cloud-snow", "Snow"),
        80..=82 => (lit("cloud-sun-rain", "cloud-moon-rain"), "Showers"),
        85 | 86 => ("cloud-snow", "Snow showers"),
        95 => ("cloud-lightning", "Thunderstorm"),
        96 | 99 => ("cloud-hail", "Thunderstorm, hail"),
        _ => ("cloud", "Cloudy"),
    }
}

/// `value` rounded, with a true minus sign and no `-0`, and a degree sign.
pub fn degrees(value: f64) -> String {
    let rounded = value.round();
    if rounded < 0. {
        format!("\u{2212}{}°", -rounded)
    } else {
        format!("{}°", rounded.abs())
    }
}

/// How long ago `fetched` was at `now`, in words.
pub fn age(fetched: i64, now: i64) -> String {
    let secs = now - fetched;
    if secs < 120 {
        "just now".into()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 48 * 3600 {
        format!("{} h ago", secs / 3600)
    } else {
        format!("{} days ago", secs / 86_400)
    }
}

pub(crate) fn wind_unit(units: Units) -> &'static str {
    units.wind.label()
}

pub(crate) fn caption(text: impl Into<String>) -> Node {
    Node::text(text, TextStyle::Caption)
        .color(Color::token(ColorToken::Muted))
        .max_lines(1)
}

/// What a tile without a reading says: an icon, a word and a caption.
pub(crate) fn calm(v: &View) -> (&'static str, &'static str, String) {
    match v.status {
        Status::NoLocation => (
            "map-pin-off",
            "No location",
            "Set a city in launcher settings".into(),
        ),
        Status::Unknown => ("map-pin-off", "Unknown city", v.city.clone()),
        Status::Failed => ("cloud-off", "No weather", "Offline, will retry".into()),
        Status::Ok => ("loader", "Loading", v.name.clone()),
    }
}

/// The weather of `v` in one line, for the clipboard: `Berlin, DE: 3°, Clear, H 5° L 2°`.
pub fn summary(v: &View) -> Option<String> {
    let r = v.reading?;
    let mut text = format!(
        "{}: {}, {}",
        v.name,
        degrees(r.temperature),
        condition(r.code, r.is_day).1
    );
    if let (Some(high), Some(low)) = (r.high, r.low) {
        text.push_str(&format!(", H {} L {}", degrees(high), degrees(low)));
    }
    Some(text)
}

/// The left part of both tiles, in `column`: the icon and word in a row, then the
/// temperature, then `under`.
fn glance(
    frame: FrameBuilder,
    column: NodeRef,
    icon: &str,
    word: &str,
    value: &str,
    under: String,
) -> FrameBuilder {
    let (frame, top) = frame.child(
        column,
        Node::row(Layout::default().gap(6.).align(Align::Center)),
    );
    let (frame, _) = frame.child(top, Node::icon(Icon::Named(icon.into()), ICON));
    let (frame, _) = frame.child(top, caption(word).grow(1.));
    let (frame, _) = frame.child(column, Node::spacer().grow(1.));
    let (frame, _) = frame.child(column, Node::text(value, TextStyle::Value).max_lines(1));
    frame.child(column, caption(under)).0
}

fn column() -> Node {
    Node::column(Layout::default().gap(2.)).grow(1.)
}

/// The frame's own actions: Enter refreshes.
pub(crate) fn with_actions(frame: FrameBuilder) -> Result<WidgetFrame, String> {
    Ok(frame.action("refresh", "Refresh")?.build())
}

/// Whether `context` is a single's or wider. The render context gives no size, only the
/// tile's width and height, and the strip's width is fixed: a quarter is 173.5 x 120 (about
/// 1.4 times as wide as high), a single 355 x 120 (about 3 times), so twice the height
/// tells them apart.
pub fn is_wide(context: &WidgetContext) -> bool {
    context.width >= context.height * 2.
}

/// The tile of `widget` showing `v` at `now` (unix seconds): `now` as a quarter or a single,
/// `today` in its own sizes (see [`today`](crate::today)), and each id of [`MEASURES`] as a
/// quarter or a single (see [`measures`](crate::measures)), its pressure in mm Hg with `mmhg`.
/// Its actions: Enter refreshes, and a tile of a city (`copy`) also offers to copy its
/// weather, as its search tile does.
pub fn tile(
    widget: &str,
    v: &View,
    context: &WidgetContext,
    now: i64,
    copy: bool,
    mmhg: bool,
) -> Result<WidgetFrame, String> {
    let mut frame = match widget {
        NOW if !is_wide(context) => quarter(v, context, now),
        NOW => single(v, context, now),
        TODAY => crate::today::today(v, context, now),
        _ if MEASURES.contains(&widget) => measures::measure(widget, v, context, now, mmhg),
        _ => Err(format!("no widget {widget}")),
    }?;
    if copy {
        frame.actions.push(Action {
            id: "copy".into(),
            title: "Copy Weather".into(),
            shortcut: None,
        });
    }
    Ok(frame)
}

/// A quarter tile of `v` at `now` (unix seconds).
pub fn quarter(v: &View, _context: &WidgetContext, now: i64) -> Result<WidgetFrame, String> {
    let frame = WidgetFrame::new(Node::column(Layout::default().padding(PADDING)));
    let root = frame.root();
    let (frame, col) = frame.child(root, column());
    let frame = match v.reading {
        Some(r) => {
            let (icon, word) = condition(r.code, r.is_day);
            glance(
                frame,
                col,
                icon,
                word,
                &degrees(r.temperature),
                place(v, r, now),
            )
        }
        None => {
            let (icon, word, under) = calm(v);
            return calm_tile(icon, word, under);
        }
    };
    with_actions(frame)
}

/// A quarter that says calmly why there is nothing to show: `icon` and `word` in a row, a
/// dash, and `under`. Every size of every tile without its data draws this.
pub(crate) fn calm_tile(icon: &str, word: &str, under: String) -> Result<WidgetFrame, String> {
    let frame = WidgetFrame::new(Node::column(Layout::default().padding(PADDING)));
    let root = frame.root();
    let (frame, col) = frame.child(root, column());
    with_actions(glance(frame, col, icon, word, "—", under))
}

/// The place line of a quarter: the place's name, and the reading's age when it is old or
/// the last refresh failed.
pub(crate) fn place(v: &View, r: &Reading, now: i64) -> String {
    if now - r.fetched >= OLD_SECS || v.status == Status::Failed {
        format!("{} · {}", v.name, age(r.fetched, now))
    } else {
        v.name.clone()
    }
}

/// A single tile of `v` at `now` (unix seconds).
pub fn single(v: &View, context: &WidgetContext, now: i64) -> Result<WidgetFrame, String> {
    let frame = WidgetFrame::new(Node::row(Layout::default().padding(PADDING).gap(12.)));
    let root = frame.root();
    let (frame, left) = frame.child(root, column());
    let Some(r) = v.reading else {
        let (icon, word, under) = calm(v);
        return with_actions(glance(frame, left, icon, word, "—", under));
    };
    let (icon, word) = condition(r.code, r.is_day);
    let frame = glance(
        frame,
        left,
        icon,
        word,
        &degrees(r.temperature),
        place(v, r, now),
    );
    // The details stand from the top of the tile to its bottom, level with the left column.
    let (mut frame, right) = frame.child(
        root,
        Node::column(
            Layout::default()
                .align(Align::End)
                .justify(Justify::SpaceBetween),
        )
        .grow(1.),
    );
    if let (Some(high), Some(low)) = (r.high, r.low) {
        let range = format!("↑ {}  ↓ {}", degrees(high), degrees(low));
        frame = frame.child(right, detail_text(range)).0;
    }
    let details = [
        ("thermometer", r.feels_like.map(degrees)),
        (
            "wind",
            r.wind
                .map(|w| format!("{} {}", w.round(), wind_unit(r.units))),
        ),
        ("droplets", r.humidity.map(|h| format!("{}%", h.round()))),
    ];
    for (icon, text) in details {
        let Some(text) = text else { continue };
        let (f, row) = frame.child(
            right,
            Node::row(Layout::default().gap(6.).align(Align::Center)),
        );
        let f = crate::ink::icon(f, row, icon, DETAIL_ICON, crate::ink::MUTED, context);
        frame = f.child(row, detail_text(text)).0;
    }
    with_actions(frame)
}

/// The size of the icons of the single's details.
const DETAIL_ICON: f32 = 14.;

/// A value of the single's details: body text, one line, at the right.
fn detail_text(text: String) -> Node {
    Node::text(text, TextStyle::Body)
        .max_lines(1)
        .align(TextAlign::End)
}

/// The age line of the wider tiles.
pub(crate) fn updated(v: &View, r: &Reading, now: i64) -> String {
    let age = age(r.fetched, now);
    if v.status == Status::Failed {
        format!("Offline · {age}")
    } else {
        format!("Updated {age}")
    }
}

#[cfg(test)]
mod tests {
    use tomakast_plugin::NodeKind;

    use super::*;
    use crate::api;
    use crate::ink::{context, shown};

    const FORECAST: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin.json");

    /// The fixture's reading, fetched at `at`.
    fn reading(at: i64) -> Reading {
        api::parse_forecast(200, FORECAST, Units::default(), at).unwrap()
    }

    /// Berlin (as geocoding names it) with `reading`, in `status`.
    fn berlin(reading: Option<&Reading>, status: Status) -> View<'_> {
        View {
            name: "Berlin".into(),
            reading,
            status,
            city: "Berlin".into(),
            air: None,
            air_status: Status::Ok,
        }
    }

    #[test]
    fn the_summary_is_one_line() {
        let r = reading(0);
        assert_eq!(
            summary(&berlin(Some(&r), Status::Ok)).unwrap(),
            "Berlin: 3°, Clear, H 5° L 2°"
        );
        assert_eq!(summary(&berlin(None, Status::Ok)), None);
    }

    #[test]
    fn degrees_round_with_a_true_minus() {
        assert_eq!(degrees(2.6), "3°");
        assert_eq!(degrees(-0.4), "0°");
        assert_eq!(degrees(-0.5), "\u{2212}1°");
        assert_eq!(degrees(-12.2), "\u{2212}12°");
    }

    #[test]
    fn ages_read_as_words() {
        assert_eq!(age(0, 30), "just now");
        assert_eq!(age(0, 600), "10 min ago");
        assert_eq!(age(0, 7200), "2 h ago");
        assert_eq!(age(0, 3 * 86_400), "3 days ago");
    }

    #[test]
    fn every_code_has_an_icon_and_words() {
        for code in 0..=99 {
            for day in [true, false] {
                let (icon, word) = condition(code, day);
                assert!(!icon.is_empty() && !word.is_empty());
            }
        }
        assert_eq!(condition(0, false), ("moon", "Clear"));
        assert_eq!(condition(63, true), ("cloud-rain", "Rain"));
    }

    #[test]
    fn the_quarter_shows_the_condition_the_temperature_and_the_place() {
        let r = reading(0);
        let v = berlin(Some(&r), Status::Ok);
        let frame = quarter(&v, &context(173.5), 60).unwrap();
        assert_eq!(shown(&frame), ["icon:moon", "Clear", "3°", "Berlin"]);
        let actions: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["refresh"]);
        // An old reading says its age.
        let frame = quarter(&v, &context(173.5), 2 * 3600).unwrap();
        assert_eq!(shown(&frame)[3], "Berlin · 2 h ago");
    }

    #[test]
    fn now_in_a_single_draws_the_day() {
        let r = reading(0);
        let v = berlin(Some(&r), Status::Ok);
        let quarter_now = shown(&tile(NOW, &v, &context(173.5), 300, false, false).unwrap());
        assert_eq!(quarter_now.len(), 4, "{quarter_now:?}");
        let single_now = shown(&tile(NOW, &v, &context(355.), 300, false, false).unwrap());
        assert_eq!(single_now.len(), 11, "{single_now:?}");
        assert_eq!(single_now[..4], ["icon:moon", "Clear", "3°", "Berlin"]);
        assert!(tile("city", &v, &context(173.5), 0, false, false).is_err());
    }

    #[test]
    fn a_city_tile_also_copies() {
        let r = reading(0);
        let v = berlin(Some(&r), Status::Ok);
        let ids = |copy| -> Vec<String> {
            let frame = tile(NOW, &v, &context(173.5), 0, copy, false).unwrap();
            frame.actions.iter().map(|a| a.id.clone()).collect()
        };
        assert_eq!(ids(false), ["refresh"]);
        assert_eq!(ids(true), ["refresh", "copy"]);
    }

    #[test]
    fn the_single_adds_the_day_and_the_age() {
        let r = reading(0);
        let frame = single(&berlin(Some(&r), Status::Ok), &context(355.), 300).unwrap();
        assert_eq!(
            shown(&frame),
            [
                "icon:moon",
                "Clear",
                "3°",
                "Berlin",
                "↑ 5°  ↓ 2°",
                "faint:thermometer",
                "\u{2212}1°",
                "faint:wind",
                "6 km/h",
                "faint:droplets",
                "81%",
            ]
        );
        // The details stand from the top of the tile to its bottom, at the right.
        let column = frame
            .nodes
            .iter()
            .find_map(|n| match &n.kind {
                NodeKind::Column(l) if l.justify == Justify::SpaceBetween => Some(l.align),
                _ => None,
            })
            .expect("a column of details");
        assert_eq!(column, Align::End);
    }

    #[test]
    fn a_failed_refresh_keeps_the_reading_and_says_so() {
        let r = reading(0);
        let v = berlin(Some(&r), Status::Failed);
        let quarter = shown(&quarter(&v, &context(173.5), 1000).unwrap());
        assert_eq!(
            quarter[2..],
            ["3°".to_string(), "Berlin · 16 min ago".into()]
        );
        let single = shown(&single(&v, &context(355.), 1000).unwrap());
        assert_eq!(single[3], "Berlin · 16 min ago");
    }

    #[test]
    fn without_a_reading_the_tiles_stay_calm() {
        let calm = |v: &View| shown(&quarter(v, &context(173.5), 0).unwrap());
        assert_eq!(
            calm(&berlin(None, Status::Ok)),
            ["icon:loader", "Loading", "—", "Berlin"]
        );
        let here = View {
            name: "Your location".into(),
            city: String::new(),
            ..berlin(None, Status::Ok)
        };
        assert_eq!(
            calm(&here),
            ["icon:loader", "Loading", "—", "Your location"]
        );
        let nowhere = View {
            status: Status::NoLocation,
            ..here
        };
        assert_eq!(
            calm(&nowhere),
            [
                "icon:map-pin-off",
                "No location",
                "—",
                "Set a city in launcher settings"
            ]
        );
        assert_eq!(
            calm(&berlin(None, Status::Failed)),
            ["icon:cloud-off", "No weather", "—", "Offline, will retry"]
        );
        let unknown = View {
            name: "Xyzzyqqplonk".into(),
            city: "Xyzzyqqplonk".into(),
            ..berlin(None, Status::Unknown)
        };
        assert_eq!(
            calm(&unknown),
            ["icon:map-pin-off", "Unknown city", "—", "Xyzzyqqplonk"]
        );
        let single = shown(&single(&unknown, &context(355.), 0).unwrap());
        assert_eq!(single, calm(&unknown));
    }
}
