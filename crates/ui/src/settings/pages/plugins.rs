//! A plugin's page: what it is and its state, its preferences by declaration order with the
//! control of their kind, its commands as a table of hotkeys (as the Hotkeys page draws
//! them), and the values of its `[plugins.<id>]` table it does not declare. A table with no
//! installed plugin gets a greyed page of read-only lines, each removable, and "Remove all"; a
//! plugin whose manifest does not parse gets the lines alone, with nothing to remove. Plugin
//! pages exist only once the plugins folder is listed; they stay editable while the runtime
//! is down.

use std::borrow::Cow;
use std::rc::Rc;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window, div,
};
use tomakast_core::hotkeys::item_hotkeys;
use tomakast_core::settings_schema::{KeyValue, ValueKind, value_fits};
use tomakast_core::{CatalogPlugin, Hotkey, Icon, ItemId, PluginPreference, PluginState};

use super::hotkeys;
use super::{KeyState, Reset, error_line, key_control, note};
use crate::settings::SettingsPage;
use crate::settings::fields::{AUTOMATIC, NOT_SET, PickItem, zone_items};
use crate::settings::layout::{Group, Page, PageIcon, Row, Section, control_button};
use crate::settings::window::{FileState, SettingsWindow};
use crate::theme::{self, Palette};

