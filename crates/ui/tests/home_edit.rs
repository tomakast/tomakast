//! Headless tests of the Home edit mode by keyboard and by mouse: getting in and out, the keys,
//! the pointer, reduced motion, and every change written to `config.toml` at once.
//!
//! As in `strip.rs`, module calls run on real worker threads and no gpui task waits on them:
//! the test thread waits for each reply and hands it to the controller, as the daemon's event
//! loop does. Home is read from a temporary `config.toml`, so the file and the strip agree.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, Bounds, KeyDownEvent, Keystroke,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput,
    Point, QuitMode, Role, ScrollDelta, TestAppContext, point, px,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, SeenWidgetEvent, wait_event};
use tomakast_core::{
    Align, ButtonNode, CanvasNode, Color, ColorToken, Core, CoreUpdate, Draw, Edges, Effect, Frame,
    Item, ItemId, ItemKind, Justify, Layout, LocalNow, Node, NodeKind, Paint, Params, RankWeights,
    RectCmd, ShapeStyle, Slot, TextAlign, TextNode, TextStyle, TileKey, WidgetContext, WidgetDecl,
    WidgetEvent, WidgetKey, WidgetSize, read_home,
};
use tomakast_core::{DetailView, View, ViewId};
use tomakast_ui::{LauncherController, LauncherView, ReducedMotionSource, tile_name};

/// How long a poll for a worker's call waits before failing the test.
const POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Ctrl+K, Cmd+K on macOS.
const MENU_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};

/// Ctrl+Z, Cmd+Z on macOS.
const UNDO_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-z"
} else {
    "ctrl-z"
};

/// The chip label of the command modifier.
const COMMAND_CHIP: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl"
};

fn run_action() -> Action {
    Action {
        id: "run".into(),
        title: "Run".into(),
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
        kind: ItemKind::Command,
        source: None,
        actions: vec![run_action()],
    }
}

/// Every action of the fake module asks for the edit mode, as the "Edit Home" command does.
fn edit_home() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::EditHome])
}

/// A widget `id` offering `sizes`, with no parameters.
fn decl_sized(id: &str, sizes: Vec<WidgetSize>) -> WidgetDecl {
    WidgetDecl {
        id: id.into(),
        title: format!("Widget {id}"),
        sizes,
        params: vec![],
    }
}

fn decl(id: &str) -> WidgetDecl {
    decl_sized(id, vec![WidgetSize::Single])
}

fn open_action() -> Action {
    Action {
        id: "open".into(),
        title: "Open".into(),
        shortcut: None,
    }
}

fn edges(all: f32) -> Edges {
    Edges {
        top: all,
        right: all,
        bottom: all,
        left: all,
    }
}

fn layout(align: Align) -> Layout {
    Layout {
        padding: edges(0.),
        gap: 0.,
        align,
        justify: Justify::Start,
    }
}

fn node(id: &str, parent: Option<u32>, kind: NodeKind) -> Node {
    Node {
        id: id.into(),
        parent,
        grow: 0.,
        width: None,
        height: None,
        kind,
    }
}

/// Every widget's frame: a text, with the action `open`, except `plain`, which has no
/// action, `canvas`, which is a canvas `canvas` (node 1) filling the tile, and `button`, a
/// button `go` (node 1) with the action `go`.
fn tile_frame(widget: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    let text = NodeKind::Text(TextNode {
        text: widget.into(),
        style: TextStyle::Body,
        color: Color::Token(ColorToken::Text),
        align: TextAlign::Start,
        max_lines: 1,
        mono: false,
    });
    let canvas = NodeKind::Canvas(CanvasNode {
        focusable: false,
        commands: vec![Draw::Rect(RectCmd {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 50.,
            radius: 0.,
            style: ShapeStyle {
                paint: Paint::Solid(Color::Rgba(0xd02040ff)),
                stroke: None,
                opacity: 1.,
            },
        })],
    });
    let nodes = match widget {
        "canvas" => vec![
            node("", None, NodeKind::Column(layout(Align::Stretch))),
            Node {
                grow: 1.,
                ..node("canvas", Some(0), canvas)
            },
        ],
        // Centred, away from the remove badge in the tile's corner.
        "button" => vec![
            node("", None, NodeKind::Column(layout(Align::Center))),
            node(
                "go",
                Some(0),
                NodeKind::Button(ButtonNode {
                    label: Some("Go".into()),
                    icon: None,
                    action: "go".into(),
                }),
            ),
        ],
        _ => vec![
            node("", None, NodeKind::Column(layout(Align::Start))),
            node("", Some(0), text),
        ],
    };
    Ok(Frame {
        nodes,
        animate: false,
        actions: if widget == "plain" {
            vec![]
        } else {
            vec![open_action()]
        },
        images: vec![],
    })
}

/// The fake module: the "Edit Home" command and a row "Files", and the widgets `a`, `b`,
/// `c`, `plain`, `canvas`, `button` (one size each) and `sized` (quarter, then single).
fn module() -> FakeModule {
    module_with(edit_home)
}

/// [`module`] with every action answered by `result`.
fn module_with(result: fn() -> anyhow::Result<Vec<Effect>>) -> FakeModule {
    FakeModule::new(
        vec![item("edit-home", "Edit Home"), item("files", "Files")],
        result,
    )
    .with_widgets(vec![
        decl("a"),
        decl("b"),
        decl("c"),
        decl("plain"),
        decl("canvas"),
        decl("button"),
        decl_sized("sized", vec![WidgetSize::Quarter, WidgetSize::Single]),
    ])
    .with_render(tile_frame)
}

/// The launcher's core, what its `FakeModule` saw, and its config file.
struct Launcher {
    core: Rc<RefCell<Core>>,
    widget_events: Arc<Mutex<Vec<SeenWidgetEvent>>>,
    config: PathBuf,
    _dir: tempfile::TempDir,
}

impl Launcher {
    fn file(&self) -> String {
        std::fs::read_to_string(&self.config).unwrap()
    }

    /// The widgets of the file's Home list, in order.
    fn file_widgets(&self) -> Vec<String> {
        read_home(&self.config)
            .unwrap()
            .into_iter()
            .map(|entry| entry.widget.widget)
            .collect()
    }

    /// The widgets of the core's Home list, in order.
    fn home(&self) -> Vec<String> {
        self.core
            .borrow()
            .home()
            .iter()
            .map(|entry| entry.widget.widget.clone())
            .collect()
    }
}

