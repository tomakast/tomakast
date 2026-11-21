//! The widget strip: a row of tiles between the header and the root list,
//! scrolled sideways, each showing its module's last frame.
//!
//! The strip spans the panel; [`layout`] places the tiles so that the next
//! one peeks in at the right margin, and at the left once scrolled. A tile
//! shows the frame the core cached for it, so a reopened launcher draws its
//! tiles before any module renders. The view drives the core's frame clock:
//! every render ticks the core with the tiles in the viewport
//! ([`visible_tiles`]) and asks for the next animation frame while the core
//! says a tile on screen still wants one.
//!
//! The keyboard reaches the tiles as [`keys`] says; the focus is kept here,
//! by tile, not in the tiles' elements, which a tile scrolled away does not
//! build. A click on a tile runs its first action, one on a button presses
//! it, and the canvases' pointer events go out on the next tick.

pub(crate) mod canvas;
pub(crate) mod components;
pub(crate) mod images;
pub(crate) mod keys;
pub(crate) mod layout;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::{
    AnyElement, App, Context, ElementId, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Rgba, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div, point, px,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tomakast_core::strip::{StripTile, VisibleTile, WidgetEnv};
use tomakast_core::{
    Action, Core, CoreUpdate, Frame, PointerAt, RequestId, TextSizes, ThemeColors, WidgetEvent,
    WidgetKey, WidgetSize, WidgetTheme,
};

use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};
use canvas::{CanvasPaints, CanvasPointer};
use components::FrameInput;
use images::FrameImages;
use keys::Focus;

/// The strip's state in the launcher view.
#[derive(Default)]
pub(crate) struct StripState {
    pub scroll: ScrollHandle,
    /// Where the keyboard is at the root; a focused tile and its focused
    /// part show the focus ring.
    pub focus: Focus,
    /// The user moved the root selection since the query changed, so a
    /// condition tile that comes first no longer takes it.
    pub moved: bool,
    /// The canvases' pointer events, sent on the next tick.
    pub pointer: CanvasPointer,
    /// The tiles of the last tick, drawn again by a frame that cannot read
    /// the core.
    pub tiles: Vec<StripTile>,
    /// The scale of the last env sent to the core; `None` before the first,
    /// or when the last could not be sent.
    pub env_scale: Option<f32>,
    /// How long the tiles' canvases take to paint.
    pub paints: CanvasPaints,
    /// By tile: its slow paints in a row.
    pub slow_paints: HashMap<WidgetKey, u32>,
    /// The tiles painted without a break since they came on screen, with
    /// the instance of their module then: their first paint had its grace.
    pub graced: HashMap<WidgetKey, Option<u64>>,
    /// The images the frames of the tiles in the viewport name, as of the
    /// last tick.
    pub images: FrameImages,
}

/// A tile whose canvases take longer than this to paint, this many times
/// in a row, is stopped; so is one that takes [`STALLED_PAINT`] once,
/// except in its first paint after it comes on screen or its module's
/// instance changes, which may shape its text and load fonts cold (that one
/// counts as slow only). The grace is once per showing, not per frame: an
/// animation brings a new frame every paint. A tile not painted in a frame
/// (hidden, scrolled away) starts afresh: its slow run and its grace. A frame at 60 fps has
/// 16 ms for everything. A debug build gets [`DEBUG_BUDGET_FACTOR`] times
/// as long: it paints ~1,000 primitives in ~40 ms.
pub const SLOW_PAINT: Duration = Duration::from_millis(8);
pub const SLOW_PAINTS_TO_STOP: u32 = 5;
pub const STALLED_PAINT: Duration = Duration::from_millis(50);
pub const DEBUG_BUDGET_FACTOR: u32 = 4;

