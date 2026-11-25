//! Headless tests of the settings window: opened through the controller as the daemon does
//! (a kit window with its Root), over a temporary `config.toml`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::{TestEventExt, TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, MouseButton, QuitMode,
    TestAppContext, Window,
};
use tempfile::TempDir;
use tomakast_core::hotkeys::{HotkeyOrigin, HotkeyTarget};
use tomakast_core::store::Store;
use tomakast_core::{
    CatalogPlugin, Core, FontChoice, Hotkey, LocalNow, PluginCatalog, PluginState, RankWeights,
    TextSettings, TextSize, ThemeChoice,
};
use tomakast_ui::{ConfigOpener, HotkeyRequests, SettingsController, SettingsPage};

type Requests = Rc<RefCell<Vec<(HotkeyTarget, Option<Hotkey>, HotkeyOrigin)>>>;

struct FakeRequests(Requests);

impl HotkeyRequests for FakeRequests {
    fn assign(&self, target: HotkeyTarget, hotkey: Option<Hotkey>, origin: HotkeyOrigin) {
        self.0.borrow_mut().push((target, hotkey, origin));
    }
}

struct Setup {
    _dir: TempDir,
    config: PathBuf,
    requests: Requests,
    core: Rc<RefCell<Core>>,
}

fn hk(s: &str) -> Hotkey {
    Hotkey::parse(s).unwrap()
}

/// The controller over a `config.toml` holding `text`, with hotkey requests recorded.
fn setup(cx: &mut TestAppContext, text: &str) -> Setup {
    setup_with(cx, text, new_core())
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

fn setup_with(cx: &mut TestAppContext, text: &str, core: Core) -> Setup {
    setup_in(cx, text, core, "")
}

/// [`setup_with`], with `config.toml` in the folder `folder` of the temporary one.
fn setup_in(cx: &mut TestAppContext, text: &str, core: Core, folder: &str) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(folder).join("config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, text).unwrap();
    let core = Rc::new(RefCell::new(core));
    let requests = Requests::default();
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        let mut controller = SettingsController::new(config.clone(), core.clone());
        controller.set_hotkey_requests(Rc::new(FakeRequests(requests.clone())));
        cx.set_global(controller);
    });
    Setup {
        _dir: dir,
        config,
        requests,
        core,
    }
}

/// Opens the window as the user would see it: active, so focus moves report blurs.
fn open(cx: &mut TestAppContext, page: Option<SettingsPage>) {
    cx.update(|cx| cx.update_global::<SettingsController, _>(|c, cx| c.open(page, cx)));
    cx.run_until_parked();
    on_window(cx, |window, _| window.activate_window());
    cx.run_until_parked();
}

fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| cx.global::<SettingsController>().window())
        .expect("the settings window is open")
}

/// Runs `f` on a fresh frame of the settings window.
fn on_window<R>(cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx)
    })
    .unwrap()
}

fn has_label(cx: &mut TestAppContext, label: &str) -> bool {
    on_window(cx, |window, _| window.try_find_by_label(label).is_some())
}

/// Clicks the element labelled `label`.
fn click(cx: &mut TestAppContext, label: &str) {
    on_window(cx, |window, cx| {
        let at = window.find_by_label(label).bounds().center();
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, key: &str) {
    on_window(cx, |window, cx| window.press(key, cx));
    cx.run_until_parked();
}

fn type_text(cx: &mut TestAppContext, text: &str) {
    on_window(cx, |window, cx| window.input(text, cx));
    cx.run_until_parked();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn config_changed(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<SettingsController, _>(|c, cx| c.config_changed(cx)));
    cx.run_until_parked();
}

fn value_of(cx: &mut TestAppContext, label: &str) -> Option<String> {
    on_window(cx, |window, _| {
        window.find_by_label(label).value().map(str::to_string)
    })
}

/// The accessible description of the element labelled `label`: how a hotkey stands.
fn description_of(cx: &mut TestAppContext, label: &str) -> Option<String> {
    on_window(cx, |window, _| {
        window
            .find_by_label(label)
            .description()
            .map(str::to_string)
    })
}

/// Clicks the element whose label starts with `prefix`: a line of a list whose title names
/// what the computer has.
fn click_starting(cx: &mut TestAppContext, prefix: &str) {
    let label = labels(cx)
        .into_iter()
        .find(|label| label.starts_with(prefix))
        .unwrap_or_else(|| panic!("no label starts with {prefix:?}"));
    click(cx, &label);
}

/// The list field labelled `field` shows `value`.
fn shows(cx: &mut TestAppContext, field: &str, value: &str) -> bool {
    has_label(cx, field) && value_of(cx, field).as_deref() == Some(value)
}

fn checked(cx: &mut TestAppContext, label: &str) -> Option<bool> {
    on_window(cx, |window, _| window.find_by_label(label).checked())
}

#[gpui_kit::test]
fn window_opens_once_and_switches_page(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, None);
    assert!(has_label(cx, "Start at login"));
    assert!(!has_label(cx, "City"));
    open(cx, Some(SettingsPage::Location));
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert!(has_label(cx, "City"));
    assert!(!has_label(cx, "Start at login"));
}

#[gpui_kit::test]
fn a_closed_window_opens_again_on_the_last_page(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Location));
    let w = window(cx);
    cx.update_window(w, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| cx.global::<SettingsController>().window()),
        None
    );
    open(cx, None);
    assert!(has_label(cx, "City"));
}

#[gpui_kit::test]
fn switch_writes_at_once(cx: &mut TestAppContext) {
    let s = setup(cx, "# mine\n[launcher]\n# login\nautostart = true # yes\n");
    open(cx, None);
    assert_eq!(checked(cx, "Start at login"), Some(true));
    click(cx, "Start at login");
    assert_eq!(
        read(&s.config),
        "# mine\n[launcher]\n# login\nautostart = false # yes\n"
    );
    assert_eq!(checked(cx, "Start at login"), Some(false));
}

#[gpui_kit::test]
fn a_switch_on_a_file_without_the_key_adds_it(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Location));
    assert_eq!(checked(cx, "Look up location by Wi-Fi"), Some(false));
    assert!(!has_label(cx, "Reset Look up location by Wi-Fi"));
    click(cx, "Look up location by Wi-Fi");
    assert_eq!(read(&s.config), "[location]\nwifi_lookup = true\n");
    assert!(has_label(cx, "Reset Look up location by Wi-Fi"));
}

#[gpui_kit::test]
fn reset_removes_the_key(cx: &mut TestAppContext) {
    let s = setup(cx, "# mine\nzz = 1\n\n[launcher]\nautostart = false\n");
    open(cx, None);
    assert_eq!(checked(cx, "Start at login"), Some(false));
    click(cx, "Reset Start at login");
    assert_eq!(read(&s.config), "# mine\nzz = 1\n");
    assert_eq!(checked(cx, "Start at login"), Some(true));
    assert!(!has_label(cx, "Reset Start at login"));
}

#[gpui_kit::test]
fn city_pick_writes_the_name(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Location));
    click(cx, "City");
    type_text(cx, "Berlin");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[location]\ncity = \"Berlin\"\n");
}

#[gpui_kit::test]
fn city_automatic_removes_the_key(cx: &mut TestAppContext) {
    let s = setup(cx, "[location]\ncity = \"Berlin\"\nwifi_lookup = true\n");
    open(cx, Some(SettingsPage::Location));
    click(cx, "City");
    type_text(cx, "Automatic");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[location]\nwifi_lookup = true\n");
}

/// Location's time zone: unset, "Automatic" names the computer's zone; a zone picked is
/// written to `[location]`, and "Automatic" removes it again.
#[gpui_kit::test]
fn the_time_zone_is_picked_on_location(cx: &mut TestAppContext) {
    let s = setup(cx, "[location]\nwifi_lookup = true\n");
    open(cx, Some(SettingsPage::Location));
    let automatic = value_of(cx, "Time zone").expect("a time zone row");
    assert!(automatic.starts_with("Automatic ("), "{automatic}");
    click(cx, "Time zone");
    type_text(cx, "Asia/Tokyo");
    press(cx, "enter");
    assert_eq!(
        read(&s.config),
        "[location]\nwifi_lookup = true\ntimezone = \"Asia/Tokyo\"\n"
    );
    assert_eq!(value_of(cx, "Time zone").as_deref(), Some("Asia/Tokyo"));
    click(cx, "Time zone");
    type_text(cx, "Automatic");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[location]\nwifi_lookup = true\n");
    assert_eq!(value_of(cx, "Time zone"), Some(automatic));
}

/// The launcher hotkey goes to the daemon, which writes the file once the system took it.
#[gpui_kit::test]
fn hotkey_is_requested_only_on_enter_or_blur(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(
        value_of(cx, "Launcher hotkey").as_deref(),
        Some("Alt+Space")
    );

    click(cx, "Launcher hotkey");
    press(cx, "ctrl-alt-k");
    assert_eq!(
        value_of(cx, "Launcher hotkey").as_deref(),
        Some("Ctrl+Alt+K")
    );
    assert!(s.requests.borrow().is_empty());
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Launcher,
            Some(hk("ctrl+alt+k")),
            HotkeyOrigin::Settings
        )]
    );
    assert_eq!(read(&s.config), "");
    // Shown as asked for; the release of Enter does not start another recording.
    assert_eq!(
        value_of(cx, "Launcher hotkey").as_deref(),
        Some("Ctrl+Alt+K")
    );

    click(cx, "Launcher hotkey");
    press(cx, "super-l");
    assert_eq!(value_of(cx, "Launcher hotkey").as_deref(), Some("Super+L"));
    assert_eq!(s.requests.borrow().len(), 1);
    // Clicking elsewhere keeps it: here, the sidebar's search box.
    click(cx, "Search...");
    assert_eq!(s.requests.borrow().last().unwrap().1, Some(hk("super+l")));
}

#[gpui_kit::test]
fn esc_restores_the_file_value(cx: &mut TestAppContext) {
    let s = setup(cx, "[launcher]\nhotkey = \"Super+Space\"\n");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Launcher hotkey");
    press(cx, "ctrl-alt-k");
    press(cx, "escape");
    assert_eq!(
        value_of(cx, "Launcher hotkey").as_deref(),
        Some("Super+Space")
    );
    assert!(s.requests.borrow().is_empty());
}

#[gpui_kit::test]
fn resetting_the_launcher_hotkey_asks_for_the_default(cx: &mut TestAppContext) {
    let s = setup(cx, "[launcher]\nhotkey = \"Super+Space\"\n");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Reset Launcher hotkey");
    assert_eq!(
        *s.requests.borrow(),
        [(HotkeyTarget::Launcher, None, HotkeyOrigin::Settings)]
    );
}

#[gpui_kit::test]
fn hotkey_notices_show_in_the_field_while_open(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    let notice = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            cx.update_global::<SettingsController, _>(|c, cx| {
                c.hotkey_notice(HotkeyTarget::Launcher, "Applies after a restart".into(), cx)
            })
        })
    };
    assert!(!notice(cx), "no window: the caller shows it");
    open(cx, Some(SettingsPage::Launcher));
    assert!(notice(cx));
    assert_eq!(
        description_of(cx, "Launcher hotkey").as_deref(),
        Some("Applies after a restart")
    );
}

/// A file that is not TOML: the banner says so in the page's column, where its boxes start,
/// with where the error is in one line.
#[gpui_kit::test]
fn the_banner_of_a_file_that_is_not_toml_is_in_the_page_column(cx: &mut TestAppContext) {
    let _s = setup(cx, "[launcher]\nthis is not = = toml\n");
    open(cx, None);
    let (banner, detail, first_box, saying_it) = on_window(cx, |window, _| {
        (
            window.find("settings-banner"),
            window.find("settings-banner-detail"),
            window.find_all("settings-group-box")[0].bounds(),
            window
                .elements()
                .iter()
                .filter(|e| {
                    [e.label(), e.description()]
                        .into_iter()
                        .flatten()
                        .any(|text| text.contains("invalid TOML at line 2, column "))
                })
                .count(),
        )
    });
    let label = banner.label().unwrap_or_default();
    assert!(label.starts_with("config.toml has an error:"), "{label}");
    assert!(!label.contains('\n'), "{label}");
    assert!(label.contains("invalid TOML at line 2, column "), "{label}");
    // The error is said once: in the banner's name, not again by its muted line.
    assert_eq!(saying_it, 1);
    assert!(detail.bounds().top() > banner.bounds().top());
    assert_eq!(banner.bounds().left(), first_box.left());
    assert_eq!(banner.bounds().right(), first_box.right());
    assert!(banner.bounds().bottom() < first_box.top());
}

#[gpui_kit::test]
fn broken_file_disables_controls_and_shows_the_banner(cx: &mut TestAppContext) {
    let s = setup(cx, "[launcher]\nautostart = false\nhotkey = 5\n");
    open(cx, None);
    let banner = on_window(cx, |window, _| {
        window.find("settings-banner").label().map(str::to_string)
    })
    .unwrap();
    assert!(banner.starts_with("config.toml has an error:"), "{banner}");
    click(cx, "Start at login");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Launcher hotkey");
    press(cx, "ctrl-alt-k");
    press(cx, "enter");
    assert_eq!(
        read(&s.config),
        "[launcher]\nautostart = false\nhotkey = 5\n"
    );
    assert!(s.requests.borrow().is_empty());

    // Fixed by hand: the window recovers by itself.
    std::fs::write(&s.config, "[launcher]\nautostart = false\n").unwrap();
    config_changed(cx);
    assert!(on_window(cx, |window, _| window
        .try_find("settings-banner")
        .is_none()));
    open(cx, Some(SettingsPage::General));
    click(cx, "Start at login");
    assert_eq!(read(&s.config), "[launcher]\nautostart = true\n");
}

/// A path too long for its room is cut at its start at a folder, with an ellipsis, so the
/// file name shows; the whole path is its tooltip and accessible name.
#[gpui_kit::test]
fn a_long_config_path_is_cut_at_a_folder_from_its_start(cx: &mut TestAppContext) {
    let s = setup_in(
        cx,
        "",
        new_core(),
        "a-folder-with-a-rather-long-name/and-another-one/tomakast",
    );
    open(cx, None);
    let full = s.config.display().to_string();
    let path = on_window(cx, |window, _| window.find("config-path"));
    let shown = path.value().unwrap_or_default();
    assert_eq!(path.label(), Some(full.as_str()));
    assert!(shown.starts_with("…/"), "{shown}");
    assert!(full.ends_with(&shown["…".len()..]), "{shown} of {full}");
    assert!(shown.ends_with("/tomakast/config.toml"), "{shown}");
    assert!(
        path.bounds().size.width
            <= gpui_kit::px(tomakast_ui::test_metrics::SETTINGS_CONFIG_PATH_WIDTH),
        "{path:?}"
    );
}

#[gpui_kit::test]
fn unrecognized_keys_are_listed_on_general(cx: &mut TestAppContext) {
    let s = setup(cx, "zz = 1\n[launcher]\nautostart = false\n");
    open(cx, None);
    assert!(has_label(cx, "Remove zz"));
    click(cx, "Remove zz");
    assert_eq!(read(&s.config), "[launcher]\nautostart = false\n");
    assert!(!has_label(cx, "Remove zz"));
}

#[gpui_kit::test]
fn unrecognized_keys_of_location_are_on_its_page(cx: &mut TestAppContext) {
    let _s = setup(cx, "[location]\nzone = \"x\"\n");
    open(cx, None);
    assert!(!has_label(cx, "Remove location.zone"));
    open(cx, Some(SettingsPage::Location));
    assert!(has_label(cx, "Remove location.zone"));
}

/// A `location` that is not a table is listed on Location alone, and can be removed.
#[gpui_kit::test]
fn a_location_that_is_not_a_table_is_on_its_page(cx: &mut TestAppContext) {
    let s = setup(cx, "location = 5\n[launcher]\nautostart = false\n");
    open(cx, None);
    assert!(!has_label(cx, "Remove location"));
    open(cx, Some(SettingsPage::Location));
    assert!(has_text(cx, "location = 5"));
    click(cx, "Remove location");
    assert_eq!(read(&s.config), "[launcher]\nautostart = false\n");
    assert!(!has_label(cx, "Remove location"));
}

/// The same for `home`, on Home.
#[gpui_kit::test]
fn a_home_that_is_not_a_table_is_on_its_page(cx: &mut TestAppContext) {
    let s = setup(cx, "home = 5\n");
    open(cx, None);
    assert!(!has_label(cx, "Remove home"));
    open(cx, Some(SettingsPage::Home));
    click(cx, "Remove home");
    assert_eq!(read(&s.config), "");
}

#[gpui_kit::test]
fn invalid_builtin_value_shows_error_state(cx: &mut TestAppContext) {
    let _s = setup(cx, "[location]\ncity = 5\n");
    open(cx, Some(SettingsPage::Location));
    assert!(has_label(cx, "Not valid here: 5; the default is used"));
    assert!(has_label(cx, "Reset City"));
}

#[gpui_kit::test]
fn an_outside_edit_updates_the_controls(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, None);
    std::fs::write(
        &s.config,
        "[launcher]\nautostart = false\nhotkey = \"Super+K\"\n",
    )
    .unwrap();
    config_changed(cx);
    assert_eq!(checked(cx, "Start at login"), Some(false));
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(value_of(cx, "Launcher hotkey").as_deref(), Some("Super+K"));
}

