//! A canvas's draw commands as paint ops in window coordinates.
//!
//! GPUI draws no transformed primitive, so [`plan`] applies each command's
//! transform on the CPU: shapes and paths go through the whole transform
//! (a shape that is not an axis-aligned box or circle becomes a path, whose
//! cubics stay exact under any affine map), text and images take only its
//! translation and scale. GPUI clips to rectangles only: a clip under a
//! rotation clips to the box around the rotated rectangle. Every number is
//! checked again after the transforms, which a frame's 32 nested scales
//! of 100 overflow: what is not finite is not drawn.

use gpui_kit::{
    Background, Bounds, Hsla, Pixels, SharedString, linear_color_stop, linear_gradient, point, px,
    size,
};
// The largest text, after its transform's scale, is the largest a text
// command may ask for: GPUI rasterizes every glyph at its size, on the UI
// thread.
use tomakast_core::MAX_TEXT_SIZE;
use tomakast_core::{
    Draw, EllipseCmd, ImageCmd, Paint, PathSegment, Point, RectCmd, ShapeStyle, TextCmd, Transform,
    WidgetTheme,
};

use crate::strip::components::color;

/// How far along its tangent a cubic's control point lies from each end of
/// a quarter of a circle, in radii.
const KAPPA: f32 = 0.5523;
/// A text's line box, in text sizes.
pub(crate) const LINE_HEIGHT: f32 = 1.25;
/// The farthest a path or a gradient box may reach from the window's
/// origin, in logical pixels: 2^22. Past it, an f32 holds less than a
/// pixel's eighth, too coarse to tessellate; such a shape is not drawn.
const MAX_COORDINATE: f32 = 4_194_304.;
/// Below this, relative to the transform's scale, a transform has no
/// rotation, and two lengths are equal.
const EPSILON: f32 = 1e-4;

/// An affine map: `(x, y)` goes to `(a x + c y + e, b x + d y + f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub fn identity() -> Affine {
        Affine {
            a: 1.,
            b: 0.,
            c: 0.,
            d: 1.,
            e: 0.,
            f: 0.,
        }
    }

    /// `inner`, then `self`.
    pub fn then(&self, inner: &Affine) -> Affine {
        Affine {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// The lengths of the transformed unit vectors.
    pub fn scale_only(&self) -> (f32, f32) {
        (self.a.hypot(self.b), self.c.hypot(self.d))
    }

    fn mean_scale(&self) -> f32 {
        let (x, y) = self.scale_only();
        (x + y) / 2.
    }

    /// Whether the map keeps boxes axis-aligned and unflipped: a scale and
    /// a translation.
    fn axis_aligned(&self) -> bool {
        self.a > 0.
            && self.d > 0.
            && self.b.abs() <= EPSILON * self.a
            && self.c.abs() <= EPSILON * self.d
    }

    /// Direction `degrees` (0 up, clockwise, as CSS) under the map's linear
    /// part; `None` when the map makes it no direction.
    fn turn(&self, degrees: f32) -> Option<f32> {
        let (sin, cos) = degrees.to_radians().sin_cos();
        // Up is -y on screen.
        let (x, y) = (sin, -cos);
        let (x, y) = (self.a * x + self.c * y, self.b * x + self.d * y);
        let turned = x.atan2(-y).to_degrees().rem_euclid(360.);
        (turned.is_finite() && (x != 0. || y != 0.)).then_some(turned)
    }
}

impl From<&Transform> for Affine {
    /// Scale, then rotate (degrees, clockwise on screen), then translate.
    fn from(t: &Transform) -> Affine {
        let (sin, cos) = t.rotate.to_radians().sin_cos();
        Affine {
            a: cos * t.scale.x,
            b: sin * t.scale.x,
            c: -sin * t.scale.y,
            d: cos * t.scale.y,
            e: t.translate.x,
            f: t.translate.y,
        }
    }
}

/// One step of a [`PaintOp::Path`], in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PathPoint {
    MoveTo(gpui_kit::Point<Pixels>),
    LineTo(gpui_kit::Point<Pixels>),
    QuadTo {
        ctrl: gpui_kit::Point<Pixels>,
        to: gpui_kit::Point<Pixels>,
    },
    CubicTo {
        ctrl_a: gpui_kit::Point<Pixels>,
        ctrl_b: gpui_kit::Point<Pixels>,
        to: gpui_kit::Point<Pixels>,
    },
    Close,
}

/// What the canvas paints, in window coordinates, in command order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintOp {
    /// A box, a rounded box or a circle; a stroke is a border, centred on
    /// the outline as a path's stroke is.
    Quad {
        bounds: Bounds<Pixels>,
        radius: Pixels,
        fill: Option<Background>,
        border: Option<(Pixels, Hsla)>,
    },
    /// Filled, or stroked that wide.
    Path {
        points: Vec<PathPoint>,
        fill: Option<Background>,
        stroke: Option<(Pixels, Background)>,
    },
    /// One line; `origin` is the top-left of its line box.
    Text {
        origin: gpui_kit::Point<Pixels>,
        text: SharedString,
        size: Pixels,
        color: Hsla,
        family: SharedString,
        /// Shaped with the monospace features: `family` is the theme's
        /// monospace one, asked for by name or not.
        mono: bool,
    },
    Image {
        bounds: Bounds<Pixels>,
        image: String,
        opacity: f32,
        svg: bool,
    },
    /// Clips what follows to these bounds, already inside the enclosing
    /// clip's, until its [`PaintOp::Unclip`].
    Clip(Bounds<Pixels>),
    Unclip,
}

