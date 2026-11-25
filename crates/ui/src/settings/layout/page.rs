//! The page area: the page's groups (headings inside the page, rows with their title on the
//! left and their control on the right, or a table under the titles of its columns) and the
//! state of a search nothing matches. Groups that open and close, one after the other, are
//! one table: the titles of its columns once, then for each group a row that opens and closes
//! it, and its rows under it, indented. Also the launcher form's look for the kit's buttons in
//! rows. The band with the page's title is the frame's (`settings::frame`). Of a box whose
//! lines have a fixed height only those in view are drawn ([`super::windowing`]).

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::{Icon, ItemId};

use super::windowing::{Line, Viewports, Windowing, box_lines};
use super::{Group, GroupKey, OpenGroups, Page, PageIcon, Row, RowOptions};
use crate::icons::IconSpec;
use crate::theme::{self, Palette};
use crate::views::icon_element;

/// What the page area says when search keeps nothing.
const NO_MATCH: &str = "No settings match";

/// What a click on the group row of a collapsible group does: `revealed` when a search opened
/// it.
pub(crate) type OnToggle = Rc<dyn Fn(&GroupKey, bool, &mut Window, &mut App)>;

/// How the page area draws a group under the query.
struct Shown {
    /// A search opened it.
    revealed: bool,
    open: bool,
    /// The rows drawn, by index.
    rows: Vec<usize>,
}

/// How `group` is drawn: as `open` has it, or as the query opens it on `revealed`, its rows
/// the query matches.
fn shown(group: &Group, revealed: Option<&[usize]>, open: &OpenGroups) -> Shown {
    let all = || (0..group.rows.len()).collect();
    let Some(key) = &group.collapsible else {
        return Shown {
            revealed: false,
            open: true,
            rows: all(),
        };
    };
    let revealed = revealed.map(<[usize]>::to_vec);
    let is_open = open.is_open(key, revealed.is_some());
    Shown {
        revealed: revealed.is_some(),
        open: is_open,
        rows: match (is_open, revealed) {
            (false, _) => Vec::new(),
            (true, Some(rows)) => rows,
            (true, None) => all(),
        },
    }
}

/// The rows the query opens group `ix` on, of the page's `revealed`.
fn revealed_of(revealed: &[Option<Vec<usize>>], ix: usize) -> Option<&[usize]> {
    revealed.get(ix).and_then(Option::as_deref)
}

/// The rows the page area draws of the groups `groups` of `page`, with the rows the query
/// opens each on, `revealed`.
pub(crate) fn drawn_rows<'a>(
    page: &'a Page,
    groups: &[usize],
    revealed: &[Option<Vec<usize>>],
    open: &OpenGroups,
) -> Vec<&'a Row> {
    groups
        .iter()
        .filter_map(|&ix| Some((ix, page.groups.get(ix)?)))
        .flat_map(|(ix, group)| {
            shown(group, revealed_of(revealed, ix), open)
                .rows
                .into_iter()
                .filter_map(|ix| group.rows.get(ix))
        })
        .collect()
}