/// How slowly a tile's canvases may paint, summed over one paint of the
/// window, before the strip stops the widget. A global; the default is
/// [`SLOW_PAINT`], [`SLOW_PAINTS_TO_STOP`] and [`STALLED_PAINT`], times
/// [`DEBUG_BUDGET_FACTOR`] in a debug build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPaintBudget {
    /// A paint longer than this is slow.
    pub slow: Duration,
    /// This many slow paints in a row stop the widget.
    pub slow_paints: u32,
    /// One paint longer than this stops it.
    pub stall: Duration,
}

impl Default for CanvasPaintBudget {
    fn default() -> CanvasPaintBudget {
        let factor = if cfg!(debug_assertions) {
            DEBUG_BUDGET_FACTOR
        } else {
            1
        };
        CanvasPaintBudget {
            slow: SLOW_PAINT * factor,
            slow_paints: SLOW_PAINTS_TO_STOP,
            stall: STALLED_PAINT * factor,
        }
    }
}

impl gpui_kit::Global for CanvasPaintBudget {}

/// The width the strip lays its tiles out for, and its viewport: the panel's.
const VIEWPORT: f32 = theme::PANEL_SIZE.0;

/// Element id of `key`'s tile: `widget-tile:<provider>/<widget>`.
pub(crate) fn tile_element_id(key: &WidgetKey) -> ElementId {
    ElementId::Name(SharedString::from(format!("widget-tile:{key}")))
}

/// Element id of `key`'s focus ring, there only while the tile is focused.
fn focus_element_id(key: &WidgetKey) -> ElementId {
    ElementId::Name(SharedString::from(format!("widget-focus:{key}")))
}

/// The tiles of `tiles` that show in the strip's viewport, even in part, at
/// its current scroll offset: those on screen and the peeking ones.
/// Each with its focused part, and where the pointer is over it.
pub(crate) fn visible_tiles(state: &StripState, tiles: &[StripTile]) -> Vec<VisibleTile> {
    let sizes: Vec<WidgetSize> = tiles.iter().map(|tile| tile.size).collect();
    let slots = layout::slots(&sizes, VIEWPORT);
    on_screen(state, &slots)
        .into_iter()
        .map(|ix| {
            let key = &tiles[ix].key;
            let focused = state
                .focus
                .part()
                .filter(|_| state.focus.tile() == Some(key));
            VisibleTile {
                key: key.clone(),
                focused: focused.map(str::to_string),
                pointer: pointer_over(state, ix, key),
            }
        })
        .collect()
}

/// Where the pointer is over tile `ix`, `key`, in its logical pixels, as of
/// the last frame, with the canvas under it.
fn pointer_over(state: &StripState, ix: usize, key: &WidgetKey) -> Option<PointerAt> {
    let at = state.pointer.over()?;
    let tile = state.scroll.bounds_for_item(ix)?;
    tile.contains(&at).then(|| PointerAt {
        node: state.pointer.hovered(key),
        at: tomakast_core::Point {
            x: f32::from(at.x - tile.origin.x),
            y: f32::from(at.y - tile.origin.y),
        },
    })
}

/// Scrolls the strip so that tile `ix` of `tiles` starts at the content edge.
pub(crate) fn scroll_to(state: &StripState, tiles: &[StripTile], ix: usize) {
    let sizes: Vec<WidgetSize> = tiles.iter().map(|tile| tile.size).collect();
    let slots = layout::slots(&sizes, VIEWPORT);
    let offset = layout::offset_for(&slots, ix, VIEWPORT);
    state.scroll.set_offset(point(-px(offset), px(0.)));
}

/// The actions of `tile`: its condition tile's, none while answered for an
/// older query; else those of its frame. As [`Core::widget_actions`] says.
pub(crate) fn tile_actions(tile: &StripTile) -> &[Action] {
    match (&tile.condition, &tile.frame) {
        (Some(condition), _) if condition.stale => &[],
        (Some(condition), _) => &condition.actions,
        (None, Some(frame)) => &frame.actions,
        (None, None) => &[],
    }
}

