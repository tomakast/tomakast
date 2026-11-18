//! Headless tests of the view stack: pushed list views, effects and toasts.
//!
//! As in `launcher_view.rs`, module calls run on real worker threads and the
//! test thread hands each reply to the controller, as the daemon does.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, Bounds, Pixels, QuitMode, Role,
    TestAppContext,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreUpdate, Effect, Item, ItemId, ItemKind, ListRow, ListView, LocalNow, RankWeights,
    SearchMode, Toast, ToastKind, View, ViewEvent, ViewId,
};
use tomakast_ui::LauncherController;

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Command,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
        }],
    }
}

/// Sorted by title with an empty query: Files, Firefox, Terminal.
fn items() -> Vec<Item> {
    vec![
        item("firefox", "Firefox"),
        item("files", "Files"),
        item("terminal", "Terminal"),
    ]
}

fn row(id: &str, title: &str, section: Option<&str>) -> ListRow {
    ListRow {
        id: id.to_string(),
        section: section.map(str::to_string),
        title: title.to_string(),
        subtitle: None,
        icon: None,
        accessories: Vec::new(),
        detail_markdown: Some(format!("About {title}")),
        actions: vec![Action {
            id: "pick".into(),
            title: "Pick".into(),
        }],
    }
}

fn list(title: &str, search: SearchMode, rows: Vec<ListRow>) -> ListView {
    ListView {
        id: ViewId("fake/sub".into()),
        title: title.to_string(),
        placeholder: "Filter…".into(),
        search,
        query: String::new(),
        revision: 0,
        rows,
        show_detail: false,
    }
}

fn sub_rows() -> Vec<ListRow> {
    vec![
        row("alpha", "Alpha", None),
        row("beta", "Beta", None),
        row("gamma", "Gamma", None),
    ]
}

fn push_sub() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(list(
        "Sub",
        SearchMode::CoreFilter,
        sub_rows(),
    )))])
}

fn push_module_list() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(list(
        "Search",
        SearchMode::Module,
        sub_rows(),
    )))])
}

fn push_sections() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(list(
        "Sections",
        SearchMode::CoreFilter,
        vec![
            row("a1", "One", Some("A")),
            row("a2", "Two", Some("A")),
            row("b1", "Three", Some("B")),
        ],
    )))])
}

fn push_detail_list() -> anyhow::Result<Vec<Effect>> {
    let mut view = list("Details", SearchMode::CoreFilter, sub_rows());
    view.show_detail = true;
    Ok(vec![Effect::PushView(View::List(view))])
}

fn push_detail_list_without_detail() -> anyhow::Result<Vec<Effect>> {
    let mut rows = sub_rows();
    rows[0].detail_markdown = None;
    rows[1].detail_markdown = Some(String::new());
    rows[2].detail_markdown = Some("[a]: http://x".into());
    let mut view = list("Details", SearchMode::CoreFilter, rows);
    view.show_detail = true;
    Ok(vec![Effect::PushView(View::List(view))])
}

fn hide() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CloseWindow])
}

fn nothing() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

fn success_toast() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::Toast(Toast {
        kind: ToastKind::Success,
        title: "Saved".into(),
        message: None,
    })])
}

fn replace_with_gamma() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::ReplaceItems(vec![item("gamma", "Gamma")])])
}

fn pop() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PopView])
}

fn copy_x() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CopyText("x".into())])
}

/// Answers each query of a `Module` list with one row named after the text,
/// tagged with the query's revision.
fn echo_query(view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::QueryChanged { revision, text } = event else {
        return Ok(vec![]);
    };
    let mut reply = list(
        "Search",
        SearchMode::Module,
        vec![row("r", &format!("{text} result"), None)],
    );
    reply.id = view.clone();
    reply.revision = revision;
    reply.query = text;
    Ok(vec![Effect::UpdateView(View::List(reply))])
}

/// Like [`echo_query`], with a toast naming the text.
fn echo_with_toast(view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let text = match &event {
        ViewEvent::QueryChanged { text, .. } => text.clone(),
        _ => String::new(),
    };
    let mut effects = echo_query(view, event)?;
    effects.push(Effect::Toast(Toast {
        kind: ToastKind::Success,
        title: format!("{text} done"),
        message: None,
    }));
    Ok(effects)
}

