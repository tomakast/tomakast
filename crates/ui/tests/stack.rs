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
    Core, CoreUpdate, DetailView, Effect, Item, ItemId, ItemKind, ListRow, ListView, LocalNow,
    RankWeights, ScratchpadView, SearchMode, Toast, ToastKind, View, ViewEvent, ViewId,
};
use tomakast_ui::{LauncherController, LauncherView};

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
            shortcut: None,
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
            shortcut: None,
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

/// Answers every query with the sub rows minus the one the query names ("b" drops Beta,
/// "g" drops Gamma).
fn drop_named_row_on_query(view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::QueryChanged { revision, text } = event else {
        return Ok(vec![]);
    };
    let rows = sub_rows()
        .into_iter()
        .filter(|r| !r.id.starts_with(text.as_str()))
        .collect();
    let mut reply = list("Search", SearchMode::Module, rows);
    reply.id = view.clone();
    reply.revision = revision;
    Ok(vec![Effect::UpdateView(View::List(reply))])
}

/// The row id of the last `ActionChosen` the module saw.
fn chosen_row(events: &std::sync::Mutex<Vec<(ViewId, ViewEvent)>>) -> Option<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|(_, e)| match e {
            ViewEvent::ActionChosen { row_id, .. } => Some(row_id.clone()),
            _ => None,
        })
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
    // The reply to the `Closed` of the popped view.
    deliver(cx, &core);

    assert_eq!(
        *events.lock().unwrap(),
        [
            (
                ViewId("fake/sub".into()),
                ViewEvent::ActionChosen {
                    row_id: "beta".into(),
                    action_id: "pick".into()
                }
            ),
            (ViewId("fake/sub".into()), ViewEvent::Closed)
        ]
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

#[gpui_kit::test]
fn update_view_selects_the_row_now_at_the_index_when_the_selected_one_goes(
    cx: &mut TestAppContext,
) {
    let module =
        FakeModule::new(items(), push_module_list).with_view_events(drop_named_row_on_query);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    type_text(cx, "b");
    press(cx, "down");
    // Beta is selected and the update removes it: Gamma now sits at its index.
    deliver(cx, &core);
    assert_eq!(row_titles(cx), ["Alpha", "Gamma"]);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(chosen_row(&events).as_deref(), Some("gamma"));
}

#[gpui_kit::test]
fn update_view_clamps_the_selection_when_the_last_row_goes(cx: &mut TestAppContext) {
    let module =
        FakeModule::new(items(), push_module_list).with_view_events(drop_named_row_on_query);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    type_text(cx, "g");
    press(cx, "down");
    press(cx, "down");
    deliver(cx, &core);
    assert_eq!(row_titles(cx), ["Alpha", "Beta"]);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(chosen_row(&events).as_deref(), Some("beta"));
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
            module_instance: core.borrow().module_instance("fake").unwrap(),
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
            module_instance: core.borrow().module_instance("fake").unwrap(),
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

fn pushed_ids(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |root, _, cx| {
        root.downcast::<LauncherView>()
            .unwrap()
            .read(cx)
            .pushed_view_ids()
    })
    .unwrap()
}

/// `fake/sub` (a history) opens `fake/sheet`, whose Enter opens `fake/sub` again.
fn history_and_sheet(view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::ActionChosen { .. } = event else {
        return Ok(vec![]);
    };
    let next = match view.0.as_str() {
        "fake/sub" => {
            let mut sheet = list("Sheet", SearchMode::CoreFilter, vec![row("x", "X", None)]);
            sheet.id = ViewId("fake/sheet".into());
            sheet
        }
        _ => list("Sub again", SearchMode::CoreFilter, sub_rows()),
    };
    Ok(vec![Effect::PushView(View::List(next))])
}

/// Ruling 9: a push of a view id the module already has on the stack returns to that
/// entry (closing the views above it) and replaces it, instead of stacking a copy.
#[gpui_kit::test]
fn push_of_a_view_on_the_stack_returns_to_it(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(history_and_sheet);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    type_text(cx, "be");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(pushed_ids(cx), ["fake/sub", "fake/sheet"]);

    press(cx, "enter");
    deliver(cx, &core);
    // The answer to the sheet's `Closed`.
    deliver(cx, &core);
    // One history: the sheet above it was closed, and the history is the new push.
    assert_eq!(pushed_ids(cx), ["fake/sub"]);
    assert_eq!(header(cx).as_deref(), Some("Sub again"));
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"], "a fresh filter");
    let closed: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, e)| *e == ViewEvent::Closed)
        .map(|(v, _)| v.0.clone())
        .collect();
    assert_eq!(closed, ["fake/sheet"], "the history itself stays open");
    press(cx, "escape");
    deliver(cx, &core);
    assert_eq!(header(cx), None, "Esc goes back to the root list");

    // A daemon run (hotkey, `tomakast run`) of the same view while it is open.
    press(cx, "enter");
    deliver(cx, &core);
    core.borrow_mut()
        .request_action(&ItemId::new("fake", "files"), "run", "")
        .unwrap();
    deliver(cx, &core);
    assert_eq!(pushed_ids(cx), ["fake/sub"]);
}