/// The groups `groups` (indices into its groups) of `page`, scrolling, in a column no wider
/// than [`theme::SETTINGS_PAGE_MAX_WIDTH`] in the middle of the page area; a collapsible
/// group as `open` has it, or as `query` opens it on its rows in `revealed`. Collapsible groups
/// one after the other are one table.
///
/// Of a box whose lines have a fixed height, only those in view are drawn, with the rows of
/// the fields `in_use`, the group rows, and the lines next to them and to a group row with the
/// focus.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_body(
    page: &Page,
    groups: &[usize],
    query: &str,
    revealed: &[Option<Vec<usize>>],
    open: &mut OpenGroups,
    views: &mut Viewports,
    in_use: &HashSet<ItemId>,
    on_toggle: OnToggle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let palette = theme::palette(cx);
    views.follow(&page.key, query, open.changes(), window);
    let scroll = views.scroll();
    // Each a group, or the groups of one table.
    let mut blocks: Vec<Vec<(usize, &Group)>> = Vec::new();
    for (ix, group) in groups
        .iter()
        .filter_map(|&ix| Some((ix, page.groups.get(ix)?)))
    {
        match blocks.last_mut() {
            Some(block) if group.collapsible.is_some() && block[0].1.collapsible.is_some() => {
                block.push((ix, group))
            }
            _ => blocks.push(vec![(ix, group)]),
        }
    }
    let groups: Vec<AnyElement> = blocks
        .into_iter()
        .enumerate()
        .map(|(n, block)| {
            let windowing = views.windowing(block[0].0, &scroll);
            let element = if block[0].1.collapsible.is_some() {
                let groups = block
                    .into_iter()
                    .filter_map(|(ix, group)| {
                        let key = group.collapsible.clone()?;
                        let focus = open.focus(&key, cx);
                        let shown = shown(group, revealed_of(revealed, ix), open);
                        Some(TableGroup {
                            ix,
                            group,
                            shown,
                            key,
                            focus,
                        })
                    })
                    .collect();
                render_table(groups, in_use, &on_toggle, windowing, &palette, window, cx)
            } else {
                let (ix, group) = block[0];
                let shown = shown(group, revealed_of(revealed, ix), open);
                render_group(ix, group, shown, in_use, windowing, &palette, window, cx)
            };
            let gap = if n == 0 {
                0.
            } else {
                theme::SETTINGS_GROUP_GAP
            };
            div().pt(px(gap)).child(element).into_any_element()
        })
        .collect();
    let top = theme::SETTINGS_EDGE;
    div()
        .id("settings-page")
        .test_support()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            // Its own id keeps the ids of its rows, and its scroll position, apart from
            // another page's.
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "{:?}",
                    page.key
                ))))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .child(
                    page_column(div().flex().flex_col().children(groups))
                        .pt(px(top))
                        .pb(px(top + theme::SETTINGS_GROUP_GAP)),
                ),
        )
        .into_any_element()
}

/// `content` in the page's column: the page's padding at its sides, and no wider than a page
/// reads well, centred. The page's boxes, its title and the file's banner all start at its
/// left edge.
pub(crate) fn page_column(content: Div) -> Div {
    div()
        .w_full()
        .px(px(theme::SETTINGS_EDGE))
        .flex()
        .justify_center()
        .child(content.w_full().max_w(px(theme::SETTINGS_PAGE_MAX_WIDTH)))
}

/// What the page area shows when search keeps nothing.
pub(crate) fn empty_state(palette: &Palette) -> AnyElement {
    div()
        .id("settings-no-match")
        .aria_label(NO_MATCH)
        .test_support()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(theme::text(theme::ROW_TEXT_SIZE))
        .text_color(palette.muted)
        .child(NO_MATCH)
        .into_any_element()
}

/// A text button of a row, with the launcher form's look.
///
/// Its fills are the kit's custom variant with the palette's own: the kit's outline hover
/// replaces the fill with a shade that reads darker on the dark theme.
pub(crate) fn control_button(id: impl Into<ElementId>, palette: &Palette, cx: &App) -> Button {
    let fills = palette.control_fills();
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(fills.border)
                .foreground(palette.text)
                .hover(fills.hover)
                .active(fills.active),
        )
        .border_1()
        .rounded(px(theme::CONTROL_RADIUS))
        .bg(fills.rest)
        .border_color(fills.border)
}

/// A group: its heading and description over a box of its rows, one hairline between two
/// rows, the titles of its columns over them when it is a table.
#[allow(clippy::too_many_arguments)]
fn render_group(
    ix: usize,
    group: &Group,
    shown: Shown,
    in_use: &HashSet<ItemId>,
    windowing: Windowing,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let rows: Vec<(usize, &Row)> = shown
        .rows
        .iter()
        .filter_map(|&ix| Some((ix, group.rows.get(ix)?)))
        .collect();
    let header = (group.title.is_some() || group.description.is_some()).then(|| {
        div()
            .px(px(theme::SETTINGS_TEXT_INSET))
            .pb(px(theme::SETTINGS_HEADING_GAP))
            .flex()
            .flex_col()
            .gap(px(theme::SETTINGS_DESCRIPTION_GAP))
            .children(group.title.as_ref().map(|title| heading(title, palette)))
            .children(group.description.as_ref().map(|description| {
                Label::new(description.clone())
                    .text_size(theme::text(theme::MUTED_TEXT_SIZE))
                    .text_color(palette.muted)
            }))
            .into_any_element()
    });
    if group.plain {
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .map(|(ix, row)| {
                render_row(
                    ("settings-row", ix).into(),
                    row,
                    true,
                    0.,
                    palette,
                    window,
                    cx,
                )
            })
            .collect();
        return div()
            .id(("settings-group", ix))
            .flex()
            .flex_col()
            .gap(px(theme::SETTINGS_HEADING_GAP))
            .children(header)
            .children(rows)
            .into_any_element();
    }
    let body = (!rows.is_empty()).then(|| {
        let columns = group.columns.as_ref();
        let lines = rows
            .into_iter()
            .map(|(ix, row)| row_line(("settings-row", ix).into(), row, 0., in_use, palette))
            .collect();
        table_box(columns, lines, windowing, palette, window, cx)
    });
    div()
        .id(("settings-group", ix))
        .flex()
        .flex_col()
        .children(header)
        .children(body)
        .into_any_element()
}