/// The ops that paint `commands` on a canvas at `bounds`. Commands whose
/// box misses the current clip, and those with a number that is not finite
/// after the transforms, are left out; the clips stay balanced.
pub(crate) fn plan(commands: &[Draw], bounds: Bounds<Pixels>, theme: &WidgetTheme) -> Vec<PaintOp> {
    let mut planner = Planner {
        theme,
        ops: Vec::with_capacity(commands.len()),
        affines: vec![Affine {
            e: f32::from(bounds.origin.x),
            f: f32::from(bounds.origin.y),
            ..Affine::identity()
        }],
        clips: vec![bounds],
    };
    for command in commands {
        planner.command(command);
    }
    // A checked frame balances its clips; any left open close here.
    for _ in 1..planner.clips.len() {
        planner.ops.push(PaintOp::Unclip);
    }
    planner.ops
}

struct Planner<'a> {
    theme: &'a WidgetTheme,
    ops: Vec<PaintOp>,
    /// The transforms in force, composed; the first places the canvas.
    affines: Vec<Affine>,
    /// The clips in force, each inside the one before; the first is the
    /// canvas.
    clips: Vec<Bounds<Pixels>>,
}

impl Planner<'_> {
    fn affine(&self) -> Affine {
        *self.affines.last().expect("the canvas's own transform")
    }

    fn clip(&self) -> Bounds<Pixels> {
        *self.clips.last().expect("the canvas's own clip")
    }

    fn command(&mut self, command: &Draw) {
        match command {
            Draw::Rect(rect) => self.rect(rect),
            Draw::Ellipse(ellipse) => self.ellipse(ellipse),
            Draw::Path(path) => {
                let mut points = Points::new(self.affine());
                for segment in &path.segments {
                    points.segment(segment);
                }
                self.path(points.out, &path.style);
            }
            Draw::Text(text) => self.text(text),
            Draw::Image(image) => self.image(image, false),
            Draw::Svg(image) => self.image(image, true),
            Draw::PushClip(rect) => {
                let a = self.affine();
                let (x0, y0) = (rect.x, rect.y);
                let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
                let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| to_px(a.apply(Point { x, y })));
                let enclosing = self.clip();
                // One that is not finite clips everything away.
                let clip = bounding_box(&corners)
                    .map(|b| b.intersect(&enclosing))
                    .unwrap_or_else(|| Bounds::new(enclosing.origin, size(px(0.), px(0.))));
                self.clips.push(clip);
                self.ops.push(PaintOp::Clip(clip));
            }
            Draw::PopClip => {
                if self.clips.len() > 1 {
                    self.clips.pop();
                    self.ops.push(PaintOp::Unclip);
                }
            }
            Draw::PushTransform(t) => {
                let affine = self.affine().then(&Affine::from(t));
                self.affines.push(affine);
            }
            Draw::PopTransform => {
                if self.affines.len() > 1 {
                    self.affines.pop();
                }
            }
        }
    }

    /// Whether `bounds` shows in the current clip, even in part.
    fn shows(&self, bounds: &Bounds<Pixels>) -> bool {
        let clip = self.clip();
        clip.size.width > px(0.)
            && clip.size.height > px(0.)
            && bounds.left() <= clip.right()
            && bounds.right() >= clip.left()
            && bounds.top() <= clip.bottom()
            && bounds.bottom() >= clip.top()
    }

    /// The paint of `style`, at its opacity, its gradient's direction
    /// transformed with the shape (turned, and mirrored by a flip). GPUI
    /// spreads it over the shape's axis-aligned box.
    fn background(&self, style: &ShapeStyle) -> Background {
        match &style.paint {
            Paint::Solid(c) => color(c, self.theme).opacity(style.opacity).into(),
            Paint::Linear(g) => {
                let stop = |s: &tomakast_core::ColorStop| {
                    linear_color_stop(color(&s.color, self.theme).opacity(style.opacity), s.at)
                };
                let angle = self.affine().turn(g.angle).unwrap_or(g.angle);
                linear_gradient(angle, stop(&g.start), stop(&g.end))
            }
        }
    }

    /// GPUI's quads take a solid border only: a gradient stroke is a path.
    fn quad_paints(style: &ShapeStyle) -> bool {
        style.stroke.is_none() || matches!(style.paint, Paint::Solid(_))
    }

    fn rect(&mut self, r: &RectCmd) {
        let a = self.affine();
        let radius = r.radius.min(r.width / 2.).min(r.height / 2.).max(0.);
        // A rounded corner stays round under an even scale only.
        if a.axis_aligned() && Self::quad_paints(&r.style) && (radius == 0. || same(a.a, a.d)) {
            let p0 = a.apply(Point { x: r.x, y: r.y });
            let p1 = a.apply(Point {
                x: r.x + r.width,
                y: r.y + r.height,
            });
            self.quad(p0, p1, radius * a.a, &r.style);
            return;
        }
        let mut points = Points::new(a);
        let (x, y, w, h) = (r.x, r.y, r.width, r.height);
        if radius == 0. {
            points.move_to(x, y);
            points.line_to(x + w, y);
            points.line_to(x + w, y + h);
            points.line_to(x, y + h);
        } else {
            let (r, k) = (radius, KAPPA * radius);
            points.move_to(x + r, y);
            points.line_to(x + w - r, y);
            points.cubic_to((x + w - r + k, y), (x + w, y + r - k), (x + w, y + r));
            points.line_to(x + w, y + h - r);
            points.cubic_to(
                (x + w, y + h - r + k),
                (x + w - r + k, y + h),
                (x + w - r, y + h),
            );
            points.line_to(x + r, y + h);
            points.cubic_to((x + r - k, y + h), (x, y + h - r + k), (x, y + h - r));
            points.line_to(x, y + r);
            points.cubic_to((x, y + r - k), (x + r - k, y), (x + r, y));
        }
        points.close();
        self.path(points.out, &r.style);
    }

    fn ellipse(&mut self, e: &EllipseCmd) {
        let a = self.affine();
        if a.axis_aligned() && Self::quad_paints(&e.style) && same(e.rx * a.a, e.ry * a.d) {
            let p0 = a.apply(Point {
                x: e.cx - e.rx,
                y: e.cy - e.ry,
            });
            let p1 = a.apply(Point {
                x: e.cx + e.rx,
                y: e.cy + e.ry,
            });
            self.quad(p0, p1, e.rx * a.a, &e.style);
            return;
        }
        let (cx, cy, rx, ry) = (e.cx, e.cy, e.rx, e.ry);
        let (kx, ky) = (KAPPA * rx, KAPPA * ry);
        let mut points = Points::new(a);
        points.move_to(cx + rx, cy);
        points.cubic_to((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry));
        points.cubic_to((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy));
        points.cubic_to((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry));
        points.cubic_to((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy));
        points.close();
        self.path(points.out, &e.style);
    }

    /// A box from `p0` to `p1` with corners of `radius`, both transformed.
    fn quad(&mut self, p0: Point, p1: Point, radius: f32, style: &ShapeStyle) {
        let Some(mut bounds) = bounding_box(&[to_px(p0), to_px(p1)]) else {
            return;
        };
        let mut radius = radius;
        let (fill, border) = match style.stroke {
            None => (Some(self.background(style)), None),
            Some(width) => {
                let width = width * self.affine().mean_scale();
                if !(width.is_finite() && width > 0.) {
                    return;
                }
                // Centred on the outline, as a path's stroke is.
                bounds = bounds.dilate(px(width / 2.));
                if radius > 0. {
                    radius += width / 2.;
                }
                let Paint::Solid(c) = &style.paint else {
                    return;
                };
                let border = color(c, self.theme).opacity(style.opacity);
                (None, Some((px(width), border)))
            }
        };
        if !radius.is_finite() || !finite_bounds(&bounds) || !self.shows(&bounds) {
            return;
        }
        // Past the clip, by more than its corner and border, a solid box
        // looks the same cut there: a huge one stays a small one. GPUI
        // spreads a gradient over the box, which must keep its size.
        let bounds = if fill.is_none_or(|f| f.as_solid().is_some()) {
            let margin = px(radius + border.map_or(0., |(w, _)| f32::from(w)) + 1.);
            bounds.intersect(&self.clip().dilate(margin))
        } else if within_reach(&bounds) {
            bounds
        } else {
            return;
        };
        self.ops.push(PaintOp::Quad {
            bounds,
            radius: px(radius),
            fill,
            border,
        });
    }

    fn path(&mut self, points: Vec<PathPoint>, style: &ShapeStyle) {
        let Some(bounds) = bounding_box(&path_corners(&points)) else {
            return;
        };
        let (fill, stroke, half) = match style.stroke {
            None => (Some(self.background(style)), None, 0.),
            Some(width) => {
                let width = width * self.affine().mean_scale();
                if !(width.is_finite() && width > 0.) {
                    return;
                }
                (None, Some((px(width), self.background(style))), width / 2.)
            }
        };
        let bounds = bounds.dilate(px(half));
        if !finite_bounds(&bounds) || !within_reach(&bounds) || !self.shows(&bounds) {
            return;
        }
        self.ops.push(PaintOp::Path {
            points,
            fill,
            stroke,
        });
    }

    fn text(&mut self, t: &TextCmd) {
        let a = self.affine();
        let (sx, sy) = a.scale_only();
        let origin = point(px(a.e + sx * t.x), px(a.f + sy * t.y));
        let Some(size) = text_size(t.size, (sx, sy)).filter(|_| finite_point(origin)) else {
            return;
        };
        // Its width is not known before it is shaped: only a text below,
        // right of or above the clip is left out.
        let clip = self.clip();
        if clip.size.width <= px(0.)
            || clip.size.height <= px(0.)
            || origin.x > clip.right()
            || origin.y > clip.bottom()
            || origin.y + px(size * LINE_HEIGHT) < clip.top()
        {
            return;
        }
        let text = one_line(&t.text);
        if text.is_empty() {
            return;
        }
        let family = match &t.font {
            Some(family) => family.clone(),
            None if t.mono => self.theme.mono_font.clone(),
            None => self.theme.ui_font.clone(),
        };
        let mono = family == self.theme.mono_font;
        self.ops.push(PaintOp::Text {
            origin,
            text: text.into(),
            size: px(size),
            color: color(&t.color, self.theme).opacity(t.opacity),
            family: family.into(),
            mono,
        });
    }

    fn image(&mut self, i: &ImageCmd, svg: bool) {
        let a = self.affine();
        let (sx, sy) = a.scale_only();
        let bounds = Bounds::new(
            point(px(a.e + sx * i.x), px(a.f + sy * i.y)),
            size(px(i.width * sx), px(i.height * sy)),
        );
        if !finite_bounds(&bounds) || !self.shows(&bounds) {
            return;
        }
        self.ops.push(PaintOp::Image {
            bounds,
            image: i.image.clone(),
            opacity: i.opacity,
            svg,
        });
    }
}

