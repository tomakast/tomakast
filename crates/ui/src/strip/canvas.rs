//! The native canvas: a canvas node's draw commands painted with GPUI's
//! quads, paths, text and images, from the ops [`plan`] makes of them. An
//! image draws once [`super::images`] has decoded it, and nothing before.
//!
//! A canvas with an id hears the pointer: a press and a release over it, and
//! the pointer coming onto it, moving over it and leaving it, in its own
//! logical pixels. They wait in [`CanvasPointer`] for the strip's next tick,
//! which sends them; the moves of one frame go as one.

mod plan;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyElement, App, BorderStyle, Bounds, ContentMask, Corners, DispatchPhase, Element, ElementId,
    FillOptions, FillRule, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    PathBuilder, PathStyle, Pixels, Point, Position, RenderImage, ShapedLine, Size, StrokeOptions,
    Style, Styled as _, StyledImage as _, TextAlign, TextRun, Window, font, img, px, quad,
    transparent_black,
};
use tomakast_core::{
    Frame, ImageFit, ImageFormat, NodeKind, PointerEvent, PointerKind, TileKey, WidgetTheme,
};

use super::images::{self, FrameImages};

pub(crate) use plan::{LINE_HEIGHT, PaintOp, PathPoint, plan};
use plan::{bounding_box, path_corners};

/// Paths are flattened to lines at most this far from the curve, in
/// logical pixels; or at [`TOLERANCE_PER_EXTENT`] of the path's size, when
/// that is more, so a huge path cannot flatten to millions of lines.
const TOLERANCE: f32 = 0.1;
const TOLERANCE_PER_EXTENT: f32 = 1e-4;
/// How many times a stroke too long for one tessellation is halved.
const MAX_SPLITS: u32 = 8;
/// How much coarser a fill too big for one tessellation is tried again.
const COARSER_FILL: f32 = 4.;

/// What the strip's canvases share across frames: how long each tile's
/// canvases took to prepaint and paint since it was last read, and the
/// widgets that logged a path they could not draw.
#[derive(Clone, Default)]
pub(crate) struct CanvasPaints(Rc<RefCell<Paints>>);

#[derive(Default)]
struct Paints {
    took: HashMap<TileKey, Duration>,
    logged: HashSet<TileKey>,
}

impl CanvasPaints {
    pub(super) fn add(&self, key: &TileKey, took: Duration) {
        let mut paints = self.0.borrow_mut();
        match paints.took.get_mut(key) {
            Some(total) => *total += took,
            None => {
                paints.took.insert(key.clone(), took);
            }
        }
    }

    /// How long each tile's canvases took since the last call.
    pub(crate) fn take(&self) -> HashMap<TileKey, Duration> {
        std::mem::take(&mut self.0.borrow_mut().took)
    }

    /// Logs, once per widget, that `key`'s canvas has a path it cannot draw.
    fn dropped(&self, key: &TileKey) {
        if self.0.borrow_mut().logged.insert(key.clone()) {
            tracing::warn!(widget = %key, "a canvas path is too big to tessellate and is not drawn");
        }
    }
}

/// The canvases' pointer events not yet sent, and where the pointer is.
#[derive(Clone, Default)]
pub(crate) struct CanvasPointer(Rc<RefCell<Pointer>>);

#[derive(Default)]
struct Pointer {
    /// In order; a canvas has at most one move after its last other event.
    queue: Vec<(TileKey, PointerEvent)>,
    /// The canvases under the pointer, by tile and node id, with where it
    /// last was on each.
    hovered: HashMap<(TileKey, String), tomakast_core::Point>,
    /// Where the pointer is over the strip, in window coordinates.
    over: Option<Point<Pixels>>,
    /// A frame passed since the moves were queued: they may go.
    due: bool,
    /// A frame is asked for the moves queued.
    asked: bool,
}

