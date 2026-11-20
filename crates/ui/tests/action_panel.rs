//! Headless tests of the action panel (Ctrl+K) and of the root selection
//! after a new query.
//!
//! As in `launcher_view.rs`, module calls run on real worker threads and the
//! test thread hands each reply to the controller, as the daemon does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, Role, TestAppContext,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, Effect, Item, ItemId, ItemKind, ListRow, ListView, LocalNow, RankWeights, SearchMode,
    View, ViewEvent, ViewId,
};
use tomakast_ui::LauncherController;

/// Ctrl+K, or Cmd+K on macOS.
const TOGGLE: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        source: None,
        actions: vec![
            action("open", "Open"),
            action("reveal", "Reveal in Files"),
            action("copy", "Copy Path"),
        ],
    }
}

/// With an empty query: Files, Firefox, Terminal.
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

fn row(id: &str, title: &str) -> ListRow {
    ListRow {
        id: id.to_string(),
        section: None,
        title: title.to_string(),
        subtitle: None,
        icon: None,
        accessories: Vec::new(),
        detail_markdown: None,
        actions: vec![action("pick", "Pick"), action("drop", "Drop")],
    }
}

fn push_sub() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(ListView {
        id: ViewId("fake/sub".into()),
        title: "Sub".into(),
        placeholder: "Filter…".into(),
        search: SearchMode::CoreFilter,
        query: String::new(),
        revision: 0,
        rows: vec![row("alpha", "Alpha"), row("beta", "Beta")],
        show_detail: false,
    }))])
}

fn nothing(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

fn setup(cx: &mut TestAppContext, module: FakeModule) -> Rc<RefCell<Core>> {
    let mut core = new_core();
    core.register(Box::new(module));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    let core = Rc::new(RefCell::new(core));
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core.clone()));
    });
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show(cx)));
    cx.run_until_parked();
    core
}

/// Waits for the next reply from a module worker and hands it to the UI.
fn deliver(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let ev = wait_event(&core.borrow().events());
    let update = core.borrow_mut().apply(ev, LocalNow::now());
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_update(update, cx)));
    cx.run_until_parked();
}

fn is_visible(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<LauncherController>().is_visible())
}

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

fn labels_by_role(cx: &mut TestAppContext, role: Role) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(role)
            .iter()
            .filter_map(|e| e.label().map(str::to_string))
            .collect()
    })
    .unwrap()
}

fn row_titles(cx: &mut TestAppContext) -> Vec<String> {
    labels_by_role(cx, Role::ListItem)
}

/// Titles of the actions in the open panel; empty when there is none.
fn panel_titles(cx: &mut TestAppContext) -> Vec<String> {
    labels_by_role(cx, Role::MenuItem)
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

fn panel_open(cx: &mut TestAppContext) -> bool {
    label_of(cx, "action-panel").is_some()
}

/// Rows `Files`, `Firefox`, `Terminal` have three actions each.
#[gpui_kit::test]
fn action_panel_lists_item_actions(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), hide));

    press(cx, TOGGLE);

    assert!(panel_open(cx));
    assert_eq!(
        panel_titles(cx),
        ["Open", "Reveal in Files", "Copy Path"],
        "the selected row's actions"
    );
}

/// Review Focus 4.
#[gpui_kit::test]
fn action_panel_runs_secondary_action(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide);
    let calls: Arc<Mutex<Vec<(ItemId, String)>>> = module.calls.clone();
    let core = setup(cx, module);

    press(cx, TOGGLE);
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &core);

    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("fake", "files"), "reveal".to_string())]
    );
    assert!(
        !is_visible(cx),
        "the module's CloseWindow hides the launcher"
    );
}

#[gpui_kit::test]
fn action_panel_filters(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), hide));

    press(cx, TOGGLE);
    type_text(cx, "cop");

    assert_eq!(panel_titles(cx), ["Copy Path"]);
    // The text went to the panel's filter, not to the root search.
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn escape_closes_panel_not_view(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(nothing);
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(label_of(cx, "view-header").as_deref(), Some("Sub"));

    press(cx, TOGGLE);
    assert!(panel_open(cx));
    press(cx, "escape");

    assert!(!panel_open(cx));
    assert!(is_visible(cx));
    assert_eq!(
        label_of(cx, "view-header").as_deref(),
        Some("Sub"),
        "the view is still on top"
    );
    assert_eq!(row_titles(cx), ["Alpha", "Beta"]);

    // The focus is back on the view's filter field: typing filters it.
    type_text(cx, "be");
    assert_eq!(row_titles(cx), ["Beta"]);
}

