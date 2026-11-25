//! Widgets (the Home page): the widgets of `[[home.widgets]]` in the file's order, one line
//! each: a grip, its plugin's icon, its title over the plugin's name and its params, a size
//! dropdown of the sizes the widget declares, and remove. A widget is dragged by its whole
//! line (all but its controls), which follows the pointer while the line it came from stays
//! dimmed; dropping it on another puts it in that one's place. Params are not edited here: a
//! widget gets them where it is made, Add to Home in the launcher. An entry the launcher
//! cannot read is a greyed line that is never moved or changed; a widget no installed plugin
//! offers is greyed, movable and removable. Every change is written at once through
//! `home_file::edit_home`, as in the launcher's Edit Home.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::select::Caret;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::component::{Icon, Size as ControlSize};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Render, SharedString, Size,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px, relative, svg, transparent_black,
};
use tomakast_core::settings::home_rows;
use tomakast_core::{
    ADD_TO_HOME, CatalogPlugin, HomeEdit, HomeEntry, PluginCatalog, WidgetDecl, WidgetKey,
    WidgetSize,
};

use super::plugins::plugin_icon;
use super::{error_line, note, with_unrecognized};
use crate::settings::SettingsPage;
use crate::settings::fields::PickField;
use crate::settings::fields::PickItem;
use crate::settings::layout::{Group, Page, PageIcon, Row, Section, page_icon};
use crate::settings::window::{SettingsWindow, home_error_key};
use crate::theme::{self, Palette};
use crate::views::action_panel::size_title;

/// What the size dropdown shows when the file gives the widget no size.
const DEFAULT_SIZE: &str = "Default";
/// Between the plugin's name and each param value in a widget's second line.
const SEPARATOR: &str = " · ";
/// The drag handle's glyph and the plugin's icon.
const HANDLE_SIZE: f32 = 16.;
const WIDGET_ICON_SIZE: f32 = 24.;
/// The line that shows where a dragged widget goes.
const INSERTION_LINE: f32 = 2.;
/// How opaque the line that follows the pointer while a widget is dragged is.
const DRAG_IMAGE_OPACITY: f32 = 0.9;
/// The plugin and the declaration of `widget`, when an installed plugin offers it.
fn widget_decl<'a>(
    catalog: &'a PluginCatalog,
    widget: &WidgetKey,
) -> Option<(&'a CatalogPlugin, &'a WidgetDecl)> {
    let plugin = catalog.plugins.iter().find(|p| p.id == widget.provider)?;
    let decl = plugin.widgets.iter().find(|w| w.id == widget.widget)?;
    Some((plugin, decl))
}

/// How widget `tile` (an index of the entries the launcher reads) is named to assistive
/// technology: identical widgets are allowed, so the position tells them apart.
fn widget_name(title: &str, tile: usize) -> String {
    format!("{title} (widget {})", tile + 1)
}

/// The `size` entry `ix` of `home.widgets` holds, as written.
fn raw_size(view: &SettingsWindow, ix: usize) -> Option<toml::Value> {
    view.file
        .file
        .as_ref()?
        .get("home")?
        .get("widgets")?
        .as_array()?
        .get(ix)?
        .get("size")
        .cloned()
}

pub(super) fn page(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    palette: &Palette,
) -> Page {
    let mut page = Page::new(
        SettingsPage::Home,
        Section::Launcher,
        "Widgets",
        PageIcon::Named("layout-grid"),
    )
    .group(Group::new().plain().row(note(
        "home-note",
        format!("Add widgets from the launcher: choose {ADD_TO_HOME} on a result."),
        palette,
    )));
    // A file that is not TOML has the banner; one whose Home has the wrong form says so here.
    let rows = match home_rows(&view.file.text) {
        Ok(rows) => rows,
        Err(e) if view.file.file.is_some() => {
            let error = format!("{e:#}");
            page = page.group(Group::new().plain().row(Row::custom(move |_, _, cx| {
                error_line("home-error".into(), error.clone(), &theme::palette(cx))
            })));
            Vec::new()
        }
        Err(_) => Vec::new(),
    };
    if !rows.is_empty() {
        let disabled = view.file.error().is_some();
        let mut tile = 0;
        let rows = rows.into_iter().enumerate().map(|(ix, row)| match row {
            None => unreadable(ix, palette),
            Some(entry) => {
                let size = raw_size(view, ix);
                let row = widget_row(view, this, tile, entry, size).disabled(disabled);
                tile += 1;
                row
            }
        });
        page = page.group(Group::new().rows(rows.collect::<Vec<_>>()));
    }
    with_unrecognized(page, view, this, |path| path[0] == "home")
}