/// The indices of `slots` that show in the viewport, even in part, at the
/// strip's current scroll offset.
fn on_screen(state: &StripState, slots: &[layout::TileSlot]) -> Vec<usize> {
    // GPUI's offset is ≤ 0: the content moves left as the strip scrolls right.
    // A strip that shrank keeps its old offset until its next layout clamps
    // it; clamped here already, the tiles on screen are not taken for gone.
    let max = (layout::content_width(slots) - VIEWPORT).max(0.);
    let offset = (-f32::from(state.scroll.offset().x)).clamp(0., max);
    layout::visible(slots, offset, VIEWPORT)
}

/// What one tick found: the strip's tiles, whether a tile on screen still
/// wants a frame, the widgets stopped for painting too slowly, to show, and
/// the requests of the pointer events it sent, by provider, to record.
pub(crate) struct Ticked {
    pub tiles: Vec<StripTile>,
    pub wants_frame: bool,
    pub stopped: Vec<CoreUpdate>,
    pub requests: Vec<(String, RequestId)>,
}

/// One tick of the core's frame clock at `now`, with the tiles in the
/// strip's viewport; none on screen unless the root list shows (`root`), so a
/// pushed view hides them. First, a tile whose canvases painted too slowly
/// for `budget` in the last paints is stopped. Reads the images the frames
/// in the viewport name, and drops the decoded images the core no longer
/// has. Returns the strip's tiles (empty when not `root`).
pub(crate) fn tick(
    state: &mut StripState,
    core: &mut Core,
    root: bool,
    now: Instant,
    budget: &CanvasPaintBudget,
    cx: &mut App,
) -> Ticked {
    let stopped = judge_paints(state, core, budget);
    let tiles = if root { core.strip() } else { Vec::new() };
    let requests = if root {
        leave_gone_canvases(state, &tiles);
        send_pointer(state, core)
    } else {
        state.pointer.clear();
        Vec::new()
    };
    let visible = visible_tiles(state, &tiles);
    let wants_frame = core.widget_tick(now, &visible);
    let frames = tiles.iter().filter_map(|tile| {
        let shown = visible.iter().any(|v| v.key == tile.key);
        Some((&tile.key, tile.frame.as_deref().filter(|_| shown)?))
    });
    state.images = FrameImages::of(core, frames);
    images::sweep(
        |provider, id| core.widget_image(provider, id).map(|(version, _)| version),
        cx,
    );
    state.tiles.clone_from(&tiles);
    Ticked {
        tiles,
        wants_frame,
        stopped,
        requests,
    }
}

/// The canvases under the pointer that the tiles on screen no longer draw
/// (a new frame without them, or their tile left) get their leave.
fn leave_gone_canvases(state: &StripState, tiles: &[StripTile]) {
    let visible = visible_tiles(state, tiles);
    state.pointer.leave_gone(|key, node| {
        visible.iter().any(|v| &v.key == key)
            && tiles
                .iter()
                .find(|tile| &tile.key == key)
                .and_then(|tile| tile.frame.as_deref())
                .is_some_and(|frame| {
                    frame.nodes.iter().any(|n| {
                        n.id == node && matches!(n.kind, tomakast_core::NodeKind::Canvas(_))
                    })
                })
    });
}

/// Sends the canvases' pointer events that may go now; returns the requests
/// of those that have one, by provider. A tile that takes none (stale, or
/// its plugin reloaded) refuses them, hovering included: they are dropped.
fn send_pointer(state: &StripState, core: &mut Core) -> Vec<(String, RequestId)> {
    let mut requests = Vec::new();
    for (key, event) in state.pointer.take() {
        match core.send_widget_event(&key, WidgetEvent::Pointer(event)) {
            Ok(Some(request)) => requests.push((key.provider.clone(), request)),
            Ok(None) => {}
            Err(err) => tracing::debug!(widget = %key, %err, "pointer event not sent"),
        }
    }
    requests
}

