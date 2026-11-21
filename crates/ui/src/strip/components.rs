//! Components: a widget frame's nodes as GPUI elements inside its tile.
//!
//! A frame is a flat list of nodes, each naming an earlier container as its
//! parent, so the elements are built from the last node to the first: every
//! child is built before its parent takes it. Every node clips what it
//! holds and may shrink to nothing, and the tile clips the whole frame, so
//! no text or size a module sends reaches past its tile. A canvas node's
//! commands are painted by [`super::canvas`]; an image node's image comes
//! from [`super::images`], and draws nothing until it is decoded. A click
//! on a button presses it; the focused button or canvas shows a focus ring.

use std::sync::Arc;

use gpui_kit::{
    AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, ObjectFit, ParentElement as _, Pixels,
    Position, Role, SharedString, Stateful, StatefulInteractiveElement as _, Style, Styled as _,
    StyledImage as _, TestSupportExt as _, WeakEntity, Window, div, img, px, relative, rgba,
};
use tomakast_core::{
    Align, Color, ColorToken, Edges, Frame, Icon, ImageFit, ImageFormat, ImageNode, Justify,
    Layout, Node, NodeKind, TextAlign, TextNode, TextStyle, TileKey, WidgetImage, WidgetTheme,
};

use super::canvas::{self, CanvasPaints, CanvasPointer};
use super::images::{self, FrameImages};
use crate::icons::IconSpec;
use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};
use crate::views::icon_element;

/// The longest length a node, its padding, gap or icon is laid out with: a
/// frame is checked for finite lengths only, and 256 nodes of the largest
/// `f32` would add up to infinity. Larger than any tile.
const MAX_LENGTH: f32 = 4096.;
/// The largest grow factor, for the same reason.
const MAX_GROW: f32 = 1_000_000.;
/// The largest icon: a tile's height. GPUI rasterizes an icon, an emoji
/// glyph or a monogram letter at its size, on the UI thread.
const MAX_ICON_SIZE: f32 = theme::STRIP_TILE_HEIGHT;
/// Height of a progress track.
const PROGRESS_HEIGHT: f32 = 4.;
/// Corner radius of a button.
const BUTTON_RADIUS: f32 = 6.;
/// Space around a button's content.
const BUTTON_PADDING: (f32, f32) = (8., 4.);
/// Space between a button's icon and its label.
const BUTTON_GAP: f32 = 4.;
/// Size of a button's icon.
const BUTTON_ICON_SIZE: f32 = 16.;

/// Element id of node `ix` of the frame of the tile named `name` (see
/// [`tile_name`](super::tile_name)): `widget-node:<name>:<ix>`.
pub(crate) fn node_element_id(name: &str, ix: usize) -> ElementId {
    ElementId::Name(SharedString::from(format!("widget-node:{name}:{ix}")))
}

/// Element id of the box image node `ix` of the frame of the tile named `name` draws its
/// image in, once decoded: `widget-image:<name>:<ix>`.
pub(crate) fn image_element_id(name: &str, ix: usize) -> ElementId {
    ElementId::Name(SharedString::from(format!("widget-image:{name}:{ix}")))
}

/// Element id of the focus ring of node `ix` of the frame of the tile named `name`, there
/// only while that node has the keyboard focus: `widget-focus:<name>:<ix>`.
pub(crate) fn part_focus_element_id(name: &str, ix: usize) -> ElementId {
    ElementId::Name(SharedString::from(format!("widget-focus:{name}:{ix}")))
}

/// What the nodes of a tile's frame talk to: the canvases' pointer events
/// wait in `pointer`, and a pressed button goes to `view`.
#[derive(Clone)]
pub(crate) struct FrameInput {
    pub pointer: CanvasPointer,
    pub view: WeakEntity<LauncherView>,
}

