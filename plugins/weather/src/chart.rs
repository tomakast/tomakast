//! The charts of the tiles, drawn on a canvas: a day's 24 hours as a line or as bars, with
//! the past faded and a dashed line at now, and a week as seven bars. Pure, so it is tested
//! natively.

use tomakast_plugin::{Color, ColorToken, Draw, Node, Paint, PathSegment, Point, WidgetContext};

use crate::day::Series;
use crate::ink::{MUTED, PAST};

/// The colour of a line chart (and of rain bars).
pub const LINE: u32 = 0x6EA8FEFF;
/// The width of a chart's line.
const STROKE: f32 = 2.;
/// The radius of the dot at now; the line keeps this far from the top and the bottom.
const DOT: f32 = 3.;
/// The now line: dashes this long, this far apart, in the theme's text at this opacity.
const DASH: f32 = 2.;
const NOW_ALPHA: f32 = 0.35;
/// The space between two bars.
const BAR_GAP: f32 = 2.;
/// The hour marks, every six hours from the day's start at 06:00.
const MARKS: [&str; 4] = ["06", "12", "18", "00"];

/// How a day chart draws its values.
#[derive(Debug, Clone, Copy)]
pub enum Mark {
    /// A line, with a dot at the now value.
    Line,
    /// One bar per hour, coloured by the function (`0xRRGGBBAA` of a value).
    Bars(fn(f64) -> u32),
}

/// The size of a chart, and whether it has the hour marks 06, 12, 18, 00 along the bottom
/// (a day chart in a single).
#[derive(Debug, Clone, Copy)]
pub struct Chart {
    pub width: f32,
    pub height: f32,
    pub marks: bool,
}

fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

fn text_paint() -> Paint {
    Paint::Solid(Color::Token(ColorToken::Text))
}

/// The fraction of `value` from `lo` to `hi`; a flat range is half way.
fn share(value: f64, lo: f64, hi: f64) -> f32 {
    if hi > lo {
        ((value - lo) / (hi - lo)) as f32
    } else {
        0.5
    }
}

/// A canvas `chart.width` x `chart.height` (node id `chart`) of `series`: its values scaled
/// to their range (bars from 0 when no value is below it), hours without a value left out.
/// The hours before now are drawn at [`PAST`]: the line clipped at now, a bar when its hour
/// is over. A dashed line marks now; a line has a dot on the point of the hour that holds
/// now, its value being the one the tile shows.
pub fn day_chart(chart: &Chart, series: &Series, mark: Mark, context: &WidgetContext) -> Node {
    let mark_size = context.theme.sizes.caption;
    let plot = if chart.marks {
        (chart.height - mark_size * 1.25 - 2.).max(0.)
    } else {
        chart.height
    };
    let step = chart.width / 24.;
    let now = series.now.clamp(0., 24.);
    let now_x = now * step;
    let known = series.values.iter().flatten().copied();
    let (lo, hi) = known.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    let mut draws = Vec::new();
    if lo <= hi {
        match mark {
            Mark::Line => {
                let y = |v: f64| DOT + (1. - share(v, lo, hi)) * (plot - 2. * DOT).max(0.);
                let line = line(&series.values, step, y);
                for (from, to, alpha) in [(0., now_x, PAST), (now_x, chart.width, 1.)] {
                    if to > from {
                        draws.extend([
                            Draw::push_clip(from, 0., to - from, chart.height),
                            line.clone().opacity(alpha),
                            Draw::pop_clip(),
                        ]);
                    }
                }
                let hour = (now as usize).min(23);
                if let Some(v) = series.values[hour] {
                    let x = (hour as f32 + 0.5) * step;
                    draws.push(Draw::circle(x, y(v), DOT).fill(text_paint()));
                }
            }
            Mark::Bars(colour) => {
                let lo = lo.min(0.);
                let width = (step - BAR_GAP).max(1.);
                for (hour, value) in series.values.iter().enumerate() {
                    let Some(v) = *value else { continue };
                    let height = share(v, lo, hi) * plot;
                    if hi <= lo || height <= 0. {
                        continue;
                    }
                    let alpha = if (hour + 1) as f32 <= now { PAST } else { 1. };
                    let x = hour as f32 * step + (step - width) / 2.;
                    draws.push(
                        Draw::rect(x, plot - height, width, height)
                            .radius(1.)
                            .fill(Paint::Solid(Color::Rgba(colour(v))))
                            .opacity(alpha),
                    );
                }
            }
        }
    }
    draws.push(now_line(now_x, plot));
    if chart.marks {
        for (i, text) in MARKS.iter().enumerate() {
            let x = i as f32 * 6. * step;
            draws.push(
                Draw::text(x, plot + 2., *text, mark_size).color(Color::Token(ColorToken::Muted)),
            );
        }
    }
    Node::canvas("chart", draws).size(chart.width, chart.height)
}