/// Path points, transformed as they are added.
struct Points {
    affine: Affine,
    out: Vec<PathPoint>,
}

impl Points {
    fn new(affine: Affine) -> Points {
        Points {
            affine,
            out: Vec::new(),
        }
    }

    fn at(&self, (x, y): (f32, f32)) -> gpui_kit::Point<Pixels> {
        to_px(self.affine.apply(Point { x, y }))
    }

    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at((x, y));
        self.out.push(PathPoint::MoveTo(p));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at((x, y));
        self.out.push(PathPoint::LineTo(p));
    }

    fn cubic_to(&mut self, a: (f32, f32), b: (f32, f32), to: (f32, f32)) {
        let (ctrl_a, ctrl_b, to) = (self.at(a), self.at(b), self.at(to));
        self.out.push(PathPoint::CubicTo { ctrl_a, ctrl_b, to });
    }

    fn close(&mut self) {
        self.out.push(PathPoint::Close);
    }

    fn segment(&mut self, segment: &PathSegment) {
        let at = |p: &Point| self.at((p.x, p.y));
        let point = match segment {
            PathSegment::MoveTo(p) => PathPoint::MoveTo(at(p)),
            PathSegment::LineTo(p) => PathPoint::LineTo(at(p)),
            PathSegment::QuadTo { ctrl, to } => PathPoint::QuadTo {
                ctrl: at(ctrl),
                to: at(to),
            },
            PathSegment::CubicTo { ctrl_a, ctrl_b, to } => PathPoint::CubicTo {
                ctrl_a: at(ctrl_a),
                ctrl_b: at(ctrl_b),
                to: at(to),
            },
            PathSegment::Close => PathPoint::Close,
        };
        self.out.push(point);
    }
}

