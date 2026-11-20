//! Headless tests of the detail and grid views.
//!
//! As in `stack.rs`, module calls run on real worker threads and the test
//! thread hands each reply to the controller, as the daemon does.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, Role, TestAppContext,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreUpdate, DetailView, Effect, GridTile, GridView, Item, ItemId, ItemKind, LocalNow,
    RankWeights, View, ViewEvent, ViewId,
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

fn tiles(count: usize) -> Vec<GridTile> {
    (0..count)
        .map(|ix| GridTile {
            id: format!("t{ix}"),
            image: None,
            caption: format!("Tile {ix}"),
            actions: vec![
                Action {
                    id: "open".into(),
                    title: "Open".into(),
                    shortcut: None,
                },
                Action {
                    id: "copy".into(),
                    title: "Copy".into(),
                    shortcut: None,
                },
            ],
        })
        .collect()
}

fn push_grid() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Grid(GridView {
        id: ViewId("fake/grid".into()),
        title: "Grid".into(),
        columns: 3,
        tiles: tiles(7),
    }))])
}

fn push_detail_with_actions() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Detail(DetailView {
        id: ViewId("fake/detail".into()),
        title: "Detail".into(),
        markdown: "Body".into(),
        metadata: Vec::new(),
        actions: vec![
            Action {
                id: "edit".into(),
                title: "Edit".into(),
                shortcut: None,
            },
            Action {
                id: "copy".into(),
                title: "Copy".into(),
                shortcut: None,
            },
        ],
    }))])
}

fn push_detail() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Detail(DetailView {
        id: ViewId("fake/detail".into()),
        title: "Detail".into(),
        markdown: "# Heading\n\nHello **world** <b>x</b>".into(),
        metadata: vec![
            ("Size".into(), "12 MB".into()),
            ("Kind".into(), "Folder".into()),
        ],
        actions: Vec::new(),
    }))])
}

fn nothing(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
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
    let update: CoreUpdate = core.borrow_mut().apply(ev, LocalNow::now());
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

fn pushed(cx: &mut TestAppContext, module: FakeModule) -> Rc<RefCell<Core>> {
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    core
}

#[gpui_kit::test]
fn detail_shows_markdown_and_metadata(cx: &mut TestAppContext) {
    pushed(cx, FakeModule::new(items(), push_detail));

    assert_eq!(label_of(cx, "view-header").as_deref(), Some("Detail"));
    // The markdown as text, HTML kept literal.
    assert_eq!(
        label_of(cx, "detail-markdown").as_deref(),
        Some("Heading\nHello world <b>x</b>")
    );
    assert_eq!(
        labels_by_role(cx, Role::Row),
        ["Size: 12 MB", "Kind: Folder"]
    );

    press(cx, "escape");
    assert_eq!(label_of(cx, "detail-markdown"), None);
}

/// The tile whose action Enter ran, from the module's events.
fn run_enter(
    cx: &mut TestAppContext,
    core: &Rc<RefCell<Core>>,
    events: &Arc<Mutex<Vec<(ViewId, ViewEvent)>>>,
) -> String {
    press(cx, "enter");
    deliver(cx, core);
    let events = events.lock().unwrap();
    let Some((_, ViewEvent::ActionChosen { row_id, action_id })) = events.last() else {
        panic!("no ActionChosen: {events:?}");
    };
    assert_eq!(action_id, "open");
    row_id.clone()
}

use std::sync::{Arc, Mutex};

#[gpui_kit::test]
fn grid_renders_tiles(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_grid).with_view_events(nothing),
    );
    assert_eq!(label_of(cx, "view-header").as_deref(), Some("Grid"));
    let tiles = labels_by_role(cx, Role::ListItem);
    assert_eq!(tiles.len(), 7);
    assert_eq!(tiles[0], "Tile 0");
}

#[gpui_kit::test]
fn grid_arrow_navigation_2d(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_grid).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    // 3 columns, 7 tiles: right, right, down -> tile 5.
    press(cx, "right");
    press(cx, "right");
    press(cx, "down");
    assert_eq!(run_enter(cx, &core, &events), "t5");
}

#[gpui_kit::test]
fn grid_navigation_stays_inside_the_grid(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_grid).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    // The edges hold: no wrapping to the previous row, no moving off the top.
    press(cx, "left");
    press(cx, "up");
    assert_eq!(run_enter(cx, &core, &events), "t0");

    press(cx, "right");
    press(cx, "right");
    press(cx, "right");
    assert_eq!(run_enter(cx, &core, &events), "t2");

    // Down into the short last row lands on its last tile.
    press(cx, "down");
    press(cx, "down");
    assert_eq!(run_enter(cx, &core, &events), "t6");
    press(cx, "right");
    assert_eq!(run_enter(cx, &core, &events), "t6");
    press(cx, "up");
    press(cx, "up");
    assert_eq!(run_enter(cx, &core, &events), "t0");
}

#[gpui_kit::test]
fn grid_enter_runs_primary_action(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_grid).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    press(cx, "right");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/grid".into()),
            ViewEvent::ActionChosen {
                row_id: "t1".into(),
                action_id: "open".into()
            }
        )]
    );
}

#[gpui_kit::test]
fn grid_enter_is_gated_while_an_action_is_pending(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_grid)
        .with_view_events(nothing)
        .with_delay(Duration::from_millis(100));
    let events = module.view_events.clone();
    // Pushing the grid is itself delayed by the module's `run_action` sleep.
    let core = pushed(cx, module);

    press(cx, "enter");
    press(cx, "enter");
    deliver(cx, &core);
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(events.lock().unwrap().len(), 1, "a double Enter runs once");
}

#[gpui_kit::test]
fn grid_opens_the_action_panel_for_the_selected_tile(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_grid).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    press(cx, "right");
    press(
        cx,
        if cfg!(target_os = "macos") {
            "cmd-k"
        } else {
            "ctrl-k"
        },
    );
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/grid".into()),
            ViewEvent::ActionChosen {
                row_id: "t1".into(),
                action_id: "copy".into()
            }
        )]
    );
}

/// Ruling: a detail view's events carry the view's id as `row_id`.
#[gpui_kit::test]
fn detail_enter_runs_the_primary_action(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_detail_with_actions).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/detail".into()),
            ViewEvent::ActionChosen {
                row_id: "fake/detail".into(),
                action_id: "edit".into()
            }
        )]
    );
}

#[gpui_kit::test]
fn detail_opens_the_action_panel(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_detail_with_actions).with_view_events(nothing);
    let events = module.view_events.clone();
    let core = pushed(cx, module);

    press(
        cx,
        if cfg!(target_os = "macos") {
            "cmd-k"
        } else {
            "ctrl-k"
        },
    );
    press(cx, "down");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/detail".into()),
            ViewEvent::ActionChosen {
                row_id: "fake/detail".into(),
                action_id: "copy".into()
            }
        )]
    );
}

#[gpui_kit::test]
fn detail_without_actions_ignores_enter(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_detail).with_view_events(nothing);
    let events = module.view_events.clone();
    pushed(cx, module);

    press(cx, "enter");
    std::thread::sleep(Duration::from_millis(100));
    assert!(events.lock().unwrap().is_empty());
}
