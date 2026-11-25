//! The settings window's view: the file as last read, the fields that keep typed values, and
//! the pages built from both (again only once what they are built from changes, not on every
//! frame), drawn in the window's own layout (sidebar with search, page area). Every write
//! reads the file's current text, applies one edit and writes it atomically; the window never
//! writes its model back.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName, window_corner_radii};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestSupportExt as _, Window, div, px,
};
use tomakast_core::hotkeys::{HotkeyOrigin, HotkeyTable, HotkeyTarget, item_hotkeys};
use tomakast_core::settings::home_rows;
use tomakast_core::settings_edit::{remove_plugin_table, set_plugin_preference, write_config};
use tomakast_core::settings_schema::{KeyValue, SettingKey, ValueKind, builtin_keys, read_key};
use tomakast_core::{
    Core, HomeEdit, HomeEntry, HomeWriteError, Hotkey, ItemId, PluginCatalog, Settings, edit_home,
};

use super::conflict::{self, Answer, Conflict};
use super::fields::{
    AUTOMATIC, Commit, FieldKind, HotkeyCheck, HotkeyClear, HotkeyCommit, HotkeyField, PickField,
    PickItem, PickSpec, TextField, TextKind, city_items, computer_zone, zone_items,
};
use super::frame::Frame;
use super::layout::{self, OnSelect, OnToggle, OpenGroups, Page, Viewports, search};
use super::opener::ConfigOpener;
use super::pages::{
    command_hotkey, hotkey_stand, page_keys, pick_file_value, pick_list, unset_text, value_text,
};
use super::{SettingsPage, SharedRequests, pages};
use crate::theme;
use crate::views::hotkey_recorder::refusal_text;

/// The banner's first line while the file has an error; the error itself is under it.
const BANNER_TITLE: &str = "config.toml has an error. Fix the file; nothing is written until then.";

/// `config.toml` as last read.
pub(crate) struct FileState {
    pub(crate) text: String,
    /// The error when the launcher cannot load the file; the window then writes nothing.
    pub(crate) settings: Result<Settings, String>,
    /// The file as a TOML table, when it is valid TOML.
    pub(crate) file: Option<toml::Table>,
}

impl FileState {
    fn read(path: &Path) -> FileState {
        match read_text(path) {
            Ok(text) => FileState {
                settings: Settings::parse(&text).map_err(|e| format!("{e:#}")),
                file: text.parse().ok(),
                text,
            },
            Err(e) => FileState {
                text: String::new(),
                settings: Err(format!("{e:#}")),
                file: None,
            },
        }
    }

    /// Why nothing can be written, if nothing can.
    pub(crate) fn error(&self) -> Option<&str> {
        self.settings.as_ref().err().map(String::as_str)
    }

    /// `key`'s value in the file.
    pub(crate) fn value(&self, key: &SettingKey) -> KeyValue {
        self.file
            .as_ref()
            .map_or(KeyValue::Missing, |file| read_key(file, key))
    }
}

/// The file's text; a missing file is an empty one.
fn read_text(path: &Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(anyhow::Error::new(e).context(format!("reading {}", path.display()))),
    }
}

/// The schema key at `path`.
pub(crate) fn key(path: &[&str]) -> &'static SettingKey {
    builtin_keys()
        .iter()
        .find(|k| k.path == path)
        .expect("a built-in key")
}

/// What the controller hands a new window.
pub(crate) struct WindowInit {
    pub(crate) config_file: PathBuf,
    pub(crate) core: Rc<RefCell<Core>>,
    pub(crate) catalog: PluginCatalog,
    pub(crate) catalog_known: bool,
    pub(crate) table: HotkeyTable,
    pub(crate) requests: SharedRequests,
    pub(crate) opener: Option<Arc<dyn ConfigOpener>>,
    pub(crate) page: SettingsPage,
    pub(crate) last_page: Rc<RefCell<SettingsPage>>,
    /// The computer's time zone as the operating system names it, read when the window
    /// opens; `None` when it cannot be told.
    pub(crate) time_zone: Option<String>,
}

pub struct SettingsWindow {
    config_file: PathBuf,
    pub(crate) file: FileState,
    /// The text of the window's last write: a change event with exactly this content is its
    /// own write.
    last_written: Option<String>,
    pub(crate) catalog: PluginCatalog,
    /// Whether the plugin manager has listed the plugins yet: until then a `[plugins.<id>]`
    /// table may be a plugin's that is still there.
    pub(crate) catalog_known: bool,
    pub(crate) table: Rc<RefCell<HotkeyTable>>,
    core: Rc<RefCell<Core>>,
    requests: SharedRequests,
    opener: Option<Arc<dyn ConfigOpener>>,
    /// The page on display; while a search keeps nothing of it, the first page it keeps is
    /// shown instead.
    shown: SettingsPage,
    /// The page the last draw put on display: `shown`, or the first page the search kept in its
    /// place. Clearing the query makes it `shown`.
    displayed: SettingsPage,
    /// The settings page on display, which the controller opens on next time.
    pub(crate) last_page: Rc<RefCell<SettingsPage>>,
    /// A plugin page asked for before the catalog listed the plugin, and the page on display
    /// then: once the plugin is listed the window goes there, unless the user moved on.
    pending_plugin: Option<(String, SettingsPage)>,
    /// The search over the settings, at the top of the sidebar.
    search: Entity<InputState>,
    /// The title area's window controls and how it moves the window.
    frame: Frame,
    /// What the window's own background holds: a click on empty space takes the focus here,
    /// which ends an edit in a field as leaving it does.
    root_focus: FocusHandle,
    /// Why a write did not happen, by key path (`launcher.autostart`).
    pub(crate) errors: HashMap<String, String>,
    /// The last word on a hotkey change asked for here, and whose change it was.
    pub(crate) hotkey_notice: Option<(HotkeyTarget, String)>,
    pub(crate) hotkey: Entity<HotkeyField>,
    /// The leader key's field.
    pub(crate) leader: Entity<HotkeyField>,
    pub(crate) city: Entity<PickField>,
    /// `[location] timezone`: "Automatic" (the computer's zone), then every zone.
    pub(crate) time_zone: Entity<PickField>,
    /// `[appearance] font` and `mono_font`: the desktop's font, then every installed family
    /// and the bundled one, alphabetically.
    pub(crate) font: Entity<PickField>,
    pub(crate) mono_font: Entity<PickField>,
    /// `[appearance] mono_size`.
    pub(crate) mono_size: Entity<TextField>,
    /// The built-in choice keys ([`CHOICES`]), in that order.
    choices: Vec<(&'static [&'static str], Entity<PickField>)>,
    /// Text and number fields of plugin preferences, by plugin id and key.
    pub(crate) prefs: HashMap<(String, String), Entity<TextField>>,
    /// Pick fields of plugin preferences (a time zone, a choice of many options), by plugin
    /// id and key.
    pub(crate) picks: HashMap<(String, String), Entity<PickField>>,
    /// The size lists of the Home widgets, by tile, with what writes their picks.
    home_sizes: HashMap<usize, (Entity<PickField>, Subscription)>,
    /// The zone an unset time zone stands for: the computer's, read when the window opened.
    zone: String,
    /// Hotkey fields of items: plugin commands, the items the file gives a hotkey, and the
    /// items of the Hotkeys page's groups once drawn (kept while they are indexed). One field
    /// per item, whichever page shows it.
    pub(crate) items: HashMap<ItemId, Entity<HotkeyField>>,
    /// The items whose rows the hotkey tables list first, as having a hotkey: the file's, kept
    /// as they were while the user is in one of the items' fields, so no row moves under them.
    pub(crate) listed_first: HashSet<ItemId>,
    /// Which groups that open and close are open.
    open_groups: OpenGroups,
    /// How far each page is scrolled, and where its long tables are.
    viewports: Viewports,
    /// Fields whose typed value an outside edit changed, in the order found: the first is
    /// asked about, the others wait. Each is held (writes nothing) until answered.
    pub(crate) conflicts: VecDeque<Conflict>,
    /// Whether the question about the first is open.
    asking: bool,
    /// Fields whose question was closed without an answer: held while other questions are
    /// open, since each takes the focus back and forth.
    later: Vec<Entity<TextField>>,
    /// Kept with the field they listen to.
    field_subscriptions: HashMap<FieldId, Subscription>,
    /// The Home widget last lifted by a drag: dimmed while a drag is on.
    pub(crate) lifted_widget: Rc<Cell<Option<usize>>>,
    /// Counts the changes of what the pages are built from besides [`PageInputs`]'s other
    /// parts: the file, the catalog and the fields the window made.
    generation: u64,
    /// The pages as last built, until what they are built from changes.
    built: Option<BuiltPages>,
    /// How many times the pages were built, for tests.
    pub(crate) page_builds: usize,
    /// How many times the rows a query opens groups on were read, for tests.
    pub(crate) search_reads: usize,
    _subscriptions: Vec<Subscription>,
}

