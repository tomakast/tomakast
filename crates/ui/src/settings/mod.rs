//! The settings window: every key of `config.toml` in an ordinary window, opened through the
//! kit (its Root draws dialogs and tooltips; the window draws its own layout with the kit's
//! controls in it). Nothing of it is built until it is first opened.

mod conflict;
mod fields;
mod frame;
mod layout;
mod opener;
mod pages;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Bounds, Entity, Global,
    Subscription, TitlebarOptions, WindowBounds, WindowDecorations, WindowId, WindowKind,
    WindowOptions, point, px, size,
};
use tomakast_core::hotkeys::{HotkeyTable, HotkeyTarget};
use tomakast_core::{Core, Hotkey, ItemId, PluginCatalog};

use crate::hotkeys::HotkeyRequests;
pub use opener::{ConfigOpener, PlatformOpener};
pub use window::SettingsWindow;
use window::WindowInit;

/// Where hotkey fields send their changes; `None` until the daemon sets it.
type SharedRequests = Rc<RefCell<Option<Rc<dyn HotkeyRequests>>>>;

/// Wayland app id and X11 class of the settings window.
const APP_ID: &str = "dev.tomakast.Tomakast.Settings";
const TITLE: &str = "Settings — tomakast";

/// A page of the settings window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    #[default]
    General,
    Launcher,
    Appearance,
    Location,
    Hotkeys,
    About,
    Home,
    /// Every installed plugin with its switch: the head of the plugin pages.
    Plugins,
    /// A plugin's page, by plugin id.
    Plugin(String),
}

impl SettingsPage {
    /// The page `id` names: `general`, `launcher`, `appearance`, `location`, `hotkeys`, `about`,
    /// `home`, `plugins` or `plugin:<id>`.
    pub fn parse(id: &str) -> Option<SettingsPage> {
        match id {
            "general" => Some(SettingsPage::General),
            "launcher" => Some(SettingsPage::Launcher),
            "appearance" => Some(SettingsPage::Appearance),
            "location" => Some(SettingsPage::Location),
            "hotkeys" => Some(SettingsPage::Hotkeys),
            "about" => Some(SettingsPage::About),
            "home" => Some(SettingsPage::Home),
            "plugins" => Some(SettingsPage::Plugins),
            _ => id
                .strip_prefix("plugin:")
                .filter(|plugin| !plugin.is_empty())
                .map(|plugin| SettingsPage::Plugin(plugin.to_string())),
        }
    }
}

/// Stored in a [`gpui_kit::Global`]; call it through `cx.update_global`. Opens the one
/// settings window and hands it what the daemon learns while it is open.
pub struct SettingsController {
    config_file: PathBuf,
    core: Rc<RefCell<Core>>,
    catalog: PluginCatalog,
    /// Whether the plugin manager has published a catalog yet: until then `catalog` is empty
    /// for want of a list, not because nothing is installed.
    catalog_known: bool,
    table: HotkeyTable,
    /// Shared with the open window.
    requests: SharedRequests,
    /// What the Config file row opens the file with; `None` until the daemon sets it.
    opener: Option<Arc<dyn ConfigOpener>>,
    /// The page last on display, which the window opens on when no page is asked for.
    last_page: Rc<RefCell<SettingsPage>>,
    /// Where a new window reads the computer's time zone (an IANA name, `None` when it cannot
    /// be told), which an unset time zone stands for.
    time_zone: TimeZoneSource,
    open: Option<OpenWindow>,
}

/// Where the computer's time zone is read: an IANA name, or `None` when it cannot be told.
pub type TimeZoneSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

struct OpenWindow {
    handle: AnyWindowHandle,
    view: Entity<SettingsWindow>,
    /// Forgets the window when the user closes it.
    _on_closed: Subscription,
}

impl Global for SettingsController {}

impl SettingsController {
    pub fn new(config_file: PathBuf, core: Rc<RefCell<Core>>) -> SettingsController {
        SettingsController {
            config_file,
            core,
            catalog: PluginCatalog::default(),
            catalog_known: false,
            table: HotkeyTable::default(),
            requests: SharedRequests::default(),
            opener: None,
            last_page: Rc::default(),
            time_zone: Arc::new(|| iana_time_zone::get_timezone().ok()),
            open: None,
        }
    }

    /// Where a window opened after this reads the computer's time zone; the operating
    /// system's setting unless set.
    pub fn set_time_zone_source(&mut self, source: TimeZoneSource) {
        self.time_zone = source;
    }

