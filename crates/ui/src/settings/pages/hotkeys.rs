//! Hotkeys: the leader key and its mode, then one table of every item a hotkey can open (its icon, its title and its hotkey
//! field), in groups by where the items come from: Applications, then the built-in modules,
//! then the plugins by name, then "Other" for sources with no name to show; in each, the items
//! with a hotkey come first (a row moves once its field is left). Each group opens and closes
//! under its group row: Applications is open when the page is first shown, another group if
//! one of its items has a hotkey; a group opens when one of them gets one, unless closed with
//! a click. An item with a hotkey that is not there now is greyed in the group of where it came
//! from, or under "Not available" when that is not known either. The `[hotkeys]` entries the
//! launcher does not use are greyed rows with why and a remove button, in a last group, "Not
//! used".

use std::collections::{HashMap, HashSet};

use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};
use tomakast_core::hotkeys::{DropReason, DroppedHotkey, Refusal, dropped_hotkeys, item_hotkeys};
use tomakast_core::{CatalogCommand, Hotkey, Icon, ItemId};

use super::error_line;
use super::general::{leader_key, leader_mode};
use super::plugins::plugin_icon;
use crate::icons::IconSpec;
use crate::settings::SettingsPage;
use crate::settings::layout::{Group, GroupKey, Page, PageIcon, Row, Section, control_button};
use crate::settings::window::{APPS, SettingsWindow, gone_title};
use crate::theme;
use crate::views::icon_element;

/// The titles of a hotkey table's columns.
pub(super) const NAME: &str = "Name";
pub(super) const HOTKEY: &str = "Hotkey";
/// The size of an item's icon in its row.
const ICON_SIZE: f32 = 16.;
/// The gap after the icon of a "Not used" entry.
const DROPPED_ICON_GAP: f32 = 8.;
/// The built-in modules whose items can have a hotkey, by provider: their name and icon.
const MODULES: &[(&str, &str, &str)] = &[
    ("demo", "Demo", "flask-conical"),
    ("files", "Files", "folder"),
    ("history", "Search History", "clock"),
    ("home", "Home", "house"),
    ("plugins", "Plugins", "puzzle"),
    ("settings", "Settings", "settings"),
];

pub(super) fn page(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>) -> Page {
    let broken = view.file.error().is_some();
    let with_hotkey: HashSet<ItemId> = view.hotkey_items().into_iter().collect();
    // The launcher hotkey is on the Launcher page only; the leader is there and here too,
    // where "Set Leader Key…" opens.
    let mut page = Page::new(
        SettingsPage::Hotkeys,
        Section::Launcher,
        "Hotkeys",
        PageIcon::Named("keyboard"),
    )
    .group(
        Group::new()
            .row(leader_key(view))
            .row(leader_mode(view, this)),
    );
    for source in sources(view) {
        let has_hotkey = source
            .entries
            .iter()
            .any(|entry| with_hotkey.contains(&entry.item));
        // Applications is open when the page is first shown, whatever it holds.
        let wants_open = has_hotkey || source.key == source_key(APPS);
        page = page.group(
            Group::new()
                .title(source.title)
                .icon(source.icon)
                .columns(NAME, HOTKEY)
                .collapsible(GroupKey(source.key), wants_open)
                .rows(
                    source
                        .entries
                        .iter()
                        .map(|entry| item_row(view, entry, broken)),
                ),
        );
    }
    if let Ok(settings) = &view.file.settings {
        let keys = hotkeys_keys(view);
        let rows: Vec<Row> = dropped_hotkeys(settings)
            .iter()
            .map(|dropped| {
                let key = keys
                    .get(dropped.position)
                    .filter(|key| Hotkey::parse(key).ok().as_ref() == Some(&dropped.hotkey))
                    .cloned();
                dropped_row(view, this, dropped, key).disabled(broken)
            })
            .collect();
        if !rows.is_empty() {
            page = page.group(
                Group::new()
                    .title("Not used")
                    .icon(PageIcon::Named("circle-slash"))
                    .columns(NAME, HOTKEY)
                    .collapsible(GroupKey("not-used".into()), true)
                    .rows(rows),
            );
        }
    }
    page
}

/// Why an entry is not used: `short`, one muted line cut with an ellipsis at the Hotkey
/// column, and `full`, the whole sentence, as its tooltip and its accessible name (said once:
/// the short line is not told again).
fn reason_line(short: &str, full: &str, palette: &theme::Palette) -> AnyElement {
    let shown = SharedString::from(short.to_string());
    let tooltip = SharedString::from(capitalized(full.strip_prefix("Not used: ").unwrap_or(full)));
    div()
        .id("why")
        .aria_label(SharedString::from(full.to_string()))
        .test_support()
        .min_w_0()
        .truncate()
        .text_size(theme::text(theme::SMALL_TEXT_SIZE))
        .text_color(palette.muted)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(shown.clone())
        .test_text(shown)
        .into_any_element()
}