/// `c` in the launcher's colors: a token is the theme's color of that name.
pub(crate) fn color(c: &Color, theme: &WidgetTheme) -> Hsla {
    let colors = &theme.colors;
    let packed = match *c {
        Color::Rgba(packed) => packed,
        Color::Token(ColorToken::Text) => colors.text,
        Color::Token(ColorToken::Muted) => colors.muted,
        Color::Token(ColorToken::Panel) => colors.panel,
        Color::Token(ColorToken::Border) => colors.border,
        Color::Token(ColorToken::Selected) => colors.selected,
        Color::Token(ColorToken::Error) => colors.error,
        Color::Token(ColorToken::Success) => colors.success,
    };
    rgba(packed).into()
}

/// `frame` of `key`'s tile, named `name` in its element ids, as one element filling the
/// tile. `focused` is the
/// id of the node with the keyboard focus, which shows a focus ring;
/// `images` holds the images the frame names.
#[expect(
    clippy::too_many_arguments,
    reason = "what one frame is drawn with, each its own"
)]
pub(crate) fn frame_element(
    key: &TileKey,
    name: &str,
    frame: &Arc<Frame>,
    theme: &WidgetTheme,
    focused: Option<&str>,
    palette: &Palette,
    paints: &CanvasPaints,
    images: &FrameImages,
    input: &FrameInput,
) -> AnyElement {
    let nodes = &frame.nodes;
    // A checked frame names an earlier container as every parent; a node
    // whose parent is not one is left out with its subtree.
    let parent = |ix: usize| -> Option<usize> {
        let p = nodes[ix].parent? as usize;
        (p < ix && container(&nodes[p]).is_some()).then_some(p)
    };
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for ix in 1..nodes.len() {
        if let Some(p) = parent(ix) {
            children[p].push(ix);
        }
    }
    let cx = NodeCx {
        key,
        name,
        frame,
        theme,
        focused,
        palette,
        paints,
        images,
        input,
    };
    let mut built: Vec<Option<AnyElement>> = (0..nodes.len()).map(|_| None).collect();
    for ix in (0..nodes.len()).rev() {
        let kids: Vec<AnyElement> = children[ix]
            .iter()
            .filter_map(|&child| built[child].take())
            .collect();
        // An overlay's first child is laid out in it and gives it its
        // size; the later ones are stacked on it, inset by its padding.
        let overlay = parent(ix).and_then(|p| match &nodes[p].kind {
            NodeKind::Overlay(layout) if children[p].first() != Some(&ix) => Some(layout.padding),
            _ => None,
        });
        built[ix] = Some(node_element(&cx, ix, &nodes[ix], overlay, kids));
    }
    built
        .into_iter()
        .next()
        .flatten()
        .unwrap_or_else(|| div().into_any_element())
}

/// What every node of one frame is drawn with.
struct NodeCx<'a> {
    key: &'a TileKey,
    /// The tile's name in element ids.
    name: &'a str,
    /// Shared with the canvases, which paint from it.
    frame: &'a Arc<Frame>,
    theme: &'a WidgetTheme,
    focused: Option<&'a str>,
    palette: &'a Palette,
    /// Where the canvases add how long they take.
    paints: &'a CanvasPaints,
    images: &'a FrameImages,
    input: &'a FrameInput,
}

fn container(node: &Node) -> Option<&Layout> {
    match &node.kind {
        NodeKind::Row(layout) | NodeKind::Column(layout) | NodeKind::Overlay(layout) => {
            Some(layout)
        }
        _ => None,
    }
}

/// `v` as a length to lay out: at most [`MAX_LENGTH`].
fn length(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0., MAX_LENGTH)
    } else {
        0.
    }
}