    /// Opens the window on `page` (`None`: the page last on display). When it is open
    /// already, brings it forward and switches to `page`. A plugin page that is not there
    /// yet (the catalog does not list the plugin) opens once it is; until then the page last
    /// on display stays.
    pub fn open(&mut self, page: Option<SettingsPage>, cx: &mut App) {
        if let Some(open) = &self.open {
            let view = open.view.clone();
            let shown = open.handle.update(cx, |_, window, cx| {
                if crate::window::keyboard_allowed() {
                    window.activate_window();
                }
                if let Some(page) = page.clone() {
                    view.update(cx, |view, cx| view.open_page(page, window, cx));
                }
            });
            if shown.is_ok() {
                return;
            }
            self.open = None;
        }
        let page = page.unwrap_or_else(|| self.last_page.borrow().clone());
        let init = WindowInit {
            config_file: self.config_file.clone(),
            core: self.core.clone(),
            catalog: self.catalog.clone(),
            catalog_known: self.catalog_known,
            table: self.table.clone(),
            requests: self.requests.clone(),
            opener: self.opener.clone(),
            page,
            last_page: self.last_page.clone(),
            time_zone: (self.time_zone)(),
        };
        crate::theme::round_kit_windows(cx);
        let opened = gpui_kit::open_window(options(cx), cx, |window, cx| {
            cx.new(|cx| SettingsWindow::new(init, window, cx))
        });
        match opened {
            Ok((handle, view)) => {
                let id = handle.window_id();
                // Close observers run inside the closing window's update, where the global
                // may be leased, so the forget is deferred.
                let on_closed = cx.on_window_closed(move |cx, closed| {
                    if closed == id {
                        cx.defer(move |cx| forget_closed_window(id, cx));
                    }
                });
                self.open = Some(OpenWindow {
                    handle,
                    view,
                    _on_closed: on_closed,
                });
            }
            Err(e) => tracing::error!(error = %e, "settings window could not be opened"),
        }
    }

    /// Every installed plugin, as the plugin manager published it last.
    pub fn set_catalog(&mut self, catalog: PluginCatalog, cx: &mut App) {
        self.catalog = catalog.clone();
        self.catalog_known = true;
        self.update_view(cx, move |view, window, cx| {
            view.set_catalog(catalog, window, cx)
        });
    }

    pub fn catalog(&self) -> &PluginCatalog {
        &self.catalog
    }

    /// The hotkeys as the daemon applied them, for the recorder's refusals.
    pub fn set_hotkey_table(&mut self, table: HotkeyTable, cx: &mut App) {
        self.table = table.clone();
        self.update_view(cx, move |view, _, cx| view.set_hotkey_table(table, cx));
    }

    /// Where the window's hotkey fields send their changes; without it they cannot change.
    pub fn set_hotkey_requests(&mut self, requests: Rc<dyn HotkeyRequests>) {
        *self.requests.borrow_mut() = Some(requests);
    }

    /// What the Config file row opens the file and its folder with. Takes effect for a
    /// window opened after this.
    pub fn set_config_opener(&mut self, opener: Arc<dyn ConfigOpener>) {
        self.opener = Some(opener);
    }

    /// A message about the change of `target`'s hotkey the window asked for, shown under
    /// that field. `false` when the window is not open: the caller shows it elsewhere.
    pub fn hotkey_notice(&mut self, target: HotkeyTarget, text: String, cx: &mut App) -> bool {
        self.update_view(cx, move |view, _, cx| view.hotkey_notice(target, text, cx))
            .is_some()
    }

    /// `config.toml` changed on disk: the window shows the file's values, except in a field
    /// the user is editing; when the change is to the value being typed, it asks which to
    /// keep. Its own writes are recognised and ignored.
    pub fn config_changed(&mut self, cx: &mut App) {
        self.update_view(cx, |view, window, cx| view.reload_from_disk(window, cx));
    }

    /// Whether a hotkey field of the window is recording: presses of tomakast's own hotkeys
    /// then go to [`recorded_press`](Self::recorded_press) instead of running.
    pub fn is_recording(&self, cx: &App) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.view.read(cx).is_recording(cx))
    }

    /// The leader key went down; `true` when a recorder took it (then nothing else sees it).
    pub fn leader_pressed(&mut self, cx: &mut App) -> bool {
        self.update_view(cx, |view, _, cx| view.recorded_leader(true, cx))
            .unwrap_or(false)
    }

    /// The leader key went up; `true` when a recorder took it (then nothing else sees it).
    pub fn leader_released(&mut self, cx: &mut App) -> bool {
        self.update_view(cx, |view, _, cx| view.recorded_leader(false, cx))
            .unwrap_or(false)
    }

    /// A tomakast hotkey the system fired while a field was recording.
    pub fn recorded_press(&mut self, hotkey: Hotkey, cx: &mut App) {
        self.update_view(cx, move |view, _, cx| view.recorded_press(hotkey, cx));
    }

    /// For tests: the items the open window has made a hotkey field for, by id.
    #[doc(hidden)]
    pub fn hotkey_field_items(&self, cx: &App) -> Vec<ItemId> {
        let mut items: Vec<ItemId> = self
            .open
            .as_ref()
            .map(|open| open.view.read(cx).items.keys().cloned().collect())
            .unwrap_or_default();
        items.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        items
    }

    /// For tests: how many times the open window has built its pages.
    #[doc(hidden)]
    pub fn page_builds(&self, cx: &App) -> usize {
        self.open
            .as_ref()
            .map_or(0, |open| open.view.read(cx).page_builds)
    }

    /// For tests: how many times the open window has read which rows a query opens its
    /// groups on.
    #[doc(hidden)]
    pub fn search_reads(&self, cx: &App) -> usize {
        self.open
            .as_ref()
            .map_or(0, |open| open.view.read(cx).search_reads)
    }

    /// The open settings window.
    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.open.as_ref().map(|open| open.handle)
    }

    /// Runs `f` on the open window's view; `None` when there is none.
    fn update_view<R>(
        &mut self,
        cx: &mut App,
        f: impl FnOnce(
            &mut SettingsWindow,
            &mut gpui_kit::Window,
            &mut gpui_kit::Context<SettingsWindow>,
        ) -> R,
    ) -> Option<R> {
        let open = self.open.as_ref()?;
        let view = open.view.clone();
        open.handle
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| f(view, window, cx))
            })
            .ok()
    }
}