/// The ids of the file's `[plugins.<id>]` entries, tables or not.
pub(super) fn table_ids(file: Option<&toml::Table>) -> Vec<String> {
    match file.and_then(|file| file.get("plugins")) {
        Some(toml::Value::Table(plugins)) => plugins.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// A value the file holds for a plugin, as a line of its page.
struct Entry {
    /// Where it is: `["plugins", id, key]`, or `["plugins", id]` when that is not a table.
    path: Vec<String>,
    /// What the line names: the key, or `plugins.<id>`.
    name: String,
    value: toml::Value,
}

/// Every value the file holds under `[plugins.<id>]`.
fn entries(file: &FileState, id: &str) -> Vec<Entry> {
    let table = file
        .file
        .as_ref()
        .and_then(|file| file.get("plugins"))
        .and_then(|plugins| plugins.get(id));
    match table {
        None => Vec::new(),
        Some(toml::Value::Table(table)) => table
            .iter()
            .map(|(key, value)| Entry {
                path: vec!["plugins".into(), id.into(), key.clone()],
                name: key.clone(),
                value: value.clone(),
            })
            .collect(),
        Some(other) => vec![Entry {
            path: vec!["plugins".into(), id.into()],
            name: format!("plugins.{id}"),
            value: other.clone(),
        }],
    }
}

/// `pref`'s value in the file, checked against its kind.
fn plugin_value(file: &FileState, plugin: &str, pref: &PluginPreference) -> KeyValue {
    let value = file
        .file
        .as_ref()
        .and_then(|file| file.get("plugins"))
        .and_then(|plugins| plugins.get(plugin))
        .and_then(|table| table.as_table())
        .and_then(|table| table.get(&pref.key));
    match value {
        None => KeyValue::Missing,
        Some(value) if value_fits(&pref.kind, value) => KeyValue::Valid(value.clone()),
        Some(value) => KeyValue::Invalid(value.clone()),
    }
}

/// What a text or number field shows for `pref`: the file's value, or the default when the
/// file has none or none of the kind.
pub(crate) fn value_text(file: &FileState, plugin: &str, pref: &PluginPreference) -> String {
    match plugin_value(file, plugin, pref) {
        KeyValue::Valid(toml::Value::String(s)) => s,
        KeyValue::Valid(value) => value.to_string(),
        _ => pref.shown_default().unwrap_or_default().to_string(),
    }
}

/// What a preference of `plugin` without a value shows when no default is known: "Automatic"
/// once the plugin has told a default of its own as it runs (it works them out, so the one it
/// has not told is still its own choice), else "Not set".
pub(crate) fn unset_text(plugin: &CatalogPlugin) -> &'static str {
    if plugin
        .preferences
        .iter()
        .any(|pref| pref.reported_default.is_some())
    {
        AUTOMATIC
    } else {
        NOT_SET
    }
}

/// What a pick field lists, and what its search field says while empty.
pub(crate) struct PickList {
    pub(crate) items: Cow<'static, [PickItem]>,
    pub(crate) search_placeholder: &'static str,
}

/// Whether a preference of `kind` is a pick field: a time zone, a choice.
fn is_pick(kind: &ValueKind) -> bool {
    matches!(kind, ValueKind::TimeZone | ValueKind::Choice(_))
}

/// The list a pick field of `pref` offers ([`is_pick`]); `None` for any other preference.
pub(crate) fn pick_list(pref: &PluginPreference) -> Option<PickList> {
    if !is_pick(&pref.kind) {
        return None;
    }
    match &pref.kind {
        ValueKind::TimeZone => Some(PickList {
            items: Cow::Borrowed(zone_items()),
            search_placeholder: "Search time zones…",
        }),
        ValueKind::Choice(options) => Some(PickList {
            items: options
                .iter()
                .map(|o| PickItem::new(o.value.clone(), o.title.clone()))
                .collect::<Vec<_>>()
                .into(),
            search_placeholder: "Search…",
        }),
        _ => None,
    }
}

/// The file's value of a pick preference, trimmed (empty: none), and whether the file holds
/// one of the wrong type.
pub(crate) fn pick_file_value(
    file: &FileState,
    plugin: &str,
    pref: &PluginPreference,
) -> (String, bool) {
    match plugin_value(file, plugin, pref) {
        KeyValue::Valid(toml::Value::String(s)) => (s.trim().to_string(), false),
        KeyValue::Invalid(_) => (String::new(), true),
        _ => (String::new(), false),
    }
}

/// The hotkey the file gives `item`, if it gives it one it can use: the first, as in the
/// daemon.
pub(crate) fn command_hotkey(file: &FileState, item: &ItemId) -> Option<Hotkey> {
    let settings = file.settings.as_ref().ok()?;
    item_hotkeys(settings)
        .0
        .into_iter()
        .find(|(of, _)| of == item)
        .map(|(_, hotkey)| hotkey)
}

/// A plugin's icon: the icon of its first command that has one, else the monogram of its
/// name.
pub(super) fn plugin_icon(plugin: &CatalogPlugin) -> PageIcon {
    PageIcon::Item {
        icon: plugin
            .commands
            .iter()
            .find_map(|command| command.icon.clone())
            .map(Icon::Named),
        monogram: plugin.name.clone(),
    }
}

pub(super) fn plugin_page(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    plugin: &CatalogPlugin,
    palette: &Palette,
) -> Page {
    let icon = plugin_icon(plugin);
    let key = SettingsPage::Plugin(plugin.id.clone());
    let page =
        Page::new(key, Section::Plugins, plugin.name.clone(), icon).group(about(plugin, palette));
    if matches!(plugin.state, PluginState::ManifestError(_)) {
        // What the plugin declares is not known: every value is a read-only line, and none
        // is removed from here, since the plugin is still there.
        let lines = lines(
            view,
            this,
            &plugin.id,
            entries(&view.file, &plugin.id),
            false,
        );
        return match lines {
            Some(lines) => page.group(saved_group(lines)),
            None => page,
        };
    }
    let disabled = view.file.error().is_some();
    let mut page = page;
    if !plugin.preferences.is_empty() {
        page = page.group(
            Group::new().title("Preferences").rows(
                plugin
                    .preferences
                    .iter()
                    .map(|pref| preference(view, this, &plugin.id, pref, disabled)),
            ),
        );
    }
    if !plugin.commands.is_empty() {
        let mut commands: Vec<hotkeys::Entry> = plugin
            .commands
            .iter()
            .map(|command| hotkeys::Entry::command(&plugin.id, command))
            .collect();
        hotkeys::bound_first(view, &mut commands, |entry| &entry.item);
        page = page.group(
            Group::new()
                .title("Commands")
                .columns(hotkeys::NAME, hotkeys::HOTKEY)
                .rows(
                    commands
                        .iter()
                        .map(|entry| hotkeys::item_row(view, entry, disabled)),
                ),
        );
    }
    let undeclared: Vec<Entry> = entries(&view.file, &plugin.id)
        .into_iter()
        .filter(|entry| {
            entry.path.len() != 3 || !plugin.preferences.iter().any(|p| p.key == entry.name)
        })
        .collect();
    if let Some(lines) = lines(view, this, &plugin.id, undeclared, true) {
        page = page.group(
            Group::new()
                .title("Not used by this plugin")
                .description("The plugin does not declare these keys.")
                .rows(lines),
        );
    }
    page
}

/// The page of a `[plugins.<id>]` table with no plugin `id` in the catalog.
pub(super) fn table_page(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    id: &str,
    palette: &Palette,
) -> Page {
    let page = Page::new(
        SettingsPage::Plugin(id.to_string()),
        Section::Plugins,
        format!("{id} (not installed)"),
        PageIcon::Greyed(id.to_string()),
    )
    .group(
        Group::new().plain().row(note(
            "table-note",
            "This plugin is not installed. Its settings stay in config.toml until you remove them."
                .into(),
            palette,
        )),
    );
    let page = match lines(view, this, id, entries(&view.file, id), true) {
        Some(lines) => page.group(saved_group(lines)),
        None => page.group(Group::new().plain().row(note(
            "table-empty",
            "No settings saved.".into(),
            palette,
        ))),
    };
    page.group(Group::new().plain().row(remove_all(view, this, id)))
}

fn saved_group(lines: Vec<Row>) -> Group {
    Group::new().title("Saved in config.toml").rows(lines)
}

/// Name, version and author, and the state when the plugin is not running.
fn about(plugin: &CatalogPlugin, palette: &Palette) -> Group {
    let mut about = Vec::new();
    if !plugin.version.is_empty() {
        about.push(format!("Version {}", plugin.version));
    }
    if let Some(author) = &plugin.author {
        about.push(format!("by {author}"));
    }
    let state = match &plugin.state {
        PluginState::Running => None,
        PluginState::NotRunning(reason) => Some(format!("Not running: {reason}")),
        PluginState::Review => Some("Waiting for your approval in the launcher".to_string()),
        PluginState::Denied => Some("Not allowed".to_string()),
        PluginState::Failed(reason) => Some(format!("Failed to load: {reason}")),
        PluginState::ManifestError(reason) => Some(format!("Manifest error: {reason}")),
    };
    let mut group = Group::new().plain();
    if !about.is_empty() {
        group = group.row(note("plugin-about", about.join(" "), palette));
    }
    if let Some(state) = state {
        group = group.row(note("plugin-state", state, palette));
    }
    if about.is_empty() && plugin.state == PluginState::Running {
        group = group.row(note("plugin-about", "Running.".into(), palette));
    }
    group
}

/// A declared preference with the control of its kind.
fn preference(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    plugin: &str,
    pref: &PluginPreference,
    disabled: bool,
) -> Row {
    let value = plugin_value(&view.file, plugin, pref);
    let mut state = KeyState {
        held: value != KeyValue::Missing,
        ..KeyState::default()
    };
    if let KeyValue::Invalid(value) = &value {
        state.lines.push(format!("Not valid here: {value}"));
    }
    let path = format!("plugins.{plugin}.{}", pref.key);
    state.lines.extend(view.errors.get(&path).cloned());
    let set = setter(this, plugin, &pref.key);
    let reset: Reset = {
        let set = set.clone();
        Rc::new(move |window, cx| set(None, window, cx))
    };
    let title = pref.title.clone();
    // A time zone, a choice: the field made for it in `sync_pick_fields`.
    let pick: Control = match view.picks.get(&(plugin.to_string(), pref.key.clone())) {
        Some(field) => {
            let field = field.clone();
            Rc::new(move |_, _| field.clone().into_any_element())
        }
        None => Rc::new(|_, _| div().into_any_element()),
    };
    let control: Control = match &pref.kind {
        ValueKind::Bool => {
            let on = match &value {
                KeyValue::Valid(toml::Value::Boolean(on)) => *on,
                _ => pref.shown_default() == Some("true"),
            };
            let title = title.clone();
            Rc::new(move |disabled, _| {
                let set = set.clone();
                Switch::new("switch")
                    .accessibility_label(title.clone())
                    .checked(on)
                    .disabled(disabled)
                    .on_click(move |checked: &bool, window, cx| {
                        set(Some(toml::Value::Boolean(*checked)), window, cx)
                    })
                    .into_any_element()
            })
        }
        ValueKind::TimeZone | ValueKind::Choice(_) => pick,
        // A plugin cannot declare a font; it would be a text field.
        ValueKind::Text | ValueKind::Number(_) | ValueKind::Font => {
            match view.prefs.get(&(plugin.to_string(), pref.key.clone())) {
                Some(field) => {
                    let field = field.clone();
                    Rc::new(move |_, _| field.clone().into_any_element())
                }
                None => Rc::new(|_, _| div().into_any_element()),
            }
        }
        // Plugins declare neither.
        ValueKind::Hotkey | ValueKind::City => Rc::new(|_, _| div().into_any_element()),
    };
    let row = Row::new(pref.title.clone(), move |options, _, cx| {
        key_control(
            &state,
            &title,
            control(options.disabled, cx),
            options.disabled,
            reset.clone(),
            cx,
        )
    })
    .keywords([pref.key.clone()])
    .disabled(disabled);
    match &pref.description {
        Some(description) => row.description(description.clone()),
        None => row,
    }
}

/// Writes (or, for `None`, removes) a value.
pub(super) type Setter = Rc<dyn Fn(Option<toml::Value>, &mut Window, &mut App)>;

fn setter(this: &WeakEntity<SettingsWindow>, plugin: &str, key: &str) -> Setter {
    let this = this.clone();
    let plugin = plugin.to_string();
    let key = key.to_string();
    Rc::new(move |value, window, cx| {
        let _ = this.update(cx, |view, cx| {
            view.set_preference(&plugin, &key, value, window, cx)
        });
    })
}

/// A control built for a row: whether it is disabled, and the app.
type Control = Rc<dyn Fn(bool, &App) -> AnyElement>;

/// Read-only `key = value` lines, each with a remove button when `removable`; `None` when
/// there are none.
fn lines(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    plugin: &str,
    entries: Vec<Entry>,
    removable: bool,
) -> Option<Vec<Row>> {
    if entries.is_empty() {
        return None;
    }
    let disabled = view.file.error().is_some();
    let items = entries.into_iter().map(|entry| {
        let text = format!("{} = {}", entry.name, entry.value);
        let error = view.errors.get(&entry.path.join(".")).cloned();
        let label = format!("Remove {}", entry.name);
        let this = this.clone();
        let plugin = plugin.to_string();
        let path = entry.path;
        let keyword = entry.name;
        Row::custom(move |options, _, cx| {
            let palette = theme::palette(cx);
            let remove = removable.then(|| {
                let this = this.clone();
                let plugin = plugin.clone();
                let path = path.clone();
                control_button("remove", &palette, cx)
                    .label("Remove")
                    .accessibility_label(label.clone())
                    .disabled(options.disabled)
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |view, cx| match path.get(2) {
                            Some(key) => view.set_preference(&plugin, key, None, window, cx),
                            None => view.remove_plugin_table(&plugin, window, cx),
                        });
                    })
            });
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .id("line")
                        .aria_label(SharedString::from(text.clone()))
                        .test_support()
                        .min_w_0()
                        .text_size(theme::text(theme::MUTED_TEXT_SIZE))
                        .text_color(palette.muted)
                        .child(text.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap_1()
                        .children(remove)
                        .children(
                            error
                                .clone()
                                .map(|error| error_line("remove-error".into(), error, &palette)),
                        ),
                )
        })
        .keywords([keyword])
        .disabled(disabled)
    });
    Some(items.collect())
}

/// "Remove all": the whole `[plugins.<id>]` table, once the user asks.
fn remove_all(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>, id: &str) -> Row {
    let error = view.errors.get(&format!("plugins.{id}")).cloned();
    let label = format!("Remove all settings of {id}");
    let this = this.clone();
    let id = id.to_string();
    Row::custom(move |options, _, cx| {
        let palette = theme::palette(cx);
        let this = this.clone();
        let id = id.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .child(
                control_button("remove-all", &palette, cx)
                    .label("Remove all")
                    .accessibility_label(label.clone())
                    .disabled(options.disabled)
                    .on_click(move |_, window, cx| {
                        let _ =
                            this.update(cx, |view, cx| view.remove_plugin_table(&id, window, cx));
                    }),
            )
            .children(
                error
                    .clone()
                    .map(|error| error_line("remove-all-error".into(), error, &palette)),
            )
    })
    .disabled(view.file.error().is_some())
}