/// A field the window made for the catalog.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FieldId {
    Preference(String, String),
    Pick(String, String),
    Item(ItemId),
    /// The clear button of an item's field.
    ItemClear(ItemId),
}

/// What the pages are built from: while it is the same, the pages built last are current.
#[derive(PartialEq)]
struct PageInputs {
    /// [`SettingsWindow::generation`]: the file, the catalog, the fields made.
    generation: u64,
    /// The core's index generation; `None` while the core is busy, when nothing indexed can
    /// be read: such pages are built again on the next frame.
    index: Option<u64>,
    errors: HashMap<String, String>,
    listed_first: HashSet<ItemId>,
    /// How many searches are saved (the Launcher page tells it).
    history: usize,
    palette: theme::Palette,
}

/// The pages as built from [`PageInputs`], with what search reads of them and what the last
/// query kept.
struct BuiltPages {
    inputs: PageInputs,
    keys: Vec<SettingsPage>,
    pages: Vec<Page>,
    texts: Vec<search::PageText>,
    matches: Option<(SharedString, Vec<Option<Vec<usize>>>)>,
    /// The rows the last query opens each group on.
    revealed: Option<(SharedString, Rc<search::Revealed>)>,
}

impl BuiltPages {
    /// For each page, the indices of the groups `query` keeps, or `None`.
    fn matches(&mut self, query: &SharedString) -> Vec<Option<Vec<usize>>> {
        match &self.matches {
            Some((last, matches)) if last == query => matches.clone(),
            _ => {
                let matches = search::filter(&self.texts, query);
                self.matches = Some((query.clone(), matches.clone()));
                matches
            }
        }
    }

    /// For each page, the rows `query` opens each of its groups on; `reads` counts the times
    /// they are read from the pages' text.
    fn revealed(&mut self, query: &SharedString, reads: &mut usize) -> Rc<search::Revealed> {
        match &self.revealed {
            Some((last, revealed)) if last == query => revealed.clone(),
            _ => {
                *reads += 1;
                let revealed = Rc::new(search::revealed(&self.texts, query));
                self.revealed = Some((query.clone(), revealed.clone()));
                revealed
            }
        }
    }
}

/// Where the error of a write to Home widget `tile` shows.
pub(crate) fn home_error_key(tile: usize) -> String {
    format!("home.widgets.{tile}")
}

/// The search field's accessible name and placeholder.
const SEARCH_LABEL: &str = "Search...";
const SEARCH_PLACEHOLDER: &str = "Search…";

/// Key paths of the fields the window owns.
pub(crate) const LAUNCHER_HOTKEY: &[&str] = &["launcher", "hotkey"];
pub(crate) const LEADER: &[&str] = &["launcher", "leader"];
pub(crate) const CITY: &[&str] = &["location", "city"];
pub(crate) const TIME_ZONE: &[&str] = &["location", "timezone"];
pub(crate) const FONT: &[&str] = &["appearance", "font"];
pub(crate) const MONO_FONT: &[&str] = &["appearance", "mono_font"];
pub(crate) const MONO_SIZE: &[&str] = &["appearance", "mono_size"];
pub(crate) const THEME: &[&str] = &["appearance", "theme"];
pub(crate) const TEXT_SIZE: &[&str] = &["appearance", "text_size"];
pub(crate) const POP_TO_ROOT: &[&str] = &["launcher", "pop_to_root"];
pub(crate) const SCREEN: &[&str] = &["launcher", "screen"];
pub(crate) const LEADER_MODE: &[&str] = &["launcher", "leader_mode"];
/// Each theme's icon: the computer's setting, the sun, the moon.
pub(crate) const THEME_ICONS: &[(&str, &str)] =
    &[("system", "monitor"), ("light", "sun"), ("dark", "moon")];
/// The built-in choice keys, each a list field of the window.
const CHOICES: [&[&str]; 5] = [THEME, TEXT_SIZE, POP_TO_ROOT, SCREEN, LEADER_MODE];
/// Where the Config file row's error shows in `errors`.
pub(crate) const CONFIG_FILE: &str = "config-file";
/// The key of the error of clearing the search history.
pub(crate) const HISTORY_CLEAR: &str = "history-clear";