fn set_catalog(cx: &mut TestAppContext, catalog: PluginCatalog) {
    cx.update(|cx| cx.update_global::<SettingsController, _>(|c, cx| c.set_catalog(catalog, cx)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_plugin_page_opens_by_id_and_shows_its_state(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(
        cx,
        PluginCatalog {
            plugins: vec![CatalogPlugin {
                id: "weather".into(),
                name: "Weather".into(),
                version: "1.0.0".into(),
                author: Some("tomakast".into()),
                state: PluginState::Review,
                preferences: vec![],
                commands: vec![],
                widgets: vec![],
                param_titles: vec![],
            }],
            unavailable: None,
        },
    );
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(has_label(cx, "Waiting for your approval in the launcher"));
    // A plugin that is not installed has no page: the page on display stays.
    open(cx, Some(SettingsPage::Plugin("calc".into())));
    assert!(has_label(cx, "Waiting for your approval in the launcher"));
}

/// The weather plugin as the folder listing gives it when the plugin runtime is down.
fn weather_not_running() -> CatalogPlugin {
    let wind = ValueKind::Choice(vec![
        ChoiceOption {
            value: "kmh".into(),
            title: "km/h".into(),
        },
        ChoiceOption {
            value: "ms".into(),
            title: "m/s".into(),
        },
    ]);
    CatalogPlugin {
        state: PluginState::NotRunning("no database".into()),
        preferences: vec![PluginPreference {
            default: Some("kmh".into()),
            ..pref("wind", "Wind", None, wind)
        }],
        ..plugin("weather", "Weather")
    }
}

#[gpui_kit::test]
fn plugin_pages_are_editable_when_plugins_do_not_run(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather_not_running()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(has_label(cx, "Not running: no database"));
    click(cx, "Wind");
    click(cx, "m/s");
    assert_eq!(read(&s.config), "[plugins.weather]\nwind = \"ms\"\n");
}

/// The titles of the sidebar's entries, top to bottom.
fn pages(cx: &mut TestAppContext) -> Vec<String> {
    on_window(cx, |window, _| {
        window
            .within("settings-sidebar")
            .find_all_by_role(gpui_kit::Role::ListItem)
            .iter()
            .filter_map(|entry| entry.label().map(str::to_string))
            .collect()
    })
}

const TABLES: &str = "[plugins.weather]\nunits = \"metric\"\n\n[plugins.gone]\nx = 1\n";

#[gpui_kit::test]
fn no_plugin_pages_until_the_plugins_are_listed(cx: &mut TestAppContext) {
    let _s = setup(cx, TABLES);
    open(cx, None);
    assert_eq!(
        pages(cx),
        [
            "General",
            "Launcher",
            "Appearance",
            "Location",
            "Hotkeys",
            "Widgets",
            "About"
        ]
    );
    assert!(!has_label(cx, "Plugins"));
    set_catalog(cx, catalog(vec![weather()]));
    // The listed plugin, and the table of the one that is not installed.
    assert_eq!(
        pages(cx),
        [
            "General",
            "Launcher",
            "Appearance",
            "Location",
            "Hotkeys",
            "Widgets",
            "About",
            "Weather",
            "gone (not installed)"
        ]
    );
}

#[gpui_kit::test]
fn no_plugin_pages_when_the_folder_cannot_be_listed(cx: &mut TestAppContext) {
    let _s = setup(cx, TABLES);
    open(cx, None);
    set_catalog(
        cx,
        PluginCatalog {
            plugins: vec![],
            unavailable: Some("cannot list /plugins: denied".into()),
        },
    );
    assert_eq!(
        pages(cx),
        [
            "General",
            "Launcher",
            "Appearance",
            "Location",
            "Hotkeys",
            "Widgets",
            "About"
        ]
    );
    assert!(!has_label(cx, "Plugins"));
    assert!(
        !texts(cx)
            .iter()
            .any(|t| t.contains("Plugins are not running"))
    );
}

#[gpui_kit::test]
fn an_outside_edit_back_to_the_window_s_own_text_is_shown(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, None);
    click(cx, "Start at login");
    let own = read(&s.config);
    assert_eq!(own, "[launcher]\nautostart = false\n");
    config_changed(cx);
    std::fs::write(&s.config, "[launcher]\nautostart = true\n").unwrap();
    config_changed(cx);
    assert_eq!(checked(cx, "Start at login"), Some(true));
    // Undone in the editor: the file holds the window's own text again.
    std::fs::write(&s.config, &own).unwrap();
    config_changed(cx);
    assert_eq!(checked(cx, "Start at login"), Some(false));
}

/// Whether a line starting with "Not saved:" is shown.
fn not_saved(cx: &mut TestAppContext) -> bool {
    on_window(cx, |window, _| {
        window
            .elements()
            .iter()
            .any(|e| e.label().is_some_and(|l| l.starts_with("Not saved:")))
    })
}

/// Valid TOML the launcher refuses: no edit of it is written.
const REFUSED: &str = "zz = 1\n[launcher]\nhotkey = 5\n";

#[gpui_kit::test]
fn a_failed_write_says_so_under_the_switch(cx: &mut TestAppContext) {
    let s = setup(cx, "zz = 1\n");
    open(cx, None);
    // Broken on disk before the window heard of it.
    std::fs::write(&s.config, REFUSED).unwrap();
    click(cx, "Start at login");
    assert_eq!(read(&s.config), REFUSED);
    assert!(not_saved(cx));
}

#[gpui_kit::test]
fn a_failed_removal_says_so_under_its_row(cx: &mut TestAppContext) {
    let s = setup(cx, "zz = 1\n");
    open(cx, None);
    std::fs::write(&s.config, REFUSED).unwrap();
    click(cx, "Remove zz");
    assert_eq!(read(&s.config), REFUSED);
    assert!(has_label(cx, "Remove zz"));
    assert!(not_saved(cx));
}

#[gpui_kit::test]
fn a_failed_city_pick_shows_the_file_s_city_again(cx: &mut TestAppContext) {
    let s = setup(cx, "[location]\ncity = \"Paris\"\n");
    open(cx, Some(SettingsPage::Location));
    let before = value_of(cx, "City");
    assert_eq!(before.as_deref(), Some("Paris"));
    std::fs::write(
        &s.config,
        "[location]\ncity = \"Paris\"\n[launcher]\nhotkey = 5\n",
    )
    .unwrap();
    click(cx, "City");
    type_text(cx, "Berlin");
    press(cx, "enter");
    assert!(!read(&s.config).contains("Berlin"));
    assert!(not_saved(cx));
    assert_eq!(value_of(cx, "City"), before);
}

#[gpui_kit::test]
fn automatic_removes_a_city_of_the_wrong_type(cx: &mut TestAppContext) {
    let s = setup(cx, "[location]\ncity = 5\nwifi_lookup = true\n");
    open(cx, Some(SettingsPage::Location));
    click(cx, "City");
    type_text(cx, "Automatic");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[location]\nwifi_lookup = true\n");
}

#[gpui_kit::test]
fn esc_shows_the_hotkey_the_file_has_now(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Launcher hotkey");
    press(cx, "ctrl-alt-k");
    std::fs::write(&s.config, "[launcher]\nhotkey = \"Super+K\"\n").unwrap();
    config_changed(cx);
    assert_eq!(
        value_of(cx, "Launcher hotkey").as_deref(),
        Some("Ctrl+Alt+K")
    );
    press(cx, "escape");
    assert_eq!(value_of(cx, "Launcher hotkey").as_deref(), Some("Super+K"));
}

// Plugin pages.

use tomakast_core::settings_schema::{ChoiceOption, NumberRange, ValueKind};
use tomakast_core::{CatalogCommand, ItemId, PluginPreference};

fn pref(key: &str, title: &str, description: Option<&str>, kind: ValueKind) -> PluginPreference {
    PluginPreference {
        key: key.into(),
        title: title.into(),
        description: description.map(str::to_string),
        kind,
        default: None,
        reported_default: None,
    }
}

fn plugin(id: &str, name: &str) -> CatalogPlugin {
    CatalogPlugin {
        id: id.into(),
        name: name.into(),
        version: "1.0.0".into(),
        author: Some("tomakast".into()),
        state: PluginState::Running,
        preferences: vec![],
        commands: vec![],
        widgets: vec![],
        param_titles: vec![],
    }
}

/// Weather: one preference of each kind and one command.
fn weather() -> CatalogPlugin {
    let units = ValueKind::Choice(vec![
        ChoiceOption {
            value: "metric".into(),
            title: "Metric".into(),
        },
        ChoiceOption {
            value: "imperial".into(),
            title: "Imperial".into(),
        },
    ]);
    let precision = ValueKind::Number(NumberRange {
        min: Some(0.),
        max: Some(20.),
        step: Some(1.),
        integer: true,
    });
    CatalogPlugin {
        preferences: vec![
            PluginPreference {
                default: Some("Ann".into()),
                ..pref(
                    "greeting",
                    "Greeting",
                    Some("Who the forecast greets."),
                    ValueKind::Text,
                )
            },
            PluginPreference {
                default: Some("true".into()),
                ..pref(
                    "alerts",
                    "Alerts",
                    Some("Warn about storms."),
                    ValueKind::Bool,
                )
            },
            PluginPreference {
                default: Some("metric".into()),
                ..pref("units", "Units", Some("How values are shown."), units)
            },
            PluginPreference {
                default: Some("10".into()),
                ..pref("precision", "Precision", Some("Digits shown."), precision)
            },
        ],
        commands: vec![CatalogCommand {
            id: "forecast".into(),
            title: "Forecast".into(),
            icon: Some("cloud".into()),
        }],
        ..plugin("weather", "Weather")
    }
}

fn catalog(plugins: Vec<CatalogPlugin>) -> PluginCatalog {
    PluginCatalog {
        plugins,
        unavailable: None,
    }
}

/// The text of every label shown: titles, descriptions, group titles.
fn texts(cx: &mut TestAppContext) -> Vec<String> {
    on_window(cx, |window, _| {
        window
            .find_all_by_role(gpui_kit::Role::Label)
            .iter()
            .filter_map(|e| e.value().map(str::to_string))
            .collect()
    })
}

fn has_text(cx: &mut TestAppContext, text: &str) -> bool {
    texts(cx).iter().any(|t| t == text)
}

fn has_description(cx: &mut TestAppContext, text: &str) -> bool {
    on_window(cx, |window, _| {
        window
            .elements()
            .iter()
            .any(|e| e.description() == Some(text))
    })
}

/// Clicks the sidebar entry titled `title`.
fn click_page(cx: &mut TestAppContext, title: &str) {
    on_window(cx, |window, cx| {
        let at = window
            .within("settings-sidebar")
            .find_by_label(title)
            .bounds()
            .center();
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn plugin_page_shows_each_kind(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    for text in ["Greeting", "Alerts", "Units", "Precision"] {
        assert!(has_text(cx, text), "{text}: {:?}", texts(cx));
    }
    for text in [
        "Who the forecast greets.",
        "Warn about storms.",
        "How values are shown.",
        "Digits shown.",
    ] {
        assert!(has_description(cx, text), "{text}");
    }
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Ann"));
    assert_eq!(checked(cx, "Alerts"), Some(true));
    assert!(shows(cx, "Units", "Metric"));
    let precision = on_window(cx, |window, _| window.find_by_label("Precision").role());
    assert_eq!(precision, Some(gpui_kit::Role::SpinButton));
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Add hotkey"));
    assert!(has_label(cx, "Version 1.0.0 by tomakast"));
}

/// A page lines up: its title, its headings and its notes start where the rows' text does (the
/// title also in a wide window, where the column is centred); its wide controls
/// share one left edge with the hotkey fields, and every one-line control one width.
#[gpui_kit::test]
fn a_plugin_page_lines_up(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    let title_and_box = |cx: &mut TestAppContext| {
        on_window(cx, |window, _| {
            (
                window.find("page-title").bounds().left(),
                window.find_by_label("Preferences").bounds().left(),
                window.find_all("settings-group-box")[0].bounds().left(),
            )
        })
    };
    let (title, heading, _) = title_and_box(cx);
    assert_eq!(title, heading, "the title and the headings");
    let (heading, note, name) = on_window(cx, |window, _| {
        (
            window.find_by_label("Preferences").bounds().left(),
            window
                .find_by_label("Version 1.0.0 by tomakast")
                .bounds()
                .left(),
            window.find_by_label("Name").bounds().left(),
        )
    });
    assert_eq!(heading, name, "the heading and the rows' text");
    assert_eq!(note, name, "a note and the rows' text");
    let (greeting, precision, forecast, units) = on_window(cx, |window, _| {
        (
            window.find_by_label("Greeting").bounds(),
            window.find_by_label("Precision").bounds(),
            window.find_by_label("Forecast").bounds(),
            window.find_by_label("Units").bounds(),
        )
    });
    // A text field is labelled at its text, which starts at the inset.
    let inset = px(tomakast_ui::test_metrics::CONTROL_TEXT_INSET);
    assert_eq!(
        greeting.left() - inset,
        forecast.left(),
        "{greeting:?} {forecast:?}"
    );
    // A number field is labelled inside its 1 px border.
    assert_eq!(
        precision.left() - px(1.),
        forecast.left(),
        "{precision:?} {forecast:?}"
    );
    // Every one-line control is as wide as the others: a dropdown as a hotkey field, flush.
    let width = px(tomakast_ui::test_metrics::SETTINGS_CONTROL_WIDTH);
    assert_eq!(units.size.width, width, "{units:?}");
    assert_eq!(forecast.size.width, width, "{forecast:?}");
    assert_eq!(units.right(), forecast.right(), "{units:?} {forecast:?}");
    let w = window(cx);
    cx.simulate_window_resize(w, size(px(1600.), px(900.)));
    cx.run_until_parked();
    let (title, heading, first_box) = title_and_box(cx);
    assert!(first_box > px(400.), "the column is centred: {first_box:?}");
    assert_eq!(title, heading, "the title and the headings");
}

#[gpui_kit::test]
fn a_switch_preference_writes_a_boolean(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Alerts");
    assert_eq!(read(&s.config), "[plugins.weather]\nalerts = false\n");
    assert!(has_label(cx, "Reset Alerts"));
    click(cx, "Reset Alerts");
    assert_eq!(read(&s.config), "");
}

#[gpui_kit::test]
fn choice_writes_the_option_value(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Units");
    click(cx, "Imperial");
    assert_eq!(read(&s.config), "[plugins.weather]\nunits = \"imperial\"\n");
    assert!(shows(cx, "Units", "Imperial"));
}

#[gpui_kit::test]
fn number_writes_a_toml_number(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Precision");
    press(cx, "end");
    press(cx, "backspace");
    press(cx, "backspace");
    type_text(cx, "12");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.weather]\nprecision = 12\n");
}

#[gpui_kit::test]
fn a_text_preference_reaches_the_file_on_enter_only(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "a");
    assert_eq!(read(&s.config), "");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.weather]\ngreeting = \"Anna\"\n");
}

#[gpui_kit::test]
fn invalid_plugin_value_shows_error_state(cx: &mut TestAppContext) {
    let s = setup(
        cx,
        "[plugins.weather]\nunits = \"kelvin\"\nprecision = \"high\"\n",
    );
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(has_label(cx, "Not valid here: \"kelvin\""));
    assert!(has_label(cx, "Not valid here: \"high\""));
    // The controls show the default in effect; nothing is rewritten, not even when the
    // number field is entered and left.
    assert!(shows(cx, "Units", "Metric"));
    click(cx, "Precision");
    press(cx, "enter");
    click(cx, "Search...");
    assert_eq!(
        read(&s.config),
        "[plugins.weather]\nunits = \"kelvin\"\nprecision = \"high\"\n"
    );
    click(cx, "Reset Units");
    assert_eq!(read(&s.config), "[plugins.weather]\nprecision = \"high\"\n");
}

#[gpui_kit::test]
fn undeclared_keys_are_read_only_with_remove(cx: &mut TestAppContext) {
    let s = setup(cx, "[plugins.weather]\nold = 5\nunits = \"metric\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(
        has_text(cx, "The plugin does not declare these keys."),
        "{:?}",
        texts(cx)
    );
    assert!(has_label(cx, "old = 5"));
    click(cx, "Remove old");
    assert_eq!(read(&s.config), "[plugins.weather]\nunits = \"metric\"\n");
    assert!(!has_label(cx, "old = 5"));
}

#[gpui_kit::test]
fn not_installed_plugin_has_remove_all(cx: &mut TestAppContext) {
    let s = setup(
        cx,
        "[launcher]\nautostart = false\n\n[plugins.gone]\na = 1\nb = \"x\"\n",
    );
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("gone".into())));
    assert!(has_label(cx, "a = 1"));
    assert!(has_label(cx, "b = \"x\""));
    click(cx, "Remove b");
    assert_eq!(
        read(&s.config),
        "[launcher]\nautostart = false\n\n[plugins.gone]\na = 1\n"
    );
    click(cx, "Remove all settings of gone");
    assert_eq!(read(&s.config), "[launcher]\nautostart = false\n");
    // Its page is gone; the window shows another.
    assert!(!has_label(cx, "a = 1"));
}

#[gpui_kit::test]
fn manifest_error_page_has_no_remove_all(cx: &mut TestAppContext) {
    let _s = setup(cx, "[plugins.broken]\na = 1\n");
    let broken = CatalogPlugin {
        version: String::new(),
        author: None,
        state: PluginState::ManifestError("bad toml".into()),
        ..plugin("broken", "broken")
    };
    set_catalog(cx, catalog(vec![broken]));
    open(cx, Some(SettingsPage::Plugin("broken".into())));
    assert!(has_label(cx, "Manifest error: bad toml"));
    assert!(has_label(cx, "a = 1"));
    assert!(!has_label(cx, "Remove a"));
    assert!(!has_label(cx, "Remove all settings of broken"));
}

#[gpui_kit::test]
fn command_hotkey_field_sends_a_request(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Forecast");
    press(cx, "ctrl-alt-w");
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(ItemId::new("weather", "forecast")),
            Some(hk("ctrl+alt+w")),
            HotkeyOrigin::Settings
        )]
    );
}

#[gpui_kit::test]
fn a_command_hotkey_in_the_file_shows_and_clears(cx: &mut TestAppContext) {
    let s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Ctrl+Alt+W"));
    click(cx, "Clear hotkey of Forecast");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(ItemId::new("weather", "forecast")),
            None,
            HotkeyOrigin::Settings
        )]
    );
}

#[gpui_kit::test]
fn a_catalog_change_keeps_the_page_on_display(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(shows(cx, "Units", "Metric"));
    // A plugin sorted before Weather arrives: Weather's page moves down one.
    set_catalog(cx, catalog(vec![plugin("alarm", "Alarm"), weather()]));
    assert!(shows(cx, "Units", "Metric"));
    // And leaves again.
    set_catalog(cx, catalog(vec![weather()]));
    assert!(shows(cx, "Units", "Metric"));
}

#[gpui_kit::test]
fn a_plugin_page_asked_for_before_the_catalog_lists_it_opens_once_listed(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(has_label(cx, "Start at login"));
    set_catalog(cx, catalog(vec![weather()]));
    assert!(shows(cx, "Units", "Metric"));
}

#[gpui_kit::test]
fn plugin_pages_come_by_name_then_tables_without_a_plugin(cx: &mut TestAppContext) {
    let _s = setup(cx, "[plugins.zed]\na = 1\n[plugins.beta]\nb = 2\n");
    set_catalog(cx, catalog(vec![plugin("alarm", "Alarm"), weather()]));
    open(cx, None);
    click_page(cx, "Weather");
    assert!(shows(cx, "Units", "Metric"));
    click_page(cx, "beta (not installed)");
    assert!(has_label(cx, "b = 2"));
    click_page(cx, "zed (not installed)");
    assert!(has_label(cx, "a = 1"));
}

/// A command's notice shows in the command's field, not in the launcher hotkey's.
#[gpui_kit::test]
fn a_command_s_notice_shows_in_its_field(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Forecast");
    press(cx, "ctrl-alt-w");
    press(cx, "enter");
    // Enter kept the combination: the field is not recording again.
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Ctrl+Alt+W"));
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| {
            c.hotkey_notice(
                HotkeyTarget::Item(ItemId::new("weather", "forecast")),
                "Refused by the desktop".into(),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert_eq!(
        description_of(cx, "Forecast").as_deref(),
        Some("Refused by the desktop")
    );
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Add hotkey"));
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(description_of(cx, "Launcher hotkey"), None);
}

/// A command hotkey saved for after a restart says so.
#[gpui_kit::test]
fn a_command_hotkey_for_after_a_restart_says_so(cx: &mut TestAppContext) {
    use tomakast_core::hotkeys::{HotkeyState, HotkeyTable, ItemHotkey};
    let _s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    let table = HotkeyTable {
        items: vec![ItemHotkey {
            item: ItemId::new("weather", "forecast"),
            hotkey: hk("ctrl+alt+w"),
            state: HotkeyState::AfterRestart,
        }],
        ..HotkeyTable::default()
    };
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| c.set_hotkey_table(table, cx))
    });
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert_eq!(
        description_of(cx, "Forecast").as_deref(),
        Some("Applies after a restart")
    );
}

/// Emptying a text preference removes the key; the field then shows the default in effect.
#[gpui_kit::test]
fn an_emptied_text_preference_falls_back_to_the_default(cx: &mut TestAppContext) {
    let s = setup(cx, "[plugins.weather]\ngreeting = \"Bob\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Greeting");
    press(cx, "end");
    for _ in 0..3 {
        press(cx, "backspace");
    }
    press(cx, "enter");
    assert_eq!(read(&s.config), "");
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Ann"));
}

/// Out of range: refused with a hint on Enter and on blur alike; the typed text stays and
/// the file is not touched.
#[gpui_kit::test]
fn an_out_of_range_number_is_refused_on_enter_and_on_blur(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Precision");
    press(cx, "end");
    type_text(cx, "0");
    press(cx, "enter");
    assert_eq!(read(&s.config), "");
    assert!(has_label(cx, "Enter a whole number from 0 to 20."));
    click(cx, "Search...");
    assert_eq!(read(&s.config), "");
    assert!(has_label(cx, "Enter a whole number from 0 to 20."));
    // Still typed: fixed and confirmed, it is written.
    click(cx, "Precision");
    press(cx, "end");
    press(cx, "backspace");
    press(cx, "backspace");
    type_text(cx, "2");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.weather]\nprecision = 12\n");
}

/// A number typed but not confirmed is not written.
#[gpui_kit::test]
fn typing_in_a_number_field_leaves_the_file_alone(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Precision");
    press(cx, "end");
    press(cx, "backspace");
    assert_eq!(read(&s.config), "");
}

/// Leaving a text preference writes it.
#[gpui_kit::test]
fn a_text_preference_is_written_on_blur(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "e");
    assert_eq!(read(&s.config), "");
    click(cx, "Search...");
    assert_eq!(read(&s.config), "[plugins.weather]\ngreeting = \"Anne\"\n");
}

/// A plugin page asked for early does not pull the user away from a page they went to.
#[gpui_kit::test]
fn a_pending_plugin_page_is_dropped_once_the_user_changes_page(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click_page(cx, "Location");
    assert!(has_label(cx, "City"));
    set_catalog(cx, catalog(vec![weather()]));
    assert!(has_label(cx, "City"));
    assert!(!shows(cx, "Units", "Metric"));
}

/// A renamed preference or command: the reused field takes the new name.
#[gpui_kit::test]
fn a_renamed_preference_renames_its_field(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    let mut renamed = weather();
    renamed.preferences[0].title = "Salutation".into();
    renamed.commands[0].title = "Outlook".into();
    set_catalog(cx, catalog(vec![renamed]));
    assert!(has_label(cx, "Salutation"));
    assert!(has_label(cx, "Outlook"));
    assert!(!has_label(cx, "Greeting"));
}

/// A choice with neither a default nor a value says so.
#[gpui_kit::test]
fn a_choice_without_a_value_shows_not_set(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    let mut plugin = weather();
    plugin.preferences[2].default = None;
    set_catalog(cx, catalog(vec![plugin]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(shows(cx, "Units", "Not set"));
}

/// A plugin that tells defaults as it runs works out the ones it has not told: its choice
/// without a value is "Automatic" rather than "Not set".
#[gpui_kit::test]
fn a_choice_of_a_plugin_that_tells_defaults_shows_automatic(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    let mut plugin = weather();
    plugin.preferences[2].default = None;
    plugin.preferences[0].reported_default = Some("Bea".into());
    set_catalog(cx, catalog(vec![plugin]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(shows(cx, "Units", "Automatic"));
}

/// An empty `[plugins.x]` of no plugin still has its page, with Remove all.
#[gpui_kit::test]
fn an_empty_table_of_no_plugin_can_be_removed(cx: &mut TestAppContext) {
    let s = setup(cx, "[plugins.gone]\n");
    set_catalog(cx, catalog(vec![]));
    open(cx, Some(SettingsPage::Plugin("gone".into())));
    click(cx, "Remove all settings of gone");
    assert_eq!(read(&s.config), "");
}

// The Hotkeys page.

use tomakast_core::hotkeys::{HotkeyState, HotkeyTable, ItemHotkey};
use tomakast_core::index::Action;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{Item, ItemKind};

fn fake_item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("demo", local),
        title: title.into(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        source: None,
        actions: vec![Action {
            id: "open".into(),
            title: "Open".into(),
            shortcut: None,
        }],
    }
}

/// As [`setup`], over a core that indexed Firefox, Files and Terminal (`demo/<name>`).
fn setup_items(cx: &mut TestAppContext, text: &str) -> Setup {
    let mut core = new_core();
    let items = vec![
        fake_item("firefox", "Firefox"),
        fake_item("files", "Files"),
        fake_item("terminal", "Terminal"),
    ];
    // The built-in demo module's provider: its group is "Demo".
    core.register(Box::new(
        FakeModule::new(items, || Ok(Vec::new())).with_id("demo"),
    ));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    setup_with(cx, text, core)
}

fn fake(local: &str) -> ItemId {
    ItemId::new("demo", local)
}

/// The launcher on Alt+Space and these item hotkeys, as the daemon applied them.
fn table(items: &[(&str, &str, HotkeyState)]) -> HotkeyTable {
    HotkeyTable {
        launcher: Some(hk("alt+space")),
        launcher_state: Some(HotkeyState::Active { trigger: None }),
        items: items
            .iter()
            .map(|(local, hotkey, state)| ItemHotkey {
                item: fake(local),
                hotkey: hk(hotkey),
                state: state.clone(),
            })
            .collect(),
        ..HotkeyTable::default()
    }
}

fn set_table(cx: &mut TestAppContext, table: HotkeyTable) {
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| c.set_hotkey_table(table, cx))
    });
    cx.run_until_parked();
}

/// The daemon's answer to the change of `target` asked for in the window.
fn notice(cx: &mut TestAppContext, target: HotkeyTarget, text: &str) {
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| c.hotkey_notice(target, text.into(), cx))
    });
    cx.run_until_parked();
}

fn top(cx: &mut TestAppContext, label: &str) -> gpui_kit::Pixels {
    on_window(cx, |window, _| window.find_by_label(label).bounds().top())
}

const ACTIVE: HotkeyState = HotkeyState::Active { trigger: None };

