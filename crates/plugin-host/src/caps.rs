//! Size caps, validity rules, permissions and rates for what a plugin returns, checked before
//! the core or the UI sees it (`plugin.wit`, "Size caps and validity" and "Permissions").
//!
//! Two outcomes:
//! - a cap or validity violation rejects the whole answer; the only thing dropped instead is
//!   an item whose id belongs to another provider;
//! - an effect the plugin may not send (a missing permission, a rate exceeded) is dropped on
//!   its own and reported in [`Accepted::denied`]; the rest of the answer is kept.
//!
//! The 4 MiB bound on one call's returned value is enforced earlier, while wasmtime lifts it
//! out of the guest (`Store::set_hostcall_fuel` in [`crate::plugin`]).

use std::collections::VecDeque;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use tomakast_core::view::{FieldKind, View};
use tomakast_core::widget::{MAX_WIDGETS, WidgetDecl};
use tomakast_core::{Accessory, Action, Effect, Icon, Item, is_valid_id};

use crate::manifest::Permissions;
use crate::widget_caps::{ImageBudget, check_image, is_declared};

const KIB: usize = 1024;

pub const MAX_LIST_ROWS: usize = 2000;
pub const MAX_GRID_TILES: usize = 500;
pub const MAX_FORM_FIELDS: usize = 50;
pub const MAX_MARKDOWN_BYTES: usize = 64 * KIB;
pub const MAX_SCRATCHPAD_BYTES: usize = 256 * KIB;
/// Any title, subtitle, label, caption, placeholder, section, error or accessory.
pub const MAX_LABEL_BYTES: usize = KIB;
pub const MAX_ACTIONS: usize = 16;
pub const MAX_KEYWORDS: usize = 32;
pub const MAX_ACCESSORIES: usize = 8;
pub const MAX_METADATA: usize = 64;
pub const MAX_DROPDOWN_OPTIONS: usize = 200;
pub const MAX_ANNOTATIONS: usize = 10_000;
pub const MAX_TEXT_FIELD_BYTES: usize = 64 * KIB;
pub const MAX_MULTILINE_BYTES: usize = 256 * KIB;
pub const MAX_TOAST_MESSAGE_BYTES: usize = 4 * KIB;
pub const MAX_METADATA_VALUE_BYTES: usize = 4 * KIB;
/// A keyword, like a label.
pub const MAX_KEYWORD_BYTES: usize = KIB;
pub const MAX_ANNOTATION_BYTES: usize = KIB;
/// Annotation lines are below this.
pub const MAX_ANNOTATION_LINES: u32 = 10_000;
pub const MAX_LIST_QUERY_BYTES: usize = 64 * KIB;
pub const MAX_GRID_COLUMNS: u8 = 12;
/// Items one `replace-items` may hold (the commands of `plugin.toml` are not counted).
pub const MAX_INDEX_ITEMS: usize = 1000;
/// Items one `search` answer may hold.
pub const MAX_SEARCH_ITEMS: usize = 50;
/// The value one call returns, in bytes.
pub const MAX_RETURN_BYTES: usize = 4 * KIB * KIB;
/// Toasts a plugin may show per [`TOAST_WINDOW`].
pub const MAX_TOASTS: usize = 10;
pub const TOAST_WINDOW: Duration = Duration::from_secs(10);
/// Unsolicited views a plugin may push or update, counted together, per [`UPDATE_WINDOW`].
pub const MAX_UNSOLICITED_UPDATES: usize = 30;
pub const UPDATE_WINDOW: Duration = Duration::from_secs(1);
/// `push-view`s one answer may hold; later ones are dropped.
pub const MAX_PUSHES_PER_ANSWER: usize = 1;
/// Unsolicited `replace-items` a plugin may send per [`REPLACE_WINDOW`] (`init`'s counts).
pub const MAX_UNSOLICITED_REPLACES: usize = 1;
pub const REPLACE_WINDOW: Duration = Duration::from_secs(5);
/// Unsolicited `show-widgets` and `set-widget-image` a plugin may send, counted together, per
/// [`WIDGET_WINDOW`].
pub const MAX_UNSOLICITED_WIDGET_EFFECTS: usize = 30;
pub const WIDGET_WINDOW: Duration = Duration::from_secs(1);

/// Why an answer was rejected; shown to the user in an error toast.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CapError(pub String);

pub(crate) type Checked = Result<(), CapError>;

pub(crate) fn fail<T>(message: String) -> Result<T, CapError> {
    Err(CapError(message))
}

pub(crate) fn count(what: &str, n: usize, max: usize) -> Checked {
    if n > max {
        return fail(format!("{what} has {n} entries, at most {max}"));
    }
    Ok(())
}

pub(crate) fn bytes(what: &str, s: &str, max: usize) -> Checked {
    if s.len() > max {
        return fail(format!("{what} is {} bytes, at most {max}", s.len()));
    }
    Ok(())
}

/// A title, subtitle, label, caption, placeholder, section, error or accessory: at most
/// [`MAX_LABEL_BYTES`] and on one line, with no control characters (newlines and tabs
/// included). Format characters are allowed in display text.
pub(crate) fn label(what: &str, s: &str) -> Checked {
    bytes(what, s, MAX_LABEL_BYTES)?;
    if s.chars().any(char::is_control) {
        return fail(format!("{what} has control characters (such as a newline)"));
    }
    Ok(())
}

pub(crate) fn opt_label(what: &str, s: &Option<String>) -> Checked {
    s.as_deref().map_or(Ok(()), |s| label(what, s))
}

pub(crate) fn id(what: &str, s: &str) -> Checked {
    if !is_valid_id(s) {
        return fail(format!(
            "{what} id {s:?} is longer than 256 bytes or has control or format characters"
        ));
    }
    Ok(())
}

fn action(a: &Action) -> Checked {
    id("action", &a.id)?;
    label("action title", &a.title)?;
    if let Some(Err(e)) = a.hotkey() {
        return fail(format!("action {:?}: {e}", a.id));
    }
    Ok(())
}

pub(crate) fn actions(what: &str, list: &[Action]) -> Checked {
    if list.len() > MAX_ACTIONS {
        return fail(format!(
            "{what} has {} actions, at most {MAX_ACTIONS}",
            list.len()
        ));
    }
    list.iter().try_for_each(action)
}

