//! Builders for the contract's records.
//!
//! Everything with an id (an item, an action, a view, a row, a tile, a field, a dropdown
//! option) is made by a constructor that checks the id and returns [`InvalidId`] when it
//! breaks the contract's id rule: at most 256 bytes, no control characters and no Unicode
//! format characters. An item id must also be `<plugin-id>/<local-id>`. The other fields are
//! set with methods that take `self` and return it, so a record reads as one expression:
//!
//! ```
//! # use tomakast_plugin::{Item, View, Effect, InvalidId};
//! # fn f() -> Result<(), InvalidId> {
//! let item = Item::new("weather/today", "Today")?
//!     .subtitle("Sunny, 21 °C")
//!     .action("show", "Show forecast")?;
//! let effect = Effect::push_view(View::detail("forecast", "Forecast")?.markdown("# Sunny"));
//! # Ok(()) }
//! ```
//!
//! What the builders do not check, the launcher does when it receives an effect: display text
//! (titles, subtitles, labels) is one line of at most 1 KiB, and the caps of each record
//! (16 actions, 2000 rows and so on). An effect that breaks one is rejected with an error
//! toast. The records' fields are public, for what the builders do not cover.

use std::fmt;

use crate::{
    Action, Align, ButtonNode, CanvasNode, ClipRect, Color, ColorStop, ColorToken, DetailView,
    Draw, DropdownField, DropdownOption, Edges, Effect, EllipseCmd, FieldKind, FormField, FormView,
    GridTile, GridView, HttpRequest, Icon, IconNode, ImageCmd, ImageFit, ImageFormat, ImageNode,
    Item, Justify, KeyValue, Layout, LineAnnotation, LinearGradient, ListRow, ListView, Node,
    NodeKind, Paint, PathCmd, PathSegment, Point, RectCmd, ScratchpadView, SearchMode, ShapeStyle,
    TextAlign, TextCmd, TextField, TextNode, TextStyle, Toast, ToastKind, Transform, View,
    WidgetAnswer, WidgetFrame, WidgetImage, WidgetParam, WidgetSize,
};

/// Longest id, in bytes.
pub const MAX_ID_BYTES: usize = 256;

/// Ancestors a frame node may have: nodes nest at most this deep.
pub const MAX_NODE_DEPTH: usize = 32;

/// An id that breaks the contract's id rule; see the [module docs](self). It converts into
/// `String`, so `?` works in the [`Plugin`](crate::Plugin) functions, whose error is the text
/// the launcher shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidId {
    what: &'static str,
    id: String,
    problem: &'static str,
}

impl InvalidId {
    /// What the id names: `"item"`, `"action"`, `"view"`, `"row"`, `"tile"`, `"field"`,
    /// `"dropdown option"` or `"widget"`.
    pub fn what(&self) -> &'static str {
        self.what
    }

    /// The id as given.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // At most 64 characters of the id, escaped.
        let shown: String = self.id.chars().take(64).collect();
        let more = if shown.len() < self.id.len() {
            "…"
        } else {
            ""
        };
        write!(
            f,
            "invalid {} id {:?}{more}: {}",
            self.what, shown, self.problem
        )
    }
}

impl std::error::Error for InvalidId {}

impl From<InvalidId> for String {
    fn from(err: InvalidId) -> String {
        err.to_string()
    }
}

/// `id` if it follows the id rule.
fn check(what: &'static str, id: impl Into<String>) -> Result<String, InvalidId> {
    let id = id.into();
    let problem = if id.len() > MAX_ID_BYTES {
        "longer than 256 bytes"
    } else if id.chars().any(char::is_control) {
        "contains a control character"
    } else if id.chars().any(is_format_char) {
        "contains an invisible formatting character"
    } else {
        return Ok(id);
    };
    Err(InvalidId { what, id, problem })
}

/// `id` if it follows the id rule and has the shape `<plugin-id>/<local-id>`, where
/// `<plugin-id>` has the shape of a manifest id and `<local-id>` is not empty.
fn check_item(id: impl Into<String>) -> Result<String, InvalidId> {
    let id = check("item", id)?;
    match id.split_once('/') {
        Some((plugin, local)) if is_plugin_id(plugin) && !local.is_empty() => Ok(id),
        _ => Err(InvalidId {
            what: "item",
            id,
            problem: "must be <plugin-id>/<local-id>, with the plugin id of plugin.toml \
                      (1-32 characters from a-z, 0-9 and '-')",
        }),
    }
}

/// Whether `id` has the shape of the `id` of `plugin.toml`: 1-32 characters from a-z, 0-9
/// and `-`.
fn is_plugin_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Unicode format characters (general category Cf, Unicode 16), as the launcher refuses them
/// in ids: the same table as `tomakast-core`'s (`crates/plugin-host/tests/sdk_drift.rs`
/// checks it).
const FORMAT_CHARS: &[(char, char)] = &[
    ('\u{00ad}', '\u{00ad}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061c}', '\u{061c}'),
    ('\u{06dd}', '\u{06dd}'),
    ('\u{070f}', '\u{070f}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08e2}', '\u{08e2}'),
    ('\u{180e}', '\u{180e}'),
    ('\u{200b}', '\u{200f}'),
    ('\u{202a}', '\u{202e}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206f}'),
    ('\u{feff}', '\u{feff}'),
    ('\u{fff9}', '\u{fffb}'),
    ('\u{110bd}', '\u{110bd}'),
    ('\u{110cd}', '\u{110cd}'),
    ('\u{13430}', '\u{1343f}'),
    ('\u{1bca0}', '\u{1bca3}'),
    ('\u{1d173}', '\u{1d17a}'),
    ('\u{e0001}', '\u{e0001}'),
    ('\u{e0020}', '\u{e007f}'),
];

fn is_format_char(c: char) -> bool {
    FORMAT_CHARS.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

impl Action {
    /// An action without a shortcut.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Result<Action, InvalidId> {
        Ok(Action {
            id: check("action", id)?,
            title: title.into(),
            shortcut: None,
        })
    }

    /// A key combination that runs the action while its row is selected, such as
    /// `ctrl+shift+c`. It must include `ctrl`, `alt` or `super`, and may not take the keys
    /// the user types and navigates with (see `action` in `plugin.wit`).
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Action {
        self.shortcut = Some(shortcut.into());
        self
    }
}

/// `action(id, title)` and `with_action(action)` for a record with an `actions` list. The
/// first action is the primary one (Enter); at most 16.
macro_rules! actions {
    ($ty:ty) => {
        impl $ty {
            /// Adds the action `id` titled `title`.
            pub fn action(
                self,
                id: impl Into<String>,
                title: impl Into<String>,
            ) -> Result<Self, InvalidId> {
                Ok(self.with_action(Action::new(id, title)?))
            }

            /// Adds `action` (made with [`Action::new`], for example with a shortcut).
            pub fn with_action(mut self, action: Action) -> Self {
                self.actions.push(action);
                self
            }
        }
    };
}

actions!(Item);
actions!(ListRow);
actions!(GridTile);
actions!(DetailView);
actions!(ScratchpadView);

