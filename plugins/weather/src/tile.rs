//! The tiles, drawn from a [`View`]: pure, so they are tested natively.
//!
//! - `now`, a quarter: the condition's icon and word, the temperature large, and the place.
//! - `today`, a single: the same on the left; today's high and low, the feel, the wind, the
//!   humidity and the age of the reading on the right.
//! - `city`, a quarter like `now`: the condition tile of a city typed in the search (see
//!   [`search`](crate::search)).
//!
//! Without a reading a tile says why, calmly, in the same place: loading, no city set, a city
//! geocoding does not know, or no connection. With a reading and a failed refresh it keeps
//! the reading and says how old it is.

use tomakast_plugin::{
    Align, Color, ColorToken, Icon, Justify, Layout, Node, NodeRef, TextAlign, TextStyle,
    WidgetFrame,
};
use tomakast_plugin::{FrameBuilder, WidgetContext};

use crate::api::{Reading, Units};
use crate::state::{Status, Weather};

pub const NOW: &str = "now";
pub const TODAY: &str = "today";
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
}

impl Weather {
    /// What the `now` and `today` tiles show.
    pub fn view(&self) -> View<'_> {
        View {
            name: match &self.place {
                Some(place) => place.name.clone(),
                None if self.settings.auto() => "Your location".into(),
                None => self.settings.city.clone(),
            },
            reading: self.reading.as_ref(),
            status: self.status(),
            city: self.settings.city.clone(),
        }
    }
}

/// The tile's padding.
const PADDING: f32 = 12.;
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

fn wind_unit(units: Units) -> &'static str {
    match units {
        Units::Metric => "km/h",
        Units::Imperial => "mph",
    }
}

fn caption(text: impl Into<String>) -> Node {
    Node::text(text, TextStyle::Caption)
        .color(Color::token(ColorToken::Muted))
        .max_lines(1)
}