/// Every `[hotkeys]` entry by its item's title, or, when the item is not indexed, by its id
/// without its source (and an application's without `.desktop`); the launcher hotkey is on
/// General, not here.
#[gpui_kit::test]
fn hotkeys_page_lists_the_items(cx: &mut TestAppContext) {
    let _s = setup_items(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n\"Ctrl+Alt+G\" = \"apps/gone.desktop\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Ctrl+Alt+F"));
    assert_eq!(value_of(cx, "gone").as_deref(), Some("Ctrl+Alt+G"));
    assert!(!has_label(cx, "apps/gone.desktop"));
    // Applications come first.
    assert!(top(cx, "gone") < top(cx, "Firefox"));
    assert!(!has_label(cx, "Add hotkey…"));
}

/// Applications, as the launcher's apps module indexes them.
fn app_item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("apps", local),
        ..fake_item(local, title)
    }
}

/// As [`setup`], over a core whose applications are Firefox and Terminal.
fn setup_apps(cx: &mut TestAppContext, text: &str) -> Setup {
    let mut core = new_core();
    let apps = vec![
        app_item("firefox.desktop", "Firefox"),
        app_item("terminal.desktop", "Terminal"),
    ];
    core.register(Box::new(
        FakeModule::new(apps, || Ok(Vec::new())).with_id("apps"),
    ));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    setup_with(cx, text, core)
}

fn expanded(cx: &mut TestAppContext, label: &str) -> Option<bool> {
    on_window(cx, |window, _| window.find_by_label(label).expanded())
}

const APPLICATIONS: &str = "Applications, 2 items";
const WEATHER: &str = "Weather, 1 item";

/// Items in groups by where they come from, Applications first, each a table under the titles
/// of its columns; no "Add hotkey…".
#[gpui_kit::test]
fn hotkeys_lists_items_by_source(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    assert!(has_label(cx, APPLICATIONS));
    assert!(has_label(cx, WEATHER));
    assert!(top(cx, APPLICATIONS) < top(cx, WEATHER));
    assert!(!has_label(cx, "Add hotkey…"));
    assert!(has_label(cx, "Name"));
    assert!(has_label(cx, "Hotkey"));
    assert!(top(cx, "Firefox") < top(cx, "Terminal"));
    // The "Name" title starts where the group rows' chevrons do.
    columns_line_up(cx, "group-chevron");
}

/// The "Hotkey" column title starts where the text in the first hotkey field under it does,
/// the "Name" title where the first element `name_over` (a row's icon, a group row's chevron)
/// does.
fn columns_line_up(cx: &mut TestAppContext, name_over: &'static str) {
    let (name, hotkey, first, text) = on_window(cx, |window, _| {
        (
            window.find_by_label("Name").bounds(),
            window.find_by_label("Hotkey").bounds(),
            window.find_all(name_over)[0].bounds(),
            window.find_all("hotkey-placeholder")[0].bounds(),
        )
    });
    assert_eq!(hotkey.left(), text.left(), "{hotkey:?} {text:?}");
    assert_eq!(name.left(), first.left(), "{name:?} {first:?}");
}

/// The Hotkeys page is one table: one box, the titles of its columns once at its top, a row
/// for each source in it, "Not used" the last of them.
#[gpui_kit::test]
fn the_hotkeys_page_is_one_table(cx: &mut TestAppContext) {
    let _s = setup_apps(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n\"Ctrl+Alt+F\" = \"apps/firefox.desktop\"\n\"ctrl+alt+g\" = \"apps/firefox.desktop\"\n",
    );
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    assert_eq!(expanded(cx, WEATHER), Some(true));
    const NOT_USED: &str = "Not used, 1 item";
    assert_eq!(expanded(cx, NOT_USED), Some(true));
    let (boxes, columns) = on_window(cx, |window, _| {
        (
            window
                .find_all("settings-group-box")
                .iter()
                .map(|b| b.bounds())
                .collect::<Vec<_>>(),
            window.find_all("column-left").len(),
        )
    });
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert_eq!(columns, 1);
    let inside = |label: &str, cx: &mut TestAppContext| {
        let b = bounds_of(cx, label);
        b.top() >= boxes[0].top() && b.bottom() <= boxes[0].bottom()
    };
    for label in [APPLICATIONS, "Firefox", WEATHER, "Forecast", NOT_USED] {
        assert!(inside(label, cx), "{label} outside the table");
    }
    assert!(top(cx, APPLICATIONS) < top(cx, WEATHER));
    assert!(top(cx, WEATHER) < top(cx, NOT_USED));
    assert!(top(cx, NOT_USED) < top(cx, "Remove Ctrl+Alt+G of Firefox"));
    // A group row is a little taller than an item row.
    let header = bounds_of(cx, APPLICATIONS).size.height;
    let pitch = top(cx, "Terminal") - top(cx, "Firefox");
    assert!(header > pitch - gpui_kit::px(1.), "{header:?} {pitch:?}");
}

/// An item's icon starts where its group row's title does, the rows of "Not used" too.
#[gpui_kit::test]
fn an_item_s_icon_lines_up_with_its_group_s_title(cx: &mut TestAppContext) {
    let _s = setup_apps(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+F\" = \"apps/firefox.desktop\"\n\"ctrl+alt+g\" = \"apps/firefox.desktop\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    let (titles, item, dropped) = on_window(cx, |window, _| {
        (
            window
                .find_all("group-title")
                .iter()
                .map(|t| t.bounds())
                .collect::<Vec<_>>(),
            window.find_all("item")[0].bounds(),
            window.find("item-dropped").bounds(),
        )
    });
    assert_eq!(titles.len(), 2, "{titles:?}");
    assert_eq!(item.left(), titles[0].left(), "{item:?} {:?}", titles[0]);
    assert_eq!(
        dropped.left(),
        titles[1].left(),
        "{dropped:?} {:?}",
        titles[1]
    );
}

/// A group row and its first item are split by a hairline, as two rows are.
#[gpui_kit::test]
fn a_hairline_splits_a_group_row_from_its_first_item(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "");
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    let hairline_at = |cx: &mut TestAppContext, y: gpui_kit::Pixels| {
        on_window(cx, |window, _| {
            let scale = window.scale_factor();
            let table = window.find("settings-group-box").bounds().scale(scale);
            let y = f32::from(y) * scale;
            window.painted_quads().iter().any(|q| {
                q.border_widths.top.0 > 0.
                    && q.border_color.a > 0.
                    && (q.bounds.top().0 - y).abs() < 1.5
                    && q.bounds.size.width.0 > table.size.width.0 / 2.
            })
        })
    };
    // Under the group row, and between its first two items.
    let header = bounds_of(cx, APPLICATIONS).bottom();
    assert!(hairline_at(cx, header), "no hairline under the group row");
    let pitch = top(cx, "Terminal") - top(cx, "Firefox");
    assert!(
        hairline_at(cx, header + pitch),
        "no hairline between the items"
    );
}

/// Whether a fill that is not clear is painted with the bounds of the element labelled
/// `label`.
fn filled(cx: &mut TestAppContext, label: &str) -> bool {
    on_window(cx, |window, _| {
        let scale = window.scale_factor();
        let bounds = window.find_by_label(label).bounds().scale(scale);
        let near = |a: gpui_kit::ScaledPixels, b: gpui_kit::ScaledPixels| (a.0 - b.0).abs() < 1.5;
        window.painted_quads().iter().any(|q| {
            near(q.bounds.left(), bounds.left())
                && near(q.bounds.right(), bounds.right())
                && near(q.bounds.top(), bounds.top())
                && near(q.bounds.bottom(), bounds.bottom())
                && q.background.as_solid().is_some_and(|c| c.a > 0.)
        })
    })
}

/// A group row has no fill of its own; the pointer over it tints it.
#[gpui_kit::test]
fn group_rows_have_no_fill_at_rest(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    for label in [APPLICATIONS, WEATHER] {
        assert!(!filled(cx, label), "{label} filled at rest");
    }
    move_pointer_to_center_of(cx, WEATHER);
    assert!(filled(cx, WEATHER), "no tint under the pointer");
    assert!(!filled(cx, APPLICATIONS));
}

/// The rows of an open group under a long one are drawn where they are when it scrolls into
/// view: none in view is missing, none is out of place.
#[gpui_kit::test]
fn rows_of_a_later_group_are_drawn_where_they_are(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let mut core = new_core();
    let apps = (0..130)
        .map(|ix| app_item(&format!("app{ix:03}.desktop"), &app(ix)))
        .collect();
    let demo = (0..60)
        .map(|ix| fake_item(&format!("d{ix:02}"), &format!("Demo item {ix:02}")))
        .collect();
    for (id, items) in [("apps", apps), ("demo", demo)] {
        core.register(Box::new(
            FakeModule::new(items, || Ok(Vec::new())).with_id(id),
        ));
    }
    core.start_modules();
    for _ in 0..2 {
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    let _s = setup_with(cx, "[hotkeys]\n\"Ctrl+Alt+D\" = \"demo/d00\"\n", core);
    open(cx, Some(SettingsPage::Hotkeys));
    const DEMO: &str = "Demo, 60 items";
    const APPS: &str = "Applications, 130 items";
    assert_eq!(expanded(cx, DEMO), Some(true));
    // Under a group row, its first item, then one item every `pitch`.
    let first = top(cx, &app(0)) - top(cx, APPS);
    let pitch = top(cx, &app(1)) - top(cx, &app(0));
    let demo = |ix: usize| format!("Demo item {ix:02}");
    let mut seen = 0;
    // Each row in view is drawn and in place; at the top edge, a row is drawn exactly when its
    // line ends below the page area's top less the room drawn beyond it, so a group row
    // counted a few pixels off draws the wrong row on some of the scroll positions.
    let mut check = |cx: &mut TestAppContext| {
        let (area, header) = on_window(cx, |window, _| {
            (
                window.find("settings-page").bounds(),
                window.find_by_label(DEMO).bounds(),
            )
        });
        let edge = area.top() - px(OVERSCAN);
        for ix in 0..60 {
            let at = header.top() + first + pitch * ix as f32;
            let line_bottom = header.bottom() + pitch * (ix + 1) as f32;
            let drawn = has_label(cx, &demo(ix));
            if drawn {
                assert_eq!(top(cx, &demo(ix)), at, "{}", demo(ix));
                seen += 1;
            }
            if at + pitch > area.top() && at < area.bottom() {
                assert!(drawn, "{} in view not drawn", demo(ix));
            }
            if line_bottom <= edge - px(0.5) {
                assert!(!drawn, "{} drawn above the edge", demo(ix));
            } else if line_bottom > edge + px(0.5) && line_bottom - pitch < area.top() {
                assert!(drawn, "{} just inside the edge not drawn", demo(ix));
            }
        }
    };
    // Down to Demo, then on in steps prime to the row pitch, so the edge falls at every pixel
    // of a row.
    for _ in 0..60 {
        scroll_page(cx, -200.);
        check(cx);
        let header = top(cx, DEMO);
        let area = on_window(cx, |window, _| window.find("settings-page").bounds());
        if header < area.top() - px(300.) {
            break;
        }
    }
    for _ in 0..60 {
        scroll_page(cx, -13.);
        check(cx);
    }
    assert!(seen > 60, "the group never scrolled into view");
}

/// The rows drawn beyond each edge of the page area: two item rows.
const OVERSCAN: f32 = 96.;

/// A group holding an item with a hotkey starts open; one without starts closed and opens
/// and closes on a click on its header, or on Enter there.
#[gpui_kit::test]
fn a_group_with_a_hotkey_starts_open(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, WEATHER), Some(true));
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Ctrl+Alt+W"));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Add hotkey"));
    click(cx, WEATHER);
    assert_eq!(expanded(cx, WEATHER), Some(false));
    assert!(!has_label(cx, "Forecast"));
    let focused = on_window(cx, |window, _| window.find_by_label(WEATHER).focused());
    assert_eq!(focused, Some(true));
    press(cx, "enter");
    assert_eq!(expanded(cx, WEATHER), Some(true));
}

/// Enter and Space on a group's header open and close it.
#[gpui_kit::test]
fn enter_and_space_open_and_close_a_group(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "");
    open(cx, Some(SettingsPage::Hotkeys));
    // A click on the header focuses it (and closes the open group).
    click(cx, APPLICATIONS);
    let focused = on_window(cx, |window, _| window.find_by_label(APPLICATIONS).focused());
    assert_eq!(focused, Some(true));
    assert_eq!(expanded(cx, APPLICATIONS), Some(false));
    for (key, open) in [
        ("enter", true),
        ("enter", false),
        ("space", true),
        ("space", false),
    ] {
        press(cx, key);
        assert_eq!(expanded(cx, APPLICATIONS), Some(open), "{key}");
        assert_eq!(has_label(cx, "Firefox"), open, "{key}");
    }
}

/// In a group the items with a hotkey come first, each part by title.
#[gpui_kit::test]
fn items_with_a_hotkey_come_first(cx: &mut TestAppContext) {
    let _s = setup_apps(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+T\" = \"apps/terminal.desktop\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    assert!(top(cx, "Terminal") < top(cx, "Firefox"));
}

/// Applications is open the first time Hotkeys is shown, whatever it holds; another group with
/// no hotkey starts closed.
#[gpui_kit::test]
fn applications_are_open_when_hotkeys_is_first_shown(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    assert!(has_label(cx, "Firefox"));
    assert!(has_label(cx, "Terminal"));
    assert_eq!(expanded(cx, WEATHER), Some(false));
    click(cx, WEATHER);
    assert_eq!(expanded(cx, WEATHER), Some(true));
}

/// An item given a hotkey moves up once its field is left, not while the user is in it.
#[gpui_kit::test]
fn a_row_moves_up_once_its_field_is_left(cx: &mut TestAppContext) {
    let s = setup_apps(cx, "");
    open(cx, Some(SettingsPage::Hotkeys));
    assert!(top(cx, "Firefox") < top(cx, "Terminal"));
    click(cx, "Terminal");
    press(cx, "ctrl-alt-t");
    press(cx, "enter");
    assert_eq!(s.requests.borrow().len(), 1);
    daemon_wrote(
        cx,
        &s,
        "[hotkeys]\n\"Ctrl+Alt+T\" = \"apps/terminal.desktop\"\n",
    );
    assert_eq!(value_of(cx, "Terminal").as_deref(), Some("Ctrl+Alt+T"));
    assert!(top(cx, "Firefox") < top(cx, "Terminal"));
    click(cx, "Search...");
    assert!(top(cx, "Terminal") < top(cx, "Firefox"));
}

/// A plugin's commands with a hotkey come first, the others in the order declared.
#[gpui_kit::test]
fn commands_with_a_hotkey_come_first(cx: &mut TestAppContext) {
    let mut weather = weather();
    weather.commands.push(CatalogCommand {
        id: "radar".into(),
        title: "Radar".into(),
        icon: None,
    });
    let _s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+R\" = \"weather/radar\"\n");
    set_catalog(cx, catalog(vec![weather]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(top(cx, "Radar") < top(cx, FORECAST));
}

/// A group's hotkey fields are made when it is shown open, and kept once it closes and opens
/// again.
#[gpui_kit::test]
fn hotkey_fields_are_made_when_a_group_opens(cx: &mut TestAppContext) {
    let _s = setup_apps(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    let fields = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.global::<SettingsController>().hotkey_field_items(cx))
            .iter()
            .map(|item| item.as_str().to_string())
            .collect::<Vec<_>>()
    };
    // Applications is open when the page is shown: its fields are made then.
    let all = [
        "apps/firefox.desktop",
        "apps/terminal.desktop",
        "weather/forecast",
    ];
    assert_eq!(fields(cx), all);
    click(cx, APPLICATIONS);
    assert_eq!(fields(cx), all);
    click(cx, APPLICATIONS);
    assert_eq!(fields(cx), all);
}

/// Clearing the last hotkey of an open group leaves it open.
#[gpui_kit::test]
fn a_group_stays_open_when_its_last_hotkey_goes(cx: &mut TestAppContext) {
    let s = setup_apps(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"apps/firefox.desktop\"\n");
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    std::fs::write(&s.config, "").unwrap();
    config_changed(cx);
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Add hotkey"));
}

/// The daemon took a hotkey asked for in the window and wrote it to the file.
fn daemon_wrote(cx: &mut TestAppContext, s: &Setup, text: &str) {
    std::fs::write(&s.config, text).unwrap();
    config_changed(cx);
}

const FORECAST_ON_W: &str = "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n";

/// A hotkey set on a plugin's page before Hotkeys is first shown: its group starts open.
#[gpui_kit::test]
fn a_hotkey_set_on_a_plugin_page_opens_its_group_on_hotkeys(cx: &mut TestAppContext) {
    let s = setup_apps(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Forecast");
    press(cx, "ctrl-alt-w");
    press(cx, "enter");
    assert_eq!(s.requests.borrow().len(), 1);
    daemon_wrote(cx, &s, FORECAST_ON_W);
    click_page(cx, "Hotkeys");
    assert_eq!(expanded(cx, WEATHER), Some(true));
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Ctrl+Alt+W"));
}

/// A group seen closed opens once one of its items gets a hotkey elsewhere; one the user
/// closed with a click stays closed.
#[gpui_kit::test]
fn a_group_opens_when_one_of_its_items_gets_a_hotkey(cx: &mut TestAppContext) {
    let s = setup_apps(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, WEATHER), Some(false));
    click_page(cx, "Weather");
    daemon_wrote(cx, &s, FORECAST_ON_W);
    click_page(cx, "Hotkeys");
    assert_eq!(expanded(cx, WEATHER), Some(true));

    // Closed with a click: a new hotkey there does not open it again.
    click(cx, WEATHER);
    assert_eq!(expanded(cx, WEATHER), Some(false));
    daemon_wrote(cx, &s, "");
    daemon_wrote(cx, &s, FORECAST_ON_W);
    assert_eq!(expanded(cx, WEATHER), Some(false));
}

/// A source with no name to show is listed under "Other"; a plugin the catalog does not list
/// goes by the name its items carry.
#[gpui_kit::test]
fn sources_without_a_name_are_under_other(cx: &mut TestAppContext) {
    let mut core = new_core();
    let unnamed = Item {
        id: ItemId::new("mystery", "thing"),
        ..fake_item("thing", "Thing")
    };
    let named = Item {
        id: ItemId::new("clock", "alarm"),
        source: Some("Clock".into()),
        ..fake_item("alarm", "Alarm")
    };
    for (id, item) in [("mystery", unnamed), ("clock", named)] {
        core.register(Box::new(
            FakeModule::new(vec![item], || Ok(Vec::new())).with_id(id),
        ));
    }
    core.start_modules();
    for _ in 0..2 {
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    let _s = setup_with(cx, "", core);
    open(cx, Some(SettingsPage::Hotkeys));
    assert!(has_label(cx, "Other, 1 item"));
    assert!(has_label(cx, "Clock, 1 item"));
    assert!(!has_label(cx, "mystery, 1 item"));
}

/// As [`setup`], over a core with `n` applications, "Application 000" (`apps/app000.desktop`)
/// on.
fn setup_many_apps(cx: &mut TestAppContext, n: usize, text: &str) -> Setup {
    let mut core = new_core();
    let apps = (0..n)
        .map(|ix| {
            app_item(
                &format!("app{ix:03}.desktop"),
                &format!("Application {ix:03}"),
            )
        })
        .collect();
    core.register(Box::new(
        FakeModule::new(apps, || Ok(Vec::new())).with_id("apps"),
    ));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    setup_with(cx, text, core)
}

/// How many times the open window has built its pages.
fn page_builds(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.global::<SettingsController>().page_builds(cx))
}

/// How many times the open window has read which rows a query opens its groups on.
fn search_reads(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.global::<SettingsController>().search_reads(cx))
}

/// Under a query, which rows it opens the groups on is read once for the pages as built: a
/// frame in which nothing changed (a scroll, a redraw) reads it no more; another query does.
#[gpui_kit::test]
fn a_frame_under_a_query_reads_its_groups_once(cx: &mut TestAppContext) {
    let _s = setup_many_apps(cx, 130, "");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Search...");
    type_text(cx, "app");
    let read = search_reads(cx);
    assert!(read > 0);
    for _ in 0..3 {
        scroll_page(cx, -200.);
    }
    on_window(cx, |_, _| {});
    assert_eq!(search_reads(cx), read);
    type_text(cx, "l");
    assert!(search_reads(cx) > read);
}

/// Scrolls the page area by `dy` pixels (negative: down the page).
fn scroll_page(cx: &mut TestAppContext, dy: f32) {
    use gpui_kit::{ScrollDelta, point, px};
    on_window(cx, |window, cx| {
        window.scroll(
            "settings-page",
            ScrollDelta::Pixels(point(px(0.), px(dy))),
            cx,
        )
    });
    cx.run_until_parked();
}