impl Item {
    /// Starts an item with the id `<plugin-id>/<local-id>` (items with another plugin's
    /// prefix are dropped by the launcher).
    ///
    /// An item needs at least one action, so this gives an [`ItemDraft`], which becomes an
    /// `Item` with its first action:
    ///
    /// ```
    /// # use tomakast_plugin::{Item, InvalidId};
    /// # fn f() -> Result<Item, InvalidId> {
    /// let item: Item = Item::new("demo/one", "One")?.subtitle("first").action("open", "Open")?;
    /// # Ok(item) }
    /// ```
    ///
    /// A draft is not an item, so an item without actions does not compile:
    ///
    /// ```compile_fail
    /// # use tomakast_plugin::{Effect, Item};
    /// let draft = Item::new("demo/one", "One").unwrap();
    /// let effect = Effect::replace_items(vec![draft]);
    /// ```
    #[expect(
        clippy::new_ret_no_self,
        reason = "an item starts as a draft, so that one without actions does not compile"
    )]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Result<ItemDraft, InvalidId> {
        Ok(ItemDraft(Item {
            id: check_item(id)?,
            title: title.into(),
            subtitle: None,
            keywords: Vec::new(),
            icon: None,
            actions: Vec::new(),
        }))
    }

    /// The second line of the row.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Item {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Adds words the item is also found by (at most 32 in all).
    pub fn keywords<S: Into<String>>(mut self, keywords: impl IntoIterator<Item = S>) -> Item {
        self.keywords.extend(keywords.into_iter().map(Into::into));
        self
    }

    /// The row's icon.
    pub fn icon(mut self, icon: Icon) -> Item {
        self.icon = Some(icon);
        self
    }
}

/// An [`Item`] that has no action yet, made by [`Item::new`]. Its first action, the primary
/// one (Enter), makes it an `Item`: [`action`](ItemDraft::action) or
/// [`with_action`](ItemDraft::with_action).
#[derive(Debug, Clone)]
#[must_use = "an item draft becomes an item with its first action"]
pub struct ItemDraft(Item);

impl ItemDraft {
    /// The second line of the row.
    pub fn subtitle(self, subtitle: impl Into<String>) -> ItemDraft {
        ItemDraft(self.0.subtitle(subtitle))
    }

    /// Adds words the item is also found by (at most 32 in all).
    pub fn keywords<S: Into<String>>(self, keywords: impl IntoIterator<Item = S>) -> ItemDraft {
        ItemDraft(self.0.keywords(keywords))
    }

    /// The row's icon.
    pub fn icon(self, icon: Icon) -> ItemDraft {
        ItemDraft(self.0.icon(icon))
    }

    /// The item, with the primary action `id` titled `title`.
    pub fn action(
        self,
        id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<Item, InvalidId> {
        Ok(self.with_action(Action::new(id, title)?))
    }

    /// The item, with `action` (made with [`Action::new`]) as its primary action.
    pub fn with_action(self, action: Action) -> Item {
        self.0.with_action(action)
    }
}

impl View {
    /// A searchable list, filtered by the launcher over row titles
    /// ([`SearchMode::CoreFilter`]) until you set [`ListView::search`].
    pub fn list(id: impl Into<String>, title: impl Into<String>) -> Result<ListView, InvalidId> {
        Ok(ListView {
            id: check("view", id)?,
            title: title.into(),
            placeholder: String::new(),
            search: SearchMode::CoreFilter,
            query: String::new(),
            revision: 0,
            rows: Vec::new(),
            show_detail: false,
        })
    }

    /// A grid of tiles, one column until you set [`GridView::columns`].
    pub fn grid(id: impl Into<String>, title: impl Into<String>) -> Result<GridView, InvalidId> {
        Ok(GridView {
            id: check("view", id)?,
            title: title.into(),
            columns: 0,
            tiles: Vec::new(),
        })
    }

    /// Markdown with metadata and actions, empty until you set [`DetailView::markdown`].
    pub fn detail(
        id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<DetailView, InvalidId> {
        Ok(DetailView {
            id: check("view", id)?,
            title: title.into(),
            markdown: String::new(),
            metadata: Vec::new(),
            actions: Vec::new(),
        })
    }

    /// A form whose submit button reads `submit_title`; add fields with [`FormView::field`].
    pub fn form(
        id: impl Into<String>,
        title: impl Into<String>,
        submit_title: impl Into<String>,
    ) -> Result<FormView, InvalidId> {
        Ok(FormView {
            id: check("view", id)?,
            title: title.into(),
            fields: Vec::new(),
            submit_title: submit_title.into(),
        })
    }

    /// A text editor holding `text`, with per-line annotations (a calculator pad). Edits
    /// arrive as `text-changed` events; see the revision rule on [`ScratchpadView`].
    pub fn scratchpad(
        id: impl Into<String>,
        title: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<ScratchpadView, InvalidId> {
        Ok(ScratchpadView {
            id: check("view", id)?,
            title: title.into(),
            text: text.into(),
            revision: 0,
            annotations: Vec::new(),
            actions: Vec::new(),
        })
    }
}

macro_rules! into_view {
    ($($ty:ident => $variant:ident),*) => {$(
        impl From<$ty> for View {
            fn from(view: $ty) -> View {
                View::$variant(view)
            }
        }
    )*};
}

into_view!(
    ListView => ListView,
    GridView => GridView,
    DetailView => DetailView,
    FormView => FormView,
    ScratchpadView => ScratchpadView
);

impl ListView {
    /// The search field's placeholder.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> ListView {
        self.placeholder = placeholder.into();
        self
    }

    /// Who filters the rows: the launcher ([`SearchMode::CoreFilter`]) or the plugin
    /// ([`SearchMode::Module`], through `query-changed` events and `update-view`).
    pub fn search(mut self, search: SearchMode) -> ListView {
        self.search = search;
        self
    }

    /// The text in the search field when the view is pushed (an `update-view` keeps what
    /// the user typed).
    pub fn query(mut self, query: impl Into<String>) -> ListView {
        self.query = query.into();
        self
    }

    /// The revision this view answers: for an `update-view`, the `revision` of the
    /// `query-changed` event it answers, or the launcher drops it.
    pub fn revision(mut self, revision: u64) -> ListView {
        self.revision = revision;
        self
    }

    /// Adds a row.
    pub fn row(mut self, row: ListRow) -> ListView {
        self.rows.push(row);
        self
    }

    /// Adds rows (at most 2000 in all).
    pub fn rows(mut self, rows: impl IntoIterator<Item = ListRow>) -> ListView {
        self.rows.extend(rows);
        self
    }

    /// Shows a detail pane with the selected row's [`ListRow::detail`].
    pub fn show_detail(mut self, show: bool) -> ListView {
        self.show_detail = show;
        self
    }
}

impl ListRow {
    /// A row with no subtitle, icon, accessories, detail or actions.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Result<ListRow, InvalidId> {
        Ok(ListRow {
            id: check("row", id)?,
            section: None,
            title: title.into(),
            subtitle: None,
            icon: None,
            accessories: Vec::new(),
            detail_markdown: None,
            actions: Vec::new(),
        })
    }

    /// Starts a section with this header; the rows after it with no section stay in it.
    pub fn section(mut self, section: impl Into<String>) -> ListRow {
        self.section = Some(section.into());
        self
    }

    /// The second line of the row.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> ListRow {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// The row's icon.
    pub fn icon(mut self, icon: Icon) -> ListRow {
        self.icon = Some(icon);
        self
    }

    /// Adds a text or a tag at the end of the row (at most 8).
    pub fn accessory(mut self, accessory: crate::Accessory) -> ListRow {
        self.accessories.push(accessory);
        self
    }

