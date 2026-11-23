//! Headless tests of a list view drawn as a grid of tiles.
//!
//! As in `stack.rs`, module calls run on real worker threads and the test
//! thread hands each reply to the controller, as the daemon does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, Role, TestAppContext,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, Effect, Item, ItemId, ItemKind, ListLayout, ListRow, ListView, LocalNow, RankWeights,
    SearchMode, View, ViewEvent, ViewId,
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
        source: None,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
            shortcut: None,
        }],
    }
}

fn items() -> Vec<Item> {
    vec![item("files", "Files")]
}

/// Rows `r0`.. titled "Row 0".., in two sections, each with a detail.
fn rows(count: usize) -> Vec<ListRow> {
    (0..count)
        .map(|ix| ListRow {
            id: format!("r{ix}"),
            section: Some(if ix < 2 { "First" } else { "Second" }.into()),
            title: format!("Row {ix}"),
            subtitle: None,
            icon: None,
            accessories: Vec::new(),
            detail_markdown: Some(format!("About row {ix}")),
            actions: vec![Action {
                id: "open".into(),
                title: "Open".into(),
                shortcut: None,
            }],
        })
        .collect()
}

fn list(layout: ListLayout, rows: Vec<ListRow>) -> ListView {
    ListView {
        id: ViewId("fake/files".into()),
        title: "Search Files".into(),
        placeholder: "Search…".into(),
        search: SearchMode::Module,
        query: String::new(),
        revision: 0,
        rows,
        show_detail: true,
        layout,
        footer_action: None,
        busy: false,
    }
}

fn push_grid() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(list(
        ListLayout::Grid { columns: 3 },
        rows(7),
    )))])
}

fn push_rows() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(list(
        ListLayout::Rows,
        rows(4),
    )))])
}

fn nothing(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

/// The revision of the last `QueryChanged` [`switch_on_action`] saw.
static LAST_REVISION: AtomicU64 = AtomicU64::new(0);

/// Answers a chosen action as the files screen's toggle does: the same view at
/// its current revision, with an empty query and a grid of 4 columns.
fn switch_on_action(view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    match event {
        ViewEvent::QueryChanged { revision, .. } => {
            LAST_REVISION.store(revision, Ordering::SeqCst);
            Ok(vec![])
        }
        ViewEvent::ActionChosen { .. } => {
            let mut update = list(ListLayout::Grid { columns: 4 }, rows(4));
            update.id = view.clone();
            update.revision = LAST_REVISION.load(Ordering::SeqCst);
            Ok(vec![Effect::UpdateView(View::List(update))])
        }
        _ => Ok(vec![]),
    }
}

fn setup(cx: &mut TestAppContext, module: FakeModule) -> Rc<RefCell<Core>> {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap();
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

fn deliver(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let ev = wait_event(&core.borrow().events());
    let update = core.borrow_mut().apply(ev, LocalNow::now());
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_update(update, cx)));
    cx.run_until_parked();
}

fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "expected exactly the launcher window");
    windows[0]
}

fn press(cx: &mut TestAppContext, key: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Types `text` into the list's field, one character at a time, and hands
/// the module's reply to each query to the UI.
fn type_text(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>, text: &str) {
    let w = window(cx);
    for ch in text.chars() {
        cx.update_window(w, |_, window, cx| window.input(&ch.to_string(), cx))
            .unwrap();
        cx.run_until_parked();
        deliver(cx, core);
    }
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

fn exists(cx: &mut TestAppContext, id: &'static str) -> bool {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn field(cx: &mut TestAppContext) -> String {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find("list-input")
            .value()
            .unwrap_or_default()
            .to_string()
    })
    .unwrap()
}

/// Shows the launcher over `module` and opens its list with Enter on Files.
fn pushed(cx: &mut TestAppContext, module: FakeModule) -> Rc<RefCell<Core>> {
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    core
}

/// Presses Enter and returns the row id of the `ActionChosen` it sent.
fn enter(
    cx: &mut TestAppContext,
    core: &Rc<RefCell<Core>>,
    events: &Mutex<Vec<(ViewId, ViewEvent)>>,
) -> String {
    press(cx, "enter");
    deliver(cx, core);
    let events = events.lock().unwrap();
    let Some((_, ViewEvent::ActionChosen { row_id, .. })) = events.last() else {
        panic!("no ActionChosen: {events:?}");
    };
    row_id.clone()
}

#[gpui_kit::test]
fn grid_list_draws_tiles_without_sections_or_detail(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_grid).with_view_events(nothing),
    );
    assert!(exists(cx, "list-grid"), "the rows are drawn as a grid");
    let tiles = labels_by_role(cx, Role::ListItem);
    assert_eq!(tiles.len(), 7);
    assert_eq!(tiles[0], "Row 0");
    assert!(labels_by_role(cx, Role::Heading).is_empty(), "no sections");
    assert!(!exists(cx, "list-detail"), "no detail pane");
}