static SUB_THEN_X: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The first run pushes `fake/sub`, every later one `fake/x`.
fn push_sub_then_x() -> anyhow::Result<Vec<Effect>> {
    if SUB_THEN_X.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
        return push_sub();
    }
    let mut x = list("X", SearchMode::CoreFilter, sub_rows());
    x.id = ViewId("fake/x".into());
    Ok(vec![Effect::PushView(View::List(x))])
}

fn push_other() -> anyhow::Result<Vec<Effect>> {
    let mut view = list("Other", SearchMode::CoreFilter, sub_rows());
    view.id = ViewId("other/o".into());
    Ok(vec![Effect::PushView(View::List(view))])
}

/// Hands replies to the UI until the answer of an action arrives (answers to `Closed`
/// events may come first).
fn deliver_action(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    loop {
        let update = next_update(core);
        let done = matches!(update, CoreUpdate::ActionDone { .. });
        hand(cx, update);
        if done {
            return;
        }
    }
}

/// Ruling 9b: an unsolicited push returns to the module's view only over the module's own
/// views; over another module's view it is dropped.
#[gpui_kit::test]
fn unsolicited_push_returns_only_over_its_own_views(cx: &mut TestAppContext) {
    let other_item = Item {
        id: ItemId::new("other", "o"),
        ..item("o", "Other thing")
    };
    let core = setup_modules(
        cx,
        vec![
            FakeModule::new(items(), push_sub_then_x),
            FakeModule::new(vec![other_item], push_other).with_id("other"),
        ],
    );
    let run = |cx: &mut TestAppContext, provider: &str, local: &str| {
        core.borrow_mut()
            .request_action(&ItemId::new(provider, local), "run", "")
            .unwrap();
        deliver_action(cx, &core);
    };
    run(cx, "fake", "files");
    run(cx, "other", "o");
    run(cx, "fake", "files");
    assert_eq!(pushed_ids(cx), ["fake/sub", "other/o", "fake/x"]);
    let again = || {
        let mut view = list("Again", SearchMode::CoreFilter, sub_rows());
        view.id = ViewId("fake/sub".into());
        vec![Effect::PushView(View::List(view))]
    };

    unsolicited(cx, &core, "fake", again());
    assert_eq!(pushed_ids(cx), ["fake/sub", "other/o", "fake/x"], "dropped");
    assert_eq!(header(cx).as_deref(), Some("X"));

    // Only the module's own views above: it returns to the view.
    press(cx, "escape");
    press(cx, "escape");
    run(cx, "fake", "files");
    assert_eq!(pushed_ids(cx), ["fake/sub", "fake/x"]);
    unsolicited(cx, &core, "fake", again());
    assert_eq!(pushed_ids(cx), ["fake/sub"]);
    assert_eq!(header(cx).as_deref(), Some("Again"));
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
    // The reply to the first instance's `Closed` (a toast) is dropped too.
    deliver(cx, &core);
    assert_eq!(toast(cx), None);
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

// Unsolicited effects (WIT `effect`, W11), the view lifecycle (`Closed`)
// and the link policy.

/// Hands `effects` to the UI as sent by the running instance of `provider`
/// (an unknown provider gets instance 0, which no module has).
fn unsolicited(
    cx: &mut TestAppContext,
    core: &Rc<RefCell<Core>>,
    provider: &str,
    effects: Vec<Effect>,
) {
    let module_instance = core.borrow().module_instance(provider).unwrap_or(0);
    hand(
        cx,
        CoreUpdate::ModuleEffects {
            provider: provider.into(),
            module_instance,
            effects,
            items_changed: false,
        },
    );
}

/// A `CoreFilter` list `id`, titled `title`, with one row `row_title`.
fn one_row_list(id: &str, title: &str, row_title: &str) -> View {
    let mut view = list(
        title,
        SearchMode::CoreFilter,
        vec![row("r", row_title, None)],
    );
    view.id = ViewId(id.into());
    View::List(view)
}

/// `ActionChosen` pushes a second view with the same id, "Sub 2".
fn push_same_id_on_action(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    match event {
        ViewEvent::ActionChosen { .. } => Ok(vec![Effect::PushView(one_row_list(
            "fake/sub", "Sub 2", "Second",
        ))]),
        _ => Ok(vec![]),
    }
}

#[gpui_kit::test]
fn unsolicited_update_targets_topmost_matching_view(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(push_same_id_on_action);
    let core = pushed(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Sub 2"));
    // Ruling 9: the push of the same id replaced the view instead of stacking a copy.
    assert_eq!(pushed_ids(cx), ["fake/sub"]);

    // Another module's view of the same id is not this one's.
    unsolicited(
        cx,
        &core,
        "other",
        vec![Effect::UpdateView(one_row_list("fake/sub", "Hijack", "x"))],
    );
    assert_eq!(header(cx).as_deref(), Some("Sub 2"));
    assert_eq!(row_titles(cx), ["Second"]);

    unsolicited(
        cx,
        &core,
        "fake",
        vec![Effect::UpdateView(one_row_list(
            "fake/sub", "Updated", "Fresh",
        ))],
    );
    assert_eq!(header(cx).as_deref(), Some("Updated"));
    assert_eq!(row_titles(cx), ["Fresh"]);

    press(cx, "escape");
    assert_eq!(header(cx), None, "one view: Esc goes back to the root list");
}

#[gpui_kit::test]
fn unsolicited_update_without_target_is_dropped(cx: &mut TestAppContext) {
    let core = pushed(cx, FakeModule::new(items(), push_sub));
    unsolicited(
        cx,
        &core,
        "fake",
        vec![Effect::UpdateView(one_row_list("fake/other", "Other", "x"))],
    );
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"]);

    // On the root list there is no view to update.
    press(cx, "escape");
    unsolicited(
        cx,
        &core,
        "fake",
        vec![Effect::UpdateView(one_row_list("fake/sub", "Sub", "x"))],
    );
    assert_eq!(header(cx), None);
    assert_eq!(row_titles(cx), ["Files", "Firefox", "Terminal"]);

    // A hidden launcher has no views; nothing opens.
    hide_launcher(cx);
    unsolicited(
        cx,
        &core,
        "fake",
        vec![Effect::UpdateView(one_row_list("fake/sub", "Sub", "x"))],
    );
    assert!(!is_visible(cx));
}

#[gpui_kit::test]
fn unsolicited_push_needs_open_window_and_owner_on_top(cx: &mut TestAppContext) {
    let next = || vec![Effect::PushView(one_row_list("fake/next", "Next", "n"))];
    let core = setup(cx, FakeModule::new(items(), push_sub));

    // The root list belongs to no module.
    unsolicited(cx, &core, "fake", next());
    assert_eq!(header(cx), None);

    // A hidden launcher is not opened.
    hide_launcher(cx);
    unsolicited(cx, &core, "fake", next());
    assert!(!is_visible(cx));

    show(cx);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Sub"));

    // The top view is fake's, not other's.
    unsolicited(
        cx,
        &core,
        "other",
        vec![Effect::PushView(one_row_list("other/x", "Other", "o"))],
    );
    assert_eq!(header(cx).as_deref(), Some("Sub"));

    unsolicited(cx, &core, "fake", next());
    assert_eq!(header(cx).as_deref(), Some("Next"));
    press(cx, "escape");
    assert_eq!(header(cx).as_deref(), Some("Sub"));
}

/// Review Focus: a module replaced (a reloaded plugin) while one of its views
/// is open. The view stays; its events reach neither the stopped instance nor
/// the new one, and the user sees "plugin reloaded". The new instance cannot
/// push over or update the old instance's view either.
#[gpui_kit::test]
fn events_of_a_view_of_a_replaced_module_show_plugin_reloaded(cx: &mut TestAppContext) {
    let old = FakeModule::new(items(), push_sub);
    let old_events = old.view_events.clone();
    let core = pushed(cx, old);
    assert_eq!(header(cx).as_deref(), Some("Sub"));

    let new = FakeModule::new(items(), push_sub);
    let new_events = new.view_events.clone();
    core.borrow_mut().register(Box::new(new));
    deliver(cx, &core);

    // The new instance's unsolicited effects do not touch the old view.
    unsolicited(
        cx,
        &core,
        "fake",
        vec![
            Effect::UpdateView(one_row_list("fake/sub", "Sub", "Hijacked")),
            Effect::PushView(one_row_list("fake/next", "Next", "n")),
        ],
    );
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    assert_eq!(row_titles(cx), ["Alpha", "Beta", "Gamma"]);

    // Enter on a row: `ActionChosen` for the old view.
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(toast(cx).as_deref(), Some(tomakast_core::RELOADED));
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    assert!(old_events.lock().unwrap().is_empty());
    assert!(new_events.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn unsolicited_close_and_pop_are_ignored(cx: &mut TestAppContext) {
    let core = pushed(cx, FakeModule::new(items(), push_sub));

    unsolicited(
        cx,
        &core,
        "fake",
        vec![
            Effect::PopView,
            Effect::CloseWindow,
            Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "Tick".into(),
                message: None,
            }),
            Effect::CopyText("one".into()),
        ],
    );
    assert!(is_visible(cx));
    assert_eq!(header(cx).as_deref(), Some("Sub"));
    // Toasts show while the window is open; copying always works.
    assert_eq!(toast(cx).as_deref(), Some("Tick"));
    let text = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(text.as_deref(), Some("one"));

    hide_launcher(cx);
    unsolicited(
        cx,
        &core,
        "fake",
        vec![
            Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "Hidden".into(),
                message: None,
            }),
            Effect::CopyText("two".into()),
        ],
    );
    assert!(!is_visible(cx));
    let text = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(text.as_deref(), Some("two"));
}

