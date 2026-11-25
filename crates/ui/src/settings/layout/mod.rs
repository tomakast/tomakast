//! The settings window's own layout: pages in sidebar sections, each a list of groups of
//! rows. The pages build this model, which the window keeps until what it is built from
//! changes; the sidebar, the page area and search all read it. The kit's controls draw
//! inside the rows; the layout around them is ours. A group may open and close under its
//! group row; which are open is the window's ([`OpenGroups`]), and a closed group's rows are
//! still there for search, only not drawn. Groups that open and close, one after the other,
//! are drawn as one table, each under its group row.

mod page;
pub(crate) mod search;
mod sidebar;
mod windowing;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::{AnyElement, App, FocusHandle, IntoElement, Window};
use tomakast_core::ItemId;

pub(crate) use page::{
    OnToggle, control_button, drawn_rows, empty_state, page_body, page_column, page_icon,
};
pub(crate) use sidebar::{OnSelect, sidebar};
pub(crate) use windowing::Viewports;

use super::SettingsPage;
use search::{GroupText, PageText, RowText};

/// A part of the sidebar; parts are separated by a gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    /// The launcher's own pages and those of its built-in modules.
    Launcher,
    /// Installed plugins, then the tables of plugins that are not.
    Plugins,
}

/// The icon of a sidebar entry.
#[derive(Clone)]
pub(crate) enum PageIcon {
    /// A bundled Lucide icon.
    Named(&'static str),
    /// An icon as the launcher draws an item's, or the monogram of `monogram`.
    Item {
        icon: Option<tomakast_core::Icon>,
        monogram: String,
    },
    /// The monogram of the text, greyed.
    Greyed(String),
}

/// A page of the window: a sidebar entry and what it shows.
pub(crate) struct Page {
    pub(crate) key: SettingsPage,
    pub(crate) section: Section,
    pub(crate) title: String,
    pub(crate) icon: PageIcon,
    pub(crate) groups: Vec<Group>,
}

impl Page {
    pub(crate) fn new(
        key: SettingsPage,
        section: Section,
        title: impl Into<String>,
        icon: PageIcon,
    ) -> Page {
        Page {
            key,
            section,
            title: title.into(),
            icon,
            groups: Vec::new(),
        }
    }

    pub(crate) fn group(mut self, group: Group) -> Page {
        self.groups.push(group);
        self
    }

    /// What search reads of the page.
    pub(crate) fn text(&self) -> PageText {
        PageText {
            title: self.title.clone(),
            groups: self.groups.iter().map(Group::text).collect(),
        }
    }
}

/// Which group of a page opens and closes: the same group from render to render.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GroupKey(pub(crate) String);

/// Rows under an optional heading and description, in a box.
#[derive(Default)]
pub(crate) struct Group {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) rows: Vec<Row>,
    /// Lines of text rather than settings (a note, an error): drawn on the page, in no box.
    pub(crate) plain: bool,
    /// Drawn before the title: where the group's rows come from.
    pub(crate) icon: Option<PageIcon>,
    /// The titles of the two columns of a table of rows (a name, and the control under the
    /// right one), over the rows; of a table of collapsible groups, those of its first group
    /// that has them, once.
    pub(crate) columns: Option<(String, String)>,
    /// The group opens and closes under a group row with its title, by this key; with the
    /// collapsible groups next to it, it is one table.
    pub(crate) collapsible: Option<GroupKey>,
    /// A collapsible group asks to be open (it holds what the user set): open the first
    /// time its page is shown, and opened again each time this turns true, unless the user
    /// closed it with a click.
    pub(crate) wants_open: bool,
}

impl Group {
    pub(crate) fn new() -> Group {
        Group::default()
    }

    pub(crate) fn title(mut self, title: impl Into<String>) -> Group {
        self.title = Some(title.into());
        self
    }

    pub(crate) fn description(mut self, description: impl Into<String>) -> Group {
        self.description = Some(description.into());
        self
    }

    /// Draws the rows as lines on the page, in no box.
    pub(crate) fn plain(mut self) -> Group {
        self.plain = true;
        self
    }

    pub(crate) fn row(mut self, row: Row) -> Group {
        self.rows.push(row);
        self
    }

    pub(crate) fn rows(mut self, rows: impl IntoIterator<Item = Row>) -> Group {
        self.rows.extend(rows);
        self
    }

    pub(crate) fn icon(mut self, icon: PageIcon) -> Group {
        self.icon = Some(icon);
        self
    }

    /// Draws the rows as a table under the titles of its two columns.
    pub(crate) fn columns(mut self, left: impl Into<String>, right: impl Into<String>) -> Group {
        self.columns = Some((left.into(), right.into()));
        self
    }

    /// Makes the group open and close under its group row; see [`Group::wants_open`].
    pub(crate) fn collapsible(mut self, key: GroupKey, wants_open: bool) -> Group {
        self.collapsible = Some(key);
        self.wants_open = wants_open;
        self
    }

    /// What search reads of the group.
    fn text(&self) -> GroupText {
        GroupText {
            title: self.title.clone(),
            rows: self
                .rows
                .iter()
                .map(|row| RowText {
                    title: row.title.clone(),
                    keywords: row.keywords.clone(),
                })
                .collect(),
        }
    }
}