impl CanvasPointer {
    /// The pointer is at `at` over the strip, in window coordinates, or
    /// left it (`None`).
    pub(crate) fn set_over(&self, at: Option<Point<Pixels>>) {
        self.0.borrow_mut().over = at;
    }

    pub(crate) fn over(&self) -> Option<Point<Pixels>> {
        self.0.borrow().over
    }

    /// The node id of `key`'s canvas under the pointer, if any.
    pub(crate) fn hovered(&self, key: &TileKey) -> Option<String> {
        let pointer = self.0.borrow();
        pointer
            .hovered
            .keys()
            .find(|(tile, _)| tile == key)
            .map(|(_, node)| node.clone())
    }

    /// The canvases under the pointer that `present` says are no longer
    /// drawn get their leave, where the pointer last was on them; they go on
    /// the next send.
    pub(crate) fn leave_gone(&self, present: impl Fn(&TileKey, &str) -> bool) {
        let gone: Vec<((TileKey, String), tomakast_core::Point)> = {
            let mut pointer = self.0.borrow_mut();
            let gone = pointer
                .hovered
                .iter()
                .filter(|((key, node), _)| !present(key, node))
                .map(|(entry, at)| (entry.clone(), *at))
                .collect::<Vec<_>>();
            for (entry, _) in &gone {
                pointer.hovered.remove(entry);
            }
            pointer.due |= !gone.is_empty();
            gone
        };
        for ((key, node), at) in gone {
            self.push(&key, &node, PointerKind::Leave, at);
        }
    }

    /// The events to send now, in order: all of them once a frame passed
    /// since the moves were queued, or once a press or a release is queued;
    /// none before.
    pub(crate) fn take(&self) -> Vec<(TileKey, PointerEvent)> {
        let mut pointer = self.0.borrow_mut();
        let pressed = pointer
            .queue
            .iter()
            .any(|(_, e)| matches!(e.kind, PointerKind::Down | PointerKind::Up));
        if !(pointer.due || pressed) {
            return Vec::new();
        }
        pointer.due = false;
        std::mem::take(&mut pointer.queue)
    }

    /// Forgets the events not sent: the strip is not on screen.
    pub(crate) fn clear(&self) {
        let mut pointer = self.0.borrow_mut();
        pointer.queue.clear();
        pointer.hovered.clear();
        pointer.over = None;
    }

    /// Queues `kind` at `at` on canvas `node` of `key`; a move replaces the
    /// canvas's last event when that is a move too.
    fn push(&self, key: &TileKey, node: &str, kind: PointerKind, at: tomakast_core::Point) {
        let mut pointer = self.0.borrow_mut();
        if kind == PointerKind::Move
            && let Some((_, last)) = pointer
                .queue
                .iter_mut()
                .rev()
                .find(|(tile, e)| tile == key && e.node == node)
            && last.kind == PointerKind::Move
        {
            last.at = at;
            return;
        }
        pointer.queue.push((
            key.clone(),
            PointerEvent {
                node: node.to_string(),
                kind,
                at,
            },
        ));
    }

    /// The pointer moved: canvas `node` of `key`, at `at`, is now under it
    /// (`hovered`) or not; queues its enter, move or leave. Returns whether
    /// it queued any.
    fn moved(&self, key: &TileKey, node: &str, hovered: bool, at: tomakast_core::Point) -> bool {
        let entry = (key.clone(), node.to_string());
        let was = self.0.borrow().hovered.contains_key(&entry);
        match (was, hovered) {
            (false, true) => {
                self.0.borrow_mut().hovered.insert(entry, at);
                self.push(key, node, PointerKind::Enter, at);
                self.push(key, node, PointerKind::Move, at);
            }
            (true, true) => {
                self.0.borrow_mut().hovered.insert(entry, at);
                self.push(key, node, PointerKind::Move, at);
            }
            (true, false) => {
                self.0.borrow_mut().hovered.remove(&entry);
                self.push(key, node, PointerKind::Leave, at);
            }
            (false, false) => return false,
        }
        true
    }