/// A box of lines, under the titles of its columns when it has them.
fn table_box(
    columns: Option<&(String, String)>,
    lines: Vec<Line>,
    windowing: Windowing,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let lines = box_lines(lines, columns.is_some(), windowing, palette, window, cx);
    div()
        .id("settings-group-box")
        .test_support()
        .relative()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(theme::SETTINGS_GROUP_RADIUS))
        .border_1()
        .border_color(palette.group_border)
        .bg(palette.group)
        .children(columns.map(|(left, right)| column_titles(left, right, palette)))
        .children(lines)
        .into_any_element()
}

/// The line of a row of a box, `indent` in from the box's padding.
fn row_line<'a>(
    id: ElementId,
    row: &'a Row,
    indent: f32,
    in_use: &HashSet<ItemId>,
    palette: &'a Palette,
) -> Line<'a> {
    Line {
        height: row.height,
        kept: false,
        in_use: row
            .hotkey_of
            .as_ref()
            .is_some_and(|item| in_use.contains(item)),
        draw: Box::new(move |window, cx| render_row(id, row, false, indent, palette, window, cx)),
    }
}

/// A collapsible group of a table, as the page area draws it now.
struct TableGroup<'a> {
    ix: usize,
    group: &'a Group,
    shown: Shown,
    key: GroupKey,
    focus: gpui_kit::FocusHandle,
}

/// Collapsible groups as one table: the titles of the columns (of the first group that has
/// them) once, then for each group its group row and, when it is open, its rows under it,
/// indented so a row's icon starts where the group's title does.
fn render_table(
    groups: Vec<TableGroup>,
    in_use: &HashSet<ItemId>,
    on_toggle: &OnToggle,
    windowing: Windowing,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let first = groups.first().map_or(0, |group| group.ix);
    let columns = groups.iter().find_map(|group| group.group.columns.as_ref());
    let last_group = groups.len().saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();
    for (
        n,
        TableGroup {
            ix,
            group,
            shown,
            key,
            focus,
        },
    ) in groups.into_iter().enumerate()
    {
        let rows: Vec<(usize, &Row)> = shown
            .rows
            .iter()
            .filter_map(|&row_ix| Some((row_ix, group.rows.get(row_ix)?)))
            .collect();
        // The box's last line: rounded at the bottom as the box is.
        let last = n == last_group && rows.is_empty();
        let in_use_now = focus.is_focused(window);
        // Reached with the keyboard: a click on the row opens or closes it, and a ring after
        // it would only be noise.
        let ring = in_use_now && window.last_input_was_keyboard();
        let on_toggle = on_toggle.clone();
        lines.push(Line {
            height: Some(theme::SETTINGS_TABLE_GROUP_HEIGHT),
            kept: true,
            in_use: in_use_now,
            draw: Box::new(move |_, cx| {
                let row = GroupRow {
                    ix,
                    key,
                    focus,
                    ring,
                    last,
                };
                group_row(row, group, &shown, on_toggle, palette, cx)
            }),
        });
        for (row_ix, row) in rows {
            // The rows of all groups are siblings in the box: the group's index and the row's
            // in one number keep their ids apart.
            let id = ElementId::NamedInteger("settings-row".into(), table_row_id(ix, row_ix));
            lines.push(row_line(
                id,
                row,
                theme::SETTINGS_TABLE_INDENT,
                in_use,
                palette,
            ));
        }
    }
    let table = table_box(columns, lines, windowing, palette, window, cx);
    div()
        .id(("settings-group", first))
        .flex()
        .flex_col()
        .child(table)
        .into_any_element()
}

