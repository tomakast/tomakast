//! The grid view: tiles in rows of `columns`, moved over in two dimensions.

use gpui_kit::{
    AnyElement, Context, FocusHandle, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::{GridTile, GridView, Icon};

use crate::icons::IconSpec;
use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};
use crate::views;

pub(crate) struct GridState {
    focus: FocusHandle,
    /// Index of the selected tile.
    selected: usize,
    /// Scrolls the rows, which are its children.
    scroll: ScrollHandle,
}

/// Number of tiles per row; a view that says 0 has one column.
fn columns(view: &GridView) -> usize {
    usize::from(view.columns).max(1)
}

impl GridState {
    /// The first tile is selected; the grid takes the focus.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<LauncherView>) -> Self {
        let state = GridState {
            focus: cx.focus_handle(),
            selected: 0,
            scroll: ScrollHandle::new(),
        };
        state.focus(window, cx);
        state
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        self.focus.focus(window, cx);
    }

    pub(crate) fn selected_tile<'v>(&self, view: &'v GridView) -> Option<&'v GridTile> {
        view.tiles
            .get(self.selected.min(view.tiles.len().saturating_sub(1)))
    }

    /// Keeps tile `keep` selected after the view's tiles changed, if it is
    /// still there; otherwise keeps the position, clamped to the new tiles.
    pub(crate) fn reselect(&mut self, view: &GridView, keep: Option<&str>) {
        if let Some(ix) = keep.and_then(|id| view.tiles.iter().position(|t| t.id == id)) {
            self.selected = ix;
        }
        self.select(view, self.selected);
    }

    pub(crate) fn left(&mut self, view: &GridView) {
        if !self.selected.is_multiple_of(columns(view)) {
            self.select(view, self.selected - 1);
        }
    }

    pub(crate) fn right(&mut self, view: &GridView) {
        if self.selected % columns(view) < columns(view) - 1 {
            self.select(view, self.selected + 1);
        }
    }

    pub(crate) fn up(&mut self, view: &GridView) {
        let columns = columns(view);
        if self.selected >= columns {
            self.select(view, self.selected - columns);
        }
    }

    /// Down a row; into a short last row it lands on that row's last tile.
    pub(crate) fn down(&mut self, view: &GridView) {
        let columns = columns(view);
        let last = view.tiles.len().saturating_sub(1);
        if self.selected / columns < last / columns {
            self.select(view, (self.selected + columns).min(last));
        }
    }

    fn select(&mut self, view: &GridView, ix: usize) {
        self.selected = ix.min(view.tiles.len().saturating_sub(1));
        self.scroll.scroll_to_item(self.selected / columns(view));
    }
}

/// Tile `ix`: `image` large, over `caption`. Also the tiles of a list drawn
/// as a grid.
pub(crate) fn tile(
    ix: usize,
    image: Option<&Icon>,
    caption: &str,
    selected: bool,
    palette: &Palette,
) -> AnyElement {
    let spec = IconSpec {
        icon: image,
        monogram: caption,
    };
    let image = views::icon_element(spec, theme::IMAGE_SIZE, palette);
    div()
        .id(("tile", ix))
        .role(Role::ListItem)
        .aria_label(caption.to_string())
        .test_support()
        .flex_1()
        .min_w_0()
        .h(px(theme::TILE_HEIGHT))
        .p_2()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .rounded(px(theme::TILE_RADIUS))
        .when(selected, |tile| tile.bg(palette.selected))
        .child(image)
        .child(
            div()
                .max_w_full()
                .truncate()
                .text_size(theme::text(theme::TILE_TITLE_SIZE))
                .child(caption.to_string()),
        )
        .into_any_element()
}

/// The empty place of a short last row: sized as a [`tile`] is (its padding too), so the
/// row's tiles keep the width and columns of the full rows.
pub(crate) fn filler() -> AnyElement {
    div().flex_1().min_w_0().p_2().into_any_element()
}

pub(crate) fn render(
    view: &GridView,
    state: &GridState,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let columns = columns(view);
    let selected = state.selected.min(view.tiles.len().saturating_sub(1));
    let rows = view.tiles.chunks(columns).enumerate().map(|(row, tiles)| {
        div()
            .flex()
            .gap_2()
            .children(tiles.iter().enumerate().map(|(col, t)| {
                let ix = row * columns + col;
                tile(ix, t.image.as_ref(), &t.caption, ix == selected, &palette)
            }))
            // The last row is padded, so its tiles are as wide as the others.
            .children((tiles.len()..columns).map(|_| filler()))
    });
    div()
        .id("grid-view")
        .track_focus(&state.focus)
        .track_scroll(&state.scroll)
        .flex()
        .flex_col()
        .gap_2()
        .flex_1()
        .min_h_0()
        .p(px(theme::VIEW_PADDING))
        .text_size(theme::text(theme::BODY_TEXT_SIZE))
        .overflow_y_scroll()
        .children(rows)
        .into_any_element()
}