/// The text of a config file whose Home is the fake module's `widgets`, in the launcher's own
/// form, tables parted by blank lines.
fn file_of(widgets: &[&str]) -> String {
    widgets
        .iter()
        .map(|id| format!("[[home.widgets]]\nwidget = \"fake/{id}\"\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

/// Initializes the UI over a core with [`module`], whose Home is read from a config file
/// holding `file`, once the module has published its items; then shows the launcher.
fn setup(cx: &mut TestAppContext, file: &str) -> Launcher {
    setup_with(cx, file, module())
}

/// [`setup`] over `module`.
fn setup_with(cx: &mut TestAppContext, file: &str, module: FakeModule) -> Launcher {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, file).unwrap();
    let widget_events = module.widget_events.clone();
    let mut core = new_core();
    core.register(Box::new(module));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    core.set_config_file(config.clone());
    core.home_file_changed(Vec::new());
    let core = Rc::new(RefCell::new(core));
    tomakast_ui::set_raster_helper(env!("CARGO_BIN_EXE_tomakast-raster").into());
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core.clone()));
        // Never the desktop's own setting: the tiles jiggle unless a test says otherwise.
        cx.set_global(ReducedMotionSource(|| None));
    });
    show(cx);
    render(cx);
    Launcher {
        core,
        widget_events,
        config,
        _dir: dir,
    }
}

/// [`setup`] with `widgets` on Home.
fn setup_home(cx: &mut TestAppContext, widgets: &[&str]) -> Launcher {
    setup(cx, &file_of(widgets))
}

/// Hands `update` to the controller, as the daemon's event loop does.
fn hand(cx: &mut TestAppContext, update: CoreUpdate) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_update(update, cx)));
}

/// Applies the core events already queued, without waiting for more.
fn drain(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    loop {
        let Ok(ev) = core.borrow().events().try_recv() else {
            break;
        };
        let update = core.borrow_mut().apply(ev, LocalNow::now());
        hand(cx, update);
    }
    cx.run_until_parked();
}

/// Applies core events as they arrive until `done` holds; panics after 5 s.
fn deliver_until(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>, done: impl Fn(&Core) -> bool) {
    while !done(&core.borrow()) {
        let ev = wait_event(&core.borrow().events());
        let update = core.borrow_mut().apply(ev, LocalNow::now());
        hand(cx, update);
        cx.run_until_parked();
    }
}

/// Applies core events as they arrive until an action's reply was handed on.
fn deliver_reply(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    loop {
        let ev = wait_event(&core.borrow().events());
        let update = core.borrow_mut().apply(ev, LocalNow::now());
        let reply = matches!(update, CoreUpdate::ActionDone { .. });
        hand(cx, update);
        cx.run_until_parked();
        if reply {
            break;
        }
    }
    render(cx);
}

/// Polls `done` until it holds; panics after 5 s.
fn poll(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + POLL_TIMEOUT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} within {POLL_TIMEOUT:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The key of Home tile `slot` of the fake module's widget `widget`.
fn home_key(widget: &str, slot: u32) -> TileKey {
    TileKey {
        widget: WidgetKey::new("fake", widget),
        params: Params::from([]),
        slot: Slot::Home(slot),
    }
}

fn show(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show(cx)));
    cx.run_until_parked();
}

fn hide_launcher(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.hide(cx)));
    cx.run_until_parked();
}

/// How many windows are open: the launcher's, or none once it hid.
fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

/// The launcher is the only window the tests open.
fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "expected exactly the launcher window");
    windows[0]
}

fn view(cx: &mut TestAppContext) -> gpui_kit::Entity<LauncherView> {
    let w = window(cx);
    cx.update(|cx| w.downcast::<LauncherView>().unwrap().root(cx).unwrap())
}

/// Draws a frame, which ticks the frame clock.
fn render(cx: &mut TestAppContext) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Bounds of element `id` in the last frame; `None` when it is not there.
fn bounds(cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> Option<Bounds<Pixels>> {
    let w = window(cx);
    let id = id.into();
    cx.update_window(w, |_, window, _| window.try_find(id).map(|e| e.bounds()))
        .unwrap()
}

/// Element id `id`, a name built at run time.
fn named(id: String) -> gpui_kit::ElementId {
    gpui_kit::ElementId::Name(id.into())
}

/// The label of element `id` in a fresh frame, if it is there.
fn label(cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    let w = window(cx);
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

/// Sends `keys` (space-separated keystrokes) to the window as typed keys, then draws.
fn keys(cx: &mut TestAppContext, keys: &str) {
    let w = window(cx);
    cx.simulate_keystrokes(w, keys);
    render(cx);
}

/// Types `text` as text input, the way a real character arrives.
fn type_text(cx: &mut TestAppContext, text: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
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

fn home_editing(cx: &mut TestAppContext) -> bool {
    let view = view(cx);
    cx.update(|cx| view.read(cx).home_editing())
}

/// The keys of the strip's tiles as last drawn, in order.
fn strip_keys(cx: &mut TestAppContext) -> Vec<TileKey> {
    render(cx);
    let view = view(cx);
    cx.update(|cx| view.read(cx).strip_tile_keys())
}

/// The widgets of the strip's tiles as last drawn, in order.
fn strip_widgets(cx: &mut TestAppContext) -> Vec<String> {
    strip_keys(cx)
        .into_iter()
        .map(|key| key.widget.widget)
        .collect()
}

/// The strip positions of the tiles that show the focus ring.
fn ringed(cx: &mut TestAppContext) -> Vec<usize> {
    let keys = strip_keys(cx);
    keys.iter()
        .enumerate()
        .filter(|(_, key)| bounds(cx, named(format!("widget-focus:{}", tile_name(key)))).is_some())
        .map(|(ix, _)| ix)
        .collect()
}

/// The labels of the open action panel's entries.
fn panel_entries(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::MenuItem)
            .iter()
            .filter_map(|row| row.label().map(str::to_string))
            .collect()
    })
    .unwrap()
}

fn toast(cx: &mut TestAppContext) -> Option<String> {
    label(cx, "launcher-toast")
}

/// The events but `Shown` and `Hidden` the module's tiles received.
fn tile_events(launcher: &Launcher) -> Vec<WidgetEvent> {
    launcher
        .widget_events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, _, event)| event.clone())
        .filter(|event| !matches!(event, WidgetEvent::Shown | WidgetEvent::Hidden))
        .collect()
}

/// Enters the mode from the strip: Tab to the first tile, `rights` times Right, then its
/// panel's "Edit Home".
fn enter_at(cx: &mut TestAppContext, rights: usize) {
    keys(cx, "tab");
    for _ in 0..rights {
        keys(cx, "right");
    }
    keys(cx, MENU_KEY);
    assert_eq!(
        panel_entries(cx).last().map(String::as_str),
        Some("Edit Home")
    );
    type_text(cx, "edit home");
    keys(cx, "enter");
    assert!(home_editing(cx), "the mode is entered");
}