#[gpui_kit::test]
fn escape_closes_panel_on_root_then_hides(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), hide));

    press(cx, TOGGLE);
    press(cx, "escape");
    assert!(!panel_open(cx));
    assert!(is_visible(cx));

    // Focus is back on the search field.
    type_text(cx, "fir");
    assert_eq!(row_titles(cx), ["Firefox"]);
    press(cx, "escape");
    assert!(!is_visible(cx));
}

/// Review Focus 4.
#[gpui_kit::test]
fn action_panel_without_selection_is_noop(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), hide));
    type_text(cx, "zzzz");
    assert!(row_titles(cx).is_empty());

    press(cx, TOGGLE);

    assert!(!panel_open(cx));
    // Typing still goes to the search field.
    type_text(cx, "x");
    assert!(row_titles(cx).is_empty());
}

#[gpui_kit::test]
fn toggle_closes_the_panel(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), hide));

    press(cx, TOGGLE);
    assert!(panel_open(cx));
    press(cx, TOGGLE);

    assert!(!panel_open(cx));
}

#[gpui_kit::test]
fn pushed_view_panel_sends_action_chosen(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);

    press(cx, "down");
    press(cx, TOGGLE);
    assert_eq!(panel_titles(cx), ["Pick", "Drop"]);
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &core);

    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/sub".into()),
            ViewEvent::ActionChosen {
                row_id: "beta".into(),
                action_id: "drop".into()
            }
        )]
    );
    assert!(!panel_open(cx), "running an action closes the panel");
}

#[gpui_kit::test]
fn footer_shows_primary_action_and_shortcut(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), hide));

    assert_eq!(label_of(cx, "footer-primary").as_deref(), Some("⏎ Open"));
    let shortcut = if cfg!(target_os = "macos") {
        "Cmd+K Actions"
    } else {
        "Ctrl+K Actions"
    };
    assert_eq!(label_of(cx, "footer-actions").as_deref(), Some(shortcut));
}

/// Enter while an action of the same provider is pending is ignored, in the
/// panel too: the action runs once.
#[gpui_kit::test]
fn panel_enter_respects_the_pending_action_gate(cx: &mut TestAppContext) {
    fn nothing_happens() -> anyhow::Result<Vec<Effect>> {
        Ok(vec![])
    }
    let module = FakeModule::new(items(), nothing_happens);
    let calls: Arc<Mutex<Vec<(ItemId, String)>>> = module.calls.clone();
    let core = setup(cx, module);

    press(cx, "enter"); // "open" is pending: its reply is not delivered yet
    press(cx, TOGGLE);
    press(cx, "down");
    press(cx, "enter");

    assert!(panel_open(cx), "a gated Enter leaves the panel open");
    deliver(cx, &core);
    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("fake", "files"), "open".to_string())]
    );
}

/// Controller ruling: after a new root query the selection goes to the first
/// row that is not a held answer to the previous query, so a quick Enter
/// runs the first real match.
#[gpui_kit::test]
fn new_query_selects_the_first_non_held_row(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide).with_search(vec![item("answer", "= 42")]);
    let calls: Arc<Mutex<Vec<(ItemId, String)>>> = module.calls.clone();
    let core = setup(cx, module);
    type_text(cx, "f");
    // Replies to the searches for "" and "f" may come first.
    while row_titles(cx).len() < 3 {
        deliver(cx, &core);
    }
    assert_eq!(row_titles(cx), ["= 42", "Files", "Firefox"]);

    // The answer is held while "fi" is searched, still above the real matches.
    type_text(cx, "i");
    assert_eq!(row_titles(cx), ["= 42", "Files", "Firefox"]);
    press(cx, "enter");

    while calls.lock().unwrap().is_empty() {
        deliver(cx, &core);
    }
    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("fake", "files"), "open".to_string())]
    );
}