/// A `plugin-file` path is relative and stays inside the plugin folder.
fn plugin_path_ok(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Empty, or exactly `YYYY-MM-DD` with a month 01–12 and a day 01–31. The digit checks come
/// before the `str` slices, so a multibyte character cannot split a slice.
fn date_ok(date: &str) -> bool {
    let b = date.as_bytes();
    if date.is_empty() {
        return true;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let number = |r: std::ops::Range<usize>| date[r].parse::<u8>().unwrap_or(0);
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && digits(0..4)
        && digits(5..7)
        && digits(8..10)
        && (1..=12).contains(&number(5..7))
        && (1..=31).contains(&number(8..10))
}

pub(crate) fn icon(icon: Option<&Icon>) -> Checked {
    match icon {
        None => Ok(()),
        Some(Icon::Named(name)) => id("icon name", name),
        Some(Icon::PluginFile(path)) if plugin_path_ok(path) => {
            id("icon file", &path.to_string_lossy())
        }
        // Only built-in modules give host paths; conversion never makes one from a plugin.
        Some(Icon::File(_)) => fail("a plugin may not name a host image file".into()),
        Some(Icon::PluginFile(path)) => fail(format!(
            "icon file {:?} must be a relative path inside the plugin folder",
            path.display()
        )),
        Some(i) if i.is_valid() => Ok(()),
        Some(Icon::Emoji(s)) => fail(format!("{s:?} is not a valid emoji icon")),
        Some(Icon::Color(s)) => fail(format!("{s:?} is not a #rrggbb colour")),
    }
}

fn item(item: &Item) -> Checked {
    // Conversion already required the `/`.
    id("item", item.id.as_str())?;
    label("item title", &item.title)?;
    opt_label("item subtitle", &item.subtitle)?;
    count("item keywords", item.keywords.len(), MAX_KEYWORDS)?;
    for keyword in &item.keywords {
        bytes("keyword", keyword, MAX_KEYWORD_BYTES)?;
    }
    icon(item.icon.as_ref())?;
    if item.actions.is_empty() {
        return fail(format!("item {:?} has no actions", item.id.as_str()));
    }
    actions("item", &item.actions)
}

/// Keeps `provider`'s items (warning about the others) and checks each.
fn own_items(provider: &str, items: Vec<Item>) -> Result<Vec<Item>, CapError> {
    let before = items.len();
    let items: Vec<Item> = items
        .into_iter()
        .filter(|i| i.id.provider() == provider)
        .collect();
    let dropped = before - items.len();
    if dropped > 0 {
        tracing::warn!(target: "plugin", plugin = provider, "dropped {dropped} items of another provider");
    }
    items.iter().try_for_each(item)?;
    Ok(items)
}

fn view(v: &View) -> Checked {
    id("view", &v.id().0)?;
    match v {
        View::List(list) => {
            label("list title", &list.title)?;
            label("list placeholder", &list.placeholder)?;
            bytes("list query", &list.query, MAX_LIST_QUERY_BYTES)?;
            if list.rows.len() > MAX_LIST_ROWS {
                return fail(format!(
                    "list view has {} rows, at most {MAX_LIST_ROWS}",
                    list.rows.len()
                ));
            }
            for row in &list.rows {
                id("row", &row.id)?;
                opt_label("row section", &row.section)?;
                label("row title", &row.title)?;
                opt_label("row subtitle", &row.subtitle)?;
                icon(row.icon.as_ref())?;
                count("row accessories", row.accessories.len(), MAX_ACCESSORIES)?;
                for accessory in &row.accessories {
                    let (Accessory::Text(s) | Accessory::Tag(s)) = accessory;
                    label("accessory", s)?;
                }
                if let Some(md) = &row.detail_markdown {
                    bytes("row detail markdown", md, MAX_MARKDOWN_BYTES)?;
                }
                actions("row", &row.actions)?;
            }
        }
        View::Grid(grid) => {
            label("grid title", &grid.title)?;
            if grid.columns > MAX_GRID_COLUMNS {
                return fail(format!(
                    "grid view has {} columns, at most {MAX_GRID_COLUMNS}",
                    grid.columns
                ));
            }
            if grid.tiles.len() > MAX_GRID_TILES {
                return fail(format!(
                    "grid view has {} tiles, at most {MAX_GRID_TILES}",
                    grid.tiles.len()
                ));
            }
            for tile in &grid.tiles {
                id("tile", &tile.id)?;
                icon(tile.image.as_ref())?;
                label("tile caption", &tile.caption)?;
                actions("tile", &tile.actions)?;
            }
        }
        View::Detail(detail) => {
            label("detail title", &detail.title)?;
            bytes("detail markdown", &detail.markdown, MAX_MARKDOWN_BYTES)?;
            count("detail metadata", detail.metadata.len(), MAX_METADATA)?;
            for (key, value) in &detail.metadata {
                label("metadata label", key)?;
                bytes("metadata value", value, MAX_METADATA_VALUE_BYTES)?;
            }
            actions("detail view", &detail.actions)?;
        }
        View::Form(form) => {
            label("form title", &form.title)?;
            label("form submit title", &form.submit_title)?;
            if form.fields.len() > MAX_FORM_FIELDS {
                return fail(format!(
                    "form view has {} fields, at most {MAX_FORM_FIELDS}",
                    form.fields.len()
                ));
            }
            for field in &form.fields {
                id("field", &field.id)?;
                label("field label", &field.label)?;
                opt_label("field error", &field.error)?;
                match &field.kind {
                    FieldKind::Text { value, placeholder } => {
                        bytes("text field value", value, MAX_TEXT_FIELD_BYTES)?;
                        label("text field placeholder", placeholder)?;
                    }
                    FieldKind::Multiline { value } => {
                        bytes("multiline field value", value, MAX_MULTILINE_BYTES)?;
                    }
                    FieldKind::Dropdown { options, value } => {
                        count("dropdown options", options.len(), MAX_DROPDOWN_OPTIONS)?;
                        id("dropdown value", value)?;
                        for (value, option) in options {
                            id("dropdown option value", value)?;
                            label("dropdown option label", option)?;
                        }
                    }
                    FieldKind::Date { value: Some(date) } if !date_ok(date) => {
                        return fail(format!(
                            "date {:?} is not empty or YYYY-MM-DD",
                            date.chars().take(32).collect::<String>()
                        ));
                    }
                    FieldKind::Checkbox { .. } | FieldKind::Date { .. } => {}
                }
            }
        }
        View::Scratchpad(pad) => {
            label("scratchpad title", &pad.title)?;
            actions("scratchpad", &pad.actions)?;
            bytes("scratchpad text", &pad.text, MAX_SCRATCHPAD_BYTES)?;
            count(
                "scratchpad annotations",
                pad.annotations.len(),
                MAX_ANNOTATIONS,
            )?;
            for annotation in &pad.annotations {
                bytes("annotation text", &annotation.text, MAX_ANNOTATION_BYTES)?;
                if annotation.line >= MAX_ANNOTATION_LINES {
                    return fail(format!(
                        "annotation line {} is not below {MAX_ANNOTATION_LINES}",
                        annotation.line
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Where an answer's effects come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The answer to a user action: `run`, `action-chosen` or `form-submitted` (Ruling AL).
    Answer,
    /// The answer to a view event that is not a user action: `query-changed`,
    /// `text-changed` or `closed`. It updates views like an answer, but what needs a user
    /// action (`copy-text`) is dropped.
    Reply,
    /// Unsolicited: from `init`, a timer, an HTTP answer or a preference change.
    Unsolicited,
    /// The answer to a widget event that is not a user action: `shown`, `hidden`, a pointer
    /// `move`, `enter` or `leave`, a `down`, or an `up` that completes no click. Unsolicited
    /// for the rates; what needs a user action is dropped: `copy-text`, and `push-view`,
    /// `pop-view` and `close-window`, which the launcher would apply as in answer to `run`
    /// (the core gives a press a request).
    WidgetEvent,
}

/// A sliding window: at most `max` events in any `per`.
#[derive(Debug, Default)]
pub(crate) struct Window(VecDeque<Instant>);

impl Window {
    /// Counts an event at `now` and returns true, unless `max` events are already in the
    /// window ending at `now`.
    pub(crate) fn admit(&mut self, now: Instant, max: usize, per: Duration) -> bool {
        while self
            .0
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) >= per)
        {
            self.0.pop_front();
        }
        if self.0.len() >= max {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

/// The rate state of one plugin, and the images it has stored.
#[derive(Debug, Default)]
pub struct Rates {
    toasts: Window,
    updates: Window,
    replaces: Window,
    /// Unsolicited `show-widgets` and `set-widget-image`, counted together.
    widgets: Window,
    /// The images of `set-widget-image` and of frames, as the core stores them.
    pub(crate) images: ImageBudget,
}

/// The effects that passed, and why the others were dropped (each reason once, in order).
#[derive(Debug, Default, PartialEq)]
pub struct Accepted {
    pub effects: Vec<Effect>,
    pub denied: Vec<String>,
}

impl Accepted {
    fn deny(&mut self, reason: &str) {
        if !self.denied.iter().any(|d| d == reason) {
            self.denied.push(reason.to_owned());
        }
    }
}

/// Checks `effects` from `provider`'s plugin, at `now`: first the view caps, the id rules,
/// shortcuts, emoji and colours over the whole list (items of another provider are dropped;
/// anything else that breaks a rule rejects the whole list), then each effect against `perms`
/// and `rates`:
/// - `copy-text` needs `clipboard` and a user action: it is dropped unless the origin is
///   [`Origin::Answer`] (Rulings X and AL);
/// - `push-view`, `pop-view` and `close-window` are dropped in answer to a widget event that
///   is not a user action ([`Origin::WidgetEvent`]);
/// - `replace-items` needs `index` (its 1000-item cap is a size cap), and at most
///   [`MAX_UNSOLICITED_REPLACES`] unsolicited one per [`REPLACE_WINDOW`];
/// - at most [`MAX_PUSHES_PER_ANSWER`] `push-view` per answer, whatever the origin;
/// - at most [`MAX_TOASTS`] toasts per [`TOAST_WINDOW`];
/// - at most [`MAX_UNSOLICITED_UPDATES`] unsolicited `push-view`s and `update-view`s,
///   counted together, per [`UPDATE_WINDOW`];
/// - at most [`MAX_UNSOLICITED_WIDGET_EFFECTS`] unsolicited `show-widgets` and
///   `set-widget-image`, counted together, per [`WIDGET_WINDOW`]; `redraw-widget` is never
///   counted (the core merges redraws).
///
/// The widget effects are rules of the whole list too: `show-widgets` names at most
/// [`MAX_WIDGETS`] ids of `widgets`, none twice; `redraw-widget` names one of `widgets`;
/// `set-widget-image` passes [`check_image`], and the images of the list fit the plugin's
/// image budget ([`ImageBudget`]) together.
///
/// A denied effect is dropped and named in [`Accepted::denied`]; only effects that pass count
/// towards a rate or the image budget.
pub fn check_effects(
    provider: &str,
    perms: &Permissions,
    widgets: &[WidgetDecl],
    rates: &mut Rates,
    origin: Origin,
    now: Instant,
    effects: Vec<Effect>,
) -> Result<Accepted, CapError> {
    let mut valid = Vec::with_capacity(effects.len());
    // The images of the list, counted on a copy: a rejected list counts nothing.
    let mut trial: Option<ImageBudget> = None;
    for effect in effects {
        valid.push(match effect {
            Effect::PushView(v) => {
                view(&v)?;
                Effect::PushView(v)
            }
            Effect::UpdateView(v) => {
                view(&v)?;
                Effect::UpdateView(v)
            }
            Effect::Toast(t) => {
                label("toast title", &t.title)?;
                if let Some(message) = &t.message {
                    bytes("toast message", message, MAX_TOAST_MESSAGE_BYTES)?;
                }
                Effect::Toast(t)
            }
            Effect::ReplaceItems(items) => {
                let items = own_items(provider, items)?;
                count("replace-items", items.len(), MAX_INDEX_ITEMS)?;
                Effect::ReplaceItems(items)
            }
            Effect::ShowWidgets(ids) => {
                show_widgets(widgets, &ids)?;
                Effect::ShowWidgets(ids)
            }
            Effect::RedrawWidget(id) => {
                is_declared(widgets, &id)?;
                Effect::RedrawWidget(id)
            }
            Effect::SetWidgetImage(image) => {
                check_image(&image)?;
                trial
                    .get_or_insert_with(|| rates.images.clone())
                    .admit(&image)?;
                Effect::SetWidgetImage(image)
            }
            other => other,
        });
    }
    let unsolicited = matches!(origin, Origin::Unsolicited | Origin::WidgetEvent);
    let widget_event = origin == Origin::WidgetEvent;
    let mut checked = Accepted::default();
    let mut pushes = 0;
    for effect in valid {
        let denied = match &effect {
            Effect::SetRootQuery(_) => Some("set-root-query is not a plugin effect"),
            Effect::PushView(_) if widget_event => {
                Some("push-view needs a user action (an action, a key or a completed click)")
            }
            Effect::PopView if widget_event => {
                Some("pop-view needs a user action (an action, a key or a completed click)")
            }
            Effect::CloseWindow if widget_event => {
                Some("close-window needs a user action (an action, a key or a completed click)")
            }
            Effect::PushView(_) if pushes >= MAX_PUSHES_PER_ANSWER => {
                Some("more than 1 push-view in one answer")
            }
            Effect::CopyText(_) if !perms.clipboard => {
                Some("copy-text needs the clipboard permission")
            }
            Effect::CopyText(_) if origin != Origin::Answer => Some(
                "copy-text needs a user action (run, a chosen action, a submitted form, or a \
                     widget's action, key or completed click)",
            ),
            Effect::ReplaceItems(_) if !perms.index => {
                Some("replace-items needs the index permission")
            }
            Effect::ReplaceItems(_)
                if unsolicited
                    && !rates
                        .replaces
                        .admit(now, MAX_UNSOLICITED_REPLACES, REPLACE_WINDOW) =>
            {
                Some("more than 1 unsolicited replace-items per 5 s")
            }
            Effect::Toast(_) if !rates.toasts.admit(now, MAX_TOASTS, TOAST_WINDOW) => {
                Some("more than 10 toasts in 10 s")
            }
            Effect::PushView(_) | Effect::UpdateView(_)
                if unsolicited
                    && !rates
                        .updates
                        .admit(now, MAX_UNSOLICITED_UPDATES, UPDATE_WINDOW) =>
            {
                Some("more than 30 unsolicited view pushes and updates per s")
            }
            Effect::ShowWidgets(_) | Effect::SetWidgetImage(_)
                if unsolicited
                    && !rates
                        .widgets
                        .admit(now, MAX_UNSOLICITED_WIDGET_EFFECTS, WIDGET_WINDOW) =>
            {
                Some("more than 30 unsolicited show-widgets and set-widget-image per s")
            }
            _ => None,
        };
        match denied {
            Some(reason) => checked.deny(reason),
            None => {
                pushes += usize::from(matches!(effect, Effect::PushView(_)));
                if let Effect::SetWidgetImage(image) = &effect {
                    // Every effect of the list is checked at the same `now`, so once the window
                    // is full every later image is dropped: the images that pass are a prefix
                    // of the list, and the trial above admitted each prefix in turn.
                    let admitted = rates.images.admit(image);
                    debug_assert!(
                        admitted.is_ok(),
                        "a prefix of an admitted list: {admitted:?}"
                    );
                }
                checked.effects.push(effect);
            }
        }
    }
    Ok(checked)
}

/// The ids of a `show-widgets`: at most [`MAX_WIDGETS`], each declared, none twice.
fn show_widgets(widgets: &[WidgetDecl], ids: &[String]) -> Checked {
    count("show-widgets", ids.len(), MAX_WIDGETS)?;
    for (at, id) in ids.iter().enumerate() {
        is_declared(widgets, id)?;
        if ids[..at].contains(id) {
            return fail(format!("show-widgets names {id:?} twice"));
        }
    }
    Ok(())
}

/// Checks a `search` answer of `answered` items, of which `items` had a provider prefix: at
/// most [`MAX_SEARCH_ITEMS`] as answered (before any item is dropped), each item as in
/// `replace-items`.
pub fn check_search(
    provider: &str,
    answered: usize,
    items: Vec<Item>,
) -> Result<Vec<Item>, CapError> {
    if answered > MAX_SEARCH_ITEMS {
        return fail(format!(
            "search answered {answered} items, at most {MAX_SEARCH_ITEMS}"
        ));
    }
    own_items(provider, items)
}

/// Calls `f` on every icon in `effect`: item icons, row icons and tile images.
pub(crate) fn for_each_icon(effect: &mut Effect, f: &mut impl FnMut(&mut Option<Icon>)) {
    match effect {
        Effect::PushView(v) | Effect::UpdateView(v) => match v {
            View::List(list) => list.rows.iter_mut().for_each(|r| f(&mut r.icon)),
            View::Grid(grid) => grid.tiles.iter_mut().for_each(|t| f(&mut t.image)),
            View::Detail(_) | View::Form(_) | View::Scratchpad(_) => {}
        },
        Effect::ReplaceItems(items) => items.iter_mut().for_each(|i| f(&mut i.icon)),
        Effect::PopView
        | Effect::CloseWindow
        | Effect::Toast(_)
        | Effect::CopyText(_)
        | Effect::SetRootQuery(_)
        | Effect::ShowWidgets(_)
        | Effect::RedrawWidget(_)
        | Effect::SetWidgetImage(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_core::view::{
        DetailView, FormField, FormView, GridTile, GridView, LineAnnotation, ListRow, ListView,
        ScratchpadView, SearchMode,
    };
    use tomakast_core::{ItemId, ItemKind, Toast, ToastKind, ViewId};

    const P: &str = "demo";

    /// Every permission, so that only the caps decide.
    fn all() -> Permissions {
        Permissions {
            storage: true,
            clipboard: true,
            open: true,
            timers: true,
            index: true,
            search: true,
            http: vec![],
        }
    }

    /// The widgets the tests' plugin declares: `w0` to `w16`.
    fn decls() -> Vec<WidgetDecl> {
        (0..=MAX_WIDGETS)
            .map(|i| WidgetDecl {
                id: format!("w{i}"),
                title: format!("W{i}"),
                size: tomakast_core::widget::WidgetSize::Single,
            })
            .collect()
    }

    fn check_all(effects: Vec<Effect>) -> Result<Vec<Effect>, CapError> {
        let checked = check_effects(
            P,
            &all(),
            &decls(),
            &mut Rates::default(),
            Origin::Answer,
            Instant::now(),
            effects,
        )?;
        assert!(checked.denied.is_empty(), "{:?}", checked.denied);
        Ok(checked.effects)
    }

    fn check(effect: Effect) -> Result<Vec<Effect>, CapError> {
        check_all(vec![effect])
    }

    fn act(id: &str) -> Action {
        Action {
            id: id.into(),
            title: "A".into(),
            shortcut: None,
        }
    }

    fn row(id: &str) -> ListRow {
        ListRow {
            id: id.into(),
            section: None,
            title: "Row".into(),
            subtitle: None,
            icon: None,
            accessories: vec![],
            detail_markdown: None,
            actions: vec![],
        }
    }

    fn list(rows: Vec<ListRow>) -> Effect {
        Effect::PushView(View::List(ListView {
            id: ViewId("v".into()),
            title: "List".into(),
            placeholder: String::new(),
            search: SearchMode::CoreFilter,
            query: String::new(),
            revision: 0,
            rows,
            show_detail: false,
        }))
    }

    fn with_row(f: impl FnOnce(&mut ListRow)) -> Effect {
        let mut r = row("r");
        f(&mut r);
        list(vec![r])
    }

    fn detail(f: impl FnOnce(&mut DetailView)) -> Effect {
        let mut view = DetailView {
            id: ViewId("d".into()),
            title: "Detail".into(),
            markdown: String::new(),
            metadata: vec![],
            actions: vec![],
        };
        f(&mut view);
        Effect::UpdateView(View::Detail(view))
    }

    fn form(fields: Vec<FormField>) -> Effect {
        Effect::PushView(View::Form(FormView {
            id: ViewId("f".into()),
            title: "Form".into(),
            fields,
            submit_title: "Go".into(),
        }))
    }

    fn field(kind: FieldKind) -> FormField {
        FormField {
            id: "x".into(),
            label: "X".into(),
            kind,
            error: None,
        }
    }

    fn item(local: &str) -> Item {
        Item {
            id: ItemId::new(P, local),
            title: "T".into(),
            subtitle: None,
            keywords: vec![],
            icon: None,
            kind: ItemKind::Command,
            actions: vec![act("a")],
            source: None,
        }
    }

    fn with_item(f: impl FnOnce(&mut Item)) -> Effect {
        let mut i = item("x");
        f(&mut i);
        Effect::ReplaceItems(vec![i])
    }

    /// `make(limit)` passes and `make(limit + 1)` is rejected.
    fn at_limit(limit: usize, make: impl Fn(usize) -> Effect) {
        assert!(check(make(limit)).is_ok(), "at the limit {limit}");
        assert!(
            check(make(limit + 1)).is_err(),
            "one over the limit {limit}"
        );
    }

    #[test]
    fn view_size_caps() {
        at_limit(MAX_LIST_ROWS, |n| {
            list((0..n).map(|i| row(&format!("r{i}"))).collect())
        });
        at_limit(MAX_GRID_TILES, |n| {
            Effect::PushView(View::Grid(GridView {
                id: ViewId("g".into()),
                title: "Grid".into(),
                columns: 4,
                tiles: (0..n)
                    .map(|i| GridTile {
                        id: format!("t{i}"),
                        image: None,
                        caption: "c".into(),
                        actions: vec![],
                    })
                    .collect(),
            }))
        });
        at_limit(MAX_FORM_FIELDS, |n| {
            form(
                (0..n)
                    .map(|_| field(FieldKind::Checkbox { value: false }))
                    .collect(),
            )
        });
        at_limit(MAX_MARKDOWN_BYTES, |n| {
            detail(|d| d.markdown = "m".repeat(n))
        });
        at_limit(MAX_MARKDOWN_BYTES, |n| {
            with_row(|r| r.detail_markdown = Some("m".repeat(n)))
        });
        at_limit(MAX_SCRATCHPAD_BYTES, |n| {
            Effect::UpdateView(View::Scratchpad(ScratchpadView {
                id: ViewId("s".into()),
                title: "Pad".into(),
                text: "t".repeat(n),
                revision: 0,
                annotations: vec![],
                actions: vec![],
            }))
        });
    }

    #[test]
    fn inner_caps() {
        at_limit(MAX_KEYWORDS, |n| {
            with_item(|i| i.keywords = vec!["k".into(); n])
        });
        at_limit(MAX_ACCESSORIES, |n| {
            with_row(|r| r.accessories = vec![Accessory::Tag("t".into()); n])
        });
        at_limit(MAX_METADATA, |n| {
            detail(|d| d.metadata = vec![("k".into(), "v".into()); n])
        });
        at_limit(MAX_DROPDOWN_OPTIONS, |n| {
            form(vec![field(FieldKind::Dropdown {
                options: vec![("v".into(), "l".into()); n],
                value: "v".into(),
            })])
        });
        at_limit(MAX_ANNOTATIONS, |n| {
            Effect::PushView(View::Scratchpad(ScratchpadView {
                id: ViewId("s".into()),
                title: "Pad".into(),
                text: String::new(),
                revision: 0,
                annotations: vec![
                    LineAnnotation {
                        line: 0,
                        text: "a".into(),
                        is_error: false,
                    };
                    n
                ],
                actions: vec![],
            }))
        });
        at_limit(MAX_TEXT_FIELD_BYTES, |n| {
            form(vec![field(FieldKind::Text {
                value: "v".repeat(n),
                placeholder: String::new(),
            })])
        });
        at_limit(MAX_MULTILINE_BYTES, |n| {
            form(vec![field(FieldKind::Multiline {
                value: "v".repeat(n),
            })])
        });
    }

    fn pad(f: impl FnOnce(&mut ScratchpadView)) -> Effect {
        let mut view = ScratchpadView {
            id: ViewId("s".into()),
            title: "Pad".into(),
            text: String::new(),
            revision: 0,
            annotations: vec![],
            actions: vec![],
        };
        f(&mut view);
        Effect::PushView(View::Scratchpad(view))
    }

    fn annotation(line: u32, text: &str) -> LineAnnotation {
        LineAnnotation {
            line,
            text: text.into(),
            is_error: false,
        }
    }

    fn grid(columns: u8) -> Effect {
        Effect::PushView(View::Grid(GridView {
            id: ViewId("g".into()),
            title: "Grid".into(),
            columns,
            tiles: vec![],
        }))
    }

    fn dropdown(value: &str, option_value: &str) -> Effect {
        form(vec![field(FieldKind::Dropdown {
            options: vec![(option_value.into(), "Label".into())],
            value: value.into(),
        })])
    }

    fn date(value: Option<&str>) -> Effect {
        form(vec![field(FieldKind::Date {
            value: value.map(str::to_string),
        })])
    }

    /// Ruling V: the strings and numbers the first caps left to the 4 MiB aggregate.
    #[test]
    fn text_and_number_caps() {
        at_limit(MAX_TOAST_MESSAGE_BYTES, |n| {
            Effect::Toast(Toast {
                kind: ToastKind::Error,
                title: "t".into(),
                message: Some("m".repeat(n)),
            })
        });
        at_limit(MAX_METADATA_VALUE_BYTES, |n| {
            detail(|d| d.metadata = vec![("k".into(), "v".repeat(n))])
        });
        at_limit(MAX_ANNOTATION_BYTES, |n| {
            pad(|p| p.annotations = vec![annotation(0, &"a".repeat(n))])
        });
        at_limit(MAX_KEYWORD_BYTES, |n| {
            with_item(|i| i.keywords = vec!["k".repeat(n)])
        });
        at_limit(MAX_LIST_QUERY_BYTES, |n| {
            let Effect::PushView(View::List(mut view)) = list(vec![]) else {
                unreachable!()
            };
            view.query = "q".repeat(n);
            Effect::PushView(View::List(view))
        });
        at_limit(usize::from(MAX_GRID_COLUMNS), |n| grid(n as u8));
        assert!(check(grid(0)).is_ok(), "0 means 1");
        at_limit(MAX_ANNOTATION_LINES as usize - 1, |n| {
            pad(|p| p.annotations = vec![annotation(n as u32, "a")])
        });
    }

    /// Ruling V: icon names, plugin-file paths and dropdown values follow the id rule.
    #[test]
    fn names_and_values_follow_the_id_rule() {
        let named = |s: &str| with_row(|r| r.icon = Some(Icon::Named(s.into())));
        let file = |s: &str| with_row(|r| r.icon = Some(Icon::PluginFile(s.into())));
        let makers: [&dyn Fn(&str) -> Effect; 4] =
            [&named, &file, &|s: &str| dropdown(s, "v"), &|s: &str| {
                dropdown("v", s)
            }];
        for make in makers {
            at_limit(256, |n| make(&"x".repeat(n)));
            assert!(check(make("a\u{202e}b")).is_err());
            assert!(check(make("a\nb")).is_err());
        }
    }

    #[test]
    fn dates_are_empty_or_iso() {
        for ok in [None, Some(""), Some("2026-11-19"), Some("0001-01-31")] {
            assert!(check(date(ok)).is_ok(), "{ok:?}");
        }
        for bad in [
            "2026-1-19",
            "2026-13-01",
            "2026-00-10",
            "2026-11-32",
            "2026-11-00",
            "20261119",
            "2026-11-19x",
            "2026/11/19",
            "2026-1１-19",
            " 2026-11-19",
            // Ten bytes with a two-byte character where the month goes: no panic.
            "2026-é-19",
        ] {
            assert!(check(date(Some(bad))).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn actions_at_most_16_everywhere() {
        let acts = |n: usize| (0..n).map(|i| act(&format!("a{i}"))).collect::<Vec<_>>();
        at_limit(MAX_ACTIONS, |n| with_row(|r| r.actions = acts(n)));
        at_limit(MAX_ACTIONS, |n| with_item(|i| i.actions = acts(n)));
        at_limit(MAX_ACTIONS, |n| detail(|d| d.actions = acts(n)));
        at_limit(MAX_ACTIONS, |n| pad(|p| p.actions = acts(n)));
        // A scratchpad's actions follow the same id, title and shortcut rules (Ruling AQ).
        assert!(check(pad(|p| p.actions = vec![act("a\nb")])).is_err());
        assert!(
            check(pad(|p| p.actions = vec![Action {
                shortcut: Some("ctrl+c".into()),
                ..act("copy")
            }]))
            .is_err()
        );
        // An item needs at least one action.
        assert!(check(with_item(|i| i.actions.clear())).is_err());
    }

    #[test]
    fn labels_at_most_1_kib() {
        at_limit(MAX_LABEL_BYTES, |n| with_row(|r| r.title = "t".repeat(n)));
        at_limit(MAX_LABEL_BYTES, |n| {
            with_row(|r| r.subtitle = Some("t".repeat(n)))
        });
        at_limit(MAX_LABEL_BYTES, |n| {
            with_row(|r| r.section = Some("t".repeat(n)))
        });
        at_limit(MAX_LABEL_BYTES, |n| {
            with_row(|r| r.accessories = vec![Accessory::Text("t".repeat(n))])
        });
        at_limit(MAX_LABEL_BYTES, |n| with_item(|i| i.title = "t".repeat(n)));
        at_limit(MAX_LABEL_BYTES, |n| {
            with_row(|r| {
                r.actions = vec![Action {
                    id: "a".into(),
                    title: "t".repeat(n),
                    shortcut: None,
                }]
            })
        });
        at_limit(MAX_LABEL_BYTES, |n| {
            form(vec![FormField {
                error: Some("e".repeat(n)),
                ..field(FieldKind::Checkbox { value: true })
            }])
        });
        at_limit(MAX_LABEL_BYTES, |n| {
            Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "t".repeat(n),
                message: None,
            })
        });
    }

    #[test]
    fn ids_have_no_control_or_format_characters() {
        for bad in ["a\nb", "a\u{202e}b", "a\u{200b}b", &"x".repeat(257)] {
            assert!(check(with_row(|r| r.id = bad.into())).is_err(), "{bad:?}");
            assert!(
                check(detail(|d| d.id = ViewId(bad.into()))).is_err(),
                "{bad:?}"
            );
            assert!(
                check(with_row(|r| r.actions = vec![act(bad)])).is_err(),
                "{bad:?}"
            );
            assert!(
                check(form(vec![FormField {
                    id: bad.into(),
                    ..field(FieldKind::Checkbox { value: true })
                }]))
                .is_err(),
                "{bad:?}"
            );
            let id = ItemId::parse(&format!("{P}/{bad}")).unwrap();
            assert!(check(with_item(|i| i.id = id)).is_err(), "{bad:?}");
        }
        assert!(check(with_row(|r| r.id = "x".repeat(256))).is_ok());
    }

    #[test]
    fn invalid_shortcut_emoji_or_colour_rejects_the_list() {
        let shortcut = |s: &str| {
            with_row(|r| {
                r.actions = vec![Action {
                    id: "a".into(),
                    title: "A".into(),
                    shortcut: Some(s.into()),
                }]
            })
        };
        assert!(check(shortcut("ctrl+shift+c")).is_ok());
        for bad in [
            "ctrl+c",
            "shift+x",
            "alt+tab",
            "hyper+q",
            "ctrl+",
            "ctrl+x\nx",
        ] {
            assert!(check(shortcut(bad)).is_err(), "{bad}");
        }
        let icon = |i: Icon| with_row(|r| r.icon = Some(i));
        assert!(check(icon(Icon::Emoji("🍎".into()))).is_ok());
        assert!(check(icon(Icon::Emoji("🍎\u{202e}".into()))).is_err());
        assert!(check(icon(Icon::Color("#a1b2c3".into()))).is_ok());
        assert!(check(icon(Icon::Color("red".into()))).is_err());
        // The whole list goes, not just the bad effect.
        let ok = Effect::CopyText("x".into());
        let bad = icon(Icon::Color("red".into()));
        assert!(check_all(vec![ok, bad]).is_err());
    }

    #[test]
    fn plugin_files_stay_inside_the_plugin_folder() {
        let file = |p: &str| with_item(|i| i.icon = Some(Icon::PluginFile(p.into())));
        assert!(check(file("icons/a.png")).is_ok());
        assert!(check(file("./a.png")).is_ok());
        for bad in ["/etc/passwd", "../a.png", "icons/../../a.png", ""] {
            assert!(check(file(bad)).is_err(), "{bad:?}");
        }
        let tile = |p: &str| {
            Effect::PushView(View::Grid(GridView {
                id: ViewId("g".into()),
                title: "G".into(),
                columns: 1,
                tiles: vec![GridTile {
                    id: "t".into(),
                    image: Some(Icon::PluginFile(p.into())),
                    caption: "c".into(),
                    actions: vec![],
                }],
            }))
        };
        assert!(check(tile("a.png")).is_ok());
        assert!(check(tile("../a.png")).is_err());
    }

    #[test]
    fn replace_items_drops_foreign_items_and_caps_the_count() {
        let mut foreign = item("x");
        foreign.id = ItemId::new("other", "x");
        let effects = check(Effect::ReplaceItems(vec![item("a"), foreign])).unwrap();
        assert!(matches!(&effects[..], [Effect::ReplaceItems(items)] if items.len() == 1));
        at_limit(MAX_INDEX_ITEMS, |n| {
            Effect::ReplaceItems((0..n).map(|i| item(&format!("i{i}"))).collect())
        });
    }

    #[test]
    fn search_answers_at_most_50_items() {
        let items = |n: usize| (0..n).map(|i| item(&format!("i{i}"))).collect::<Vec<_>>();
        let search = |items: Vec<Item>| check_search(P, items.len(), items);
        assert_eq!(search(items(MAX_SEARCH_ITEMS)).unwrap().len(), 50);
        assert!(search(items(MAX_SEARCH_ITEMS + 1)).is_err());
        // Counted as answered: items dropped for lacking a provider prefix count too.
        assert!(check_search(P, MAX_SEARCH_ITEMS + 1, items(1)).is_err());
        let mut foreign = item("x");
        foreign.id = ItemId::new("other", "x");
        assert_eq!(search(vec![item("a"), foreign]).unwrap().len(), 1);
        let no_actions = Item {
            actions: vec![],
            ..item("a")
        };
        assert!(search(vec![no_actions]).is_err());
    }

    fn toast() -> Effect {
        Effect::Toast(Toast {
            kind: ToastKind::Success,
            title: "t".into(),
            message: None,
        })
    }

    fn update() -> Effect {
        detail(|_| {})
    }

    fn gate(
        perms: &Permissions,
        rates: &mut Rates,
        origin: Origin,
        now: Instant,
        effects: Vec<Effect>,
    ) -> Accepted {
        check_effects(P, perms, &decls(), rates, origin, now, effects).unwrap()
    }

    #[test]
    fn set_root_query_is_never_a_plugin_effect() {
        let checked = gate(
            &all(),
            &mut Rates::default(),
            Origin::Answer,
            Instant::now(),
            vec![Effect::SetRootQuery("x".into()), toast()],
        );
        assert_eq!(checked.effects, [toast()]);
        assert_eq!(checked.denied, ["set-root-query is not a plugin effect"]);
    }

    #[test]
    fn copy_text_needs_the_clipboard_permission() {
        let effects = vec![Effect::CopyText("x".into()), toast()];
        let none = Permissions::default();
        let checked = gate(
            &none,
            &mut Rates::default(),
            Origin::Unsolicited,
            Instant::now(),
            effects.clone(),
        );
        assert_eq!(checked.effects, [toast()], "the rest of the answer is kept");
        assert_eq!(checked.denied, ["copy-text needs the clipboard permission"]);
        let clipboard = Permissions {
            clipboard: true,
            ..Permissions::default()
        };
        let checked = gate(
            &clipboard,
            &mut Rates::default(),
            Origin::Answer,
            Instant::now(),
            effects.clone(),
        );
        assert_eq!(checked.effects, effects);
        assert!(checked.denied.is_empty());
        // Rulings X and AL: with the permission, still only in answer to a user action.
        for origin in [Origin::Unsolicited, Origin::Reply, Origin::WidgetEvent] {
            let checked = gate(
                &clipboard,
                &mut Rates::default(),
                origin,
                Instant::now(),
                effects.clone(),
            );
            assert_eq!(checked.effects, [toast()], "{origin:?}");
            assert_eq!(
                checked.denied,
                [
                    "copy-text needs a user action (run, a chosen action, a submitted form, or a \
                     widget's action, key or completed click)"
                ]
            );
        }
    }

    /// Ruling AD: at most one unsolicited replace-items per 5 s; answers are not limited.
    #[test]
    fn unsolicited_replace_items_once_per_5_s() {
        let mut rates = Rates::default();
        let start = Instant::now();
        let replace = || vec![Effect::ReplaceItems(vec![item("a")])];
        let answer = gate(
            &all(),
            &mut rates,
            Origin::Answer,
            start,
            [replace(), replace()].concat(),
        );
        assert_eq!(answer.effects.len(), 2);
        let first = gate(&all(), &mut rates, Origin::Unsolicited, start, replace());
        assert_eq!(first.effects.len(), 1);
        let later = start + Duration::from_millis(4_900);
        let second = gate(&all(), &mut rates, Origin::Unsolicited, later, replace());
        assert!(second.effects.is_empty());
        assert_eq!(
            second.denied,
            ["more than 1 unsolicited replace-items per 5 s"]
        );
        let after = start + REPLACE_WINDOW;
        assert_eq!(
            gate(&all(), &mut rates, Origin::Unsolicited, after, replace())
                .effects
                .len(),
            1
        );
    }

    #[test]
    fn replace_items_needs_the_index_permission() {
        let effects = vec![
            Effect::ReplaceItems(vec![item("a")]),
            Effect::ReplaceItems(vec![]),
        ];
        let none = Permissions::default();
        let checked = gate(
            &none,
            &mut Rates::default(),
            Origin::Answer,
            Instant::now(),
            effects.clone(),
        );
        assert!(checked.effects.is_empty());
        assert_eq!(
            checked.denied,
            ["replace-items needs the index permission"],
            "named once"
        );
        let checked = gate(
            &all(),
            &mut Rates::default(),
            Origin::Answer,
            Instant::now(),
            effects.clone(),
        );
        assert_eq!(checked.effects, effects);
        // The cap still rejects the whole answer, with or without the permission.
        let flood = Effect::ReplaceItems(
            (0..=MAX_INDEX_ITEMS)
                .map(|i| item(&format!("i{i}")))
                .collect(),
        );
        for perms in [none, all()] {
            let result = check_effects(
                P,
                &perms,
                &decls(),
                &mut Rates::default(),
                Origin::Answer,
                Instant::now(),
                vec![flood.clone()],
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn toasts_10_per_10_s() {
        let mut rates = Rates::default();
        let start = Instant::now();
        let toasts = |n: usize| vec![toast(); n];
        let checked = gate(&all(), &mut rates, Origin::Answer, start, toasts(12));
        assert_eq!(checked.effects.len(), 10);
        assert_eq!(checked.denied, ["more than 10 toasts in 10 s"]);
        // Sliding: 9.9 s later the window is still full, at 10 s it is empty again.
        let later = start + Duration::from_millis(9_900);
        assert!(
            gate(&all(), &mut rates, Origin::Unsolicited, later, toasts(1))
                .effects
                .is_empty()
        );
        let after = start + TOAST_WINDOW;
        assert_eq!(
            gate(&all(), &mut rates, Origin::Unsolicited, after, toasts(11))
                .effects
                .len(),
            10
        );
    }

    #[test]
    fn unsolicited_updates_30_per_s() {
        let mut rates = Rates::default();
        let start = Instant::now();
        let updates = |n: usize| vec![update(); n];
        // Answers are not unsolicited: no limit, and they do not count.
        let checked = gate(&all(), &mut rates, Origin::Answer, start, updates(100));
        assert_eq!(checked.effects.len(), 100);
        let checked = gate(&all(), &mut rates, Origin::Unsolicited, start, updates(31));
        assert_eq!(checked.effects.len(), 30);
        assert_eq!(
            checked.denied,
            ["more than 30 unsolicited view pushes and updates per s"]
        );
        let later = start + Duration::from_millis(999);
        assert!(
            gate(&all(), &mut rates, Origin::Unsolicited, later, updates(1))
                .effects
                .is_empty()
        );
        let after = start + UPDATE_WINDOW;
        assert_eq!(
            gate(&all(), &mut rates, Origin::Unsolicited, after, updates(30))
                .effects
                .len(),
            30
        );
        // Ruling AB: unsolicited pushes share the window with updates.
        let later = after + UPDATE_WINDOW;
        let mixed = [vec![list(vec![])], updates(30)].concat();
        let checked = gate(&all(), &mut rates, Origin::Unsolicited, later, mixed);
        assert_eq!(checked.effects.len(), 30);
        assert!(matches!(checked.effects[0], Effect::PushView(_)));
        assert_eq!(checked.denied.len(), 1);
    }

    /// Minor 2 of the final review: one answer pushes at most one view, whatever its origin,
    /// so one `run` cannot fill the launcher's stack. The rest of the answer is kept.
    #[test]
    fn one_push_view_per_answer() {
        for origin in [Origin::Answer, Origin::Reply, Origin::Unsolicited] {
            let mut rates = Rates::default();
            let effects = vec![list(vec![]), toast(), list(vec![]), list(vec![])];
            let checked = gate(&all(), &mut rates, origin, Instant::now(), effects);
            assert_eq!(checked.effects, [list(vec![]), toast()], "{origin:?}");
            assert_eq!(checked.denied, ["more than 1 push-view in one answer"]);
        }
        // A push dropped by the update rate leaves the answer's one push unused.
        let mut rates = Rates::default();
        let now = Instant::now();
        gate(
            &all(),
            &mut rates,
            Origin::Unsolicited,
            now,
            vec![update(); 30],
        );
        let checked = gate(
            &all(),
            &mut rates,
            Origin::Unsolicited,
            now,
            vec![list(vec![])],
        );
        assert!(checked.effects.is_empty());
    }

    /// Minor 9 of the final review: titles and labels are one line; markdown, metadata
    /// values, text values and toast messages keep their newlines.
    #[test]
    fn labels_have_no_control_characters() {
        for bad in ["a\nb", "a\rb", "a\tb", "a\u{0}b", "a\u{85}b"] {
            let rejected = [
                with_row(|r| r.title = bad.into()),
                with_row(|r| r.subtitle = Some(bad.into())),
                with_row(|r| r.section = Some(bad.into())),
                with_row(|r| r.accessories = vec![Accessory::Tag(bad.into())]),
                with_item(|i| i.title = bad.into()),
                detail(|d| d.title = bad.into()),
                detail(|d| d.metadata = vec![(bad.into(), "v".into())]),
                form(vec![FormField {
                    label: bad.into(),
                    ..field(FieldKind::Checkbox { value: true })
                }]),
                Effect::Toast(Toast {
                    kind: ToastKind::Success,
                    title: bad.into(),
                    message: None,
                }),
            ];
            for effect in rejected {
                assert!(check(effect.clone()).is_err(), "{bad:?} in {effect:?}");
            }
        }
        let multi = "line 1\nline 2";
        for kept in [
            detail(|d| d.markdown = multi.into()),
            detail(|d| d.metadata = vec![("k".into(), multi.into())]),
            form(vec![field(FieldKind::Multiline {
                value: multi.into(),
            })]),
            Effect::Toast(Toast {
                kind: ToastKind::Error,
                title: "t".into(),
                message: Some(multi.into()),
            }),
            // Format characters stay allowed in display text.
            with_row(|r| r.title = "a\u{200d}b".into()),
        ] {
            assert!(check(kept.clone()).is_ok(), "{kept:?}");
        }
    }

    #[test]
    fn a_rejected_answer_counts_towards_no_rate() {
        let mut rates = Rates::default();
        let now = Instant::now();
        let mut effects = vec![toast(); 10];
        effects.push(with_row(|r| r.id = "a\nb".into()));
        assert!(
            check_effects(
                P,
                &all(),
                &decls(),
                &mut rates,
                Origin::Answer,
                now,
                effects
            )
            .is_err()
        );
        assert_eq!(
            gate(&all(), &mut rates, Origin::Answer, now, vec![toast(); 10])
                .effects
                .len(),
            10
        );
    }

    fn png(id: &str, len: usize) -> Effect {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.resize(len, 0);
        Effect::SetWidgetImage(tomakast_core::widget::WidgetImage {
            id: id.into(),
            format: tomakast_core::widget::ImageFormat::Png,
            bytes: bytes.into(),
        })
    }

    fn show(ids: &[&str]) -> Effect {
        Effect::ShowWidgets(ids.iter().map(|id| id.to_string()).collect())
    }

    /// A widget event that is not a user action may not close the launcher or push or pop a
    /// view; it may update one, unsolicited, and its answer counts towards the unsolicited
    /// rates.
    #[test]
    fn a_widget_event_that_is_no_user_action_cannot_navigate() {
        let now = Instant::now();
        let effects = vec![
            Effect::CloseWindow,
            list(vec![]),
            Effect::PopView,
            update(),
            toast(),
        ];
        let mut rates = Rates::default();
        let checked = gate(
            &all(),
            &mut rates,
            Origin::WidgetEvent,
            now,
            effects.clone(),
        );
        assert_eq!(checked.effects, [update(), toast()]);
        assert_eq!(
            checked.denied,
            [
                "close-window needs a user action (an action, a key or a completed click)",
                "push-view needs a user action (an action, a key or a completed click)",
                "pop-view needs a user action (an action, a key or a completed click)",
            ]
        );
        // The same answer to a user action passes whole.
        let answer = gate(&all(), &mut Rates::default(), Origin::Answer, now, effects);
        assert_eq!(answer.effects.len(), 5);
        assert!(answer.denied.is_empty());
        // Unsolicited for the rates: 29 more updates fit the window, the 31st does not.
        let updates = gate(
            &all(),
            &mut rates,
            Origin::WidgetEvent,
            now,
            vec![update(); 30],
        );
        assert_eq!(updates.effects.len(), 29);
    }

    #[test]
    fn show_widgets_names_declared_widgets_once() {
        assert!(check(show(&["w0", "w1"])).is_ok());
        let err = check(show(&["w0", "nope"])).unwrap_err();
        assert!(err.0.contains("\"nope\" is not declared"), "{err}");
        let err = check(show(&["w0", "w0"])).unwrap_err();
        assert!(err.0.contains("twice"), "{err}");
        let ids: Vec<String> = (0..=MAX_WIDGETS).map(|i| format!("w{i}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert!(check(show(&ids[..MAX_WIDGETS])).is_ok());
        assert!(check(show(&ids)).is_err());
        // The whole answer is rejected.
        assert!(check_all(vec![toast(), show(&["nope"])]).is_err());
    }

    #[test]
    fn redraw_widget_names_a_declared_widget() {
        assert!(check(Effect::RedrawWidget("w3".into())).is_ok());
        assert!(check_all(vec![toast(), Effect::RedrawWidget("nope".into())]).is_err());
    }

    #[test]
    fn set_widget_image_is_checked_and_budgeted_as_a_whole() {
        assert!(check(png("a", 64)).is_ok());
        let mut jpeg_as_png = png("a", 64);
        if let Effect::SetWidgetImage(image) = &mut jpeg_as_png {
            image.bytes = b"\xff\xd8\xff".to_vec().into();
        }
        assert!(check(jpeg_as_png).is_err());
        let now = Instant::now();
        let mut rates = Rates::default();
        let flood = |name: &str, n: usize| {
            (0..n)
                .map(|i| png(&format!("{name}{i}"), 16))
                .collect::<Vec<_>>()
        };
        let err = check_effects(
            P,
            &all(),
            &decls(),
            &mut rates,
            Origin::Answer,
            now,
            flood("j", 65),
        )
        .unwrap_err();
        assert!(err.0.contains("more than 64 images"), "{err}");
        // A rejected list counts nothing: 64 other ids fit afterwards.
        assert_eq!(
            gate(&all(), &mut rates, Origin::Answer, now, flood("i", 64))
                .effects
                .len(),
            64
        );
        // Replacing is no new image; a new id is one too many.
        assert_eq!(
            gate(&all(), &mut rates, Origin::Answer, now, vec![png("i0", 32)])
                .effects
                .len(),
            1
        );
        let more = vec![png("more", 16)];
        assert!(check_effects(P, &all(), &decls(), &mut rates, Origin::Answer, now, more).is_err());
    }

    #[test]
    fn unsolicited_widget_effects_30_per_s() {
        let now = Instant::now();
        let mut rates = Rates::default();
        let mut effects = vec![show(&["w0"])];
        effects.extend((0..30).map(|i| png(&format!("i{i}"), 16)));
        effects.extend((0..40).map(|_| Effect::RedrawWidget("w0".into())));
        let checked = gate(&all(), &mut rates, Origin::Unsolicited, now, effects);
        assert_eq!(
            checked.denied,
            ["more than 30 unsolicited show-widgets and set-widget-image per s"]
        );
        let images = checked
            .effects
            .iter()
            .filter(|e| matches!(e, Effect::SetWidgetImage(_)))
            .count();
        assert_eq!(images, 29, "show-widgets took the first of the 30");
        let redraws = checked
            .effects
            .iter()
            .filter(|e| matches!(e, Effect::RedrawWidget(_)))
            .count();
        assert_eq!(redraws, 40, "redraw-widget is never counted");
        // A window later, the dropped one passes.
        let later = now + WIDGET_WINDOW;
        let again = gate(
            &all(),
            &mut rates,
            Origin::Unsolicited,
            later,
            vec![png("i29", 16)],
        );
        assert_eq!(again.effects.len(), 1);
        // Answers are not counted.
        let answer = gate(
            &all(),
            &mut rates,
            Origin::Answer,
            later,
            vec![show(&["w1"]); 40],
        );
        assert_eq!(answer.effects.len(), 40);
    }
}
