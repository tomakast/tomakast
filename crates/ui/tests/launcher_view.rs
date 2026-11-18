//! Headless tests of the launcher window: typing, keyboard navigation, errors.
//!
//! Module calls run on real worker threads. No gpui task waits on them (a
//! wake-up from another thread breaks the test scheduler's determinism):
//! the test thread waits for each reply and hands it to the controller, as
//! the daemon's event loop does, then runs the executor until parked.

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
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{Core, Effect, Item, ItemId, ItemKind, LocalNow, RankWeights};
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

fn command(local: &str, title: &str) -> Item {
    Item {
        kind: ItemKind::Command,
        ..item(local, title)
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

fn hide() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CloseWindow])
}

fn gone() -> anyhow::Result<Vec<Effect>> {
    Err(anyhow::anyhow!("gone"))
}

/// The core the launcher runs on, and the calls its `FakeModule` received.
struct Launcher {
    core: Rc<RefCell<Core>>,
    calls: Calls,
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

/// Installs the UI as the daemon does, over `core`.
fn install(cx: &mut TestAppContext, core: Rc<RefCell<Core>>) {
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core));
    });
}

/// Initializes the UI over a core with `module`, once the module has
/// published its items, and shows the launcher.
fn setup_with(cx: &mut TestAppContext, module: FakeModule) -> Launcher {
    let calls = module.calls.clone();
    let mut core = new_core();
    core.register(Box::new(module));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    let core = Rc::new(RefCell::new(core));
    install(cx, core.clone());
    show(cx);
    Launcher { core, calls }
}

fn setup(cx: &mut TestAppContext, result: fn() -> anyhow::Result<Vec<Effect>>) -> Launcher {
    setup_with(cx, FakeModule::new(items(), result))
}

/// Waits for the next reply from a module worker and hands it to the UI.
fn deliver(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let ev = wait_event(&core.borrow().events());
    let update = core.borrow_mut().apply(ev, LocalNow::now());
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_update(update, cx)));
    cx.run_until_parked();
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

/// Accessible labels of the rendered section labels, top to bottom.
fn section_labels(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::Heading)
            .iter()
            .filter_map(|e| e.label().map(str::to_string))
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

fn error_toast(cx: &mut TestAppContext) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find("launcher-toast")
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
    let launcher = setup(cx, hide);

    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        *launcher.calls.lock().unwrap(),
        [(ItemId::new("fake", "firefox"), "run".to_string())]
    );
    assert!(!is_visible(cx));
    assert!(cx.update(|cx| cx.windows()).is_empty());
}

#[gpui_kit::test]
fn up_moves_selection_back(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);

    press(cx, "down");
    press(cx, "down");
    press(cx, "up");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        *launcher.calls.lock().unwrap(),
        [(ItemId::new("fake", "firefox"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn typing_resets_selection(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);

    press(cx, "down");
    type_text(cx, "f");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    // "f" matches Files and Firefox; the selection is back on the first row.
    assert_eq!(
        *launcher.calls.lock().unwrap(),
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
    let launcher = setup(cx, gone);
    assert_eq!(error_toast(cx), None);

    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(launcher.calls.lock().unwrap().len(), 1);
    assert!(is_visible(cx));
    let message = error_toast(cx).expect("an error toast");
    assert!(message.contains("gone"), "error toast: {message:?}");

    // The error toast stays for 5 s, then disappears.
    cx.executor().advance_clock(Duration::from_millis(4900));
    cx.run_until_parked();
    assert!(error_toast(cx).is_some());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(error_toast(cx), None);
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
fn items_changed_recomputes_results(cx: &mut TestAppContext) {
    let mut core = new_core();
    core.register(Box::new(FakeModule::new(items(), hide)));
    let core = Rc::new(RefCell::new(core));
    install(cx, core.clone());
    show(cx);
    assert!(row_titles(cx).is_empty());

    // Items arrive after the window is open.
    core.borrow_mut().start_modules();
    deliver(cx, &core);

    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn enter_does_not_wait_for_the_action(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide).with_delay(Duration::from_millis(500));
    let launcher = setup_with(cx, module);

    press(cx, "enter");

    // The module is still busy: the action has not run and the window is open.
    assert!(launcher.calls.lock().unwrap().is_empty());
    assert!(is_visible(cx));
    deliver(cx, &launcher.core);
    assert_eq!(launcher.calls.lock().unwrap().len(), 1);
    assert!(!is_visible(cx));
}

#[gpui_kit::test]
fn dynamic_answers_join_the_results(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide).with_search(vec![item("answer", "= 42")]);
    let launcher = setup_with(cx, module);

    type_text(cx, "fi");
    // Static matches are listed at once; the answer joins when its reply comes.
    assert_eq!(row_titles(cx), ["Files", "Firefox"]);
    // Replies to the searches for "" and "f" may come first; they are dropped.
    while row_titles(cx).len() < 3 {
        deliver(cx, &launcher.core);
    }

    assert_eq!(row_titles(cx), ["= 42", "Files", "Firefox"]);
}

#[gpui_kit::test]
fn reply_to_an_action_of_no_open_view_still_hides(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);
    // As a hotkey or `tomakast run` does: the request does not come from the view.
    launcher
        .core
        .borrow_mut()
        .request_action(&ItemId::new("fake", "files"), "run", "")
        .unwrap();

    deliver(cx, &launcher.core);

    assert!(!is_visible(cx));
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

/// Command "Alpha" sorts before application "Beta" by title, but the empty
/// query lists applications first.
fn mixed_kinds() -> Vec<Item> {
    vec![command("alpha", "Alpha"), item("beta", "Beta")]
}

#[gpui_kit::test]
fn empty_query_lists_kind_sections(cx: &mut TestAppContext) {
    setup_with(cx, FakeModule::new(mixed_kinds(), hide));

    assert_eq!(section_labels(cx), ["Applications", "Commands"]);
    assert_eq!(row_titles(cx), ["Beta", "Alpha"]);
}

#[gpui_kit::test]
fn typed_query_lists_one_results_section(cx: &mut TestAppContext) {
    setup_with(cx, FakeModule::new(mixed_kinds(), hide));

    type_text(cx, "a");

    assert_eq!(section_labels(cx), ["Results"]);
}

#[gpui_kit::test]
fn selection_starts_on_the_first_listed_row(cx: &mut TestAppContext) {
    let launcher = setup_with(cx, FakeModule::new(mixed_kinds(), hide));

    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        *launcher.calls.lock().unwrap(),
        [(ItemId::new("fake", "beta"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn arrows_follow_the_listed_order_and_skip_labels(cx: &mut TestAppContext) {
    let launcher = setup_with(cx, FakeModule::new(mixed_kinds(), hide));

    // Beta, over the "Commands" label, to Alpha; Down stays on the last row.
    press(cx, "down");
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        *launcher.calls.lock().unwrap(),
        [(ItemId::new("fake", "alpha"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn up_on_the_first_row_stays(cx: &mut TestAppContext) {
    let launcher = setup_with(cx, FakeModule::new(mixed_kinds(), hide));

    press(cx, "down");
    press(cx, "up");
    press(cx, "up");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        *launcher.calls.lock().unwrap(),
        [(ItemId::new("fake", "beta"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn used_item_is_suggested(cx: &mut TestAppContext) {
    let launcher = setup_with(cx, FakeModule::new(mixed_kinds(), hide));
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    show(cx);

    assert_eq!(section_labels(cx), ["Suggestions", "Applications"]);
    assert_eq!(row_titles(cx), ["Alpha", "Beta"]);
}