fn pad(id: &str) -> View {
    View::Scratchpad(ScratchpadView {
        id: ViewId(id.into()),
        title: "Pad".into(),
        text: String::new(),
        revision: 0,
        annotations: Vec::new(),
        actions: Vec::new(),
    })
}

fn push_pad() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(pad("fake/pad"))])
}

/// `ActionChosen` pushes a scratchpad.
fn push_pad_on_action(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    match event {
        ViewEvent::ActionChosen { .. } => Ok(vec![Effect::PushView(pad("fake/pad"))]),
        _ => Ok(vec![]),
    }
}

type Events = std::sync::Arc<std::sync::Mutex<Vec<(ViewId, ViewEvent)>>>;

/// Delivers the replies to the `count` view events the module gets next,
/// and returns every event it received so far.
fn received(
    cx: &mut TestAppContext,
    core: &Rc<RefCell<Core>>,
    events: &Events,
    count: usize,
) -> Vec<(ViewId, ViewEvent)> {
    for _ in 0..count {
        deliver(cx, core);
    }
    events.lock().unwrap().clone()
}

fn text_changed(id: &str, revision: u64, text: &str) -> (ViewId, ViewEvent) {
    (
        ViewId(id.into()),
        ViewEvent::TextChanged {
            revision,
            text: text.into(),
        },
    )
}