/// A frame in which nothing the pages show changed (a scroll, a redraw) builds no page; a
/// change of the file, or of what is indexed, builds them again.
#[gpui_kit::test]
fn a_frame_with_nothing_changed_builds_no_page(cx: &mut TestAppContext) {
    let s = setup_many_apps(cx, 130, "");
    open(cx, Some(SettingsPage::Hotkeys));
    let built = page_builds(cx);
    for _ in 0..3 {
        scroll_page(cx, -200.);
    }
    on_window(cx, |_, _| {});
    assert_eq!(page_builds(cx), built);

    daemon_wrote(
        cx,
        &s,
        "[hotkeys]\n\"Ctrl+Alt+A\" = \"apps/app000.desktop\"\n",
    );
    assert!(page_builds(cx) > built);
    scroll_page(cx, 600.);
    assert_eq!(
        value_of(cx, "Application 000").as_deref(),
        Some("Ctrl+Alt+A")
    );
    let built = page_builds(cx);
    scroll_page(cx, -200.);
    on_window(cx, |_, _| {});
    assert_eq!(page_builds(cx), built);

    // A source indexed after the page was built shows on the next frame.
    let alarm = Item {
        id: ItemId::new("clock", "alarm"),
        source: Some("Clock".into()),
        ..fake_item("alarm", "Alarm")
    };
    {
        let mut core = s.core.borrow_mut();
        core.register(Box::new(
            FakeModule::new(vec![alarm], || Ok(Vec::new())).with_id("clock"),
        ));
        core.start_modules();
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    assert!(has_label(cx, "Clock, 1 item"));
    assert!(page_builds(cx) > built);
}

fn app(ix: usize) -> String {
    format!("Application {ix:03}")
}

/// How many item rows the page area draws.
fn drawn_items(cx: &mut TestAppContext) -> usize {
    on_window(cx, |window, _| window.find_all("item").len())
}

/// A long table draws only the rows in view: scrolled, the rows that come into view are where
/// they would be with every row drawn, what follows the table does not move, and their fields
/// work. Every row's field is made as before.
#[gpui_kit::test]
fn a_long_table_draws_only_the_rows_in_view(cx: &mut TestAppContext) {
    const APPS: &str = "Applications, 130 items";
    let s = setup_many_apps(cx, 130, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    assert!(drawn_items(cx) < 40, "{} rows drawn", drawn_items(cx));
    let fields = cx.update(|cx| cx.global::<SettingsController>().hotkey_field_items(cx));
    assert_eq!(fields.len(), 131);
    let pitch = top(cx, &app(1)) - top(cx, &app(0));
    let first = top(cx, &app(0)) - top(cx, APPS);
    let weather = top(cx, WEATHER) - top(cx, APPS);

    for _ in 0..40 {
        scroll_page(cx, -200.);
    }
    assert!(has_label(cx, &app(129)));
    assert!(!has_label(cx, &app(0)));
    assert!(drawn_items(cx) < 40, "{} rows drawn", drawn_items(cx));
    for ix in 0..130 {
        if has_label(cx, &app(ix)) {
            assert_eq!(
                top(cx, &app(ix)) - top(cx, APPS),
                first + pitch * ix as f32,
                "{}",
                app(ix)
            );
        }
    }
    assert_eq!(top(cx, WEATHER) - top(cx, APPS), weather);

    click(cx, &app(129));
    press(cx, "ctrl-alt-z");
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(ItemId::new("apps", "app129.desktop")),
            Some(hk("ctrl+alt+z")),
            HotkeyOrigin::Settings
        )]
    );
}

/// A long table below one that closes is drawn where it now is on the very next frame.
#[gpui_kit::test]
fn a_long_table_moved_up_is_drawn_at_once(cx: &mut TestAppContext) {
    let mut core = new_core();
    let apps = (0..130)
        .map(|ix| app_item(&format!("app{ix:03}.desktop"), &app(ix)))
        .collect();
    let demo = (0..60)
        .map(|ix| fake_item(&format!("d{ix:02}"), &format!("Demo item {ix:02}")))
        .collect();
    for (id, items) in [("apps", apps), ("demo", demo)] {
        core.register(Box::new(
            FakeModule::new(items, || Ok(Vec::new())).with_id(id),
        ));
    }
    core.start_modules();
    for _ in 0..2 {
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    // A hotkey in Demo opens it under Applications.
    let _s = setup_with(cx, "[hotkeys]\n\"Ctrl+Alt+D\" = \"demo/d30\"\n", core);
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, "Demo, 60 items"), Some(true));
    assert!(!has_label(cx, "Demo item 05"));
    // The frame the click calls for, with nothing run between.
    let drawn = on_window(cx, |window, cx| {
        let at = window
            .find_by_label("Applications, 130 items")
            .bounds()
            .center();
        use gpui_kit::{InputEvent as _, MouseDownEvent, MouseUpEvent};
        window.pointer_move(at, None, cx);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            click_count: 1,
            ..Default::default()
        };
        window.dispatch_event(down.to_platform_input(), cx);
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            click_count: 1,
            ..Default::default()
        };
        window.dispatch_event(up.to_platform_input(), cx);
        window.render_frame(cx);
        window.try_find_by_label("Demo item 05").is_some()
    });
    assert!(drawn);
    cx.run_until_parked();
    assert!(drawn_items(cx) < 40, "{} rows drawn", drawn_items(cx));
}

/// A field in use stays drawn when its row scrolls out of view, so a recording goes on; Tab
/// from it goes row by row, also past the rows in view.
#[gpui_kit::test]
fn a_field_in_use_stays_drawn_out_of_view(cx: &mut TestAppContext) {
    let _s = setup_many_apps(cx, 130, "");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, &app(1));
    assert!(recording(cx));
    for _ in 0..10 {
        scroll_page(cx, -200.);
    }
    assert!(recording(cx));
    assert!(has_label(cx, &app(1)));
    press(cx, "escape");
    assert!(!recording(cx));
    for _ in 0..10 {
        scroll_page(cx, 200.);
    }
    for ix in 2..40 {
        press(cx, "tab");
        let focused = on_window(cx, |window, _| window.find_by_label(&app(ix)).focused());
        assert_eq!(focused, Some(true), "{}", app(ix));
    }
}

fn focused(cx: &mut TestAppContext, label: &str) -> Option<bool> {
    on_window(cx, |window, _| window.find_by_label(label).focused())
}

/// A group row reached with the keyboard shows the focus ring, inside its own bounds (the
/// table's box would cut one drawn outside); one focused by a click does not, and the ring
/// goes with the focus.
#[gpui_kit::test]
fn a_group_row_shows_the_focus_ring_when_reached_by_keyboard(cx: &mut TestAppContext) {
    const APPS: &str = "Applications, 3 items";
    let _s = setup_many_apps(cx, 3, "");
    open(cx, Some(SettingsPage::Hotkeys));
    let ring = |cx: &mut TestAppContext| {
        on_window(cx, |window, _| {
            window
                .try_find("group-focus-ring")
                .map(|ring| ring.bounds())
        })
    };
    click(cx, APPS);
    click(cx, APPS);
    assert_eq!(focused(cx, APPS), Some(true));
    assert_eq!(ring(cx), None, "a click shows no ring");
    press(cx, "tab");
    assert_eq!(focused(cx, &app(0)), Some(true));
    assert_eq!(ring(cx), None);
    press(cx, "shift-tab");
    assert_eq!(focused(cx, APPS), Some(true));
    let header = on_window(cx, |window, _| window.find_by_label(APPS).bounds());
    let ring = ring(cx).expect("the ring of the focused group row");
    assert!(
        ring.left() > header.left()
            && ring.right() < header.right()
            && ring.top() > header.top()
            && ring.bottom() < header.bottom(),
        "{ring:?} inside {header:?}"
    );
    press(cx, "tab");
    assert!(
        on_window(cx, |window, _| window
            .try_find("group-focus-ring")
            .is_none()),
        "the ring goes with the focus"
    );
}

/// Tab from a table's header goes to its first row, also when that row is out of view.
#[gpui_kit::test]
fn tab_from_a_header_reaches_its_first_row_out_of_view(cx: &mut TestAppContext) {
    const APPS: &str = "Applications, 130 items";
    let _s = setup_many_apps(cx, 130, "");
    open(cx, Some(SettingsPage::Hotkeys));
    // Closed and opened again: the header keeps the focus.
    click(cx, APPS);
    click(cx, APPS);
    assert_eq!(expanded(cx, APPS), Some(true));
    assert_eq!(focused(cx, APPS), Some(true));
    for _ in 0..10 {
        scroll_page(cx, -200.);
    }
    assert!(has_label(cx, &app(40)));
    press(cx, "tab");
    assert_eq!(focused(cx, &app(0)), Some(true));
}

/// Shift+Tab from the header of the group under a table goes to the table's last row, also
/// when that row is out of view.
#[gpui_kit::test]
fn shift_tab_from_the_next_header_reaches_the_last_row_out_of_view(cx: &mut TestAppContext) {
    let _s = setup_many_apps(cx, 130, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Hotkeys));
    for _ in 0..40 {
        scroll_page(cx, -200.);
    }
    click(cx, WEATHER);
    click(cx, WEATHER);
    assert_eq!(focused(cx, WEATHER), Some(true));
    for _ in 0..40 {
        scroll_page(cx, 200.);
    }
    assert!(has_label(cx, &app(0)));
    press(cx, "shift-tab");
    assert_eq!(focused(cx, &app(129)), Some(true));
}

/// A page shown again after another one is at its top again.
#[gpui_kit::test]
fn a_page_shown_again_is_at_its_top(cx: &mut TestAppContext) {
    let _s = setup_many_apps(cx, 130, "");
    open(cx, Some(SettingsPage::Hotkeys));
    let first = top(cx, &app(0));
    for _ in 0..10 {
        scroll_page(cx, -200.);
    }
    assert!(!has_label(cx, &app(0)));
    click_page(cx, "General");
    click_page(cx, "Hotkeys");
    assert!(has_label(cx, &app(0)));
    assert_eq!(top(cx, &app(0)), first);
}

/// A window that grows taller while a long table is scrolled to its end draws every row that
/// comes into view.
#[gpui_kit::test]
fn a_window_grown_taller_at_the_end_draws_the_rows_in_view(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let _s = setup_many_apps(cx, 130, "");
    open(cx, Some(SettingsPage::Hotkeys));
    let pitch = top(cx, &app(1)) - top(cx, &app(0));
    for _ in 0..60 {
        scroll_page(cx, -200.);
    }
    assert!(has_label(cx, &app(129)));
    let w = window(cx);
    cx.simulate_window_resize(w, size(px(900.), px(1600.)));
    // The frame the resize calls for, with nothing run between.
    let missing: Vec<usize> = cx
        .update_window(w, |_, window, _| {
            let area = window.find("settings-page").bounds();
            assert!(area.size.height > px(1000.), "not drawn at the new size");
            let last = window.find_by_label(&app(129)).bounds().top();
            (0..130)
                .filter(|&ix| {
                    let at = last - pitch * (129 - ix) as f32;
                    at + pitch > area.top()
                        && at < area.bottom()
                        && window.try_find_by_label(&app(ix)).is_none()
                })
                .collect()
        })
        .unwrap();
    assert!(missing.is_empty(), "rows in view not drawn: {missing:?}");
}

/// The window's search opens a closed group on the rows that match, and their fields work.
#[gpui_kit::test]
fn search_opens_a_closed_group(cx: &mut TestAppContext) {
    let s = setup_apps(cx, "");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, APPLICATIONS);
    assert!(!has_label(cx, "Firefox"));
    click(cx, "Search...");
    type_text(cx, "firef");
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Add hotkey"));
    assert!(!has_label(cx, "Terminal"));
    assert_eq!(expanded(cx, APPLICATIONS), Some(true));
    click(cx, "Firefox");
    press(cx, "ctrl-alt-f");
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(ItemId::new("apps", "firefox.desktop")),
            Some(hk("ctrl+alt+f")),
            HotkeyOrigin::Settings
        )]
    );
    // A click closes what the search opened; once the query is cleared the group is as it
    // was before.
    click(cx, APPLICATIONS);
    assert_eq!(expanded(cx, APPLICATIONS), Some(false));
    assert!(!has_label(cx, "Firefox"));
    click(cx, "Search...");
    press(cx, "escape");
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
    assert_eq!(expanded(cx, APPLICATIONS), Some(false));
}

/// An item with a hotkey that is not indexed now is greyed, in its source's group or under
/// "Not available"; the × in its field asks for the hotkey to go.
#[gpui_kit::test]
fn an_unindexed_item_with_a_hotkey_is_greyed_and_removable(cx: &mut TestAppContext) {
    let s = setup_apps(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+G\" = \"gone/thing\"\n\"Ctrl+Alt+H\" = \"apps/gone.desktop\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(expanded(cx, "Applications, 3 items"), Some(true));
    assert_eq!(expanded(cx, "Not available, 1 item"), Some(true));
    assert_eq!(value_of(cx, "thing").as_deref(), Some("Ctrl+Alt+G"));
    assert_eq!(value_of(cx, "gone").as_deref(), Some("Ctrl+Alt+H"));
    assert!(!has_label(cx, "gone/thing"));
    let (gone, there) = on_window(cx, |window, _| {
        (
            window.find_all("item-gone").len(),
            window.find_all("item").len(),
        )
    });
    assert_eq!((gone, there), (2, 2));
    assert!(!has_label(cx, "Remove hotkey of thing"));
    click(cx, "Clear hotkey of thing");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(ItemId::new("gone", "thing")),
            None,
            HotkeyOrigin::Settings
        )]
    );
}

/// Closing a group ends a recording in it.
#[gpui_kit::test]
fn closing_a_group_ends_its_recording(cx: &mut TestAppContext) {
    let _s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Firefox");
    assert!(recording(cx));
    click(cx, "Demo, 3 items");
    assert!(!has_label(cx, "Firefox"));
    assert!(!recording(cx));
}

/// An outside edit read while the core is busy keeps the hotkey fields made for the rows
/// drawn: nothing can be told about their items then.
#[gpui_kit::test]
fn a_busy_core_keeps_the_fields_of_drawn_rows(cx: &mut TestAppContext) {
    let s = setup_apps(cx, "");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, APPLICATIONS);
    let fields = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.global::<SettingsController>().hotkey_field_items(cx))
            .iter()
            .map(|item| item.as_str().to_string())
            .collect::<Vec<_>>()
    };
    let drawn = ["apps/firefox.desktop", "apps/terminal.desktop"];
    assert_eq!(fields(cx), drawn);
    {
        let _busy = s.core.borrow_mut();
        daemon_wrote(cx, &s, "[launcher]\nautostart = false\n");
        assert_eq!(fields(cx), drawn);
    }
    daemon_wrote(cx, &s, "");
    assert_eq!(fields(cx), drawn);
}

/// A plugin page's commands are a table as on the Hotkeys page.
#[gpui_kit::test]
fn plugin_commands_are_a_hotkey_table(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert!(has_label(cx, "Commands"));
    assert!(has_label(cx, "Name"));
    assert!(has_label(cx, "Hotkey"));
    assert_eq!(value_of(cx, "Forecast").as_deref(), Some("Add hotkey"));
    // No group rows: the "Name" title starts where the rows' icons do.
    columns_line_up(cx, "item");
    let group_rows = on_window(cx, |window, _| {
        window.find_all("group-chevron").len() + window.find_all("group-title").len()
    });
    assert_eq!(group_rows, 0);
    assert!(!has_label(cx, "Not set"));
}

/// A second hotkey of an item is a greyed row whose remove takes out exactly that entry, as
/// the file spells it.
#[gpui_kit::test]
fn dropped_entry_is_greyed_with_remove(cx: &mut TestAppContext) {
    let s = setup_items(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n\"ctrl+alt+g\" = \"demo/firefox\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    assert!(has_label(cx, "Not used: Firefox already has Ctrl+Alt+F"));
    // The line under its title says it short; the whole sentence is its name and tooltip.
    assert_eq!(
        reason_shown(cx, "Not used: Firefox already has Ctrl+Alt+F"),
        "Already has Ctrl+Alt+F"
    );
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Ctrl+Alt+F"));
    // Its hotkey is in the Hotkey column, where the keys in the fields start.
    let (dropped, keys) = on_window(cx, |window, _| {
        (
            window.find("dropped-hotkey").bounds(),
            window.find_all("hotkey-chips")[0].bounds(),
        )
    });
    assert_eq!(dropped.left(), keys.left(), "{dropped:?} {keys:?}");
    click(cx, "Remove Ctrl+Alt+G of Firefox");
    assert_eq!(
        read(&s.config),
        "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n"
    );
    assert!(!has_label(cx, "Not used: Firefox already has Ctrl+Alt+F"));
    assert!(s.requests.borrow().is_empty());
}

/// Why an entry is not used is one muted line under its title, cut with an ellipsis where
/// the Hotkey column starts; the whole sentence is its accessible name (and tooltip).
#[gpui_kit::test]
fn a_not_used_reason_is_one_line(cx: &mut TestAppContext) {
    let _s = setup_items(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+Shift+Super+PageDown\" = \"demo/firefox\"\n\"ctrl+alt+g\" = \"demo/firefox\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    const WHY: &str = "Not used: Firefox already has Ctrl+Alt+Shift+Super+Page Down";
    let (why, title, hotkey) = on_window(cx, |window, _| {
        (
            window.find_by_label(WHY).bounds(),
            window.find("item-dropped").bounds(),
            window.find("dropped-hotkey").bounds(),
        )
    });
    assert!(why.top() >= title.bottom(), "{why:?} under {title:?}");
    // Under the title's text, past the icon, as a row's description is.
    let name = on_window(cx, |window, _| window.find("dropped-title").bounds());
    assert_eq!(why.left(), name.left(), "{why:?} under {name:?}");
    assert!(
        why.size.height <= hotkey.size.height,
        "one line: {why:?} against {hotkey:?}"
    );
    assert!(why.right() < hotkey.left(), "{why:?} before {hotkey:?}");
}

/// The text shown for the "Not used" reason named `full`: the one text drawn in its bounds.
/// The reason has no value: a screen reader hears it once, by its name.
fn reason_shown(cx: &mut TestAppContext, full: &str) -> String {
    on_window(cx, |window, _| {
        let reason = window.find_by_label(full);
        assert_eq!(reason.value(), None, "{full}");
        let shown: Vec<String> = gpui_kit::base::test_support::texts(window)
            .into_iter()
            .filter(|text| text.bounds() == reason.bounds())
            .map(|text| text.text().to_string())
            .collect();
        assert_eq!(shown.len(), 1, "{full}: {shown:?}");
        shown[0].clone()
    })
}

/// Entries dropped for other reasons say why: short under the title, short enough for one
/// line, and the whole sentence as its name. An item is named by its title, or by its
/// readable id when it is not there, never by its raw id.
#[gpui_kit::test]
fn entries_dropped_for_other_reasons_say_why(cx: &mut TestAppContext) {
    let _s = setup_items(
        cx,
        "[hotkeys]\n\"Alt+Space\" = \"demo/files\"\n\"Ctrl+Alt+T\" = \"demo/terminal\"\n\"F1\" = \"demo/firefox\"\n\"ctrl+alt+t\" = \"demo/files\"\n\"Ctrl+Alt+Escape\" = \"demo/terminal\"\n\"Ctrl+Alt+Y\" = \"apps/org.gone.App.desktop\"\n\"ctrl+alt+y\" = \"demo/firefox\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    for (full, short) in [
        (
            "Not used: the launcher already has Alt+Space",
            "The launcher's hotkey",
        ),
        (
            "Not used: Terminal already has Ctrl+Alt+T",
            "Used by Terminal",
        ),
        (
            "Not used: Add Ctrl, Alt or Super.",
            "Needs Ctrl, Alt or Super",
        ),
        (
            "Not used: Esc cannot be a hotkey.",
            "Esc cannot be a hotkey",
        ),
        (
            "Not used: org.gone.App already has Ctrl+Alt+Y",
            "Used by org.gone.App",
        ),
    ] {
        assert_eq!(reason_shown(cx, full), short, "{full}");
    }
}

/// Each row shows how its hotkey stands; the launcher's on General.
#[gpui_kit::test]
fn refused_state_is_shown(cx: &mut TestAppContext) {
    let _s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    let mut t = table(&[(
        "firefox",
        "ctrl+alt+f",
        HotkeyState::Refused {
            reason: "used by krunner: Run".into(),
        },
    )]);
    t.launcher_state = Some(HotkeyState::AfterRestart);
    set_table(cx, t);
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(
        description_of(cx, "Firefox").as_deref(),
        Some("Refused: used by krunner: Run")
    );
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(
        description_of(cx, "Launcher hotkey").as_deref(),
        Some("Applies after a restart")
    );
}

#[gpui_kit::test]
fn hotkeys_off_in_this_run_say_so(cx: &mut TestAppContext) {
    let _s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    let mut t = table(&[("firefox", "ctrl+alt+f", HotkeyState::Off)]);
    t.launcher_state = Some(HotkeyState::Refused {
        reason: "used by krunner: Run".into(),
    });
    set_table(cx, t);
    open(cx, Some(SettingsPage::Hotkeys));
    assert_eq!(description_of(cx, "Firefox").as_deref(), Some("Off"));
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(
        description_of(cx, "Launcher hotkey").as_deref(),
        Some("Refused: used by krunner: Run")
    );
}

/// The settings recorder refuses as the launcher's does.
#[gpui_kit::test]
fn taken_hotkey_is_refused_in_the_field(cx: &mut TestAppContext) {
    let s = setup_items(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n\"Ctrl+Alt+T\" = \"demo/terminal\"\n",
    );
    set_table(
        cx,
        table(&[
            ("firefox", "ctrl+alt+f", ACTIVE),
            ("terminal", "ctrl+alt+t", ACTIVE),
        ]),
    );
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Firefox");
    press(cx, "ctrl-alt-t");
    assert!(has_label(
        cx,
        "Already set on \"Terminal\". Clear it there first."
    ));
    press(cx, "enter");
    assert!(s.requests.borrow().is_empty());
    press(cx, "alt-space");
    assert!(has_label(cx, "That is the launcher hotkey."));
}

/// A tomakast hotkey pressed while a field records is recorded there, not run.
#[gpui_kit::test]
fn global_press_reaches_the_settings_recorder(cx: &mut TestAppContext) {
    let s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    open(cx, Some(SettingsPage::Hotkeys));
    let recording = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.global::<SettingsController>().is_recording(cx))
    };
    assert!(!recording(cx));
    click(cx, "Firefox");
    assert!(recording(cx));
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| c.recorded_press(hk("ctrl+alt+g"), cx))
    });
    cx.run_until_parked();
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Ctrl+Alt+G"));
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(fake("firefox")),
            Some(hk("ctrl+alt+g")),
            HotkeyOrigin::Settings
        )]
    );
}

/// The answer to a change asked for on the Hotkeys page shows in that row's field.
#[gpui_kit::test]
fn notice_shows_in_the_field(cx: &mut TestAppContext) {
    let _s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Firefox");
    press(cx, "ctrl-alt-g");
    press(cx, "enter");
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Ctrl+Alt+G"));
    notice(
        cx,
        HotkeyTarget::Item(fake("firefox")),
        "Ctrl+Alt+G for Firefox: used by krunner: Run; nothing changed",
    );
    assert_eq!(
        description_of(cx, "Firefox").as_deref(),
        Some("Ctrl+Alt+G for Firefox: used by krunner: Run; nothing changed")
    );
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Ctrl+Alt+F"));
}

