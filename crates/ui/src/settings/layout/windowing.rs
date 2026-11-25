//! Drawing only the lines in view of a box whose lines have a fixed height (the hotkey
//! tables, a hundred applications and more), with room of the exact height of the others, so
//! a frame of a long page costs what its lines in view do. A line in use (the row of a field
//! in use, a group row with the focus) and the lines next to it are drawn wherever they are:
//! a recording goes on out of view, and Tab reaches the next line.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, Pixels, ScrollHandle, Styled as _,
    Window, canvas, div, prelude::FluentBuilder as _, px,
};

use crate::settings::SettingsPage;
use crate::theme::{self, Palette};

/// Rows drawn beyond each edge of the page area, in pixels: what a frame that has not caught
/// up with a scroll yet still shows.
const OVERSCAN: f32 = 2. * theme::SETTINGS_ROW_HEIGHT;

/// How far the page on display is scrolled and where its boxes of fixed-height lines were
/// last drawn: what the page area needs to draw only their lines in view. The window keeps it
/// while it lives; a page shown again after another one starts at its top.
#[derive(Default)]
pub(crate) struct Viewports {
    scroll: ScrollHandle,
    /// By the index of the box's first group.
    placements: HashMap<usize, Rc<Cell<Option<Placement>>>>,
    /// Counts the changes of what the page area's layout follows from: a placement measured
    /// before the last one is not trusted.
    layout: u64,
    /// What the last page drawn was laid out from.
    drawn: Option<LayoutKey>,
}

/// What the layout of the page on display follows from, besides the pages themselves.
#[derive(PartialEq)]
struct LayoutKey {
    page: SettingsPage,
    query: String,
    open: u64,
    width: f32,
    height: f32,
}

impl Viewports {
    /// The pages were built again: what they hold may have moved.
    pub(crate) fn pages_changed(&mut self) {
        self.layout += 1;
    }

    /// The page area lays out `page` under `query`, with the groups open as `open` counts
    /// their changes, in `window` as it is now.
    pub(super) fn follow(&mut self, page: &SettingsPage, query: &str, open: u64, window: &Window) {
        let key = LayoutKey {
            page: page.clone(),
            query: query.to_string(),
            open,
            width: f32::from(window.viewport_size().width),
            height: f32::from(window.viewport_size().height),
        };
        if self.drawn.as_ref().map(|drawn| &drawn.page) != Some(page) {
            self.scroll = ScrollHandle::default();
            self.placements.clear();
        }
        if self.drawn.as_ref() != Some(&key) {
            self.layout += 1;
            self.drawn = Some(key);
        }
    }

    /// The scroll of the page on display.
    pub(super) fn scroll(&self) -> ScrollHandle {
        self.scroll.clone()
    }

    /// What the box of the page on display that starts with group `group` draws its lines
    /// in view from, with the page's `scroll`.
    pub(super) fn windowing<'a>(
        &mut self,
        group: usize,
        scroll: &'a ScrollHandle,
    ) -> Windowing<'a> {
        let placement = self.placements.entry(group).or_default().clone();
        Windowing {
            scroll,
            placement,
            layout: self.layout,
        }
    }
}

/// Where a group's box was last drawn: its top in the page's content (unscrolled), under the
/// layout [`Viewports::layout`] counted then.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Placement {
    top: f32,
    layout: u64,
}

impl Placement {
    /// Whether `other` is this, but for a rounding.
    fn same(self, other: Placement) -> bool {
        (self.top - other.top).abs() < 0.5 && self.layout == other.layout
    }
}

/// What the page area knows to draw a box's lines in view only.
pub(super) struct Windowing<'a> {
    scroll: &'a ScrollHandle,
    placement: Rc<Cell<Option<Placement>>>,
    /// [`Viewports::layout`] now.
    layout: u64,
}

/// Draws a line of a box, once.
pub(super) type DrawLine<'a> = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement + 'a>;

/// What draws a line of a box (a row, a group row), and what drawing it in view only needs.
pub(super) struct Line<'a> {
    /// The line is always this tall, the hairline over it aside; `None`: it varies.
    pub(super) height: Option<f32>,
    /// Drawn wherever it is (a group row: few, and a focus target).
    pub(super) kept: bool,
    /// In use (its field is in use, it has the focus): it and the lines next to it are drawn
    /// wherever they are, so Tab and Shift+Tab reach them.
    pub(super) in_use: bool,
    pub(super) draw: DrawLine<'a>,
}

