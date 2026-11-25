//! The `today` tile: the four parts of the plugin's day (see [`day`](crate::day)) side by
//! side, morning, afternoon, evening and night. Pure, so it is tested natively.
//!
//! - Quarter: each part's icon, its sky and its temperature, and the place under them.
//! - Single: each part's name, its sky beside its temperature, and its highest chance of
//!   rain, blue from 50%; the place under them.
//! - Double: the place, the day's high and low, sunrise, sunset and the reading's age on the
//!   left; the single's parts with the feel, the wind and the rain sum on the right.
//!
//! Every size says where it is and, when the reading is old or the last refresh failed, how
//! old the reading is: the quarter and the single after the place, the double on its own line.
//! A part already over is faded. The part icons of the quarter and the small icons beside the
//! values are muted; the current part's icon is bright. A value the day lacks is "—", a sky it
//! lacks is left blank. Without a reading the tile is calm, as the `now` quarter is, in every
//! size.

use tomakast_plugin::{
    Align, Color, ColorToken, FrameBuilder, Justify, Layout, Node, NodeRef, TextStyle,
    WidgetContext, WidgetFrame,
};

use crate::api::Reading;
use crate::chart::LINE;
use crate::day::{self, Part, PartSummary, When};
use crate::ink::{self, MUTED, PAST, Size};
use crate::tile::{self, PADDING, View, caption, condition, degrees};

/// The sky icon of a part.
const SKY: f32 = 20.;
/// The part icons of the quarter.
const PART: f32 = 14.;
/// The icons beside the details.
const SMALL: f32 = 12.;
/// The width of the double's left column.
const LEFT: f32 = 150.;
/// From what chance of rain it shows blue, in the colour of the rain charts.
const HIGH_CHANCE: f64 = 50.;
const DASH: &str = "—";

/// The `today` tile of `v` at `now` (unix seconds), in the size `context` tells.
pub fn today(v: &View, context: &WidgetContext, now: i64) -> Result<WidgetFrame, String> {
    let Some(r) = v.reading else {
        return tile::quarter(v, context, now);
    };
    let parts = day::parts(r, now);
    let frame = match ink::size_of(context) {
        Size::Quarter => quarter(v, r, &parts, context, now),
        Size::Single => single(v, r, &parts, context, now),
        Size::Double => double(v, r, &parts, context, now),
    };
    tile::with_actions(frame)
}

/// How a part's column is inked: faded when the part is over.
struct Ink<'a> {
    when: When,
    context: &'a WidgetContext,
}

impl Ink<'_> {
    fn past(&self) -> bool {
        self.when == When::Past
    }

    /// `color`, or faded when the part is over.
    fn text(&self, color: Color) -> Color {
        if self.past() {
            ink::faded(self.context, PAST)
        } else {
            color
        }
    }

    /// The opacity of an icon at `alpha`, or faded when the part is over.
    fn alpha(&self, alpha: f32) -> f32 {
        if self.past() { PAST } else { alpha }
    }

    fn icon(
        &self,
        frame: FrameBuilder,
        parent: NodeRef,
        name: &str,
        size: f32,
        alpha: f32,
    ) -> FrameBuilder {
        ink::icon(frame, parent, name, size, self.alpha(alpha), self.context)
    }
}

fn put(frame: FrameBuilder, parent: NodeRef, node: Node) -> FrameBuilder {
    frame.child(parent, node).0
}

fn text_color() -> Color {
    Color::token(ColorToken::Text)
}

fn muted() -> Color {
    Color::token(ColorToken::Muted)
}

/// `value` as `show` writes it, or a dash.
fn or_dash(value: Option<f64>, show: fn(f64) -> String) -> String {
    value.map_or_else(|| DASH.to_string(), show)
}

/// A chance of rain: "25%".
fn chance(value: f64) -> String {
    format!("{value:.0}%")
}

/// A wind speed, without its unit: "18".
fn speed(value: f64) -> String {
    format!("{value:.0}")
}

