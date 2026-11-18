//! Headless tests of the launcher window: typing, keyboard navigation, errors.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, Role, TestAppContext,
    VisualTestContext, WindowAppearance,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::FakeModule;
use tomakast_core::{ActionOutcome, Core, Item, ItemId, ItemKind, LocalNow, RankWeights};
use tomakast_ui::LauncherController;

type Calls = Arc<Mutex<Vec<(ItemId, String)>>>;

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: Some(format!("{title} subtitle")),
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
        }],
    }
}

/// With an empty query and no usage the list is sorted by title:
/// Files, Firefox, Terminal.
fn items() -> Vec<Item> {
    vec![
        item("firefox", "Firefox"),
        item("files", "Files"),
        item("terminal", "Terminal"),
    ]
}

fn hide() -> anyhow::Result<ActionOutcome> {
    Ok(ActionOutcome::HideWindow)
}

fn gone() -> anyhow::Result<ActionOutcome> {
    Err(anyhow::anyhow!("gone"))
}

/// Initializes the UI as the daemon does, installs the controller over a core
/// with a `FakeModule`, and shows the launcher.
fn setup(cx: &mut TestAppContext, result: fn() -> anyhow::Result<ActionOutcome>) -> Calls {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap();
    let module = FakeModule::new(items(), result);
    let calls = module.calls.clone();
    core.register(Box::new(module));
    core.start_modules();
    let rx = core.events();
    while let Ok(ev) = rx.try_recv() {
        core.apply(ev);
    }
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(Rc::new(RefCell::new(core))));
    });
    show(cx);
    calls
}

fn show(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show(cx)));
    cx.run_until_parked();
}

fn hide_launcher(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.hide(cx)));
    cx.run_until_parked();
}

fn is_visible(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<LauncherController>().is_visible())
}

/// The launcher is the only window the tests open.
fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "expected exactly the launcher window");
    windows[0]
}

fn type_text(cx: &mut TestAppContext, text: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, key: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Accessible labels of the rendered rows, top to bottom.
fn row_titles(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::ListItem)
            .iter()
            .filter_map(|row| row.label().map(str::to_string))
            .collect()
    })
    .unwrap()
}

fn query_value(cx: &mut TestAppContext) -> String {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find("launcher-input")
            .value()
            .unwrap_or_default()
            .to_string()
    })
    .unwrap()
}

fn error_line(cx: &mut TestAppContext) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find("launcher-error")
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

#[gpui_kit::test]
fn typing_filters_results(cx: &mut TestAppContext) {
    setup(cx, hide);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);

    type_text(cx, "fir");

    assert_eq!(query_value(cx), "fir");
    assert_eq!(row_titles(cx), ["Firefox"]);
}

#[gpui_kit::test]
fn enter_executes_selected_and_hides(cx: &mut TestAppContext) {
    let calls = setup(cx, hide);

    press(cx, "down");
    press(cx, "enter");

    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("fake", "firefox"), "run".to_string())]
    );
    assert!(!is_visible(cx));
    assert!(cx.update(|cx| cx.windows()).is_empty());
}