/// Why `dropped` (of `title`, on `hotkey`) is not used: a short form that fits under its
/// title, and the whole sentence. Items are named by their titles.
fn reasons(
    view: &SettingsWindow,
    dropped: &DroppedHotkey,
    title: &str,
    hotkey: &str,
) -> (String, String) {
    // With a modifier leader in hold mode another hotkey can be the same keys without being
    // equal (Leader+F is Ctrl+F): then the other's own hotkey is named.
    let settings = view.file.settings.as_ref().ok();
    let theirs_differ = |theirs: Option<Hotkey>| theirs.filter(|theirs| *theirs != dropped.hotkey);
    match &dropped.reason {
        DropReason::ItemHasOne(kept) => (
            format!("Already has {}", kept.label()),
            format!("Not used: {title} already has {}", kept.label()),
        ),
        DropReason::Refused(Refusal::TakenBy(other)) => {
            let theirs = theirs_differ(settings.and_then(|settings| {
                item_hotkeys(settings)
                    .0
                    .into_iter()
                    .find(|(item, _)| item == other)
                    .map(|(_, theirs)| theirs)
            }));
            let other = view.item_title(other);
            match theirs {
                Some(theirs) => (
                    format!("Same keys as {other}'s"),
                    format!(
                        "Not used: {hotkey} is the same keys as {other}'s {}",
                        theirs.label()
                    ),
                ),
                None => (
                    format!("Used by {other}"),
                    format!("Not used: {other} already has {hotkey}"),
                ),
            }
        }
        DropReason::Refused(Refusal::LauncherHotkey) => {
            match theirs_differ(settings.map(|settings| settings.launcher_hotkey.clone())) {
                Some(theirs) => (
                    "Same keys as the launcher's".to_string(),
                    format!(
                        "Not used: {hotkey} is the same keys as the launcher's {}",
                        theirs.label()
                    ),
                ),
                None => (
                    "The launcher's hotkey".to_string(),
                    format!("Not used: the launcher already has {hotkey}"),
                ),
            }
        }
        DropReason::Refused(Refusal::LeaderHotkey) => (
            "The leader key".to_string(),
            format!("Not used: {}", Refusal::LeaderHotkey),
        ),
        DropReason::Refused(Refusal::LeaderKey) => (
            "The leader's own key".to_string(),
            format!("Not used: {}", Refusal::LeaderKey),
        ),
        DropReason::Refused(Refusal::NoModifier) => (
            "Needs Ctrl, Alt or Super".to_string(),
            format!("Not used: {}", Refusal::NoModifier),
        ),
        DropReason::Refused(other) => {
            let text = other.text(&|item| Some(view.item_title(item)));
            (
                text.strip_suffix('.').unwrap_or(&text).to_string(),
                format!("Not used: {text}"),
            )
        }
    }
}

/// `text` with its first letter in capitals.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// An item a hotkey can open, as its row shows it.
pub(super) struct Entry {
    pub(super) item: ItemId,
    pub(super) title: String,
    pub(super) icon: Option<Icon>,
    /// Not there now (an item with a hotkey that is not indexed): greyed.
    pub(super) gone: bool,
}

impl Entry {
    /// A command a plugin declares.
    pub(super) fn command(plugin: &str, command: &CatalogCommand) -> Entry {
        Entry {
            item: ItemId::new(plugin, &command.id),
            title: command.title.clone(),
            icon: command.icon.clone().map(Icon::Named),
            gone: false,
        }
    }
}

/// The key of the group of the source `provider`.
fn source_key(provider: &str) -> String {
    format!("source:{provider}")
}

/// Where items come from, as a group of the page.
struct Source {
    key: String,
    title: String,
    icon: PageIcon,
    entries: Vec<Entry>,
}

/// Which kind of source, in the page's order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Applications,
    Module,
    Plugin,
    Other,
    NotAvailable,
}