#[gpui_kit::test]
fn grid_list_moves_in_two_dimensions(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_grid).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    // Text in the field: Left and Right would move its caret in a list of rows.
    type_text(cx, &core, "ab");

    // 3 columns, 7 rows.
    press(cx, "right");
    press(cx, "right");
    press(cx, "down");
    assert_eq!(enter(cx, &core, &events), "r5");
    press(cx, "left");
    assert_eq!(enter(cx, &core, &events), "r4");
    // Down into the short last row lands on its last tile.
    press(cx, "down");
    assert_eq!(enter(cx, &core, &events), "r6");
    press(cx, "up");
    assert_eq!(enter(cx, &core, &events), "r3");
    // The edges hold, as in a grid view: Left at a row's first tile stays,
    // and the key is the grid's, not the caret's.
    press(cx, "left");
    assert_eq!(enter(cx, &core, &events), "r3");
    type_text(cx, &core, "c");
    assert_eq!(field(cx), "abc");

    // Typing selected r0 again. With the caret at the start, so that a Right
    // reaching the field would show: Right at a row's last tile stays, Up at
    // the top row stays.
    press(cx, "home");
    press(cx, "right");
    press(cx, "right");
    press(cx, "right");
    press(cx, "up");
    assert_eq!(enter(cx, &core, &events), "r2");
    // Right at the last tile of the short last row stays.
    press(cx, "down");
    press(cx, "down");
    assert_eq!(enter(cx, &core, &events), "r6");
    press(cx, "right");
    assert_eq!(enter(cx, &core, &events), "r6");
    type_text(cx, &core, "d");
    assert_eq!(field(cx), "dabc");
}

#[gpui_kit::test]
fn rows_list_left_and_right_still_move_the_caret(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_rows).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    assert!(!exists(cx, "list-grid"));
    type_text(cx, &core, "ab");
    press(cx, "left");
    type_text(cx, &core, "x");
    assert_eq!(field(cx), "axb");
    // The selection did not move.
    assert_eq!(enter(cx, &core, &events), "r0");
}

#[gpui_kit::test]
fn switching_layout_keeps_query_and_selection(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_rows).with_view_events(switch_on_action);
    let events = module.view_events.clone();
    let core = pushed(cx, module);
    type_text(cx, &core, "ab");
    press(cx, "down");
    press(cx, "down");

    // The module answers the chosen action with the same view as a grid.
    assert_eq!(enter(cx, &core, &events), "r2");
    assert!(exists(cx, "list-grid"), "the update switched the layout");
    assert_eq!(field(cx), "ab");
    assert!(!exists(cx, "list-detail"));

    // Still on r2 (the reply switches again, to the same grid).
    assert_eq!(enter(cx, &core, &events), "r2");
    // Right now moves the selection, not the caret.
    press(cx, "right");
    assert_eq!(enter(cx, &core, &events), "r3");
    assert_eq!(field(cx), "ab");
}

fn label(cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

/// Rows with a layout toggle on Ctrl+G, the list naming it as its footer action.
fn push_with_footer_action() -> anyhow::Result<Vec<Effect>> {
    let mut rows = rows(3);
    for row in &mut rows {
        row.actions.push(Action {
            id: "toggle".into(),
            title: "Show as Grid".into(),
            shortcut: Some("ctrl+g".into()),
        });
    }
    let mut view = list(ListLayout::Rows, rows);
    view.footer_action = Some("toggle".into());
    Ok(vec![Effect::PushView(View::List(view))])
}

#[gpui_kit::test]
fn footer_shows_the_lists_footer_action_with_its_keys(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_with_footer_action).with_view_events(nothing),
    );
    assert_eq!(label(cx, "footer-primary").as_deref(), Some("⏎ Open"));
    assert_eq!(
        label(cx, "footer-extra").as_deref(),
        Some("Ctrl+G Show as Grid")
    );
    assert!(label(cx, "footer-actions").is_some());
}

#[gpui_kit::test]
fn a_list_without_a_footer_action_shows_none(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_rows).with_view_events(nothing),
    );
    assert!(label(cx, "footer-primary").is_some());
    assert_eq!(label(cx, "footer-extra"), None);
}

/// The bounds of the tiles, in order.
fn tile_bounds(cx: &mut TestAppContext) -> Vec<gpui_kit::Bounds<gpui_kit::Pixels>> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::ListItem)
            .iter()
            .map(|e| e.bounds())
            .collect()
    })
    .unwrap()
}

/// A short last row (7 tiles in rows of 3: the last has one) lays its tiles out as the
/// full rows do: same width, same columns, nothing shifted.
#[gpui_kit::test]
fn a_short_last_row_keeps_the_columns(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_grid).with_view_events(nothing),
    );
    let tiles = tile_bounds(cx);
    assert_eq!(tiles.len(), 7);
    assert_eq!(tiles[6].origin.x, tiles[0].origin.x);
    assert_eq!(tiles[6].size.width, tiles[0].size.width);
}

fn push_busy() -> anyhow::Result<Vec<Effect>> {
    let mut view = list(ListLayout::Rows, Vec::new());
    view.busy = true;
    Ok(vec![Effect::PushView(View::List(view))])
}

#[gpui_kit::test]
fn a_busy_list_shows_the_busy_line(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_busy).with_view_events(nothing),
    );
    assert!(exists(cx, "view-busy"), "the busy line under the header");
}

#[gpui_kit::test]
fn a_list_that_is_not_busy_shows_no_busy_line(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_rows).with_view_events(nothing),
    );
    assert!(!exists(cx, "view-busy"));
}