/// Runs the "Edit Home" command from the search, and hands its reply on.
fn run_command(cx: &mut TestAppContext, launcher: &Launcher) {
    type_text(cx, "edit home");
    keys(cx, "enter");
    deliver_reply(cx, &launcher.core);
}

#[gpui_kit::test]
fn edit_home_from_the_panel_enters_the_mode_and_selects_the_tile(cx: &mut TestAppContext) {
    setup_home(cx, &["a", "b", "c"]);
    keys(cx, "tab");
    keys(cx, "right");
    keys(cx, MENU_KEY);
    assert_eq!(
        panel_entries(cx).last().map(String::as_str),
        Some("Edit Home")
    );
    type_text(cx, "edit home");
    assert_eq!(panel_entries(cx), ["Edit Home"]);

    keys(cx, "enter");

    assert!(home_editing(cx));
    assert!(panel_entries(cx).is_empty(), "the panel closes");
    assert_eq!(ringed(cx), [1], "the tile stays selected");
}

/// A Home tile with no actions of its own still has a panel: its "Edit Home".
#[gpui_kit::test]
fn a_home_tile_without_actions_opens_a_panel_with_edit_home(cx: &mut TestAppContext) {
    setup_home(cx, &["plain"]);
    keys(cx, "tab");
    let shortcut = if cfg!(target_os = "macos") {
        "Cmd+K Actions"
    } else {
        "Ctrl+K Actions"
    };
    assert_eq!(label(cx, "footer-actions").as_deref(), Some(shortcut));

    keys(cx, MENU_KEY);

    assert_eq!(panel_entries(cx), ["Edit Home"]);
}

#[gpui_kit::test]
fn the_command_with_an_empty_home_toasts_and_stays_out(cx: &mut TestAppContext) {
    let launcher = setup(cx, "# no Home yet\n");

    run_command(cx, &launcher);

    assert_eq!(
        toast(cx).as_deref(),
        Some("Home is empty: add widgets from the search")
    );
    assert!(!home_editing(cx));
    assert_eq!(query_value(cx), "", "the query was cleared");
}

#[gpui_kit::test]
fn the_command_from_a_typed_query_clears_it_and_enters(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);

    run_command(cx, &launcher);

    assert_eq!(query_value(cx), "");
    assert!(home_editing(cx));
    assert_eq!(ringed(cx), [0], "the first tile is selected");
    assert_eq!(strip_widgets(cx), ["a", "b"], "the strip shows Home");
}

#[gpui_kit::test]
fn arrows_and_tab_move_the_selection_ring(cx: &mut TestAppContext) {
    setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    assert_eq!(ringed(cx), [0]);

    keys(cx, "right");
    assert_eq!(ringed(cx), [1]);
    keys(cx, "tab");
    assert_eq!(ringed(cx), [2]);
    keys(cx, "right");
    assert_eq!(ringed(cx), [2], "it stays at the end");
    keys(cx, "left");
    assert_eq!(ringed(cx), [1]);
    keys(cx, "shift-tab");
    assert_eq!(ringed(cx), [0]);
    keys(cx, "left");
    assert_eq!(ringed(cx), [0]);
    assert!(home_editing(cx));
}

#[gpui_kit::test]
fn shift_right_moves_the_tile_and_writes_the_file(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);

    keys(cx, "shift-right");

    assert_eq!(launcher.file_widgets(), ["b", "a", "c"]);
    assert_eq!(launcher.home(), ["b", "a", "c"]);
    assert_eq!(strip_widgets(cx), ["b", "a", "c"]);
    assert_eq!(ringed(cx), [1], "the selection follows the tile");

    keys(cx, "shift-left");
    assert_eq!(launcher.file_widgets(), ["a", "b", "c"]);
    assert_eq!(ringed(cx), [0]);
    keys(cx, "shift-left");
    assert_eq!(launcher.file_widgets(), ["a", "b", "c"], "nowhere to go");
}

#[gpui_kit::test]
fn delete_removes_writes_and_toasts_with_key_chips(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 1);

    keys(cx, "delete");

    assert_eq!(launcher.file_widgets(), ["a", "c"]);
    assert_eq!(strip_widgets(cx), ["a", "c"]);
    assert_eq!(ringed(cx), [1], "the next tile is selected");
    assert_eq!(
        toast(cx),
        Some(format!("Press {COMMAND_CHIP} Z to restore widget"))
    );
    assert_eq!(
        label(cx, ("toast-key", 1usize)).as_deref(),
        Some(COMMAND_CHIP)
    );
    assert_eq!(label(cx, ("toast-key", 2usize)).as_deref(), Some("Z"));
    assert_eq!(
        label(cx, ("toast-key", 0usize)),
        None,
        "a text part is no chip"
    );

    keys(cx, "backspace");
    assert_eq!(launcher.file_widgets(), ["a"]);
    assert_eq!(ringed(cx), [0], "the last tile, as none comes next");
}

/// Two removals, then two undos: each comes back at its place, last first, and the file is
/// the original byte for byte.
#[gpui_kit::test]
fn ctrl_z_restores_removed_tiles_in_turn_and_the_file_bytes(cx: &mut TestAppContext) {
    let original = "[[home.widgets]]\nwidget = \"fake/a\"\n\n\
                    # the second\n[[home.widgets]]\nwidget = \"fake/b\"\n\n\
                    [[home.widgets]]\nwidget = \"fake/c\"\n";
    let launcher = setup(cx, original);
    enter_at(cx, 1);
    keys(cx, "delete");
    keys(cx, "delete");
    assert_eq!(launcher.file_widgets(), ["a"]);

    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["a", "c"]);
    assert_eq!(strip_widgets(cx), ["a", "c"]);
    assert_eq!(ringed(cx), [1], "the restored tile is selected");

    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["a", "b", "c"]);
    assert_eq!(ringed(cx), [1]);
    assert_eq!(launcher.file(), original);

    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file(), original, "nothing more to restore");
    assert!(home_editing(cx));
}

/// The undo toast goes with the undo: when the mode ends, and when the last removal is
/// restored.
#[gpui_kit::test]
fn the_undo_toast_goes_when_nothing_is_left_to_restore(cx: &mut TestAppContext) {
    let _launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    keys(cx, "delete");
    assert_eq!(
        toast(cx),
        Some(format!("Press {COMMAND_CHIP} Z to restore widget"))
    );

    keys(cx, "escape");

    assert!(!home_editing(cx));
    assert_eq!(toast(cx), None, "no toast offers an undo that is gone");

    enter_at(cx, 0);
    keys(cx, "delete");
    keys(cx, "delete");
    keys(cx, UNDO_KEY);
    assert!(toast(cx).is_some(), "one removal is left to restore");
    keys(cx, UNDO_KEY);
    assert_eq!(toast(cx), None, "nothing is left to restore");
}