/// Stops each tile whose canvases took longer than `budget.stall` in the
/// last paint, unless it was its first since it showed (see [`SLOW_PAINT`]), or longer than
/// `budget.slow` that many paints in a row; returns what the core says
/// about each one stopped. A tile not painted starts its run again.
fn judge_paints(
    state: &mut StripState,
    core: &mut Core,
    budget: &CanvasPaintBudget,
) -> Vec<CoreUpdate> {
    let mut stopped = Vec::new();
    let painted = state.paints.take();
    state.slow_paints.retain(|key, _| painted.contains_key(key));
    state.graced.retain(|key, _| painted.contains_key(key));
    for (key, took) in painted {
        let instance = core.module_instance(&key.provider);
        let first = state.graced.insert(key.clone(), instance) != Some(instance);
        let reason = if took > budget.stall && !first {
            Some(format!(
                "its canvas took longer than {} ms to paint",
                budget.stall.as_millis()
            ))
        } else if took > budget.slow {
            let slow = state.slow_paints.entry(key.clone()).or_default();
            *slow += 1;
            (*slow >= budget.slow_paints).then(|| {
                format!(
                    "its canvas took longer than {} ms to paint {} times in a row",
                    budget.slow.as_millis(),
                    budget.slow_paints
                )
            })
        } else {
            state.slow_paints.remove(&key);
            None
        };
        if let Some(reason) = reason {
            state.slow_paints.remove(&key);
            state.graced.remove(&key);
            let update = core.stop_widget(&key, &reason);
            if !matches!(update, CoreUpdate::None) {
                stopped.push(update);
            }
        }
    }
    stopped
}

/// Sends the core what the tiles look like now, and remembers its scale.
pub(crate) fn send_env(state: &mut StripState, core: &mut Core, window: &Window, cx: &App) {
    let env = widget_env(window, cx);
    state.env_scale = Some(env.scale);
    core.set_widget_env(env);
}

/// Sends the env again when the window's scale is not the last env's: a new
/// Wayland surface learns its output's scale only after it opens, and a
/// window may move to another output.
pub(crate) fn sync_scale(state: &mut StripState, core: &mut Core, window: &Window, cx: &App) {
    if state.env_scale != Some(window.scale_factor()) {
        send_env(state, core, window, cx);
    }
}

/// The root body: the strip, when it has tiles, above `list`. `focus` is
/// the strip's own focus handle, which a focused canvas has.
pub(crate) fn root_body(
    state: &StripState,
    tiles: &[StripTile],
    palette: &Palette,
    list: AnyElement,
    focus: &FocusHandle,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let strip = (!tiles.is_empty()).then(|| render(state, tiles, palette, focus, cx));
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .children(strip)
        .child(list)
        .into_any_element()
}

/// The strip of `tiles`: the scrolled row of tiles, with the kit's overlay
/// scrollbar at its bottom while it scrolls. It holds `focus`, and its keys
/// go to the focused canvas.
pub(crate) fn render(
    state: &StripState,
    tiles: &[StripTile],
    palette: &Palette,
    focus: &FocusHandle,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let sizes: Vec<WidgetSize> = tiles.iter().map(|tile| tile.size).collect();
    let slots = layout::slots(&sizes, VIEWPORT);
    let tile_theme = widget_theme(palette, cx);
    // Only the tiles on screen and the peeking ones build their frames; the
    // others keep their boxes, so the strip scrolls the same.
    let shown = on_screen(state, &slots);
    let input = FrameInput {
        pointer: state.pointer.clone(),
        view: cx.entity().downgrade(),
    };
    let (moved, left) = (state.pointer.clone(), state.pointer.clone());
    let row = div()
        .id("widget-strip")
        .test_support()
        .track_focus(focus)
        // A click in the strip leaves the keyboard in the field: only a
        // canvas reached by the keys takes this focus.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_key_down(cx.listener(LauncherView::canvas_key))
        .on_mouse_move(move |e, _, _| moved.set_over(Some(e.position)))
        .on_hover(move |&hovered, _, _| {
            if !hovered {
                left.set_over(None);
            }
        })
        .w_full()
        .flex()
        .gap(px(theme::STRIP_GAP))
        .px(px(theme::EDGE_PADDING))
        .py(px(theme::STRIP_PADDING_Y))
        .overflow_x_scroll()
        .track_scroll(&state.scroll)
        .children(
            tiles
                .iter()
                .zip(&slots)
                .enumerate()
                .map(|(ix, (tile, slot))| {
                    let frame = tile.frame.as_ref().filter(|_| shown.contains(&ix));
                    let key = tile.key.clone();
                    let click = cx.listener(move |this, _, _, cx| this.click_tile(&key, cx));
                    tile_element(tile, frame, slot.width, &tile_theme, palette, state, &input)
                        .on_click(click)
                        .test_support()
                        .into_any_element()
                }),
        );
    div()
        .relative()
        .flex_none()
        // The strip spans the panel under its border, so the viewport is
        // the panel's width (see `layout`).
        .mx(px(-theme::PANEL_BORDER))
        .child(row)
        // Over the whole strip, outside the scrolled content; it draws at the bottom.
        .child(Scrollbar::horizontal(&state.scroll).mode(ScrollbarMode::Scrolling))
        .into_any_element()
}