/// The greyed row of entry `ix` of the list, which the launcher skips: neither dragged nor
/// dropped on.
fn unreadable(ix: usize, palette: &Palette) -> Row {
    let muted = palette.muted;
    let text = "An entry the launcher cannot read";
    Row::custom(move |_, _, _| {
        div()
            .id(("home-unreadable", ix))
            .aria_label(text)
            .test_support()
            .text_size(theme::text(theme::ROW_TEXT_SIZE))
            .text_color(muted)
            .child(text)
    })
}

/// What a widget's row draws, gathered before the layout asks for it.
struct Widget {
    tile: usize,
    /// The widget's title, or, when no installed plugin offers it, its id within its plugin.
    title: String,
    /// The plugin's name (or why the widget is greyed) and the param values, joined.
    subtitle: String,
    icon: PageIcon,
    /// No installed plugin offers the widget: it can be moved and removed, nothing else.
    greyed: bool,
    /// The size dropdown, when an installed plugin offers the widget.
    sizes: Option<Sizes>,
    /// The window's list field for the size, when there is one.
    size_field: Option<gpui_kit::Entity<PickField>>,
    /// The file's size when the widget cannot have it, as written: the default is used.
    invalid_size: Option<String>,
    error: Option<String>,
}

/// What the size list shows (the window makes the list itself: [`size_lists`]).
#[derive(Clone)]
struct Sizes {
    /// The file's size, when the widget can have it.
    current: Option<WidgetSize>,
}

/// A widget being dragged: its index.
#[derive(Clone)]
struct DraggedWidget {
    tile: usize,
}

/// What follows the pointer while a widget is dragged: its whole row of the box, as large as
/// the row it was lifted from, raised over the page and a little translucent. Only a picture:
/// nothing in it is a control or answers the pointer; of what it shows, only the widget's
/// title and subtitle are text a screen reader can read.
struct WidgetImage {
    row: Rc<Widget>,
    size: Size<Pixels>,
}

impl Render for WidgetImage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let row = &self.row;
        // The controls' look drawn plainly, as the kit draws a medium dropdown button and an
        // extra-small ghost icon button: not buttons, and nothing a screen reader names.
        let size = row.sizes.as_ref().map(|sizes| {
            let fills = palette.control_fills();
            div()
                .id(("home-image-size", row.tile))
                .test_support()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .w(px(theme::settings_control_width()))
                .h_8()
                .px_2p5()
                .border_1()
                .rounded(px(theme::CONTROL_RADIUS))
                .bg(fills.rest)
                .border_color(fills.border)
                .text_color(palette.text)
                .text_sm()
                .line_height(relative(1.25))
                .whitespace_nowrap()
                .child(div().min_w_0().text_ellipsis().child(shown_size(sizes)))
                .child(Caret::new(ControlSize::Medium).text_color(palette.text.opacity(0.75)))
                .into_any_element()
        });
        let remove = div()
            .id(("home-image-remove", row.tile))
            .test_support()
            .flex_none()
            .size_5()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(Lucide::Trash)
                    .with_size(ControlSize::XSmall)
                    .text_color(cx.theme().secondary_foreground),
            )
            .into_any_element();
        // A row of the page's box, the line in it past the row's padding: the line is where it
        // was on the page when the drag began.
        let (pad_x, pad_y) = (theme::SETTINGS_ROW_PADDING_X, theme::SETTINGS_ROW_PADDING_Y);
        let card = div()
            .absolute()
            .inset_0()
            .rounded(px(theme::SETTINGS_GROUP_RADIUS))
            .border_1()
            .border_color(palette.group_border)
            .bg(palette.group)
            .shadow_lg();
        div()
            .id("home-drag-image")
            .test_support()
            .relative()
            .w(self.size.width + px(2. * pad_x))
            .h(self.size.height + px(2. * pad_y))
            .opacity(DRAG_IMAGE_OPACITY)
            .child(card)
            .child(
                widget_line(widget_face(row, &palette), widget_controls(size, remove))
                    .absolute()
                    .left(px(pad_x))
                    .top(px(pad_y))
                    .w(self.size.width)
                    .h(self.size.height),
            )
    }
}

