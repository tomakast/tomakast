//! Module-owned views and the events they send back.
//!
//! Views are flat: no view contains another view, and sections are a field on
//! rows. Every type here is expressible in WIT 0.1 (see `plugin.wit`).

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
    /// the action panel or by their shortcuts.
    pub actions: Vec<Action>,
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