/// The id of row `row` of group `group` of a table.
fn table_row_id(group: usize, row: usize) -> u64 {
    ((group as u64) << 32) | row as u64
}

/// What a group row is drawn from, besides its group.
struct GroupRow {
    ix: usize,
    key: GroupKey,
    focus: gpui_kit::FocusHandle,
    /// Focused from the keyboard: the focus ring shows.
    ring: bool,
    /// The box's last line.
    last: bool,
}

/// The row of a collapsible group in its table: a chevron, the icon of where its rows come
/// from, its title and how many rows it has; no fill of its own, tinted under the pointer;
/// rounded at the bottom as the box is when it is the `last` line. A click (or Enter or Space)
/// opens or closes it. Focused from the keyboard, the focus ring runs inside its edges, where
/// the box does not cut it.
fn group_row(
    row: GroupRow,
    group: &Group,
    shown: &Shown,
    on_toggle: OnToggle,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let GroupRow {
        ix,
        key,
        focus,
        ring,
        last,
    } = row;
    let title = group.title.clone().unwrap_or_default();
    let count = group.rows.len();
    let items = if count == 1 { "item" } else { "items" };
    let chevron = if shown.open {
        "chevron-down"
    } else {
        "chevron-right"
    };
    let revealed = shown.revealed;
    div()
        .id(("group-header", ix))
        .role(Role::Button)
        .aria_label(format!("{title}, {count} {items}"))
        .aria_expanded(shown.open)
        .test_support()
        .track_focus(&focus)
        .relative()
        .w_full()
        .flex_none()
        .h(px(theme::SETTINGS_TABLE_GROUP_HEIGHT))
        .px(px(theme::SETTINGS_ROW_PADDING_X))
        .flex()
        .items_center()
        .gap(px(theme::SETTINGS_TABLE_GROUP_GAP))
        .when(last, |row| {
            row.rounded_b(px(theme::SETTINGS_GROUP_RADIUS - theme::DIVIDER_THICKNESS))
        })
        .cursor_pointer()
        .hover(|row| row.bg(palette.hover))
        .on_click(move |_, window, cx| on_toggle(&key, revealed, window, cx))
        .child(
            div()
                .id("group-chevron")
                .test_support()
                .flex_none()
                .size(px(theme::SETTINGS_TABLE_CHEVRON_SIZE))
                .child(
                    gpui_kit::svg()
                        .path(crate::icons::bundled_path(chevron).unwrap_or_default())
                        .size(px(theme::SETTINGS_TABLE_CHEVRON_SIZE))
                        .text_color(palette.muted),
                ),
        )
        .child(
            // Its room also when the group has no icon: the titles start at one place.
            div()
                .flex_none()
                .size(px(theme::SETTINGS_TABLE_SOURCE_ICON_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .children(
                    group.icon.as_ref().map(|icon| {
                        page_icon(icon, theme::SETTINGS_TABLE_SOURCE_ICON_SIZE, palette)
                    }),
                ),
        )
        .child(
            div()
                .id("group-title")
                .test_support()
                .min_w_0()
                .truncate()
                .text_size(theme::text(theme::ROW_TEXT_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.text)
                .child(title),
        )
        .child(
            div()
                .flex_none()
                .text_size(theme::text(theme::MUTED_TEXT_SIZE))
                .text_color(palette.muted)
                .child(count.to_string()),
        )
        .when(ring, |row| {
            // The kit's ring turned inward: its translucent band toward the row's edge, its
            // line next to what the row holds.
            let ring = cx.theme().ring;
            let band = theme::FOCUS_BAND_WIDTH;
            row.child(
                div()
                    .id("group-focus-ring")
                    .test_support()
                    .absolute()
                    .inset(px(theme::FOCUS_RING_INSET))
                    .rounded(px(theme::ROW_RADIUS - theme::FOCUS_RING_INSET))
                    .border(px(band))
                    .border_color(ring.opacity(theme::FOCUS_BAND_OPACITY))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(theme::ROW_RADIUS - theme::FOCUS_RING_INSET - band))
                            .border_1()
                            .border_color(ring),
                    ),
            )
        })
        .into_any_element()
}

/// The titles of a table's columns: the left over the rows' titles, starting where they (or
/// their icons) start; the right over their controls, starting where the text inside a field
/// does.
fn column_titles(left: &str, right: &str, palette: &Palette) -> AnyElement {
    let cell = |id: &'static str, text: &str| {
        div()
            .id(id)
            .aria_label(text.to_string())
            .test_support()
            .child(text.to_string())
    };
    div()
        .w_full()
        .h(px(theme::SETTINGS_COLUMNS_HEIGHT))
        .px(px(theme::SETTINGS_ROW_PADDING_X))
        .flex()
        .items_center()
        .gap(px(theme::LABEL_GAP))
        .text_size(theme::text(theme::SMALL_TEXT_SIZE))
        .text_color(palette.muted)
        .child(div().flex_1().min_w_0().child(cell("column-left", left)))
        .child(
            div()
                .flex_none()
                .w(px(theme::settings_control_width()))
                .pl(px(theme::CONTROL_TEXT_INSET))
                .child(cell("column-right", right)),
        )
        .into_any_element()
}