/// A notice goes to the field whose change it answers, even when a change was asked on
/// another field since.
#[gpui_kit::test]
fn a_notice_goes_to_the_field_it_answers(cx: &mut TestAppContext) {
    let _s = setup_items(
        cx,
        "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n\"Ctrl+Alt+T\" = \"demo/terminal\"\n",
    );
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Firefox");
    press(cx, "ctrl-alt-g");
    press(cx, "enter");
    click(cx, "Terminal");
    press(cx, "ctrl-alt-y");
    press(cx, "enter");
    let text = "Ctrl+Alt+G for Firefox: refused; nothing changed";
    notice(cx, HotkeyTarget::Item(fake("firefox")), text);
    assert_eq!(description_of(cx, "Firefox").as_deref(), Some(text));
    assert_eq!(description_of(cx, "Terminal"), None);
    assert_eq!(value_of(cx, "Firefox").as_deref(), Some("Ctrl+Alt+F"));
    assert_eq!(value_of(cx, "Terminal").as_deref(), Some("Ctrl+Alt+Y"));
}

// The hotkey field.

/// The command field of the weather plugin's page, as `field`'s tests use it.
const FORECAST: &str = "Forecast";

fn bounds_of(cx: &mut TestAppContext, label: &str) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    on_window(cx, |window, _| window.find_by_label(label).bounds())
}

/// Every element on display outside the field labelled `label` (and what floats under it),
/// with its bounds: what must not move while the field changes.
fn around(
    cx: &mut TestAppContext,
    label: &str,
) -> Vec<(String, gpui_kit::Bounds<gpui_kit::Pixels>)> {
    on_window(cx, |window, _| {
        let field = window.find_by_label(label).path().to_vec();
        window
            .elements()
            .into_iter()
            .filter(|e| !e.path().starts_with(&field))
            .map(|e| (format!("{:?}", e.path()), e.bounds()))
            .collect()
    })
}

/// The field's size is the same empty, recording with nothing pressed, recording the longest
/// combination and refused.
#[gpui_kit::test]
fn the_hotkey_field_keeps_its_size(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    let empty = bounds_of(cx, FORECAST);
    click(cx, FORECAST);
    assert_eq!(value_of(cx, FORECAST).as_deref(), Some("Press keys…"));
    let waiting = bounds_of(cx, FORECAST);
    // Every modifier and the longest key name a hotkey can use (`tomakast_core::keys`), as
    // the system hands a press over.
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, cx| {
            c.recorded_press(hk("ctrl+alt+shift+super+backspace"), cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(
        value_of(cx, FORECAST).as_deref(),
        Some("Ctrl+Alt+Shift+Super+Backspace")
    );
    let longest = bounds_of(cx, FORECAST);
    press(cx, "a");
    assert!(has_label(cx, "Add Ctrl, Alt or Super."));
    let refused = bounds_of(cx, FORECAST);
    assert_eq!(empty, waiting);
    assert_eq!(empty, longest);
    assert_eq!(empty, refused);
}

/// The icon of how a hotkey stands sits at the field's right, after the keys and before the
/// clear button. (Which combinations fit, and how a longer one fades, is measured with the real
/// font in `fields.rs`.)
#[gpui_kit::test]
fn the_state_icon_sits_after_the_keys(cx: &mut TestAppContext) {
    let _s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    let table = HotkeyTable {
        items: vec![ItemHotkey {
            item: ItemId::new("weather", "forecast"),
            hotkey: hk("ctrl+alt+w"),
            state: HotkeyState::AfterRestart,
        }],
        ..HotkeyTable::default()
    };
    set_table(cx, table);
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    let (field, chips, stand, clear) = on_window(cx, |window, _| {
        (
            window.find_by_label(FORECAST).bounds(),
            window.find("hotkey-chips").bounds(),
            window.find("hotkey-stand").bounds(),
            window.find_by_label("Clear hotkey of Forecast").bounds(),
        )
    });
    assert!(chips.right() <= stand.left(), "{chips:?} {stand:?}");
    assert!(stand.right() <= clear.left(), "{stand:?} {clear:?}");
    assert!(field.left() <= chips.left() && clear.right() <= field.right());
}

/// General's launcher hotkey field reads from its left, where a text field's text starts
/// (its 1 px edge and the kit input's 10 px padding): no room is kept for an icon there.
#[gpui_kit::test]
fn the_launcher_hotkey_field_has_no_gap_at_its_left(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    let (field, chips) = on_window(cx, |window, _| {
        (
            window.find_by_label("Launcher hotkey").bounds(),
            window.find("hotkey-chips").bounds(),
        )
    });
    assert!(on_window(cx, |window, _| window
        .try_find("hotkey-stand")
        .is_none()));
    assert_eq!(
        chips.left() - field.left(),
        gpui_kit::px(tomakast_ui::test_metrics::CONTROL_TEXT_INSET),
        "{field:?} {chips:?}"
    );
}

#[gpui_kit::test]
fn an_empty_hotkey_field_reads_add_hotkey(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert_eq!(value_of(cx, FORECAST).as_deref(), Some("Add hotkey"));
    click(cx, FORECAST);
    assert_eq!(value_of(cx, FORECAST).as_deref(), Some("Press keys…"));
    press(cx, "escape");
    assert_eq!(value_of(cx, FORECAST).as_deref(), Some("Add hotkey"));
}

/// What a field reads starts at its left, the words of an empty one as the keys of one with a
/// hotkey; both fields are the same size.
#[gpui_kit::test]
fn a_hotkey_field_reads_from_its_left(cx: &mut TestAppContext) {
    let mut weather = weather();
    weather.commands.push(CatalogCommand {
        id: "radar".into(),
        title: "Radar".into(),
        icon: None,
    });
    let _s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert_eq!(value_of(cx, "Radar").as_deref(), Some("Add hotkey"));
    let (forecast, chips, radar, words) = on_window(cx, |window, _| {
        (
            window.find_by_label(FORECAST).bounds(),
            window.find("hotkey-chips").bounds(),
            window.find_by_label("Radar").bounds(),
            window.find("hotkey-placeholder").bounds(),
        )
    });
    assert_eq!(forecast.size, radar.size);
    assert_eq!(chips.left() - forecast.left(), words.left() - radar.left());
    assert!(words.left() < radar.center().x - (words.right() - words.left()) / 2.);
}

/// Recording, its hint and a refusal move nothing on the page: they float over the rows.
#[gpui_kit::test]
fn the_row_keeps_its_height_while_recording(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    let idle = around(cx, "Launcher hotkey");
    let field = bounds_of(cx, "Launcher hotkey");
    click(cx, "Launcher hotkey");
    assert!(recording(cx));
    assert_eq!(around(cx, "Launcher hotkey"), idle);
    press(cx, "a");
    // The hint has slid into its place.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    on_window(cx, |window, _| window.refresh());
    cx.run_until_parked();
    let refusal = on_window(cx, |window, _| {
        window.find_by_label("Add Ctrl, Alt or Super.").bounds()
    });
    assert_eq!(around(cx, "Launcher hotkey"), idle);
    assert_eq!(bounds_of(cx, "Launcher hotkey"), field);
    assert!(refusal.top() >= field.bottom(), "{refusal:?} {field:?}");
}

/// The clear button sits in a field with a hotkey and asks for it to go, as the reset button
/// did; it is not there while the field records or has none.
#[gpui_kit::test]
fn clear_removes_the_hotkey(cx: &mut TestAppContext) {
    let s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    const CLEAR: &str = "Clear hotkey of Forecast";
    assert!(has_label(cx, CLEAR));
    click(cx, FORECAST);
    assert!(recording(cx));
    assert!(!has_label(cx, CLEAR));
    press(cx, "escape");
    assert!(has_label(cx, CLEAR));
    click(cx, CLEAR);
    assert!(!recording(cx), "the click is not one on the field");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Item(ItemId::new("weather", "forecast")),
            None,
            HotkeyOrigin::Settings
        )]
    );
    assert_eq!(value_of(cx, FORECAST).as_deref(), Some("Add hotkey"));
    assert!(!has_label(cx, CLEAR));
}

/// Tab reaches the clear button; Enter there clears without starting a recording.
#[gpui_kit::test]
fn clear_works_from_the_keyboard(cx: &mut TestAppContext) {
    let s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, FORECAST);
    press(cx, "escape");
    press(cx, "tab");
    let focused = on_window(cx, |window, _| {
        window.find_by_label("Clear hotkey of Forecast").focused()
    });
    assert_eq!(focused, Some(true));
    press(cx, "enter");
    assert!(!recording(cx));
    assert_eq!(s.requests.borrow().len(), 1);
    assert_eq!(s.requests.borrow()[0].1, None);
    // The clear button went with the hotkey: the focus is on the field.
    let focused = on_window(cx, |window, _| window.find_by_label(FORECAST).focused());
    assert_eq!(focused, Some(true));
    assert!(!recording(cx));
}

/// The launcher hotkey's button goes back to the default, and is there only while the file
/// holds a hotkey of its own.
#[gpui_kit::test]
fn the_launcher_hotkey_resets_to_the_default(cx: &mut TestAppContext) {
    let s = setup(cx, "[launcher]\nhotkey = \"Ctrl+Alt+K\"\n");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Reset Launcher hotkey");
    assert!(!recording(cx));
    assert_eq!(
        *s.requests.borrow(),
        [(HotkeyTarget::Launcher, None, HotkeyOrigin::Settings)]
    );
    assert_eq!(
        value_of(cx, "Launcher hotkey"),
        Some(
            tomakast_core::settings_schema::builtin_keys()
                .iter()
                .find(|k| k.path == ["launcher", "hotkey"])
                .map(|k| hk(k.default).to_string())
                .unwrap()
        )
    );
    assert!(!has_label(cx, "Reset Launcher hotkey"));
}

/// A file that spells out the default (however written) has no Reset: removing the key would
/// change nothing.
#[gpui_kit::test]
fn the_launcher_hotkey_has_no_reset_on_its_default(cx: &mut TestAppContext) {
    let _s = setup(cx, "[launcher]\nhotkey = \"alt+space\"\n");
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(
        value_of(cx, "Launcher hotkey").as_deref(),
        Some("Alt+Space")
    );
    assert!(!has_label(cx, "Reset Launcher hotkey"));
}

/// Any last part is a key to the launcher, so `"banana"` is a hotkey of its own: Reset asks for
/// the default, and recording the default asks for it too.
#[gpui_kit::test]
fn a_hand_written_launcher_hotkey_resets_and_records_the_default(cx: &mut TestAppContext) {
    let s = setup(cx, "[launcher]\nhotkey = \"banana\"\n");
    open(cx, Some(SettingsPage::Launcher));
    assert_eq!(value_of(cx, "Launcher hotkey").as_deref(), Some("Banana"));
    click(cx, "Reset Launcher hotkey");
    assert_eq!(
        *s.requests.borrow(),
        [(HotkeyTarget::Launcher, None, HotkeyOrigin::Settings)]
    );

    let s = setup(cx, "[launcher]\nhotkey = \"banana\"\n");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Launcher hotkey");
    press(cx, "alt-space");
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Launcher,
            Some(hk("alt+space")),
            HotkeyOrigin::Settings
        )]
    );
}

/// A launcher hotkey that is not one makes the launcher refuse the file: the window writes
/// nothing until it is fixed, so the field has no Reset and records nothing.
#[gpui_kit::test]
fn an_unreadable_launcher_hotkey_leaves_the_field_read_only(cx: &mut TestAppContext) {
    let s = setup(cx, "[launcher]\nhotkey = \"Ctrl+\"\n");
    open(cx, Some(SettingsPage::Launcher));
    assert!(on_window(cx, |window, _| window
        .try_find("settings-banner")
        .is_some()));
    assert!(!has_label(cx, "Reset Launcher hotkey"));
    click(cx, "Launcher hotkey");
    assert!(!recording(cx));
    assert!(s.requests.borrow().is_empty());
}

/// How a hotkey stands is the field's description and an icon's tooltip, not a line under
/// the field.
#[gpui_kit::test]
fn a_hotkey_state_is_a_tooltip_icon(cx: &mut TestAppContext) {
    let _s = setup(cx, "[hotkeys]\n\"Ctrl+Alt+W\" = \"weather/forecast\"\n");
    set_catalog(cx, catalog(vec![weather()]));
    let table = HotkeyTable {
        items: vec![ItemHotkey {
            item: ItemId::new("weather", "forecast"),
            hotkey: hk("ctrl+alt+w"),
            state: HotkeyState::AfterRestart,
        }],
        ..HotkeyTable::default()
    };
    set_table(cx, table);
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    assert_eq!(
        description_of(cx, FORECAST).as_deref(),
        Some("Applies after a restart")
    );
    assert!(!has_label(cx, "Applies after a restart"));
    let (field, icon) = on_window(cx, |window, _| {
        (
            window.find_by_label(FORECAST).bounds(),
            window.find("hotkey-stand").bounds(),
        )
    });
    assert!(field.contains(&icon.center()), "{field:?} {icon:?}");
}

// The Home page.

use tomakast_core::{ParamTitle, WidgetDecl, WidgetSize};

/// Weather with one widget: "Current weather", single or double, taking a city.
fn weather_widget() -> CatalogPlugin {
    CatalogPlugin {
        widgets: vec![WidgetDecl {
            id: "now".into(),
            title: "Current weather".into(),
            sizes: vec![WidgetSize::Single, WidgetSize::Double],
            params: vec!["city".into()],
        }],
        param_titles: vec![ParamTitle {
            widget: "now".into(),
            key: "city".into(),
            title: "City".into(),
        }],
        ..plugin("weather", "Weather")
    }
}

/// The controller over `text`, with Weather's widget installed, on the Home page.
fn setup_home(cx: &mut TestAppContext, text: &str) -> Setup {
    let s = setup(cx, text);
    set_catalog(cx, catalog(vec![weather_widget()]));
    open(cx, Some(SettingsPage::Home));
    s
}

fn disabled(cx: &mut TestAppContext, label: &str) -> Option<bool> {
    on_window(cx, |window, _| window.find_by_label(label).disabled())
}

const NOW: &str = "[[home.widgets]]\nwidget = \"weather/now\"\n";

/// How widget `n` (counted from 1) titled `title` is named to assistive technology.
fn widget(title: &str, n: usize) -> String {
    format!("{title} (widget {n})")
}

/// The handle widget `n` titled `title` is dragged by.
fn handle(title: &str, n: usize) -> String {
    format!("Drag {} to reorder", widget(title, n))
}

/// Drags the element labelled `from` onto the element labelled `to` with the mouse: press,
/// moves with the button held, release.
fn drag(cx: &mut TestAppContext, from: &str, to: &str) {
    on_window(cx, |window, cx| {
        let from = window.find_by_label(from).bounds().center();
        let to = window.find_by_label(to).bounds().center();
        window.drag(from, to, cx);
    });
    cx.run_until_parked();
}

/// Every accessible name on display.
fn labels(cx: &mut TestAppContext) -> Vec<String> {
    on_window(cx, |window, _| {
        window
            .elements()
            .iter()
            .filter_map(|e| e.label().map(str::to_string))
            .collect()
    })
}

/// Three widgets of Weather, told apart by their cities.
const THREE: &str = "[[home.widgets]]\nwidget = \"weather/now\"\ncity = \"Oslo\"\n\n\
                     [[home.widgets]]\nwidget = \"weather/now\"\ncity = \"Rome\"\n\n\
                     [[home.widgets]]\nwidget = \"weather/now\"\ncity = \"Kazan\"\n";

#[gpui_kit::test]
fn home_rows_in_file_order_with_unreadable_greyed(cx: &mut TestAppContext) {
    let s = setup_home(
        cx,
        "[[home.widgets]]\nwidget = \"weather/now\"\n\n[[home.widgets]]\nwidget = 5\n\n\
         [[home.widgets]]\nwidget = \"clock/analog\"\n",
    );
    assert!(has_label(
        cx,
        "Add widgets from the launcher: choose Add to Home on a result."
    ));
    let first = top(cx, &handle("Current weather", 1));
    let unreadable = top(cx, "An entry the launcher cannot read");
    // The unreadable entry is not counted.
    let last = top(cx, &handle("analog", 2));
    assert!(first < unreadable && unreadable < last);
    // A widget dragged past it moves; it stays where it is.
    drag(cx, &handle("Current weather", 1), &handle("analog", 2));
    assert_eq!(
        read(&s.config),
        "[[home.widgets]]\nwidget = \"clock/analog\"\n\n[[home.widgets]]\nwidget = 5\n\n\
         [[home.widgets]]\nwidget = \"weather/now\"\n"
    );
    assert!(has_label(cx, "An entry the launcher cannot read"));
}

/// A row is one line: the handle, the plugin's icon, the title over the plugin's name and the
/// param values, the size dropdown and remove. No up and down, no param fields.
#[gpui_kit::test]
fn widgets_are_one_line_each(cx: &mut TestAppContext) {
    let _s = setup_home(
        cx,
        &format!("{NOW}city = \"Oslo\"\nunits = \"metric\"\nsize = \"double\"\n"),
    );
    let name = widget("Current weather", 1);
    assert!(has_text(cx, "Current weather"), "{:?}", texts(cx));
    // Every param's value, declared or not.
    assert!(has_text(cx, "Weather · Oslo · metric"), "{:?}", texts(cx));
    let size = format!("Size of {name}");
    let remove = format!("Remove {name}");
    let grab = bounds_of(cx, &handle("Current weather", 1));
    for control in [&size, &remove] {
        let bounds = bounds_of(cx, control);
        let off = (bounds.center().y - grab.center().y).abs();
        assert!(
            off < gpui_kit::px(2.),
            "{control} is off the line by {off:?}"
        );
        assert!(bounds.left() > grab.right());
    }
    let labels = labels(cx);
    assert!(!labels.iter().any(|l| l.starts_with("Move ")), "{labels:?}");
    assert!(!labels.iter().any(|l| l.starts_with("City")), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("metric\"")), "{labels:?}");
    assert!(!has_text(cx, "City"));
}

/// The size dropdown is as wide on every line, whatever size it shows: the remove buttons line
/// up.
#[gpui_kit::test]
fn size_dropdowns_are_one_width(cx: &mut TestAppContext) {
    let _s = setup_home(cx, &format!("{NOW}size = \"double\"\n\n{NOW}"));
    let double = bounds_of(cx, &format!("Size of {}", widget("Current weather", 1)));
    let default = bounds_of(cx, &format!("Size of {}", widget("Current weather", 2)));
    assert_eq!(double.size.width, default.size.width);
    assert_eq!(double.left(), default.left());
    let first = bounds_of(cx, &format!("Remove {}", widget("Current weather", 1)));
    let second = bounds_of(cx, &format!("Remove {}", widget("Current weather", 2)));
    assert_eq!(first.left(), second.left());
}

/// A widget dropped on another takes its place: one write, as the launcher's move.
#[gpui_kit::test]
fn dragging_a_widget_moves_it(cx: &mut TestAppContext) {
    let s = setup_home(cx, THREE);
    drag(
        cx,
        &handle("Current weather", 1),
        &handle("Current weather", 3),
    );
    assert_eq!(
        read(&s.config),
        "[[home.widgets]]\nwidget = \"weather/now\"\ncity = \"Rome\"\n\n\
         [[home.widgets]]\nwidget = \"weather/now\"\ncity = \"Kazan\"\n\n\
         [[home.widgets]]\nwidget = \"weather/now\"\ncity = \"Oslo\"\n"
    );
    assert!(has_text(cx, "Weather · Rome"));
    // And back up: the last onto the first.
    drag(
        cx,
        &handle("Current weather", 3),
        &handle("Current weather", 1),
    );
    assert_eq!(read(&s.config), THREE);
}

/// While a widget is dragged nothing on the page moves, and nothing is written until the
/// drop.
#[gpui_kit::test]
fn a_drag_moves_nothing_until_the_drop(cx: &mut TestAppContext) {
    let s = setup_home(cx, THREE);
    let handles = [1, 2, 3].map(|n| handle("Current weather", n));
    let tops = |cx: &mut TestAppContext| handles.clone().map(|h| top(cx, &h));
    let before = tops(cx);
    let from = bounds_of(cx, &handles[2]).center();
    let to = bounds_of(cx, &handles[0]).center();
    on_window(cx, |window, cx| {
        window.pointer_down(from, MouseButton::Left, cx);
        for step in 1..=4 {
            let y = from.y + (to.y - from.y) * (step as f32 / 4.);
            window.pointer_move(gpui_kit::point(from.x, y), Some(MouseButton::Left), cx);
        }
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.has_active_drag()), "the drag started");
    assert_eq!(tops(cx), before);
    assert_eq!(read(&s.config), THREE);
    on_window(cx, |window, cx| {
        window.pointer_up(to, MouseButton::Left, cx)
    });
    cx.run_until_parked();
    assert!(read(&s.config).starts_with(&format!("{NOW}city = \"Kazan\"\n")));
}

/// A drop where the drag began calls for no write. The file was changed outside first: any
/// write would have been refused with "try again".
#[gpui_kit::test]
fn dropping_a_widget_on_itself_writes_nothing(cx: &mut TestAppContext) {
    let s = setup_home(cx, THREE);
    let outside = THREE.replace("Kazan", "Paris");
    std::fs::write(&s.config, &outside).unwrap();
    let grab = handle("Current weather", 2);
    on_window(cx, |window, cx| {
        let from = window.find_by_label(&grab).bounds().center();
        let to = gpui_kit::point(from.x + gpui_kit::px(40.), from.y);
        window.drag(from, to, cx);
    });
    cx.run_until_parked();
    assert_eq!(read(&s.config), outside);
    assert!(!has_label(cx, "config.toml changed: try again"));
}

/// The title of widget `n` (counted from 1) on display: the `n`-th label showing `title`.
fn title_bounds(
    cx: &mut TestAppContext,
    title: &str,
    n: usize,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    on_window(cx, |window, _| {
        window
            .find_all_by_role(gpui_kit::Role::Label)
            .into_iter()
            .filter(|e| e.value() == Some(title))
            .nth(n - 1)
            .unwrap_or_else(|| panic!("no title {title:?} of widget {n}"))
            .bounds()
    })
}

type Point = gpui_kit::Point<gpui_kit::Pixels>;

/// The point `step` quarters of the way from `from` to `to`.
fn quarter(from: Point, to: Point, step: usize) -> Point {
    let part = step as f32 / 4.;
    gpui_kit::point(
        from.x + (to.x - from.x) * part,
        from.y + (to.y - from.y) * part,
    )
}