    /// The markdown of the detail pane while this row is selected (at most 64 KiB).
    pub fn detail(mut self, markdown: impl Into<String>) -> ListRow {
        self.detail_markdown = Some(markdown.into());
        self
    }
}

impl GridView {
    /// The number of columns, at most 12; `0` means 1.
    pub fn columns(mut self, columns: u8) -> GridView {
        self.columns = columns;
        self
    }

    /// Adds a tile.
    pub fn tile(mut self, tile: GridTile) -> GridView {
        self.tiles.push(tile);
        self
    }

    /// Adds tiles (at most 500 in all).
    pub fn tiles(mut self, tiles: impl IntoIterator<Item = GridTile>) -> GridView {
        self.tiles.extend(tiles);
        self
    }
}

impl GridTile {
    /// A tile with no image and no actions.
    pub fn new(id: impl Into<String>, caption: impl Into<String>) -> Result<GridTile, InvalidId> {
        Ok(GridTile {
            id: check("tile", id)?,
            image: None,
            caption: caption.into(),
            actions: Vec::new(),
        })
    }

    /// The tile's image.
    pub fn image(mut self, image: Icon) -> GridTile {
        self.image = Some(image);
        self
    }
}

impl DetailView {
    /// The markdown shown (at most 64 KiB). It is inert: raw HTML shows as text and remote
    /// images are not loaded.
    pub fn markdown(mut self, markdown: impl Into<String>) -> DetailView {
        self.markdown = markdown.into();
        self
    }

    /// Adds a metadata entry (at most 64; each value at most 4 KiB).
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> DetailView {
        self.metadata.push(KeyValue {
            key: key.into(),
            value: value.into(),
        });
        self
    }
}

impl FormView {
    /// Adds a field (at most 50).
    pub fn field(mut self, field: FormField) -> FormView {
        self.fields.push(field);
        self
    }
}

impl FormField {
    /// A field of any kind.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        kind: FieldKind,
    ) -> Result<FormField, InvalidId> {
        Ok(FormField {
            id: check("field", id)?,
            label: label.into(),
            kind,
            error: None,
        })
    }

    /// A one-line text field holding `value`.
    pub fn text(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<FormField, InvalidId> {
        let kind = FieldKind::Text(TextField {
            value: value.into(),
            placeholder: String::new(),
        });
        FormField::new(id, label, kind)
    }

    /// A multi-line text field holding `value` (at most 256 KiB).
    pub fn multiline(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<FormField, InvalidId> {
        FormField::new(id, label, FieldKind::Multiline(value.into()))
    }

    /// A checkbox.
    pub fn checkbox(
        id: impl Into<String>,
        label: impl Into<String>,
        checked: bool,
    ) -> Result<FormField, InvalidId> {
        FormField::new(id, label, FieldKind::Checkbox(checked))
    }

    /// A dropdown of `options` (at most 200) with `value` selected; `value` follows the id
    /// rule.
    pub fn dropdown(
        id: impl Into<String>,
        label: impl Into<String>,
        options: Vec<DropdownOption>,
        value: impl Into<String>,
    ) -> Result<FormField, InvalidId> {
        let kind = FieldKind::Dropdown(DropdownField {
            options,
            value: check("dropdown option", value)?,
        });
        FormField::new(id, label, kind)
    }

    /// A date field, empty or holding `YYYY-MM-DD`.
    pub fn date(
        id: impl Into<String>,
        label: impl Into<String>,
        value: Option<String>,
    ) -> Result<FormField, InvalidId> {
        FormField::new(id, label, FieldKind::Date(value))
    }

    /// The placeholder of a text field; other kinds have none and are left as they are.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> FormField {
        if let FieldKind::Text(text) = &mut self.kind {
            text.placeholder = placeholder.into();
        }
        self
    }

    /// An error shown under the field, for example after a `form-submitted` you reject.
    pub fn error(mut self, error: impl Into<String>) -> FormField {
        self.error = Some(error.into());
        self
    }
}

impl DropdownOption {
    /// An option whose `value` (the id rule applies) is shown as `label`.
    pub fn new(
        value: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<DropdownOption, InvalidId> {
        Ok(DropdownOption {
            value: check("dropdown option", value)?,
            label: label.into(),
        })
    }
}

impl ScratchpadView {
    /// The revision this view answers: for an `update-view`, the `revision` of the
    /// `text-changed` event it answers, or the launcher drops it. Its text is ignored then.
    pub fn revision(mut self, revision: u64) -> ScratchpadView {
        self.revision = revision;
        self
    }

    /// Adds an annotation; the last one for a line wins.
    pub fn annotation(mut self, annotation: LineAnnotation) -> ScratchpadView {
        self.annotations.push(annotation);
        self
    }

    /// Adds annotations (at most 10 000 in all).
    pub fn annotations(
        mut self,
        annotations: impl IntoIterator<Item = LineAnnotation>,
    ) -> ScratchpadView {
        self.annotations.extend(annotations);
        self
    }
}

impl LineAnnotation {
    /// A note at the end of line `line` (0-based, below 10 000).
    pub fn note(line: u32, text: impl Into<String>) -> LineAnnotation {
        LineAnnotation {
            line,
            text: text.into(),
            is_error: false,
            swatch: None,
        }
    }

    /// Draws a colour square before the text: `#rrggbb` or `#rrggbbaa`.
    pub fn swatch(mut self, color: impl Into<String>) -> LineAnnotation {
        self.swatch = Some(color.into());
        self
    }

    /// An error note at the end of line `line` (0-based, below 10 000).
    pub fn error(line: u32, text: impl Into<String>) -> LineAnnotation {
        LineAnnotation {
            is_error: true,
            ..LineAnnotation::note(line, text)
        }
    }
}

impl Toast {
    /// A success toast, shown for 3 s.
    pub fn success(title: impl Into<String>) -> Toast {
        Toast::new(ToastKind::Success, title)
    }

    /// An error toast, shown for 5 s.
    pub fn error(title: impl Into<String>) -> Toast {
        Toast::new(ToastKind::Error, title)
    }

    /// A progress toast, shown until another toast replaces it.
    pub fn progress(title: impl Into<String>) -> Toast {
        Toast::new(ToastKind::Progress, title)
    }

    fn new(kind: ToastKind, title: impl Into<String>) -> Toast {
        Toast {
            kind,
            title: title.into(),
            message: None,
        }
    }

    /// A message under the title; it may span lines (at most 4 KiB).
    pub fn message(mut self, message: impl Into<String>) -> Toast {
        self.message = Some(message.into());
        self
    }
}

/// What each effect does, and when the launcher honours it, is on `effect` in `plugin.wit`.
impl Effect {
    /// Shows `view` on top of the launcher's stack (one per answer). A view whose id this
    /// plugin already has on the stack is not stacked twice: the launcher returns to it,
    /// closing the views above, and replaces it with `view`.
    pub fn push_view(view: impl Into<View>) -> Effect {
        Effect::PushView(view.into())
    }

    /// Updates the view with `view`'s id: the one the event came from, or the topmost of the
    /// plugin's views with that id.
    pub fn update_view(view: impl Into<View>) -> Effect {
        Effect::UpdateView(view.into())
    }

    /// Pops the view the event came from.
    pub fn pop_view() -> Effect {
        Effect::PopView
    }

    /// Hides the launcher.
    pub fn close_window() -> Effect {
        Effect::CloseWindow
    }

    /// Shows `toast` while the launcher window is open (at most 10 per 10 s).
    pub fn toast(toast: Toast) -> Effect {
        Effect::Toast(toast)
    }