/// Node `ix`, holding `children`; `overlay` is the padding of its parent
/// when that is an overlay and the node is not its first child.
fn node_element(
    cx: &NodeCx<'_>,
    ix: usize,
    node: &Node,
    overlay: Option<Edges>,
    children: Vec<AnyElement>,
) -> AnyElement {
    let el = sized(cx.name, ix, node, overlay);
    let el = match &node.kind {
        NodeKind::Row(layout) => laid_out(el.flex().flex_row(), layout),
        NodeKind::Column(layout) => laid_out(el.flex().flex_col(), layout),
        // Its later children are placed in it (see `sized`); one stacked
        // on another overlay already is, being absolute.
        NodeKind::Overlay(layout) if overlay.is_some() => laid_out(el.flex().flex_col(), layout),
        NodeKind::Overlay(layout) => laid_out(el.relative().flex().flex_col(), layout),
        NodeKind::Text(text) => text_element(el, text, cx.theme),
        NodeKind::Icon(icon) => el
            .flex()
            .items_center()
            .justify_center()
            .child(icon_element(
                IconSpec {
                    icon: Some(&icon.icon),
                    monogram: "",
                },
                length(icon.size).min(MAX_ICON_SIZE),
                cx.palette,
            )),
        NodeKind::Progress(value) => {
            let el = if node.width.is_none() {
                el.w_full()
            } else {
                el
            };
            let el = if node.height.is_none() {
                el.h(px(PROGRESS_HEIGHT))
            } else {
                el
            };
            el.rounded_full().bg(cx.palette.chip).child(
                div()
                    .h_full()
                    .w(relative(if value.is_finite() {
                        value.clamp(0., 1.)
                    } else {
                        0.
                    }))
                    .rounded_full()
                    .bg(cx.palette.text),
            )
        }
        NodeKind::Button(button) => {
            let label = button
                .label
                .clone()
                .or_else(|| button.icon.as_ref().map(icon_name))
                .unwrap_or_else(|| node.id.clone());
            let palette = cx.palette;
            let (key, id, action) = (cx.key.clone(), node.id.clone(), button.action.clone());
            let view = cx.input.view.clone();
            let focused = cx.focused == Some(node.id.as_str());
            el.role(Role::Button)
                .aria_label(label.clone())
                .aria_selected(focused)
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(BUTTON_GAP))
                .px(px(BUTTON_PADDING.0))
                .py(px(BUTTON_PADDING.1))
                .rounded(px(BUTTON_RADIUS))
                .bg(palette.chip)
                .hover(|style| style.bg(palette.selected))
                .text_size(px(text_size(TextStyle::Body)))
                .font_family(cx.theme.ui_font.clone())
                .text_color(palette.text)
                // Not a click on the tile as well.
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    let _ = view.update(cx, |view, cx| view.press_button(&key, &id, &action, cx));
                })
                .children(button.icon.as_ref().map(|icon| {
                    icon_element(
                        IconSpec {
                            icon: Some(icon),
                            monogram: &label,
                        },
                        BUTTON_ICON_SIZE,
                        palette,
                    )
                }))
                .children(
                    button
                        .label
                        .as_ref()
                        .map(|text| div().min_w_0().truncate().child(one_line(text))),
                )
        }
        NodeKind::Canvas(_) => el.child(canvas::canvas_element(
            cx.key,
            cx.frame.clone(),
            ix,
            cx.theme.clone(),
            cx.paints.clone(),
            cx.images.clone(),
            cx.input.pointer.clone(),
        )),
        NodeKind::Image(image) => el.child(image_element(cx, ix, image)),
        NodeKind::Spacer => el,
    };
    // Drawn over the node's content, inside its box.
    let ring = (cx.focused == Some(node.id.as_str()) && !node.id.is_empty()).then(|| {
        div()
            .id(part_focus_element_id(cx.name, ix))
            .test_support()
            .absolute()
            .inset_0()
            .rounded(px(match node.kind {
                NodeKind::Button(_) => BUTTON_RADIUS,
                _ => 0.,
            }))
            .border(px(theme::FOCUS_RING_WIDTH))
            .border_color(super::focus_color(cx.palette))
    });
    el.children(children)
        .children(ring)
        .test_support()
        .into_any_element()
}