/// A rain sum: one decimal under 10 mm, whole from there, "0 mm" for none.
fn rain(mm: f64) -> String {
    let tenths = (mm * 10.).round();
    if tenths <= 0. {
        "0 mm".into()
    } else if tenths < 100. {
        format!("{:.1} mm", tenths / 10.)
    } else {
        format!("{mm:.0} mm")
    }
}

/// The tile's root layout: padded, its children filling it across, so the columns' row
/// spreads over the whole tile.
fn stretched() -> Layout {
    Layout::default().padding(PADDING).align(Align::Stretch)
}

/// The row of the four parts' columns, spread over `parent`.
fn columns(frame: FrameBuilder, parent: NodeRef) -> (FrameBuilder, NodeRef) {
    frame.child(
        parent,
        Node::row(
            Layout::default()
                .justify(Justify::SpaceBetween)
                .align(Align::Center),
        )
        .grow(1.),
    )
}

fn column(frame: FrameBuilder, parent: NodeRef) -> (FrameBuilder, NodeRef) {
    frame.child(
        parent,
        Node::column(Layout::default().gap(2.).align(Align::Center)),
    )
}

fn row(frame: FrameBuilder, parent: NodeRef, gap: f32) -> (FrameBuilder, NodeRef) {
    frame.child(
        parent,
        Node::row(Layout::default().gap(gap).align(Align::Center)),
    )
}

/// The sky of `part`, by day for the morning and the afternoon; a blank when it is unknown.
fn sky(frame: FrameBuilder, parent: NodeRef, part: &PartSummary, ink: &Ink) -> FrameBuilder {
    match part.code {
        Some(code) => {
            let by_day = matches!(part.part, Part::Morning | Part::Afternoon);
            ink.icon(frame, parent, condition(code, by_day).0, SKY, 1.)
        }
        None => put(frame, parent, Node::spacer().size(SKY, SKY)),
    }
}

/// A muted small icon and a caption beside it, under `parent`.
fn detail(
    frame: FrameBuilder,
    parent: NodeRef,
    icon: &str,
    text: String,
    color: Color,
    ink: &Ink,
) -> FrameBuilder {
    let (frame, line) = row(frame, parent, 3.);
    let frame = ink.icon(frame, line, icon, SMALL, MUTED);
    put(frame, line, caption(text).color(ink.text(color)))
}

/// The chance of rain of `part`: blue when it is high.
fn chance_line(
    frame: FrameBuilder,
    parent: NodeRef,
    part: &PartSummary,
    ink: &Ink,
) -> FrameBuilder {
    let color = match part.rain_chance {
        Some(c) if c >= HIGH_CHANCE => Color::rgba(LINE),
        _ => muted(),
    };
    detail(
        frame,
        parent,
        "droplets",
        or_dash(part.rain_chance, chance),
        color,
        ink,
    )
}

/// The quarter: each part's icon, sky and temperature, and the place under them.
fn quarter(
    v: &View,
    r: &Reading,
    parts: &[PartSummary; 4],
    context: &WidgetContext,
    now: i64,
) -> FrameBuilder {
    let frame = WidgetFrame::new(Node::column(stretched().gap(4.)));
    let root = frame.root();
    let (mut frame, row) = columns(frame, root);
    for part in parts {
        let ink = Ink {
            when: part.when,
            context,
        };
        let (f, col) = column(frame, row);
        let alpha = if part.when == When::Now { 1. } else { MUTED };
        let f = ink.icon(f, col, part.part.icon(), PART, alpha);
        let f = sky(f, col, part, &ink);
        let temperature = or_dash(part.temperature, degrees);
        frame = put(
            f,
            col,
            Node::text(temperature, TextStyle::Body).color(ink.text(text_color())),
        );
    }
    put(frame, root, caption(tile::place(v, r, now)))
}

/// The name of a part in a single or double, under `col`, and its sky and temperature in a
/// row under it (a row keeps the column low enough for the place or the details).
fn head(frame: FrameBuilder, col: NodeRef, part: &PartSummary, ink: &Ink) -> FrameBuilder {
    let name = match part.when {
        When::Now => text_color(),
        _ => muted(),
    };
    let frame = put(frame, col, caption(part.part.name()).color(ink.text(name)));
    let (frame, sky_at) = row(frame, col, 6.);
    let frame = sky(frame, sky_at, part, ink);
    let temperature = or_dash(part.temperature, degrees);
    put(
        frame,
        sky_at,
        Node::text(temperature, TextStyle::Title).color(ink.text(text_color())),
    )
}

