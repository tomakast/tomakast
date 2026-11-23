//! Headless tests of the launcher window: typing, keyboard navigation, errors.
//!
//! Module calls run on real worker threads. No gpui task waits on them (a
//! wake-up from another thread breaks the test scheduler's determinism):
//! the test thread waits for each reply and hands it to the controller, as
//! the daemon's event loop does, then runs the executor until parked.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
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
use tomakast_core::{
    Core, Effect, FieldKind, FormField, FormView, GridTile, GridView, Icon, Item, ItemId, ItemKind,
    ListRow, ListView, LocalNow, RankWeights, SearchMode, View, ViewId,
};
use tomakast_ui::{LauncherController, LauncherView};

type Calls = Arc<Mutex<Vec<(ItemId, String)>>>;

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: Some(format!("{title} subtitle")),
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        source: None,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
            shortcut: None,
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

/// The labels of the highlighted rows of the root list.
fn highlighted_rows(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::ListItem)
            .iter()
            .filter(|row| row.selected() == Some(true))
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

fn exists(cx: &mut TestAppContext, id: &'static str) -> bool {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_busy_answer_shows_the_busy_line_over_the_root_list(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide).with_search(Vec::new());
    module.search_busy.store(true, Ordering::SeqCst);
    let launcher = setup_with(cx, module);
    assert!(!exists(cx, "view-busy"), "nothing asked yet");

    type_text(cx, "fi");
    // Replies to the searches for "" and "f" may come first; they are dropped.
    while !exists(cx, "view-busy") {
        deliver(cx, &launcher.core);
    }
}

#[gpui_kit::test]
fn an_answer_that_is_not_busy_shows_no_busy_line(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide).with_search(vec![item("answer", "= 42")]);
    let launcher = setup_with(cx, module);

    type_text(cx, "fi");
    while row_titles(cx).len() < 3 {
        deliver(cx, &launcher.core);
    }
    assert!(!exists(cx, "view-busy"));
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

/// With no strip, Up on the first row leaves the list for the field: no
/// row is highlighted, and Enter there still runs the first row.
#[gpui_kit::test]
fn up_on_the_first_row_reaches_the_field_where_enter_runs_the_first_row(cx: &mut TestAppContext) {
    let launcher = setup_with(cx, FakeModule::new(mixed_kinds(), hide));

    press(cx, "down");
    press(cx, "up");
    assert_eq!(highlighted_rows(cx), ["Beta"]);
    press(cx, "up");
    assert!(highlighted_rows(cx).is_empty(), "in the field");
    press(cx, "up");
    assert!(highlighted_rows(cx).is_empty());
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

// The view model of plan 2b-2a: form errors, action shortcuts, emoji and
// colour icons, and the item source label.

fn action(id: &str, title: &str, shortcut: Option<&str>) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: shortcut.map(str::to_string),
    }
}

fn label_of(cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .map(|e| e.label().unwrap_or_default().to_string())
    })
    .unwrap()
}

fn push_form() -> anyhow::Result<Vec<Effect>> {
    let field = |id: &str, error: Option<&str>| FormField {
        id: id.into(),
        label: id.into(),
        kind: FieldKind::Text {
            value: String::new(),
            placeholder: String::new(),
        },
        error: error.map(str::to_string),
    };
    Ok(vec![Effect::PushView(View::Form(FormView {
        id: ViewId("fake/form".into()),
        title: "Form".into(),
        fields: vec![
            field("name", Some("This name is taken")),
            field("other", None),
        ],
        submit_title: "Save".into(),
    }))])
}

#[gpui_kit::test]
fn form_field_error_is_rendered(cx: &mut TestAppContext) {
    let launcher = setup(cx, push_form);
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        label_of(cx, "field-error-name").as_deref(),
        Some("This name is taken")
    );
    assert_eq!(label_of(cx, "field-error-other"), None);
}

/// Rows whose second action has a shortcut; the third has an invalid one
/// (Ctrl+K is the action panel's).
fn push_shortcut_list() -> anyhow::Result<Vec<Effect>> {
    let row = |id: &str, title: &str| ListRow {
        id: id.into(),
        section: None,
        title: title.into(),
        subtitle: None,
        icon: None,
        accessories: Vec::new(),
        detail_markdown: None,
        actions: vec![
            action("pick", "Pick", None),
            action("copy", "Copy", Some("ctrl+shift+c")),
            action("bad", "Bad", Some("ctrl+k")),
        ],
    };
    Ok(vec![Effect::PushView(View::List(ListView {
        id: ViewId("fake/list".into()),
        title: "List".into(),
        placeholder: String::new(),
        search: SearchMode::CoreFilter,
        query: String::new(),
        revision: 0,
        rows: vec![row("alpha", "Alpha"), row("beta", "Beta")],
        show_detail: false,
        layout: tomakast_core::ListLayout::Rows,
        footer_action: None,
        busy: false,
    }))])
}