    /// Copies `text` to the clipboard; needs `clipboard = true` and a user action.
    pub fn copy_text(text: impl Into<String>) -> Effect {
        Effect::CopyText(text.into())
    }

    /// Replaces the items this plugin indexed before; needs `index = true` (at most 1000).
    pub fn replace_items(items: Vec<Item>) -> Effect {
        Effect::ReplaceItems(items)
    }
}

impl HttpRequest {
    /// A request with `method` (GET, HEAD, POST, PUT, PATCH, DELETE or OPTIONS) to `url`, with
    /// no headers and no body.
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> HttpRequest {
        HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// A GET request to `url`.
    pub fn get(url: impl Into<String>) -> HttpRequest {
        HttpRequest::new("GET", url)
    }

    /// Adds a header. The launcher sets `host`, `content-length` and the other connection
    /// headers itself, and refuses a request that sets one.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> HttpRequest {
        self.headers.push(KeyValue {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    /// The request body (GET and HEAD carry none).
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> HttpRequest {
        self.body = Some(body.into());
        self
    }
}

actions!(WidgetAnswer);

impl WidgetAnswer {
    /// A tile for the widget `id` (one of the ids this plugin declares under `[[widgets]]`),
    /// with no params, in the widget's default size (the first of its `sizes`).
    pub fn new(id: impl Into<String>) -> Result<WidgetAnswer, InvalidId> {
        Ok(WidgetAnswer {
            id: check("widget", id)?,
            params: Vec::new(),
            size: None,
            actions: Vec::new(),
        })
    }

    /// Sets the param `key` (one the widget declares under `[[widgets.params]]`) to `value`.
    /// The params come back in the tile's render contexts and events, and "Add to Home" keeps
    /// them; all values of a tile together are at most 4 KiB. Setting a key again replaces
    /// its value.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> WidgetAnswer {
        let (key, value) = (key.into(), value.into());
        match self.params.iter_mut().find(|p| p.key == key) {
            Some(param) => param.value = value,
            None => self.params.push(WidgetParam { key, value }),
        }
        self
    }

    /// Shows the tile in `size`, one of the widget's `sizes`.
    pub fn size(mut self, size: WidgetSize) -> WidgetAnswer {
        self.size = Some(size);
        self
    }
}

/// The value of the param `key` in `params` (a render context's, a widget event's or a Home
/// tile's), or `None` when the tile has none.
///
/// ```
/// # use tomakast_plugin::{WidgetParam, param};
/// let params = vec![WidgetParam { key: "city".into(), value: "Oslo".into() }];
/// assert_eq!(param(&params, "city"), Some("Oslo"));
/// assert_eq!(param(&params, "units"), None);
/// ```
pub fn param<'a>(params: &'a [WidgetParam], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.value.as_str())
}

/// The index of a node in the frame being built; get one from [`FrameBuilder::root`] or
/// [`FrameBuilder::child`], and use it only with the builder it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeRef(u32);

/// Builds a [`WidgetFrame`] node by node. A node's parent is always an earlier node, so a
/// frame it builds follows the tree rule of the contract (node 0 is the root).
///
/// ```
/// # use tomakast_plugin::{Node, Layout, TextStyle, WidgetFrame};
/// let frame = WidgetFrame::new(Node::column(Layout::default().padding(8.)));
/// let root = frame.root();
/// let (frame, _) = frame.child(root, Node::text("Hello", TextStyle::Title));
/// let frame = frame.build();
/// assert_eq!(frame.nodes.len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct FrameBuilder {
    frame: WidgetFrame,
}

impl WidgetFrame {
    #[allow(clippy::new_ret_no_self)] // `WidgetFrame::new` starts its builder, as `Item::new` does
    /// A frame whose root is `root` (a row, a column or an overlay, to hold children).
    pub fn new(root: Node) -> FrameBuilder {
        FrameBuilder {
            frame: WidgetFrame {
                nodes: vec![Node {
                    parent: None,
                    ..root
                }],
                animate: false,
                actions: Vec::new(),
                images: Vec::new(),
            },
        }
    }
}

impl FrameBuilder {
    /// The root node.
    pub fn root(&self) -> NodeRef {
        NodeRef(0)
    }

    /// Adds `node` under `parent`; returns the builder and the new node's index.
    ///
    /// `parent` must be a row, a column or an overlay of this builder (checked in debug
    /// builds). Root first and parents earlier hold by construction.
    pub fn child(mut self, parent: NodeRef, node: Node) -> (FrameBuilder, NodeRef) {
        debug_assert!(
            self.frame
                .nodes
                .get(parent.0 as usize)
                .is_some_and(|p| matches!(
                    p.kind,
                    NodeKind::Row(_) | NodeKind::Column(_) | NodeKind::Overlay(_)
                )),
            "parent is not a container node of this frame"
        );
        let at = NodeRef(self.frame.nodes.len() as u32);
        self.frame.nodes.push(Node {
            parent: Some(parent.0),
            ..node
        });
        (self, at)
    }

    /// Render again on the next frame (an animation).
    pub fn animate(mut self, animate: bool) -> FrameBuilder {
        self.frame.animate = animate;
        self
    }

    /// Adds an action of a Home tile (a search tile takes its answer's); the first is the primary one.
    pub fn action(
        mut self,
        id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<FrameBuilder, InvalidId> {
        self.frame.actions.push(Action::new(id, title)?);
        Ok(self)
    }

    /// Sends `image` with the frame, for image nodes and canvas commands to name.
    pub fn image(mut self, image: WidgetImage) -> FrameBuilder {
        self.frame.images.push(image);
        self
    }

    /// The finished frame. Debug builds panic on a frame the launcher would reject for
    /// these rules: a button needs an id, a focusable canvas needs an id, node ids are
    /// unique, nodes nest at most [`MAX_NODE_DEPTH`] deep, and clips and transforms
    /// balance in each canvas. Release builds do not check.
    pub fn build(self) -> WidgetFrame {
        #[cfg(debug_assertions)]
        debug_check(&self.frame);
        self.frame
    }
}

/// The rules of the launcher's frame check that a builder makes easy to break.
#[cfg(debug_assertions)]
fn debug_check(frame: &WidgetFrame) {
    let mut ids = std::collections::HashSet::new();
    // The ancestors of every node so far; a parent is always an earlier node.
    let mut depths: Vec<usize> = Vec::with_capacity(frame.nodes.len());
    for (index, node) in frame.nodes.iter().enumerate() {
        let depth = node.parent.map_or(0, |p| depths[p as usize] + 1);
        assert!(
            depth <= MAX_NODE_DEPTH,
            "node {index} nests deeper than {MAX_NODE_DEPTH}"
        );
        depths.push(depth);
        if !node.id.is_empty() {
            assert!(
                ids.insert(node.id.as_str()),
                "node id {:?} is used twice",
                node.id
            );
        }
        match &node.kind {
            NodeKind::Button(_) => assert!(!node.id.is_empty(), "button node {index} needs an id"),
            NodeKind::Canvas(c) => {
                assert!(
                    !c.focusable || !node.id.is_empty(),
                    "focusable canvas node {index} needs an id"
                );
                let mut open = Vec::new();
                for draw in &c.commands {
                    let ok = match draw {
                        Draw::PushClip(_) => {
                            open.push(true);
                            true
                        }
                        Draw::PushTransform(_) => {
                            open.push(false);
                            true
                        }
                        Draw::PopClip => open.pop() == Some(true),
                        Draw::PopTransform => open.pop() == Some(false),
                        _ => true,
                    };
                    assert!(ok, "unbalanced clips and transforms");
                }
                assert!(open.is_empty(), "unbalanced clips and transforms");
            }
            _ => {}
        }
    }
}

impl WidgetImage {
    fn new(id: impl Into<String>, format: ImageFormat, bytes: Vec<u8>) -> WidgetImage {
        WidgetImage {
            id: id.into(),
            format,
            bytes,
        }
    }