/// A group's title: small and semibold, in the text colour, over the box.
fn heading(title: &str, palette: &Palette) -> AnyElement {
    div()
        .id("heading")
        .role(Role::Heading)
        .aria_label(title.to_string())
        .test_support()
        .text_size(theme::text(theme::ROW_TEXT_SIZE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette.text)
        .child(title.to_string())
        .into_any_element()
}

/// The icon of a sidebar entry, or of where a group's rows come from, `size` across.
pub(crate) fn page_icon(icon: &PageIcon, size: f32, palette: &Palette) -> AnyElement {
    match icon {
        PageIcon::Named(name) => {
            let icon = Icon::Named(name.to_string());
            let spec = IconSpec {
                icon: Some(&icon),
                monogram: name,
            };
            icon_element(spec, size, palette)
        }
        PageIcon::Item { icon, monogram } => {
            let spec = IconSpec {
                icon: icon.as_ref(),
                monogram,
            };
            icon_element(spec, size, palette)
        }
        PageIcon::Greyed(text) => {
            let spec = IconSpec {
                icon: None,
                monogram: text,
            };
            div()
                .opacity(theme::DISABLED_OPACITY)
                .child(icon_element(spec, size, palette))
                .into_any_element()
        }
    }
}

/// A row, `indent` in from the padding of its box: its title and control, or what it draws
/// whole.
fn render_row(
    id: ElementId,
    row: &Row,
    plain: bool,
    indent: f32,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let options = RowOptions {
        disabled: row.disabled,
    };
    let content = (row.render)(options, window, cx);
    // A plain row has no box around it: its text starts where a box's rows' text does.
    let pad = if plain {
        theme::SETTINGS_TEXT_INSET
    } else {
        theme::SETTINGS_ROW_PADDING_X
    };
    let element = div()
        .id(id)
        .w_full()
        .px(px(pad))
        .pl(px(pad + indent))
        .when(!plain, |row| row.py(px(theme::SETTINGS_ROW_PADDING_Y)))
        .when(!plain && row.height.is_none(), |row| {
            row.min_h(px(theme::SETTINGS_ROW_HEIGHT))
        })
        .when_some(row.height.filter(|_| !plain), |row, height| {
            row.h(px(height))
        })
        .flex()
        .items_center();
    // Controls dim themselves when disabled; a custom row draws what it dims.
    let Some(title) = &row.title else {
        return element
            .child(div().w_full().child(content))
            .into_any_element();
    };
    let text = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::SETTINGS_DESCRIPTION_GAP))
        .when(row.disabled, |text| text.opacity(theme::DISABLED_OPACITY))
        .child(
            Label::new(title.clone())
                .text_size(theme::text(theme::ROW_TEXT_SIZE))
                .text_color(palette.text),
        )
        .children(row.description.as_ref().map(|description| {
            Label::new(description.clone())
                .text_size(theme::text(theme::MUTED_TEXT_SIZE))
                .text_color(palette.muted)
        }));
    // The description is told to assistive technology on the control it describes.
    let control = div()
        .id("control")
        .when_some(row.description.clone(), |control, description| {
            control.aria_description(SharedString::from(description))
        })
        .test_support()
        .flex_none()
        .child(content);
    element
        .justify_between()
        .gap(px(theme::LABEL_GAP))
        .child(text)
        .child(control)
        .into_any_element()
}