/// `ActionChosen` pops the view.
fn pop_on_action(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    match event {
        ViewEvent::ActionChosen { .. } => Ok(vec![Effect::PopView]),
        _ => Ok(vec![]),
    }
}

/// Answers every query with a toast and a pushed view.
fn noisy_reply(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::QueryChanged { .. } = event else {
        return Ok(vec![]);
    };
    let mut next = list("Next", SearchMode::CoreFilter, sub_rows());
    next.id = ViewId("fake/next".into());
    Ok(vec![
        Effect::Toast(Toast {
            kind: ToastKind::Success,
            title: "late".into(),
            message: None,
        }),
        Effect::PushView(View::List(next)),
    ])
}

/// Inserts a row above the others on every query.
fn insert_on_query(view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::QueryChanged { revision, .. } = event else {
        return Ok(vec![]);
    };
    let mut rows = vec![row("new", "New", None)];
    rows.extend(sub_rows());
    let mut reply = list("Search", SearchMode::Module, rows);
    reply.id = view.clone();
    reply.revision = revision;
    Ok(vec![Effect::UpdateView(View::List(reply))])
}

fn fail_on_action(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    match event {
        ViewEvent::ActionChosen { .. } => Err(anyhow::anyhow!("broken")),
        _ => Ok(vec![]),
    }
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

/// Initializes the UI over a core with `module` once it has published its
/// items, and shows the launcher.
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
    show(cx);
    core
}

/// Like [`setup`], over several modules.
fn setup_modules(cx: &mut TestAppContext, modules: Vec<FakeModule>) -> Rc<RefCell<Core>> {
    let mut core = new_core();
    let count = modules.len();
    for module in modules {
        core.register(Box::new(module));
    }
    core.start_modules();
    for _ in 0..count {
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    let core = Rc::new(RefCell::new(core));
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core.clone()));
    });
    show(cx);
    core
}

/// The next reply of a module worker, applied to the core.
fn next_update(core: &Rc<RefCell<Core>>) -> CoreUpdate {
    let ev = wait_event(&core.borrow().events());
    core.borrow_mut().apply(ev, LocalNow::now())
}

fn hand(cx: &mut TestAppContext, update: CoreUpdate) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_update(update, cx)));
    cx.run_until_parked();
}

/// Waits for the next reply of a module worker and hands it to the UI.
fn deliver(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let update = next_update(core);
    hand(cx, update);
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

/// Accessible labels of the rendered rows of the visible view, top to bottom.
fn row_titles(cx: &mut TestAppContext) -> Vec<String> {
    labels_by_role(cx, Role::ListItem)
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

fn value_of(cx: &mut TestAppContext, id: &'static str) -> String {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).value().unwrap_or_default().to_string()
    })
    .unwrap()
}

fn header(cx: &mut TestAppContext) -> Option<String> {
    label_of(cx, "view-header")
}

fn toast(cx: &mut TestAppContext) -> Option<String> {
    label_of(cx, "launcher-toast")
}

/// Shows the launcher over `module` and opens its sub-view with Enter on Files.
fn pushed(cx: &mut TestAppContext, module: FakeModule) -> Rc<RefCell<Core>> {
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    core
}

#[gpui_kit::test]
fn push_view_then_escape_pops(cx: &mut TestAppContext) {
    pushed(cx, FakeModule::new(items(), push_sub));
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"]);
    assert!(label_of(cx, "view-back").is_some(), "a back affordance");

    press(cx, "escape");
    assert!(is_visible(cx));
    assert_eq!(header(cx), None);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);

    press(cx, "escape");
    assert!(!is_visible(cx));
}

/// Review Focus 3.
#[gpui_kit::test]
fn escape_pops_then_hides(cx: &mut TestAppContext) {
    pushed(cx, FakeModule::new(items(), push_sub));
    type_text(cx, "be");
    assert_eq!(row_titles(cx), ["Beta"]);

    press(cx, "escape");
    press(cx, "escape");
    assert!(!is_visible(cx));

    show(cx);
    assert!(is_visible(cx));
    assert_eq!(header(cx), None);
    assert_eq!(value_of(cx, "launcher-input"), "");
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    outer.contains(&inner.origin) && outer.contains(&inner.bottom_right())
}