    /// A PNG image, named `id` by image nodes and commands.
    pub fn png(id: impl Into<String>, bytes: impl Into<Vec<u8>>) -> WidgetImage {
        WidgetImage::new(id, ImageFormat::Png, bytes.into())
    }

    /// A JPEG image.
    pub fn jpeg(id: impl Into<String>, bytes: impl Into<Vec<u8>>) -> WidgetImage {
        WidgetImage::new(id, ImageFormat::Jpeg, bytes.into())
    }

    /// An SVG document.
    pub fn svg(id: impl Into<String>, text: impl Into<String>) -> WidgetImage {
        WidgetImage::new(id, ImageFormat::Svg, text.into().into_bytes())
    }
}

/// The widget effects (they need no permission).
impl Effect {
    /// Renders every tile of the widget `id` again, whatever its params, as soon as the rate
    /// cap allows.
    pub fn redraw_widget(id: impl Into<String>) -> Effect {
        Effect::RedrawWidget(id.into())
    }

    /// Stores `image` for frames to name, replacing one with the same id.
    pub fn set_widget_image(image: WidgetImage) -> Effect {
        Effect::SetWidgetImage(image)
    }
}

impl Layout {
    /// Sets the padding on all four sides.
    pub fn padding(mut self, padding: f32) -> Layout {
        self.padding = Edges {
            top: padding,
            right: padding,
            bottom: padding,
            left: padding,
        };
        self
    }

    /// Sets the padding per side.
    pub fn padding_edges(mut self, padding: Edges) -> Layout {
        self.padding = padding;
        self
    }

    /// Sets the gap between children.
    pub fn gap(mut self, gap: f32) -> Layout {
        self.gap = gap;
        self
    }

    /// Sets the alignment across the main axis.
    pub fn align(mut self, align: Align) -> Layout {
        self.align = align;
        self
    }

    /// Sets how children share the main axis.
    pub fn justify(mut self, justify: Justify) -> Layout {
        self.justify = justify;
        self
    }
}

/// Zero padding, zero gap, `Align::Start`, `Justify::Start`.
impl Default for Layout {
    fn default() -> Layout {
        Layout {
            padding: Edges {
                top: 0.,
                right: 0.,
                bottom: 0.,
                left: 0.,
            },
            gap: 0.,
            align: Align::Start,
            justify: Justify::Start,
        }
    }
}

impl Color {
    /// An opaque colour from `0xRRGGBB`.
    pub fn rgb(rgb: u32) -> Color {
        Color::Rgba(rgb << 8 | 0xff)
    }

    /// A colour from `0xRRGGBBAA`.
    pub fn rgba(rgba: u32) -> Color {
        Color::Rgba(rgba)
    }

    /// A colour of the launcher's theme.
    pub fn token(token: ColorToken) -> Color {
        Color::Token(token)
    }
}

impl Paint {
    /// One colour.
    pub fn solid(color: Color) -> Paint {
        Paint::Solid(color)
    }

    /// A linear gradient at `angle` degrees, from `start` (at 0) to `end` (at 1).
    pub fn linear(angle: f32, start: Color, end: Color) -> Paint {
        Paint::Linear(LinearGradient {
            angle,
            start: ColorStop {
                color: start,
                at: 0.,
            },
            end: ColorStop { color: end, at: 1. },
        })
    }
}

impl Transform {
    /// No change.
    pub fn identity() -> Transform {
        Transform {
            translate: Point { x: 0., y: 0. },
            scale: Point { x: 1., y: 1. },
            rotate: 0.,
        }
    }

    /// A move by `x`, `y`.
    pub fn translate(x: f32, y: f32) -> Transform {
        Transform {
            translate: Point { x, y },
            ..Transform::identity()
        }
    }

    /// Scales by `sx`, `sy`.
    pub fn scale(mut self, sx: f32, sy: f32) -> Transform {
        self.scale = Point { x: sx, y: sy };
        self
    }

    /// Rotates by `deg` degrees.
    pub fn rotate(mut self, deg: f32) -> Transform {
        self.rotate = deg;
        self
    }
}

impl Node {
    fn new(kind: NodeKind) -> Node {
        Node {
            id: String::new(),
            parent: None,
            grow: 0.,
            width: None,
            height: None,
            kind,
        }
    }

    /// A row of children.
    pub fn row(layout: Layout) -> Node {
        Node::new(NodeKind::Row(layout))
    }

    /// A column of children.
    pub fn column(layout: Layout) -> Node {
        Node::new(NodeKind::Column(layout))
    }

    /// Children stacked on top of each other. The overlay takes its size from its first
    /// child; later children are stacked on it at the same origin, inset by the padding.
    pub fn overlay(layout: Layout) -> Node {
        Node::new(NodeKind::Overlay(layout))
    }

    /// Text in the theme's text colour, one line, start-aligned.
    pub fn text(text: impl Into<String>, style: TextStyle) -> Node {
        Node::new(NodeKind::Text(TextNode {
            text: text.into(),
            style,
            color: Color::Token(ColorToken::Text),
            align: TextAlign::Start,
            max_lines: 1,
            mono: false,
        }))
    }

    /// An icon of `size` logical pixels.
    pub fn icon(icon: Icon, size: f32) -> Node {
        Node::new(NodeKind::Icon(IconNode { icon, size }))
    }

    /// The image `id` of the frame (or of [`Effect::set_widget_image`]).
    pub fn image(id: impl Into<String>, fit: ImageFit) -> Node {
        Node::new(NodeKind::Image(ImageNode {
            image: id.into(),
            fit,
            radius: 0.,
        }))
    }

    /// A bar filled to `value`, from 0 to 1.
    pub fn progress(value: f32) -> Node {
        Node::new(NodeKind::Progress(value))
    }

    /// Free space; give it a [`grow`](Node::grow).
    pub fn spacer() -> Node {
        Node::new(NodeKind::Spacer)
    }

    /// A button with node id `id` (it must not be empty) that runs the action `action_id`; add a
    /// [`label`](Node::label).
    pub fn button(id: impl Into<String>, action_id: impl Into<String>) -> Node {
        Node {
            id: id.into(),
            ..Node::new(NodeKind::Button(ButtonNode {
                label: None,
                icon: None,
                action_id: action_id.into(),
            }))
        }
    }

    /// A canvas with node id `id` that draws `commands`. Every `push_clip` and
    /// `push_transform` needs its pop, in order, within the canvas; a focusable canvas needs
    /// a non-empty id.
    pub fn canvas(id: impl Into<String>, commands: Vec<Draw>) -> Node {
        Node {
            id: id.into(),
            ..Node::new(NodeKind::Canvas(CanvasNode {
                focusable: false,
                commands,
            }))
        }
    }

    /// The node id, which events name. Unique in the frame (checked by
    /// [`FrameBuilder::build`] in debug builds); empty by default.
    pub fn id(mut self, id: impl Into<String>) -> Node {
        self.id = id.into();
        self
    }

    /// Takes this share of the free space of the parent's main axis.
    pub fn grow(mut self, grow: f32) -> Node {
        self.grow = grow;
        self
    }