/// The box of node `ix`: its id, its size and grow; it clips what it holds
/// and may shrink to nothing. Node 0 fills the tile. A later child of an
/// overlay (`overlay`) is stacked on its first, covering the overlay inset
/// by its padding: an absolute box is placed in its parent's padding box,
/// which the padding alone would not shrink.
fn sized(name: &str, ix: usize, node: &Node, overlay: Option<Edges>) -> Stateful<Div> {
    let mut el = div()
        .id(node_element_id(name, ix))
        .min_w_0()
        .min_h_0()
        .overflow_hidden();
    if ix == 0 {
        el = el.size_full();
    } else {
        // Never wider than its parent: a box that is not stretched takes
        // its content's width, and a text that does not wrap would be as
        // wide as all of it, not truncated.
        el = el.max_w_full();
    }
    if let Some(inset) = overlay {
        el = el
            .absolute()
            .top(px(length(inset.top)))
            .right(px(length(inset.right)))
            .bottom(px(length(inset.bottom)))
            .left(px(length(inset.left)));
    }
    if node.grow > 0. {
        el = el.flex_grow(node.grow.min(MAX_GROW));
    } else if node.width.is_some() {
        // Only a width: a node with a height alone must still shrink across
        // a row, or a long text would not truncate.
        el = el.flex_none();
    }
    if let Some(width) = node.width {
        el = el.w(px(length(width)));
    }
    if let Some(height) = node.height {
        el = el.h(px(length(height)));
    }
    el
}

/// Image node `ix`: a box filling the node, which draws the image in it once
/// it is decoded, and nothing before or if it never is.
fn image_element(cx: &NodeCx<'_>, ix: usize, node: &ImageNode) -> AnyElement {
    div()
        .id(image_element_id(cx.name, ix))
        .test_support()
        .absolute()
        .inset_0()
        .child(ImageNodeElement {
            provider: cx.key.widget.provider.clone(),
            image: cx.images.get(&cx.key.widget.provider, &node.image).cloned(),
            fit: node.fit,
            radius: length(node.radius),
        })
        .into_any_element()
}

/// An image node's image, with its version; it fills its box. A PNG or JPEG
/// keeps its own pixels and GPUI fits them; an SVG is rasterized fitted to
/// the box, in device pixels, which it knows only once laid out.
struct ImageNodeElement {
    provider: String,
    image: Option<(u64, WidgetImage)>,
    fit: ImageFit,
    radius: f32,
}

impl IntoElement for ImageNodeElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ImageNodeElement {
    type RequestLayoutState = ();
    /// The image, laid out over the box, once decoded.
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

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
    ) -> Option<AnyElement> {
        let (version, image) = self.image.as_ref()?;
        let (raster, fit) = match image.format {
            // Rasterized at the node's box, fitted already: a raster
            // of the box, no larger, that GPUI draws as it is.
            ImageFormat::Svg => {
                let device = images::device_size(bounds.size, bounds.size, window.scale_factor())?;
                let raster = images::svg(&self.provider, image, *version, device, self.fit, cx)?;
                (raster, ObjectFit::Fill)
            }
            ImageFormat::Png | ImageFormat::Jpeg => {
                let raster = images::bitmap(&self.provider, image, *version, cx)?;
                let fit = match self.fit {
                    ImageFit::Contain => ObjectFit::Contain,
                    ImageFit::Cover => ObjectFit::Cover,
                    ImageFit::Fill => ObjectFit::Fill,
                };
                (raster, fit)
            }
        };
        // GPUI's image element has no role of its own: its box has.
        let mut element = div()
            .id("image")
            .role(Role::Image)
            .aria_label(image.id.clone())
            .test_support()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .child(
                img(raster)
                    .size_full()
                    .rounded(px(self.radius))
                    .object_fit(fit),
            )
            .into_any_element();
        element.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
        Some(element)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        image: &mut Option<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(image) = image {
            image.paint(window, cx);
        }
    }
}