/// Every action pushes a detail view.
fn push_detail() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Detail(DetailView {
        id: ViewId("fake/detail".into()),
        title: "Detail".into(),
        markdown: "Body".into(),
        metadata: Vec::new(),
        actions: Vec::new(),
    }))])
}

/// Every action puts "files" in the search field.
fn set_query() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::SetRootQuery("files".into())])
}

/// A root action's reply that comes while the mode lasts, its tile menu open: a pushed view
/// or a new query ends the mode, and the menu with it.
#[gpui_kit::test]
fn a_late_reply_that_pushes_a_view_or_sets_the_query_leaves_the_mode(cx: &mut TestAppContext) {
    for result in [push_detail as fn() -> _, set_query] {
        let launcher = setup_with(cx, &file_of(&["a", "b"]), module_with(result));
        keys(cx, "enter");
        enter_at(cx, 0);
        keys(cx, MENU_KEY);
        assert_eq!(panel_entries(cx), ["Remove"]);

        deliver_reply(cx, &launcher.core);

        assert!(!home_editing(cx), "the reply left the mode");
        assert!(panel_entries(cx).is_empty(), "the tile menu went with it");
        assert_eq!(launcher.file_widgets(), ["a", "b"]);
        hide_launcher(cx);
    }
}

#[gpui_kit::test]
fn removing_the_second_of_two_identical_tiles_removes_it(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "a"]);
    enter_at(cx, 2);
    assert_eq!(ringed(cx), [2]);

    keys(cx, "delete");

    assert_eq!(launcher.file_widgets(), ["a", "b"]);
    assert_eq!(strip_keys(cx), [home_key("a", 0), home_key("b", 0)]);
}

#[gpui_kit::test]
fn ctrl_k_lists_sizes_with_the_current_marked_and_resizes(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["sized", "a"]);
    enter_at(cx, 0);

    keys(cx, MENU_KEY);
    assert_eq!(panel_entries(cx), ["Quarter", "Single", "Remove"]);
    assert_eq!(
        label(cx, ("action-current", 0usize)).as_deref(),
        Some("Current size")
    );
    assert_eq!(label(cx, ("action-current", 1usize)), None);

    keys(cx, "down");
    keys(cx, "enter");

    assert!(panel_entries(cx).is_empty(), "the panel closes");
    assert!(home_editing(cx), "back in the mode");
    assert_eq!(
        launcher.core.borrow().home()[0].size,
        Some(WidgetSize::Single)
    );
    assert!(
        launcher.file().contains("size = \"single\""),
        "{}",
        launcher.file()
    );
    assert_eq!(ringed(cx), [0]);

    // Choosing the current size changes nothing.
    let file = launcher.file();
    keys(cx, MENU_KEY);
    assert_eq!(
        label(cx, ("action-current", 1usize)).as_deref(),
        Some("Current size")
    );
    keys(cx, "down");
    keys(cx, "enter");
    assert!(panel_entries(cx).is_empty());
    assert_eq!(launcher.file(), file);

    // A widget of one size lists only Remove, which removes it.
    keys(cx, "right");
    keys(cx, MENU_KEY);
    assert_eq!(panel_entries(cx), ["Remove"]);
    keys(cx, "enter");
    assert_eq!(launcher.file_widgets(), ["sized"]);
    assert!(home_editing(cx));
}

/// Esc closes the tile menu back into the mode, then leaves the mode; the launcher never
/// hides on the way.
#[gpui_kit::test]
fn escape_closes_the_menu_then_leaves_the_mode_and_never_hides(cx: &mut TestAppContext) {
    setup_home(cx, &["a", "b"]);
    enter_at(cx, 0);
    keys(cx, MENU_KEY);
    assert_eq!(panel_entries(cx), ["Remove"]);

    keys(cx, "escape");
    assert_eq!(windows(cx), 1, "the launcher stays");
    assert!(panel_entries(cx).is_empty());
    assert!(home_editing(cx));
    keys(cx, "right");
    assert_eq!(ringed(cx), [1], "the mode has the keys again");

    keys(cx, "escape");
    assert_eq!(windows(cx), 1, "the launcher stays");
    assert!(!home_editing(cx));
    type_text(cx, "f");
    assert_eq!(query_value(cx), "f", "the field has the keyboard back");
}

#[gpui_kit::test]
fn enter_leaves_and_the_strip_selection_stays_on_the_tile(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    keys(cx, "right");

    keys(cx, "enter");

    assert!(!home_editing(cx));
    assert_eq!(ringed(cx), [1]);
    assert!(tile_events(&launcher).is_empty(), "Enter ran nothing");
    keys(cx, "right");
    assert_eq!(ringed(cx), [2], "the strip's own keys again");
}

#[gpui_kit::test]
fn hiding_the_launcher_leaves_the_mode(cx: &mut TestAppContext) {
    setup_home(cx, &["a", "b"]);
    enter_at(cx, 0);
    let view = view(cx);

    hide_launcher(cx);

    assert!(!cx.update(|cx| view.read(cx).home_editing()));
    show(cx);
    assert!(!home_editing(cx));
}

/// A stray key neither types, nor leaves the mode, nor reaches a tile.
#[gpui_kit::test]
fn typing_in_the_mode_does_nothing(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    deliver_until(cx, &launcher.core, |core| {
        core.widget_frame(&home_key("a", 0)).is_some()
    });
    enter_at(cx, 0);

    for key in ["a", "1", "space", "up", "down"] {
        keys(cx, key);
    }
    type_text(cx, "x y");
    render(cx);

    assert_eq!(query_value(cx), "");
    assert!(home_editing(cx));
    assert_eq!(ringed(cx), [0]);
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert!(
        tile_events(&launcher).is_empty(),
        "{:?}",
        tile_events(&launcher)
    );
}