/// Every item a hotkey can open (the indexed ones, the commands the plugins declare, the
/// items the file gives a hotkey), by where they come from, in the page's order, each
/// group's items with a hotkey first, then by title.
fn sources(view: &SettingsWindow) -> Vec<Source> {
    let mut by_provider: HashMap<String, Vec<Entry>> = HashMap::new();
    // The name a plugin gives its items, for a plugin the catalog does not list (yet).
    let mut source_names: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<ItemId> = HashSet::new();
    let mut add = |entry: Entry, by_provider: &mut HashMap<String, Vec<Entry>>| {
        if seen.insert(entry.item.clone()) {
            by_provider
                .entry(entry.item.provider().to_string())
                .or_default()
                .push(entry);
        }
    };
    if let Ok(core) = view.core().try_borrow() {
        for item in core.indexed_items() {
            if let Some(name) = &item.source {
                source_names
                    .entry(item.id.provider().to_string())
                    .or_insert_with(|| name.clone());
            }
            let entry = Entry {
                item: item.id.clone(),
                title: item.title.clone(),
                icon: item.icon.clone(),
                gone: false,
            };
            add(entry, &mut by_provider);
        }
    }
    for plugin in &view.catalog.plugins {
        for command in &plugin.commands {
            add(Entry::command(&plugin.id, command), &mut by_provider);
        }
    }
    for item in view.hotkey_items() {
        let entry = Entry {
            title: gone_title(&item),
            item,
            icon: None,
            gone: true,
        };
        add(entry, &mut by_provider);
    }
    let mut sources: Vec<(Kind, Source)> = Vec::new();
    let mut not_available = Vec::new();
    let mut other = Vec::new();
    for (provider, entries) in by_provider {
        let plugin = view.catalog.plugins.iter().find(|p| p.id == provider);
        let module = MODULES.iter().find(|(id, _, _)| *id == provider);
        let there = entries.iter().any(|entry| !entry.gone);
        let (kind, title, icon) = if provider == APPS {
            (
                Kind::Applications,
                "Applications".to_string(),
                PageIcon::Named("app-window"),
            )
        } else if let Some(plugin) = plugin {
            (Kind::Plugin, plugin.name.clone(), plugin_icon(plugin))
        } else if let Some((_, name, icon)) = module {
            (Kind::Module, name.to_string(), PageIcon::Named(icon))
        } else if let Some(name) = source_names.remove(&provider).filter(|_| there) {
            // A plugin the catalog does not list (yet): by the name it gives its items, with
            // the icon of its first item that has one.
            let icon = entries.iter().find_map(|entry| entry.icon.clone());
            let monogram = name.clone();
            (Kind::Plugin, name, PageIcon::Item { icon, monogram })
        } else if there {
            // A source with no name to show: under "Other".
            other.extend(entries);
            continue;
        } else {
            not_available.extend(entries);
            continue;
        };
        sources.push((
            kind,
            Source {
                key: source_key(&provider),
                title,
                icon,
                entries,
            },
        ));
    }
    if !other.is_empty() {
        sources.push((
            Kind::Other,
            Source {
                key: "other".into(),
                title: "Other".into(),
                icon: PageIcon::Named("blocks"),
                entries: other,
            },
        ));
    }
    if !not_available.is_empty() {
        sources.push((
            Kind::NotAvailable,
            Source {
                key: "not-available".into(),
                title: "Not available".into(),
                icon: PageIcon::Named("unplug"),
                entries: not_available,
            },
        ));
    }
    sources.sort_by(|(a, x), (b, y)| {
        a.cmp(b)
            .then_with(|| x.title.to_lowercase().cmp(&y.title.to_lowercase()))
            .then_with(|| x.key.cmp(&y.key))
    });
    sources
        .into_iter()
        .map(|(_, mut source)| {
            source.entries.sort_by(|a, b| {
                a.title
                    .to_lowercase()
                    .cmp(&b.title.to_lowercase())
                    .then_with(|| a.item.as_str().cmp(b.item.as_str()))
            });
            bound_first(view, &mut source.entries, |entry| &entry.item);
            source
        })
        .collect()
}

/// Puts the rows of items with a hotkey before the others, each part in the order it had.
pub(super) fn bound_first<T>(view: &SettingsWindow, rows: &mut [T], item: impl Fn(&T) -> &ItemId) {
    rows.sort_by_key(|row| !view.listed_first.contains(item(row)));
}

/// The keys of the file's `[hotkeys]` as written, in the order [`Settings::hotkeys`] holds
/// them (both read the table as a `toml::Table`).
///
/// [`Settings::hotkeys`]: tomakast_core::Settings::hotkeys
fn hotkeys_keys(view: &SettingsWindow) -> Vec<String> {
    view.file
        .file
        .as_ref()
        .and_then(|file| file.get("hotkeys"))
        .and_then(|table| table.as_table())
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default()
}