    /// Fixes the width and the height, in logical pixels.
    pub fn size(self, width: f32, height: f32) -> Node {
        self.width(width).height(height)
    }

    /// Fixes the width.
    pub fn width(mut self, width: f32) -> Node {
        self.width = Some(width);
        self
    }

    /// Fixes the height.
    pub fn height(mut self, height: f32) -> Node {
        self.height = Some(height);
        self
    }

    /// The colour of a text node.
    pub fn color(mut self, color: Color) -> Node {
        if let NodeKind::Text(t) = &mut self.kind {
            t.color = color;
        } else {
            debug_assert!(false, "color is for text nodes");
        }
        self
    }

    /// The alignment of a text node.
    pub fn align(mut self, align: TextAlign) -> Node {
        if let NodeKind::Text(t) = &mut self.kind {
            t.align = align;
        } else {
            debug_assert!(false, "align is for text nodes");
        }
        self
    }

    /// The most lines of a text node (at least 1).
    pub fn max_lines(mut self, lines: u8) -> Node {
        if let NodeKind::Text(t) = &mut self.kind {
            t.max_lines = lines.max(1);
        } else {
            debug_assert!(false, "max_lines is for text nodes");
        }
        self
    }

    /// A text node in the monospace font.
    pub fn mono(mut self) -> Node {
        if let NodeKind::Text(t) = &mut self.kind {
            t.mono = true;
        } else {
            debug_assert!(false, "mono is for text nodes");
        }
        self
    }

    /// The label of a button node.
    pub fn label(mut self, text: impl Into<String>) -> Node {
        if let NodeKind::Button(b) = &mut self.kind {
            b.label = Some(text.into());
        } else {
            debug_assert!(false, "label is for button nodes");
        }
        self
    }

    /// A canvas that takes keys; it needs an id.
    pub fn focusable(mut self) -> Node {
        if let NodeKind::Canvas(c) = &mut self.kind {
            c.focusable = true;
        } else {
            debug_assert!(false, "focusable is for canvas nodes");
        }
        self
    }
}

impl ShapeStyle {
    fn new() -> ShapeStyle {
        ShapeStyle {
            paint: Paint::Solid(Color::Token(ColorToken::Text)),
            stroke: None,
            opacity: 1.,
        }
    }
}

impl Draw {
    /// A rectangle, filled with the `text` colour until [`fill`](Draw::fill) or
    /// [`stroke`](Draw::stroke).
    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Draw {
        Draw::Rect(RectCmd {
            x,
            y,
            width,
            height,
            radius: 0.,
            style: ShapeStyle::new(),
        })
    }

    /// An ellipse around `cx`, `cy`.
    pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Draw {
        Draw::Ellipse(EllipseCmd {
            cx,
            cy,
            rx,
            ry,
            style: ShapeStyle::new(),
        })
    }

    /// A circle around `cx`, `cy`.
    pub fn circle(cx: f32, cy: f32, r: f32) -> Draw {
        Draw::ellipse(cx, cy, r, r)
    }

    /// A path of `segments`.
    pub fn path(segments: Vec<PathSegment>) -> Draw {
        Draw::Path(PathCmd {
            segments,
            style: ShapeStyle::new(),
        })
    }

    /// Text with its top left at `x`, `y`, `size` logical pixels high.
    pub fn text(x: f32, y: f32, text: impl Into<String>, size: f32) -> Draw {
        Draw::Text(TextCmd {
            x,
            y,
            text: text.into(),
            size,
            color: Color::Token(ColorToken::Text),
            font: None,
            mono: false,
            opacity: 1.,
        })
    }

    fn image_cmd(image: impl Into<String>, x: f32, y: f32, width: f32, height: f32) -> ImageCmd {
        ImageCmd {
            image: image.into(),
            x,
            y,
            width,
            height,
            opacity: 1.,
        }
    }

    /// The image `id` stretched over the rectangle.
    pub fn image(id: impl Into<String>, x: f32, y: f32, width: f32, height: f32) -> Draw {
        Draw::Image(Draw::image_cmd(id, x, y, width, height))
    }

    /// The SVG image `id` rasterized to the rectangle.
    pub fn svg(id: impl Into<String>, x: f32, y: f32, width: f32, height: f32) -> Draw {
        Draw::Svg(Draw::image_cmd(id, x, y, width, height))
    }

    /// Clips what follows to the rectangle, until [`pop_clip`](Draw::pop_clip).
    pub fn push_clip(x: f32, y: f32, width: f32, height: f32) -> Draw {
        Draw::PushClip(ClipRect {
            x,
            y,
            width,
            height,
        })
    }

    /// Ends the last clip.
    pub fn pop_clip() -> Draw {
        Draw::PopClip
    }

    /// Transforms what follows, until [`pop_transform`](Draw::pop_transform).
    pub fn push_transform(transform: Transform) -> Draw {
        Draw::PushTransform(transform)
    }

    /// Ends the last transform.
    pub fn pop_transform() -> Draw {
        Draw::PopTransform
    }

    fn style_mut(&mut self) -> Option<&mut ShapeStyle> {
        match self {
            Draw::Rect(c) => Some(&mut c.style),
            Draw::Ellipse(c) => Some(&mut c.style),
            Draw::Path(c) => Some(&mut c.style),
            _ => None,
        }
    }

    /// Fills a rectangle, ellipse or path with `paint`. A shape has one paint and either a
    /// fill or an outline (the contract's `shape-style`), so this replaces an earlier
    /// [`stroke`](Draw::stroke), and the other way round.
    pub fn fill(mut self, paint: Paint) -> Draw {
        if let Some(style) = self.style_mut() {
            style.paint = paint;
            style.stroke = None;
        } else {
            debug_assert!(false, "fill is for rect, ellipse and path commands");
        }
        self
    }

    /// Outlines a rectangle, ellipse or path with a line `width` wide, in `paint`.
    pub fn stroke(mut self, width: f32, paint: Paint) -> Draw {
        if let Some(style) = self.style_mut() {
            style.paint = paint;
            style.stroke = Some(width);
        } else {
            debug_assert!(false, "stroke is for rect, ellipse and path commands");
        }
        self
    }

    /// The opacity, 0 to 1, of a shape, text or image.
    pub fn opacity(mut self, opacity: f32) -> Draw {
        match &mut self {
            Draw::Text(c) => c.opacity = opacity,
            Draw::Image(c) | Draw::Svg(c) => c.opacity = opacity,
            _ => {
                if let Some(style) = self.style_mut() {
                    style.opacity = opacity;
                } else {
                    debug_assert!(false, "opacity is for shapes, text and images");
                }
            }
        }
        self
    }

    /// The corner radius of a rectangle.
    pub fn radius(mut self, radius: f32) -> Draw {
        if let Draw::Rect(c) = &mut self {
            c.radius = radius;
        } else {
            debug_assert!(false, "radius is for rect commands");
        }
        self
    }

    /// The colour of a text command.
    pub fn color(mut self, color: Color) -> Draw {
        if let Draw::Text(c) = &mut self {
            c.color = color;
        } else {
            debug_assert!(false, "color is for text commands");
        }
        self
    }

    /// The font family of a text command.
    pub fn font(mut self, family: impl Into<String>) -> Draw {
        if let Draw::Text(c) = &mut self {
            c.font = Some(family.into());
        } else {
            debug_assert!(false, "font is for text commands");
        }
        self
    }

