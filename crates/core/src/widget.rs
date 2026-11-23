//! Widgets: tiles a module draws, as a flat frame of nodes, plus the
//! checks of structure and numbers a frame must pass. GPUI-free.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::index::{Action, Icon, Item};
use crate::settings::Modifiers;

/// Widgets one plugin may declare.
pub const MAX_WIDGETS: usize = 16;
/// Widget answers one `search` may carry.
pub const MAX_WIDGET_ANSWERS: usize = 16;
/// Bytes of all of one tile's parameter values together.
pub const MAX_PAYLOAD_BYTES: usize = 4096;
/// Nodes in one frame.
pub const MAX_FRAME_NODES: usize = 256;
/// Ancestors a node may have: nodes nest at most this deep.
pub const MAX_NODE_DEPTH: usize = 32;
/// Canvas commands plus path segments over all canvases of one frame.
pub const MAX_FRAME_COMMANDS: usize = 10_000;
/// [`frame_bytes`] of one frame.
pub const MAX_FRAME_BYTES: usize = 1 << 20;
/// Nesting of clips and transforms inside one canvas.
pub const MAX_DRAW_NESTING: usize = 32;
/// The largest size of a canvas text, in logical pixels; the UI caps a text
/// its transform scales up at it too.
pub const MAX_TEXT_SIZE: f32 = 200.;
/// Bytes of one image.
pub const MAX_IMAGE_BYTES: usize = 2 << 20;
/// Bytes of one SVG.
pub const MAX_SVG_BYTES: usize = 512 << 10;
/// Elements of one SVG. The host counts the XML's; the launcher counts the
/// parsed document's nodes again, which also holds what `<use>` copies.
pub const MAX_SVG_ELEMENTS: usize = 4096;
/// Filter primitives (elements whose name starts with `fe`) of one SVG.
pub const MAX_SVG_FILTERS: usize = 32;
/// Nodes (elements, text runs, comments) an SVG parser reads before it gives
/// up, so a hostile document stops early instead of being parsed whole and
/// counted after. Text runs and comments count too, so a formatted document
/// at [`MAX_SVG_ELEMENTS`] needs about twice as many nodes; four per element
/// leaves room for that.
pub const MAX_SVG_NODES: usize = 4 * MAX_SVG_ELEMENTS;
/// Filter primitives an SVG applies in all: each filtered element counts
/// its filter's primitives, so a filter shared by many elements counts many
/// times.
pub const MAX_SVG_FILTER_APPLICATIONS: usize = 32;
/// `numOctaves` of an SVG `feTurbulence`.
pub const MAX_SVG_OCTAVES: u32 = 10;
/// The radius of an SVG `feMorphology`, in device pixels (the host checks it
/// in the document's own units).
pub const MAX_SVG_MORPHOLOGY_RADIUS: f32 = 32.;
/// The `order` of an SVG `feConvolveMatrix`, on each side.
pub const MAX_SVG_CONVOLVE_ORDER: u32 = 9;
/// Raster images (`data:` PNG or JPEG) an SVG draws, each `<use>` of one
/// counting again (the host counts the `<image>` elements).
pub const MAX_SVG_IMAGES: usize = 16;
/// Pixels of all the raster images an SVG draws, decoded.
pub const MAX_SVG_IMAGE_PIXELS: u64 = 4 << 20;
/// Images per plugin.
pub const MAX_IMAGES: usize = 64;
/// Bytes of all images of one plugin.
pub const MAX_IMAGES_BYTES: usize = 8 << 20;
/// At most 60 renders per second per tile.
pub const MIN_RENDER_INTERVAL: Duration = Duration::from_micros(16_667);
/// A frame that took longer, in the worker thread's CPU time, is dropped
/// (see [`soft_render_deadline`]).
pub const SOFT_RENDER_DEADLINE: Duration = Duration::from_millis(16);
/// How many times [`SOFT_RENDER_DEADLINE`] a debug build of the launcher
/// gives a render, as the UI's paint and raster budgets do.
pub const DEBUG_DEADLINE_FACTOR: u32 = 4;
/// Consecutive late or failed frames that stop a widget.
pub const LATE_FRAMES_TO_STOP: u32 = 5;