/// One tile `width` wide: `frame`, or an empty placeholder of its size
/// without one, under its border and focus ring; drawn with `state`'s
/// focus, canvas paints and images.
fn tile_element(
    tile: &StripTile,
    frame: Option<&Arc<Frame>>,
    width: f32,
    tile_theme: &WidgetTheme,
    palette: &Palette,
    state: &StripState,
    input: &FrameInput,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let focused = state.focus.tile() == Some(&tile.key);
    // Painted after the content and outside the layout, so the content box
    // is the whole tile, the size the module draws for. No listeners.
    let ring = div()
        .absolute()
        .inset_0()
        .rounded(px(theme::STRIP_TILE_RADIUS));
    let ring = if focused {
        ring.id(focus_element_id(&tile.key))
            .test_support()
            .border(px(theme::FOCUS_RING_WIDTH))
            .border_color(focus_color(palette))
            .into_any_element()
    } else {
        ring.border_1()
            .border_color(palette.border)
            .into_any_element()
    };
    div()
        .id(tile_element_id(&tile.key))
        .role(Role::Group)
        .aria_label(tile.title.clone())
        .aria_selected(focused)
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(theme::STRIP_TILE_HEIGHT))
        .rounded(px(theme::STRIP_TILE_RADIUS))
        .overflow_hidden()
        .bg(palette.chip)
        .children(frame.map(|frame| {
            components::frame_element(
                &tile.key,
                frame,
                tile_theme,
                state.focus.part().filter(|_| focused),
                palette,
                &state.paints,
                &state.images,
                input,
            )
        }))
        .child(ring)
}

/// The focus ring: the text color at half strength.
fn focus_color(palette: &Palette) -> Hsla {
    palette.text.alpha(0.5)
}

/// What the tiles look like now: their sizes in the panel, the window's
/// scale, and the launcher's colors, fonts and widget text sizes.
pub(crate) fn widget_env(window: &Window, cx: &App) -> WidgetEnv {
    let tile = |size| (layout::tile_width(size, VIEWPORT), theme::STRIP_TILE_HEIGHT);
    WidgetEnv {
        quarter: tile(WidgetSize::Quarter),
        single: tile(WidgetSize::Single),
        double: tile(WidgetSize::Double),
        scale: window.scale_factor(),
        theme: widget_theme(&theme::palette(cx), cx),
    }
}

/// The launcher's colors (`palette`), fonts and widget text sizes, as the
/// tiles see them.
fn widget_theme(palette: &Palette, cx: &App) -> WidgetTheme {
    let theme = cx.theme();
    let (title, value, body, caption) = theme::WIDGET_TEXT_SIZES;
    WidgetTheme {
        dark: theme.mode.is_dark(),
        colors: theme_colors(palette),
        ui_font: theme.font_family.to_string(),
        mono_font: theme.mono_font_family.to_string(),
        sizes: TextSizes {
            title,
            value,
            body,
            caption,
        },
    }
}