/// The chips of Ctrl+Shift+C, as the panel labels them.
fn copy_chips() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌃+⇧+C"
    } else {
        "Ctrl+Shift+C"
    }
}

fn chips_of(cx: &mut TestAppContext, ix: usize) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(("action-keys", ix))
            .map(|e| e.label().unwrap_or_default().to_string())
    })
    .unwrap()
}

#[gpui_kit::test]
fn action_shortcut_runs_action_and_shows_chips(cx: &mut TestAppContext) {
    let files = Item {
        actions: vec![
            action("run", "Run", None),
            action("copy", "Copy", Some("ctrl+shift+c")),
        ],
        ..item("files", "Files")
    };
    let module = FakeModule::new(vec![files], push_shortcut_list);
    let events = module.view_events.clone();
    let launcher = setup_with(cx, module);

    // On the root list, the selected item's action.
    press(cx, "ctrl-shift-c");
    deliver(cx, &launcher.core);
    assert_eq!(
        *launcher.calls.lock().unwrap(),
        [(ItemId::new("fake", "files"), "copy".to_string())]
    );
    assert_eq!(row_titles(cx), ["Alpha", "Beta"]);

    // In a pushed list, the selected row's, with the focus in the filter.
    press(cx, "down");
    press(cx, "ctrl-shift-c");
    deliver(cx, &launcher.core);
    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/list".into()),
            tomakast_core::ViewEvent::ActionChosen {
                row_id: "beta".into(),
                action_id: "copy".into()
            }
        )]
    );

    // Ctrl+K stays the panel's: the invalid shortcut is ignored.
    press(cx, "ctrl-k");
    assert!(label_of(cx, "action-panel").is_some(), "the panel opened");
    assert_eq!(chips_of(cx, 0), None);
    assert_eq!(chips_of(cx, 1).as_deref(), Some(copy_chips()));
    assert_eq!(chips_of(cx, 2), None, "no chips for an ignored shortcut");
    std::thread::sleep(Duration::from_millis(50));
    assert!(launcher.core.borrow().events().is_empty());
}

fn push_icon_grid() -> anyhow::Result<Vec<Effect>> {
    let tile = |id: &str, image: Icon| GridTile {
        id: id.into(),
        image: Some(image),
        caption: id.into(),
        actions: vec![action("pick", "Pick", None)],
    };
    Ok(vec![Effect::PushView(View::Grid(GridView {
        id: ViewId("fake/grid".into()),
        title: "Grid".into(),
        columns: 3,
        tiles: vec![
            tile("crab", Icon::Emoji("🦀".into())),
            tile("orange", Icon::Color("#ff8800".into())),
            tile("bad", Icon::Color("orange".into())),
        ],
    }))])
}