    /// A text command in the monospace font.
    pub fn mono(mut self) -> Draw {
        if let Draw::Text(c) = &mut self {
            c.mono = true;
        } else {
            debug_assert!(false, "mono is for text commands");
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Accessory;

    #[test]
    fn frame_builder_assigns_parents_in_order() {
        let frame = WidgetFrame::new(Node::row(Layout::default()));
        let root = frame.root();
        let (frame, _) = frame.child(root, Node::text("a", TextStyle::Body));
        let (frame, _) = frame.child(root, Node::text("b", TextStyle::Body));
        let (frame, column) = frame.child(root, Node::column(Layout::default()));
        let (frame, _) = frame.child(column, Node::canvas("c", Vec::new()));
        let frame = frame.build();
        let parents: Vec<_> = frame.nodes.iter().map(|n| n.parent).collect();
        assert_eq!(parents, [None, Some(0), Some(0), Some(0), Some(3)]);
    }

    #[test]
    fn nodes_and_commands_have_documented_defaults() {
        let node = Node::text("a", TextStyle::Body);
        assert_eq!((node.grow, node.width, node.height), (0., None, None));
        let NodeKind::Text(t) = node.kind else {
            panic!("not text")
        };
        assert!(matches!(t.color, Color::Token(ColorToken::Text)));
        assert_eq!(t.max_lines, 1);
        let Draw::Rect(r) = Draw::rect(0., 0., 4., 4.) else {
            panic!("not a rect")
        };
        assert_eq!((r.style.opacity, r.radius, r.style.stroke), (1., 0., None));
        assert!(matches!(
            r.style.paint,
            Paint::Solid(Color::Token(ColorToken::Text))
        ));
    }

    #[test]
    fn modifiers_set_their_fields() {
        let node = Node::button("b", "go").label("Go").size(10., 20.).grow(1.);
        assert_eq!(
            (node.width, node.height, node.grow),
            (Some(10.), Some(20.), 1.)
        );
        let NodeKind::Button(b) = node.kind else {
            panic!("not a button")
        };
        assert_eq!(b.label.as_deref(), Some("Go"));
        let Draw::Rect(r) = Draw::rect(0., 0., 1., 1.)
            .stroke(2., Paint::solid(Color::rgb(0)))
            .radius(3.)
            .opacity(0.5)
        else {
            panic!("not a rect")
        };
        assert_eq!(
            (r.style.stroke, r.radius, r.style.opacity),
            (Some(2.), 3., 0.5)
        );
    }

    fn typical() -> FrameBuilder {
        let frame = WidgetFrame::new(Node::column(Layout::default()));
        let root = frame.root();
        let (frame, _) = frame.child(root, Node::button("b", "go").label("Go"));
        let canvas = Node::canvas(
            "c",
            vec![
                Draw::push_clip(0., 0., 4., 4.),
                Draw::push_transform(Transform::translate(1., 1.)),
                Draw::pop_transform(),
                Draw::pop_clip(),
            ],
        )
        .focusable();
        frame.child(root, canvas).0
    }

    #[test]
    fn a_typical_frame_passes_the_debug_check() {
        assert_eq!(typical().build().nodes.len(), 3);
    }

    fn with_child(node: Node) -> WidgetFrame {
        let frame = WidgetFrame::new(Node::row(Layout::default()));
        let root = frame.root();
        frame.child(root, node).0.build()
    }

    #[test]
    #[should_panic(expected = "button node 1 needs an id")]
    fn a_button_without_an_id_panics_in_debug() {
        with_child(Node::button("", "go"));
    }

    #[test]
    #[should_panic(expected = "focusable canvas node 1 needs an id")]
    fn a_focusable_canvas_without_an_id_panics_in_debug() {
        with_child(Node::canvas("", Vec::new()).focusable());
    }

    #[test]
    #[should_panic(expected = "is used twice")]
    fn duplicate_node_ids_panic_in_debug() {
        let frame = WidgetFrame::new(Node::row(Layout::default()).id("a"));
        let root = frame.root();
        frame.child(root, Node::spacer().id("a")).0.build();
    }

    #[test]
    #[should_panic(expected = "unbalanced clips and transforms")]
    fn an_unclosed_clip_panics_in_debug() {
        with_child(Node::canvas("c", vec![Draw::push_clip(0., 0., 1., 1.)]));
    }

    #[test]
    #[should_panic(expected = "unbalanced clips and transforms")]
    fn a_crossed_pop_panics_in_debug() {
        with_child(Node::canvas(
            "c",
            vec![Draw::push_clip(0., 0., 1., 1.), Draw::pop_transform()],
        ));
    }

    /// A frame whose root holds `depth` nested columns.
    fn nested(depth: usize) -> WidgetFrame {
        let mut frame = WidgetFrame::new(Node::column(Layout::default()));
        let mut parent = frame.root();
        for _ in 0..depth {
            (frame, parent) = frame.child(parent, Node::column(Layout::default()));
        }
        frame.build()
    }

    #[test]
    fn nodes_nest_32_deep() {
        assert_eq!(nested(32).nodes.len(), 33);
    }

    #[test]
    #[should_panic(expected = "node 33 nests deeper than 32")]
    fn nodes_nested_33_deep_panic_in_debug() {
        nested(33);
    }

    #[test]
    #[should_panic(expected = "not a container")]
    fn a_child_of_a_leaf_panics_in_debug() {
        let frame = WidgetFrame::new(Node::row(Layout::default()));
        let root = frame.root();
        let (frame, leaf) = frame.child(root, Node::spacer());
        frame.child(leaf, Node::spacer());
    }

    #[test]
    #[should_panic(expected = "color is for text nodes")]
    fn a_wrong_kind_modifier_panics_in_debug() {
        let _ = Node::row(Layout::default()).color(Color::rgb(0));
    }

    #[test]
    fn max_lines_is_at_least_one() {
        let NodeKind::Text(t) = Node::text("a", TextStyle::Body).max_lines(0).kind else {
            panic!("not text")
        };
        assert_eq!(t.max_lines, 1);
    }

    #[test]
    fn colors_pack_as_rrggbbaa() {
        assert!(matches!(Color::rgb(0x112233), Color::Rgba(0x1122_33ff)));
        assert!(matches!(Color::rgba(0x11223344), Color::Rgba(0x1122_3344)));
    }

    #[test]
    fn paint_linear_has_stops_at_the_ends() {
        let Paint::Linear(g) = Paint::linear(90., Color::rgb(1), Color::rgb(2)) else {
            panic!("not linear")
        };
        assert_eq!((g.angle, g.start.at, g.end.at), (90., 0., 1.));
        assert!(matches!(g.end.color, Color::Rgba(0x0000_02ff)));
    }

    #[test]
    fn widget_ids_follow_the_id_rule() {
        assert!(WidgetAnswer::new("a\u{200b}").is_err());
        assert!(WidgetFrame::new(Node::spacer()).action("a\n", "A").is_err());
        assert!(
            WidgetAnswer::new("clock")
                .unwrap()
                .action("go", "Go")
                .is_ok()
        );
    }

    #[test]
    fn widget_answer_sets_params_and_size() {
        let answer = WidgetAnswer::new("now").unwrap();
        assert!(answer.params.is_empty());
        assert!(answer.size.is_none());
        let answer = answer
            .param("city", "Oslo")
            .param("units", "metric")
            .param("city", "Bergen")
            .size(WidgetSize::Quarter)
            .action("copy", "Copy")
            .unwrap();
        let pairs: Vec<(&str, &str)> = answer
            .params
            .iter()
            .map(|p| (p.key.as_str(), p.value.as_str()))
            .collect();
        // A key set again keeps its place and takes the new value.
        assert_eq!(pairs, [("city", "Bergen"), ("units", "metric")]);
        assert!(matches!(answer.size, Some(WidgetSize::Quarter)));
        assert_eq!(answer.actions[0].id, "copy");
    }

    #[test]
    fn param_finds_a_value_by_key() {
        let params = WidgetAnswer::new("now")
            .unwrap()
            .param("city", "Oslo")
            .param("empty", "")
            .params;
        assert_eq!(param(&params, "city"), Some("Oslo"));
        assert_eq!(param(&params, "empty"), Some(""));
        assert_eq!(param(&params, "units"), None);
        assert_eq!(param(&[], "city"), None);
    }

    #[test]
    fn item_builder_sets_every_field() {
        let item = Item::new("demo/one", "One")
            .unwrap()
            .subtitle("first")
            .keywords(["a", "b"])
            .icon(Icon::Emoji("1️⃣".into()))
            .action("open", "Open")
            .unwrap()
            .with_action(
                Action::new("copy", "Copy")
                    .unwrap()
                    .shortcut("ctrl+shift+c"),
            );
        assert_eq!(item.id, "demo/one");
        assert_eq!(item.title, "One");
        assert_eq!(item.subtitle.as_deref(), Some("first"));
        assert_eq!(item.keywords, ["a", "b"]);
        assert_eq!(item.actions.len(), 2);
        assert_eq!(item.actions[0].id, "open");
        assert_eq!(item.actions[1].shortcut.as_deref(), Some("ctrl+shift+c"));
    }

    #[test]
    fn item_id_needs_a_plugin_prefix() {
        let bad = [
            "no-prefix",
            "/local",
            "plugin/",
            "",
            "Demo/x",
            "my_plugin/x",
            "dé/x",
            "a.b/x",
            &format!("{}/x", "p".repeat(33)),
        ];
        for bad in bad {
            let err = Item::new(bad, "x").unwrap_err();
            assert_eq!(err.what(), "item");
            assert!(err.to_string().contains("<plugin-id>/<local-id>"), "{err}");
        }
        assert!(Item::new("demo/a/b", "nested").is_ok());
        assert!(Item::new("my-plugin-2/x", "dashes and digits").is_ok());
        assert!(Item::new(format!("{}/x", "p".repeat(32)), "32 characters").is_ok());
    }

    #[test]
    fn ids_follow_the_id_rule() {
        let cases = [
            ("r0\nr1", "control character"),
            ("bell\u{7}", "control character"),
            ("rtl\u{202e}", "formatting character"),
            ("zero\u{200b}width", "formatting character"),
        ];
        for (id, problem) in cases {
            let err = ListRow::new(id, "x").unwrap_err();
            assert!(err.to_string().contains(problem), "{id:?}: {err}");
            assert_eq!(err.id(), id);
        }
        let long = "x".repeat(MAX_ID_BYTES + 1);
        let err = Action::new(long, "x").unwrap_err();
        assert!(err.to_string().contains("256 bytes"), "{err}");
        assert!(Action::new("x".repeat(MAX_ID_BYTES), "x").is_ok());
        // Display text may hold what ids may not.
        assert!(Action::new("ok", "Title with \u{202e} and emoji 👨‍💻").is_ok());
    }

    #[test]
    fn every_id_is_checked() {
        let bad = "x\n";
        assert_eq!(View::list(bad, "t").unwrap_err().what(), "view");
        assert_eq!(View::grid(bad, "t").unwrap_err().what(), "view");
        assert_eq!(View::detail(bad, "t").unwrap_err().what(), "view");
        assert_eq!(View::form(bad, "t", "Go").unwrap_err().what(), "view");
        assert_eq!(View::scratchpad(bad, "t", "").unwrap_err().what(), "view");
        assert_eq!(GridTile::new(bad, "t").unwrap_err().what(), "tile");
        assert_eq!(
            FormField::checkbox(bad, "t", true).unwrap_err().what(),
            "field"
        );
        assert_eq!(
            DropdownOption::new(bad, "t").unwrap_err().what(),
            "dropdown option"
        );
        assert_eq!(
            FormField::dropdown("f", "t", Vec::new(), bad)
                .unwrap_err()
                .what(),
            "dropdown option"
        );
        let row = ListRow::new("r", "t").unwrap();
        assert_eq!(row.action(bad, "t").unwrap_err().what(), "action");
    }

    #[test]
    fn invalid_id_message_is_escaped_and_short() {
        let err = ListRow::new(format!("{}\u{1b}[31m", "y".repeat(300)), "t").unwrap_err();
        let message = String::from(err);
        assert!(message.starts_with("invalid row id \"yyy"), "{message}");
        assert!(!message.contains('\u{1b}'), "escaped: {message}");
        assert!(message.len() < 200, "{message}");
    }

    #[test]
    fn question_mark_turns_an_invalid_id_into_the_export_error() {
        fn search() -> Result<Vec<Item>, String> {
            Ok(vec![Item::new("bare", "x")?.action("open", "Open")?])
        }
        let err = search().unwrap_err();
        assert!(err.starts_with("invalid item id \"bare\""), "{err}");
    }

    #[test]
    fn views_convert_into_effects() {
        let list = View::list("l", "List")
            .unwrap()
            .placeholder("Find")
            .search(SearchMode::Module)
            .revision(3)
            .row(
                ListRow::new("r", "Row")
                    .unwrap()
                    .section("S")
                    .accessory(Accessory::Tag("t".into()))
                    .detail("# d"),
            )
            .show_detail(true);
        let Effect::PushView(View::ListView(list)) = Effect::push_view(list) else {
            panic!("a pushed list");
        };
        assert_eq!(list.revision, 3);
        assert_eq!(list.rows[0].section.as_deref(), Some("S"));

        let pad = View::scratchpad("pad", "Pad", "1+1")
            .unwrap()
            .revision(7)
            .annotation(LineAnnotation::note(0, "2"))
            .annotation(LineAnnotation::error(1, "division by zero"));
        let Effect::UpdateView(View::ScratchpadView(pad)) = Effect::update_view(pad) else {
            panic!("an updated scratchpad");
        };
        assert!(!pad.annotations[0].is_error && pad.annotations[1].is_error);
        assert_eq!(
            LineAnnotation::note(0, "x").swatch("#ff8800").swatch,
            Some("#ff8800".into())
        );
        assert_eq!(pad.annotations[0].swatch, None);

        let form = View::form("f", "Form", "Save")
            .unwrap()
            .field(
                FormField::text("name", "Name", "")
                    .unwrap()
                    .placeholder("You"),
            )
            .field(
                FormField::date("when", "When", None)
                    .unwrap()
                    .error("Pick a day"),
            );
        let FieldKind::Text(text) = &form.fields[0].kind else {
            panic!("a text field");
        };
        assert_eq!(text.placeholder, "You");
        assert_eq!(form.fields[1].error.as_deref(), Some("Pick a day"));

        let toast = Toast::error("Failed").message("why");
        assert!(matches!(toast.kind, ToastKind::Error));
        let request = HttpRequest::get("https://example.org")
            .header("accept", "text/plain")
            .body("x");
        assert_eq!(request.method, "GET");
        assert_eq!(request.body.as_deref(), Some(&b"x"[..]));
    }
}