#[gpui_kit::test]
fn up_moves_selection_back(cx: &mut TestAppContext) {
    let calls = setup(cx, hide);

    press(cx, "down");
    press(cx, "down");
    press(cx, "up");
    press(cx, "enter");

    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("fake", "firefox"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn typing_resets_selection(cx: &mut TestAppContext) {
    let calls = setup(cx, hide);

    press(cx, "down");
    type_text(cx, "f");
    press(cx, "enter");

    // "f" matches Files and Firefox; the selection is back on the first row.
    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("fake", "files"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn escape_hides(cx: &mut TestAppContext) {
    setup(cx, hide);
    assert!(is_visible(cx));

    press(cx, "escape");

    assert!(!is_visible(cx));
    assert!(cx.update(|cx| cx.windows()).is_empty());
}

#[gpui_kit::test]
fn deactivation_hides(cx: &mut TestAppContext) {
    setup(cx, hide);
    let w = window(cx);
    // The test platform does not activate new windows; do what the compositor
    // does when the window gets the keyboard.
    cx.update_window(w, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert!(is_visible(cx));

    VisualTestContext::from_window(w, cx).deactivate_window();
    cx.run_until_parked();

    assert!(!is_visible(cx));
    assert!(cx.update(|cx| cx.windows()).is_empty());
}

#[gpui_kit::test]
fn error_keeps_window_and_shows_message(cx: &mut TestAppContext) {
    let calls = setup(cx, gone);
    assert_eq!(error_line(cx), None);

    press(cx, "enter");

    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(is_visible(cx));
    let message = error_line(cx).expect("an error line");
    assert!(message.contains("gone"), "error line: {message:?}");

    // The message stays for 4 s, then disappears.
    cx.executor().advance_clock(Duration::from_millis(3900));
    cx.run_until_parked();
    assert!(error_line(cx).is_some());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(error_line(cx), None);
}

#[gpui_kit::test]
fn show_resets_query(cx: &mut TestAppContext) {
    setup(cx, hide);
    type_text(cx, "x");
    assert_eq!(query_value(cx), "x");

    hide_launcher(cx);
    assert!(!is_visible(cx));
    show(cx);

    assert!(is_visible(cx));
    assert_eq!(query_value(cx), "");
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn toggle_shows_and_hides(cx: &mut TestAppContext) {
    setup(cx, hide);
    let toggle = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.toggle(cx)));
        cx.run_until_parked();
    };

    toggle(cx);
    assert!(!is_visible(cx));
    toggle(cx);
    assert!(is_visible(cx));
}

/// The daemon passes the receipt time of a hotkey or IPC request; the
/// `_at` variants must behave like `show`/`toggle` otherwise. (The headless
/// platform never runs `on_next_frame` callbacks, so the logged latency
/// value itself is not observable here.)
#[gpui_kit::test]
fn toggle_at_and_show_at_behave_like_toggle_and_show(cx: &mut TestAppContext) {
    setup(cx, hide);
    let received = Instant::now() - Duration::from_secs(1);
    let toggle_at = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            cx.update_global::<LauncherController, _>(|c, cx| c.toggle_at(received, cx))
        });
        cx.run_until_parked();
    };

    toggle_at(cx);
    assert!(!is_visible(cx));
    toggle_at(cx);
    assert!(is_visible(cx));
    assert_eq!(query_value(cx), "");
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);

    // No-op while open, like `show`.
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show_at(received, cx)));
    cx.run_until_parked();
    assert!(is_visible(cx));
    window(cx);
}

#[gpui_kit::test]
fn core_changed_recomputes_results(cx: &mut TestAppContext) {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap();
    core.register(Box::new(FakeModule::new(items(), hide)));
    let core = Rc::new(RefCell::new(core));
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core.clone()));
    });
    show(cx);
    assert!(row_titles(cx).is_empty());

    // Items arrive after the window is open.
    core.borrow_mut().start_modules();
    let rx = core.borrow().events();
    while let Ok(ev) = rx.try_recv() {
        core.borrow_mut().apply(ev);
    }
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_changed(cx)));
    cx.run_until_parked();

    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn prewarm_leaves_no_window(cx: &mut TestAppContext) {
    setup(cx, hide);
    hide_launcher(cx);

    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.prewarm(cx)));
    cx.run_until_parked();

    assert!(!is_visible(cx));
    assert!(cx.update(|cx| cx.windows()).is_empty());
    show(cx);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn theme_follows_window_appearance(cx: &mut TestAppContext) {
    setup(cx, hide);
    hide_launcher(cx);
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    assert!(cx.update(|cx| cx.theme().mode.is_dark()));

    show(cx);

    // The test platform's windows report a light appearance.
    let w = window(cx);
    let appearance = cx
        .update_window(w, |_, window, _| window.appearance())
        .unwrap();
    assert_eq!(appearance, WindowAppearance::Light);
    assert!(!cx.update(|cx| cx.theme().mode.is_dark()));
}

#[gpui_kit::test]
fn window_closed_elsewhere_resets_controller(cx: &mut TestAppContext) {
    setup(cx, hide);
    let w = window(cx);

    // The compositor or the user closes the window without going through `hide`.
    cx.update_window(w, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    assert!(cx.update(|cx| cx.windows()).is_empty());
    assert!(!is_visible(cx));
    show(cx);
    assert!(is_visible(cx));
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}