/// Whether widget `tile`'s line is the one a drag lifted: dimmed until the drop. `lifted` is
/// the last widget a drag lifted, `dragging` whether a drag is on.
fn is_lifted(lifted: Option<usize>, dragging: bool, tile: usize) -> bool {
    dragging && lifted == Some(tile)
}

/// The left of a widget's line: the handle, the plugin's icon and the title over the plugin's
/// name and the param values.
fn widget_face(row: &Widget, palette: &Palette) -> Div {
    let title = Label::new(row.title.clone())
        .text_size(theme::text(theme::ROW_TEXT_SIZE))
        .text_color(if row.greyed {
            palette.muted
        } else {
            palette.text
        });
    let subtitle = (!row.subtitle.is_empty()).then(|| {
        Label::new(row.subtitle.clone())
            .text_size(theme::text(theme::SMALL_TEXT_SIZE))
            .text_color(palette.muted)
    });
    let handle = svg()
        .path("icons/grip-vertical.svg")
        .flex_none()
        .size(px(HANDLE_SIZE))
        .text_color(palette.muted);
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(theme::SETTINGS_ENTRY_ICON_GAP))
        .child(handle)
        .child(
            div()
                .flex_none()
                .when(row.greyed, |icon| icon.opacity(theme::DISABLED_OPACITY))
                .child(page_icon(&row.icon, WIDGET_ICON_SIZE, palette)),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(title)
                .children(subtitle),
        )
}

/// The right of a widget's line: the size dropdown, when there is one, and remove.
fn widget_controls(size: Option<AnyElement>, remove: AnyElement) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .children(size)
        .child(remove)
}

/// A widget's line: `face` at the left, `controls` at the right.
fn widget_line(face: impl IntoElement, controls: impl IntoElement) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(face)
        .child(controls)
}

fn widget_row(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    tile: usize,
    entry: HomeEntry,
    raw_size: Option<toml::Value>,
) -> Row {
    let values = entry
        .params
        .iter()
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty());
    let error = view.errors.get(&home_error_key(tile)).cloned();
    let row = match widget_decl(&view.catalog, &entry.widget) {
        Some((plugin, decl)) => {
            let current = entry.size.filter(|size| decl.sizes.contains(size));
            let invalid_size = raw_size
                .filter(|_| current.is_none())
                .map(|value| value.to_string());
            Widget {
                tile,
                title: decl.title.clone(),
                subtitle: std::iter::once(plugin.name.clone())
                    .chain(values)
                    .collect::<Vec<_>>()
                    .join(SEPARATOR),
                icon: plugin_icon(plugin),
                greyed: false,
                sizes: Some(Sizes { current }),
                size_field: view.home_size(tile),
                invalid_size,
                error,
            }
        }
        None => {
            let installed = view
                .catalog
                .plugins
                .iter()
                .find(|p| p.id == entry.widget.provider);
            // While plugins are not running nothing is known about any widget.
            let why = match installed {
                _ if view.catalog.unavailable.is_some() => None,
                Some(plugin) => Some(format!("Not offered by {}", plugin.name)),
                None => Some("Not installed".to_string()),
            };
            // The plugin is named where the subtitle does not say it: by its name when
            // installed, else by its id.
            let plugin = match (installed, &why) {
                (Some(_), Some(_)) => None,
                (Some(plugin), None) => Some(plugin.name.clone()),
                (None, _) => Some(entry.widget.provider.clone()),
            };
            Widget {
                tile,
                title: entry.widget.widget.clone(),
                subtitle: plugin
                    .into_iter()
                    .chain(why)
                    .chain(values)
                    .collect::<Vec<_>>()
                    .join(SEPARATOR),
                // Dimmed when drawn, as the rest of the row.
                icon: installed.map_or_else(
                    || PageIcon::Item {
                        icon: None,
                        monogram: entry.widget.provider.clone(),
                    },
                    plugin_icon,
                ),
                greyed: true,
                sizes: None,
                size_field: None,
                invalid_size: None,
                error,
            }
        }
    };
    let keywords = [
        entry.widget.to_string(),
        row.title.clone(),
        row.subtitle.clone(),
    ];
    let row = Rc::new(row);
    let this = this.clone();
    let lifted = view.lifted_widget.clone();
    Row::custom(move |options, _, cx| render_widget(&row, &this, &lifted, options.disabled, cx))
        .keywords(keywords)
}

