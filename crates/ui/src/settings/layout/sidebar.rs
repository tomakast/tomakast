//! The sidebar: its part of the title area (with the search field), then the pages search
//! keeps, each a launcher-style row with its icon. Sections have no titles: a gap separates
//! them. A section none of whose pages is kept is left out, and so is its gap.

use std::rc::Rc;

use super::{Page, page_icon};
use crate::settings::SettingsPage;
use crate::theme::{self, Palette};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
    prelude::FluentBuilder as _, px,
};

/// What a click on a sidebar entry does.
pub(crate) type OnSelect = Rc<dyn Fn(&SettingsPage, &mut Window, &mut App)>;

/// The sidebar over `pages`, whose kept groups are `matches`; `title` (its part of the
/// title area) on top, `shown` selected.
pub(crate) fn sidebar(
    title: AnyElement,
    pages: &[Page],
    matches: &[Option<Vec<usize>>],
    shown: Option<&SettingsPage>,
    on_select: OnSelect,
    palette: &Palette,
) -> AnyElement {
    let mut list = Vec::new();
    let mut section = None;
    let kept = pages
        .iter()
        .enumerate()
        .filter(|(ix, _)| matches.get(*ix).is_some_and(Option::is_some));
    for (ix, page) in kept {
        if section != Some(page.section) {
            if section.is_some() {
                list.push(
                    div()
                        .id(("sidebar-gap", page.section as usize))
                        .test_support()
                        .flex_none()
                        .h(px(theme::SETTINGS_GROUP_GAP))
                        .into_any_element(),
                );
            }
            section = Some(page.section);
        }
        let selected = shown == Some(&page.key);
        list.push(entry(ix, page, selected, on_select.clone(), palette));
    }
    div()
        .id("settings-sidebar")
        .test_support()
        .flex_none()
        .w(px(theme::settings_sidebar_width()))
        .h_full()
        .flex()
        .flex_col()
        .child(title)
        .child(
            div()
                .id("settings-sidebar-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(theme::SETTINGS_EDGE))
                .children(list),
        )
        .into_any_element()
}

/// The entry of `page`, the `ix`th page: its icon and title.
fn entry(
    ix: usize,
    page: &Page,
    selected: bool,
    on_select: OnSelect,
    palette: &Palette,
) -> AnyElement {
    let key = page.key.clone();
    div()
        .id(("sidebar-entry", ix))
        .role(Role::ListItem)
        .aria_label(page.title.clone())
        .aria_selected(selected)
        .test_support()
        .w_full()
        .h(px(theme::SETTINGS_ENTRY_HEIGHT))
        .px(px(theme::SETTINGS_EDGE))
        .cursor_pointer()
        .on_click(move |_, window, cx| on_select(&key, window, cx))
        .child(
            // An id of its own: GPUI redraws a hovered element only when it has one.
            div()
                .id("sidebar-entry-fill")
                .test_support()
                .size_full()
                .px(px(theme::ROW_PADDING_X))
                .flex()
                .items_center()
                .gap(px(theme::SETTINGS_ENTRY_ICON_GAP))
                .rounded(px(theme::ACTION_RADIUS))
                .when(selected, |row| row.bg(palette.sidebar_selected))
                .when(!selected, |row| row.hover(|row| row.bg(palette.hover)))
                .child(
                    div()
                        .id("sidebar-icon")
                        .test_support()
                        .flex_none()
                        .child(page_icon(
                            &page.icon,
                            theme::SETTINGS_SIDEBAR_ICON_SIZE,
                            palette,
                        )),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_size(theme::text(theme::ROW_TEXT_SIZE))
                        .text_color(palette.text)
                        .child(page.title.clone()),
                ),
        )
        .into_any_element()
}