fn bounds_of(cx: &mut TestAppContext, id: &'static str) -> Bounds<Pixels> {
    let w = window(cx);
    cx.update_window(w, |_, window, _| window.find(id).bounds())
        .unwrap()
}

/// The pushed view's header is its toolbar: it holds the back chip, and for a
/// list the filter input; the root has no toolbar.
#[gpui_kit::test]
fn pushed_header_is_a_toolbar_with_back_and_filter(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list);
    pushed(cx, module);
    assert_eq!(header(cx).as_deref(), Some("Search"));
    let toolbar = bounds_of(cx, "view-header");
    assert!(contains(toolbar, bounds_of(cx, "view-back")), "back chip");
    assert!(contains(toolbar, bounds_of(cx, "list-input")), "filter");

    press(cx, "escape");
    assert_eq!(header(cx), None, "no toolbar at the root");
    assert!(label_of(cx, "view-back").is_none());
}

#[gpui_kit::test]
fn update_view_with_stale_revision_is_ignored(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list).with_view_events(echo_query);
    let core = pushed(cx, module);
    type_text(cx, "a");
    type_text(cx, "b");

    // The reply to revision 1 ("a") arrives while the view is at revision 2.
    deliver(cx, &core);
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"]);
    assert_eq!(value_of(cx, "list-input"), "ab");

    deliver(cx, &core);
    assert_eq!(row_titles(cx), ["ab result"]);
}

/// Review Focus 1.
#[gpui_kit::test]
fn late_reply_does_not_clobber_query(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list).with_view_events(echo_query);
    let core = pushed(cx, module);
    type_text(cx, "a");
    type_text(cx, "b");
    type_text(cx, "c");
    let r1 = next_update(&core);
    let r2 = next_update(&core);
    let r3 = next_update(&core);

    // The newest reply first, then the older ones.
    hand(cx, r3);
    hand(cx, r1);
    hand(cx, r2);

    assert_eq!(value_of(cx, "list-input"), "abc");
    assert_eq!(row_titles(cx), ["abc result"]);
}

#[gpui_kit::test]
fn module_list_sends_query_changed_with_revisions(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list).with_view_events(echo_query);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    type_text(cx, "a");
    type_text(cx, "b");
    deliver(cx, &core);
    deliver(cx, &core);

    let view = ViewId("fake/sub".into());
    assert_eq!(
        *events.lock().unwrap(),
        [
            (
                view.clone(),
                ViewEvent::QueryChanged {
                    revision: 1,
                    text: "a".into()
                }
            ),
            (
                view,
                ViewEvent::QueryChanged {
                    revision: 2,
                    text: "ab".into()
                }
            ),
        ]
    );
}

#[gpui_kit::test]
fn core_filter_list_filters_locally(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub);
    let events = module.view_events.clone();
    pushed(cx, module);

    type_text(cx, "ga");

    assert_eq!(row_titles(cx), ["Gamma"]);
    assert!(events.lock().unwrap().is_empty(), "no event to the module");
}

#[gpui_kit::test]
fn enter_in_list_sends_action_chosen(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(pop_on_action);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &core);

    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/sub".into()),
            ViewEvent::ActionChosen {
                row_id: "beta".into(),
                action_id: "pick".into()
            }
        )]
    );
    // The module answered with PopView.
    assert_eq!(header(cx), None);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn update_view_keeps_the_selected_row(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list).with_view_events(insert_on_query);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    type_text(cx, "x");
    press(cx, "down");
    // Beta is selected when a row is inserted above it.
    deliver(cx, &core);
    assert_eq!(row_titles(cx), ["New", "Alpha", "Beta", "Gamma"]);

    press(cx, "enter");
    deliver(cx, &core);

    let chosen = events.lock().unwrap().last().cloned();
    assert_eq!(
        chosen,
        Some((
            ViewId("fake/sub".into()),
            ViewEvent::ActionChosen {
                row_id: "beta".into(),
                action_id: "pick".into()
            }
        ))
    );
}