/// A line of a hotkey table: the item's icon and title, and its hotkey field under the
/// "Hotkey" column; greyed when the item is not there now. The field's × clears the hotkey.
pub(super) fn item_row(view: &SettingsWindow, entry: &Entry, disabled: bool) -> Row {
    let field = view.items.get(&entry.item).cloned();
    let title = entry.title.clone();
    let icon = entry.icon.clone();
    let gone = entry.gone;
    Row::custom(move |_, _, cx| {
        let palette = theme::palette(cx);
        let spec = IconSpec {
            icon: icon.as_ref(),
            monogram: &title,
        };
        let name = div()
            .id(if gone { "item-gone" } else { "item" })
            .test_support()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .when(gone, |name| name.opacity(theme::DISABLED_OPACITY))
            .child(
                div()
                    .flex_none()
                    .child(icon_element(spec, ICON_SIZE, &palette)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(theme::text(theme::ROW_TEXT_SIZE))
                    .text_color(palette.text)
                    .child(title.clone()),
            );
        // Until the window makes the field (the first draw of the row), its room.
        let control = match &field {
            Some(field) => field.clone().into_any_element(),
            None => div().h(px(theme::CONTROL_HEIGHT)).into_any_element(),
        };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(theme::LABEL_GAP))
            .child(name)
            .child(
                div()
                    .flex_none()
                    .w(px(theme::settings_control_width()))
                    .child(control),
            )
    })
    .keywords([entry.title.clone(), entry.item.as_str().to_string()])
    .hotkey_of(entry.item.clone())
    .fixed_height(theme::SETTINGS_ROW_HEIGHT)
    .disabled(disabled)
}

/// A `[hotkeys]` entry the launcher does not use: greyed, with why under its title, its
/// hotkey under the "Hotkey" column and a remove button that takes out exactly that entry
/// (`key`, as the file writes it).
fn dropped_row(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    dropped: &DroppedHotkey,
    key: Option<String>,
) -> Row {
    let title = view.item_title(&dropped.item);
    let hotkey = dropped.hotkey.label();
    let (short_why, why) = reasons(view, dropped, &title, &hotkey);
    let error = key
        .as_ref()
        .and_then(|key| view.errors.get(&format!("hotkeys.{key}")).cloned());
    let icon = view
        .core()
        .try_borrow()
        .ok()
        .and_then(|core| core.indexed_item(&dropped.item))
        .and_then(|item| item.icon.clone());
    let remove_label = format!("Remove {hotkey} of {title}");
    // The file's form too: a search for what the file says finds the entry.
    let keywords = [title.clone(), hotkey.clone(), dropped.hotkey.to_string()];
    let this = this.clone();
    Row::custom(move |options, _, cx| {
        let palette = theme::palette(cx);
        let remove = key.clone().map(|key| {
            let this = this.clone();
            control_button("remove", &palette, cx)
                .label("Remove")
                .accessibility_label(remove_label.clone())
                .disabled(options.disabled)
                .on_click(move |_, window, cx| {
                    let _ = this.update(cx, |view, cx| {
                        view.set_key(&["hotkeys", &key], None, window, cx)
                    });
                })
        });
        let spec = IconSpec {
            icon: icon.as_ref(),
            monogram: &title,
        };
        let name = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::SETTINGS_DESCRIPTION_GAP))
            .child(
                div()
                    .id("item-dropped")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap(px(DROPPED_ICON_GAP))
                    .opacity(theme::DISABLED_OPACITY)
                    .child(
                        div()
                            .flex_none()
                            .child(icon_element(spec, ICON_SIZE, &palette)),
                    )
                    .child(
                        div()
                            .id("dropped-title")
                            .test_support()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::text(theme::ROW_TEXT_SIZE))
                            .text_color(palette.text)
                            .child(title.clone()),
                    ),
            )
            .child(
                // Under the title's text, past the icon and its gap.
                div()
                    .min_w_0()
                    .pl(px(ICON_SIZE + DROPPED_ICON_GAP))
                    .child(reason_line(&short_why, &why, &palette)),
            )
            .children(
                error
                    .clone()
                    .map(|error| error_line("remove-error".into(), error, &palette)),
            );
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(theme::LABEL_GAP))
            .child(name)
            .child(
                div()
                    .flex_none()
                    .w(px(theme::settings_control_width()))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        // Where the keys in a hotkey field start, under the column's title.
                        div().min_w_0().pl(px(theme::CONTROL_TEXT_INSET)).child(
                            div()
                                .id("dropped-hotkey")
                                .test_support()
                                .truncate()
                                .text_size(theme::text(theme::MUTED_TEXT_SIZE))
                                .text_color(palette.muted)
                                .child(hotkey.clone()),
                        ),
                    )
                    .children(remove),
            )
    })
    .keywords(keywords)
}