/// The soft deadline a render is held to, in the CPU time of the worker
/// thread that runs the module call (wall time where there is no thread
/// clock): [`SOFT_RENDER_DEADLINE`], [`DEBUG_DEADLINE_FACTOR`] times as long
/// in a debug build. CPU time, so a loaded machine that takes the thread off
/// its core for a while does not make a quick render late.
pub fn soft_render_deadline() -> Duration {
    if cfg!(debug_assertions) {
        SOFT_RENDER_DEADLINE * DEBUG_DEADLINE_FACTOR
    } else {
        SOFT_RENDER_DEADLINE
    }
}

/// Bytes every node, command and path segment counts in [`frame_bytes`].
const UNIT_BYTES: usize = 16;

/// How wide a tile is; every tile is as high as the strip. Four quarters,
/// two singles or one double fill the strip's width, and two quarters with
/// the gap between them are one single.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WidgetSize {
    Quarter,
    Single,
    Double,
}

impl WidgetSize {
    /// Parses the name used in `config.toml`: `quarter`, `single` or `double`.
    pub fn parse(s: &str) -> Option<WidgetSize> {
        match s {
            "quarter" => Some(WidgetSize::Quarter),
            "single" => Some(WidgetSize::Single),
            "double" => Some(WidgetSize::Double),
            _ => None,
        }
    }

    /// The name used in `config.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            WidgetSize::Quarter => "quarter",
            WidgetSize::Single => "single",
            WidgetSize::Double => "double",
        }
    }
}

/// The settings of one Home tile (a city, units), as key and value pairs, always sorted by
/// key with each key once, so equal settings compare equal.
pub type Params = Arc<[(String, String)]>;

/// Builds [`Params`]: sorts by key and keeps the first of a repeated key.
pub fn params(pairs: impl IntoIterator<Item = (String, String)>) -> Params {
    let mut pairs: Vec<(String, String)> = pairs.into_iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs.dedup_by(|later, first| later.0 == first.0);
    pairs.into()
}

/// A widget a module declares.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetDecl {
    pub id: String,
    pub title: String,
    /// The sizes the widget can be drawn in, without repeats; the first is its default.
    pub sizes: Vec<WidgetSize>,
    /// The keys of the parameters a tile of it takes (a city, units).
    pub params: Vec<String>,
}

impl WidgetDecl {
    /// The size a tile of it has unless one is asked for: the first of its sizes.
    pub fn default_size(&self) -> WidgetSize {
        self.sizes.first().copied().unwrap_or(WidgetSize::Single)
    }

    /// `size` if the widget offers it, else its default size.
    pub fn size_or_default(&self, size: Option<WidgetSize>) -> WidgetSize {
        size.filter(|s| self.sizes.contains(s))
            .unwrap_or_else(|| self.default_size())
    }

    /// The pairs of `params` whose key the widget declares: what a tile passes to it.
    pub fn declared_params(&self, params: &Params) -> Params {
        if params.iter().all(|(k, _)| self.params.contains(k)) {
            return params.clone();
        }
        params
            .iter()
            .filter(|(k, _)| self.params.contains(k))
            .cloned()
            .collect()
    }
}

/// A widget of a provider: `<provider>/<widget>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetKey {
    pub provider: String,
    pub widget: String,
}

impl WidgetKey {
    pub fn new(provider: impl Into<String>, widget: impl Into<String>) -> WidgetKey {
        WidgetKey {
            provider: provider.into(),
            widget: widget.into(),
        }
    }

    /// Parses `<provider>/<widget>`; both parts are non-empty and contain no `/`.
    pub fn parse(s: &str) -> Option<WidgetKey> {
        let (provider, widget) = s.split_once('/')?;
        if provider.is_empty() || widget.is_empty() || widget.contains('/') {
            return None;
        }
        Some(WidgetKey::new(provider, widget))
    }
}