/// Applies `edit` to widget `tile` once clicked.
fn on_edit(
    this: &WeakEntity<SettingsWindow>,
    tile: usize,
    edit: HomeEdit,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let this = this.clone();
    move |_, window, cx| {
        let edit = edit.clone();
        let _ = this.update(cx, |view, cx| view.edit_home(tile, edit, window, cx));
    }
}

fn render_widget(
    row: &Rc<Widget>,
    this: &WeakEntity<SettingsWindow>,
    lifted: &Rc<Cell<Option<usize>>>,
    disabled: bool,
    cx: &App,
) -> AnyElement {
    let palette = theme::palette(cx);
    let accent = cx.theme().primary;
    let name = widget_name(&row.title, row.tile);
    let tile = row.tile;
    let face = widget_face(row, &palette)
        .id(("home-grab", tile))
        .aria_label(format!("Drag {name} to reorder"))
        .test_support();
    let size = row.size_field.clone().map(|field| field.into_any_element());
    let remove = Button::new(("home-remove", tile))
        .icon(Lucide::Trash)
        .ghost()
        .xsmall()
        .accessibility_label(format!("Remove {name}"))
        .tooltip("Remove")
        .disabled(disabled)
        .on_click(on_edit(this, tile, HomeEdit::Remove(tile)))
        .into_any_element();
    // The controls are clicked, never dragged: a press on them is not the line's.
    let controls = widget_controls(size, remove)
        .id("home-controls")
        .cursor_default()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    // The image under the pointer is drawn around a line as large as this one, as the last
    // frame laid it out.
    let line_size = Rc::new(Cell::new(Size::default()));
    let line = widget_line(face, controls)
        .id(("home-line", tile))
        .test_support()
        .when(
            is_lifted(lifted.get(), cx.has_active_drag(), tile),
            |line| {
                // The mark, out of the flow and of no size, is how the dim is found.
                line.opacity(theme::DISABLED_OPACITY).child(
                    div()
                        .id(("home-line-dimmed", tile))
                        .test_support()
                        .absolute()
                        .size_0(),
                )
            },
        );
    let lines = row
        .invalid_size
        .as_ref()
        .map(|value| {
            error_line(
                ("home-size-error", tile).into(),
                format!("Not valid here: {value}; the default is used"),
                &palette,
            )
        })
        .into_iter()
        .chain(
            row.error
                .clone()
                .map(|error| error_line(("home-row-error", tile).into(), error, &palette)),
        );
    // The row reaches over the layout's padding, so the whole of it takes a drop, and the
    // insertion line sits on its edge: under it when the dragged widget comes from above
    // (it ends up after this one), over it when from below. The lines' room is always
    // there, so nothing moves while dragging.
    let pad = theme::SETTINGS_ROW_PADDING_Y;
    let before = div()
        .id(("home-drop-before", tile))
        .test_support()
        .w_full()
        .px(px(theme::SETTINGS_ROW_PADDING_X))
        .pt(px(pad - INSERTION_LINE))
        .border_t(px(INSERTION_LINE))
        .border_color(transparent_black())
        .when(!disabled, |before| {
            before.drag_over::<DraggedWidget>(move |style, dragged, _, _| {
                if dragged.tile > tile {
                    style.border_color(accent)
                } else {
                    style
                }
            })
        })
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .on_children_prepainted({
                    let line_size = line_size.clone();
                    move |bounds, _, _| {
                        if let Some(line) = bounds.first() {
                            line_size.set(line.size);
                        }
                    }
                })
                .child(line)
                .children(lines),
        );
    // Its own id scopes the ids of its controls, which repeat across rows. It is the whole
    // row of the box, over the layout's padding on every side: a drag starts anywhere on it.
    let pad_x = theme::SETTINGS_ROW_PADDING_X;
    div()
        .id(("home-widget", tile))
        .test_support()
        .mx(px(-pad_x))
        .my(px(-pad))
        .pb(px(pad - INSERTION_LINE))
        .border_b(px(INSERTION_LINE))
        .border_color(transparent_black())
        .when(!disabled, |after| {
            let this = this.clone();
            let row = row.clone();
            let lifted = lifted.clone();
            after
                .cursor_grab()
                .on_drag(DraggedWidget { tile }, move |_, _, _, cx| {
                    lifted.set(Some(tile));
                    let row = row.clone();
                    let size = line_size.get();
                    cx.new(|_| WidgetImage { row, size })
                })
                .drag_over::<DraggedWidget>(move |style, dragged, _, _| {
                    if dragged.tile < tile {
                        style.border_color(accent)
                    } else {
                        style
                    }
                })
                .on_drop(move |dragged: &DraggedWidget, window, cx| {
                    let from = dragged.tile;
                    if from == tile {
                        return;
                    }
                    let edit = HomeEdit::Move { from, to: tile };
                    let _ = this.update(cx, |view, cx| view.edit_home(from, edit, window, cx));
                })
        })
        .child(before)
        .into_any_element()
}