fn closed(id: &str) -> (ViewId, ViewEvent) {
    (ViewId(id.into()), ViewEvent::Closed)
}

/// I3: the debounced text is not lost when the view goes.
#[gpui_kit::test]
fn pop_flushes_pending_scratchpad_text_then_sends_closed(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_pad);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    assert_eq!(header(cx).as_deref(), Some("Pad"));

    // Typed, and popped before the debounce fires.
    type_text(cx, "a");
    press(cx, "escape");
    assert_eq!(header(cx), None);
    assert_eq!(
        received(cx, &core, &events, 2),
        [text_changed("fake/pad", 1, "a"), closed("fake/pad")]
    );

    // Typed, sent, then popped: the text is not sent twice.
    press(cx, "enter");
    deliver(cx, &core);
    type_text(cx, "b");
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    press(cx, "escape");
    let events_now = received(cx, &core, &events, 2);
    assert_eq!(
        events_now[2..],
        [text_changed("fake/pad", 1, "b"), closed("fake/pad")]
    );

    // Never edited: only `Closed`.
    press(cx, "enter");
    deliver(cx, &core);
    press(cx, "escape");
    let events_now = received(cx, &core, &events, 1);
    assert_eq!(events_now[4..], [closed("fake/pad")]);
    std::thread::sleep(Duration::from_millis(50));
    assert!(core.borrow().events().is_empty(), "one event per pop");
}