impl fmt::Display for WidgetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.widget)
    }
}

/// Where a tile of the strip comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// A search's answer: at most one per widget at a time.
    Search,
    /// An entry of the user's Home: the number of entries with the same widget and the same
    /// shown parameters before it, so identical entries are 0, 1, 2…
    Home(u32),
}

/// One tile of the strip: a widget, the parameters it is drawn with, and where it comes from.
/// Two tiles of one widget with different parameters, or two identical Home entries, are two
/// tiles.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub widget: WidgetKey,
    pub params: Params,
    pub slot: Slot,
}

impl fmt::Display for TileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.widget)?;
        for (k, v) in self.params.iter() {
            write!(f, " {k}={v:?}")?;
        }
        match self.slot {
            Slot::Search => f.write_str(" (search)"),
            Slot::Home(n) => write!(f, " (home {n})"),
        }
    }
}

/// A condition tile a `search` answers with.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetAnswer {
    pub widget: String,
    /// Declared keys only, each once.
    pub params: Params,
    /// One of the widget's sizes; `None`: its default size.
    pub size: Option<WidgetSize>,
    pub actions: Vec<Action>,
}

/// What a `search` answers: rows and tiles.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchAnswer {
    pub items: Vec<Item>,
    pub widgets: Vec<WidgetAnswer>,
    /// Internal, for built-in modules: there is no WIT field, so a plugin's answer is never
    /// busy. The module is still getting ready to answer this query in full (the file index
    /// is being built): the root list shows the busy line until its next answer is not busy.
    /// A module that answers busy sends [`Effect::SearchAgain`](crate::Effect) once ready.
    pub busy: bool,
}