/// What a tile without a reading says: an icon, a word and a caption.
fn calm(v: &View) -> (&'static str, &'static str, String) {
    match v.status {
        Status::NoLocation => (
            "map-pin-off",
            "No location",
            "Set a city in preferences".into(),
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
fn with_actions(frame: FrameBuilder) -> Result<WidgetFrame, String> {
    Ok(frame.action("refresh", "Refresh")?.build())
}

/// The quarter tile `now` at `now` (unix seconds).
pub fn now_tile(w: &Weather, context: &WidgetContext, now: i64) -> Result<WidgetFrame, String> {
    quarter(&w.view(), context, now)
}

/// A quarter tile of `v` at `now` (unix seconds): `now`, or a searched city's.
pub fn quarter(v: &View, _context: &WidgetContext, now: i64) -> Result<WidgetFrame, String> {
    let frame = WidgetFrame::new(Node::column(Layout::default().padding(PADDING)));
    let root = frame.root();
    let (frame, col) = frame.child(root, column());
    let frame = match v.reading {
        Some(r) => {
            let (icon, word) = condition(r.code, r.is_day);
            let old = now - r.fetched >= OLD_SECS || v.status == Status::Failed;
            let under = if old {
                format!("{} · {}", v.name, age(r.fetched, now))
            } else {
                v.name.clone()
            };
            glance(frame, col, icon, word, &degrees(r.temperature), under)
        }
        None => {
            let (icon, word, under) = calm(v);
            glance(frame, col, icon, word, "—", under)
        }
    };
    with_actions(frame)
}

/// The single tile `today` at `now` (unix seconds).
pub fn today_tile(w: &Weather, _context: &WidgetContext, now: i64) -> Result<WidgetFrame, String> {
    let v = w.view();
    let frame = WidgetFrame::new(Node::row(Layout::default().padding(PADDING).gap(12.)));
    let root = frame.root();
    let (frame, left) = frame.child(root, column());
    let Some(r) = v.reading else {
        let (icon, word, under) = calm(&v);
        return with_actions(glance(frame, left, icon, word, "—", under));
    };
    let (icon, word) = condition(r.code, r.is_day);
    let frame = glance(
        frame,
        left,
        icon,
        word,
        &degrees(r.temperature),
        v.name.clone(),
    );
    let (frame, right) = frame.child(
        root,
        Node::column(
            Layout::default()
                .gap(2.)
                .align(Align::End)
                .justify(Justify::End),
        )
        .grow(1.),
    );
    let mut lines = Vec::new();
    if let (Some(high), Some(low)) = (r.high, r.low) {
        lines.push(format!("H {}  L {}", degrees(high), degrees(low)));
    }
    if let Some(feels) = r.feels_like {
        lines.push(format!("Feels like {}", degrees(feels)));
    }
    if let Some(wind) = r.wind {
        lines.push(format!("Wind {} {}", wind.round(), wind_unit(r.units)));
    }
    if let Some(humidity) = r.humidity {
        lines.push(format!("Humidity {}%", humidity.round()));
    }
    lines.push(updated(&v, r, now));
    let mut frame = frame;
    for line in lines {
        frame = frame.child(right, caption(line).align(TextAlign::End)).0;
    }
    with_actions(frame)
}

/// The age line of the `today` tile.
fn updated(v: &View, r: &Reading, now: i64) -> String {
    let age = age(r.fetched, now);
    if v.status == Status::Failed {
        format!("Offline · {age}")
    } else {
        format!("Updated {age}")
    }
}

#[cfg(test)]
mod tests {
    use tomakast_plugin::{NodeKind, TextSizes, ThemeColors, WidgetTheme};

    use super::*;
    use crate::state::Settings;

    const FORECAST: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin.json");
    const BERLIN: &[u8] = include_bytes!("../tests/fixtures/geocoding-berlin.json");
    const MISS: &[u8] = include_bytes!("../tests/fixtures/geocoding-miss.json");

    fn context(width: f32) -> WidgetContext {
        WidgetContext {
            width,
            height: 120.,
            scale: 1.,
            theme: WidgetTheme {
                dark: true,
                colors: ThemeColors {
                    text: 0,
                    muted: 0,
                    panel: 0,
                    border: 0,
                    selected: 0,
                    error: 0,
                    success: 0,
                },
                ui_font: "Inter".into(),
                mono_font: "Mono".into(),
                sizes: TextSizes {
                    title: 16.,
                    value: 28.,
                    body: 14.,
                    caption: 12.,
                },
            },
            millis_shown: 0,
            focused: None,
            pointer: None,
            query: None,
            payload: None,
        }
    }

    fn weather(city: &str) -> Weather {
        let mut w = Weather::default();
        w.set_settings(Settings {
            city: city.into(),
            units: Units::Metric,
        });
        w
    }

    /// Berlin, geocoded and with the fixture's reading, fetched at `at`.
    fn berlin(at: i64) -> Weather {
        let mut w = weather("Berlin");
        let fetch = w.due(at, false).unwrap();
        w.started(&fetch, 1);
        w.answered(1, Ok((200, BERLIN)), at);
        let fetch = w.due(at, false).unwrap();
        w.started(&fetch, 2);
        w.answered(2, Ok((200, FORECAST)), at);
        w
    }

    /// The texts and icon names of `frame`, in order.
    fn shown(frame: &WidgetFrame) -> Vec<String> {
        frame
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Text(t) => Some(t.text.clone()),
                NodeKind::Icon(i) => match &i.icon {
                    Icon::Named(name) => Some(format!("icon:{name}")),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_summary_is_one_line() {
        let w = berlin(0);
        assert_eq!(summary(&w.view()).unwrap(), "Berlin: 3°, Clear, H 5° L 2°");
        assert_eq!(summary(&weather("Berlin").view()), None);
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
        let w = berlin(0);
        let frame = now_tile(&w, &context(173.5), 60).unwrap();
        assert_eq!(shown(&frame), ["icon:moon", "Clear", "3°", "Berlin"]);
        let actions: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["refresh"]);
        // An old reading says its age.
        let frame = now_tile(&w, &context(173.5), 2 * 3600).unwrap();
        assert_eq!(shown(&frame)[3], "Berlin · 2 h ago");
    }

    #[test]
    fn the_single_adds_the_day_and_the_age() {
        let w = berlin(0);
        let frame = today_tile(&w, &context(355.), 300).unwrap();
        assert_eq!(
            shown(&frame),
            [
                "icon:moon",
                "Clear",
                "3°",
                "Berlin",
                "H 5°  L 2°",
                "Feels like \u{2212}1°",
                "Wind 6 km/h",
                "Humidity 81%",
                "Updated 5 min ago",
            ]
        );
    }

    #[test]
    fn a_failed_refresh_keeps_the_reading_and_says_so() {
        let mut w = berlin(0);
        let fetch = w.due(1000, false).unwrap();
        w.started(&fetch, 3);
        w.answered(3, Err("offline".into()), 1000);
        let quarter = shown(&now_tile(&w, &context(173.5), 1000).unwrap());
        assert_eq!(
            quarter[2..],
            ["3°".to_string(), "Berlin · 16 min ago".into()]
        );
        let single = shown(&today_tile(&w, &context(355.), 1000).unwrap());
        assert_eq!(single.last().unwrap(), "Offline · 16 min ago");
    }

    #[test]
    fn without_a_reading_the_tiles_stay_calm() {
        let calm = |w: &Weather| shown(&now_tile(w, &context(173.5), 0).unwrap());
        let mut w = weather("Berlin");
        assert_eq!(calm(&w), ["icon:loader", "Loading", "—", "Berlin"]);
        assert_eq!(
            calm(&weather("")),
            ["icon:loader", "Loading", "—", "Your location"]
        );
        let mut nowhere = weather("auto");
        nowhere.located(Err("off".into()));
        assert_eq!(
            calm(&nowhere),
            [
                "icon:map-pin-off",
                "No location",
                "—",
                "Set a city in preferences"
            ]
        );
        let fetch = w.due(0, false).unwrap();
        w.started(&fetch, 1);
        w.answered(1, Err("timed out".into()), 0);
        assert_eq!(
            calm(&w),
            ["icon:cloud-off", "No weather", "—", "Offline, will retry"]
        );
        let mut unknown = weather("Xyzzyqqplonk");
        let fetch = unknown.due(0, false).unwrap();
        unknown.started(&fetch, 1);
        unknown.answered(1, Ok((200, MISS)), 0);
        assert_eq!(
            calm(&unknown),
            ["icon:map-pin-off", "Unknown city", "—", "Xyzzyqqplonk"]
        );
        let single = shown(&today_tile(&unknown, &context(355.), 0).unwrap());
        assert_eq!(single, calm(&unknown));
    }
}