    /// Asks once for the next frame, after which the queued moves go.
    fn ask_frame(&self, window: &mut Window) {
        if std::mem::replace(&mut self.0.borrow_mut().asked, true) {
            return;
        }
        let pointer = self.clone();
        window.on_next_frame(move |window, _| {
            let mut state = pointer.0.borrow_mut();
            state.asked = false;
            state.due = true;
            window.refresh();
        });
    }
}

/// Node `ix` of `frame`, a canvas of `key`'s tile, filling its node's box;
/// it adds the time it takes to `paints`, draws the images it names from
/// `images`, and queues its pointer events in `pointer`.
pub(crate) fn canvas_element(
    key: &TileKey,
    frame: Arc<Frame>,
    ix: usize,
    theme: WidgetTheme,
    paints: CanvasPaints,
    images: FrameImages,
    pointer: CanvasPointer,
) -> AnyElement {
    CanvasElement {
        key: key.clone(),
        frame,
        ix,
        theme,
        paints,
        images,
        pointer,
    }
    .into_any_element()
}

/// The frame is shared, not copied: a canvas holds up to 10 000 commands
/// and paints up to 60 times a second.
struct CanvasElement {
    key: TileKey,
    frame: Arc<Frame>,
    ix: usize,
    theme: WidgetTheme,
    paints: CanvasPaints,
    images: FrameImages,
    pointer: CanvasPointer,
}

/// A canvas's ops, its texts shaped in op order, what each image op draws,
/// once decoded, and where it takes the pointer, when it has an id.
struct Prepared {
    ops: Vec<PaintOp>,
    lines: Vec<ShapedLine>,
    images: Vec<Option<CanvasImage>>,
    hitbox: Option<Hitbox>,
}

/// An image op's image, ready to paint.
enum CanvasImage {
    /// Painted as it is, at the op's bounds.
    Opaque(Arc<RenderImage>),
    /// Painted through an element, laid out at the op's bounds: GPUI paints
    /// an image at no opacity of its own, an element's does.
    Faded(AnyElement),
}

/// How an image op draws: an SVG command an SVG, an image command a PNG or
/// JPEG; through an element when it is translucent. `None` draws nothing:
/// the other kind of image, or an opacity of zero or less.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Route {
    Svg { faded: bool },
    Bitmap { faded: bool },
}

fn route(format: ImageFormat, svg: bool, opacity: f32) -> Option<Route> {
    if opacity.is_nan() || opacity <= 0. {
        return None;
    }
    let faded = opacity < 1.;
    match (format, svg) {
        (ImageFormat::Svg, true) => Some(Route::Svg { faded }),
        (ImageFormat::Png | ImageFormat::Jpeg, false) => Some(Route::Bitmap { faded }),
        _ => None,
    }
}