impl From<Vec<Item>> for SearchAnswer {
    fn from(items: Vec<Item>) -> SearchAnswer {
        SearchAnswer {
            items,
            widgets: vec![],
            busy: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorToken {
    Text,
    Muted,
    Panel,
    Border,
    Selected,
    Error,
    Success,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Token(ColorToken),
    /// `0xRRGGBBAA`.
    Rgba(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeColors {
    pub text: u32,
    pub muted: u32,
    pub panel: u32,
    pub border: u32,
    pub selected: u32,
    pub error: u32,
    pub success: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSizes {
    pub title: f32,
    pub value: f32,
    pub body: f32,
    pub caption: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetTheme {
    pub dark: bool,
    pub colors: ThemeColors,
    pub ui_font: String,
    pub mono_font: String,
    pub sizes: TextSizes,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerAt {
    pub node: Option<String>,
    pub at: Point,
}

/// What a render call knows about the tile and its surroundings.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetContext {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub theme: WidgetTheme,
    pub millis_shown: u64,
    pub focused: Option<String>,
    pub pointer: Option<PointerAt>,
    /// The search's query, for a search tile only.
    pub query: Option<String>,
    pub params: Params,
}

/// One drawn state of a tile: a flat list of nodes, node 0 the root.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub nodes: Vec<Node>,
    pub animate: bool,
    pub actions: Vec<Action>,
    pub images: Vec<WidgetImage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    /// `None` for node 0 only; otherwise an earlier container node.
    pub parent: Option<u32>,
    pub grow: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Row(Layout),
    Column(Layout),
    Overlay(Layout),
    Text(TextNode),
    Icon(IconNode),
    Image(ImageNode),
    Progress(f32),
    Spacer,
    Button(ButtonNode),
    Canvas(CanvasNode),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub padding: Edges,
    pub gap: f32,
    pub align: Align,
    pub justify: Justify,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextNode {
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
    pub align: TextAlign,
    pub max_lines: u8,
    pub mono: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Title,
    Value,
    Body,
    Caption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconNode {
    pub icon: Icon,
    pub size: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageNode {
    pub image: String,
    pub fit: ImageFit,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFit {
    Contain,
    Cover,
    Fill,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonNode {
    pub label: Option<String>,
    pub icon: Option<Icon>,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasNode {
    pub focusable: bool,
    pub commands: Vec<Draw>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Rect(RectCmd),
    Ellipse(EllipseCmd),
    Path(PathCmd),
    Text(TextCmd),
    Image(ImageCmd),
    Svg(ImageCmd),
    PushClip(ClipRect),
    PopClip,
    PushTransform(Transform),
    PopTransform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapeStyle {
    pub paint: Paint,
    pub stroke: Option<f32>,
    pub opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    Linear(LinearGradient),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub angle: f32,
    pub start: ColorStop,
    pub end: ColorStop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub color: Color,
    pub at: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectCmd {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub style: ShapeStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EllipseCmd {
    pub cx: f32,
    pub cy: f32,
    pub rx: f32,
    pub ry: f32,
    pub style: ShapeStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathCmd {
    pub segments: Vec<PathSegment>,
    pub style: ShapeStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSegment {
    MoveTo(Point),
    LineTo(Point),
    QuadTo {
        ctrl: Point,
        to: Point,
    },
    CubicTo {
        ctrl_a: Point,
        ctrl_b: Point,
        to: Point,
    },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextCmd {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub color: Color,
    pub font: Option<String>,
    pub mono: bool,
    pub opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageCmd {
    pub image: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Applied as scale, then rotate (degrees, clockwise), then translate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translate: Point,
    pub scale: Point,
    pub rotate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Svg,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetImage {
    pub id: String,
    pub format: ImageFormat,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WidgetEvent {
    Shown,
    Hidden,
    Action {
        node: Option<String>,
        action: String,
    },
    Pointer(PointerEvent),
    Key(KeyEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerEvent {
    pub node: String,
    pub kind: PointerKind,
    pub at: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Down,
    Up,
    Move,
    Enter,
    Leave,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub node: String,
    pub key: String,
    pub modifiers: Modifiers,
}

/// Why a frame is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameError(pub String);

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FrameError {}

fn err<T>(message: impl Into<String>) -> Result<T, FrameError> {
    Err(FrameError(message.into()))
}

fn finite(what: &str, v: f32) -> Result<(), FrameError> {
    if v.is_finite() {
        Ok(())
    } else {
        err(format!("{what} is not a finite number"))
    }
}

fn at_least_zero(what: &str, v: f32) -> Result<(), FrameError> {
    finite(what, v)?;
    if v < 0.0 {
        return err(format!("{what} is negative"));
    }
    Ok(())
}

fn between(what: &str, v: f32, lo: f32, hi: f32) -> Result<(), FrameError> {
    finite(what, v)?;
    if !(lo..=hi).contains(&v) {
        return err(format!("{what} is outside {lo}..={hi}"));
    }
    Ok(())
}

fn unit(what: &str, v: f32) -> Result<(), FrameError> {
    between(what, v, 0.0, 1.0)
}

fn point(what: &str, p: Point) -> Result<(), FrameError> {
    finite(what, p.x)?;
    finite(what, p.y)
}

fn style(s: &ShapeStyle) -> Result<(), FrameError> {
    unit("opacity", s.opacity)?;
    if let Some(stroke) = s.stroke {
        at_least_zero("stroke", stroke)?;
    }
    if let Paint::Linear(g) = &s.paint {
        finite("gradient angle", g.angle)?;
        unit("color stop", g.start.at)?;
        unit("color stop", g.end.at)?;
    }
    Ok(())
}

fn image_cmd(c: &ImageCmd) -> Result<(), FrameError> {
    finite("x", c.x)?;
    finite("y", c.y)?;
    at_least_zero("width", c.width)?;
    at_least_zero("height", c.height)?;
    unit("opacity", c.opacity)
}

fn layout(l: &Layout) -> Result<(), FrameError> {
    at_least_zero("gap", l.gap)?;
    at_least_zero("padding", l.padding.top)?;
    at_least_zero("padding", l.padding.right)?;
    at_least_zero("padding", l.padding.bottom)?;
    at_least_zero("padding", l.padding.left)
}

#[derive(PartialEq)]
enum Scope {
    Clip,
    Transform,
}

/// Checks one canvas; returns the commands and segments it counts.
fn canvas(c: &CanvasNode) -> Result<usize, FrameError> {
    let mut open: Vec<Scope> = Vec::new();
    let mut count = 0;
    for cmd in &c.commands {
        count += 1;
        match cmd {
            Draw::Rect(r) => {
                finite("x", r.x)?;
                finite("y", r.y)?;
                at_least_zero("width", r.width)?;
                at_least_zero("height", r.height)?;
                at_least_zero("radius", r.radius)?;
                style(&r.style)?;
            }
            Draw::Ellipse(e) => {
                finite("cx", e.cx)?;
                finite("cy", e.cy)?;
                at_least_zero("rx", e.rx)?;
                at_least_zero("ry", e.ry)?;
                style(&e.style)?;
            }
            Draw::Path(p) => {
                count += p.segments.len();
                for segment in &p.segments {
                    match segment {
                        PathSegment::MoveTo(a) | PathSegment::LineTo(a) => point("path point", *a)?,
                        PathSegment::QuadTo { ctrl, to } => {
                            point("path point", *ctrl)?;
                            point("path point", *to)?;
                        }
                        PathSegment::CubicTo { ctrl_a, ctrl_b, to } => {
                            point("path point", *ctrl_a)?;
                            point("path point", *ctrl_b)?;
                            point("path point", *to)?;
                        }
                        PathSegment::Close => {}
                    }
                }
                style(&p.style)?;
            }
            Draw::Text(t) => {
                finite("x", t.x)?;
                finite("y", t.y)?;
                between("text size", t.size, 1.0, MAX_TEXT_SIZE)?;
                unit("opacity", t.opacity)?;
            }
            Draw::Image(i) | Draw::Svg(i) => image_cmd(i)?,
            Draw::PushClip(r) => {
                finite("x", r.x)?;
                finite("y", r.y)?;
                at_least_zero("width", r.width)?;
                at_least_zero("height", r.height)?;
                open.push(Scope::Clip);
            }
            Draw::PushTransform(t) => {
                point("translate", t.translate)?;
                between("scale", t.scale.x, 0.001, 100.0)?;
                between("scale", t.scale.y, 0.001, 100.0)?;
                finite("rotate", t.rotate)?;
                open.push(Scope::Transform);
            }
            Draw::PopClip => {
                if open.pop() != Some(Scope::Clip) {
                    return err("unbalanced clips and transforms");
                }
            }
            Draw::PopTransform => {
                if open.pop() != Some(Scope::Transform) {
                    return err("unbalanced clips and transforms");
                }
            }
        }
        if open.len() > MAX_DRAW_NESTING {
            return err(format!(
                "clip and transform nesting is deeper than {MAX_DRAW_NESTING}"
            ));
        }
    }
    if !open.is_empty() {
        return err("unbalanced clips and transforms");
    }
    Ok(count)
}

/// Checks the structure and the numbers of `frame`. Strings are the host's job.
pub fn check_frame(frame: &Frame) -> Result<(), FrameError> {
    if frame.nodes.is_empty() {
        return err("frame has no root node");
    }
    if frame.nodes.len() > MAX_FRAME_NODES {
        return err(format!("frame has more than {MAX_FRAME_NODES} nodes"));
    }
    let mut ids: HashSet<&str> = HashSet::new();
    let mut commands = 0usize;
    // The ancestors of every node so far.
    let mut depths: Vec<usize> = Vec::with_capacity(frame.nodes.len());
    for (index, node) in frame.nodes.iter().enumerate() {
        match node.parent {
            None if index != 0 => {
                return err(format!(
                    "node {index} has no parent: only node 0 is the root"
                ));
            }
            Some(_) if index == 0 => return err("node 0 is the root and has no parent"),
            Some(p) => {
                let parent = frame.nodes.get(p as usize).filter(|_| (p as usize) < index);
                let Some(parent) = parent else {
                    return err(format!(
                        "node {index} has a parent that is not an earlier node"
                    ));
                };
                if !matches!(
                    parent.kind,
                    NodeKind::Row(_) | NodeKind::Column(_) | NodeKind::Overlay(_)
                ) {
                    return err(format!("node {index} has a parent that is not a container"));
                }
            }
            None => {}
        }
        let depth = node.parent.map_or(0, |p| depths[p as usize] + 1);
        if depth > MAX_NODE_DEPTH {
            return err(format!("node {index} nests deeper than {MAX_NODE_DEPTH}"));
        }
        depths.push(depth);
        if !node.id.is_empty() && !ids.insert(&node.id) {
            return err(format!("node id {:?} is used twice", node.id));
        }
        at_least_zero("grow", node.grow)?;
        if let Some(w) = node.width {
            at_least_zero("width", w)?;
        }
        if let Some(h) = node.height {
            at_least_zero("height", h)?;
        }
        match &node.kind {
            NodeKind::Row(l) | NodeKind::Column(l) | NodeKind::Overlay(l) => layout(l)?,
            NodeKind::Text(_) | NodeKind::Spacer => {}
            NodeKind::Icon(i) => at_least_zero("icon size", i.size)?,
            NodeKind::Image(i) => at_least_zero("radius", i.radius)?,
            NodeKind::Progress(v) => unit("progress", *v)?,
            NodeKind::Button(_) => {
                if node.id.is_empty() {
                    return err(format!("button node {index} needs an id"));
                }
            }
            NodeKind::Canvas(c) => {
                if c.focusable && node.id.is_empty() {
                    return err(format!("focusable canvas node {index} needs an id"));
                }
                commands += canvas(c)?;
                if commands > MAX_FRAME_COMMANDS {
                    return err(format!(
                        "frame has more than {MAX_FRAME_COMMANDS} commands and path segments"
                    ));
                }
            }
        }
    }
    let bytes = frame_bytes(frame);
    if bytes > MAX_FRAME_BYTES {
        return err(format!(
            "frame is {bytes} bytes, more than {MAX_FRAME_BYTES}"
        ));
    }
    Ok(())
}

fn action_bytes(a: &Action) -> usize {
    a.id.len() + a.title.len() + a.shortcut.as_ref().map_or(0, String::len)
}

fn icon_bytes(icon: &Icon) -> usize {
    match icon {
        Icon::Named(s) | Icon::Emoji(s) | Icon::Color(s) => s.len(),
        Icon::File(p) | Icon::PluginFile(p) => p.as_os_str().len(),
    }
}

/// The size of `frame` for [`MAX_FRAME_BYTES`]: the length of every string,
/// the image bytes, and 16 per node, command and path segment.
pub fn frame_bytes(frame: &Frame) -> usize {
    let mut total = 0;
    for node in &frame.nodes {
        total += UNIT_BYTES + node.id.len();
        total += match &node.kind {
            NodeKind::Text(t) => t.text.len(),
            NodeKind::Icon(i) => icon_bytes(&i.icon),
            NodeKind::Image(i) => i.image.len(),
            NodeKind::Button(b) => {
                b.action.len()
                    + b.label.as_ref().map_or(0, String::len)
                    + b.icon.as_ref().map_or(0, icon_bytes)
            }
            NodeKind::Canvas(c) => c
                .commands
                .iter()
                .map(|cmd| {
                    UNIT_BYTES
                        + match cmd {
                            Draw::Path(p) => p.segments.len() * UNIT_BYTES,
                            Draw::Text(t) => t.text.len() + t.font.as_ref().map_or(0, String::len),
                            Draw::Image(i) | Draw::Svg(i) => i.image.len(),
                            _ => 0,
                        }
                })
                .sum(),
            _ => 0,
        };
    }
    total += frame.actions.iter().map(action_bytes).sum::<usize>();
    total += frame
        .images
        .iter()
        .map(|i| i.id.len() + i.bytes.len())
        .sum::<usize>();
    total
}