#[gpui_kit::test]
fn tiles_take_no_click_or_pointer_in_the_mode(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["canvas", "a"]);
    deliver_until(cx, &launcher.core, |core| {
        ["canvas", "a"]
            .iter()
            .all(|id| core.widget_frame(&home_key(id, 0)).is_some())
    });
    render(cx);
    enter_at(cx, 0);
    let tile = format!("widget-tile:{}", tile_name(&home_key("a", 0)));
    let canvas = format!("widget-node:{}:1", tile_name(&home_key("canvas", 0)));
    let at = bounds(cx, named(canvas)).expect("the canvas").origin;

    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.click(named(tile.clone()), cx))
        .unwrap();
    cx.run_until_parked();
    for x in [10., 20., 30.] {
        cx.update_window(w, |_, window, cx| {
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: at + point(px(x), px(5.)),
                    pressed_button: None,
                    modifiers: gpui_kit::Modifiers::default(),
                }),
                cx,
            );
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    // The queued moves go out on the next frame's tick.
    cx.update_window(w, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    render(cx);

    std::thread::sleep(Duration::from_millis(50));
    drain(cx, &launcher.core);
    assert!(
        tile_events(&launcher).is_empty(),
        "{:?}",
        tile_events(&launcher)
    );
    assert!(home_editing(cx));

    // Out of the mode, the same pointer moves reach the canvas: they were the mode's.
    keys(cx, "escape");
    for x in [12., 22.] {
        cx.update_window(w, |_, window, cx| {
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: at + point(px(x), px(6.)),
                    pressed_button: None,
                    modifiers: gpui_kit::Modifiers::default(),
                }),
                cx,
            );
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(w, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    render(cx);
    poll("a pointer event out of the mode", || {
        launcher
            .widget_events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, _, event)| matches!(event, WidgetEvent::Pointer(_)))
    });
}

/// An undo refused because the file changed by hand keeps its entry: the refusal re-reads the
/// file, so the strip shows its list at once, and the next undo restores the entry.
#[gpui_kit::test]
fn an_edit_of_a_changed_file_is_refused_and_keeps_the_undo(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    keys(cx, "delete");
    assert_eq!(launcher.file_widgets(), ["b", "c"]);
    std::fs::write(&launcher.config, file_of(&["c", "b"])).unwrap();

    keys(cx, UNDO_KEY);

    assert_eq!(toast(cx).as_deref(), Some("config.toml changed: try again"));
    assert_eq!(launcher.file_widgets(), ["c", "b"], "nothing written");
    assert!(home_editing(cx));

    // No watcher runs in the tests: the refusal itself re-read the file.
    assert_eq!(launcher.home(), ["c", "b"]);
    assert_eq!(strip_widgets(cx), ["c", "b"]);
    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["a", "c", "b"]);
    assert_eq!(ringed(cx), [0]);
}

#[gpui_kit::test]
fn removing_the_last_tile_then_leaving_removes_the_strip(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a"]);
    enter_at(cx, 0);

    keys(cx, "delete");
    assert!(launcher.file_widgets().is_empty());
    assert!(home_editing(cx), "an empty Home stays in the mode");
    assert!(ringed(cx).is_empty());

    // The keys still reach the mode with no tile left.
    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["a"]);
    assert_eq!(ringed(cx), [0]);
    keys(cx, "delete");
    assert!(launcher.file_widgets().is_empty());

    keys(cx, "escape");

    assert!(!home_editing(cx));
    assert_eq!(windows(cx), 1);
    assert!(bounds(cx, "widget-strip").is_none(), "no strip");
    type_text(cx, "f");
    assert_eq!(query_value(cx), "f");
}

#[gpui_kit::test]
fn the_footer_reads_edit_home_and_done(cx: &mut TestAppContext) {
    setup_home(cx, &["a"]);
    assert_eq!(label(cx, "footer-context").as_deref(), Some("tomakast"));
    enter_at(cx, 0);

    assert_eq!(label(cx, "footer-context").as_deref(), Some("Edit Home"));
    assert_eq!(label(cx, "footer-primary").as_deref(), Some("⏎ Done"));
    let shortcut = if cfg!(target_os = "macos") {
        "Cmd+K Actions"
    } else {
        "Ctrl+K Actions"
    };
    assert_eq!(label(cx, "footer-actions").as_deref(), Some(shortcut));
}

/// The edit mode draws a remove badge on each tile.
#[gpui_kit::test]
fn each_tile_has_a_remove_badge_in_the_mode(cx: &mut TestAppContext) {
    setup_home(cx, &["a", "b"]);
    let badge = |key: &TileKey| format!("home-edit-remove:{}", tile_name(key));
    assert_eq!(label(cx, named(badge(&home_key("a", 0)))), None);

    enter_at(cx, 0);

    assert_eq!(
        label(cx, named(badge(&home_key("a", 0)))).as_deref(),
        Some("Remove Widget a")
    );
    assert_eq!(
        label(cx, named(badge(&home_key("b", 0)))).as_deref(),
        Some("Remove Widget b")
    );
}

/// An entry that cannot show (its plugin is not running) between two tiles: the selection, a
/// move, a removal and its undo name Home entries, not strip positions.
#[gpui_kit::test]
fn an_entry_that_cannot_show_between_tiles_keeps_its_place(cx: &mut TestAppContext) {
    let file = "[[home.widgets]]\nwidget = \"fake/a\"\n\n\
                [[home.widgets]]\nwidget = \"ghost/x\"\n\n\
                [[home.widgets]]\nwidget = \"fake/b\"\n";
    let launcher = setup(cx, file);
    assert_eq!(strip_widgets(cx), ["a", "b"]);
    enter_at(cx, 0);

    keys(cx, "shift-right");
    assert_eq!(launcher.file_widgets(), ["x", "b", "a"]);
    assert_eq!(strip_widgets(cx), ["b", "a"]);
    assert_eq!(ringed(cx), [1], "the selection follows the tile");

    keys(cx, "delete");
    assert_eq!(launcher.file_widgets(), ["x", "b"]);
    assert_eq!(ringed(cx), [0]);

    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["x", "b", "a"]);
    assert_eq!(ringed(cx), [1]);

    keys(cx, "left");
    keys(cx, "shift-right");
    assert_eq!(launcher.file_widgets(), ["x", "a", "b"]);
    assert_eq!(ringed(cx), [1]);
}

/// Element id of `key`'s tile.
fn tile_id(key: &TileKey) -> gpui_kit::ElementId {
    named(format!("widget-tile:{}", tile_name(key)))
}

/// Element id of `key`'s remove badge.
fn badge_id(key: &TileKey) -> gpui_kit::ElementId {
    named(format!("home-edit-remove:{}", tile_name(key)))
}

/// The centre of element `id` in a fresh frame.
fn centre(cx: &mut TestAppContext, id: gpui_kit::ElementId) -> Point<Pixels> {
    render(cx);
    bounds(cx, id).expect("the element is drawn").center()
}