/// Labels and sizes of the rendered images, top to bottom.
fn images(cx: &mut TestAppContext) -> Vec<(String, f32)> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::Image)
            .iter()
            .map(|e| {
                let bounds = e.bounds();
                assert_eq!(bounds.size.width, bounds.size.height, "square");
                (
                    e.label().unwrap_or_default().to_string(),
                    f32::from(bounds.size.width),
                )
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn emoji_and_color_icons_render(cx: &mut TestAppContext) {
    let with_icon = |local: &str, title: &str, icon: Icon| Item {
        icon: Some(icon),
        ..item(local, title)
    };
    let module = FakeModule::new(
        vec![
            with_icon("crab", "Crab", Icon::Emoji("🦀".into())),
            with_icon("orange", "Orange", Icon::Color("#ff8800".into())),
            // Invalid: the monogram of the title instead.
            with_icon("plain", "Plain", Icon::Color("#12345".into())),
        ],
        push_icon_grid,
    );
    let launcher = setup_with(cx, module);

    assert_eq!(
        images(cx),
        [
            ("🦀".to_string(), 20.),
            ("#ff8800".to_string(), 20.),
            ("P".to_string(), 20.)
        ]
    );

    press(cx, "enter");
    deliver(cx, &launcher.core);
    assert_eq!(
        images(cx),
        [
            ("🦀".to_string(), 64.),
            ("#ff8800".to_string(), 64.),
            ("B".to_string(), 64.)
        ]
    );
}

/// A theme name draws the bundled Lucide glyph of that name; an unknown
/// name, a name that is not a plain Lucide file name, and no icon at all
/// draw a monogram: the first letter of the item's source (the plugin's
/// name) or else of its title.
#[gpui_kit::test]
fn named_icons_are_bundled_glyphs_else_monograms(cx: &mut TestAppContext) {
    let with_icon = |local: &str, title: &str, icon: Option<Icon>| Item {
        icon,
        ..item(local, title)
    };
    let from_plugin = Item {
        source: Some("Kitchen Sink".into()),
        ..item("hello", "Say hello")
    };
    let module = FakeModule::new(
        vec![
            with_icon("bell", "Alarm", Some(Icon::Named("bell".into()))),
            with_icon("nope", "Nope", Some(Icon::Named("no-such-icon".into()))),
            with_icon("dots", "Dots", Some(Icon::Named("../icons/bell".into()))),
            with_icon("upper", "Upper", Some(Icon::Named("Bell".into()))),
            // Format characters (Ruling M) and digits: the first alphanumeric.
            with_icon("zw", "\u{200b}éclair", None),
            from_plugin,
        ],
        hide,
    );
    setup_with(cx, module);

    let mut labels: Vec<String> = images(cx)
        .into_iter()
        .map(|(label, size)| {
            assert_eq!(size, 20.);
            label
        })
        .collect();
    labels.sort();
    assert_eq!(labels, ["D", "K", "N", "U", "bell", "É"]);
}

/// The text of the first accessory on root line `line`.
fn accessory(cx: &mut TestAppContext, line: usize) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .within(("hit", line))
            .try_find(("accessory", 0usize))
            .map(|e| e.label().unwrap_or_default().to_string())
    })
    .unwrap()
}

#[gpui_kit::test]
fn item_source_replaces_kind_label(cx: &mut TestAppContext) {
    let weather = Item {
        source: Some("Weather".into()),
        ..item("alpha", "Alpha")
    };
    setup_with(
        cx,
        FakeModule::new(vec![weather, item("beta", "Beta")], hide),
    );
    // Line 0 is the "Applications" label: the source does not change the section.
    assert_eq!(section_labels(cx), ["Applications"]);
    assert_eq!(accessory(cx, 1).as_deref(), Some("Weather"));
    assert_eq!(accessory(cx, 2).as_deref(), Some("Application"));
}

/// Items of a pinned provider (the `plugins` review module) lead the empty
/// query's Suggestions, above an item with usage.
#[gpui_kit::test]
fn pinned_item_leads_suggestions(cx: &mut TestAppContext) {
    let review = Item {
        id: ItemId::new("pinned", "review"),
        kind: ItemKind::Command,
        ..item("review", "Review plugin: Zed")
    };
    let mut core = new_core();
    core.register(Box::new(FakeModule::new(mixed_kinds(), hide)));
    core.register(Box::new(
        FakeModule::new(vec![review], hide).with_id("pinned"),
    ));
    core.pin_provider("pinned");
    core.start_modules();
    for _ in 0..2 {
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    let core = Rc::new(RefCell::new(core));
    install(cx, core.clone());
    show(cx);
    // Beta gets usage.
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &core);

    show(cx);

    assert_eq!(section_labels(cx), ["Suggestions", "Commands"]);
    assert_eq!(row_titles(cx), ["Review plugin: Zed", "Beta", "Alpha"]);
}

fn query_selection(cx: &mut TestAppContext) -> std::ops::Range<usize> {
    let w = window(cx);
    cx.update_window(w, |root, _, cx| {
        root.downcast::<LauncherView>()
            .unwrap()
            .read(cx)
            .query_selection(cx)
    })
    .unwrap()
}

fn root_selection(cx: &mut TestAppContext) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |root, _, cx| {
        root.downcast::<LauncherView>()
            .unwrap()
            .read(cx)
            .root_selection()
    })
    .unwrap()
}

fn record(launcher: &Launcher, query: &str, at: i64) {
    assert!(launcher.core.borrow().history().record(query, at));
}

