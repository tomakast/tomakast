//! Renderers of module-owned views, one per view kind, and their UI state.

pub(crate) mod action_panel;
pub(crate) mod detail;
pub(crate) mod form;
pub(crate) mod grid;
pub(crate) mod list;
pub(crate) mod scratchpad;

use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, div, img, prelude::FluentBuilder as _, px,
};
use tomakast_core::{Accessory, Icon, View};

use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};

/// UI state of a pushed view: what the view model does not hold.
pub(crate) enum ViewState {
    List(list::ListState),
    Grid(grid::GridState),
    Detail(detail::DetailState),
    Form(form::FormState),
    Scratchpad(scratchpad::ScratchpadState),
}

impl ViewState {
    /// State of a newly pushed view; it takes the focus.
    pub(crate) fn new(
        instance: u64,
        view: &View,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        match view {
            View::List(list) => ViewState::List(list::ListState::new(instance, list, window, cx)),
            View::Grid(_) => ViewState::Grid(grid::GridState::new(window, cx)),
            View::Detail(_) => ViewState::Detail(detail::DetailState::new(window, cx)),
            View::Form(form) => ViewState::Form(form::FormState::new(instance, form, window, cx)),
            View::Scratchpad(pad) => {
                ViewState::Scratchpad(scratchpad::ScratchpadState::new(instance, pad, window, cx))
            }
        }
    }

    /// Gives the focus back to the view, when it is on top again.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        match self {
            ViewState::List(state) => state.focus(window, cx),
            ViewState::Grid(state) => state.focus(window, cx),
            ViewState::Detail(state) => state.focus(window, cx),
            ViewState::Form(state) => state.focus(window, cx),
            ViewState::Scratchpad(state) => state.focus(window, cx),
        }
    }

    /// Id of the selected row or tile, to keep it selected across an update
    /// of `view`.
    pub(crate) fn selected_id(&self, view: &View) -> Option<String> {
        match (view, self) {
            (View::List(view), ViewState::List(state)) => {
                state.selected_row(view).map(|row| row.id.clone())
            }
            (View::Grid(view), ViewState::Grid(state)) => {
                state.selected_tile(view).map(|tile| tile.id.clone())
            }
            _ => None,
        }
    }
}

/// One list row, on one line: icon, title, the subtitle in the muted color
/// (it truncates before the title does), accessories at the right edge. The
/// row spans the list's width; its highlight is inset by the row gutter.
pub(crate) fn row(
    id: ElementId,
    title: &str,
    subtitle: Option<&str>,
    icon: Option<&Icon>,
    accessories: &[Accessory],
    selected: bool,
    palette: &Palette,
) -> AnyElement {
    let icon = match icon {
        Some(Icon::File(path)) => img(path.clone())
            .flex_none()
            .size(px(theme::ICON_SIZE))
            .into_any_element(),
        // Icon names are resolved to files by the modules; anything else is blank space.
        _ => div()
            .flex_none()
            .size(px(theme::ICON_SIZE))
            .into_any_element(),
    };
    let text = div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(theme::ROW_SUBTITLE_GAP))
        .text_size(px(theme::ROW_TEXT_SIZE))
        .child(
            div()
                .min_w_0()
                .flex_shrink(theme::TITLE_SHRINK)
                .truncate()
                .child(title.to_string()),
        )
        .children(subtitle.map(|subtitle| {
            div()
                .min_w_0()
                .flex_shrink(theme::SUBTITLE_SHRINK)
                .truncate()
                .text_color(palette.muted)
                .child(subtitle.to_string())
        }));
    let accessories = (!accessories.is_empty()).then(|| {
        div()
            .ml_auto()
            .flex_none()
            .pl(px(theme::ROW_ACCESSORY_GAP))
            .flex()
            .items_center()
            .gap(px(theme::ROW_ACCESSORY_GAP))
            .text_size(px(theme::ACCESSORY_TEXT_SIZE))
            .text_color(palette.muted)
            .children(accessories.iter().map(|accessory| {
                match accessory {
                    Accessory::Text(text) => div().child(text.clone()),
                    Accessory::Tag(tag) => div()
                        .px(px(theme::TAG_PADDING_X))
                        .rounded_full()
                        .bg(palette.chip)
                        .text_size(px(theme::SMALL_TEXT_SIZE))
                        .child(tag.clone()),
                }
            }))
    });
    div()
        .id(id)
        .role(Role::ListItem)
        .aria_label(title.to_string())
        .test_support()
        .w_full()
        .h(px(theme::ROW_HEIGHT))
        .px(px(theme::ROW_GUTTER))
        .child(
            div()
                .size_full()
                .px(px(theme::ROW_PADDING_X))
                .flex()
                .items_center()
                .gap(px(theme::ROW_ICON_GAP))
                .rounded(px(theme::ROW_RADIUS))
                .when(selected, |row| row.bg(palette.selected))
                .child(icon)
                .child(text)
                .children(accessories),
        )
        .into_any_element()
}

/// A section label of a list: muted, semibold, lined up with the rows'
/// icons; never selectable. Lists are uniform (every line as tall as a row),
/// so the label sits at the bottom of a row-tall line.
pub(crate) fn section_label(id: ElementId, text: &str, palette: &Palette) -> AnyElement {
    div()
        .id(id)
        .role(Role::Heading)
        .aria_label(text.to_string())
        .test_support()
        .w_full()
        .h(px(theme::ROW_HEIGHT))
        .flex()
        .items_end()
        .child(
            div()
                .h(px(theme::SECTION_HEIGHT))
                .min_w_0()
                .px(px(theme::EDGE_PADDING))
                .flex()
                .items_center()
                .truncate()
                .text_size(px(theme::SMALL_TEXT_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.muted)
                .child(text.to_string()),
        )
        .into_any_element()
}