impl SettingsWindow {
    pub(crate) fn new(init: WindowInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        theme::sync(window, cx);
        let file = FileState::read(&init.config_file);
        let table = Rc::new(RefCell::new(init.table));
        let check = hotkey_check(table.clone(), init.core.clone(), HotkeyTarget::Launcher);
        let hotkey_key = key(LAUNCHER_HOTKEY);
        let hotkey_value = file_hotkey(&file);
        let hotkey = cx.new(|cx| {
            let table = table.clone();
            HotkeyField::new(
                hotkey_key.title,
                hotkey_value,
                check,
                table,
                FieldKind::Launcher,
                window,
                cx,
            )
            .with_default(default_hotkey())
        });
        let check = hotkey_check(table.clone(), init.core.clone(), HotkeyTarget::Leader);
        let leader_value = file_leader(&file);
        let leader = cx.new(|cx| {
            let table = table.clone();
            HotkeyField::new(
                key(LEADER).title,
                leader_value,
                check,
                table,
                FieldKind::Leader,
                window,
                cx,
            )
        });
        let (city_value, city_invalid) = file_text(&file, CITY);
        let city = cx.new(|cx| {
            let spec = PickSpec {
                id: "city".into(),
                title: key(CITY).title.into(),
                search_placeholder: "Search cities…".into(),
                items: city_items(),
                unset: AUTOMATIC.into(),
            };
            // Unset, the city is "Automatic", the list's first entry.
            PickField::new(
                spec,
                city_value,
                city_invalid,
                Some(String::new()),
                window,
                cx,
            )
        });
        let zone = computer_zone(init.time_zone.as_deref());
        let (zone_value, zone_invalid) = file_text(&file, TIME_ZONE);
        let time_zone = cx.new(|cx| {
            let spec = PickSpec {
                id: "time-zone".into(),
                title: key(TIME_ZONE).title.into(),
                search_placeholder: "Search time zones…".into(),
                items: automatic_zone_items(&zone),
                unset: automatic_zone(&zone).into(),
            };
            PickField::new(
                spec,
                zone_value,
                zone_invalid,
                Some(String::new()),
                window,
                cx,
            )
        });
        let families = installed_families(cx);
        let font = font_field(FONT, &families, &file, window, cx);
        let mono_font = font_field(MONO_FONT, &families, &file, window, cx);
        let choices: Vec<_> = CHOICES
            .into_iter()
            .map(|path| (path, choice_field(path, &file, window, cx)))
            .collect();
        let mono_size = {
            let ValueKind::Number(range) = key(MONO_SIZE).kind else {
                unreachable!("the monospace size is a number");
            };
            let value = file_number(&file, MONO_SIZE);
            cx.new(|cx| {
                TextField::new(
                    TextKind::Number(range),
                    key(MONO_SIZE).title,
                    value,
                    window,
                    cx,
                )
            })
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH_PLACEHOLDER));
        let mut subscriptions = vec![
            cx.subscribe_in(&hotkey, window, |this, _, HotkeyCommit(hotkey), _, cx| {
                this.request_hotkey(HotkeyTarget::Launcher, Some(hotkey.clone()), cx);
            }),
            cx.subscribe_in(&hotkey, window, |this, _, HotkeyClear, _, cx| {
                this.request_hotkey(HotkeyTarget::Launcher, None, cx);
            }),
            cx.subscribe_in(&leader, window, |this, _, HotkeyCommit(hotkey), _, cx| {
                this.request_hotkey(HotkeyTarget::Leader, Some(hotkey.clone()), cx);
            }),
            cx.subscribe_in(&leader, window, |this, _, HotkeyClear, _, cx| {
                this.request_hotkey(HotkeyTarget::Leader, None, cx);
            }),
            cx.subscribe_in(&city, window, |this, _, Commit(value), window, cx| {
                let value = value.clone();
                this.set_key(CITY, value, window, cx);
            }),
            cx.subscribe_in(&time_zone, window, |this, _, Commit(value), window, cx| {
                let value = value.clone();
                this.set_key(TIME_ZONE, value, window, cx);
            }),
            cx.subscribe_in(&font, window, |this, _, Commit(value), window, cx| {
                let value = value.clone();
                this.set_key(FONT, value, window, cx);
            }),
            cx.subscribe_in(&mono_font, window, |this, _, Commit(value), window, cx| {
                let value = value.clone();
                this.set_key(MONO_FONT, value, window, cx);
            }),
            cx.subscribe_in(&mono_size, window, |this, _, Commit(value), window, cx| {
                let value = value.clone();
                this.set_key(MONO_SIZE, value, window, cx);
            }),
            cx.observe_window_appearance(window, |_, window, cx| theme::sync(window, cx)),
            cx.subscribe_in(&search, window, |view, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    view.open_groups.search_changed();
                    if view.search.read(cx).value().is_empty() {
                        view.keep_displayed_page();
                    }
                    cx.notify();
                }
            }),
        ];
        subscriptions.extend(choices.iter().map(|(path, field)| {
            let path = *path;
            cx.subscribe_in(field, window, move |this, _, Commit(value), window, cx| {
                let value = value.clone();
                this.set_key(path, value, window, cx);
            })
        }));
        let keys = page_keys(&init.catalog, init.catalog_known, file.file.as_ref());
        let last = init.last_page.borrow().clone();
        // A page that is not there (a plugin not listed yet): the page last on display, and
        // the plugin's once it is listed.
        let (page, pending_plugin) = if keys.contains(&init.page) {
            (init.page, None)
        } else {
            let page = if keys.contains(&last) {
                last
            } else {
                SettingsPage::General
            };
            let pending = match init.page {
                SettingsPage::Plugin(id) => Some((id, page.clone())),
                _ => None,
            };
            (page, pending)
        };
        *init.last_page.borrow_mut() = page.clone();
        let mut view = SettingsWindow {
            config_file: init.config_file,
            file,
            last_written: None,
            catalog: init.catalog,
            catalog_known: init.catalog_known,
            table,
            core: init.core,
            requests: init.requests,
            opener: init.opener,
            displayed: page.clone(),
            shown: page,
            last_page: init.last_page,
            pending_plugin,
            search,
            frame: Frame::new(cx),
            root_focus: cx.focus_handle(),
            errors: HashMap::new(),
            hotkey_notice: None,
            hotkey,
            leader,
            city,
            time_zone,
            font,
            mono_font,
            mono_size,
            choices,
            prefs: HashMap::new(),
            picks: HashMap::new(),
            home_sizes: HashMap::new(),
            zone,
            items: HashMap::new(),
            listed_first: HashSet::new(),
            open_groups: OpenGroups::default(),
            viewports: Viewports::default(),
            conflicts: VecDeque::new(),
            asking: false,
            later: Vec::new(),
            field_subscriptions: HashMap::new(),
            lifted_widget: Rc::default(),
            generation: 0,
            built: None,
            page_builds: 0,
            search_reads: 0,
            _subscriptions: subscriptions,
        };
        view.sync_fields(window, cx);
        view.apply_disabled(cx);
        view.listed_first = view.hotkey_items().into_iter().collect();
        view
    }

    pub(crate) fn config_path(&self) -> &Path {
        &self.config_file
    }

    /// The list field of the built-in choice key at `path` (one of [`CHOICES`]).
    pub(crate) fn choice(&self, path: &[&str]) -> Entity<PickField> {
        self.choices
            .iter()
            .find(|(of, _)| *of == path)
            .map(|(_, field)| field.clone())
            .unwrap_or_else(|| unreachable!("{path:?} is a built-in choice"))
    }

    /// How many searches the launcher keeps; 0 while the core is busy.
    pub(crate) fn history_len(&self) -> usize {
        self.core
            .try_borrow()
            .map(|core| core.history().len())
            .unwrap_or_default()
    }

    /// Deletes every saved search; a failure is told under the button.
    pub(crate) fn clear_history(&mut self, cx: &mut Context<Self>) {
        let history = self.core.borrow().history().clone();
        match history.clear() {
            Ok(()) => {
                self.errors.remove(HISTORY_CLEAR);
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot clear the search history");
                self.errors.insert(
                    HISTORY_CLEAR.to_string(),
                    format!("Could not clear the history: {e:#}"),
                );
            }
        }
        cx.notify();
    }

    /// The `[launcher] screen` choices this desktop can honour, by value; none hides the row.
    pub(crate) fn screen_choices(&self) -> Vec<&'static str> {
        tomakast_platform::screen_choices()
            .iter()
            .map(|choice| choice.value())
            .collect()
    }

    /// The window's pages now, in sidebar order.
    fn page_keys(&self) -> Vec<SettingsPage> {
        page_keys(&self.catalog, self.catalog_known, self.file.file.as_ref())
    }

    /// Switches to `page`. A plugin page that is not there yet is opened once the catalog
    /// lists the plugin.
    fn show_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        if !self.page_keys().contains(&page) {
            if let SettingsPage::Plugin(id) = page {
                self.pending_plugin = Some((id, self.shown.clone()));
            }
            return;
        }
        self.pending_plugin = None;
        *self.last_page.borrow_mut() = page.clone();
        self.shown = page;
        cx.notify();
    }

    /// Switches to `page` as asked from outside the window (the launcher): a search would hide
    /// it, so the query is cleared.
    pub(crate) fn open_page(
        &mut self,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Setting the value sends no change event: the window redraws for the cleared query
        // here, even when the page is not there yet.
        if !self.search.read(cx).value().is_empty() {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            cx.notify();
        }
        self.show_page(page, cx);
    }

    /// The pages changed: a plugin page asked for earlier opens once it is there, and a page
    /// gone (its table removed) gives way to General.
    fn follow_pages(&mut self, cx: &mut Context<Self>) {
        let pages = self.page_keys();
        if let Some((plugin, asked_on)) = self.pending_plugin.clone() {
            let asked = SettingsPage::Plugin(plugin);
            if asked_on != self.shown {
                self.pending_plugin = None;
            } else if pages.contains(&asked) {
                self.show_page(asked, cx);
                return;
            }
        }
        if !pages.contains(&self.shown) {
            self.shown = SettingsPage::General;
            *self.last_page.borrow_mut() = SettingsPage::General;
        }
    }

    pub(crate) fn set_catalog(
        &mut self,
        catalog: PluginCatalog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalog = catalog;
        self.catalog_known = true;
        self.generation += 1;
        self.sync_fields(window, cx);
        self.apply_disabled(cx);
        self.follow_pages(cx);
        cx.notify();
    }

    /// The query is cleared: the page that was on display stays, even when the query had put
    /// another in place of the chosen one.
    fn keep_displayed_page(&mut self) {
        if self.displayed != self.shown && self.page_keys().contains(&self.displayed) {
            self.shown = self.displayed.clone();
            *self.last_page.borrow_mut() = self.displayed.clone();
        }
    }

    /// Esc in the search field: clears a query, or, with none, goes on to the window.
    fn clear_search(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).value().is_empty() {
            cx.propagate();
            return;
        }
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.keep_displayed_page();
        cx.notify();
    }

    /// The search field, in the launcher form's look.
    fn search_field(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = theme::palette(cx);
        div()
            .w_full()
            .on_action(cx.listener(Self::clear_search))
            .child(
                Input::new(&self.search)
                    .aria_label(SEARCH_LABEL)
                    .prefix(Icon::new(IconName::Search).text_color(palette.muted))
                    .bg(palette.field)
                    .border_color(palette.border),
            )
            .into_any_element()
    }

    /// Makes the fields the catalog and the file call for and shows the file's values in
    /// them. Returns the text fields whose value changed under typed text.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Conflict> {
        let changed = self.sync_preference_fields(window, cx);
        self.sync_pick_fields(window, cx);
        self.sync_home_sizes(window, cx);
        self.sync_item_fields(window, cx);
        changed
    }

    /// The entries of the file's Home the launcher reads, in order.
    pub(crate) fn home_entries(&self) -> Vec<HomeEntry> {
        home_rows(&self.file.text)
            .map(|rows| rows.into_iter().flatten().collect())
            .unwrap_or_default()
    }

    /// Makes a field for every text and number preference of the catalog, keeping those
    /// already there (with what is typed in them) and showing the file's values. Returns the
    /// fields whose value changed under typed text.
    fn sync_preference_fields(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Conflict> {
        let broken = self.file.error().is_some();
        let mut changed = Vec::new();
        let mut prefs = std::mem::take(&mut self.prefs);
        for plugin in self.catalog.plugins.clone() {
            for pref in &plugin.preferences {
                let kind = match &pref.kind {
                    ValueKind::Text => TextKind::Text,
                    ValueKind::Number(range) => TextKind::Number(*range),
                    _ => continue,
                };
                let id = (plugin.id.clone(), pref.key.clone());
                let value = value_text(&self.file, &plugin.id, pref);
                let kept = prefs
                    .remove(&id)
                    .filter(|field| field.read(cx).kind() == kind);
                let field = match kept {
                    Some(field) => {
                        let title = pref.title.clone().into();
                        if show_in_field(&field, title, value, broken, window, cx) {
                            changed.push(Conflict {
                                field: field.clone(),
                            });
                        }
                        field
                    }
                    None => {
                        let title = pref.title.clone();
                        let field = cx.new(|cx| TextField::new(kind, title, value, window, cx));
                        let (plugin, key) = id.clone();
                        let subscription = cx.subscribe_in(
                            &field,
                            window,
                            move |this, _, Commit(value), window, cx| {
                                this.set_preference(&plugin, &key, value.clone(), window, cx);
                            },
                        );
                        self.field_subscriptions.insert(
                            FieldId::Preference(id.0.clone(), id.1.clone()),
                            subscription,
                        );
                        field
                    }
                };
                self.prefs.insert(id, field);
            }
        }
        // The subscriptions of fields no longer there go with them.
        let prefs = &self.prefs;
        self.field_subscriptions.retain(|id, _| match id {
            FieldId::Preference(plugin, key) => prefs.contains_key(&(plugin.clone(), key.clone())),
            _ => true,
        });
        changed
    }

    /// Makes a pick field for every time zone preference of the catalog and every choice of
    /// many options, keeping those already there whose list is the same, and shows the file's
    /// values (or the defaults in effect) in them.
    fn sync_pick_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut picks = std::mem::take(&mut self.picks);
        for plugin in &self.catalog.plugins {
            let unset = SharedString::from(unset_text(plugin));
            for pref in &plugin.preferences {
                let Some(list) = pick_list(pref) else {
                    continue;
                };
                let id = (plugin.id.clone(), pref.key.clone());
                let (value, invalid) = pick_file_value(&self.file, &plugin.id, pref);
                let default = match pref.kind {
                    ValueKind::TimeZone => Some(
                        pref.shown_default()
                            .map_or_else(|| self.zone.clone(), str::to_owned),
                    ),
                    _ => pref.shown_default().map(str::to_owned),
                };
                let kept = picks
                    .remove(&id)
                    .filter(|field| field.read(cx).lists(&list.items));
                let field = match kept {
                    Some(field) => {
                        field.update(cx, |field, cx| {
                            field.set_title(pref.title.clone().into(), cx);
                            field.set_unset(unset.clone(), cx);
                            field.set_from_file(value, invalid, default, window, cx)
                        });
                        field
                    }
                    None => {
                        let spec = PickSpec {
                            id: format!("pick-{}", pref.key).into(),
                            title: pref.title.clone().into(),
                            search_placeholder: list.search_placeholder.into(),
                            items: list.items.into_owned(),
                            unset: unset.clone(),
                        };
                        let field =
                            cx.new(|cx| PickField::new(spec, value, invalid, default, window, cx));
                        let (plugin, key) = id.clone();
                        let subscription = cx.subscribe_in(
                            &field,
                            window,
                            move |this, _, Commit(value), window, cx| {
                                this.set_preference(&plugin, &key, value.clone(), window, cx);
                            },
                        );
                        self.field_subscriptions
                            .insert(FieldId::Pick(id.0.clone(), id.1.clone()), subscription);
                        field
                    }
                };
                self.picks.insert(id, field);
            }
        }
        let picks = &self.picks;
        self.field_subscriptions.retain(|id, _| match id {
            FieldId::Pick(plugin, key) => picks.contains_key(&(plugin.clone(), key.clone())),
            _ => true,
        });
    }

    /// The size list of Home tile `tile`, when its widget has one.
    pub(crate) fn home_size(&self, tile: usize) -> Option<Entity<PickField>> {
        self.home_sizes.get(&tile).map(|(field, _)| field.clone())
    }

    /// Makes a size list for every Home widget an installed plugin offers, keeping those
    /// already there whose list is the same, and shows the file's sizes in them.
    fn sync_home_sizes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut old = std::mem::take(&mut self.home_sizes);
        let broken = self.file.error().is_some();
        for list in crate::settings::pages::size_lists(self) {
            let tile = list.tile;
            let kept = old
                .remove(&tile)
                .filter(|(field, _)| field.read(cx).lists(&list.items));
            let entry = match kept {
                Some((field, subscription)) => {
                    field.update(cx, |field, cx| {
                        field.set_title(list.title.into(), cx);
                        field.set_from_file(
                            list.value,
                            list.invalid,
                            Some(String::new()),
                            window,
                            cx,
                        )
                    });
                    (field, subscription)
                }
                None => {
                    let spec = PickSpec {
                        id: format!("home-size-{tile}").into(),
                        title: list.title.into(),
                        search_placeholder: SEARCH_PLACEHOLDER.into(),
                        items: list.items,
                        unset: SharedString::default(),
                    };
                    let (value, invalid) = (list.value, list.invalid);
                    let field = cx.new(|cx| {
                        PickField::new(spec, value, invalid, Some(String::new()), window, cx)
                    });
                    let subscription = cx.subscribe_in(
                        &field,
                        window,
                        move |this, _, Commit(value), window, cx| {
                            let size = value
                                .as_ref()
                                .and_then(toml::Value::as_str)
                                .and_then(tomakast_core::WidgetSize::parse);
                            let edit = match size {
                                Some(size) => HomeEdit::Resize(tile, size),
                                None => HomeEdit::ClearSize(tile),
                            };
                            this.edit_home(tile, edit, window, cx);
                        },
                    );
                    (field, subscription)
                }
            };
            entry
                .0
                .update(cx, |field, cx| field.set_disabled(broken, cx));
            self.home_sizes.insert(tile, entry);
        }
    }

    /// Makes a hotkey field for every command of the catalog and every item of the file's
    /// `[hotkeys]`, keeps those made for the rows drawn while their item is indexed, and shows
    /// the file's hotkeys in them.
    fn sync_item_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut wanted: Vec<(ItemId, String)> = Vec::new();
        for plugin in &self.catalog.plugins {
            for command in &plugin.commands {
                wanted.push((ItemId::new(&plugin.id, &command.id), command.title.clone()));
            }
        }
        let mut fields = std::mem::take(&mut self.items);
        // While the core is busy nothing can be told about the items: the fields there stay,
        // so a recording in one goes on. The next sync drops those no longer indexed.
        let indexed: Vec<ItemId> = match self.core.try_borrow() {
            Ok(core) => fields
                .keys()
                .filter(|item| core.is_indexed(item))
                .cloned()
                .collect(),
            Err(_) => fields.keys().cloned().collect(),
        };
        for item in self.hotkey_items().into_iter().chain(indexed) {
            if !wanted.iter().any(|(known, _)| *known == item) {
                let title = self.item_title(&item);
                wanted.push((item, title));
            }
        }
        for (item, title) in wanted {
            let value = command_hotkey(&self.file, &item);
            let field = match fields.remove(&item) {
                Some(field) => {
                    field.update(cx, |field, cx| {
                        field.set_title(title.into(), cx);
                        field.set_from_file(value, window, cx);
                    });
                    field
                }
                None => self.new_item_field(&item, title, window, cx),
            };
            self.items.insert(item, field);
        }
        let items = &self.items;
        self.field_subscriptions.retain(|id, _| match id {
            FieldId::Item(item) | FieldId::ItemClear(item) => items.contains_key(item),
            _ => true,
        });
    }

    /// Makes the hotkey fields of `items`, as the rows drawn now call for.
    fn make_item_fields(
        &mut self,
        items: Vec<ItemId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for item in items {
            if self.items.contains_key(&item) {
                continue;
            }
            let title = self.item_title(&item);
            let field = self.new_item_field(&item, title, window, cx);
            self.items.insert(item, field);
            self.generation += 1;
        }
    }

    /// What the pages would be built from now.
    fn page_inputs(&self, cx: &App) -> PageInputs {
        PageInputs {
            generation: self.generation,
            index: self
                .core
                .try_borrow()
                .ok()
                .map(|core| core.index_generation()),
            errors: self.errors.clone(),
            listed_first: self.listed_first.clone(),
            history: self.history_len(),
            palette: theme::palette(cx),
        }
    }

    /// The pages, built again only when what they are built from changed since the last
    /// time.
    fn take_pages(
        &mut self,
        this: &gpui_kit::WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> BuiltPages {
        let inputs = self.page_inputs(cx);
        match self.built.take() {
            // With the core busy (its index unreadable) the pages are built every frame until
            // it can be read again. That is rare: the core is borrowed only in short scopes on
            // this thread (applying a batch of its events, a call into a module), none of them
            // open while a window draws, so only a draw nested inside such a scope meets it.
            // And it is safe: pages built then lack only what the index tells (an item shows
            // by its readable id), and a build after the scope ends puts that back.
            Some(built) if inputs.index.is_some() && built.inputs == inputs => built,
            _ => {
                self.page_builds += 1;
                self.viewports.pages_changed();
                let keys = self.page_keys();
                let pages = pages::build(self, &keys, this, cx);
                let texts = pages.iter().map(Page::text).collect();
                BuiltPages {
                    inputs,
                    keys,
                    pages,
                    texts,
                    matches: None,
                    revealed: None,
                }
            }
        }
    }

    /// A hotkey field of `item`, showing the file's hotkey, that asks for the hotkeys set and
    /// cleared in it.
    fn new_item_field(
        &mut self,
        item: &ItemId,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<HotkeyField> {
        let value = command_hotkey(&self.file, item);
        let target = HotkeyTarget::Item(item.clone());
        let check = hotkey_check(self.table.clone(), self.core.clone(), target.clone());
        let broken = self.file.error().is_some();
        let table = self.table.clone();
        let field = cx.new(|cx| {
            let mut field =
                HotkeyField::new(title, value, check, table, FieldKind::Item, window, cx);
            field.set_disabled(broken, cx);
            field
        });
        let subscription = cx.subscribe_in(&field, window, {
            let target = target.clone();
            move |this, _, HotkeyCommit(hotkey), _, cx| {
                this.request_hotkey(target.clone(), Some(hotkey.clone()), cx);
            }
        });
        self.field_subscriptions
            .insert(FieldId::Item(item.clone()), subscription);
        let subscription = cx.subscribe_in(&field, window, move |this, _, HotkeyClear, _, cx| {
            this.request_hotkey(target.clone(), None, cx);
        });
        self.field_subscriptions
            .insert(FieldId::ItemClear(item.clone()), subscription);
        field
    }

    /// Takes which items the hotkey tables list first from the file, unless the user is in
    /// one of the items' fields: their rows stay where they are until the field is left.
    fn order_hotkey_rows(&mut self, window: &Window, cx: &App) {
        let in_use = self
            .items
            .values()
            .any(|field| field.read(cx).is_in_use(window, cx));
        if !in_use {
            self.listed_first = self.hotkey_items().into_iter().collect();
        }
    }

    /// The items the file's `[hotkeys]` gives a hotkey, in its order.
    pub(crate) fn hotkey_items(&self) -> Vec<ItemId> {
        match &self.file.settings {
            Ok(settings) => item_hotkeys(settings)
                .0
                .into_iter()
                .map(|(item, _)| item)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The title of indexed `item`, or [`gone_title`].
    pub(crate) fn item_title(&self, item: &ItemId) -> String {
        item_title(&self.core, item)
    }

    pub(crate) fn core(&self) -> &Rc<RefCell<Core>> {
        &self.core
    }

    pub(crate) fn set_hotkey_table(&mut self, table: HotkeyTable, cx: &mut Context<Self>) {
        *self.table.borrow_mut() = table;
        cx.notify();
    }

    /// The answer to the change of `target`'s hotkey: shown inside its field, until the next
    /// change is asked for.
    pub(crate) fn hotkey_notice(
        &mut self,
        target: HotkeyTarget,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(field) = self.hotkey_field(&target) {
            field.update(cx, |field, cx| field.drop_request(cx));
        }
        self.hotkey_notice = Some((target, text));
        cx.notify();
    }

    /// The hotkey field of `target`.
    fn hotkey_field(&self, target: &HotkeyTarget) -> Option<Entity<HotkeyField>> {
        match target {
            HotkeyTarget::Launcher => Some(self.hotkey.clone()),
            HotkeyTarget::Leader => Some(self.leader.clone()),
            HotkeyTarget::Item(item) => self.items.get(item).cloned(),
        }
    }

    /// Tells each hotkey field how its hotkey stands, from the daemon's table, the answer to
    /// the last change asked for here and the file. Each render: those change apart.
    fn show_hotkey_stands(&self, cx: &mut Context<Self>) {
        let stands: Vec<_> = [
            (HotkeyTarget::Launcher, self.hotkey.clone()),
            (HotkeyTarget::Leader, self.leader.clone()),
        ]
        .into_iter()
        .chain(
            self.items
                .iter()
                .map(|(item, field)| (HotkeyTarget::Item(item.clone()), field.clone())),
        )
        .map(|(target, field)| (hotkey_stand(self, &target), field))
        .collect();
        for (stand, field) in stands {
            field.update(cx, |field, cx| field.set_stand(stand, cx));
        }
    }

    fn hotkey_fields(&self) -> impl Iterator<Item = &Entity<HotkeyField>> {
        [&self.hotkey, &self.leader]
            .into_iter()
            .chain(self.items.values())
    }

    pub(crate) fn is_recording(&self, cx: &gpui_kit::App) -> bool {
        self.hotkey_fields()
            .any(|field| field.read(cx).is_recording())
    }

    pub(crate) fn recorded_press(&mut self, hotkey: Hotkey, cx: &mut Context<Self>) {
        let recording: Vec<_> = self
            .hotkey_fields()
            .filter(|field| field.read(cx).is_recording())
            .cloned()
            .collect();
        for field in recording {
            field.update(cx, |field, cx| field.recorded_press(hotkey.clone(), cx));
        }
    }

    /// The leader went down (`pressed`) or up: a recording field takes it. `false`: none is
    /// recording.
    pub(crate) fn recorded_leader(&mut self, pressed: bool, cx: &mut Context<Self>) -> bool {
        let recording: Vec<_> = self
            .hotkey_fields()
            .filter(|field| field.read(cx).is_recording())
            .cloned()
            .collect();
        let mut taken = false;
        for field in recording {
            taken |= field.update(cx, |field, cx| {
                if pressed {
                    field.leader_pressed(cx)
                } else {
                    field.leader_released(cx)
                }
            });
        }
        taken
    }

    /// The file changed on disk. Its own write is left alone; anything else is read and
    /// shown, and a field whose typed value it changed is asked about.
    pub(crate) fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = read_text(&self.config_file).ok();
        let own = text.is_some() && text == self.last_written;
        let unchanged = text.as_deref() == Some(self.file.text.as_str());
        if own || unchanged {
            return;
        }
        self.errors.clear();
        let changed = self.reload(window, cx);
        self.queue(changed, window, cx);
    }

    /// Asks about each of `changed` after those asked about already, holding its writes
    /// until answered.
    fn queue(&mut self, changed: Vec<Conflict>, window: &mut Window, cx: &mut Context<Self>) {
        for conflict in changed {
            let id = conflict.field.entity_id();
            if !self.conflicts.iter().any(|c| c.field.entity_id() == id) {
                conflict.field.update(cx, |field, _| field.hold(true));
                self.conflicts.push_back(conflict);
            }
        }
        self.ask_next(window, cx);
        cx.notify();
    }

    /// Opens the question about the first conflict still standing, unless one is open.
    fn ask_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.asking {
            return;
        }
        // Answered in the meantime: the typed text was written, dropped or equals the file.
        while let Some(conflict) = self.conflicts.front()
            && conflict.field.read(cx).pending(cx).is_none()
        {
            if let Some(conflict) = self.conflicts.pop_front() {
                conflict.field.update(cx, |field, _| field.hold(false));
            }
        }
        if self.conflicts.is_empty() {
            for field in std::mem::take(&mut self.later) {
                field.update(cx, |field, _| field.hold(false));
            }
            return;
        }
        self.asking = true;
        if let Some(first) = self.conflicts.front() {
            conflict::ask(cx.entity().downgrade(), first, window, cx);
        }
    }

    /// The user's answer about the first conflict; the next one is asked about after.
    pub(crate) fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        self.asking = false;
        if let Some(conflict) = self.conflicts.pop_front() {
            match answer {
                Answer::Later => self.later.push(conflict.field),
                Answer::KeepMine | Answer::UseFiles => conflict.field.update(cx, |field, cx| {
                    field.hold(false);
                    if answer == Answer::KeepMine {
                        field.keep_typed(window, cx);
                    } else {
                        field.use_file_value(window, cx);
                    }
                }),
            }
        }
        // Once this question has closed.
        cx.defer_in(window, |this, window, cx| this.ask_next(window, cx));
        cx.notify();
    }

    /// Reads the file again and shows it, except in fields with a pending edit. Returns the
    /// text fields whose value changed under typed text; a broken file changes none.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Conflict> {
        self.file = FileState::read(&self.config_file);
        self.generation += 1;
        // Once the file holds something else, that text is an outside edit too (an undo).
        if self.last_written.as_deref() != Some(self.file.text.as_str()) {
            self.last_written = None;
        }
        let hotkey = file_hotkey(&self.file);
        self.hotkey
            .update(cx, |field, cx| field.set_from_file(hotkey, window, cx));
        let leader = file_leader(&self.file);
        self.leader
            .update(cx, |field, cx| field.set_from_file(leader, window, cx));
        let (city, invalid) = file_text(&self.file, CITY);
        self.city.update(cx, |field, cx| {
            field.set_from_file(city, invalid, Some(String::new()), window, cx)
        });
        let (zone, invalid) = file_text(&self.file, TIME_ZONE);
        self.time_zone.update(cx, |field, cx| {
            field.set_from_file(zone, invalid, Some(String::new()), window, cx)
        });
        for (field, path) in [(&self.font, FONT), (&self.mono_font, MONO_FONT)] {
            let (family, invalid) = file_text(&self.file, path);
            field.update(cx, |field, cx| {
                field.set_from_file(family, invalid, Some(String::new()), window, cx)
            });
        }
        for (path, field) in &self.choices {
            let (value, invalid) = file_text(&self.file, path);
            let default = Some(key(path).default.to_string());
            field.update(cx, |field, cx| {
                field.set_from_file(value, invalid, default, window, cx)
            });
        }
        let mut changed = self.sync_fields(window, cx);
        let broken = self.file.error().is_some();
        let size = file_number(&self.file, MONO_SIZE);
        let title = key(MONO_SIZE).title.into();
        if show_in_field(&self.mono_size, title, size, broken, window, cx) {
            changed.push(Conflict {
                field: self.mono_size.clone(),
            });
        }
        self.apply_disabled(cx);
        self.follow_pages(cx);
        cx.notify();
        changed
    }

    fn apply_disabled(&mut self, cx: &mut Context<Self>) {
        let broken = self.file.error().is_some();
        self.hotkey
            .update(cx, |field, cx| field.set_disabled(broken, cx));
        self.leader
            .update(cx, |field, cx| field.set_disabled(broken, cx));
        self.city
            .update(cx, |field, cx| field.set_disabled(broken, cx));
        self.time_zone
            .update(cx, |field, cx| field.set_disabled(broken, cx));
        for field in [&self.font, &self.mono_font] {
            field.update(cx, |field, cx| field.set_disabled(broken, cx));
        }
        for (_, field) in &self.choices {
            field.update(cx, |field, cx| field.set_disabled(broken, cx));
        }
        self.mono_size
            .update(cx, |field, cx| field.set_disabled(broken, cx));
        for field in self.prefs.values() {
            field.update(cx, |field, cx| field.set_disabled(broken, cx));
        }
        for field in self.picks.values() {
            field.update(cx, |field, cx| field.set_disabled(broken, cx));
        }
        for field in self.items.values() {
            field.update(cx, |field, cx| field.set_disabled(broken, cx));
        }
    }

    /// Writes one edit of the file's current text; `at` is the key path its error shows
    /// under.
    pub(crate) fn write(
        &mut self,
        at: &[&str],
        edit: impl FnOnce(&str) -> anyhow::Result<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = at.join(".");
        let written = read_text(&self.config_file).and_then(|text| {
            let new = edit(&text)?;
            write_config(&self.config_file, &new)?;
            Ok(new)
        });
        let failed = written.is_err();
        match written {
            Ok(text) => {
                self.errors.remove(&at);
                self.last_written = Some(text);
            }
            Err(e) => {
                tracing::warn!(key = at, "settings not written: {e:#}");
                self.errors.insert(at, format!("Not saved: {e:#}"));
            }
        }
        let changed = self.reload(window, cx);
        // Nothing written: what changed was read from an outside edit, whose change event
        // will find the file read already.
        if failed {
            self.queue(changed, window, cx);
        }
    }

    /// Applies `edit` to the Home list of the file, against the list the window shows;
    /// `tile` is the row its error shows on. A file changed since it was read is read again
    /// and nothing is written.
    pub(crate) fn edit_home(
        &mut self,
        tile: usize,
        edit: HomeEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let written = self.write_home(edit.clone());
        match &written {
            Ok(()) => {
                self.errors.retain(|at, _| !at.starts_with("home.widgets."));
            }
            Err(e) => {
                tracing::warn!(tile, "Home not written: {e:#}");
                let text = match e.downcast_ref::<HomeWriteError>() {
                    Some(HomeWriteError::Changed) => HomeWriteError::Changed.to_string(),
                    _ => format!("Not saved: {e:#}"),
                };
                self.errors.insert(home_error_key(tile), text);
            }
        }
        let changed = self.reload(window, cx);
        if written.is_ok() {
            // Its own write: the change event of this text is ignored.
            self.last_written = Some(self.file.text.clone());
        } else {
            // As in `write`: an outside edit was read.
            self.queue(changed, window, cx);
        }
    }

    fn write_home(&self, edit: HomeEdit) -> anyhow::Result<()> {
        // `edit_home` checks only that the file is TOML: a file the launcher refuses is
        // never written.
        let text = read_text(&self.config_file)?;
        Settings::parse(&text)?;
        let expected = self.home_entries();
        edit_home(&self.config_file, &expected, edit)?;
        Ok(())
    }

    /// Sets (or, for `None`, removes) the key at `path`.
    pub(crate) fn set_key(
        &mut self,
        path: &[&str],
        value: Option<toml::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use tomakast_core::settings_edit::set_key;
        let edit = |text: &str| set_key(text, path, value.map(edit_value).transpose()?);
        self.write(path, edit, window, cx);
    }

    /// Sets (or, for `None`, removes) preference `key` of `plugin`.
    pub(crate) fn set_preference(
        &mut self,
        plugin: &str,
        key: &str,
        value: Option<toml::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = |text: &str| {
            set_plugin_preference(text, plugin, key, value.map(edit_value).transpose()?)
        };
        self.write(&["plugins", plugin, key], edit, window, cx);
    }

    /// Removes the whole `[plugins.<plugin>]` table.
    pub(crate) fn remove_plugin_table(
        &mut self,
        plugin: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = |text: &str| remove_plugin_table(text, plugin);
        self.write(&["plugins", plugin], edit, window, cx);
    }

    /// The Config file row's buttons: makes sure the file exists (a created one is the
    /// window's own write), then opens it, or shows it in its folder, off the UI thread:
    /// the system may take a while to answer. A failure is said under the row.
    pub(crate) fn open_config(&mut self, reveal: bool, cx: &mut Context<Self>) {
        use tomakast_core::settings_edit::ensure_config_file;
        let failed = |e: &dyn std::fmt::Display| format!("Could not open: {e}");
        match ensure_config_file(&self.config_file) {
            Ok(true) => self.last_written = Some(String::new()),
            Ok(false) => {}
            Err(e) => {
                self.errors.insert(CONFIG_FILE.into(), failed(&e));
                cx.notify();
                return;
            }
        }
        let Some(opener) = self.opener.clone() else {
            self.errors
                .insert(CONFIG_FILE.into(), failed(&"not available in this run"));
            cx.notify();
            return;
        };
        self.errors.remove(CONFIG_FILE);
        cx.notify();
        let path = self.config_file.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if reveal {
                        opener.reveal(&path)
                    } else {
                        opener.open(&path)
                    }
                })
                .await;
            if let Err(e) = result {
                let _ = this.update(cx, |view, cx| {
                    view.errors
                        .insert(CONFIG_FILE.into(), format!("Could not open: {e:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Asks the daemon for a new hotkey of `target` (`None`: clear an item's, or the
    /// launcher's default); it writes the file once the system took it.
    pub(crate) fn request_hotkey(
        &mut self,
        target: HotkeyTarget,
        hotkey: Option<Hotkey>,
        cx: &mut Context<Self>,
    ) {
        self.hotkey_notice = None;
        let requests = self.requests.borrow().clone();
        match requests {
            Some(requests) => requests.assign(target, hotkey, HotkeyOrigin::Settings),
            None => self.hotkey_notice(target, "Hotkeys cannot be changed in this run.".into(), cx),
        }
        cx.notify();
    }
}

/// Shows the file's `value` in `field`, titled `title`; `true` when it changed under typed
/// text. A broken file's values are not shown under typed text: the typed text and the value
/// Esc restores stay as they were until the file is fixed.
fn show_in_field(
    field: &Entity<TextField>,
    title: gpui_kit::SharedString,
    value: String,
    broken: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    field.update(cx, |field, cx| {
        field.set_title(title, cx);
        if broken && field.pending(cx).is_some() {
            return false;
        }
        field.set_from_file(value, window, cx)
    })
}

/// `value` as the file editor takes it.
fn edit_value(value: toml::Value) -> anyhow::Result<toml_edit::Value> {
    Ok(match value {
        toml::Value::String(s) => s.into(),
        toml::Value::Integer(n) => n.into(),
        toml::Value::Float(f) => f.into(),
        toml::Value::Boolean(b) => b.into(),
        other => anyhow::bail!("{other} is not a single value"),
    })
}

/// The launcher hotkey in the file, or the default.
fn file_hotkey(file: &FileState) -> Option<Hotkey> {
    match file.value(key(LAUNCHER_HOTKEY)) {
        KeyValue::Valid(toml::Value::String(s)) => Hotkey::parse(&s).ok(),
        _ => None,
    }
}

/// The leader key in the file; `None`: none, or an empty one.
fn file_leader(file: &FileState) -> Option<Hotkey> {
    match file.value(key(LEADER)) {
        KeyValue::Valid(toml::Value::String(s)) if !s.trim().is_empty() => {
            Hotkey::parse_leader(&s).ok()
        }
        _ => None,
    }
}

/// The launcher hotkey while the file has none (or one that is not valid).
fn default_hotkey() -> Hotkey {
    Hotkey::parse(key(LAUNCHER_HOTKEY).default).expect("the default hotkey parses")
}

/// The string key at `path` in the file, trimmed; empty when it has none, or none of the
/// right type (then `true`).
fn file_text(file: &FileState, path: &[&str]) -> (String, bool) {
    match file.value(key(path)) {
        KeyValue::Valid(toml::Value::String(s)) => (s.trim().to_string(), false),
        KeyValue::Invalid(_) => (String::new(), true),
        _ => (String::new(), false),
    }
}

/// The number key at `path` as its field shows it: the file's value, else the default.
fn file_number(file: &FileState, path: &[&str]) -> String {
    match file.value(key(path)) {
        KeyValue::Valid(value @ (toml::Value::Integer(_) | toml::Value::Float(_))) => {
            value.to_string()
        }
        _ => key(path).default.to_string(),
    }
}

/// The installed font families, sorted, without the hidden ones (`.SystemUIFont`).
fn installed_families(cx: &App) -> Vec<String> {
    let mut families: Vec<String> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    families
}

/// The list field of the built-in choice key at `path`: its options (on the screen key, only
/// those the desktop can honour), with their icons, standing on the file's value or the
/// default.
fn choice_field(
    path: &'static [&'static str],
    file: &FileState,
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) -> Entity<PickField> {
    let key = key(path);
    let ValueKind::Choice(options) = &key.kind else {
        unreachable!("{path:?} is a choice");
    };
    let offered: Option<Vec<&str>> = (path == SCREEN).then(|| {
        tomakast_platform::screen_choices()
            .iter()
            .map(|choice| choice.value())
            .collect()
    });
    let items = options
        .iter()
        .filter(|o| {
            offered
                .as_ref()
                .is_none_or(|offered| offered.contains(&o.value.as_str()))
        })
        .map(|o| {
            let item = PickItem::new(o.value.clone(), o.title.clone());
            match THEME_ICONS
                .iter()
                .find(|(value, _)| path == THEME && *value == o.value)
            {
                Some((_, icon)) => item.icon(icon),
                None => item,
            }
        })
        .collect();
    let (value, invalid) = file_text(file, path);
    cx.new(|cx| {
        let spec = PickSpec {
            id: path.join("-").into(),
            title: key.title.into(),
            search_placeholder: SEARCH_PLACEHOLDER.into(),
            items,
            unset: SharedString::default(),
        };
        PickField::new(
            spec,
            value,
            invalid,
            Some(key.default.to_string()),
            window,
            cx,
        )
    })
}

/// The pick field of the font key at `path`: the desktop's font, named, first; then every
/// installed family and the bundled one, in alphabetical order. The key's default (the
/// desktop's UI font; the bundled monospace family) removes the key, and the file's value
/// naming the default stands on it.
fn font_field(
    path: &'static [&'static str],
    families: &[String],
    file: &FileState,
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) -> Entity<PickField> {
    let key = key(path);
    let mono = path == MONO_FONT;
    let system = tomakast_platform::system_font(if mono {
        tomakast_platform::FontKind::Monospace
    } else {
        tomakast_platform::FontKind::Ui
    });
    let system_title = match (mono, system) {
        (false, Some(family)) => format!("System UI font ({family})"),
        (false, None) => "System UI font".to_string(),
        (true, Some(family)) => format!("System monospace font ({family})"),
        (true, None) => "System monospace font".to_string(),
    };
    let bundled = if mono {
        crate::fonts::MONO_FAMILY
    } else {
        crate::fonts::UI_FAMILY
    };
    // The default's item has the empty value: picking it removes the key.
    let value_of = |value: &str| -> SharedString {
        if value == key.default {
            SharedString::default()
        } else {
            value.to_string().into()
        }
    };
    let mut names: Vec<&str> = families.iter().map(String::as_str).collect();
    if !names.contains(&bundled) {
        names.push(bundled);
        names.sort_by_key(|name| name.to_lowercase());
    }
    let unset = if mono {
        bundled.to_string()
    } else {
        system_title.clone()
    };
    let items = std::iter::once(PickItem::new(value_of("system"), system_title))
        .chain(
            names
                .into_iter()
                .map(|family| PickItem::new(value_of(family), family.to_string())),
        )
        .collect();
    let (value, invalid) = file_text(file, path);
    let value = value_of(&value).to_string();
    let id = if mono { "mono-font" } else { "font" };
    cx.new(|cx| {
        let spec = PickSpec {
            id: id.into(),
            title: key.title.into(),
            search_placeholder: "Search fonts…".into(),
            items,
            unset: unset.into(),
        };
        PickField::new(spec, value, invalid, Some(String::new()), window, cx)
    })
}

/// The title of the time zone list's first entry, which removes the key: the computer's
/// zone, named.
fn automatic_zone(computer: &str) -> String {
    format!("Automatic ({computer})")
}

/// "Automatic", then every zone.
fn automatic_zone_items(computer: &str) -> Vec<PickItem> {
    std::iter::once(PickItem::new(
        SharedString::default(),
        automatic_zone(computer),
    ))
    .chain(zone_items().iter().cloned())
    .collect()
}

/// Why `target` cannot take a hotkey, in the recorders' words.
fn hotkey_check(
    table: Rc<RefCell<HotkeyTable>>,
    core: Rc<RefCell<Core>>,
    target: HotkeyTarget,
) -> HotkeyCheck {
    Rc::new(move |hotkey, _| {
        refusal_text(&table.borrow(), &target, hotkey, &|item| {
            item_title(&core, item)
        })
    })
}

/// The title of indexed `item`, or [`gone_title`].
fn item_title(core: &RefCell<Core>, item: &ItemId) -> String {
    core.try_borrow()
        .ok()
        .and_then(|core| core.item_title(item))
        .unwrap_or_else(|| gone_title(item))
}

/// What an item that is not there now is called: its id without where it comes from, and an
/// application's without `.desktop` (`apps/org.foo.desktop` is "org.foo").
pub(crate) fn gone_title(item: &ItemId) -> String {
    item.fallback_title()
}

/// The provider of applications.
pub(crate) const APPS: &str = "apps";

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let this = cx.entity().downgrade();
        self.order_hotkey_rows(window, cx);
        let mut built = self.take_pages(&this, cx);
        let query = self.search.read(cx).value();
        let mut matches = built.matches(&query);
        let mut revealed = built.revealed(&query, &mut self.search_reads);
        let no_rows = Vec::new();
        let kept = |ix: &usize| matches[*ix].is_some();
        // The page on display, or the first page the search keeps when it keeps nothing of it.
        let shown = built
            .keys
            .iter()
            .position(|key| *key == self.shown)
            .filter(kept)
            .or_else(|| (0..built.pages.len()).find(kept));
        if let Some(ix) = shown {
            self.displayed = built.keys[ix].clone();
            self.open_groups.follow(&built.pages[ix]);
            // The hotkey fields of the rows drawn now: made the first time a row is drawn, so
            // a group never opened has none.
            let groups = matches[ix].as_deref().unwrap_or_default();
            let page_revealed = revealed.get(ix).unwrap_or(&no_rows);
            let missing: Vec<ItemId> =
                layout::drawn_rows(&built.pages[ix], groups, page_revealed, &self.open_groups)
                    .into_iter()
                    .filter_map(|row| row.hotkey_of.clone())
                    .filter(|item| !self.items.contains_key(item))
                    .collect();
            if !missing.is_empty() {
                self.make_item_fields(missing, window, cx);
                self.built = Some(built);
                built = self.take_pages(&this, cx);
                matches = built.matches(&query);
                revealed = built.revealed(&query, &mut self.search_reads);
            }
        }
        let pages = &built.pages;
        self.show_hotkey_stands(cx);
        let on_select: OnSelect = {
            let this = this.clone();
            Rc::new(move |page, _, cx| {
                let page = page.clone();
                let _ = this.update(cx, |view, cx| view.show_page(page, cx));
            })
        };
        let on_toggle: OnToggle = Rc::new(move |key, revealed, _, cx| {
            let _ = this.update(cx, |view, cx| {
                view.open_groups.toggle(key, revealed);
                cx.notify();
            });
        });
        let controls = self.frame.controls(window);
        let sidebar_title =
            self.frame
                .sidebar_title(self.search_field(cx), &controls, window, &palette);
        let sidebar = layout::sidebar(
            sidebar_title,
            pages,
            &matches,
            shown.map(|ix| &pages[ix].key),
            on_select,
            &palette,
        );
        // In the page's column, over the page and out of its scroll: what is wrong, and what
        // follows from it.
        let banner = self.file.error().map(|error| {
            let text = format!(
                "config.toml has an error: {error}. Fix the file; nothing is written until then."
            );
            let detail = SharedString::from(error.to_string());
            layout::page_column(
                div().child(
                    div()
                        .id("settings-banner")
                        .aria_label(text)
                        .test_support()
                        .w_full()
                        .flex()
                        .items_start()
                        .gap(px(theme::BANNER_ICON_GAP))
                        .px(px(theme::SETTINGS_ROW_PADDING_X))
                        .py(px(theme::SETTINGS_ROW_PADDING_Y))
                        .rounded(px(theme::SETTINGS_GROUP_RADIUS))
                        .border_1()
                        .border_color(palette.error.opacity(theme::BANNER_BORDER_OPACITY))
                        .bg(palette.error.opacity(theme::BANNER_FILL_OPACITY))
                        .child(
                            div()
                                .flex_none()
                                .h(px(theme::BANNER_LINE_HEIGHT))
                                .flex()
                                .items_center()
                                .child(
                                    gpui_kit::svg()
                                        .path(
                                            crate::icons::bundled_path("circle-alert")
                                                .unwrap_or_default(),
                                        )
                                        .size(px(theme::SETTINGS_TABLE_SOURCE_ICON_SIZE))
                                        .text_color(palette.error),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(theme::SETTINGS_DESCRIPTION_GAP))
                                .child(
                                    div()
                                        .min_h(px(theme::BANNER_LINE_HEIGHT))
                                        .flex()
                                        .items_center()
                                        .text_size(theme::text(theme::ROW_TEXT_SIZE))
                                        .text_color(palette.text)
                                        .child(BANNER_TITLE),
                                )
                                .child(
                                    div()
                                        .id("settings-banner-detail")
                                        // No name of its own: the banner's says the error.
                                        .test_support()
                                        .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                                        .text_color(palette.muted)
                                        .child(detail),
                                ),
                        ),
                ),
            )
            .flex_none()
            .pt(px(theme::SETTINGS_EDGE))
        });
        let body = match shown {
            Some(ix) => {
                let groups = matches[ix].as_deref().unwrap_or_default();
                // Their rows are drawn wherever they are: a recording goes on out of view.
                let in_use: HashSet<ItemId> = self
                    .items
                    .iter()
                    .filter(|(_, field)| field.read(cx).is_in_use(window, cx))
                    .map(|(item, _)| item.clone())
                    .collect();
                layout::page_body(
                    &pages[ix],
                    groups,
                    &query,
                    revealed.get(ix).unwrap_or(&no_rows),
                    &mut self.open_groups,
                    &mut self.viewports,
                    &in_use,
                    on_toggle,
                    window,
                    cx,
                )
            }
            None => layout::empty_state(&palette),
        };
        let title = shown.map(|ix| pages[ix].title.clone());
        self.built = Some(built);
        // The window's corners are rounded when it draws its own frame; its fill follows them.
        let radii = window_corner_radii(window, cx);
        div()
            .id("settings-root")
            .track_focus(&self.root_focus)
            .size_full()
            .flex()
            .bg(palette.panel.alpha(1.))
            .rounded_tl(radii.top_left)
            .rounded_tr(radii.top_right)
            .rounded_bl(radii.bottom_left)
            .rounded_br(radii.bottom_right)
            .text_color(palette.text)
            .child(
                // Its own shade, following the window's left corners.
                div()
                    .flex_none()
                    .h_full()
                    .bg(palette.sidebar)
                    .rounded_tl(radii.top_left)
                    .rounded_bl(radii.bottom_left)
                    .child(sidebar),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(theme::DIVIDER_THICKNESS))
                    .h_full()
                    .bg(palette.divider),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        self.frame
                            .page_title(title.as_deref(), &controls, window, &palette),
                    )
                    .children(banner)
                    .child(body),
            )
    }
}