/// The line through the hours of `values` that have one, each at the middle of its hour;
/// it breaks where an hour has none. A lone hour is a flat stroke across its hour.
fn line(values: &[Option<f64>; 24], step: f32, y: impl Fn(f64) -> f32) -> Draw {
    let mut segments = Vec::new();
    let mut run: Vec<Point> = Vec::new();
    let mut flush = |run: &mut Vec<Point>| {
        if let [only] = run.as_slice() {
            segments.push(PathSegment::MoveTo(point(only.x - step / 2., only.y)));
            segments.push(PathSegment::LineTo(point(only.x + step / 2., only.y)));
        } else if let [first, rest @ ..] = run.as_slice() {
            segments.push(PathSegment::MoveTo(*first));
            segments.extend(rest.iter().map(|p| PathSegment::LineTo(*p)));
        }
        run.clear();
    };
    for (hour, value) in values.iter().enumerate() {
        match value {
            Some(v) => run.push(point((hour as f32 + 0.5) * step, y(*v))),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    Draw::path(segments).stroke(STROKE, Paint::Solid(Color::Rgba(LINE)))
}

/// The dashed line at `x`, from the top to `height`.
fn now_line(x: f32, height: f32) -> Draw {
    let mut segments = Vec::new();
    let mut y = 0.;
    while y + DASH <= height {
        segments.push(PathSegment::MoveTo(point(x, y)));
        segments.push(PathSegment::LineTo(point(x, y + DASH)));
        y += 2. * DASH;
    }
    Draw::path(segments)
        .stroke(1., text_paint())
        .opacity(NOW_ALPHA)
}

/// A canvas of one bar per day of `lengths` (node id `week`), the first (today) in the
/// theme's text, the rest at [`MUTED`]. The bars rise from 90% of the shortest to the
/// longest, so a few minutes' change shows. `chart.marks` is not used.
pub fn week_chart(chart: &Chart, lengths: &[f64], _context: &WidgetContext) -> Node {
    let lo = lengths.iter().copied().fold(f64::INFINITY, f64::min) * 0.9;
    let hi = lengths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let step = chart.width / lengths.len().max(1) as f32;
    let width = (step - BAR_GAP).max(1.);
    let mut draws = Vec::new();
    if hi > 0. {
        for (day, &length) in lengths.iter().enumerate() {
            let height = share(length, lo, hi) * chart.height;
            let alpha = if day == 0 { 1. } else { MUTED };
            let x = day as f32 * step + (step - width) / 2.;
            draws.push(
                Draw::rect(x, chart.height - height, width, height.max(0.))
                    .radius(1.)
                    .fill(text_paint())
                    .opacity(alpha),
            );
        }
    }
    Node::canvas("week", draws).size(chart.width, chart.height)
}

#[cfg(test)]
mod tests {
    use tomakast_plugin::{Color, Draw, NodeKind, Paint, PathSegment, Point, ShapeStyle};

    use super::*;
    use crate::ink::context;

    const SINGLE: Chart = Chart {
        width: 207.,
        height: 90.,
        marks: true,
    };
    const STEP: f32 = 207. / 24.;

    fn commands(node: &Node) -> &[Draw] {
        match &node.kind {
            NodeKind::Canvas(c) => &c.commands,
            other => panic!("not a canvas: {other:?}"),
        }
    }

    fn series(now: f32, value: impl Fn(usize) -> Option<f64>) -> Series {
        Series {
            values: std::array::from_fn(value),
            now,
        }
    }

    fn rising(now: f32) -> Series {
        series(now, |i| Some(i as f64))
    }

    fn points(draw: &Draw) -> Vec<(bool, Point)> {
        let Draw::Path(p) = draw else { return vec![] };
        p.segments
            .iter()
            .filter_map(|s| match s {
                PathSegment::MoveTo(at) => Some((true, *at)),
                PathSegment::LineTo(at) => Some((false, *at)),
                _ => None,
            })
            .collect()
    }

    /// The commands and path segments of `node`, as the frame check counts them.
    fn cost(node: &Node) -> usize {
        commands(node)
            .iter()
            .map(|d| match d {
                Draw::Path(p) => 1 + p.segments.len(),
                _ => 1,
            })
            .sum()
    }

    /// Every number of the commands of `node`.
    fn numbers(node: &Node) -> Vec<f32> {
        let mut out = Vec::new();
        for draw in commands(node) {
            match draw {
                Draw::Rect(r) => out.extend([r.x, r.y, r.width, r.height, r.style.opacity]),
                Draw::Ellipse(e) => out.extend([e.cx, e.cy, e.rx, e.ry, e.style.opacity]),
                Draw::Path(p) => {
                    out.push(p.style.opacity);
                    out.extend(points(draw).iter().flat_map(|(_, at)| [at.x, at.y]));
                }
                Draw::Text(t) => out.extend([t.x, t.y, t.size, t.opacity]),
                Draw::PushClip(c) => out.extend([c.x, c.y, c.width, c.height]),
                _ => {}
            }
        }
        out
    }

    fn opacity(draw: &Draw) -> f32 {
        match draw {
            Draw::Rect(r) => r.style.opacity,
            Draw::Path(p) => p.style.opacity,
            Draw::Ellipse(e) => e.style.opacity,
            Draw::Text(t) => t.opacity,
            other => panic!("no opacity: {other:?}"),
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    /// The bars of `node`: hour, opacity and colour.
    fn bars(node: &Node) -> Vec<(usize, f32, u32)> {
        commands(node)
            .iter()
            .filter_map(|d| match d {
                Draw::Rect(r) => Some((
                    ((r.x + r.width / 2.) / STEP) as usize,
                    r.style.opacity,
                    rgba(&r.style),
                )),
                _ => None,
            })
            .collect()
    }

    /// The colour of a solid shape.
    fn rgba(style: &ShapeStyle) -> u32 {
        match style.paint {
            Paint::Solid(Color::Rgba(rgba)) => rgba,
            ref other => panic!("not a solid rgba: {other:?}"),
        }
    }

    #[test]
    fn a_line_chart_fades_the_past_and_marks_now() {
        let node = day_chart(&SINGLE, &rising(6.), Mark::Line, &context(355.));
        assert_eq!(node.id, "chart");
        assert_eq!((node.width, node.height), (Some(207.), Some(90.)));
        let draws = commands(&node);
        let now_x = STEP * 6.;
        assert!(near(now_x, 51.75));

        // The past half, clipped to x < now, at PAST; the future half at full opacity.
        let clip = |from: f32, width: f32| {
            draws.windows(3).find_map(|w| match w {
                [Draw::PushClip(c), path @ Draw::Path(_), Draw::PopClip]
                    if near(c.x, from) && near(c.width, width) =>
                {
                    Some(path)
                }
                _ => None,
            })
        };
        let past = clip(0., now_x).expect("a clip of the past");
        assert_eq!(opacity(past), 0.4);
        let future = clip(now_x, 207. - now_x).expect("a clip of the future");
        assert_eq!(opacity(future), 1.);

        // The now line: 2 px dashes 2 px apart at x = now.
        let dashes = draws
            .iter()
            .map(points)
            .find(|p| !p.is_empty() && p.iter().all(|(_, at)| near(at.x, now_x)))
            .expect("the now line");
        assert!(dashes.len() >= 4);
        for pair in dashes.chunks(2) {
            let [(true, from), (false, to)] = pair else {
                panic!("{dashes:?}")
            };
            assert!(near(to.y - from.y, 2.), "{dashes:?}");
        }
        for gap in dashes[1..].chunks(2).filter(|c| c.len() == 2) {
            assert!(near(gap[1].1.y - gap[0].1.y, 2.), "{dashes:?}");
        }

        // The dot sits on the line, at the point of the hour that holds now (6), with its
        // value; the now line stays at now.
        let dot = draws
            .iter()
            .find_map(|d| match d {
                Draw::Ellipse(e) => Some(e),
                _ => None,
            })
            .expect("the dot");
        let six = points(future)
            .into_iter()
            .find(|(_, at)| near(at.x, STEP * 6.5))
            .expect("hour 6 on the line")
            .1;
        assert!(
            near(dot.cx, six.x) && near(dot.cy, six.y),
            "{dot:?} {six:?}"
        );
        assert_eq!(dot.rx, 3.);

        let texts: Vec<&str> = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["06", "12", "18", "00"]);
    }

    #[test]
    fn bars_take_their_colour_and_fade() {
        let colour: fn(f64) -> u32 = |v| if v < 3. { 1 } else { 2 };
        // Hour 6 holds now: it is not over, so it stays bright.
        let node = day_chart(&SINGLE, &rising(6.5), Mark::Bars(colour), &context(355.));
        let bars = bars(&node);
        // Hour 0 is 0, a bar of no height.
        assert_eq!(bars.len(), 23, "{bars:?}");
        for (hour, opacity, rgba) in bars {
            let past = if hour < 6 { 0.4 } else { 1. };
            assert_eq!(opacity, past, "hour {hour}");
            assert_eq!(rgba, if hour < 3 { 1 } else { 2 }, "hour {hour}");
        }
        assert!(
            !commands(&node)
                .iter()
                .any(|d| matches!(d, Draw::Ellipse(_)))
        );
    }

    #[test]
    fn gaps_and_empty_series() {
        let holed = series(6., |i| (i != 10).then_some(i as f64 + 1.));
        let node = day_chart(&SINGLE, &holed, Mark::Bars(|_| 1), &context(355.));
        let hours: Vec<usize> = bars(&node).iter().map(|b| b.0).collect();
        assert_eq!(hours.len(), 23);
        assert!(!hours.contains(&10));

        // The line breaks at the gap: each half of it starts twice (before and after 10).
        let node = day_chart(&SINGLE, &holed, Mark::Line, &context(355.));
        let future = commands(&node)
            .windows(3)
            .find_map(|w| match w {
                [Draw::PushClip(c), path @ Draw::Path(_), Draw::PopClip] if c.x > 0. => Some(path),
                _ => None,
            })
            .unwrap();
        let starts = points(future).iter().filter(|(start, _)| *start).count();
        assert_eq!(starts, 2);

        let empty = series(6., |_| None);
        let quiet = Chart {
            marks: false,
            ..SINGLE
        };
        for mark in [Mark::Line, Mark::Bars(|_| 1)] {
            let node = day_chart(&quiet, &empty, mark, &context(355.));
            let draws = commands(&node);
            assert_eq!(draws.len(), 1, "{draws:?}");
            assert!(points(&draws[0]).iter().all(|(_, at)| near(at.x, 51.75)));
            assert_eq!(opacity(&draws[0]), 0.35);
        }
    }

    #[test]
    fn edges_stay_finite() {
        let ctx = context(355.);
        let flat = series(24., |_| Some(0.));
        let mut nodes = Vec::new();
        for now in [0., 24.] {
            let s = Series { now, ..flat };
            nodes.push(day_chart(&SINGLE, &s, Mark::Line, &ctx));
            nodes.push(day_chart(&SINGLE, &s, Mark::Bars(|_| 1), &ctx));
        }
        nodes.push(week_chart(&SINGLE, &[0.; 7], &ctx));
        nodes.push(week_chart(&SINGLE, &[], &ctx));
        nodes.push(week_chart(&SINGLE, &[30_000.; 7], &ctx));
        for node in &nodes {
            assert!(numbers(node).iter().all(|n| n.is_finite()), "{node:?}");
        }
        // A flat line still draws; at 24 its dot is on the last hour's point.
        let dot = commands(&nodes[2])
            .iter()
            .find_map(|d| match d {
                Draw::Ellipse(e) => Some(e),
                _ => None,
            })
            .expect("the dot");
        assert!(near(dot.cx, STEP * 23.5), "{dot:?}");
    }

    #[test]
    fn the_week_chart_has_seven_bars() {
        let lengths = [
            30_000., 30_100., 30_200., 30_300., 30_400., 30_500., 30_600.,
        ];
        let node = week_chart(&SINGLE, &lengths, &context(355.));
        let rects: Vec<_> = commands(&node)
            .iter()
            .filter_map(|d| match d {
                Draw::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 7);
        let opacities: Vec<f32> = rects.iter().map(|r| r.style.opacity).collect();
        assert_eq!(opacities, [1., 0.6, 0.6, 0.6, 0.6, 0.6, 0.6]);
        // From 90% of the shortest to the longest: the shortest still shows, the longest fills.
        assert!(rects[0].height > 0. && rects[0].height < rects[6].height);
        assert!(near(
            rects[6].height + rects[6].y,
            rects[0].height + rects[0].y
        ));
    }

    #[test]
    fn a_chart_stays_small() {
        let ctx = context(355.);
        let holed = series(6.5, |i| (i % 3 != 0).then_some(i as f64));
        for s in [rising(6.5), holed] {
            for mark in [Mark::Line, Mark::Bars(|_| 1)] {
                let node = day_chart(&SINGLE, &s, mark, &ctx);
                assert!(cost(&node) <= 200, "{}", cost(&node));
            }
        }
    }
}