/// A row's or column's padding, gap and alignment.
fn laid_out(el: Stateful<Div>, layout: &Layout) -> Stateful<Div> {
    let Edges {
        top,
        right,
        bottom,
        left,
    } = layout.padding;
    let el = el
        .pt(px(length(top)))
        .pr(px(length(right)))
        .pb(px(length(bottom)))
        .pl(px(length(left)))
        .gap(px(length(layout.gap)));
    let el = match layout.align {
        Align::Start => el.items_start(),
        Align::Center => el.items_center(),
        Align::End => el.items_end(),
        Align::Stretch => el.items_stretch(),
    };
    match layout.justify {
        Justify::Start => el.justify_start(),
        Justify::Center => el.justify_center(),
        Justify::End => el.justify_end(),
        Justify::SpaceBetween => el.justify_between(),
    }
}

/// The font size of `style`.
fn text_size(style: TextStyle) -> f32 {
    let (title, value, body, caption) = theme::WIDGET_TEXT_SIZES;
    match style {
        TextStyle::Title => title,
        TextStyle::Value => value,
        TextStyle::Body => body,
        TextStyle::Caption => caption,
    }
}

/// A text node: at most `max_lines` lines (0 is 1), wrapped and line
/// breaks alike, ending in an ellipsis when cut. Its label is the whole text.
fn text_element(el: Stateful<Div>, text: &TextNode, theme: &WidgetTheme) -> Stateful<Div> {
    let el = el
        .role(Role::Label)
        .aria_label(text.text.clone())
        .text_size(px(text_size(text.style)));
    let el = if text.mono {
        el.font_family(theme.mono_font.clone())
            .font_features(crate::theme::mono_features())
    } else {
        el.font_family(theme.ui_font.clone())
    };
    let el = el.text_color(color(&text.color, theme));
    let el = match text.align {
        TextAlign::Start => el,
        TextAlign::Center => el.text_center(),
        TextAlign::End => el.text_right(),
    };
    let lines = usize::from(text.max_lines.max(1));
    if lines == 1 {
        el.truncate().child(one_line(&text.text))
    } else {
        // With an ellipsis, the clamp counts line breaks as well as wraps.
        el.line_clamp(lines)
            .text_ellipsis()
            .child(text.text.clone())
    }
}

/// `text` on one line: a line break shows as a space. A one-line text
/// does not wrap, but it would still break at every `\n`.
fn one_line(text: &str) -> String {
    text.replace('\n', " ")
}

/// What a button with only an icon is called: the icon's name, emoji or
/// colour, or its file's name.
fn icon_name(icon: &Icon) -> String {
    match icon {
        Icon::Named(name) | Icon::Emoji(name) | Icon::Color(name) => name.clone(),
        Icon::File(path) | Icon::PluginFile(path) => path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::palette_for;
    use gpui_kit::Rgba;
    use tomakast_core::TextSizes;

    fn theme_for(palette: &Palette) -> WidgetTheme {
        WidgetTheme {
            dark: false,
            colors: super::super::theme_colors(palette),
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

    fn close(a: Hsla, b: Hsla) -> bool {
        let (a, b) = (Rgba::from(a), Rgba::from(b));
        [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
            .iter()
            .all(|(x, y)| (x - y).abs() <= 0.01)
    }

    #[test]
    fn colors_resolve_tokens_and_rgba() {
        let palette = palette_for(false);
        let theme = theme_for(&palette);

        let red = color(&Color::Rgba(0xff000080), &theme);
        assert!(red.h.abs() <= 0.01, "{red:?}");
        assert!((red.s - 1.).abs() <= 0.01, "{red:?}");
        assert!((red.l - 0.5).abs() <= 0.01, "{red:?}");
        assert!((red.a - 0.5).abs() <= 0.01, "{red:?}");

        for dark in [false, true] {
            let palette = palette_for(dark);
            let theme = theme_for(&palette);
            let muted = color(&Color::Token(ColorToken::Muted), &theme);
            assert!(close(muted, palette.muted), "{muted:?} {:?}", palette.muted);
            let text = color(&Color::Token(ColorToken::Text), &theme);
            assert!(close(text, palette.text), "{text:?} {:?}", palette.text);
        }
    }

    #[test]
    fn one_line_shows_line_breaks_as_spaces() {
        assert_eq!(one_line("a\nb\n"), "a b ");
    }
}
