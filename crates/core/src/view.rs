//! Module-owned views and the events they send back.
//!
//! Views are flat: no view contains another view, and sections are a field on
//! rows. Every type here is expressible in WIT 0.1 (see `plugin.wit`), except
//! [`ListLayout`]: a list's layout is for built-in modules only, and a plugin's
//! lists are always [`ListLayout::Rows`].

use crate::index::{Action, Icon};
use crate::readings::QueryReading;

/// What the UI asks the host about the module that owns a view: whether its
/// links may open (asked when a link is clicked) and, for a plugin, the name
/// shown with its views. Defined here, not in the UI, so the plugin host
/// implements it without depending on GPUI.
pub trait LinkPolicy {
    fn may_open_links(&self, provider: &str) -> bool;

    /// The name of the plugin `provider`, shown in the footer of its views as
    /// "<name> · <view title>" so that a plugin's view cannot pass for one of
    /// the launcher's own; `None` for a built-in module.
    fn plugin_name(&self, _provider: &str) -> Option<String> {
        None
    }
}

/// Identifier of a module-owned view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ViewId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    List(ListView),
    Grid(GridView),
    Detail(DetailView),
    Form(FormView),
    Scratchpad(ScratchpadView),
}

impl View {
    pub fn id(&self) -> &ViewId {
        match self {
            View::List(v) => &v.id,
            View::Grid(v) => &v.id,
            View::Detail(v) => &v.id,
            View::Form(v) => &v.id,
            View::Scratchpad(v) => &v.id,
        }
    }
}

/// Who filters a list as the user types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    CoreFilter,
    Module,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListView {
    pub id: ViewId,
    pub title: String,
    pub placeholder: String,
    pub search: SearchMode,
    pub query: String,
    pub revision: u64,
    pub rows: Vec<ListRow>,
    pub show_detail: bool,
    pub layout: ListLayout,
    /// The id of a row action the footer shows with its keys, after the primary one, while
    /// the selected row has it (and the action has a usable shortcut). Internal, for
    /// built-in modules, like `layout`: there is no WIT field, so a plugin's list has none.
    pub footer_action: Option<String>,
    /// The list is not ready yet (its module is still gathering what it lists): the
    /// launcher shows the busy line under the header, as for a busy pad. Internal, like
    /// `layout`: a plugin's list is never busy.
    pub busy: bool,
}

/// How a list view draws its rows. Internal, for built-in modules: there is no
/// WIT field, so a plugin's list is always `Rows`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListLayout {
    #[default]
    Rows,
    /// Tiles of `columns` per row (0 means 1); each tile shows the row's icon large
    /// and the row's title as its caption.
    Grid { columns: u8 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub id: String,
    pub section: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    pub accessories: Vec<Accessory>,
    pub detail_markdown: Option<String>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Accessory {
    Text(String),
    Tag(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridView {
    pub id: ViewId,
    pub title: String,
    pub columns: u8,
    pub tiles: Vec<GridTile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridTile {
    pub id: String,
    pub image: Option<Icon>,
    pub caption: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailView {
    pub id: ViewId,
    pub title: String,
    pub markdown: String,
    pub metadata: Vec<(String, String)>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormView {
    pub id: ViewId,
    pub title: String,
    pub fields: Vec<FormField>,
    pub submit_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormField {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    /// A module-side error shown under the field ("This name is taken").
    pub error: Option<String>,
}

/// Dates are ISO `YYYY-MM-DD`; dropdown options are `(value, label)`.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Text {
        value: String,
        placeholder: String,
    },
    Multiline {
        value: String,
    },
    Checkbox {
        value: bool,
    },
    Dropdown {
        options: Vec<(String, String)>,
        value: String,
    },
    Date {
        value: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScratchpadView {
    pub id: ViewId,
    pub title: String,
    pub text: String,
    pub revision: u64,
    pub annotations: Vec<LineAnnotation>,
    /// Actions on the pad as a whole, like a detail view's: the row id of
    /// their events is the view's id. Enter types a newline, so they run from
    /// the action panel, by their shortcuts, and the first by Ctrl/Cmd+Enter.
    pub actions: Vec<Action>,
    /// Markdown shown in a pane below the editor; when set, the pad is a compose pad: the
    /// editor wraps and `annotations` are not shown.
    pub output: Option<String>,
    /// Buttons between a compose pad's editor and its pane; a sheet shows none.
    pub toolbar: Vec<ToolbarButton>,
    /// The pad is working on its text: the launcher shows it under the header.
    pub busy: bool,
    /// Shown, muted, in the empty editor; empty: none. Set when the pad is pushed.
    pub placeholder: String,
    /// Markdown after `output`, at the body size: it does not grow with the text.
    pub detail: Option<String>,
}

/// A button in a compose pad's toolbar: plain (no options) it runs as a pad action with its
/// id; with options it opens a menu of them, and choosing one sends its id as the row id
/// and the button's as the action id.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarButton {
    pub id: String,
    pub label: String,
    pub icon: Option<Icon>,
    pub shortcut: Option<String>,
    pub options: Vec<MenuOption>,
    /// The option shown as current.
    pub selected: Option<String>,
}

/// One choice in a toolbar button's menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuOption {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    /// Matched by the filter as the title and the subtitle are.
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineAnnotation {
    pub line: u32,
    pub text: String,
    pub is_error: bool,
    /// A colour square drawn before the text: `#rrggbb` or `#rrggbbaa`.
    pub swatch: Option<String>,
}

/// A swatch colour, `#rrggbb` or `#rrggbbaa` (hex digits in any case), as `0xrrggbbaa`;
/// `None` for anything else. Six digits are opaque.
pub fn swatch_rgba(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#')?;
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(if hex.len() == 6 {
        value << 8 | 0xff
    } else {
        value
    })
}

/// An event from a module-owned view.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewEvent {
    QueryChanged {
        revision: u64,
        text: String,
        /// The text on other layouts and scripts; see [`QueryReading`].
        readings: Vec<QueryReading>,
    },
    ActionChosen {
        row_id: String,
        action_id: String,
    },
    FormSubmitted {
        values: Vec<(String, FieldValue)>,
    },
    TextChanged {
        revision: u64,
        text: String,
    },
    /// The view left the screen: it was popped, or the window was hidden.
    /// It is sent once per view, after any pending `TextChanged`.
    Closed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Text(String),
    Bool(bool),
    Date(Option<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swatches_are_six_or_eight_hex_digits() {
        assert_eq!(swatch_rgba("#ff8800"), Some(0xff88_00ff));
        assert_eq!(swatch_rgba("#FF880080"), Some(0xff88_0080));
        for bad in ["#ff88", "ff8800", "#ff880", "#gg8800", "#ff8800808"] {
            assert_eq!(swatch_rgba(bad), None, "{bad}");
        }
    }
}