/// The colors of `palette` a widget may name.
fn theme_colors(palette: &Palette) -> ThemeColors {
    ThemeColors {
        text: pack(palette.text),
        muted: pack(palette.muted),
        panel: pack(palette.panel),
        border: pack(palette.border),
        selected: pack(palette.selected),
        error: pack(palette.error),
        success: pack(palette.success),
    }
}

/// `color` as `0xRRGGBBAA`.
fn pack(color: Hsla) -> u32 {
    let Rgba { r, g, b, a } = Rgba::from(color);
    let byte = |c: f32| (c.clamp(0., 1.) * 255.).round() as u32;
    byte(r) << 24 | byte(g) << 16 | byte(b) << 8 | byte(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;

    fn tile(widget: &str, size: WidgetSize) -> StripTile {
        StripTile {
            key: WidgetKey::new("fake", widget),
            title: widget.into(),
            size,
            frame: None,
            condition: None,
        }
    }

    #[test]
    fn a_strip_that_shrank_while_scrolled_keeps_its_tiles_visible() {
        use WidgetSize::{Double, Single};
        let state = StripState::default();
        // Scrolled to the end of single, single, double: 1476 - 750.
        state.scroll.set_offset(gpui_kit::point(px(-726.), px(0.)));
        let keys = |tiles: &[StripTile]| -> Vec<String> {
            visible_tiles(&state, tiles)
                .into_iter()
                .map(|tile| tile.key.widget)
                .collect()
        };

        // `b` peeks in at the left.
        let three = [tile("a", Single), tile("b", Single), tile("c", Double)];
        assert_eq!(keys(&three), ["b", "c"]);
        // One tile left: the offset GPUI clamps on the next layout is clamped now.
        assert_eq!(keys(&[tile("a", Single)]), ["a"]);
    }

    fn core() -> Core {
        Core::new(
            tomakast_core::store::Store::open_in_memory().unwrap(),
            tomakast_core::RankWeights::default(),
            tomakast_core::LocalNow::now(),
        )
        .unwrap()
    }

    /// Slow paints count in a row only while the tile paints: a frame it is
    /// not painted in (hidden, scrolled away) starts its run again.
    #[test]
    fn a_tile_not_painted_starts_its_slow_run_again() {
        let mut core = core();
        let mut state = StripState::default();
        let budget = CanvasPaintBudget {
            slow: Duration::ZERO,
            slow_paints: 5,
            stall: Duration::MAX,
        };
        let key = WidgetKey::new("fake", "a");
        for run in 1..=2 {
            state.paints.add(&key, Duration::from_millis(1));
            judge_paints(&mut state, &mut core, &budget);
            assert_eq!(state.slow_paints.get(&key), Some(&run));
        }

        judge_paints(&mut state, &mut core, &budget);

        assert_eq!(state.slow_paints.get(&key), None);
    }

    /// The budgets the launcher runs with: a debug build paints slower.
    #[test]
    fn debug_builds_get_four_times_the_budget() {
        let budget = CanvasPaintBudget::default();
        let factor = if cfg!(debug_assertions) { 4 } else { 1 };
        assert_eq!(budget.slow, SLOW_PAINT * factor);
        assert_eq!(budget.stall, STALLED_PAINT * factor);
        assert_eq!(budget.slow_paints, SLOW_PAINTS_TO_STOP);
    }

    #[test]
    fn pack_is_rgba() {
        assert_eq!(pack(hsla(0., 1., 0.5, 1.)), 0xff0000ff);
        assert_eq!(pack(hsla(0., 0., 1., 0.5)), 0xffffff80);
        assert_eq!(pack(hsla(0., 0., 0., 0.)), 0x00000000);
    }
}