/// Presses at `from` and moves to `to` with the button held, a quarter of the way at a time;
/// does not release.
fn drag_to(cx: &mut TestAppContext, from: Point, to: Point) {
    on_window(cx, |window, cx| {
        window.pointer_down(from, MouseButton::Left, cx);
        for step in 1..=4 {
            window.pointer_move(quarter(from, to, step), Some(MouseButton::Left), cx);
        }
    });
    cx.run_until_parked();
}

fn release(cx: &mut TestAppContext, at: Point) {
    on_window(cx, |window, cx| {
        window.pointer_up(at, MouseButton::Left, cx)
    });
    cx.run_until_parked();
}

fn dragging(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.has_active_drag())
}

/// The whole line is what a widget is dragged by: its title, and the room between its text
/// and its controls.
#[gpui_kit::test]
fn a_widget_is_dragged_by_its_whole_line(cx: &mut TestAppContext) {
    let s = setup_home(cx, THREE);
    let title = title_bounds(cx, "Current weather", 1).center();
    let to = bounds_of(cx, &handle("Current weather", 3)).center();
    drag_to(cx, title, to);
    assert!(dragging(cx), "a drag from the title");
    release(cx, to);
    assert!(read(&s.config).ends_with(&format!("{NOW}city = \"Oslo\"\n")));
    std::fs::write(&s.config, THREE).unwrap();
    config_changed(cx);
    // Just left of the size dropdown, past the end of the text.
    let size = bounds_of(cx, &format!("Size of {}", widget("Current weather", 3)));
    let gap = gpui_kit::point(size.left() - gpui_kit::px(4.), size.center().y);
    let to = bounds_of(cx, &handle("Current weather", 1)).center();
    drag_to(cx, gap, to);
    assert!(
        dragging(cx),
        "a drag from between the text and the controls"
    );
    release(cx, to);
    assert!(read(&s.config).starts_with(&format!("{NOW}city = \"Kazan\"\n")));
}

/// A drag starts anywhere on the widget's row of the box, also in the row's padding around
/// its line: left of the grip and over the title.
#[gpui_kit::test]
fn a_widget_is_dragged_from_its_row_padding(cx: &mut TestAppContext) {
    let s = setup_home(cx, THREE);
    let line = on_window(cx, |window, _| window.find(("home-line", 0usize)).bounds());
    let row = on_window(cx, |window, _| {
        window.find(("home-widget", 0usize)).bounds()
    });
    assert!(
        row.left() < line.left() && row.top() < line.top(),
        "{row:?} {line:?}"
    );
    // The row of the box, inside its border.
    let frame = on_window(cx, |window, _| {
        window.find_all("settings-group-box")[0].bounds()
    });
    let border = gpui_kit::px(1.);
    assert_eq!(row.left(), frame.left() + border, "{row:?} in {frame:?}");
    assert_eq!(row.right(), frame.right() - border, "{row:?} in {frame:?}");
    assert_eq!(row.top(), frame.top() + border, "{row:?} in {frame:?}");
    // What takes a drop from below, and its insertion line, are as wide as the row: the line
    // is as long whichever way the widget comes.
    let before = on_window(cx, |window, _| {
        window.find(("home-drop-before", 0usize)).bounds()
    });
    assert_eq!(
        (before.left(), before.right()),
        (row.left(), row.right()),
        "{before:?} in {row:?}"
    );
    let left = gpui_kit::point((row.left() + line.left()) / 2., line.center().y);
    let to = bounds_of(cx, &handle("Current weather", 3)).center();
    drag_to(cx, left, to);
    assert!(dragging(cx), "a drag from the padding left of the line");
    release(cx, to);
    assert!(read(&s.config).ends_with(&format!("{NOW}city = \"Oslo\"\n")));
    std::fs::write(&s.config, THREE).unwrap();
    config_changed(cx);
    let above = gpui_kit::point(line.center().x, (row.top() + line.top()) / 2.);
    drag_to(cx, above, to);
    assert!(dragging(cx), "a drag from the padding over the line");
    release(cx, to);
    assert!(read(&s.config).ends_with(&format!("{NOW}city = \"Oslo\"\n")));
}

/// Presses on the control of widget 1 labelled `control(name)` and moves onto widget 3:
/// nothing is dragged and nothing written.
fn a_control_does_not_start_a_drag(cx: &mut TestAppContext, control: impl Fn(&str) -> String) {
    let s = setup_home(cx, THREE);
    let control = control(&widget("Current weather", 1));
    let from = bounds_of(cx, &control).center();
    let to = bounds_of(cx, &handle("Current weather", 3)).center();
    drag_to(cx, from, to);
    assert!(!dragging(cx), "{control} started a drag");
    release(cx, to);
    assert_eq!(read(&s.config), THREE);
}

/// The size dropdown is clicked, never dragged.
#[gpui_kit::test]
fn the_size_dropdown_does_not_start_a_drag(cx: &mut TestAppContext) {
    a_control_does_not_start_a_drag(cx, |name| format!("Size of {name}"));
}

/// Remove is clicked, never dragged.
#[gpui_kit::test]
fn remove_does_not_start_a_drag(cx: &mut TestAppContext) {
    a_control_does_not_start_a_drag(cx, |name| format!("Remove {name}"));
}

/// While dragged, the widget's whole row of the box follows the pointer, as large as the
/// row, where it was lifted from.
#[gpui_kit::test]
fn the_drag_image_is_the_whole_line(cx: &mut TestAppContext) {
    let _s = setup_home(cx, THREE);
    let line = on_window(cx, |window, _| {
        window.find(("home-widget", 2usize)).bounds()
    });
    let from = title_bounds(cx, "Current weather", 3).center();
    let to = bounds_of(cx, &handle("Current weather", 1)).center();
    drag_to(cx, from, to);
    assert!(dragging(cx));
    let image = on_window(cx, |window, _| window.find("home-drag-image").bounds());
    assert_eq!(image.size, line.size);
    // Lifted in place: held at the point the drag began (the first move) as the line was.
    let began = quarter(from, to, 1);
    let held = to - image.origin;
    let taken = began - line.origin;
    let off = (held.x - taken.x).abs().max((held.y - taken.y).abs());
    assert!(off < gpui_kit::px(0.5), "{held:?} {taken:?}");
    // The same content: the title and the plugin's name and values, once more.
    let shown = texts(cx);
    assert_eq!(
        shown.iter().filter(|t| *t == "Weather · Kazan").count(),
        2,
        "{shown:?}"
    );
    release(cx, to);
    assert!(
        on_window(cx, |window, _| window.try_find("home-drag-image")).is_none(),
        "the image is gone after the drop"
    );
}

/// The drag image is only a picture: its size dropdown and remove look as the line's do, in
/// the same places, but are not controls and have no names, so a screen reader finds the
/// dragged widget's size once.
#[gpui_kit::test]
fn the_drag_image_names_nothing(cx: &mut TestAppContext) {
    let text = THREE.replace("city = \"Kazan\"", "size = \"double\"\ncity = \"Kazan\"");
    let _s = setup_home(cx, &text);
    let size_label = format!("Size of {}", widget("Current weather", 3));
    let remove_label = format!("Remove {}", widget("Current weather", 3));
    let named = |cx: &mut TestAppContext| {
        on_window(cx, |window, _| {
            let buttons = window.find_all_by_role(gpui_kit::Role::Button).len();
            let double = window
                .elements()
                .iter()
                .filter(|e| {
                    [e.label(), e.value(), e.description()]
                        .into_iter()
                        .flatten()
                        .any(|text| text.contains("Double"))
                })
                .count();
            (buttons, double)
        })
    };
    let before = named(cx);
    assert_eq!(before.1, 1, "the dropdown alone says Double");
    let line = on_window(cx, |window, _| {
        window.find(("home-widget", 2usize)).bounds()
    });
    let size = bounds_of(cx, &size_label);
    let remove = bounds_of(cx, &remove_label);
    let from = title_bounds(cx, "Current weather", 3).center();
    let to = bounds_of(cx, &handle("Current weather", 1)).center();
    drag_to(cx, from, to);
    assert!(dragging(cx));
    assert_eq!(named(cx), before);
    let (image, image_size, image_remove) = on_window(cx, |window, _| {
        (
            window.find("home-drag-image").bounds(),
            window.find(("home-image-size", 2usize)).bounds(),
            window.find(("home-image-remove", 2usize)).bounds(),
        )
    });
    // The same look in the same place: as large as the line's controls, as far from its edge.
    assert_eq!(image_size.size, size.size);
    assert_eq!(image_size.origin - image.origin, size.origin - line.origin);
    assert_eq!(image_remove.size, remove.size);
    assert_eq!(
        image_remove.origin - image.origin,
        remove.origin - line.origin
    );
    release(cx, to);
}

/// The line a widget is dragged from stays in place, dimmed, while the drag is on, and only
/// it; a drop where no widget is gives it back its look.
#[gpui_kit::test]
fn the_dragged_line_is_dimmed_until_the_drop(cx: &mut TestAppContext) {
    let _s = setup_home(cx, THREE);
    let dimmed = |cx: &mut TestAppContext| {
        on_window(cx, |window, _| {
            (0..3usize)
                .filter(|&tile| window.try_find(("home-line-dimmed", tile)).is_some())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(dimmed(cx), Vec::<usize>::new());
    let from = title_bounds(cx, "Current weather", 2).center();
    let outside = bounds_of(
        cx,
        "Add widgets from the launcher: choose Add to Home on a result.",
    )
    .center();
    drag_to(cx, from, outside);
    assert!(dragging(cx));
    assert_eq!(dimmed(cx), vec![1]);
    let line = on_window(cx, |window, _| window.find(("home-line", 1usize)).bounds());
    let mark = on_window(cx, |window, _| {
        window.find(("home-line-dimmed", 1usize)).bounds()
    });
    assert!(line.contains(&mark.origin), "{mark:?} in {line:?}");
    release(cx, outside);
    assert!(!dragging(cx));
    assert_eq!(dimmed(cx), Vec::<usize>::new());
}

/// A drag dropped where no widget is writes nothing.
#[gpui_kit::test]
fn a_widget_dropped_outside_the_lines_writes_nothing(cx: &mut TestAppContext) {
    let s = setup_home(cx, THREE);
    drag(
        cx,
        &handle("Current weather", 2),
        "Add widgets from the launcher: choose Add to Home on a result.",
    );
    assert!(!dragging(cx));
    assert_eq!(read(&s.config), THREE);
    assert!(!not_saved(cx));
}

/// The launcher's index of a widget skips the entries it cannot read; such an entry is
/// neither dragged nor dropped on.
#[gpui_kit::test]
fn an_unreadable_entry_is_not_draggable(cx: &mut TestAppContext) {
    let unreadable = "[[home.widgets]]\nwidget = 5\n\n";
    let s = setup_home(
        cx,
        &format!("{unreadable}{NOW}city = \"Oslo\"\n\n{NOW}city = \"Rome\"\n"),
    );
    drag(
        cx,
        &handle("Current weather", 1),
        &handle("Current weather", 2),
    );
    let moved = format!("{unreadable}{NOW}city = \"Rome\"\n\n{NOW}city = \"Oslo\"\n");
    assert_eq!(read(&s.config), moved);
    let outside = moved.replace("Oslo", "Paris");
    std::fs::write(&s.config, &outside).unwrap();
    let entry = "An entry the launcher cannot read";
    drag(cx, entry, &handle("Current weather", 2));
    drag(cx, &handle("Current weather", 2), entry);
    assert_eq!(read(&s.config), outside);
    assert!(!has_label(cx, "config.toml changed: try again"));
}

/// No accessible name or text on the page says "tile": a widget's row, a greyed one, an
/// unreadable entry, a size that is not valid and a write refused.
#[gpui_kit::test]
fn no_tile_in_accessible_names(cx: &mut TestAppContext) {
    let s = setup_home(
        cx,
        &format!(
            "{NOW}city = \"Oslo\"\nsize = \"huge\"\n\n[[home.widgets]]\nwidget = 5\n\n\
             [[home.widgets]]\nwidget = \"clock/analog\"\n\n\
             [[home.widgets]]\nwidget = \"weather/radar\"\n"
        ),
    );
    std::fs::write(&s.config, format!("{NOW}city = \"Rome\"\n")).unwrap();
    click(cx, &format!("Remove {}", widget("Current weather", 1)));
    assert!(has_label(cx, "config.toml changed: try again"));
    let no_tile = |cx: &mut TestAppContext| {
        let shown = [labels(cx), texts(cx)].concat();
        let tiles: Vec<_> = shown
            .into_iter()
            .filter(|l| l.to_lowercase().contains("tile"))
            .collect();
        assert!(tiles.is_empty(), "{tiles:?}");
    };
    no_tile(cx);
    std::fs::write(
        &s.config,
        format!(
            "{NOW}size = \"huge\"\n\n[[home.widgets]]\nwidget = 5\n\n\
             [[home.widgets]]\nwidget = \"clock/analog\"\n\n\
             [[home.widgets]]\nwidget = \"weather/radar\"\n"
        ),
    )
    .unwrap();
    config_changed(cx);
    assert!(has_text(cx, "clock · Not installed"), "{:?}", texts(cx));
    // The plugin's name is in what the line says.
    assert!(has_text(cx, "radar"), "{:?}", texts(cx));
    assert!(has_text(cx, "Not offered by Weather"), "{:?}", texts(cx));
    assert!(has_label(cx, "An entry the launcher cannot read"));
    assert!(has_label(
        cx,
        "Not valid here: \"huge\"; the default is used"
    ));
    no_tile(cx);
}

#[gpui_kit::test]
fn size_default_removes_the_size_key(cx: &mut TestAppContext) {
    let s = setup_home(
        cx,
        &format!("{NOW}size = \"double\" # wide\ncity = \"Oslo\"\n"),
    );
    let name = widget("Current weather", 1);
    click(cx, &format!("Size of {name}"));
    // Only the sizes the widget declares.
    assert!(!has_label(cx, "Quarter"));
    click(cx, "Single");
    assert_eq!(
        read(&s.config),
        format!("{NOW}size = \"single\" # wide\ncity = \"Oslo\"\n")
    );
    click(cx, &format!("Size of {name}"));
    click(cx, "Default");
    assert_eq!(read(&s.config), format!("{NOW}city = \"Oslo\"\n"));
    assert!(shows(cx, &format!("Size of {name}"), "Default"));
}

/// A widget no installed plugin offers is greyed: it has no size dropdown, and is dragged
/// and removed as any other.
#[gpui_kit::test]
fn an_uninstalled_widget_is_movable_not_editable(cx: &mut TestAppContext) {
    let clock = "[[home.widgets]]\nwidget = \"clock/analog\"\nsize = \"double\"\ncity = \"Oslo\"\n";
    let s = setup_home(cx, &format!("{clock}\n{NOW}"));
    // Titled by the widget's id within its plugin; the plugin's id leads the second line.
    assert!(has_text(cx, "analog"), "{:?}", texts(cx));
    assert!(!has_text(cx, "clock/analog"), "{:?}", texts(cx));
    assert!(
        has_text(cx, "clock · Not installed · Oslo"),
        "{:?}",
        texts(cx)
    );
    assert!(!labels(cx).iter().any(|l| l.starts_with("Size of analog")));
    drag(cx, &handle("analog", 1), &handle("Current weather", 2));
    assert_eq!(read(&s.config), format!("{NOW}\n{clock}"));
    click(cx, &format!("Remove {}", widget("analog", 2)));
    assert_eq!(read(&s.config), NOW);
}

/// A Home edit against a list the file no longer holds writes nothing, shows the file and
/// says why on the row.
#[gpui_kit::test]
fn a_home_edit_of_a_changed_file_says_try_again(cx: &mut TestAppContext) {
    let s = setup_home(cx, &format!("{NOW}\n{NOW}city = \"Rome\"\n"));
    let outside = format!("{NOW}city = \"Oslo\"\n\n{NOW}city = \"Rome\"\n");
    std::fs::write(&s.config, &outside).unwrap();
    drag(
        cx,
        &handle("Current weather", 1),
        &handle("Current weather", 2),
    );
    assert_eq!(read(&s.config), outside);
    assert!(has_label(cx, "config.toml changed: try again"));
    assert!(has_text(cx, "Weather · Oslo"));
}

/// A file the launcher refuses is never written, even when the window has not heard of it.
#[gpui_kit::test]
fn a_home_edit_of_a_refused_file_writes_nothing(cx: &mut TestAppContext) {
    let s = setup_home(cx, &format!("{NOW}\n{NOW}"));
    let refused = format!("[launcher]\nhotkey = 5\n\n{NOW}\n{NOW}");
    std::fs::write(&s.config, &refused).unwrap();
    click(cx, &format!("Remove {}", widget("Current weather", 2)));
    assert_eq!(read(&s.config), refused);
    assert!(not_saved(cx));
}

/// While the file has an error the rows are disabled: remove does nothing and a widget is
/// not dragged.
#[gpui_kit::test]
fn a_broken_file_disables_the_home_rows(cx: &mut TestAppContext) {
    let broken = format!("[launcher]\nhotkey = 5\n\n{NOW}city = \"Oslo\"\n\n{NOW}");
    let s = setup_home(cx, &broken);
    let remove = format!("Remove {}", widget("Current weather", 1));
    assert_eq!(disabled(cx, &remove), Some(true));
    click(cx, &remove);
    drag(
        cx,
        &handle("Current weather", 1),
        &handle("Current weather", 2),
    );
    assert_eq!(read(&s.config), broken);
    // A drop that went through would say why it wrote nothing.
    assert!(!not_saved(cx));
}

/// Edits name the widget among the entries the launcher reads: one it cannot read before
/// them is not counted.
#[gpui_kit::test]
fn edits_of_a_later_widget_change_that_widget(cx: &mut TestAppContext) {
    let unreadable = "[[home.widgets]]\nwidget = 5\n\n";
    let s = setup_home(
        cx,
        &format!("{unreadable}{NOW}city = \"Oslo\"\n\n{NOW}city = \"Rome\"\n"),
    );
    let second = widget("Current weather", 2);
    click(cx, &format!("Size of {second}"));
    click(cx, "Double");
    assert_eq!(
        read(&s.config),
        format!("{unreadable}{NOW}city = \"Oslo\"\n\n{NOW}city = \"Rome\"\nsize = \"double\"\n")
    );
    click(cx, &format!("Remove {second}"));
    assert_eq!(
        read(&s.config),
        format!("{unreadable}{NOW}city = \"Oslo\"\n")
    );
}

/// A size the widget cannot have is shown as an error, and Default removes it.
#[gpui_kit::test]
fn an_invalid_size_is_shown_and_default_removes_it(cx: &mut TestAppContext) {
    let s = setup_home(
        cx,
        &format!("{NOW}size = \"huge\"\n\n{NOW}size = \"quarter\"\n"),
    );
    assert!(has_label(
        cx,
        "Not valid here: \"huge\"; the default is used"
    ));
    assert!(has_label(
        cx,
        "Not valid here: \"quarter\"; the default is used"
    ));
    click(cx, &format!("Size of {}", widget("Current weather", 1)));
    click(cx, "Default");
    assert_eq!(
        read(&s.config),
        format!(
            "{NOW}
{NOW}size = \"quarter\"\n"
        )
    );
    assert!(!has_label(
        cx,
        "Not valid here: \"huge\"; the default is used"
    ));
}

// Outside edits and the conflict question.

use gpui_kit::component::WindowExt as _;

fn dialog_open(cx: &mut TestAppContext) -> bool {
    on_window(cx, |window, cx| window.has_active_dialog(cx))
}

fn recording(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<SettingsController>().is_recording(cx))
}

/// The refusal under Weather's precision field.
const PRECISION_HINT: &str = "Enter a whole number from 0 to 20.";

const ASKED_GREETING: &str = "config.toml changed while you were editing Greeting.";

/// Weather's page over `text`, "x" typed after the greeting ("Annx" over the default) and
/// not confirmed.
fn typing_greeting(cx: &mut TestAppContext, text: &str) -> Setup {
    let s = setup(cx, text);
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "x");
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
    s
}

/// The greeting and the launcher's autostart, changed by hand.
const GREETING_BOB: &str =
    "[launcher]\nautostart = false\n\n[plugins.weather]\ngreeting = \"Bob\"\n";

#[gpui_kit::test]
fn outside_edit_updates_controls_without_a_question(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    std::fs::write(
        &s.config,
        "[launcher]\nautostart = false\n\n[plugins.weather]\ngreeting = \"Bob\"\nalerts = false\n",
    )
    .unwrap();
    config_changed(cx);
    assert!(!dialog_open(cx));
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Bob"));
    assert_eq!(checked(cx, "Alerts"), Some(false));
    open(cx, Some(SettingsPage::General));
    assert_eq!(checked(cx, "Start at login"), Some(false));
}

#[gpui_kit::test]
fn own_write_raises_nothing(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    // Out of range: refused, so it stays typed when the switch takes the focus.
    click(cx, "Precision");
    press(cx, "end");
    type_text(cx, "0");
    press(cx, "enter");
    click(cx, "Alerts");
    assert_eq!(read(&s.config), "[plugins.weather]\nalerts = false\n");
    config_changed(cx);
    assert!(!dialog_open(cx));
    // Still typed: the refusal of the typed text is still shown.
    assert!(has_label(cx, PRECISION_HINT));
    assert_eq!(checked(cx, "Alerts"), Some(false));
}

#[gpui_kit::test]
fn outside_edit_of_the_typed_key_asks(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "");
    std::fs::write(&s.config, GREETING_BOB).unwrap();
    config_changed(cx);
    assert!(dialog_open(cx));
    assert!(has_label(cx, ASKED_GREETING));
    assert!(has_label(cx, "Yours: Annx"));
    assert!(has_label(cx, "In the file: Bob"));
    assert!(has_label(cx, "Keep mine"));
    assert!(has_label(cx, "Use the file's"));
    // The question took the focus: the typed text is not written by that.
    assert_eq!(read(&s.config), GREETING_BOB);
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
}

#[gpui_kit::test]
fn keep_mine_writes_on_top_of_the_new_file(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "");
    std::fs::write(&s.config, GREETING_BOB).unwrap();
    config_changed(cx);
    click(cx, "Keep mine");
    assert!(!dialog_open(cx));
    assert_eq!(
        read(&s.config),
        "[launcher]\nautostart = false\n\n[plugins.weather]\ngreeting = \"Annx\"\n"
    );
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
    // Its own write: nothing more is asked.
    config_changed(cx);
    assert!(!dialog_open(cx));
}