/// Which collapsible groups are open: the window keeps it while it lives.
#[derive(Default)]
pub(crate) struct OpenGroups {
    /// By group, once its page was shown.
    groups: HashMap<GroupKey, GroupState>,
    /// Groups a search opened and a click closed again, until the query changes.
    closed_in_search: HashSet<GroupKey>,
    /// Each group row's, so Tab reaches it and Enter or Space turns it.
    focus: HashMap<GroupKey, FocusHandle>,
    /// Counts the times a group opened or closed: what lies under it moved.
    changes: u64,
}

/// How a collapsible group stands.
#[derive(Clone, Copy)]
struct GroupState {
    open: bool,
    /// The group's [`Group::wants_open`] when its page was last shown.
    wanted: bool,
    /// The user closed it with a click: it stays closed until a click opens it.
    closed_by_click: bool,
}

impl OpenGroups {
    /// Follows the collapsible groups of `page`, the page on display: a group seen for the
    /// first time is open as it asks; one seen before opens when it starts asking, unless the
    /// user closed it with a click.
    pub(crate) fn follow(&mut self, page: &Page) {
        for group in &page.groups {
            let Some(key) = &group.collapsible else {
                continue;
            };
            let wanted = group.wants_open;
            let state = self.groups.entry(key.clone()).or_insert_with(|| {
                self.changes += 1;
                GroupState {
                    open: wanted,
                    wanted,
                    closed_by_click: false,
                }
            });
            if wanted && !state.wanted && !state.closed_by_click && !state.open {
                state.open = true;
                self.changes += 1;
            }
            state.wanted = wanted;
        }
    }

    /// Whether the group `key` is open; `revealed`: a search opened it.
    pub(crate) fn is_open(&self, key: &GroupKey, revealed: bool) -> bool {
        if revealed {
            !self.closed_in_search.contains(key)
        } else {
            self.groups.get(key).is_some_and(|state| state.open)
        }
    }

    /// A click on the group row of `key`, open or not as it is drawn.
    pub(crate) fn toggle(&mut self, key: &GroupKey, revealed: bool) {
        self.changes += 1;
        if revealed {
            if !self.closed_in_search.remove(key) {
                self.closed_in_search.insert(key.clone());
            }
        } else if let Some(state) = self.groups.get_mut(key) {
            state.open = !state.open;
            state.closed_by_click = !state.open;
        }
    }

    /// The query changed: what a search opened is open again.
    pub(crate) fn search_changed(&mut self) {
        self.closed_in_search.clear();
        self.changes += 1;
    }

    /// A number that changes whenever a group opens or closes.
    pub(crate) fn changes(&self) -> u64 {
        self.changes
    }

    fn focus(&mut self, key: &GroupKey, cx: &mut App) -> FocusHandle {
        self.focus
            .entry(key.clone())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }
}

/// What a row's renderer is told.
#[derive(Clone, Copy)]
pub(crate) struct RowOptions {
    /// The row cannot be changed now (the file has an error).
    pub disabled: bool,
}

/// Draws a row's control, or the whole of a custom row.
pub(crate) type RenderRow = Rc<dyn Fn(RowOptions, &mut Window, &mut App) -> AnyElement>;

/// A row of a group: a title on the left and its control on the right, or a custom row drawn
/// across the full width.
pub(crate) struct Row {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    /// Words search finds the row by besides its title, the key name among them.
    pub(crate) keywords: Vec<String>,
    pub(crate) disabled: bool,
    /// The item whose hotkey field the row draws: the window makes the field once the row
    /// is drawn.
    pub(crate) hotkey_of: Option<ItemId>,
    /// The row is always this tall: of a box's lines up to the first whose height varies, the
    /// page area draws only those in view, in exact room for the others.
    pub(crate) height: Option<f32>,
    pub(crate) render: RenderRow,
}

impl Row {
    /// A row titled `title` with the control `render` draws.
    pub(crate) fn new<E: IntoElement>(
        title: impl Into<String>,
        render: impl Fn(RowOptions, &mut Window, &mut App) -> E + 'static,
    ) -> Row {
        Row {
            title: Some(title.into()),
            ..Row::custom(render)
        }
    }

    /// A row `render` draws whole.
    pub(crate) fn custom<E: IntoElement>(
        render: impl Fn(RowOptions, &mut Window, &mut App) -> E + 'static,
    ) -> Row {
        Row {
            title: None,
            description: None,
            keywords: Vec::new(),
            disabled: false,
            hotkey_of: None,
            height: None,
            render: Rc::new(move |options, window, cx| {
                render(options, window, cx).into_any_element()
            }),
        }
    }

    pub(crate) fn description(mut self, description: impl Into<String>) -> Row {
        self.description = Some(description.into());
        self
    }

    pub(crate) fn keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Row {
        self.keywords.extend(keywords.into_iter().map(Into::into));
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Row {
        self.disabled = disabled;
        self
    }

    /// The row draws the hotkey field of `item`.
    pub(crate) fn hotkey_of(mut self, item: ItemId) -> Row {
        self.hotkey_of = Some(item);
        self
    }

    /// The row is always `height` tall, whatever it holds; see [`Row::height`].
    pub(crate) fn fixed_height(mut self, height: f32) -> Row {
        self.height = Some(height);
        self
    }
}