/// Carry-over ruling (c), in a pushed view.
#[gpui_kit::test]
fn enter_in_a_pushed_view_is_ignored_while_its_action_is_pending(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub)
        .with_view_events(|_, _| Ok(vec![]))
        .with_delay(Duration::from_millis(50));
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    press(cx, "enter");
    press(cx, "enter");
    deliver(cx, &core);
    // A second event, had it been sent, would have been answered by now.
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(events.lock().unwrap().len(), 1);
    assert!(core.borrow().events().is_empty());
}

#[gpui_kit::test]
fn reply_for_a_popped_view_is_ignored(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list).with_view_events(noisy_reply);
    let core = pushed(cx, module);
    type_text(cx, "a");

    press(cx, "escape");
    deliver(cx, &core);

    assert_eq!(header(cx), None);
    assert_eq!(toast(cx), None);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

#[gpui_kit::test]
fn view_failure_shows_error_toast(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(fail_on_action);
    let core = pushed(cx, module);

    press(cx, "enter");
    deliver(cx, &core);

    let message = toast(cx).expect("an error toast");
    assert!(message.contains("broken"), "toast: {message:?}");
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    // Errors stay for 5 s.
    cx.executor().advance_clock(Duration::from_millis(4900));
    cx.run_until_parked();
    assert!(toast(cx).is_some());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(toast(cx), None);
}

#[gpui_kit::test]
fn toast_appears_and_expires(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), success_toast));

    press(cx, "enter");
    deliver(cx, &core);

    assert_eq!(toast(cx).as_deref(), Some("Saved"));
    cx.executor().advance_clock(Duration::from_millis(2900));
    cx.run_until_parked();
    assert!(toast(cx).is_some());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(toast(cx), None);
}

#[gpui_kit::test]
fn progress_toast_stays_until_replaced(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), nothing));
    let progress = |title: &str| {
        Effect::Toast(Toast {
            kind: ToastKind::Progress,
            title: title.into(),
            message: Some("working".into()),
        })
    };
    // Effects of an action the window requested.
    press(cx, "enter");
    let CoreUpdate::ActionDone { request, .. } = next_update(&core) else {
        panic!("expected ActionDone");
    };
    hand(
        cx,
        CoreUpdate::ActionDone {
            request,
            provider: "fake".into(),
            items_changed: false,
            effects: vec![progress("Syncing")],
        },
    );

    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    let shown = toast(cx).expect("a progress toast");
    assert!(shown.starts_with("Syncing"), "toast: {shown:?}");

    press(cx, "enter");
    let CoreUpdate::ActionDone { request, .. } = next_update(&core) else {
        panic!("expected ActionDone");
    };
    hand(
        cx,
        CoreUpdate::ActionDone {
            request,
            provider: "fake".into(),
            items_changed: false,
            effects: vec![Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "Synced".into(),
                message: None,
            })],
        },
    );
    assert_eq!(toast(cx).as_deref(), Some("Synced"));
}

#[gpui_kit::test]
fn copy_text_writes_clipboard(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), copy_x));

    press(cx, "enter");
    deliver(cx, &core);

    let text = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(text.as_deref(), Some("x"));
}

#[gpui_kit::test]
fn sections_render_headers(cx: &mut TestAppContext) {
    pushed(cx, FakeModule::new(items(), push_sections));

    assert_eq!(labels_by_role(cx, Role::Heading), ["A", "B"]);
    assert_eq!(row_titles(cx), ["One", "Two", "Three"]);
}

#[gpui_kit::test]
fn show_detail_renders_the_selected_rows_markdown(cx: &mut TestAppContext) {
    pushed(cx, FakeModule::new(items(), push_detail_list));
    assert_eq!(label_of(cx, "list-detail").as_deref(), Some("About Alpha"));

    press(cx, "down");

    assert_eq!(label_of(cx, "list-detail").as_deref(), Some("About Beta"));
}