#[gpui_kit::test]
fn hide_flushes_and_closes_every_stack_entry(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(push_pad_on_action);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Pad"));
    type_text(cx, "x");

    hide_launcher(cx);
    assert!(!is_visible(cx));
    let all = received(cx, &core, &events, 3);
    // After the `ActionChosen` that pushed the pad: top to bottom.
    assert_eq!(
        all[1..],
        [
            text_changed("fake/pad", 1, "x"),
            closed("fake/pad"),
            closed("fake/sub")
        ]
    );

    // Each view is closed once: a new window has none of them.
    show(cx);
    hide_launcher(cx);
    std::thread::sleep(Duration::from_millis(50));
    assert!(core.borrow().events().is_empty());
    assert_eq!(events.lock().unwrap().len(), 4);
}

/// Records the providers it was asked about; allows while `allow` is set.
#[derive(Default)]
struct SwitchPolicy {
    allow: std::cell::Cell<bool>,
    asked: RefCell<Vec<String>>,
}

impl tomakast_ui::LinkPolicy for SwitchPolicy {
    fn may_open_links(&self, provider: &str) -> bool {
        self.asked.borrow_mut().push(provider.to_string());
        self.allow.get()
    }
}

fn push_link_detail() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Detail(DetailView {
        id: ViewId("fake/detail".into()),
        title: "Links".into(),
        markdown: "[Open me](https://a.test/x)".into(),
        metadata: Vec::new(),
        actions: Vec::new(),
    }))])
}

fn click_link(cx: &mut TestAppContext) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.click_at(
            "detail-markdown",
            gpui_kit::point(gpui_kit::px(8.), gpui_kit::px(8.)),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn links_disabled_when_policy_denies(cx: &mut TestAppContext) {
    let policy = Rc::new(SwitchPolicy::default());
    let core = setup(cx, FakeModule::new(items(), push_link_detail));
    hide_launcher(cx);
    let installed = policy.clone();
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, _| c.set_link_policy(installed)));
    show(cx);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Links"));

    click_link(cx);
    assert_eq!(cx.opened_url(), None, "the policy denies fake's links");
    assert_eq!(*policy.asked.borrow(), ["fake"]);

    // The same click opens the link once the policy allows it.
    policy.allow.set(true);
    click_link(cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://a.test/x"));
}

/// Names `fake` as a plugin.
struct PluginNames;

impl tomakast_ui::LinkPolicy for PluginNames {
    fn may_open_links(&self, _provider: &str) -> bool {
        false
    }

    fn plugin_name(&self, provider: &str) -> Option<String> {
        (provider == "fake").then(|| "Fake Plugin".to_owned())
    }
}

/// Ruling AN: a plugin's view shows the plugin's name before its title in the footer.
#[gpui_kit::test]
fn a_plugin_view_names_its_plugin_in_the_footer(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), push_link_detail));
    hide_launcher(cx);
    cx.update(|cx| {
        cx.update_global::<LauncherController, _>(|c, _| c.set_link_policy(Rc::new(PluginNames)))
    });
    show(cx);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Links"));
    assert_eq!(
        label_of(cx, "footer-context").as_deref(),
        Some("Fake Plugin \u{b7} Links")
    );
}

/// The twin: a built-in module's view shows its title alone.
#[gpui_kit::test]
fn a_native_view_shows_its_title_in_the_footer(cx: &mut TestAppContext) {
    let core = setup(cx, FakeModule::new(items(), push_link_detail));
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Links"));
    assert_eq!(label_of(cx, "footer-context").as_deref(), Some("Links"));
}

fn push_link_list() -> anyhow::Result<Vec<Effect>> {
    let mut rows = sub_rows();
    rows[0].detail_markdown = Some("[Open me](https://a.test/x)".into());
    let mut view = list("Details", SearchMode::CoreFilter, rows);
    view.show_detail = true;
    Ok(vec![Effect::PushView(View::List(view))])
}