/// The window draws its own title area: on Linux it asks for client-side decorations, on
/// macOS and Windows it hides the system's title bar (macOS keeps its traffic lights).
fn options(cx: &App) -> WindowOptions {
    let (x, y) = crate::theme::SETTINGS_TRAFFIC_LIGHT_POSITION;
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(x), px(y))),
        }),
        window_decorations: Some(WindowDecorations::Client),
        // macOS: the title area moves the window and answers double clicks itself, so AppKit
        // neither drags from it nor delays its clicks to tell double clicks apart.
        app_owns_titlebar_drag: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(900.), px(640.)),
            cx,
        ))),
        // `TOMAKAST_NO_KEYBOARD=1`: development runs do not take the user's keyboard.
        focus: crate::window::keyboard_allowed(),
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        // A fixed size: the pages are laid out for it.
        is_resizable: false,
        is_minimizable: true,
        app_id: Some(APP_ID.into()),
        ..Default::default()
    }
}

/// Drops the controller's window `id` if it is still the open one, so the next open makes a
/// new window.
fn forget_closed_window(id: WindowId, cx: &mut App) {
    if cx.has_global::<SettingsController>() {
        cx.update_global::<SettingsController, _>(|c, _| {
            if c.open.as_ref().is_some_and(|o| o.handle.window_id() == id) {
                c.open = None;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::SettingsPage;

    /// The window asks to draw its own frame, with no system title bar over it, and keeps the
    /// app's title for task bars and window lists.
    #[gpui_kit::test]
    fn the_window_draws_its_own_frame(cx: &mut gpui_kit::TestAppContext) {
        let options = cx.update(|cx| super::options(cx));
        assert_eq!(
            options.window_decorations,
            Some(gpui_kit::WindowDecorations::Client)
        );
        let titlebar = options.titlebar.expect("title bar options");
        assert_eq!(titlebar.title.as_deref(), Some("Settings — tomakast"));
        assert!(titlebar.appears_transparent);
        assert!(options.app_owns_titlebar_drag);
        assert_eq!(
            titlebar.traffic_light_position,
            Some(gpui_kit::point(gpui_kit::px(12.), gpui_kit::px(14.)))
        );
    }

    /// The window keeps the size its pages are laid out for: the user cannot resize or
    /// maximize it (the frame drops maximize for a window that is not resizable).
    #[gpui_kit::test]
    fn the_window_has_a_fixed_size(cx: &mut gpui_kit::TestAppContext) {
        let options = cx.update(|cx| super::options(cx));
        assert!(!options.is_resizable);
        assert_eq!(options.window_min_size, None);
        let Some(gpui_kit::WindowBounds::Windowed(bounds)) = options.window_bounds else {
            panic!("a windowed window");
        };
        assert_eq!(
            bounds.size,
            gpui_kit::size(gpui_kit::px(900.), gpui_kit::px(640.))
        );
    }

    #[test]
    fn page_ids_parse() {
        assert_eq!(SettingsPage::parse("general"), Some(SettingsPage::General));
        assert_eq!(
            SettingsPage::parse("launcher"),
            Some(SettingsPage::Launcher)
        );
        assert_eq!(
            SettingsPage::parse("location"),
            Some(SettingsPage::Location)
        );
        assert_eq!(SettingsPage::parse("hotkeys"), Some(SettingsPage::Hotkeys));
        assert_eq!(SettingsPage::parse("home"), Some(SettingsPage::Home));
        assert_eq!(
            SettingsPage::parse("appearance"),
            Some(SettingsPage::Appearance)
        );
        assert_eq!(SettingsPage::parse("about"), Some(SettingsPage::About));
        assert_eq!(SettingsPage::parse("plugins"), Some(SettingsPage::Plugins));
        assert_eq!(
            SettingsPage::parse("plugin:weather"),
            Some(SettingsPage::Plugin("weather".into()))
        );
        for unknown in ["", "plugin:", "General", "plugins:x", "other"] {
            assert_eq!(SettingsPage::parse(unknown), None, "{unknown:?}");
        }
    }
}