/// Carry-over ruling (b): a reply to a request of a window that was hidden
/// must not close the window shown since.
#[gpui_kit::test]
fn reply_for_an_earlier_window_does_not_close_the_new_one(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), hide).with_delay(Duration::from_millis(200));
    let core = setup(cx, module);

    press(cx, "enter");
    hide_launcher(cx);
    show(cx);
    deliver(cx, &core);

    assert!(is_visible(cx));
}

/// Carry-over ruling (c).
#[gpui_kit::test]
fn enter_is_ignored_while_an_action_is_pending(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), nothing).with_delay(Duration::from_millis(50));
    let calls = module.calls.clone();
    let core = setup(cx, module);

    press(cx, "enter");
    press(cx, "enter");
    deliver(cx, &core);
    // A second request, had it been sent, would have run by now.
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(core.borrow().events().is_empty());

    // Once the reply is in, Enter works again.
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(calls.lock().unwrap().len(), 2);
}

/// Carry-over ruling (d): the previous query's dynamic rows stay until the
/// new query's results arrive.
#[gpui_kit::test]
fn dynamic_rows_stay_until_new_results_arrive(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), nothing).with_search(vec![item("answer", "= 42")]);
    let core = setup(cx, module);
    type_text(cx, "f");
    while row_titles(cx).len() < 3 {
        deliver(cx, &core);
    }
    assert_eq!(row_titles(cx), ["= 42", "Files", "Firefox"]);

    // "fi": the static rows are new, the answer to "f" is held.
    type_text(cx, "i");
    assert_eq!(row_titles(cx), ["= 42", "Files", "Firefox"]);

    // The answer to "fi" replaces it, and the deadline no longer matters.
    deliver(cx, &core);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(row_titles(cx), ["= 42", "Files", "Firefox"]);
}

/// Carry-over ruling (d): ... or until the 100 ms deadline passes.
#[gpui_kit::test]
fn dynamic_rows_go_after_the_deadline(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), nothing).with_search(vec![item("answer", "= 42")]);
    let core = setup(cx, module);
    type_text(cx, "f");
    while row_titles(cx).len() < 3 {
        deliver(cx, &core);
    }

    // "fz" matches no static item; the answer to "f" is held.
    type_text(cx, "z");
    assert_eq!(row_titles(cx), ["= 42"]);
    cx.executor().advance_clock(Duration::from_millis(90));
    cx.run_until_parked();
    assert_eq!(row_titles(cx), ["= 42"]);
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();

    assert!(row_titles(cx).is_empty());
}

/// Ruling: a daemon run (hotkey, `tomakast run`) that pushes a view opens
/// the launcher on that view.
#[gpui_kit::test]
fn daemon_run_push_view_opens_the_launcher(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), push_sub));
    hide_launcher(cx);

    core.borrow_mut()
        .request_action(&ItemId::new("fake", "files"), "run", "")
        .unwrap();
    deliver(cx, &core);

    assert!(is_visible(cx));
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"]);
    // Esc goes back to the root list, as for a view pushed from the window.
    press(cx, "escape");
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);
}

/// Ruling: a daemon run's toast shows in the open window, and opens nothing
/// when the launcher is hidden.
#[gpui_kit::test]
fn daemon_run_toast_shows_only_in_an_open_window(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), success_toast));
    let files = ItemId::new("fake", "files");

    core.borrow_mut().request_action(&files, "run", "").unwrap();
    deliver(cx, &core);
    assert_eq!(toast(cx).as_deref(), Some("Saved"));

    hide_launcher(cx);
    core.borrow_mut().request_action(&files, "run", "").unwrap();
    deliver(cx, &core);
    assert!(!is_visible(cx));
}

/// Ruling: the core applies `ReplaceItems`, and the root list re-ranks.
#[gpui_kit::test]
fn replace_items_effect_refreshes_the_root_list(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), replace_with_gamma));

    press(cx, "enter");
    deliver(cx, &core);

    assert!(is_visible(cx));
    assert_eq!(row_titles(cx), ["Gamma"]);
}