#[gpui_kit::test]
fn use_the_files_drops_the_typed_text(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "");
    std::fs::write(&s.config, GREETING_BOB).unwrap();
    config_changed(cx);
    click(cx, "Use the file's");
    assert!(!dialog_open(cx));
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Bob"));
    assert_eq!(read(&s.config), GREETING_BOB);
}

/// Esc closes the question without an answer: the typed text stays, unwritten, until the
/// user confirms or restores it.
#[gpui_kit::test]
fn esc_on_the_question_keeps_the_typed_text_unwritten(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "");
    std::fs::write(&s.config, GREETING_BOB).unwrap();
    config_changed(cx);
    press(cx, "escape");
    assert!(!dialog_open(cx));
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
    assert_eq!(read(&s.config), GREETING_BOB);
    // The field has the focus again; Enter writes the typed text on top of the file.
    press(cx, "enter");
    assert_eq!(
        read(&s.config),
        "[launcher]\nautostart = false\n\n[plugins.weather]\ngreeting = \"Annx\"\n"
    );
}

#[gpui_kit::test]
fn outside_edit_of_another_key_leaves_the_typed_field(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "");
    std::fs::write(&s.config, "[plugins.weather]\nalerts = false\n").unwrap();
    config_changed(cx);
    assert!(!dialog_open(cx));
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
    assert_eq!(checked(cx, "Alerts"), Some(false));
    press(cx, "enter");
    assert_eq!(
        read(&s.config),
        "[plugins.weather]\nalerts = false\ngreeting = \"Annx\"\n"
    );
}

#[gpui_kit::test]
fn broken_file_while_typing_keeps_text_and_writes_nothing(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "[plugins.weather]\ngreeting = \"Ann\"\n");
    let broken = "[plugins.weather\ngreeting = \"Bob\"\n";
    std::fs::write(&s.config, broken).unwrap();
    config_changed(cx);
    assert!(!dialog_open(cx));
    assert!(on_window(cx, |window, _| window
        .try_find("settings-banner")
        .is_some()));
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
    press(cx, "enter");
    click(cx, "Search...");
    assert_eq!(read(&s.config), broken);
    // Fixed with another greeting than the one last read: that is asked about.
    std::fs::write(&s.config, "[plugins.weather]\ngreeting = \"Bob\"\n").unwrap();
    config_changed(cx);
    assert!(dialog_open(cx));
    assert!(has_label(cx, "Yours: Annx"));
    assert!(has_label(cx, "In the file: Bob"));
}

/// A second conflicting field waits until the first is answered.
#[gpui_kit::test]
fn a_second_conflict_waits_for_the_first(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    // Out of range: refused, so it stays typed.
    click(cx, "Precision");
    press(cx, "end");
    type_text(cx, "0");
    press(cx, "enter");
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "x");
    assert_eq!(read(&s.config), "");
    let outside = "[plugins.weather]\ngreeting = \"Bob\"\nprecision = 5\n";
    std::fs::write(&s.config, outside).unwrap();
    config_changed(cx);
    assert!(has_label(cx, ASKED_GREETING));
    click(cx, "Keep mine");
    assert_eq!(
        read(&s.config),
        "[plugins.weather]\ngreeting = \"Annx\"\nprecision = 5\n"
    );
    assert!(dialog_open(cx));
    assert!(has_label(
        cx,
        "config.toml changed while you were editing Precision."
    ));
    assert!(has_label(cx, "Yours: 100"));
    assert!(has_label(cx, "In the file: 5"));
    click(cx, "Use the file's");
    assert!(!dialog_open(cx));
    assert!(!has_label(cx, PRECISION_HINT));
    assert_eq!(
        read(&s.config),
        "[plugins.weather]\ngreeting = \"Annx\"\nprecision = 5\n"
    );
}

/// A recording goes on through an outside edit of its hotkey, without a question: Esc then
/// shows the file's hotkey (see `esc_shows_the_hotkey_the_file_has_now`).
#[gpui_kit::test]
fn an_outside_edit_does_not_interrupt_a_recording(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Launcher hotkey");
    press(cx, "ctrl-alt-k");
    std::fs::write(&s.config, "[launcher]\nhotkey = \"Super+K\"\n").unwrap();
    config_changed(cx);
    assert!(!dialog_open(cx));
    assert!(recording(cx));
    press(cx, "enter");
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Launcher,
            Some(hk("ctrl+alt+k")),
            HotkeyOrigin::Settings
        )]
    );
}

#[gpui_kit::test]
fn recording_ends_when_the_page_changes(cx: &mut TestAppContext) {
    let _s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Firefox");
    assert!(recording(cx));
    click_page(cx, "General");
    assert!(!recording(cx));

    open(cx, Some(SettingsPage::Hotkeys));
    click(cx, "Firefox");
    assert!(recording(cx));
    open(cx, Some(SettingsPage::Location));
    assert!(!recording(cx));
}

/// Enter on the question (the key the user was likely pressing) answers nothing.
#[gpui_kit::test]
fn enter_on_the_question_answers_nothing(cx: &mut TestAppContext) {
    let s = typing_greeting(cx, "");
    std::fs::write(&s.config, GREETING_BOB).unwrap();
    config_changed(cx);
    press(cx, "enter");
    assert!(dialog_open(cx));
    assert_eq!(read(&s.config), GREETING_BOB);
    click(cx, "Use the file's");
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Bob"));
}

/// Esc on the first of two questions: its typed text stays unwritten while the second asks.
#[gpui_kit::test]
fn esc_on_the_first_question_keeps_its_text_while_the_second_asks(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Precision");
    press(cx, "end");
    type_text(cx, "0");
    press(cx, "enter");
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "x");
    let outside = "[plugins.weather]\ngreeting = \"Bob\"\nprecision = 5\n";
    std::fs::write(&s.config, outside).unwrap();
    config_changed(cx);
    assert!(has_label(cx, ASKED_GREETING));
    press(cx, "escape");
    assert!(has_label(
        cx,
        "config.toml changed while you were editing Precision."
    ));
    assert_eq!(read(&s.config), outside);
    click(cx, "Use the file's");
    assert!(!dialog_open(cx));
    assert_eq!(read(&s.config), outside);
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Annx"));
}

/// A number written in another spelling ("05" as 5) is shown as written: nothing stays
/// typed, so a later outside edit of it is shown without a question.
#[gpui_kit::test]
fn a_confirmed_number_is_shown_as_written(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Precision");
    press(cx, "end");
    press(cx, "backspace");
    press(cx, "backspace");
    type_text(cx, "05");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.weather]\nprecision = 5\n");
    std::fs::write(&s.config, "[plugins.weather]\nprecision = 7\n").unwrap();
    config_changed(cx);
    assert!(!dialog_open(cx));
    // The field shows 7: Enter in it has nothing to write.
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.weather]\nprecision = 7\n");
}

#[gpui_kit::test]
fn choosing_dark_switches_the_theme(cx: &mut TestAppContext) {
    setup(cx, "[appearance]\ntheme = \"light\"\n");
    open(cx, None);
    cx.update(|cx| tomakast_ui::set_theme_preference(ThemeChoice::Light, cx));
    cx.run_until_parked();
    assert!(!cx.update(|cx| cx.theme().mode.is_dark()));
    cx.update(|cx| tomakast_ui::set_theme_preference(ThemeChoice::Dark, cx));
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.theme().mode.is_dark()));
}

// The layout: sidebar sections and icons, search.

/// The sidebar element labelled `label`.
fn in_sidebar(cx: &mut TestAppContext, label: &str) -> Option<gpui_kit::test::ElementSnapshot> {
    on_window(cx, |window, _| {
        window.within("settings-sidebar").try_find_by_label(label)
    })
}

/// The sidebar lists the launcher's pages, then the plugins', with no section titles: a gap
/// wider than the one between entries separates the two.
#[gpui_kit::test]
fn sidebar_has_sections_in_order(cx: &mut TestAppContext) {
    let _s = setup(cx, "[plugins.gone]\nx = 1\n");
    set_catalog(cx, catalog(vec![plugin("calc", "Calculator"), weather()]));
    open(cx, None);
    let order = [
        "General",
        "Launcher",
        "Appearance",
        "Location",
        "Hotkeys",
        "Widgets",
        "About",
        "Calculator",
        "Weather",
        "gone (not installed)",
    ];
    let bounds: Vec<_> = order
        .iter()
        .map(|label| {
            in_sidebar(cx, label)
                .unwrap_or_else(|| panic!("{label} is not in the sidebar"))
                .bounds()
        })
        .collect();
    for (pair, labels) in bounds.windows(2).zip(order.windows(2)) {
        assert!(
            pair[0].top() < pair[1].top(),
            "{} is not above {}",
            labels[0],
            labels[1]
        );
    }
    let gap = |a: usize| bounds[a + 1].top() - bounds[a].bottom();
    let between_entries = gap(0);
    let between_sections = gap(6);
    assert_eq!(
        between_sections - between_entries,
        gpui_kit::px(20.),
        "the sections are {between_sections:?} apart, entries {between_entries:?}"
    );
    for title in ["tomakast", "Plugins", "Core", "Modules"] {
        assert!(in_sidebar(cx, title).is_none(), "{title} is in the sidebar");
    }
}

/// The sidebar and the page keep one grid: the entries stand as far from the sidebar's edges as
/// the boxes from the page's, and the first entry starts on the first box's line.
#[gpui_kit::test]
fn the_sidebar_and_the_page_share_one_margin(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, None);
    let (sidebar, page, entry, group) = on_window(cx, |window, _| {
        let all = window.elements();
        let first = |name: &'static str| {
            all.iter()
                .find(|e| e.path().last() == Some(&name.into()))
                .unwrap_or_else(|| panic!("no {name}"))
                .bounds()
        };
        (
            first("settings-sidebar"),
            first("settings-page"),
            first("sidebar-entry-fill"),
            first("settings-group-box"),
        )
    });
    let margin = entry.left() - sidebar.left();
    assert_eq!(
        sidebar.right() - entry.right(),
        margin,
        "the sidebar's right margin"
    );
    assert_eq!(group.left() - page.left(), margin, "the page's left margin");
    assert_eq!(
        page.right() - group.right(),
        margin,
        "the page's right margin"
    );
    assert_eq!(group.top() - page.top(), margin, "the page's top margin");
    assert_eq!(
        entry.top(),
        group.top(),
        "the first entry and the first box"
    );
}

/// Presses the mouse on the page's bottom right corner, where no control is, and lets go.
fn click_empty_space(cx: &mut TestAppContext) {
    on_window(cx, |window, cx| {
        let page = window
            .elements()
            .into_iter()
            .find(|e| e.path().last() == Some(&"settings-page".into()))
            .expect("the page area")
            .bounds();
        let at = gpui_kit::point(
            page.right() - gpui_kit::px(8.),
            page.bottom() - gpui_kit::px(8.),
        );
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    cx.run_until_parked();
}

/// A click on empty space ends the edit in a field, as leaving it does: the typed text is
/// written.
#[gpui_kit::test]
fn clicking_empty_space_blurs_a_text_field(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "e");
    assert_eq!(read(&s.config), "");
    click_empty_space(cx);
    assert_eq!(read(&s.config), "[plugins.weather]\ngreeting = \"Anne\"\n");
    // The field no longer has the keys: more typing changes nothing.
    type_text(cx, "x");
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Anne"));
}

/// The window's background taking the focus does not stop a field taking it.
#[gpui_kit::test]
fn clicking_a_field_still_focuses_it(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    click_empty_space(cx);
    click(cx, "Greeting");
    press(cx, "end");
    type_text(cx, "e");
    assert_eq!(value_of(cx, "Greeting").as_deref(), Some("Anne"));
    click(cx, "Search...");
    assert_eq!(read(&s.config), "[plugins.weather]\ngreeting = \"Anne\"\n");
}

/// A click on empty space ends a recording: what was pressed is asked for, and with nothing
/// pressed nothing is.
#[gpui_kit::test]
fn a_recording_hotkey_field_ends_on_a_click_elsewhere(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Launcher hotkey");
    press(cx, "ctrl-alt-k");
    assert!(recording(cx));
    click_empty_space(cx);
    assert!(!recording(cx));
    assert_eq!(
        *s.requests.borrow(),
        [(
            HotkeyTarget::Launcher,
            Some(hk("ctrl+alt+k")),
            HotkeyOrigin::Settings
        )]
    );

    click(cx, "Launcher hotkey");
    assert!(recording(cx));
    click_empty_space(cx);
    assert!(!recording(cx));
    assert_eq!(s.requests.borrow().len(), 1);
    assert_eq!(read(&s.config), "");
}

/// With no plugin page, the sidebar is the built-in pages alone, with no title and no gap
/// after them.
#[gpui_kit::test]
fn builtin_pages_are_one_section(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, None);
    let gaps = on_window(cx, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|e| {
                e.path()
                    .last()
                    .is_some_and(|id| format!("{id:?}").contains("sidebar-gap"))
            })
            .count()
    });
    assert_eq!(gaps, 0);
    for title in ["tomakast", "Plugins"] {
        assert!(in_sidebar(cx, title).is_none(), "{title} is in the sidebar");
    }
}

/// The launcher hotkey is a row of Launcher, and only there: not on General, and not on the
/// Hotkeys page, which lists items only.
#[gpui_kit::test]
fn the_launcher_hotkey_is_only_on_launcher(cx: &mut TestAppContext) {
    let _s = setup_items(cx, "[hotkeys]\n\"Ctrl+Alt+F\" = \"demo/firefox\"\n");
    open(cx, Some(SettingsPage::Hotkeys));
    assert!(has_label(cx, "Firefox"));
    assert!(!has_label(cx, "Launcher hotkey"));
    open(cx, Some(SettingsPage::General));
    assert!(!has_label(cx, "Launcher hotkey"));
    open(cx, Some(SettingsPage::Launcher));
    assert!(has_label(cx, "Launcher hotkey"));
}

#[gpui_kit::test]
fn plugin_entries_have_no_sub_entries(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![weather()]));
    open(cx, Some(SettingsPage::Plugin("weather".into())));
    for heading in ["Preferences", "Commands"] {
        assert!(
            in_sidebar(cx, heading).is_none(),
            "{heading} in the sidebar"
        );
        let (all, in_page) = on_window(cx, |window, _| {
            let all = window.find_all_by_label(heading).len();
            let in_page = window
                .within("settings-page")
                .try_find_by_label(heading)
                .is_some();
            (all, in_page)
        });
        assert_eq!(all, 1, "{heading}");
        assert!(in_page, "{heading} is not a heading of the page");
    }
}

/// Every sidebar entry has an icon: the built-in pages their Lucide icons, a plugin the icon
/// of its first command that has one or else its monogram, a table its monogram.
#[gpui_kit::test]
fn every_sidebar_entry_has_an_icon(cx: &mut TestAppContext) {
    let _s = setup(cx, "[plugins.gone]\nx = 1\n");
    set_catalog(cx, catalog(vec![plugin("calc", "Calculator"), weather()]));
    open(cx, None);
    let icons: Vec<(String, Option<String>)> = on_window(cx, |window, _| {
        let sidebar = window.within("settings-sidebar");
        let wrappers = sidebar.find_all("sidebar-icon");
        let glyphs = sidebar.find_all("icon");
        sidebar
            .find_all_by_role(gpui_kit::Role::ListItem)
            .iter()
            .map(|entry| {
                let inside =
                    |e: &&gpui_kit::test::ElementSnapshot| e.path().starts_with(entry.path());
                let icon = wrappers
                    .iter()
                    .find(inside)
                    .and_then(|_| glyphs.iter().find(inside))
                    .and_then(|glyph| glyph.label().map(str::to_string));
                (entry.label().unwrap_or_default().to_string(), icon)
            })
            .collect()
    });
    let expected = [
        ("General", "settings"),
        ("Launcher", "search"),
        ("Appearance", "palette"),
        ("Location", "map-pin"),
        ("Hotkeys", "keyboard"),
        ("Widgets", "layout-grid"),
        ("About", "info"),
        ("Calculator", "C"),
        ("Weather", "cloud"),
        ("gone (not installed)", "G"),
    ]
    .map(|(entry, icon)| (entry.to_string(), Some(icon.to_string())));
    assert_eq!(icons, expected);
}

#[gpui_kit::test]
fn no_plugins_section_when_empty(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    set_catalog(cx, catalog(vec![]));
    open(cx, None);
    assert!(in_sidebar(cx, "General").is_some());
    assert!(in_sidebar(cx, "About").is_some());
    assert!(!has_label(cx, "Plugins"));
}

#[gpui_kit::test]
fn search_filters_and_shows_the_empty_state(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, None);
    click(cx, "Search...");
    type_text(cx, "wifi");
    assert_eq!(pages(cx), ["Location"]);
    assert!(has_label(cx, "Look up location by Wi-Fi"));
    assert!(!has_label(cx, "Start at login"));

    // Esc clears the query; the page the search put on display stays.
    press(cx, "escape");
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
    assert!(has_label(cx, "Look up location by Wi-Fi"));

    type_text(cx, "zzz");
    assert!(pages(cx).is_empty());
    assert!(has_label(cx, "No settings match"));

    press(cx, "escape");
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
    assert_eq!(
        pages(cx),
        [
            "General",
            "Launcher",
            "Appearance",
            "Location",
            "Hotkeys",
            "Widgets",
            "About"
        ]
    );
    assert!(!has_label(cx, "No settings match"));
    assert!(has_label(cx, "Look up location by Wi-Fi"));
    assert!(!has_label(cx, "Start at login"));
}

/// Clearing a query that hid the chosen page keeps the page the search put on display.
#[gpui_kit::test]
fn clearing_the_search_keeps_the_page_it_showed(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::General));
    assert!(has_label(cx, "Start at login"));
    click(cx, "Search...");
    type_text(cx, "wifi");
    assert!(has_label(cx, "Look up location by Wi-Fi"));
    press(cx, "escape");
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
    assert!(has_label(cx, "Look up location by Wi-Fi"));
    assert!(!has_label(cx, "Start at login"));
}

#[gpui_kit::test]
fn a_catalog_change_keeps_the_query(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, None);
    click(cx, "Search...");
    type_text(cx, "wind");
    assert!(has_label(cx, "No settings match"));
    set_catalog(cx, catalog(vec![weather_not_running()]));
    assert_eq!(value_of(cx, "Search...").as_deref(), Some("wind"));
    assert_eq!(pages(cx), ["Weather"]);
    assert!(shows(cx, "Wind", "km/h"));
}

/// A page asked for from outside (the launcher's "open settings on …") shows even while a
/// search would keep nothing of it: the query is cleared.
#[gpui_kit::test]
fn opening_a_page_clears_the_query(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, None);
    click(cx, "Search...");
    type_text(cx, "wifi");
    assert_eq!(pages(cx), ["Location"]);
    open(cx, Some(SettingsPage::Home));
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
    assert!(has_label(
        cx,
        "Add widgets from the launcher: choose Add to Home on a result."
    ));
}

/// Esc in an empty search field changes nothing there and goes on to the window, as Esc does
/// without the field.
#[gpui_kit::test]
fn esc_on_an_empty_query_goes_on_to_the_window(cx: &mut TestAppContext) {
    use gpui_kit::component::input::Escape;
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Location));
    let reached = Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let reached = reached.clone();
        cx.on_action(move |_: &Escape, _| reached.set(reached.get() + 1));
    });
    click(cx, "Search...");
    press(cx, "escape");
    assert_eq!(reached.get(), 1, "Esc did not go on past the search field");
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
    assert_eq!(
        pages(cx),
        [
            "General",
            "Launcher",
            "Appearance",
            "Location",
            "Hotkeys",
            "Widgets",
            "About"
        ]
    );
    assert!(has_label(cx, "City"));

    // With a query, Esc clears it and stops there.
    type_text(cx, "wifi");
    press(cx, "escape");
    assert_eq!(reached.get(), 1);
    assert_eq!(value_of(cx, "Search...").as_deref(), Some(""));
}

// Appearance, About and the Config file row.

#[gpui_kit::test]
fn theme_select_writes_the_file(cx: &mut TestAppContext) {
    let s = setup(cx, "# mine\n[launcher]\nautostart = true\n");
    open(cx, Some(SettingsPage::Appearance));
    assert!(shows(cx, "Theme", "System"));
    click(cx, "Theme");
    click(cx, "Dark");
    assert_eq!(
        read(&s.config),
        "# mine\n[launcher]\nautostart = true\n\n[appearance]\ntheme = \"dark\"\n"
    );
    assert!(shows(cx, "Theme", "Dark"));
    assert!(has_label(cx, "Reset Theme"));
}

#[gpui_kit::test]
fn an_invalid_theme_is_shown_as_invalid(cx: &mut TestAppContext) {
    let s = setup(cx, "[appearance]\ntheme = \"blue\"\n");
    open(cx, Some(SettingsPage::Appearance));
    assert!(has_label(
        cx,
        "Not valid here: \"blue\"; the default is used"
    ));
    assert!(shows(cx, "Theme", "System"));
    assert_eq!(read(&s.config), "[appearance]\ntheme = \"blue\"\n");
}

#[gpui_kit::test]
fn theme_reset_removes_the_key(cx: &mut TestAppContext) {
    let s = setup(cx, "[appearance]\ntheme = \"light\"\n");
    open(cx, Some(SettingsPage::Appearance));
    assert!(shows(cx, "Theme", "Light"));
    click(cx, "Reset Theme");
    assert!(!read(&s.config).contains("theme"));
    assert!(shows(cx, "Theme", "System"));
    assert!(!has_label(cx, "Reset Theme"));
}

// The Launcher page.

/// A core whose history holds `queries`.
fn core_with_history(queries: &[&str]) -> Core {
    let mut core = new_core();
    for query in queries {
        assert!(core.record_hidden_query(query, LocalNow::now()));
    }
    core
}

#[gpui_kit::test]
fn pop_to_root_and_history_are_written(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Launcher));
    click(cx, "Return back home");
    click(cx, "After 3 minutes");
    assert_eq!(read(&s.config), "[launcher]\npop_to_root = \"3m\"\n");
    assert!(shows(cx, "Return back home", "After 3 minutes"));
    assert_eq!(checked(cx, "Remember search history"), Some(true));
    click(cx, "Remember search history");
    assert_eq!(
        read(&s.config),
        "[launcher]\npop_to_root = \"3m\"\nhistory = false\n"
    );
}