impl CanvasElement {
    /// What image `op` draws: `None` until it is decoded, and when [`route`]
    /// draws nothing. An SVG is rasterized at its bounds, never larger than
    /// `cap`, in device pixels.
    fn image(
        &self,
        op: &PaintOp,
        cap: Size<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<CanvasImage> {
        let &PaintOp::Image {
            bounds,
            ref image,
            opacity,
            svg,
        } = op
        else {
            return None;
        };
        let provider = &self.key.widget.provider;
        let (version, image) = self.images.get(provider, image)?;
        let (raster, faded) = match route(image.format, svg, opacity)? {
            Route::Svg { faded } => {
                let device = images::device_size(bounds.size, cap, window.scale_factor())?;
                let fill = ImageFit::Fill;
                (
                    images::svg(provider, image, *version, device, fill, cx)?,
                    faded,
                )
            }
            Route::Bitmap { faded } => (images::bitmap(provider, image, *version, cx)?, faded),
        };
        if !faded {
            return Some(CanvasImage::Opaque(raster));
        }
        let mut element = img(raster)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .object_fit(ObjectFit::Fill)
            .opacity(opacity)
            .into_any_element();
        element.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
        Some(CanvasImage::Faded(element))
    }
}

impl IntoElement for CanvasElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for CanvasElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    /// The node's box, which takes the node's size and grow, holds the
    /// canvas over all of it.
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let zero = || px(0.).into();
        let style = Style {
            position: Position::Absolute,
            inset: gpui_kit::Edges {
                top: zero(),
                right: zero(),
                bottom: zero(),
                left: zero(),
            },
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepared {
        let start = Instant::now();
        let commands = match self.frame.nodes.get(self.ix).map(|node| &node.kind) {
            Some(NodeKind::Canvas(canvas)) => canvas.commands.as_slice(),
            _ => &[],
        };
        let ops = plan(commands, bounds, &self.theme);
        let text_system = window.text_system().clone();
        let lines = ops
            .iter()
            .filter_map(|op| match op {
                PaintOp::Text {
                    text,
                    size,
                    color,
                    family,
                    mono,
                    ..
                } => {
                    let mut font = font(family.clone());
                    if *mono {
                        font.features = crate::theme::mono_features();
                    }
                    let run = TextRun {
                        len: text.len(),
                        font,
                        color: *color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    Some(text_system.shape_line(text.clone(), *size, &[run], None))
                }
                _ => None,
            })
            .collect();
        // An SVG never rasterizes larger than the canvas.
        let mut images = Vec::new();
        for op in &ops {
            if matches!(op, PaintOp::Image { .. }) {
                images.push(self.image(op, bounds.size, window, cx));
            }
        }
        self.paints.add(&self.key, start.elapsed());
        let named = self
            .frame
            .nodes
            .get(self.ix)
            .is_some_and(|node| !node.id.is_empty());
        let hitbox = named.then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal));
        Prepared {
            ops,
            lines,
            images,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepared: &mut Prepared,
        window: &mut Window,
        cx: &mut App,
    ) {
        let start = Instant::now();
        let mut whole = true;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // Every clip is already inside the one before, so the last one
            // open is the whole mask of what follows.
            let mut clips: Vec<Bounds<Pixels>> = Vec::new();
            let mut lines = prepared.lines.iter();
            let mut images = prepared.images.iter_mut();
            for op in &prepared.ops {
                match op {
                    PaintOp::Clip(clip) => clips.push(*clip),
                    PaintOp::Unclip => {
                        clips.pop();
                    }
                    op => {
                        let line = match op {
                            PaintOp::Text { .. } => lines.next(),
                            _ => None,
                        };
                        let image = match op {
                            PaintOp::Image { .. } => images.next().and_then(Option::as_mut),
                            _ => None,
                        };
                        let mask = clips.last().map(|&bounds| ContentMask { bounds });
                        window.with_content_mask(mask, |window| match (image, op) {
                            (Some(CanvasImage::Faded(image)), _) => image.paint(window, cx),
                            (Some(CanvasImage::Opaque(raster)), PaintOp::Image { bounds, .. }) => {
                                // An error is an image the atlas could not take.
                                let _ = window.paint_image(
                                    *bounds,
                                    *bounds,
                                    Corners::default(),
                                    raster.clone(),
                                    0,
                                    false,
                                );
                            }
                            _ => whole &= paint_op(op, line, window, cx),
                        });
                    }
                }
            }
        });
        if !whole {
            self.paints.dropped(&self.key);
        }
        self.paints.add(&self.key, start.elapsed());
        if let Some(hitbox) = prepared.hitbox.take() {
            self.listen(hitbox, window);
        }
    }
}