/// Ruling: the Enter gate is per provider. Root rows with an empty query:
/// Files, Firefox (fake), Slow (slow), Terminal (fake).
#[gpui_kit::test]
fn a_hung_module_does_not_block_enter_on_other_providers(cx: &mut TestAppContext) {
    let slow_item = Item {
        id: ItemId::new("slow", "s"),
        ..item("s", "Slow")
    };
    let slow = FakeModule::new(vec![slow_item], nothing)
        .with_id("slow")
        .with_delay(Duration::from_millis(300));
    let fast = FakeModule::new(items(), nothing);
    let (slow_calls, fast_calls) = (slow.calls.clone(), fast.calls.clone());
    let core = setup_modules(cx, vec![slow, fast]);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Slow", "Terminal"]);

    press(cx, "down");
    press(cx, "down");
    press(cx, "enter");
    // A second Enter on the same provider is ignored while it is pending...
    press(cx, "enter");
    // ...but another provider's item runs at once.
    press(cx, "up");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(fast_calls.lock().unwrap().len(), 1);
    assert!(
        slow_calls.lock().unwrap().is_empty(),
        "the slow module is still busy"
    );

    deliver(cx, &core);
    // A second slow request, had it been sent, would have run by now.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(slow_calls.lock().unwrap().len(), 1);
    assert!(core.borrow().events().is_empty());
}

/// Ruling: a pending action stops gating Enter after 5 s.
#[gpui_kit::test]
fn the_enter_gate_expires_after_five_seconds(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), nothing).with_delay(Duration::from_millis(100));
    let calls = module.calls.clone();
    let core = setup(cx, module);

    press(cx, "enter");
    cx.executor().advance_clock(Duration::from_millis(4900));
    cx.run_until_parked();
    press(cx, "enter");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    press(cx, "enter");
    deliver(cx, &core);
    deliver(cx, &core);

    assert_eq!(calls.lock().unwrap().len(), 2);
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        core.borrow().events().is_empty(),
        "the Enter at 4.9 s was ignored"
    );
}

/// Review r1 #1: views are told apart by push, not by id. A reply for a
/// popped view must not reach a view pushed again with the same id.
#[gpui_kit::test]
fn reply_for_a_popped_view_skips_a_repushed_view_with_the_same_id(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_module_list).with_view_events(echo_with_toast);
    let core = pushed(cx, module);
    type_text(cx, "a");
    // The reply to "a" (revision 1) of the first instance, held back.
    let old = next_update(&core);

    press(cx, "escape");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Search"));
    // The new instance is at revision 1 too.
    type_text(cx, "b");

    hand(cx, old);
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"]);
    assert_eq!(toast(cx), None, "no effect of the old reply lands");
    deliver(cx, &core);
    assert_eq!(row_titles(cx), ["b result"]);
    assert_eq!(toast(cx).as_deref(), Some("b done"));
}

/// Review r1 #2: a root action's `PopView` does not pop another provider's
/// view. Root rows: Files, Firefox (fake), Slow (slow), Terminal (fake).
#[gpui_kit::test]
fn pop_view_does_not_pop_another_providers_view(cx: &mut TestAppContext) {
    let slow_item = Item {
        id: ItemId::new("slow", "s"),
        ..item("s", "Slow")
    };
    let slow = FakeModule::new(vec![slow_item], pop)
        .with_id("slow")
        .with_delay(Duration::from_millis(300));
    let fast = FakeModule::new(items(), push_sub);
    let core = setup_modules(cx, vec![slow, fast]);

    press(cx, "down");
    press(cx, "down");
    press(cx, "enter");
    press(cx, "up");
    press(cx, "enter");
    // The fast module's view comes first...
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    // ...then the slow module's PopView, which is not its view to pop.
    deliver(cx, &core);

    assert_eq!(header(cx).as_deref(), Some("Sub"));
}

/// A row with no detail, an empty one, or one with no markdown block must
/// render an empty pane, not panic.
#[gpui_kit::test]
fn detail_pane_without_markdown_renders(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_detail_list_without_detail),
    );
    for _ in 0..3 {
        assert_eq!(
            labels_by_role(cx, Role::Document),
            [""],
            "an empty detail pane"
        );
        press(cx, "down");
    }
}