/// Dispatches `event` to the window, then draws.
fn dispatch(cx: &mut TestAppContext, event: PlatformInput) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.dispatch_event(event, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn mouse_move(cx: &mut TestAppContext, at: Point<Pixels>, pressed: Option<MouseButton>) {
    dispatch(
        cx,
        PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: pressed,
            modifiers: Modifiers::default(),
        }),
    );
}

fn mouse_down(cx: &mut TestAppContext, at: Point<Pixels>, button: MouseButton) {
    dispatch(
        cx,
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
    );
}

fn mouse_up(cx: &mut TestAppContext, at: Point<Pixels>, button: MouseButton) {
    dispatch(
        cx,
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
    );
}

/// Moves the pointer to `at`, then clicks `button` there.
fn click_at(cx: &mut TestAppContext, at: Point<Pixels>, button: MouseButton) {
    mouse_move(cx, at, None);
    mouse_down(cx, at, button);
    mouse_up(cx, at, button);
}

/// Lets `ms` pass on the executor's clock, runs what became due, and draws.
fn wait_ms(cx: &mut TestAppContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
    render(cx);
}

/// One round of the frame clock: draws, lets 17 ms pass, and delivers the next animation frame.
/// Returns how many frame callbacks ran: none when nothing asked for a frame.
fn next_frame(cx: &mut TestAppContext) -> usize {
    render(cx);
    cx.executor().advance_clock(Duration::from_millis(17));
    let w = window(cx);
    let ran = cx
        .update_window(w, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    ran
}

/// Waits until the module drew a frame for each of the Home tiles of `widgets`.
fn wait_frames(cx: &mut TestAppContext, launcher: &Launcher, widgets: &[&str]) {
    deliver_until(cx, &launcher.core, |core| {
        widgets
            .iter()
            .all(|id| core.widget_frame(&home_key(id, 0)).is_some())
    });
    render(cx);
}

/// Whether a tile received an action: a click ran its first action.
fn tile_ran_action(launcher: &Launcher) -> bool {
    tile_events(launcher)
        .iter()
        .any(|event| matches!(event, WidgetEvent::Action { .. }))
}

#[gpui_kit::test]
fn right_click_opens_the_panel_with_edit_home(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);

    let at = centre(cx, tile_id(&home_key("b", 0)));
    click_at(cx, at, MouseButton::Right);

    assert_eq!(panel_entries(cx), ["Open", "Edit Home"]);
    assert_eq!(ringed(cx), [1], "the strip focus moved to the tile");
    assert!(!home_editing(cx));
    assert!(!tile_ran_action(&launcher), "a right-click runs nothing");

    // Its "Edit Home" enters the mode on that tile.
    type_text(cx, "edit home");
    keys(cx, "enter");
    assert!(home_editing(cx));
    assert_eq!(ringed(cx), [1]);
}

#[gpui_kit::test]
fn right_click_in_the_mode_opens_the_tile_menu(cx: &mut TestAppContext) {
    setup_home(cx, &["sized", "a"]);
    enter_at(cx, 1);

    let at = centre(cx, tile_id(&home_key("sized", 0)));
    click_at(cx, at, MouseButton::Right);

    assert_eq!(panel_entries(cx), ["Quarter", "Single", "Remove"]);
    assert_eq!(ringed(cx), [0], "the tile is selected");
    assert!(home_editing(cx));
    keys(cx, "escape");
    assert!(panel_entries(cx).is_empty());
    assert!(home_editing(cx), "back in the mode");
}

/// Element id of row `ix` of the open action panel.
fn panel_row(ix: usize) -> gpui_kit::ElementId {
    ("action", ix).into()
}

/// In the mode, the tile menu's "Remove" clicked removes the tile, and the mode stays with
/// its undo.
#[gpui_kit::test]
fn a_click_on_remove_in_the_tile_menu_removes_the_tile(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    let at = centre(cx, tile_id(&home_key("b", 0)));
    click_at(cx, at, MouseButton::Right);
    assert_eq!(panel_entries(cx), ["Remove"]);

    let at = centre(cx, panel_row(0));
    click_at(cx, at, MouseButton::Left);

    assert_eq!(launcher.file_widgets(), ["a", "c"]);
    assert_eq!(strip_widgets(cx), ["a", "c"]);
    assert!(panel_entries(cx).is_empty(), "the menu closes");
    assert!(home_editing(cx), "the click did not leave the mode");
    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["a", "b", "c"]);
}

/// Out of the mode, a Home tile's panel's "Edit Home" clicked enters the mode on that tile.
#[gpui_kit::test]
fn a_click_on_edit_home_in_the_panel_enters_the_mode(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let at = centre(cx, tile_id(&home_key("b", 0)));
    click_at(cx, at, MouseButton::Right);
    assert_eq!(panel_entries(cx), ["Open", "Edit Home"]);

    let at = centre(cx, panel_row(1));
    click_at(cx, at, MouseButton::Left);

    assert!(home_editing(cx));
    assert!(panel_entries(cx).is_empty());
    assert_eq!(ringed(cx), [1], "on the tile the panel was for");
    assert!(!tile_ran_action(&launcher));
}

#[gpui_kit::test]
fn a_long_press_enters_the_mode_and_its_release_clicks_nothing(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let at = centre(cx, tile_id(&home_key("b", 0)));
    mouse_move(cx, at, None);
    mouse_down(cx, at, MouseButton::Left);

    wait_ms(cx, 499);
    assert!(!home_editing(cx), "not yet");
    wait_ms(cx, 1);
    assert!(home_editing(cx), "the press entered the mode");
    assert_eq!(ringed(cx), [1], "on the pressed tile");

    mouse_up(cx, at, MouseButton::Left);

    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert!(!tile_ran_action(&launcher), "{:?}", tile_events(&launcher));
    assert!(home_editing(cx), "the release did not leave the mode");
    assert_eq!(ringed(cx), [1]);
}

#[gpui_kit::test]
fn a_short_press_is_a_click(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let at = centre(cx, tile_id(&home_key("a", 0)));

    mouse_move(cx, at, None);
    mouse_down(cx, at, MouseButton::Left);
    wait_ms(cx, 200);
    mouse_up(cx, at, MouseButton::Left);
    wait_ms(cx, 1000);

    assert!(!home_editing(cx), "the release cancelled the long press");
    poll("the tile's action", || tile_ran_action(&launcher));

    // A press that moves away is no long press either.
    mouse_down(cx, at, MouseButton::Left);
    mouse_move(cx, at + point(px(10.), px(0.)), Some(MouseButton::Left));
    wait_ms(cx, 1000);
    assert!(!home_editing(cx), "the move cancelled the long press");
    mouse_up(cx, at + point(px(10.), px(0.)), MouseButton::Left);
}