/// The single: each part's name, sky, temperature and chance of rain, and the place under
/// them.
fn single(
    v: &View,
    r: &Reading,
    parts: &[PartSummary; 4],
    context: &WidgetContext,
    now: i64,
) -> FrameBuilder {
    let frame = WidgetFrame::new(Node::column(stretched().gap(4.)));
    let root = frame.root();
    let (mut frame, row) = columns(frame, root);
    for part in parts {
        let ink = Ink {
            when: part.when,
            context,
        };
        let (f, col) = column(frame, row);
        let f = head(f, col, part, &ink);
        frame = chance_line(f, col, part, &ink);
    }
    put(frame, root, caption(tile::place(v, r, now)))
}

/// The double: the place, the day's range, the sun and the age on the left; the parts with
/// their feel, wind, chance and rain sum on the right.
fn double(
    v: &View,
    r: &Reading,
    parts: &[PartSummary; 4],
    context: &WidgetContext,
    now: i64,
) -> FrameBuilder {
    let frame = WidgetFrame::new(Node::row(stretched().gap(16.)));
    let root = frame.root();
    let (frame, left) = frame.child(root, Node::column(Layout::default().gap(2.)).width(LEFT));
    let frame = put(
        frame,
        left,
        Node::text(v.name.clone(), TextStyle::Title).max_lines(1),
    );
    let range = match day::range(r, now) {
        Some((high, low)) => format!("Today · ↑ {} ↓ {}", degrees(high), degrees(low)),
        None => "Today".into(),
    };
    let frame = put(frame, left, caption(range));
    let sun = day::daylight(r, now);
    let time =
        |at: Option<i64>| at.map_or_else(|| DASH.to_string(), |at| day::clock(at, r.utc_offset));
    let (frame, sun_row) = row(frame, left, 8.);
    // The left column is about the whole day: never faded.
    let ink = Ink {
        when: When::Now,
        context,
    };
    let frame = detail(
        frame,
        sun_row,
        "sunrise",
        time(sun.as_ref().map(|s| s.sunrise)),
        muted(),
        &ink,
    );
    let frame = detail(
        frame,
        sun_row,
        "sunset",
        time(sun.as_ref().map(|s| s.sunset)),
        muted(),
        &ink,
    );
    let frame = put(frame, left, Node::spacer().grow(1.));
    let frame = put(frame, left, caption(tile::updated(v, r, now)));

    let (mut frame, row_at) = columns(frame, root);
    for part in parts {
        let ink = Ink {
            when: part.when,
            context,
        };
        let (f, col) = column(frame, row_at);
        let f = head(f, col, part, &ink);
        let (f, feel) = row(f, col, 8.);
        let f = detail(
            f,
            feel,
            "thermometer",
            or_dash(part.feels_like, degrees),
            muted(),
            &ink,
        );
        let f = detail(f, feel, "wind", or_dash(part.wind, speed), muted(), &ink);
        let (f, wet) = row(f, col, 8.);
        let f = chance_line(f, wet, part, &ink);
        frame = put(
            f,
            wet,
            caption(or_dash(part.rain, rain)).color(ink.text(muted())),
        );
    }
    frame
}

#[cfg(test)]
mod tests {
    use tomakast_plugin::{Color, ColorToken, Draw, NodeKind, TextNode};

    use super::*;
    use crate::api::{self, Reading, Units};
    use crate::ink::{MUTED, PAST, context, faded, shown};
    use crate::location::Status;

    const BERLIN_DAY: &[u8] = include_bytes!("../tests/fixtures/forecast-berlin-day.json");
    /// Fetched, and the `now` of the tests: the place's now is 2026-11-21 14:15 local.
    const F: i64 = 1_800_000_000;
    /// 2026-11-21 06:00 local in Berlin.
    const START: i64 = 1795237200;
    const BLUE: u32 = 0x6EA8FEFF;
    const NAMES: [&str; 4] = ["Morning", "Afternoon", "Evening", "Night"];