/// The list's detail pane follows the policy too.
#[gpui_kit::test]
fn list_detail_links_follow_the_policy(cx: &mut TestAppContext) {
    let policy = Rc::new(SwitchPolicy::default());
    let core = setup(cx, FakeModule::new(items(), push_link_list));
    hide_launcher(cx);
    let installed = policy.clone();
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, _| c.set_link_policy(installed)));
    show(cx);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(label_of(cx, "list-detail").as_deref(), Some("Open me"));

    // Past the pane's padding, on the link.
    let click = |cx: &mut TestAppContext| {
        let w = window(cx);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(
                "list-detail",
                gpui_kit::point(gpui_kit::px(24.), gpui_kit::px(24.)),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    };
    click(cx);
    assert_eq!(cx.opened_url(), None, "the policy denies fake's links");
    assert_eq!(*policy.asked.borrow(), ["fake"]);

    policy.allow.set(true);
    click(cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://a.test/x"));
}

/// A window closed without `hide` (by the compositor) closes its views too.
#[gpui_kit::test]
fn window_closed_elsewhere_closes_every_stack_entry(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(push_pad_on_action);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(header(cx).as_deref(), Some("Pad"));
    type_text(cx, "x");

    let w = window(cx);
    cx.update_window(w, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(!is_visible(cx));

    let all = received(cx, &core, &events, 3);
    assert_eq!(
        all[1..],
        [
            text_changed("fake/pad", 1, "x"),
            closed("fake/pad"),
            closed("fake/sub")
        ]
    );
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        core.borrow().events().is_empty(),
        "each view is closed once"
    );
}

/// Forty views with distinct ids (a push of an id on the stack would return to it).
fn push_forty() -> anyhow::Result<Vec<Effect>> {
    Ok((0..40)
        .map(|i| {
            let mut view = list(&format!("View {i}"), SearchMode::CoreFilter, sub_rows());
            view.id = ViewId(format!("fake/v{i}"));
            Effect::PushView(View::List(view))
        })
        .collect())
}

/// Ruling AB: the stack holds at most 32 pushed views; pushes beyond are dropped.
#[gpui_kit::test]
fn view_stack_holds_at_most_32_pushed_views(cx: &mut TestAppContext) {
    pushed(cx, FakeModule::new(items(), push_forty));
    assert_eq!(header(cx).as_deref(), Some("View 31"));
    for _ in 0..31 {
        press(cx, "escape");
    }
    assert_eq!(header(cx).as_deref(), Some("View 0"));
    press(cx, "escape");
    assert_eq!(header(cx), None, "back at the root after 32 pops");
    assert!(is_visible(cx));
}

// The internal `SetRootQuery` effect: a built-in module puts a query in the root field.

/// `ActionChosen` in any view sets the root query to "fir".
fn set_root_query_on_action(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    match event {
        ViewEvent::ActionChosen { .. } => Ok(vec![Effect::SetRootQuery("fir".into())]),
        _ => Ok(vec![]),
    }
}

/// The value of the root search field.
fn query_value(cx: &mut TestAppContext) -> String {
    value_of(cx, "launcher-input")
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

#[gpui_kit::test]
fn set_root_query_pops_views_and_searches(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(set_root_query_on_action);
    let events = module.view_events.clone();
    let calls = module.calls.clone();
    let core = pushed(cx, module);
    assert_eq!(pushed_ids(cx), ["fake/sub"]);

    press(cx, "enter");
    deliver(cx, &core);

    assert!(pushed_ids(cx).is_empty());
    assert_eq!(query_value(cx), "fir");
    assert_eq!(row_titles(cx), ["Firefox"]);
    assert_eq!(
        events.lock().unwrap().last().cloned(),
        Some((ViewId("fake/sub".into()), ViewEvent::Closed))
    );
    assert_eq!(query_selection(cx), 3..3);
    assert_eq!(root_selection(cx).as_deref(), Some("Firefox"));

    press(cx, "enter");
    assert_eq!(
        calls.lock().unwrap().last().cloned(),
        Some((ItemId::new("fake", "firefox"), "run".to_string()))
    );
}

#[gpui_kit::test]
fn set_root_query_over_two_views_closes_both(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub).with_view_events(|view, event| {
        let ViewEvent::ActionChosen { .. } = event else {
            return Ok(vec![]);
        };
        match view.0.as_str() {
            "fake/sub" => history_and_sheet(view, event),
            _ => Ok(vec![Effect::SetRootQuery("fir".into())]),
        }
    });
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(pushed_ids(cx), ["fake/sub", "fake/sheet"]);

    press(cx, "enter");
    deliver(cx, &core);

    assert!(pushed_ids(cx).is_empty());
    assert_eq!(query_value(cx), "fir");
    let closed: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, e)| *e == ViewEvent::Closed)
        .map(|(v, _)| v.0.clone())
        .collect();
    assert_eq!(closed, ["fake/sheet", "fake/sub"], "top first");
}

#[gpui_kit::test]
fn unsolicited_set_root_query_is_ignored(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_sub);
    let ctx = module.ctx.clone();
    let core = pushed(cx, module);

    ctx.lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .emit(vec![Effect::SetRootQuery("x".into())]);
    deliver(cx, &core);

    assert_eq!(pushed_ids(cx), ["fake/sub"]);
    press(cx, "escape");
    assert_eq!(query_value(cx), "");
}