impl CanvasElement {
    /// Queues the pointer events of this canvas, which has an id: a press
    /// or release over it goes no further, so it is not a click on its tile.
    fn listen(&self, hitbox: Hitbox, window: &mut Window) {
        let node = self.frame.nodes[self.ix].id.clone();
        let local = move |hitbox: &Hitbox, at: Point<Pixels>| tomakast_core::Point {
            x: f32::from(at.x - hitbox.bounds.origin.x),
            y: f32::from(at.y - hitbox.bounds.origin.y),
        };
        let (key, pointer, id, area) = (
            self.key.clone(),
            self.pointer.clone(),
            node.clone(),
            hitbox.clone(),
        );
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
            if phase == DispatchPhase::Bubble
                && pointer.moved(&key, &id, area.is_hovered(window), local(&area, e.position))
            {
                pointer.ask_frame(window);
            }
        });
        let press = move |kind: PointerKind| {
            let (key, pointer, id, area) = (
                self.key.clone(),
                self.pointer.clone(),
                node.clone(),
                hitbox.clone(),
            );
            move |button: MouseButton, at: Point<Pixels>, window: &mut Window, cx: &mut App| {
                if button == MouseButton::Left && area.is_hovered(window) {
                    pointer.push(&key, &id, kind, local(&area, at));
                    cx.stop_propagation();
                    window.refresh();
                }
            }
        };
        let down = press(PointerKind::Down);
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                down(e.button, e.position, window, cx);
            }
        });
        let up = press(PointerKind::Up);
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                up(e.button, e.position, window, cx);
            }
        });
    }
}