    fn berlin() -> Reading {
        api::parse_forecast(200, BERLIN_DAY, Units::default(), F).unwrap()
    }

    fn view(reading: Option<&Reading>) -> View<'_> {
        View {
            name: "Berlin, DE".into(),
            reading,
            status: Status::Ok,
            city: "Berlin".into(),
            air: None,
            air_status: Status::Ok,
        }
    }

    fn draw(reading: Option<&Reading>, width: f32) -> WidgetFrame {
        today(&view(reading), &context(width), F).unwrap()
    }

    /// The text nodes of `frame`, in order.
    fn texts(frame: &WidgetFrame) -> Vec<&TextNode> {
        frame
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// The text nodes of `frame` ending in `%`: the chances of rain.
    fn chances(frame: &WidgetFrame) -> Vec<&TextNode> {
        texts(frame)
            .into_iter()
            .filter(|t| t.text.ends_with('%'))
            .collect()
    }

    /// The colour of a text node as `0xRRGGBBAA`, when it is not a token.
    fn rgba(t: &TextNode) -> Option<u32> {
        match t.color {
            Color::Rgba(c) => Some(c),
            _ => None,
        }
    }

    fn is_blue(t: &TextNode) -> bool {
        rgba(t) == Some(BLUE)
    }

    #[test]
    fn the_quarter_has_four_parts_with_icons() {
        let r = berlin();
        let frame = draw(Some(&r), 173.5);
        assert_eq!(
            shown(&frame),
            [
                "faint:sunrise",
                "faint:cloud",
                "2°",
                "icon:sun",
                "icon:cloud-sun-rain",
                "7°",
                "faint:sunset",
                "icon:cloud-moon-rain",
                "5°",
                "faint:moon",
                "icon:cloud-rain",
                "5°",
                "Berlin, DE",
            ]
        );
        // The morning is over: its temperature is faded, the afternoon's is not.
        let texts = texts(&frame);
        let Color::Rgba(past) = faded(&context(173.5), PAST) else {
            unreachable!()
        };
        assert_eq!(texts[0].text, "2°");
        assert_eq!(rgba(texts[0]), Some(past));
        assert_eq!(texts[1].text, "7°");
        assert_ne!(rgba(texts[1]), Some(past));
        // The morning's icons are past; the part icons ahead are muted.
        let opacities: Vec<f32> = frame
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Canvas(c) => Some(c),
                _ => None,
            })
            .flat_map(|c| &c.commands)
            .filter_map(|d| match d {
                Draw::Svg(svg) => Some(svg.opacity),
                _ => None,
            })
            .collect();
        assert_eq!(opacities, [PAST, PAST, MUTED, MUTED]);
        let actions: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["refresh"]);
    }

    #[test]
    fn the_single_adds_the_chance_of_rain() {
        let mut r = berlin();
        let frame = draw(Some(&r), 355.);
        let names: Vec<&str> = texts(&frame)
            .iter()
            .map(|t| t.text.as_str())
            .filter(|t| NAMES.contains(t))
            .collect();
        assert_eq!(names, NAMES);
        // The morning is over, the afternoon is now, the rest ahead.
        let Color::Rgba(past) = faded(&context(355.), PAST) else {
            unreachable!()
        };
        let colors: Vec<Color> = texts(&frame)
            .iter()
            .filter(|t| NAMES.contains(&t.text.as_str()))
            .map(|t| t.color)
            .collect();
        assert!(
            matches!(colors[0], Color::Rgba(c) if c == past),
            "{colors:?}"
        );
        assert!(
            matches!(colors[1], Color::Token(ColorToken::Text)),
            "{colors:?}"
        );
        assert!(
            matches!(colors[2], Color::Token(ColorToken::Muted)),
            "{colors:?}"
        );
        assert!(
            matches!(colors[3], Color::Token(ColorToken::Muted)),
            "{colors:?}"
        );
        let before = chances(&frame);
        let words: Vec<&str> = before.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(words, ["0%", "25%", "40%", "45%"]);
        assert!(!before.iter().any(|t| is_blue(t)));
        // A chance of 50% or more is blue.
        let evening = START + 12 * 3600..START + 18 * 3600;
        let hour = r
            .hours
            .iter_mut()
            .find(|h| evening.contains(&h.at))
            .unwrap();
        hour.rain_chance = Some(70.);
        let frame = draw(Some(&r), 355.);
        let after = chances(&frame);
        assert_eq!(after[2].text, "70%");
        assert!(is_blue(after[2]));
        assert!(!is_blue(after[1]) && !is_blue(after[3]));
        // The place is under the parts, so several cities on Home can be told apart.
        assert_eq!(shown(&frame).last().unwrap(), "Berlin, DE");
    }

    #[test]
    fn a_failed_refresh_says_how_old_the_reading_is() {
        let r = berlin();
        let failed = View {
            status: Status::Failed,
            ..view(Some(&r))
        };
        for width in [173.5, 355.] {
            let frame = today(&failed, &context(width), F + 16 * 60).unwrap();
            assert_eq!(
                shown(&frame).last().unwrap(),
                "Berlin, DE · 16 min ago",
                "{width}"
            );
        }
        let frame = today(&failed, &context(718.), F + 16 * 60).unwrap();
        assert!(shown(&frame).contains(&"Offline · 16 min ago".to_string()));
    }

    #[test]
    fn the_double_adds_the_place_range_sun_and_details() {
        let r = berlin();
        let shown = shown(&draw(Some(&r), 718.));
        assert_eq!(
            shown[..7],
            [
                "Berlin, DE",
                "Today · ↑ 7° ↓ 1°",
                "faint:sunrise",
                "07:38",
                "faint:sunset",
                "16:05",
                "Updated just now",
            ]
        );
        let afternoon = [
            "faint:thermometer",
            "2°",
            "faint:wind",
            "18",
            "faint:droplets",
            "25%",
            "0.2 mm",
        ];
        let at = shown.iter().position(|s| s == "Afternoon").unwrap();
        let next = shown.iter().position(|s| s == "Evening").unwrap();
        assert!(
            shown[at..next].windows(7).any(|w| w == afternoon),
            "{:?}",
            &shown[at..next]
        );
        assert_eq!(
            shown[at..at + 3],
            ["Afternoon", "icon:cloud-sun-rain", "7°"]
        );
    }

    #[test]
    fn rain_sums_read_calmly() {
        assert_eq!(rain(0.), "0 mm");
        assert_eq!(rain(0.04), "0 mm");
        assert_eq!(rain(0.2), "0.2 mm");
        assert_eq!(rain(9.96), "10 mm");
        assert_eq!(rain(12.4), "12 mm");
    }

    #[test]
    fn a_day_without_hours_shows_dashes() {
        let mut r = berlin();
        r.hours.clear();
        for width in [173.5, 355., 718.] {
            let shown = shown(&draw(Some(&r), width));
            assert!(
                !shown
                    .iter()
                    .any(|s| s.ends_with('°') && !s.starts_with("Today")),
                "{shown:?}"
            );
            let dashes = shown.iter().filter(|s| *s == "—").count();
            let per_part = match width {
                173.5 => 1,
                355. => 2,
                _ => 5,
            };
            assert_eq!(dashes, 4 * per_part, "{width}: {shown:?}");
        }
        // No reading at all is calm, as the `now` quarter is, in every size.
        for width in [173.5, 355., 718.] {
            assert_eq!(
                shown(&draw(None, width)),
                ["icon:loader", "Loading", "—", "Berlin, DE"]
            );
        }
    }

    #[test]
    fn node_and_command_caps() {
        let r = berlin();
        let frame = draw(Some(&r), 718.);
        assert!(frame.nodes.len() < 256, "{} nodes", frame.nodes.len());
        let bytes: usize = frame.images.iter().map(|i| i.bytes.len()).sum();
        assert!(bytes < 16 * 1024, "{bytes} bytes of images");
    }
}