/// Clearing the history asks first; Cancel keeps it, Delete empties it, and the button is
/// then disabled.
#[gpui_kit::test]
fn clearing_the_history_asks_first(cx: &mut TestAppContext) {
    let s = setup_with(cx, "", core_with_history(&["firefox", "2+2", "weather"]));
    open(cx, Some(SettingsPage::Launcher));
    assert!(has_label(cx, "3 saved searches"));
    click(cx, "Clear search history");
    assert!(dialog_open(cx));
    assert!(has_label(cx, "Delete all 3 saved searches?"));
    assert!(has_label(cx, "This can't be undone."));
    click(cx, "Cancel");
    assert!(!dialog_open(cx));
    assert_eq!(s.core.borrow().history().len(), 3);

    click(cx, "Clear search history");
    // Enter deletes nothing.
    press(cx, "enter");
    assert_eq!(s.core.borrow().history().len(), 3);
    click(cx, "Delete");
    cx.run_until_parked();
    assert!(!dialog_open(cx));
    assert!(s.core.borrow().history().is_empty());
    assert!(has_label(cx, "No saved searches"));
    assert_eq!(disabled(cx, "Clear search history"), Some(true));
    assert_eq!(read(&s.config), "");
}

/// The history row works with a broken file: it is not the file's.
#[gpui_kit::test]
fn the_history_clears_while_the_file_is_broken(cx: &mut TestAppContext) {
    let s = setup_with(cx, "[launcher\n", core_with_history(&["firefox"]));
    open(cx, Some(SettingsPage::Launcher));
    assert!(has_label(cx, "1 saved search"));
    click(cx, "Clear search history");
    assert!(has_label(cx, "Delete the saved search?"));
    click(cx, "Delete");
    assert!(s.core.borrow().history().is_empty());
}

/// Font: unset, it is the desktop's UI font; the bundled Inter is one family of the list,
/// after the desktop's entry, and picking it writes its name; the desktop's entry removes the
/// key again.
#[gpui_kit::test]
fn the_font_is_picked_from_a_list(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Appearance));
    assert!(value_of(cx, "Font").unwrap().starts_with("System UI font"));
    click(cx, "Font");
    let all = labels(cx);
    let system = all
        .iter()
        .rposition(|label| label.starts_with("System UI font"))
        .expect("the desktop's entry");
    let inter = all
        .iter()
        .position(|label| label == "Inter")
        .expect("Inter");
    assert!(system < inter, "{all:?}");
    click(cx, "Inter");
    assert_eq!(read(&s.config), "[appearance]\nfont = \"Inter\"\n");
    assert_eq!(value_of(cx, "Font").as_deref(), Some("Inter"));
    click(cx, "Font");
    click_starting(cx, "System UI font");
    assert_eq!(read(&s.config), "");
    assert!(value_of(cx, "Font").unwrap().starts_with("System UI font"));
}

/// The monospace font is its own list, with its own default: the bundled JetBrains Mono, listed
/// among the families after the desktop's entry, which writes `system`.
#[gpui_kit::test]
fn the_monospace_font_is_its_own_setting(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Appearance));
    assert_eq!(
        value_of(cx, "Monospace font").as_deref(),
        Some("JetBrains Mono")
    );
    click(cx, "Monospace font");
    let all = labels(cx);
    let system = all
        .iter()
        .position(|label| label.starts_with("System monospace font"))
        .expect("the desktop's entry");
    let bundled = all
        .iter()
        .rposition(|label| label == "JetBrains Mono")
        .expect("JetBrains Mono");
    assert!(system < bundled, "{all:?}");
    click_starting(cx, "System monospace font");
    assert_eq!(read(&s.config), "[appearance]\nmono_font = \"system\"\n");
}

/// A file naming the default (`font = "system"`) stands on the default's entry.
#[gpui_kit::test]
fn the_default_font_by_name_is_the_default(cx: &mut TestAppContext) {
    setup(
        cx,
        "[appearance]\nfont = \"system\"\nmono_font = \"JetBrains Mono\"\n",
    );
    open(cx, Some(SettingsPage::Appearance));
    assert!(value_of(cx, "Font").unwrap().starts_with("System UI font"));
    assert_eq!(
        value_of(cx, "Monospace font").as_deref(),
        Some("JetBrains Mono")
    );
}

/// Text size is a choice of three; the monospace size a whole number in its range.
#[gpui_kit::test]
fn text_and_monospace_sizes_are_written(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::Appearance));
    click(cx, "Text size");
    click(cx, "Large");
    assert_eq!(read(&s.config), "[appearance]\ntext_size = \"large\"\n");
    assert!(shows(cx, "Text size", "Large"));
    click(cx, "Monospace size");
    press(cx, "end");
    press(cx, "backspace");
    press(cx, "backspace");
    type_text(cx, "40");
    press(cx, "enter");
    assert!(has_label(cx, "Enter a whole number from 8 to 32."));
    press(cx, "backspace");
    press(cx, "backspace");
    type_text(cx, "18");
    press(cx, "enter");
    assert_eq!(
        read(&s.config),
        "[appearance]\ntext_size = \"large\"\nmono_size = 18\n"
    );
}

/// The text preference reaches the kit's theme: the families, and the sizes scaled.
#[gpui_kit::test]
fn the_text_preference_sets_the_theme(cx: &mut TestAppContext) {
    setup(cx, "");
    open(cx, Some(SettingsPage::Appearance));
    let text = TextSettings {
        size: TextSize::Large,
        mono_size: 18,
        font: FontChoice::Family("Not An Installed Font".into()),
        ..TextSettings::default()
    };
    cx.update(|cx| tomakast_ui::set_text_preference(&text, cx));
    cx.run_until_parked();
    let (family, mono, size, mono_size) = cx.update(|cx| {
        let theme = cx.theme();
        (
            theme.font_family.to_string(),
            theme.mono_font_family.to_string(),
            theme.font_size,
            theme.mono_font_size,
        )
    });
    // A family that is not installed falls back to the bundled one.
    assert_eq!(family, "Inter");
    assert_eq!(mono, "JetBrains Mono");
    assert_eq!(size, gpui_kit::px(16. * 16. / 14.));
    assert_eq!(mono_size, gpui_kit::px(18.));
    // Back to Default.
    cx.update(|cx| tomakast_ui::set_text_preference(&TextSettings::default(), cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.theme().font_size), gpui_kit::px(16.));
}

#[gpui_kit::test]
fn about_shows_the_version(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::About));
    assert!(has_text(cx, "tomakast"));
    let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
    assert!(has_text(cx, &version), "{:?}", texts(cx));
}

type Opened = Arc<Mutex<Vec<String>>>;

struct FakeOpener {
    calls: Opened,
    fail: bool,
}

impl ConfigOpener for FakeOpener {
    fn open(&self, path: &Path) -> anyhow::Result<()> {
        self.record("open", path)
    }
    fn reveal(&self, path: &Path) -> anyhow::Result<()> {
        self.record("reveal", path)
    }
}

impl FakeOpener {
    fn record(&self, what: &str, path: &Path) -> anyhow::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{what} {}", path.display()));
        if self.fail {
            anyhow::bail!("nope");
        }
        Ok(())
    }
}

fn install_opener(cx: &mut TestAppContext, fail: bool) -> Opened {
    let calls = Opened::default();
    let opener = Arc::new(FakeOpener {
        calls: calls.clone(),
        fail,
    });
    cx.update(|cx| cx.update_global::<SettingsController, _>(|c, _| c.set_config_opener(opener)));
    calls
}

#[gpui_kit::test]
fn config_row_shows_the_path(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    open(cx, Some(SettingsPage::General));
    assert!(has_text(cx, "Config file"));
    assert!(has_label(cx, &s.config.display().to_string()));
    assert!(has_label(cx, "Open config file"));
    assert!(has_label(cx, "Show config file in folder"));
}

#[gpui_kit::test]
fn open_and_show_go_to_the_opener(cx: &mut TestAppContext) {
    let s = setup(cx, "x = 1\n");
    let calls = install_opener(cx, false);
    open(cx, Some(SettingsPage::General));
    click(cx, "Open config file");
    click(cx, "Show config file in folder");
    let path = s.config.display();
    assert_eq!(
        *calls.lock().unwrap(),
        [format!("open {path}"), format!("reveal {path}")]
    );
    assert_eq!(read(&s.config), "x = 1\n");
}

#[gpui_kit::test]
fn open_recreates_a_deleted_config_file(cx: &mut TestAppContext) {
    let s = setup(cx, "");
    let calls = install_opener(cx, false);
    open(cx, Some(SettingsPage::General));
    std::fs::remove_file(&s.config).unwrap();
    click(cx, "Open config file");
    assert_eq!(read(&s.config), "");
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(!dialog_open(cx));
    // The new file is the window's own: a change event for it asks nothing.
    config_changed(cx);
    assert!(!dialog_open(cx));
}

#[gpui_kit::test]
fn a_failed_open_is_said_under_the_row(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    install_opener(cx, true);
    open(cx, Some(SettingsPage::General));
    click(cx, "Open config file");
    assert!(has_label(cx, "Could not open: nope"));
}

/// The title area over the page shows the page's title; the window's own title, which task
/// bars and window lists show, stays the app's.
#[gpui_kit::test]
fn title_area_shows_the_page_title(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Location));
    let titles: Vec<String> = on_window(cx, |window, _| {
        window
            .within("settings-title")
            .find_all_by_role(gpui_kit::Role::Label)
            .iter()
            .filter_map(|e| e.value().map(str::to_string))
            .collect()
    });
    assert_eq!(titles, ["Location"]);
    let w = window(cx);
    let title = gpui_kit::VisualTestContext::from_window(w, cx).window_title();
    assert!(
        matches!(title.as_deref(), None | Some("Settings — tomakast")),
        "the window title changed to {title:?}"
    );
}

/// Where the window manager draws the frame (the test platform's system decorations), the
/// window draws no controls of its own.
#[gpui_kit::test]
fn no_window_controls_under_server_decorations(cx: &mut TestAppContext) {
    let _s = setup(cx, "");
    open(cx, Some(SettingsPage::Location));
    for control in ["Minimize", "Maximize", "Restore", "Close"] {
        assert!(!has_label(cx, control), "{control} is drawn");
    }
}

// Pick fields: time zones and choices of many options.

/// A plugin with a time zone and two languages of twelve options each, as calc and the
/// translator declare them.
fn pickers() -> CatalogPlugin {
    let languages: Vec<ChoiceOption> = [
        ("de", "German"),
        ("en", "English"),
        ("es", "Spanish"),
        ("fr", "French"),
        ("it", "Italian"),
        ("ja", "Japanese"),
        ("ko", "Korean"),
        ("nl", "Dutch"),
        ("pl", "Polish"),
        ("pt", "Portuguese"),
        ("ru", "Russian"),
        ("zh-CN", "Chinese (Simplified)"),
    ]
    .into_iter()
    .map(|(value, title)| ChoiceOption {
        value: value.into(),
        title: title.into(),
    })
    .collect();
    CatalogPlugin {
        preferences: vec![
            pref("timezone", "Time zone", None, ValueKind::TimeZone),
            pref(
                "first",
                "Primary language",
                None,
                ValueKind::Choice(languages.clone()),
            ),
            pref(
                "second",
                "Secondary language",
                None,
                ValueKind::Choice(languages),
            ),
        ],
        ..plugin("pickers", "Pickers")
    }
}

/// The settings over `text`, the computer in `zone`, on the page of [`pickers`] as `plugin`
/// gives it.
fn setup_pickers(cx: &mut TestAppContext, text: &str, plugin: CatalogPlugin) -> Setup {
    let s = setup(cx, text);
    cx.update(|cx| {
        cx.update_global::<SettingsController, _>(|c, _| {
            c.set_time_zone_source(Arc::new(|| Some("Asia/Tokyo".into())))
        })
    });
    set_catalog(cx, catalog(vec![plugin]));
    open(cx, Some(SettingsPage::Plugin("pickers".into())));
    s
}

/// The solid fill painted behind the control labelled `label` (the quad that has its bounds),
/// as its lightness.
fn fill_lightness(cx: &mut TestAppContext, label: &str) -> f32 {
    on_window(cx, |window, _| {
        let scale = window.scale_factor();
        let bounds = window.find_by_label(label).bounds().scale(scale);
        let near = |a: gpui_kit::ScaledPixels, b: gpui_kit::ScaledPixels| (a.0 - b.0).abs() < 1.5;
        window
            .painted_quads()
            .iter()
            .filter(|q| {
                near(q.bounds.left(), bounds.left())
                    && near(q.bounds.right(), bounds.right())
                    && near(q.bounds.top(), bounds.top())
                    && near(q.bounds.bottom(), bounds.bottom())
            })
            .filter_map(|q| q.background.as_solid())
            .filter(|c| c.a == 1.)
            .map(|c| c.l)
            .next()
            .expect("an opaque fill behind the control")
    })
}

fn move_pointer_to_center_of(cx: &mut TestAppContext, label: &str) {
    on_window(cx, |window, cx| {
        let position = window.find_by_label(label).bounds().center();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: gpui_kit::Modifiers::default(),
            }),
            cx,
        );
    });
    cx.run_until_parked();
}

/// A picker's trigger (time zone, a long choice) lightens on hover and again when pressed in the
/// dark theme, darkens in the light one, as the control buttons do.
#[gpui_kit::test]
fn a_picker_trigger_takes_a_hover_and_a_pressed_fill(cx: &mut TestAppContext) {
    let _s = setup_pickers(cx, "", pickers());
    for label in ["Time zone", "Primary language"] {
        let dark = cx.update(|cx| cx.theme().mode.is_dark());
        let rest = fill_lightness(cx, label);
        move_pointer_to_center_of(cx, label);
        let hover = fill_lightness(cx, label);
        on_window(cx, |window, cx| {
            let at = window.find_by_label(label).bounds().center();
            window.pointer_down(at, MouseButton::Left, cx);
        });
        cx.run_until_parked();
        let pressed = fill_lightness(cx, label);
        on_window(cx, |window, cx| {
            let at = window.find_by_label(label).bounds().center();
            window.pointer_up(at, MouseButton::Left, cx);
        });
        let sign = if dark { 1. } else { -1. };
        assert!(sign * (hover - rest) > 0., "{label}: {rest} {hover}");
        assert!(sign * (pressed - hover) > 0., "{label}: {hover} {pressed}");
        press(cx, "escape");
    }
}

/// A picker's pressed fill lasts only while the button is down: released on the trigger or
/// away from it, the trigger has its rest fill again once the pointer leaves.
#[gpui_kit::test]
fn a_picker_trigger_lets_go_of_its_pressed_fill(cx: &mut TestAppContext) {
    let _s = setup_pickers(cx, "", pickers());
    let label = "Time zone";
    let rest = fill_lightness(cx, label);
    let trigger = bounds_of(cx, label);
    let away = gpui_kit::point(trigger.left() - gpui_kit::px(200.), trigger.center().y);
    let move_to = |cx: &mut TestAppContext, position: Point| {
        on_window(cx, |window, cx| {
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: gpui_kit::Modifiers::default(),
                }),
                cx,
            );
        });
        cx.run_until_parked();
    };
    for release_at in [trigger.center(), away] {
        move_to(cx, trigger.center());
        let hover = fill_lightness(cx, label);
        on_window(cx, |window, cx| {
            window.pointer_down(trigger.center(), MouseButton::Left, cx);
        });
        cx.run_until_parked();
        let pressed = fill_lightness(cx, label);
        assert_ne!(pressed, hover, "pressed");
        on_window(cx, |window, cx| {
            window.pointer_move(release_at, Some(MouseButton::Left), cx);
            window.pointer_up(release_at, MouseButton::Left, cx);
        });
        cx.run_until_parked();
        press(cx, "escape");
        move_to(cx, away);
        assert_eq!(
            fill_lightness(cx, label),
            rest,
            "released at {release_at:?}"
        );
    }
}

#[gpui_kit::test]
fn a_long_choice_is_searchable(cx: &mut TestAppContext) {
    let s = setup_pickers(cx, "", pickers());
    // The list has a search field of its own (the sidebar's is "Search...").
    assert!(!has_label(cx, "Search…"));
    click(cx, "Primary language");
    assert!(has_label(cx, "Search…"));
    // Typing keeps the options whose title matches: Enter takes the first of them.
    type_text(cx, "Pol");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.pickers]\nfirst = \"pl\"\n");
    assert_eq!(value_of(cx, "Primary language").as_deref(), Some("Polish"));
}

#[gpui_kit::test]
fn an_unset_timezone_shows_the_computers_zone(cx: &mut TestAppContext) {
    let s = setup_pickers(cx, "", pickers());
    assert_eq!(value_of(cx, "Time zone").as_deref(), Some("Asia/Tokyo"));
    // Picking the zone shown writes nothing: it is in effect already.
    click(cx, "Time zone");
    type_text(cx, "Asia/Tokyo");
    press(cx, "enter");
    assert_eq!(read(&s.config), "");
    assert!(!has_label(cx, "Reset Time zone"));
    // Another zone is written, and Reset goes back to the computer's.
    click(cx, "Time zone");
    type_text(cx, "Europe/Moscow");
    press(cx, "enter");
    assert_eq!(
        read(&s.config),
        "[plugins.pickers]\ntimezone = \"Europe/Moscow\"\n"
    );
    assert_eq!(value_of(cx, "Time zone").as_deref(), Some("Europe/Moscow"));
    click(cx, "Reset Time zone");
    assert_eq!(read(&s.config), "");
    assert_eq!(value_of(cx, "Time zone").as_deref(), Some("Asia/Tokyo"));
}

#[gpui_kit::test]
fn a_zone_is_picked_with_the_mouse(cx: &mut TestAppContext) {
    let s = setup_pickers(cx, "", pickers());
    click(cx, "Time zone");
    type_text(cx, "Berlin");
    // The list's rows carry no accessible name, so the first one is found from the search
    // field above it: the rows start under the field's padded strip (and its 1px border).
    let search = bounds_of(cx, "Search time zones…");
    let strip = on_window(cx, |window, _| {
        window
            .elements()
            .into_iter()
            .map(|e| e.bounds())
            .filter(|b| b.contains(&search.center()) && b.size.width > search.size.width)
            .min_by(|a, b| a.size.width.partial_cmp(&b.size.width).unwrap())
            .expect("the search field sits in a strip of its own")
    });
    let first_row = gpui_kit::point(
        strip.center().x,
        strip.bottom() + gpui_kit::px(1.) + gpui_kit::px(12.),
    );
    on_window(cx, |window, cx| {
        window.pointer_down(first_row, MouseButton::Left, cx);
        window.pointer_up(first_row, MouseButton::Left, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        read(&s.config),
        "[plugins.pickers]\ntimezone = \"Europe/Berlin\"\n"
    );
    assert_eq!(value_of(cx, "Time zone").as_deref(), Some("Europe/Berlin"));
    assert!(!has_label(cx, "Search time zones…"));
}

#[gpui_kit::test]
fn a_hand_written_zone_shows_as_written(cx: &mut TestAppContext) {
    let _s = setup_pickers(cx, "[plugins.pickers]\ntimezone = \"Moscow\"\n", pickers());
    assert_eq!(value_of(cx, "Time zone").as_deref(), Some("Moscow"));
    assert!(has_label(cx, "Reset Time zone"));
}

#[gpui_kit::test]
fn an_unset_language_shows_the_plugins_default(cx: &mut TestAppContext) {
    let mut plugin = pickers();
    plugin.preferences[1].reported_default = Some("ru".into());
    plugin.preferences[2].reported_default = Some("en".into());
    let s = setup_pickers(cx, "", plugin.clone());
    assert_eq!(value_of(cx, "Primary language").as_deref(), Some("Russian"));
    assert_eq!(
        value_of(cx, "Secondary language").as_deref(),
        Some("English")
    );
    // Picking the default shown writes nothing.
    click(cx, "Primary language");
    type_text(cx, "Russian");
    press(cx, "enter");
    assert_eq!(read(&s.config), "");
    // A default the plugin reports later (a new catalog) shows at once.
    plugin.preferences[1].reported_default = Some("de".into());
    set_catalog(cx, catalog(vec![plugin]));
    assert_eq!(value_of(cx, "Primary language").as_deref(), Some("German"));
}

#[gpui_kit::test]
fn an_unknown_default_shows_automatic(cx: &mut TestAppContext) {
    let mut plugin = pickers();
    plugin.preferences[1].reported_default = Some("ru".into());
    let s = setup_pickers(cx, "", plugin);
    assert_eq!(
        value_of(cx, "Secondary language").as_deref(),
        Some("Automatic")
    );
    click(cx, "Secondary language");
    type_text(cx, "English");
    press(cx, "enter");
    assert_eq!(read(&s.config), "[plugins.pickers]\nsecond = \"en\"\n");
}

/// The launcher's side of a withdrawn default (the translator's Primary English on an
/// English-only system): the told English goes, and Secondary shows "Automatic".
#[gpui_kit::test]
fn a_withdrawn_default_shows_automatic(cx: &mut TestAppContext) {
    let mut plugin = pickers();
    plugin.preferences[1].reported_default = Some("de".into());
    plugin.preferences[2].reported_default = Some("en".into());
    let _s = setup_pickers(cx, "", plugin.clone());
    assert_eq!(
        value_of(cx, "Secondary language").as_deref(),
        Some("English")
    );
    plugin.preferences[1].reported_default = Some("en".into());
    plugin.preferences[2].reported_default = None;
    set_catalog(cx, catalog(vec![plugin]));
    assert_eq!(
        value_of(cx, "Secondary language").as_deref(),
        Some("Automatic")
    );
}

#[gpui_kit::test]
fn a_long_choice_of_a_plugin_that_tells_no_defaults_shows_not_set(cx: &mut TestAppContext) {
    let mut plugin = pickers();
    let _s = setup_pickers(cx, "", plugin.clone());
    assert_eq!(
        value_of(cx, "Secondary language").as_deref(),
        Some("Not set")
    );
    // Once the plugin tells a default of one, the other is its own choice too.
    plugin.preferences[1].reported_default = Some("ru".into());
    set_catalog(cx, catalog(vec![plugin]));
    assert_eq!(
        value_of(cx, "Secondary language").as_deref(),
        Some("Automatic")
    );
}