/// The lines of a box: each with a hairline over it, but for the first when there are no
/// column titles (`columns`) over it.
///
/// While the lines have a fixed height, only those in view are drawn (and those
/// [`Line::kept`], those [`Line::in_use`] and the lines next to them), each run of the others
/// as room of their exact height; from the first line whose height varies on, every line is
/// drawn. A line that draws nothing measures where the box is, for the next frames: until it
/// was measured under the layout of now (the first time, after a group above opened or
/// closed, a search, a new size of the window), every line is drawn; when it moved all the
/// same, the next frame is asked for.
pub(super) fn box_lines(
    lines: Vec<Line>,
    columns: bool,
    windowing: Windowing,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let divided = |n: usize| n > 0 || columns;
    let hairline = |n: usize| {
        if divided(n) {
            theme::DIVIDER_THICKNESS
        } else {
            0.
        }
    };
    let line = |n: usize, draw: DrawLine<'_>, window: &mut Window, cx: &mut App| {
        div()
            .w_full()
            .when(divided(n), |line| {
                line.border_t(px(theme::DIVIDER_THICKNESS))
                    .border_color(palette.divider)
            })
            .child(draw(window, cx))
            .into_any_element()
    };
    if lines.first().is_none_or(|first| first.height.is_none()) {
        return lines
            .into_iter()
            .enumerate()
            .map(|(n, l)| line(n, l.draw, window, cx))
            .collect();
    }
    let layout = windowing.layout;
    let used = windowing
        .placement
        .get()
        .filter(|placement| placement.layout == layout);
    // The page area is never higher than the window.
    let view = f32::from(window.viewport_size().height);
    let scrolled = -f32::from(windowing.scroll.offset().y);
    let (from, to) = (scrolled - OVERSCAN, scrolled + view + OVERSCAN);
    let columns_height = if columns {
        theme::SETTINGS_COLUMNS_HEIGHT
    } else {
        0.
    };
    let in_use: Vec<bool> = lines.iter().map(|l| l.in_use).collect();
    let pinned = |n: usize| (n.saturating_sub(1)..=n + 1).any(|m| in_use.get(m) == Some(&true));
    let measure = {
        let scroll = windowing.scroll.clone();
        let placement = windowing.placement.clone();
        canvas(
            move |bounds: gpui_kit::Bounds<Pixels>, window, _| {
                let area = scroll.bounds();
                let now = Placement {
                    top: f32::from(bounds.top() - area.top() - scroll.offset().y),
                    layout,
                };
                match used {
                    Some(used) if used.same(now) => {}
                    // Drawn where it was not: the next frame draws what is in view.
                    Some(_) => {
                        placement.set(Some(now));
                        window.request_animation_frame();
                    }
                    // Every line drawn.
                    None => placement.set(Some(now)),
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
        .into_any_element()
    };
    let mut drawn = vec![measure];
    let Some(Placement { top, .. }) = used else {
        drawn.extend(
            lines
                .into_iter()
                .enumerate()
                .map(|(n, l)| line(n, l.draw, window, cx)),
        );
        return drawn;
    };
    let room = |height: f32| div().w_full().flex_none().h(px(height)).into_any_element();
    // Where the next line starts, while every line so far has a fixed height: the box's top
    // border, then the column titles.
    let mut y = Some(top + theme::DIVIDER_THICKNESS + columns_height);
    let mut skipped = 0.;
    for (n, l) in lines.into_iter().enumerate() {
        let height = l.height.map(|height| height + hairline(n));
        let in_view = match (y, height) {
            (Some(y), Some(height)) => y + height > from && y < to,
            _ => true,
        };
        y = y.zip(height).map(|(y, height)| y + height);
        if in_view || l.kept || pinned(n) {
            if skipped > 0. {
                drawn.push(room(skipped));
                skipped = 0.;
            }
            drawn.push(line(n, l.draw, window, cx));
        } else {
            // Only a line of a fixed height is ever left out.
            skipped += height.unwrap_or_default();
        }
    }
    if skipped > 0. {
        drawn.push(room(skipped));
    }
    drawn
}