/// Whether `bounds` lies within [`MAX_COORDINATE`] of the window's origin.
fn within_reach(bounds: &Bounds<Pixels>) -> bool {
    let reach = px(MAX_COORDINATE);
    bounds.left() >= -reach
        && bounds.top() >= -reach
        && bounds.right() <= reach
        && bounds.bottom() <= reach
}

/// A text of `size` under a transform of `scale` (see
/// [`Affine::scale_only`]): the mean scale times it, at most
/// [`MAX_TEXT_SIZE`]; `None` when that is not a finite size.
fn text_size(size: f32, (sx, sy): (f32, f32)) -> Option<f32> {
    let size = size * (sx + sy) / 2.;
    (size.is_finite() && size > 0.).then(|| size.min(MAX_TEXT_SIZE))
}

fn to_px(p: Point) -> gpui_kit::Point<Pixels> {
    point(px(p.x), px(p.y))
}

/// Whether `a` and `b` are equal, up to [`EPSILON`] of the larger.
fn same(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPSILON * a.abs().max(b.abs())
}

fn finite_point(p: gpui_kit::Point<Pixels>) -> bool {
    f32::from(p.x).is_finite() && f32::from(p.y).is_finite()
}

fn finite_bounds(b: &Bounds<Pixels>) -> bool {
    finite_point(b.origin)
        && f32::from(b.size.width).is_finite()
        && f32::from(b.size.height).is_finite()
}

/// Every point of `points`, control points too: a curve lies inside them.
pub(super) fn path_corners(points: &[PathPoint]) -> Vec<gpui_kit::Point<Pixels>> {
    let mut corners = Vec::with_capacity(points.len());
    for p in points {
        match *p {
            PathPoint::MoveTo(p) | PathPoint::LineTo(p) => corners.push(p),
            PathPoint::QuadTo { ctrl, to } => corners.extend([ctrl, to]),
            PathPoint::CubicTo { ctrl_a, ctrl_b, to } => corners.extend([ctrl_a, ctrl_b, to]),
            PathPoint::Close => {}
        }
    }
    corners
}