fn nth_query(launcher: &Launcher, back: usize) -> Option<String> {
    launcher.core.borrow().history().nth_query(back)
}

fn entry_count(launcher: &Launcher) -> usize {
    launcher.core.borrow().history().entries().len()
}

/// A history of "files" then "firefox" (newest).
fn with_two_queries(cx: &mut TestAppContext) -> Launcher {
    let launcher = setup(cx, hide);
    record(&launcher, "files", 100);
    record(&launcher, "firefox", 200);
    launcher
}

fn ran(launcher: &Launcher) -> Vec<(ItemId, String)> {
    launcher.calls.lock().unwrap().clone()
}

/// Up climbs from the first row to the field (there is no strip here),
/// and Up in the empty field recalls the newest query, still in the field.
#[gpui_kit::test]
fn up_in_the_empty_field_recalls_the_newest_query(cx: &mut TestAppContext) {
    with_two_queries(cx);

    press(cx, "up");
    assert_eq!(query_value(cx), "", "the field, no recall yet");
    assert!(highlighted_rows(cx).is_empty());
    press(cx, "up");

    assert_eq!(query_value(cx), "firefox");
    assert_eq!(query_selection(cx), 0..7);
    assert_eq!(row_titles(cx), ["Firefox"]);
    assert!(highlighted_rows(cx).is_empty(), "still in the field");
    // What Enter runs.
    assert_eq!(root_selection(cx).as_deref(), Some("Firefox"));
}

#[gpui_kit::test]
fn up_again_steps_back_and_stops_at_the_oldest(cx: &mut TestAppContext) {
    with_two_queries(cx);

    press(cx, "up");
    press(cx, "up");
    press(cx, "up");
    assert_eq!(query_value(cx), "files");
    press(cx, "up");
    assert_eq!(query_value(cx), "files");
}

#[gpui_kit::test]
fn down_steps_forward_then_back_to_the_empty_field(cx: &mut TestAppContext) {
    with_two_queries(cx);

    press(cx, "up");
    press(cx, "up");
    press(cx, "up");
    press(cx, "down");
    assert_eq!(query_value(cx), "firefox");

    // Past the newest: the empty field, and the selection stays there.
    press(cx, "down");
    assert_eq!(query_value(cx), "");
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);

    // No strip: Down from the field is the list's first row.
    press(cx, "down");
    assert_eq!(highlighted_rows(cx), ["Files"]);
    press(cx, "down");
    assert_eq!(highlighted_rows(cx), ["Firefox"]);

    press(cx, "up");
    assert_eq!(highlighted_rows(cx), ["Files"]);
    press(cx, "up");
    assert_eq!(query_value(cx), "");
    assert!(highlighted_rows(cx).is_empty());
    press(cx, "up");
    assert_eq!(query_value(cx), "firefox");
}