/// What the size list shows: the file's size, or "Default".
fn shown_size(sizes: &Sizes) -> SharedString {
    sizes.current.map_or(DEFAULT_SIZE, size_title).into()
}

/// The size lists of the Home widgets an installed plugin offers, by tile, as the window makes
/// their fields: "Default" (which removes the size), then the sizes the widget declares.
pub(crate) struct SizeList {
    pub(crate) tile: usize,
    /// What the list is called: "Size of Weather (widget 1)".
    pub(crate) title: String,
    pub(crate) items: Vec<PickItem>,
    /// The file's size when the widget can have it; empty: none (the default).
    pub(crate) value: String,
    /// The file holds a size the widget cannot have: any pick writes.
    pub(crate) invalid: bool,
}

pub(crate) fn size_lists(view: &SettingsWindow) -> Vec<SizeList> {
    let Ok(rows) = home_rows(&view.file.text) else {
        return Vec::new();
    };
    let mut lists = Vec::new();
    let mut tile = 0;
    for (ix, row) in rows.into_iter().enumerate() {
        let Some(entry) = row else {
            continue;
        };
        if let Some((_, decl)) = widget_decl(&view.catalog, &entry.widget) {
            let current = entry.size.filter(|size| decl.sizes.contains(size));
            let invalid = current.is_none() && raw_size(view, ix).is_some();
            let items = std::iter::once(PickItem::new(SharedString::default(), DEFAULT_SIZE))
                .chain(
                    decl.sizes
                        .iter()
                        .map(|size| PickItem::new(size.as_str(), size_title(*size))),
                )
                .collect();
            lists.push(SizeList {
                tile,
                title: format!("Size of {}", widget_name(&decl.title, tile)),
                items,
                value: current
                    .map(|size| size.as_str().to_string())
                    .unwrap_or_default(),
                invalid,
            });
        }
        tile += 1;
    }
    lists
}

#[cfg(test)]
mod tests {
    use super::is_lifted;

    /// The lifted line is dimmed while its drag is on, and only then: a drop anywhere, or a
    /// drag given up, ends the drag, and the line is drawn as before.
    #[test]
    fn only_the_lifted_line_is_dimmed_while_dragging() {
        assert!(is_lifted(Some(2), true, 2));
        assert!(!is_lifted(Some(2), true, 1));
        assert!(!is_lifted(Some(2), false, 2));
        assert!(!is_lifted(None, true, 2));
    }
}