/// The axis-aligned box around `points`; `None` when there are none or one
/// is not finite.
pub(super) fn bounding_box(points: &[gpui_kit::Point<Pixels>]) -> Option<Bounds<Pixels>> {
    let first = *points.first()?;
    let (mut min, mut max) = (first, first);
    for &p in points {
        if !finite_point(p) {
            return None;
        }
        min = min.min(&p);
        max = max.max(&p);
    }
    let bounds = Bounds::from_corners(min, max);
    finite_bounds(&bounds).then_some(bounds)
}

/// `text` on one line: line breaks and other control characters, which
/// GPUI would lay out as more lines or not at all, become spaces.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Bounds, point, px, size};
    use tomakast_core::{
        ClipRect, Color, ColorStop, Draw, EllipseCmd, LinearGradient, Paint, PathCmd, PathSegment,
        Point, RectCmd, ShapeStyle, TextCmd, TextSizes, ThemeColors, Transform, WidgetTheme,
    };

    fn theme() -> WidgetTheme {
        WidgetTheme {
            dark: false,
            colors: ThemeColors {
                text: 0x000000ff,
                muted: 0x808080ff,
                panel: 0xffffffff,
                border: 0xccccccff,
                selected: 0x0000ffff,
                error: 0xff0000ff,
                success: 0x00ff00ff,
            },
            ui_font: "UI".into(),
            mono_font: "Mono".into(),
            sizes: TextSizes {
                title: 16.,
                value: 28.,
                body: 14.,
                caption: 12.,
            },
        }
    }

    fn canvas(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(width), px(height)))
    }

    fn image_cmd(id: &str, x: f32, y: f32) -> tomakast_core::ImageCmd {
        tomakast_core::ImageCmd {
            image: id.into(),
            x,
            y,
            width: 10.,
            height: 20.,
            opacity: 0.5,
        }
    }

    /// Image ops come in command order between their clips, placed by the
    /// transform's translation and scale; one outside the clip is left out.
    #[test]
    fn images_are_planned_in_order_inside_their_clips() {
        let commands = [
            Draw::PushTransform(Transform {
                translate: tomakast_core::Point { x: 5., y: 5. },
                scale: tomakast_core::Point { x: 2., y: 2. },
                rotate: 0.,
            }),
            Draw::Image(image_cmd("a", 1., 1.)),
            Draw::PushClip(ClipRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 20.,
            }),
            Draw::Svg(image_cmd("b", 0., 0.)),
            Draw::Svg(image_cmd("hidden", 100., 100.)),
            Draw::PopClip,
            Draw::PopTransform,
        ];
        let ops = plan(&commands, canvas(355., 120.), &theme());
        let at = |x: f32, y: f32| Bounds::new(point(px(x), px(y)), size(px(20.), px(40.)));
        let image = |id: &str, bounds, svg| PaintOp::Image {
            bounds,
            image: id.into(),
            opacity: 0.5,
            svg,
        };
        assert_eq!(
            ops,
            [
                image("a", at(7., 7.), false),
                PaintOp::Clip(Bounds::new(point(px(5.), px(5.)), size(px(40.), px(40.)))),
                image("b", at(5., 5.), true),
                PaintOp::Unclip,
            ]
        );
    }

    fn solid(rgba: u32) -> ShapeStyle {
        ShapeStyle {
            paint: Paint::Solid(Color::Rgba(rgba)),
            stroke: None,
            opacity: 1.,
        }
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Draw {
        Draw::Rect(RectCmd {
            x,
            y,
            width,
            height,
            radius: 0.,
            style: solid(0xff0000ff),
        })
    }

    fn transform(translate: (f32, f32), scale: (f32, f32), rotate: f32) -> Draw {
        Draw::PushTransform(Transform {
            translate: Point {
                x: translate.0,
                y: translate.1,
            },
            scale: Point {
                x: scale.0,
                y: scale.1,
            },
            rotate,
        })
    }

    fn rotate(degrees: f32) -> Draw {
        transform((0., 0.), (1., 1.), degrees)
    }

    fn clip(x: f32, y: f32, width: f32, height: f32) -> Draw {
        Draw::PushClip(ClipRect {
            x,
            y,
            width,
            height,
        })
    }

    fn text(x: f32, y: f32, text: &str, size: f32) -> Draw {
        Draw::Text(TextCmd {
            x,
            y,
            text: text.into(),
            size,
            color: Color::Rgba(0x000000ff),
            font: None,
            mono: false,
            opacity: 1.,
        })
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4
    }

    fn near(p: gpui_kit::Point<Pixels>, x: f32, y: f32) -> bool {
        close(f32::from(p.x), x) && close(f32::from(p.y), y)
    }

    fn assert_bounds(b: Bounds<Pixels>, x: f32, y: f32, width: f32, height: f32) {
        let got = [
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        ];
        let want = [x, y, width, height];
        assert!(
            got.iter().zip(&want).all(|(g, w)| (g - w).abs() <= 0.01),
            "{got:?} != {want:?}"
        );
    }

    #[test]
    fn rotation_moves_path_vertices() {
        let path = Draw::Path(PathCmd {
            segments: vec![
                PathSegment::MoveTo(Point { x: 10., y: 0. }),
                PathSegment::LineTo(Point { x: 20., y: 0. }),
            ],
            style: ShapeStyle {
                stroke: Some(1.),
                ..solid(0x000000ff)
            },
        });
        let ops = plan(
            &[rotate(90.), path, Draw::PopTransform],
            canvas(100., 100.),
            &theme(),
        );
        let [PaintOp::Path { points, stroke, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert!(stroke.is_some());
        let [PathPoint::MoveTo(a), PathPoint::LineTo(b)] = points.as_slice() else {
            panic!("{points:?}");
        };
        assert!(near(*a, 0., 10.), "{a:?}");
        assert!(near(*b, 0., 20.), "{b:?}");
    }

    #[test]
    fn text_takes_only_translate_and_scale() {
        let ops = plan(
            &[
                transform((5., 5.), (2., 2.), 45.),
                text(1., 1., "hi", 10.),
                Draw::PopTransform,
            ],
            canvas(100., 100.),
            &theme(),
        );
        let [PaintOp::Text { origin, size, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert!(near(*origin, 7., 7.), "{origin:?}");
        assert!(close(f32::from(*size), 20.), "{size:?}");
    }

    #[test]
    fn a_rect_without_rotation_is_a_quad_with_rotation_a_path() {
        let ops = plan(&[rect(10., 10., 20., 20.)], canvas(100., 100.), &theme());
        let [PaintOp::Quad { bounds, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert_bounds(*bounds, 10., 10., 20., 20.);

        let ops = plan(
            &[rotate(30.), rect(10., 10., 20., 20.), Draw::PopTransform],
            canvas(100., 100.),
            &theme(),
        );
        let [PaintOp::Path { points, fill, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert!(fill.is_some());
        // Its first corner, (10, 10) turned 30° clockwise.
        let (sin, cos) = 30f32.to_radians().sin_cos();
        let Some(PathPoint::MoveTo(first)) = points.first() else {
            panic!("{points:?}");
        };
        assert!(
            near(*first, 10. * cos - 10. * sin, 10. * sin + 10. * cos),
            "{first:?}"
        );
    }

    #[test]
    fn a_circle_is_a_rounded_quad() {
        let circle = Draw::Ellipse(EllipseCmd {
            cx: 10.,
            cy: 10.,
            rx: 5.,
            ry: 5.,
            style: solid(0xff0000ff),
        });
        let ops = plan(&[circle], canvas(100., 100.), &theme());
        let [PaintOp::Quad { bounds, radius, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert_bounds(*bounds, 5., 5., 10., 10.);
        assert_eq!(*radius, px(5.));
    }

    #[test]
    fn an_ellipse_is_four_cubics() {
        let ellipse = Draw::Ellipse(EllipseCmd {
            cx: 50.,
            cy: 50.,
            rx: 20.,
            ry: 10.,
            style: solid(0xff0000ff),
        });
        let ops = plan(&[ellipse], canvas(100., 100.), &theme());
        let [PaintOp::Path { points, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        let cubics = points
            .iter()
            .filter(|p| matches!(p, PathPoint::CubicTo { .. }))
            .count();
        assert_eq!(cubics, 4, "{points:?}");
    }

    #[test]
    fn clip_under_rotation_is_its_bounding_box() {
        let ops = plan(
            &[
                transform((20., 20.), (1., 1.), 45.),
                clip(0., 0., 10., 10.),
                Draw::PopClip,
                Draw::PopTransform,
            ],
            canvas(100., 30.),
            &theme(),
        );
        let [PaintOp::Clip(clip), PaintOp::Unclip] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        // x from 20 - 7.07 to 20 + 7.07; y from 20 to 34.14, cut at the
        // canvas's bottom, 30.
        assert_bounds(*clip, 12.93, 20., 14.14, 10.);
    }

    #[test]
    fn nested_clips_intersect() {
        let ops = plan(
            &[
                clip(0., 0., 50., 50.),
                clip(25., 25., 50., 50.),
                rect(30., 30., 5., 5.),
                Draw::PopClip,
                Draw::PopClip,
            ],
            canvas(100., 100.),
            &theme(),
        );
        let [
            PaintOp::Clip(outer),
            PaintOp::Clip(inner),
            PaintOp::Quad { .. },
            PaintOp::Unclip,
            PaintOp::Unclip,
        ] = ops.as_slice()
        else {
            panic!("{ops:?}");
        };
        assert_bounds(*outer, 0., 0., 50., 50.);
        assert_bounds(*inner, 25., 25., 25., 25.);
    }

    #[test]
    fn opacity_multiplies_alpha() {
        let half = Draw::Rect(RectCmd {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
            radius: 0.,
            style: ShapeStyle {
                opacity: 0.5,
                ..solid(0xff0000ff)
            },
        });
        let ops = plan(&[half], canvas(100., 100.), &theme());
        let [
            PaintOp::Quad {
                fill: Some(fill), ..
            },
        ] = ops.as_slice()
        else {
            panic!("{ops:?}");
        };
        let fill = fill.as_solid().expect("a solid fill");
        assert!(close(fill.a, 0.5), "{fill:?}");
    }

    #[test]
    fn commands_outside_the_canvas_are_culled() {
        let far_path = Draw::Path(PathCmd {
            segments: vec![
                PathSegment::MoveTo(Point { x: -50., y: -50. }),
                PathSegment::LineTo(Point { x: -40., y: -40. }),
            ],
            style: solid(0x000000ff),
        });
        let ops = plan(
            &[
                rect(200., 0., 10., 10.),
                rect(0., 60., 10., 10.),
                text(0., 60., "below", 10.),
                text(120., 0., "right", 10.),
                far_path,
                // Inside the canvas, outside the clip.
                clip(0., 0., 10., 10.),
                rect(20., 20., 5., 5.),
                Draw::PopClip,
                // Under a translation that takes it out.
                transform((500., 0.), (1., 1.), 0.),
                rect(0., 0., 10., 10.),
                Draw::PopTransform,
            ],
            canvas(100., 50.),
            &theme(),
        );
        assert!(
            matches!(ops.as_slice(), [PaintOp::Clip(_), PaintOp::Unclip]),
            "{ops:?}"
        );
    }

    #[test]
    fn canvas_text_is_one_line_in_the_theme_font() {
        let mut mono = text(0., 0., "a\nb\tc", 10.);
        if let Draw::Text(t) = &mut mono {
            t.mono = true;
        }
        let mut named = text(0., 20., "named", 10.);
        if let Draw::Text(t) = &mut named {
            t.font = Some("Named".into());
            t.mono = true;
        }
        let ops = plan(
            &[mono, named, text(0., 30., "ui", 10.)],
            canvas(100., 50.),
            &theme(),
        );
        let texts: Vec<(&str, &str)> = ops
            .iter()
            .map(|op| match op {
                PaintOp::Text { text, family, .. } => (text.as_ref(), family.as_ref()),
                _ => panic!("{op:?}"),
            })
            .collect();
        assert_eq!(texts, [("a b c", "Mono"), ("named", "Named"), ("ui", "UI")]);
    }

    #[test]
    fn a_scaled_text_is_at_most_the_largest_text_size() {
        let ops = plan(
            &[
                transform((0., 0.), (100., 100.), 0.),
                text(0., 0., "huge", 200.),
                Draw::PopTransform,
            ],
            canvas(100., 50.),
            &theme(),
        );
        let [PaintOp::Text { size, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert_eq!(*size, px(MAX_TEXT_SIZE));
    }

    #[test]
    fn overflowing_transforms_draw_nothing_and_keep_clips_balanced() {
        // 32 nested scales of 100: 10^64 overflows f32.
        let mut commands: Vec<Draw> = (0..32)
            .map(|_| transform((0., 0.), (100., 100.), 0.))
            .collect();
        commands.extend([
            rect(0., 0., 1., 1.),
            text(0., 0., "t", 10.),
            clip(0., 0., 1., 1.),
            rect(0., 0., 1., 1.),
            Draw::PopClip,
        ]);
        commands.extend((0..32).map(|_| Draw::PopTransform));
        commands.push(rect(0., 0., 10., 10.));
        let ops = plan(&commands, canvas(100., 50.), &theme());
        let [
            PaintOp::Clip(empty),
            PaintOp::Unclip,
            PaintOp::Quad { bounds, .. },
        ] = ops.as_slice()
        else {
            panic!("{ops:?}");
        };
        assert!(empty.size.width <= px(0.) || empty.size.height <= px(0.));
        assert_bounds(*bounds, 0., 0., 10., 10.);
    }

    #[test]
    fn affine_composes_inner_first() {
        let outer = Affine::from(&Transform {
            translate: Point { x: 10., y: 0. },
            scale: Point { x: 1., y: 1. },
            rotate: 90.,
        });
        let inner = Affine::from(&Transform {
            translate: Point { x: 5., y: 0. },
            scale: Point { x: 2., y: 3. },
            rotate: 0.,
        });
        let p = outer.then(&inner).apply(Point { x: 1., y: 1. });
        // Inner: (1, 1) → (7, 3); outer: turned to (-3, 7), moved to (7, 7).
        assert!(close(p.x, 7.) && close(p.y, 7.), "{p:?}");
        let (sx, sy) = outer.then(&inner).scale_only();
        assert!(close(sx, 2.) && close(sy, 3.), "{sx} {sy}");
        let id = Affine::identity().apply(Point { x: 3., y: 4. });
        assert!(close(id.x, 3.) && close(id.y, 4.));
    }

    fn gradient(angle: f32) -> ShapeStyle {
        ShapeStyle {
            paint: Paint::Linear(LinearGradient {
                angle,
                start: ColorStop {
                    color: Color::Rgba(0xff0000ff),
                    at: 0.,
                },
                end: ColorStop {
                    color: Color::Rgba(0x0000ffff),
                    at: 1.,
                },
            }),
            stroke: None,
            opacity: 1.,
        }
    }

    fn styled_rect(x: f32, y: f32, width: f32, height: f32, style: ShapeStyle) -> Draw {
        Draw::Rect(RectCmd {
            x,
            y,
            width,
            height,
            radius: 0.,
            style,
        })
    }

    /// The angle of a gradient paint, which GPUI keeps private but prints.
    fn gradient_angle(paint: &Background) -> f32 {
        let printed = format!("{paint:?}");
        let angle = printed
            .strip_prefix("LinearGradient(")
            .and_then(|rest| rest.split(',').next())
            .unwrap_or_else(|| panic!("not a gradient: {printed}"));
        angle.trim().parse().unwrap()
    }

    /// Whether `a` and `b` are the same angle, within a hundredth of a degree.
    fn same_angle(a: f32, b: f32) -> bool {
        let d = (a - b).rem_euclid(360.);
        d.min(360. - d) <= 0.01
    }

    /// GPUI spreads a gradient over the quad's own bounds: one cut to the
    /// clip would be stretched.
    #[test]
    fn a_gradient_quad_keeps_its_bounds_past_the_clip() {
        let ops = plan(
            &[styled_rect(-100., 0., 400., 50., gradient(90.))],
            canvas(100., 50.),
            &theme(),
        );
        let [PaintOp::Quad { bounds, fill, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert_bounds(*bounds, -100., 0., 400., 50.);
        assert!(same_angle(gradient_angle(&fill.unwrap()), 90.));
        // A solid one is cut: it looks the same.
        let ops = plan(&[rect(-100., 0., 400., 50.)], canvas(100., 50.), &theme());
        let [PaintOp::Quad { bounds, .. }] = ops.as_slice() else {
            panic!("{ops:?}");
        };
        assert!(bounds.size.width < px(400.), "{bounds:?}");
    }

    #[test]
    fn a_gradient_stroke_takes_the_path_route() {
        let stroked = ShapeStyle {
            stroke: Some(2.),
            ..gradient(0.)
        };
        let ops = plan(
            &[styled_rect(10., 10., 20., 20., stroked)],
            canvas(100., 50.),
            &theme(),
        );
        let [
            PaintOp::Path {
                stroke: Some((width, paint)),
                fill: None,
                ..
            },
        ] = ops.as_slice()
        else {
            panic!("{ops:?}");
        };
        assert_eq!(*width, px(2.));
        assert!(same_angle(gradient_angle(paint), 0.));
    }

    #[test]
    fn a_gradient_turns_with_the_shape() {
        let turned = |transform: Draw, angle: f32| {
            let ops = plan(
                &[
                    transform,
                    styled_rect(10., 10., 20., 20., gradient(angle)),
                    Draw::PopTransform,
                ],
                canvas(100., 100.),
                &theme(),
            );
            match ops.as_slice() {
                [
                    PaintOp::Path {
                        fill: Some(paint), ..
                    },
                ] => gradient_angle(paint),
                _ => panic!("{ops:?}"),
            }
        };
        assert!(same_angle(turned(rotate(30.), 90.), 120.));
        // Mirrored left to right: up stays up, right becomes left.
        let mirror = || transform((50., 0.), (-1., 1.), 0.);
        assert!(same_angle(turned(mirror(), 180.), 180.));
        assert!(same_angle(turned(mirror(), 90.), 270.));
        assert!(same_angle(turned(mirror(), 45.), 315.));
    }

    #[test]
    fn a_text_size_that_is_not_finite_is_not_drawn() {
        assert_eq!(text_size(10., (2., 2.)), Some(20.));
        assert_eq!(text_size(200., (100., 100.)), Some(MAX_TEXT_SIZE));
        assert_eq!(text_size(10., (f32::NAN, 1.)), None);
        assert_eq!(text_size(10., (f32::INFINITY, 1.)), None);
    }

    /// Past 2^22 px, f32 coordinates are too coarse to tessellate.
    #[test]
    fn a_path_reaching_past_two_to_the_22_is_not_drawn() {
        let path = |far: f32| {
            Draw::Path(PathCmd {
                segments: vec![
                    PathSegment::MoveTo(Point { x: 10., y: 10. }),
                    PathSegment::LineTo(Point { x: far, y: 20. }),
                    PathSegment::LineTo(Point { x: 10., y: 30. }),
                ],
                style: solid(0x000000ff),
            })
        };
        let near = plan(&[path(4_000_000.)], canvas(100., 50.), &theme());
        assert!(
            matches!(near.as_slice(), [PaintOp::Path { .. }]),
            "{near:?}"
        );
        let far = plan(&[path(5_000_000.)], canvas(100., 50.), &theme());
        assert!(far.is_empty(), "{far:?}");
    }
}