#[gpui_kit::test]
fn typing_over_a_recalled_query_makes_a_new_query(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);
    record(&launcher, "firefox", 200);

    press(cx, "up");
    press(cx, "up");
    type_text(cx, "te");
    assert_eq!(query_value(cx), "te");
    assert_eq!(highlighted_rows(cx), ["Terminal"], "back on the first row");

    // Up reaches the field, where a typed query has no history.
    press(cx, "up");
    assert!(highlighted_rows(cx).is_empty());
    press(cx, "up");
    assert_eq!(query_value(cx), "te");

    press(cx, "enter");
    deliver(cx, &launcher.core);
    assert_eq!(
        ran(&launcher),
        [(ItemId::new("fake", "terminal"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn enter_on_a_recalled_query_runs_its_first_row(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);
    record(&launcher, "firefox", 200);

    press(cx, "up");
    press(cx, "up");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        ran(&launcher),
        [(ItemId::new("fake", "firefox"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn empty_query_still_selects_the_first_row(cx: &mut TestAppContext) {
    let launcher = with_two_queries(cx);
    assert_eq!(root_selection(cx).as_deref(), Some("Files"));

    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(
        ran(&launcher),
        [(ItemId::new("fake", "files"), "run".to_string())]
    );
}

#[gpui_kit::test]
fn up_with_no_history_stops_in_the_field(cx: &mut TestAppContext) {
    setup(cx, hide);

    press(cx, "up");
    press(cx, "up");

    assert_eq!(query_value(cx), "");
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(root_selection(cx).as_deref(), Some("Files"));
}

#[gpui_kit::test]
fn up_from_a_lower_row_moves_before_recalling(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);
    record(&launcher, "firefox", 200);

    press(cx, "down");
    assert_eq!(highlighted_rows(cx), ["Firefox"]);
    press(cx, "up");
    assert_eq!(query_value(cx), "");
    assert_eq!(highlighted_rows(cx), ["Files"]);
    press(cx, "up");
    assert_eq!(query_value(cx), "");
    assert!(highlighted_rows(cx).is_empty());
    press(cx, "up");
    assert_eq!(query_value(cx), "firefox");
}

#[gpui_kit::test]
fn editing_then_clearing_a_recalled_query_recalls_again(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);
    record(&launcher, "firefox", 200);

    press(cx, "up");
    press(cx, "up");
    type_text(cx, "x");
    press(cx, "backspace");
    assert_eq!(query_value(cx), "");
    assert_eq!(highlighted_rows(cx), ["Files"], "back on the first row");
    press(cx, "up");
    press(cx, "up");
    assert_eq!(query_value(cx), "firefox");
}

#[gpui_kit::test]
fn up_on_a_typed_query_moves_in_the_list(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);
    record(&launcher, "files", 100);

    type_text(cx, "f");
    press(cx, "down");
    press(cx, "up");
    assert_eq!(query_value(cx), "f");
    // Then the field, where Up does nothing; Enter runs the first row.
    press(cx, "up");
    press(cx, "up");
    assert_eq!(query_value(cx), "f");
    assert!(highlighted_rows(cx).is_empty());

    press(cx, "enter");
    deliver(cx, &launcher.core);
    assert_eq!(
        ran(&launcher),
        [(ItemId::new("fake", "files"), "run".to_string())]
    );
}

fn keeps_open() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

#[gpui_kit::test]
fn recall_survives_the_history_changing_underneath(cx: &mut TestAppContext) {
    let launcher = setup(cx, keeps_open);
    record(&launcher, "files", 1);
    record(&launcher, "firefox", 2);
    record(&launcher, "terminal", 3);

    press(cx, "up");
    press(cx, "up");
    press(cx, "up");
    assert_eq!(query_value(cx), "firefox");
    press(cx, "enter");
    deliver(cx, &launcher.core);
    assert_eq!(nth_query(&launcher, 0).as_deref(), Some("firefox"));
    assert_eq!(query_value(cx), "firefox");

    press(cx, "up");
    assert_eq!(query_value(cx), "files");
    press(cx, "up");
    assert_eq!(query_value(cx), "files");
}

#[gpui_kit::test]
fn hide_after_an_exempt_run_skips_its_query(cx: &mut TestAppContext) {
    let launcher = setup(cx, keeps_open);
    launcher.core.borrow_mut().exempt_from_history("fake");

    type_text(cx, "fi");
    press(cx, "enter");
    deliver(cx, &launcher.core);
    hide_launcher(cx);
    assert_eq!(entry_count(&launcher), 0);

    // The hook is live: a different query typed next is recorded.
    show(cx);
    type_text(cx, "term");
    hide_launcher(cx);
    assert_eq!(nth_query(&launcher, 0).as_deref(), Some("term"));

    // And the skip was one-shot: "fi" is recorded this time.
    show(cx);
    type_text(cx, "fi");
    hide_launcher(cx);
    assert_eq!(nth_query(&launcher, 0).as_deref(), Some("fi"));
}

#[gpui_kit::test]
fn hide_records_the_typed_query(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);

    type_text(cx, "fi");
    hide_launcher(cx);

    assert_eq!(nth_query(&launcher, 0).as_deref(), Some("fi"));
}

#[gpui_kit::test]
fn escape_records_the_query(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);

    type_text(cx, "term");
    press(cx, "escape");

    assert_eq!(nth_query(&launcher, 0).as_deref(), Some("term"));
}

#[gpui_kit::test]
fn hide_skips_a_one_character_query(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);

    type_text(cx, "f");
    hide_launcher(cx);

    assert_eq!(entry_count(&launcher), 0);
}

#[gpui_kit::test]
fn run_then_close_records_the_query_once(cx: &mut TestAppContext) {
    let launcher = setup(cx, hide);

    type_text(cx, "fir");
    press(cx, "enter");
    deliver(cx, &launcher.core);

    assert_eq!(entry_count(&launcher), 1);
    assert_eq!(nth_query(&launcher, 0).as_deref(), Some("fir"));
}