/// Paints `op`; returns whether all of it could be.
fn paint_op(op: &PaintOp, line: Option<&ShapedLine>, window: &mut Window, cx: &mut App) -> bool {
    match op {
        PaintOp::Quad {
            bounds,
            radius,
            fill,
            border,
        } => {
            let (width, border_color) = border.unwrap_or((px(0.), transparent_black()));
            window.paint_quad(quad(
                *bounds,
                *radius,
                fill.unwrap_or_else(|| transparent_black().into()),
                width,
                border_color,
                BorderStyle::Solid,
            ));
        }
        PaintOp::Path {
            points,
            fill,
            stroke,
        } => {
            let ((pieces, whole), paint) = match (stroke, fill) {
                (Some((width, paint)), _) => (tessellate(points, Some(*width)), *paint),
                (None, Some(paint)) => (tessellate(points, None), *paint),
                (None, None) => return true,
            };
            for piece in pieces {
                window.paint_path(piece, paint);
            }
            return whole;
        }
        PaintOp::Text { origin, size, .. } => {
            if let Some(line) = line {
                // An error is a glyph that could not be drawn: the rest is.
                let _ = line.paint(
                    *origin,
                    *size * LINE_HEIGHT,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
        PaintOp::Image { .. } | PaintOp::Clip(_) | PaintOp::Unclip => {}
    }
    true
}

/// `points` filled, or stroked `stroke` wide, as GPUI paths, and whether
/// that is all of it. GPUI tessellates into at most 65 536 vertices: a
/// stroke past that is halved, up to [`MAX_SPLITS`] times, and a fill is
/// tried once more [`COARSER_FILL`] times coarser.
fn tessellate(points: &[PathPoint], stroke: Option<Pixels>) -> (Vec<gpui_kit::Path<Pixels>>, bool) {
    let mut pieces = Vec::new();
    let whole = match stroke {
        Some(width) => stroke_pieces(points, width, MAX_SPLITS, &mut pieces),
        None => {
            match build_path(points, None, 1.).or_else(|| build_path(points, None, COARSER_FILL)) {
                Some(path) => {
                    pieces.push(path);
                    true
                }
                None => false,
            }
        }
    };
    (pieces, whole)
}

fn stroke_pieces(
    points: &[PathPoint],
    width: Pixels,
    splits: u32,
    pieces: &mut Vec<gpui_kit::Path<Pixels>>,
) -> bool {
    if let Some(path) = build_path(points, Some(width), 1.) {
        pieces.push(path);
        return true;
    }
    if splits == 0 || points.len() < 2 {
        return false;
    }
    let (first, second) = split_points(points);
    // Both halves, even when the first fails.
    let first = stroke_pieces(&first, width, splits - 1, pieces);
    stroke_pieces(&second, width, splits - 1, pieces) && first
}

/// `points` cut in two at their middle. The second half starts where the
/// first ended; a close in it before its next move goes back to its
/// subpath's start as a line.
fn split_points(points: &[PathPoint]) -> (Vec<PathPoint>, Vec<PathPoint>) {
    let middle = points.len() / 2;
    let (first, rest) = points.split_at(middle);
    let (mut start, mut current) = (None, None);
    for p in first {
        match *p {
            PathPoint::MoveTo(to) => (start, current) = (Some(to), Some(to)),
            PathPoint::LineTo(to)
            | PathPoint::QuadTo { to, .. }
            | PathPoint::CubicTo { to, .. } => current = Some(to),
            PathPoint::Close => current = start,
        }
    }
    let mut second = Vec::with_capacity(rest.len() + 1);
    let continues = !matches!(rest.first(), Some(PathPoint::MoveTo(_)));
    if continues && let Some(current) = current {
        second.push(PathPoint::MoveTo(current));
    }
    let mut open = continues;
    for p in rest {
        match *p {
            PathPoint::MoveTo(_) => {
                open = false;
                second.push(*p);
            }
            PathPoint::Close if open => {
                open = false;
                if let Some(start) = start {
                    second.push(PathPoint::LineTo(start));
                }
            }
            _ => second.push(*p),
        }
    }
    (first.to_vec(), second)
}

/// `points` filled (nonzero, as SVG fills), or stroked `stroke` wide, at
/// `coarser` times the usual tolerance; `None` when GPUI cannot tessellate
/// it.
fn build_path(
    points: &[PathPoint],
    stroke: Option<Pixels>,
    coarser: f32,
) -> Option<gpui_kit::Path<Pixels>> {
    let extent = bounding_box(&path_corners(points))
        .map_or(0., |b| f32::from(b.size.width.max(b.size.height)));
    let tolerance = TOLERANCE.max(extent * TOLERANCE_PER_EXTENT) * coarser;
    let style = match stroke {
        Some(width) => PathStyle::Stroke(
            StrokeOptions::default()
                .with_line_width(f32::from(width))
                .with_tolerance(tolerance),
        ),
        None => PathStyle::Fill(
            FillOptions::default()
                .with_fill_rule(FillRule::NonZero)
                .with_tolerance(tolerance),
        ),
    };
    let mut builder = PathBuilder::fill().with_style(style);
    for p in points {
        match *p {
            PathPoint::MoveTo(to) => builder.move_to(to),
            PathPoint::LineTo(to) => builder.line_to(to),
            PathPoint::QuadTo { ctrl, to } => builder.curve_to(to, ctrl),
            PathPoint::CubicTo { ctrl_a, ctrl_b, to } => {
                builder.cubic_bezier_to(to, ctrl_a, ctrl_b)
            }
            PathPoint::Close => builder.close(),
        }
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, size};
    use tomakast_core::{
        Color, Draw, Paint, PathCmd, PathSegment, Point, RectCmd, ShapeStyle, TextSizes,
        ThemeColors, Transform,
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

    fn solid(rgba: u32) -> ShapeStyle {
        ShapeStyle {
            paint: Paint::Solid(Color::Rgba(rgba)),
            stroke: None,
            opacity: 1.,
        }
    }

    fn rotate(degrees: f32) -> Draw {
        Draw::PushTransform(Transform {
            translate: Point { x: 0., y: 0. },
            scale: Point { x: 1., y: 1. },
            rotate: degrees,
        })
    }

    /// An image op draws only the kind of image its command names, and only
    /// a translucent one goes through an element.
    #[test]
    fn image_ops_route_by_kind_and_opacity() {
        use ImageFormat::{Jpeg, Png, Svg};
        assert_eq!(route(Svg, true, 1.), Some(Route::Svg { faded: false }));
        assert_eq!(route(Png, false, 0.5), Some(Route::Bitmap { faded: true }));
        assert_eq!(route(Jpeg, false, 1.), Some(Route::Bitmap { faded: false }));
        assert_eq!(route(Svg, false, 1.), None);
        assert_eq!(route(Png, true, 1.), None);
        assert_eq!(route(Png, false, 0.), None);
        assert_eq!(route(Png, false, f32::NAN), None);
    }

    /// The path route draws: what [`plan`] gives a rotated rounded rect,
    /// and a stroked path of the cap's line segments, tessellate.
    #[test]
    fn planned_paths_tessellate() {
        let rounded = Draw::Rect(RectCmd {
            radius: 4.,
            x: 10.,
            y: 10.,
            width: 30.,
            height: 20.,
            style: solid(0xff0000ff),
        });
        let at = |ix: usize| ((ix % 50) as f32 * 7., (ix / 50 % 20) as f32 * 6.);
        let mut segments = vec![PathSegment::MoveTo(Point { x: 0., y: 0. })];
        segments.extend((1..9_997).map(|ix| {
            let (x, y) = at(ix);
            PathSegment::LineTo(Point { x, y })
        }));
        let long = Draw::Path(PathCmd {
            segments,
            style: ShapeStyle {
                stroke: Some(1.),
                ..solid(0x000000ff)
            },
        });
        for command in [rounded, long] {
            let ops = plan(
                &[rotate(15.), command, Draw::PopTransform],
                canvas(355., 120.),
                &theme(),
            );
            let [PaintOp::Path { points, stroke, .. }] = ops.as_slice() else {
                panic!("{ops:?}");
            };
            let path = build_path(points, stroke.map(|(width, _)| width), 1.).expect("tessellated");
            assert!(!path.vertices.is_empty());
        }
    }

    /// `count` S-curves across a tile, one after another.
    fn s_curves(count: usize) -> Vec<PathPoint> {
        let at = |x: f32, y: f32| point(px(x), px(y));
        let mut points = vec![PathPoint::MoveTo(at(0., 60.))];
        for ix in 0..count {
            let (from, to) = if ix % 2 == 0 { (0., 355.) } else { (355., 0.) };
            points.push(PathPoint::CubicTo {
                ctrl_a: at(from, 0.),
                ctrl_b: at(to, 120.),
                to: at(to, 60.),
            });
        }
        points
    }

    /// A stroke too long for one tessellation (65 536 vertices) is drawn
    /// in pieces.
    #[test]
    fn a_long_curvy_stroke_draws_in_pieces() {
        let points = s_curves(600);
        assert!(
            build_path(&points, Some(px(2.)), 1.).is_none(),
            "one piece fits"
        );

        let (pieces, whole) = tessellate(&points, Some(px(2.)));
        assert!(whole);
        assert!(pieces.len() > 1, "{}", pieces.len());
        assert!(pieces.iter().all(|piece| !piece.vertices.is_empty()));
    }

    /// Each piece starts where the one before ended, and a close in a later
    /// piece still goes back to its subpath's start.
    #[test]
    fn split_points_continue_where_they_left_off() {
        let at = |x: f32, y: f32| point(px(x), px(y));
        let points = [
            PathPoint::MoveTo(at(0., 0.)),
            PathPoint::LineTo(at(10., 0.)),
            PathPoint::LineTo(at(10., 10.)),
            PathPoint::Close,
        ];
        let (first, second) = split_points(&points);
        assert_eq!(first, points[..2]);
        assert_eq!(
            second,
            [
                PathPoint::MoveTo(at(10., 0.)),
                PathPoint::LineTo(at(10., 10.)),
                PathPoint::LineTo(at(0., 0.)),
            ]
        );
    }
}