#[gpui_kit::test]
fn the_remove_badge_removes_the_tile(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);

    let at = centre(cx, badge_id(&home_key("b", 0)));
    click_at(cx, at, MouseButton::Left);

    assert_eq!(launcher.file_widgets(), ["a", "c"]);
    assert_eq!(strip_widgets(cx), ["a", "c"]);
    assert_eq!(ringed(cx), [1], "the next tile is selected");
    assert_eq!(
        toast(cx),
        Some(format!("Press {COMMAND_CHIP} Z to restore widget"))
    );
    assert!(home_editing(cx));

    keys(cx, UNDO_KEY);
    assert_eq!(launcher.file_widgets(), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn dragging_a_tile_over_the_next_moves_it_once(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    let original = launcher.file();
    let from = centre(cx, tile_id(&home_key("a", 0)));
    // A little past `b`'s centre, whatever the jiggle.
    let to = centre(cx, tile_id(&home_key("b", 0))) + point(px(10.), px(0.));
    click_at(cx, to, MouseButton::Left);
    assert_eq!(ringed(cx), [1], "a press selects its tile");
    assert_eq!(launcher.file(), original, "a click is no drag");

    mouse_move(cx, from, None);
    mouse_down(cx, from, MouseButton::Left);
    assert_eq!(ringed(cx), [0]);
    let start = centre(cx, tile_id(&home_key("a", 0)));
    for step in 1..=8 {
        let x = from.x + (to.x - from.x) * (step as f32 / 8.);
        mouse_move(cx, point(x, from.y), Some(MouseButton::Left));
        assert_eq!(launcher.file(), original, "nothing is written mid-drag");
    }
    // The dragged tile follows the pointer, within a jiggle's width.
    let dragged = centre(cx, tile_id(&home_key("a", 0)));
    assert!(
        (dragged.x - start.x - (to.x - from.x)).abs() <= px(2.),
        "{dragged:?} from {start:?}"
    );
    assert!((dragged.y - start.y).abs() <= px(2.), "only sideways");
    assert_eq!(
        strip_widgets(cx),
        ["a", "b", "c"],
        "the order holds mid-drag"
    );
    mouse_up(cx, to, MouseButton::Left);

    assert_eq!(launcher.file(), file_of(&["b", "a", "c"]), "one move");
    assert_eq!(launcher.home(), ["b", "a", "c"]);
    assert_eq!(strip_widgets(cx), ["b", "a", "c"]);
    assert_eq!(ringed(cx), [1], "the selection follows the tile");
    assert!(home_editing(cx));

    // A drag too short to pass a neighbour's centre writes nothing.
    let moved = launcher.file();
    let at = centre(cx, tile_id(&home_key("a", 0)));
    mouse_move(cx, at, None);
    mouse_down(cx, at, MouseButton::Left);
    for dx in [5., 20., 40.] {
        mouse_move(cx, at + point(px(dx), px(0.)), Some(MouseButton::Left));
    }
    mouse_up(cx, at + point(px(40.), px(0.)), MouseButton::Left);
    assert_eq!(launcher.file(), moved);
    assert_eq!(strip_widgets(cx), ["b", "a", "c"]);
}

/// A drag whose button release was lost (it went up outside the window): the next move with
/// no button down drops the drag where it began, and writes nothing.
#[gpui_kit::test]
fn a_drag_without_its_release_is_dropped_on_the_next_move(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    let original = launcher.file();
    let from = centre(cx, tile_id(&home_key("a", 0)));
    let to = centre(cx, tile_id(&home_key("b", 0))) + point(px(10.), px(0.));
    mouse_move(cx, from, None);
    mouse_down(cx, from, MouseButton::Left);
    for step in 1..=4 {
        let x = from.x + (to.x - from.x) * (step as f32 / 4.);
        mouse_move(cx, point(x, from.y), Some(MouseButton::Left));
    }

    mouse_move(cx, to + point(px(5.), px(0.)), None);

    let at = centre(cx, tile_id(&home_key("a", 0)));
    assert!((at.x - from.x).abs() <= px(2.), "back in its place: {at:?}");
    mouse_up(cx, to, MouseButton::Left);
    assert_eq!(launcher.file(), original, "nothing written");
    assert_eq!(strip_widgets(cx), ["a", "b", "c"]);
    assert!(home_editing(cx));
}

/// The strip's scrollbar works in the mode: a press on its track, below the tiles, scrolls
/// the strip and does not leave the mode.
#[gpui_kit::test]
fn the_strip_scrollbar_scrolls_in_the_mode(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    let strip = bounds(cx, "widget-strip").expect("the strip");
    let c = |cx: &mut TestAppContext| bounds(cx, tile_id(&home_key("c", 0))).unwrap().origin.x;
    // The bar shows while the strip scrolls.
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.scroll(
            "widget-strip",
            ScrollDelta::Pixels(point(px(0.), px(-1.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    let before = c(cx);

    // Below the tiles, near the right end of the track.
    let track = point(strip.right() - px(20.), strip.bottom() - px(4.));
    click_at(cx, track, MouseButton::Left);

    assert!(
        home_editing(cx),
        "the press on the bar did not leave the mode"
    );
    assert!(c(cx) < before - px(100.), "the strip scrolled");
    assert_eq!(launcher.file_widgets(), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn a_click_on_the_list_leaves_the_mode_and_runs_nothing(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let strip = bounds(cx, "widget-strip").expect("the strip");
    let list = point(strip.center().x, strip.bottom() + px(40.));
    enter_at(cx, 0);

    click_at(cx, list, MouseButton::Left);

    assert!(!home_editing(cx), "the click left the mode");
    assert_eq!(query_value(cx), "");
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert!(!home_editing(cx), "no row ran: every row asks for the mode");
    assert!(tile_events(&launcher).is_empty());

    // The search field, the footer and the strip's own background leave too.
    let field = centre(cx, "launcher-input".into());
    let footer = centre(cx, "footer-context".into());
    let background = strip.origin + point(px(2.), px(2.));
    for at in [field, footer, background] {
        enter_at(cx, 0);
        click_at(cx, at, MouseButton::Left);
        assert!(!home_editing(cx), "a click at {at:?} left the mode");
    }
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert!(tile_events(&launcher).is_empty());
    assert!(!home_editing(cx));
}

#[gpui_kit::test]
fn reduced_motion_draws_a_dashed_outline_and_requests_no_frames(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let outline = |key: &TileKey| named(format!("home-edit-outline:{}", tile_name(key)));
    // The frames the launcher asks for anyway, outside the mode, once it is still.
    keys(cx, "tab");
    next_frame(cx);
    let still = next_frame(cx);

    // With motion, the tiles jiggle: the mode asks for frames, and draws no outline.
    enter_at(cx, 0);
    cx.run_until_parked();
    next_frame(cx);
    assert!(next_frame(cx) > still, "the jiggle asks for frames");
    assert_eq!(bounds(cx, outline(&home_key("a", 0))), None);
    let at = bounds(cx, tile_id(&home_key("b", 0)));
    wait_ms(cx, 75);
    assert_ne!(
        bounds(cx, tile_id(&home_key("b", 0))),
        at,
        "the tile jiggles"
    );
    keys(cx, "escape");

    cx.update(|cx| cx.set_global(ReducedMotionSource(|| Some(true))));
    enter_at(cx, 0);
    cx.run_until_parked();
    render(cx);

    assert!(bounds(cx, outline(&home_key("a", 0))).is_some());
    assert!(bounds(cx, outline(&home_key("b", 0))).is_some());
    // The frame asked for before the answer came may still run; none after it.
    next_frame(cx);
    assert_eq!(next_frame(cx), still, "the mode asks for no frame");
    assert_eq!(next_frame(cx), still);
    let at = bounds(cx, tile_id(&home_key("b", 0)));
    // A quarter and three quarters of a jiggle later, the tile is where it was.
    wait_ms(cx, 75);
    assert_eq!(
        bounds(cx, tile_id(&home_key("b", 0))),
        at,
        "nothing jiggles"
    );
    wait_ms(cx, 150);
    assert_eq!(bounds(cx, tile_id(&home_key("b", 0))), at);
}

/// With reduced motion known from an earlier entry, the mode draws still from its first
/// frame, before its own answer comes.
#[gpui_kit::test]
fn reduced_motion_known_from_before_holds_from_the_first_frame(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let outline = named(format!(
        "home-edit-outline:{}",
        tile_name(&home_key("a", 0))
    ));
    cx.update(|cx| cx.set_global(ReducedMotionSource(|| Some(true))));
    enter_at(cx, 0);
    cx.run_until_parked();
    assert!(bounds(cx, outline.clone()).is_some());
    keys(cx, "escape");

    // Up to the "Edit Home" of the tile's panel, then its Enter and a frame, with no task
    // run in between: the answer of this entry has not come.
    keys(cx, MENU_KEY);
    type_text(cx, "edit home");
    let w = window(cx);
    let first = cx
        .update_window(w, |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
            window.render_frame(cx);
            window.try_find(outline.clone()).is_some()
        })
        .unwrap();

    assert!(home_editing(cx));
    assert!(first, "the first frame draws the outline");
}

/// A held Delete repeats, but the mode takes only its first press: one tile goes.
#[gpui_kit::test]
fn a_held_delete_removes_one_tile(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);

    for is_held in [false, true, true, true] {
        dispatch(
            cx,
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: Keystroke::parse("delete").unwrap(),
                is_held,
                prefer_character_input: false,
            }),
        );
    }

    assert_eq!(launcher.file_widgets(), ["b", "c"]);
    assert_eq!(strip_widgets(cx), ["b", "c"]);
    assert!(home_editing(cx));
}

/// A tile's button presses nothing in the mode; out of it, the same click presses it.
#[gpui_kit::test]
fn a_tile_button_presses_nothing_in_the_mode(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["button", "a"]);
    wait_frames(cx, &launcher, &["button", "a"]);
    let button = named(format!(
        "widget-node:{}:1",
        tile_name(&home_key("button", 0))
    ));
    let at = centre(cx, button);
    enter_at(cx, 1);

    click_at(cx, at, MouseButton::Left);

    assert!(home_editing(cx));
    assert_eq!(ringed(cx), [0], "the click selected the tile");
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert!(!tile_ran_action(&launcher), "{:?}", tile_events(&launcher));

    assert_eq!(launcher.file_widgets(), ["button", "a"], "nothing removed");

    keys(cx, "escape");
    click_at(cx, at, MouseButton::Left);
    poll("the button's action out of the mode", || {
        tile_events(&launcher).iter().any(|event| {
            matches!(event, WidgetEvent::Action { node: Some(node), action } if node == "go" && action == "go")
        })
    });
}

/// A tile dragged to the strip's edge scrolls the strip, a step each frame, and lands where
/// the scrolled strip puts it: at the end, once the strip scrolled as far as it goes.
#[gpui_kit::test]
fn a_drag_at_the_strip_edge_scrolls_it(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b", "c"]);
    enter_at(cx, 0);
    let strip = bounds(cx, "widget-strip").expect("the strip");
    let from = centre(cx, tile_id(&home_key("a", 0)));
    let edge = point(strip.right() - px(10.), from.y);
    let c = |cx: &mut TestAppContext| bounds(cx, tile_id(&home_key("c", 0))).unwrap().origin.x;
    let before = c(cx);

    mouse_move(cx, from, None);
    mouse_down(cx, from, MouseButton::Left);
    mouse_move(cx, edge, Some(MouseButton::Left));
    for _ in 0..3 {
        next_frame(cx);
    }

    let scrolled = before - c(cx);
    assert!(scrolled >= px(16.), "the strip scrolled by {scrolled:?}");
    // 375 px of scroll in all, 8 px a frame.
    for _ in 0..50 {
        next_frame(cx);
    }
    mouse_up(cx, edge, MouseButton::Left);
    assert_eq!(launcher.file_widgets(), ["b", "c", "a"]);
}

/// A quick click whose press and release come before the next frame is drawn is a click, not
/// the start of a long press that fires later.
#[gpui_kit::test]
fn a_click_within_one_frame_starts_no_long_press(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let at = centre(cx, tile_id(&home_key("a", 0)));
    mouse_move(cx, at, None);

    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        for event in [
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
        ] {
            window.dispatch_event(event, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    wait_ms(cx, 1000);

    assert!(!home_editing(cx), "the click was no long press");
    poll("the tile's action", || tile_ran_action(&launcher));
}

/// Hiding the launcher mid-press cancels the long press.
#[gpui_kit::test]
fn hiding_cancels_a_long_press(cx: &mut TestAppContext) {
    let launcher = setup_home(cx, &["a", "b"]);
    wait_frames(cx, &launcher, &["a", "b"]);
    let at = centre(cx, tile_id(&home_key("a", 0)));
    mouse_move(cx, at, None);
    mouse_down(cx, at, MouseButton::Left);

    hide_launcher(cx);
    show(cx);
    wait_ms(cx, 1000);

    assert!(!home_editing(cx));
}
