//! Headless tests of the widget strip: where its tiles sit, how it scrolls,
//! what a reopened launcher draws, and the frame clock.
//!
//! As in `launcher_view.rs`, module calls run on real worker threads and no
//! gpui task waits on them: the test thread waits for each reply and hands
//! it to the controller, as the daemon's event loop does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, Bounds, Pixels, QuitMode, Role,
    ScrollDelta, TestAppContext, point, px,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::strip::StoppedWidget;
use tomakast_core::testing::{FakeModule, SeenWidgetEvent, wait_event};
use tomakast_core::{
    Align, ButtonNode, CanvasNode, Color, ColorToken, Core, CoreUpdate, Draw, Edges, Effect,
    EllipseCmd, Frame, Icon, IconNode, ImageFit, ImageFormat, ImageNode, Item, ItemId, ItemKind,
    Justify, Layout, LocalNow, MAX_FRAME_COMMANDS, Module, ModuleContext, Node, NodeKind, Paint,
    PathCmd, PathSegment, PointerKind, RankWeights, RectCmd, RequestId, SearchAnswer, ShapeStyle,
    TextAlign, TextCmd, TextNode, TextStyle, Transform, WidgetAnswer, WidgetContext, WidgetDecl,
    WidgetEvent, WidgetImage, WidgetSize,
};
use tomakast_ui::{CanvasPaintBudget, LauncherController, LauncherView};

#[path = "support/font_recorder.rs"]
mod font_recorder;
use font_recorder::{FontRecorder, families_of, features_of};

/// The text of [`frame`]'s text node, its label.
const FRAME_TEXT: &str = "hello";
/// The widgets of [`three_tiles`], in strip order.
const WIDGETS: [&str; 3] = ["a", "b", "c"];
/// How long a poll for a worker's call waits before failing the test.
const POLL_TIMEOUT: Duration = Duration::from_secs(5);

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: None,
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

fn items() -> Vec<Item> {
    vec![item("firefox", "Firefox"), item("files", "Files")]
}

fn nothing() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

/// A column holding one text node: two nodes.
fn frame(animate: bool) -> Frame {
    let layout = Layout {
        padding: Edges {
            top: 0.,
            right: 0.,
            bottom: 0.,
            left: 0.,
        },
        gap: 0.,
        align: Align::Start,
        justify: Justify::Start,
    };
    let node = |parent, kind| Node {
        id: String::new(),
        parent,
        grow: 0.,
        width: None,
        height: None,
        kind,
    };
    Frame {
        nodes: vec![
            node(None, NodeKind::Column(layout)),
            node(
                Some(0),
                NodeKind::Text(TextNode {
                    text: "hello".into(),
                    style: TextStyle::Body,
                    color: Color::Token(ColorToken::Text),
                    align: TextAlign::Start,
                    max_lines: 1,
                    mono: false,
                }),
            ),
        ],
        animate,
        actions: vec![],
        images: vec![],
    }
}

fn still(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    Ok(frame(false))
}

/// Whether [`animated`] draws frames that animate; only
/// `an_animating_tile_asks_for_the_next_frame` changes it.
static ANIMATE: AtomicBool = AtomicBool::new(true);

fn animated(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    Ok(frame(ANIMATE.load(Ordering::SeqCst)))
}

/// Whether the `swap` widget draws a canvas; only
/// `a_canvas_that_leaves_under_the_pointer_gets_leave` changes it.
static SWAP_CANVAS: AtomicBool = AtomicBool::new(true);

/// While set, [`held_shown`] holds the module's worker on `Shown`; only
/// `reopen_draws_the_cached_frame_before_any_render` sets it.
static HOLD_SHOWN: AtomicBool = AtomicBool::new(false);

fn held_shown(_: &str, _: Option<&str>, event: WidgetEvent) -> anyhow::Result<Vec<Effect>> {
    if matches!(event, WidgetEvent::Shown) {
        let deadline = Instant::now() + POLL_TIMEOUT;
        while HOLD_SHOWN.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(vec![])
}

fn decl(id: &str) -> WidgetDecl {
    WidgetDecl {
        id: id.into(),
        title: format!("Widget {id}"),
        size: WidgetSize::Single,
    }
}

/// A module with three single always tiles, `a`, `b` and `c`, that draw `render`.
fn three_tiles(render: fn(&str, &WidgetContext) -> anyhow::Result<Frame>) -> FakeModule {
    FakeModule::new(items(), nothing)
        .with_widgets(WIDGETS.iter().map(|id| decl(id)).collect())
        .with_render(render)
}

/// The core the launcher runs on, and what its `FakeModule` saw.
struct Launcher {
    core: Rc<RefCell<Core>>,
    renders: Arc<AtomicUsize>,
    render_contexts: Arc<Mutex<Vec<(String, WidgetContext)>>>,
    widget_events: Arc<Mutex<Vec<SeenWidgetEvent>>>,
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

/// Initializes the UI over a core with `module`, once the module has
/// published its items and, with `shown`, shown those always tiles; then
/// shows the launcher.
fn setup_with(cx: &mut TestAppContext, module: FakeModule, shown: &[&str]) -> Launcher {
    let renders = module.renders.clone();
    let render_contexts = module.render_contexts.clone();
    let widget_events = module.widget_events.clone();
    let ctx = module.ctx.clone();
    let mut core = new_core();
    core.register(Box::new(module));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    if !shown.is_empty() {
        let ctx: ModuleContext = ctx.lock().unwrap().clone().unwrap();
        ctx.emit(vec![Effect::ShowWidgets(
            shown.iter().map(|id| id.to_string()).collect(),
        )]);
        let ev = wait_event(&core.events());
        core.apply(ev, LocalNow::now());
    }
    let core = Rc::new(RefCell::new(core));
    tomakast_ui::set_raster_helper(env!("CARGO_BIN_EXE_tomakast-raster").into());
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core.clone()));
    });
    show(cx);
    Launcher {
        core,
        renders,
        render_contexts,
        widget_events,
    }
}

fn setup(cx: &mut TestAppContext) -> Launcher {
    setup_with(cx, three_tiles(still), &WIDGETS)
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

/// Whether every tile of [`WIDGETS`] has a frame.
fn all_drawn(core: &Core) -> bool {
    WIDGETS
        .iter()
        .all(|id| core.widget_frame(&key(id)).is_some())
}

/// Polls `done` until it holds; panics after 5 s.
fn poll(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + POLL_TIMEOUT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} within {POLL_TIMEOUT:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn key(widget: &str) -> tomakast_core::WidgetKey {
    tomakast_core::WidgetKey::new("fake", widget)
}

fn tile_id(widget: &str) -> String {
    format!("widget-tile:fake/{widget}")
}

fn show(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show(cx)));
    cx.run_until_parked();
}

fn hide_launcher(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.hide(cx)));
    cx.run_until_parked();
}

/// The launcher is the only window the tests open.
fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "expected exactly the launcher window");
    windows[0]
}

/// Draws a frame, which ticks the frame clock.
fn render(cx: &mut TestAppContext) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Bounds of element `id` in the last frame; `None` when it is not there.
fn bounds(cx: &mut TestAppContext, id: &str) -> Option<Bounds<Pixels>> {
    let w = window(cx);
    let id = id.to_string();
    cx.update_window(w, |_, window, _| window.try_find(id).map(|e| e.bounds()))
        .unwrap()
}

/// Bounds of the first section heading of the root list.
fn first_heading(cx: &mut TestAppContext) -> Bounds<Pixels> {
    let w = window(cx);
    cx.update_window(w, |_, window, _| {
        window.find_all_by_role(Role::Heading)[0].bounds()
    })
    .unwrap()
}

/// How many elements are labelled `label` in the last frame.
fn labelled(cx: &mut TestAppContext, label: &str) -> usize {
    let w = window(cx);
    cx.update_window(w, |_, window, _| window.find_all_by_label(label).len())
        .unwrap()
}

/// One round of the frame clock: applies the queued core events, draws a
/// frame (a tick), lets 17 ms pass on the executor's clock, and delivers the
/// next animation frame. Returns how many callbacks it ran.
fn pump(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) -> usize {
    drain(cx, core);
    render(cx);
    cx.executor().advance_clock(Duration::from_millis(17));
    let w = window(cx);
    let ran = cx
        .update_window(w, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    ran
}

/// Pumps the frame clock until a round runs no callback; panics after 5 s.
fn settle(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let deadline = Instant::now() + POLL_TIMEOUT;
    while pump(cx, core) > 0 {
        assert!(
            Instant::now() < deadline,
            "the frame clock never stopped within {POLL_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[gpui_kit::test]
fn no_tiles_no_strip(cx: &mut TestAppContext) {
    setup_with(cx, FakeModule::new(items(), nothing), &[]);
    render(cx);

    assert!(bounds(cx, "widget-strip").is_none());
    let header = bounds(cx, "launcher-header").expect("the root header");
    // The list's first line is a section heading.
    assert_eq!(first_heading(cx).top(), header.bottom());
}

#[gpui_kit::test]
fn three_tiles_peek_from_the_right(cx: &mut TestAppContext) {
    setup(cx);
    render(cx);

    let strip = bounds(cx, "widget-strip").expect("the strip");
    let left = |cx: &mut TestAppContext, widget: &str| {
        let tile = bounds(cx, &tile_id(widget)).expect("the tile");
        f32::from(tile.left() - strip.left())
    };
    assert_eq!(left(cx, "a"), 16.);
    assert_eq!(left(cx, "b"), 379.);
    assert_eq!(left(cx, "c"), 742.);
    assert_eq!(strip.size.width, px(750.));
    // 8 px above and below the tiles; the list starts under it.
    assert_eq!(strip.size.height, px(136.));
    let header = bounds(cx, "launcher-header").unwrap();
    assert_eq!(strip.top(), header.bottom());
    assert_eq!(first_heading(cx).top(), strip.bottom());
}

#[gpui_kit::test]
fn a_tile_that_never_rendered_is_a_placeholder_of_its_size(cx: &mut TestAppContext) {
    setup(cx);
    render(cx);

    let tile = bounds(cx, &tile_id("a")).expect("the tile");
    assert_eq!(tile.size.width, px(355.));
    assert_eq!(tile.size.height, px(120.));
    assert_eq!(labelled(cx, FRAME_TEXT), 0, "no frame was delivered");
    let w = window(cx);
    let title = cx
        .update_window(w, |_, window, _| {
            let tile = window.find(tile_id("a"));
            (tile.role(), tile.label().map(str::to_string))
        })
        .unwrap();
    assert_eq!(title, (Some(Role::Group), Some("Widget a".to_string())));
}

#[gpui_kit::test]
fn reopen_draws_the_cached_frame_before_any_render(cx: &mut TestAppContext) {
    let launcher = setup_with(
        cx,
        three_tiles(still).with_widget_events(held_shown),
        &WIDGETS,
    );
    render(cx);
    deliver_until(cx, &launcher.core, all_drawn);
    render(cx);
    assert_eq!(labelled(cx, FRAME_TEXT), 3);

    hide_launcher(cx);
    // Past the 60/s gate, so the reopening tick renders again.
    cx.executor().advance_clock(Duration::from_secs(1));
    let before = launcher.renders.load(Ordering::SeqCst);
    // The worker takes `Shown` before the renders of the same tick.
    HOLD_SHOWN.store(true, Ordering::SeqCst);
    show(cx);
    render(cx);

    assert_eq!(labelled(cx, FRAME_TEXT), 3, "the cached frames");
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(launcher.renders.load(Ordering::SeqCst), before);

    HOLD_SHOWN.store(false, Ordering::SeqCst);
    poll("the reopening tick's renders", || {
        launcher.renders.load(Ordering::SeqCst) > before
    });
}

#[gpui_kit::test]
fn wheel_scrolls_the_strip_horizontally(cx: &mut TestAppContext) {
    setup(cx);
    render(cx);
    let before = bounds(cx, &tile_id("a")).unwrap().left();

    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.scroll(
            "widget-strip",
            ScrollDelta::Pixels(point(px(0.), px(-100.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    render(cx);

    assert_eq!(bounds(cx, &tile_id("a")).unwrap().left(), before - px(100.));
}

#[gpui_kit::test]
fn hiding_sends_hidden_to_every_shown_tile(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    render(cx);

    hide_launcher(cx);

    let hidden = |events: &[SeenWidgetEvent]| {
        let mut widgets: Vec<String> = events
            .iter()
            .filter(|(_, _, event)| matches!(event, WidgetEvent::Hidden))
            .map(|(widget, _, _)| widget.clone())
            .collect();
        widgets.sort();
        widgets
    };
    poll("Hidden for every tile", || {
        hidden(&launcher.widget_events.lock().unwrap()).len() >= WIDGETS.len()
    });
    assert_eq!(hidden(&launcher.widget_events.lock().unwrap()), WIDGETS);
}

#[gpui_kit::test]
fn an_animating_tile_asks_for_the_next_frame(cx: &mut TestAppContext) {
    ANIMATE.store(true, Ordering::SeqCst);
    let launcher = setup_with(cx, three_tiles(animated), &WIDGETS);
    render(cx);
    deliver_until(cx, &launcher.core, all_drawn);

    // A still strip stops at 6 renders: the first draw, then one more for
    // each tile's `Shown`. An animating one keeps asking past that.
    let deadline = Instant::now() + POLL_TIMEOUT;
    while launcher.renders.load(Ordering::SeqCst) < 30 {
        assert!(pump(cx, &launcher.core) >= 1, "an animating strip asks");
        assert!(
            Instant::now() < deadline,
            "30 renders within {POLL_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    // The next renders draw still frames; once they are in, nothing is
    // dirty, nothing animates and nothing asks for a frame.
    ANIMATE.store(false, Ordering::SeqCst);
    settle(cx, &launcher.core);
    assert!(WIDGETS.iter().all(|id| {
        !launcher
            .core
            .borrow()
            .widget_frame(&key(id))
            .unwrap()
            .animate
    }));
    let renders = launcher.renders.load(Ordering::SeqCst);
    for round in 0..3 {
        assert_eq!(pump(cx, &launcher.core), 0, "round {round}");
    }
    assert_eq!(launcher.renders.load(Ordering::SeqCst), renders);
}

#[gpui_kit::test]
fn a_still_strip_stops_asking_for_frames(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    render(cx);
    deliver_until(cx, &launcher.core, all_drawn);

    settle(cx, &launcher.core);

    assert_eq!(launcher.renders.load(Ordering::SeqCst), 6);
    assert_eq!(pump(cx, &launcher.core), 0);
}

#[gpui_kit::test]
fn tiles_render_for_their_drawable_box(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    render(cx);
    deliver_until(cx, &launcher.core, all_drawn);

    let contexts = launcher.render_contexts.lock().unwrap().clone();
    let mut widgets: Vec<&str> = contexts.iter().map(|(w, _)| w.as_str()).collect();
    widgets.sort();
    assert_eq!(widgets, WIDGETS);
    for (_, context) in &contexts {
        assert_eq!((context.width, context.height), (355., 120.));
        let sizes = &context.theme.sizes;
        assert_eq!(
            (sizes.title, sizes.value, sizes.body, sizes.caption),
            (16., 28., 14., 12.)
        );
        // The test platform's windows are light.
        assert!(!context.theme.dark);
    }
}

#[gpui_kit::test]
fn a_stopped_widget_shows_an_error_toast(cx: &mut TestAppContext) {
    setup(cx);
    let stopped = |title: &str| StoppedWidget {
        title: title.into(),
        reason: "drew late 5 times".into(),
    };

    hand(
        cx,
        CoreUpdate::WidgetStopped {
            widgets: vec![stopped("Clock"), stopped("Weather")],
        },
    );
    cx.run_until_parked();
    render(cx);

    // One toast per widget; the last one shows.
    let w = window(cx);
    let toast = cx
        .update_window(w, |_, window, _| {
            window
                .try_find("launcher-toast")
                .and_then(|e| e.label().map(str::to_string))
        })
        .unwrap();
    assert_eq!(
        toast.as_deref(),
        Some("Widget Weather stopped: drew late 5 times")
    );
}

/// Painted quads that lie inside the strip but outside every tile, with a
/// visible fill: the scrollbar's track and thumb.
fn scrollbar_quads(cx: &mut TestAppContext) -> usize {
    render(cx);
    let strip = bounds(cx, "widget-strip").expect("the strip");
    let tiles: Vec<Bounds<Pixels>> = WIDGETS
        .iter()
        .filter_map(|id| bounds(cx, &tile_id(id)))
        .collect();
    let w = window(cx);
    cx.update_window(w, |_, window, _| {
        let scale = window.scale_factor();
        let inside = |inner: &Bounds<gpui_kit::ScaledPixels>, outer: &Bounds<Pixels>| {
            let outer = outer.scale(scale);
            inner.left() >= outer.left()
                && inner.right() <= outer.right()
                && inner.top() >= outer.top()
                && inner.bottom() <= outer.bottom()
        };
        window
            .painted_quads()
            .iter()
            .filter(|q| !q.background.is_transparent())
            .filter(|q| inside(&q.bounds, &strip))
            .filter(|q| !tiles.iter().any(|tile| inside(&q.bounds, tile)))
            .count()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_scrollbar_shows_only_while_scrolling(cx: &mut TestAppContext) {
    setup(cx);

    assert_eq!(scrollbar_quads(cx), 0, "no scrollbar at rest");

    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.scroll(
            "widget-strip",
            ScrollDelta::Pixels(point(px(0.), px(-100.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    // The kit fades it in on the wall clock.
    render(cx);
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        scrollbar_quads(cx) > 0,
        "the scrollbar shows while scrolling"
    );
}

#[gpui_kit::test]
fn a_strip_change_alone_redraws_the_window(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    render(cx);
    let w = window(cx);
    let view = cx.update(|cx| {
        w.downcast::<LauncherView>()
            .expect("the launcher view")
            .root(cx)
            .expect("the window is open")
    });
    let notified = Rc::new(std::cell::Cell::new(0));
    let _observer = cx.update(|cx| {
        let notified = notified.clone();
        cx.observe(&view, move |_, _| notified.set(notified.get() + 1))
    });
    // The frames reach the core; the window hears of nothing yet.
    while !all_drawn(&launcher.core.borrow()) {
        let ev = wait_event(&launcher.core.borrow().events());
        launcher.core.borrow_mut().apply(ev, LocalNow::now());
    }
    cx.run_until_parked();
    assert_eq!(notified.get(), 0);

    hand(cx, CoreUpdate::Widgets);
    cx.run_until_parked();

    assert_eq!(notified.get(), 1, "the view asks to be drawn again");
}

/// `effects` as a widget event's answer of the running `fake` instance.
fn widget_effects(
    core: &Rc<RefCell<Core>>,
    request: Option<RequestId>,
    effects: Vec<Effect>,
) -> CoreUpdate {
    CoreUpdate::WidgetEffects {
        request,
        provider: "fake".into(),
        module_instance: core.borrow().module_instance("fake").unwrap(),
        effects,
        items_changed: false,
    }
}

#[gpui_kit::test]
fn widget_effects_route_by_their_request(cx: &mut TestAppContext) {
    let launcher = setup(cx);

    // As a timer's effects: `CloseWindow` is ignored.
    let update = widget_effects(&launcher.core, None, vec![Effect::CloseWindow]);
    hand(cx, update);
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.global::<LauncherController>().is_visible()));

    // With a request no window issued, as an action's reply: it applies.
    let update = widget_effects(
        &launcher.core,
        Some(RequestId(u64::MAX)),
        vec![Effect::CloseWindow],
    );
    hand(cx, update);
    cx.run_until_parked();
    assert!(!cx.update(|cx| cx.global::<LauncherController>().is_visible()));
}

#[gpui_kit::test]
fn a_scale_change_reaches_the_tiles(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    render(cx);
    deliver_until(cx, &launcher.core, all_drawn);
    let scales = |launcher: &Launcher| -> Vec<f32> {
        launcher
            .render_contexts
            .lock()
            .unwrap()
            .iter()
            .map(|(_, context)| context.scale)
            .collect()
    };
    // The test platform's windows start at 2.
    assert!(scales(&launcher).iter().all(|&scale| scale == 2.));

    let w = window(cx);
    cx.simulate_window_scale_factor_change(w, 3.);
    cx.run_until_parked();

    for _ in 0..100 {
        if scales(&launcher).contains(&3.) {
            break;
        }
        pump(cx, &launcher.core);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        scales(&launcher).contains(&3.),
        "a render at the new scale: {:?}",
        scales(&launcher)
    );
}

// Components: a tile draws its frame's nodes.

fn edges(all: f32) -> Edges {
    Edges {
        top: all,
        right: all,
        bottom: all,
        left: all,
    }
}

fn layout(padding: f32) -> Layout {
    Layout {
        padding: edges(padding),
        gap: 0.,
        align: Align::Start,
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

fn text(text: &str, style: TextStyle, max_lines: u8) -> NodeKind {
    NodeKind::Text(TextNode {
        text: text.into(),
        style,
        color: Color::Token(ColorToken::Text),
        align: TextAlign::Start,
        max_lines,
        mono: false,
    })
}

fn frame_of(nodes: Vec<Node>) -> Frame {
    Frame {
        nodes,
        animate: false,
        actions: vec![],
        images: vec![],
    }
}

/// 1 KiB without a space, a right-to-left override, 200 emoji, and lines.
const HOSTILE_TEXTS: [(&str, u8); 5] = [
    ("1kib", 1),
    ("1kib-clamped", 2),
    ("rtl", 1),
    ("emoji", 1),
    ("lines", 2),
];

fn hostile_text(name: &str) -> String {
    match name {
        "1kib" | "1kib-clamped" => "W".repeat(1024),
        "rtl" => format!("\u{202e}{}", "drowssap ".repeat(50)),
        "emoji" => "😀".repeat(200),
        _ => "line\n".repeat(40),
    }
}

/// The frames of the component tests, by widget id; any other widget
/// draws [`frame`].
fn named(widget: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    let column = |padding| NodeKind::Column(layout(padding));
    Ok(match widget {
        "column" => frame_of(vec![
            node("", None, column(0.)),
            node("", Some(0), text("Hello", TextStyle::Title, 1)),
            node("", Some(0), text("World", TextStyle::Caption, 1)),
        ]),
        "row" => frame_of(vec![
            node("", None, NodeKind::Row(layout(0.))),
            Node {
                grow: 1.,
                ..node("one", Some(0), NodeKind::Spacer)
            },
            Node {
                grow: 3.,
                ..node("three", Some(0), NodeKind::Spacer)
            },
        ]),
        "padded" => frame_of(vec![
            node("", None, column(10.)),
            Node {
                width: Some(40.),
                height: Some(20.),
                ..node("box", Some(0), NodeKind::Spacer)
            },
        ]),
        "button" => frame_of(vec![
            node("", None, column(0.)),
            node(
                "reset",
                Some(0),
                NodeKind::Button(ButtonNode {
                    label: Some("Reset".into()),
                    icon: None,
                    action: "reset".into(),
                }),
            ),
        ]),
        "hostile" => {
            let mut nodes = vec![node("", None, column(0.))];
            nodes.extend(HOSTILE_TEXTS.iter().map(|&(name, max_lines)| {
                node(
                    name,
                    Some(0),
                    text(&hostile_text(name), TextStyle::Value, max_lines),
                )
            }));
            frame_of(nodes)
        }
        // Few enough lines to fit: none shrinks.
        "lines" => frame_of(vec![
            node("", None, column(0.)),
            node("", Some(0), text("x", TextStyle::Caption, 1)),
            node(
                "",
                Some(0),
                text(&"line\n".repeat(40), TextStyle::Caption, 2),
            ),
            node(
                "",
                Some(0),
                text(&"line\n".repeat(40), TextStyle::Caption, 0),
            ),
        ]),
        // A long text of a fixed height beside a fixed width; then lines
        // that wrap, and a line break between them.
        "mixed" => frame_of(vec![
            node("", None, column(0.)),
            node("", Some(0), NodeKind::Row(layout(0.))),
            Node {
                height: Some(20.),
                ..node("", Some(1), text(&"W".repeat(1024), TextStyle::Caption, 1))
            },
            Node {
                width: Some(40.),
                height: Some(20.),
                ..node("", Some(1), NodeKind::Spacer)
            },
            node(
                "",
                Some(0),
                text(
                    &format!("{0}\n{0}", "W".repeat(1024)),
                    TextStyle::Caption,
                    2,
                ),
            ),
            node("", Some(0), text(&"W".repeat(1024), TextStyle::Caption, 2)),
            // A row as wide as the tile, then a text right of it: wholly
            // outside the tile, inside the strip.
            node("", Some(0), NodeKind::Row(layout(0.))),
            Node {
                width: Some(355.),
                height: Some(20.),
                ..node("", Some(6), NodeKind::Spacer)
            },
            Node {
                width: Some(100.),
                ..node("", Some(6), text("Outside", TextStyle::Caption, 1))
            },
        ]),
        "deep" => {
            // A button under 31 nested columns: 32 ancestors.
            let mut nodes = vec![node("", None, column(0.))];
            nodes.extend((1..32).map(|ix| node("", Some(ix - 1), column(0.))));
            nodes.push(node(
                "deep-button",
                Some(31),
                NodeKind::Button(ButtonNode {
                    label: Some("Deep".into()),
                    icon: None,
                    action: "go".into(),
                }),
            ));
            frame_of(nodes)
        }
        "big-icon" => frame_of(vec![
            node("", None, column(0.)),
            node(
                "",
                Some(0),
                NodeKind::Icon(IconNode {
                    icon: Icon::Emoji("😀".into()),
                    size: 4096.,
                }),
            ),
        ]),
        "overlay" => {
            let sized = |id, parent, w, h| Node {
                width: Some(w),
                height: Some(h),
                ..node(id, Some(parent), NodeKind::Spacer)
            };
            frame_of(vec![
                node("", None, column(0.)),
                // 1: sized by its first child, inset by its padding of 5.
                node("", Some(0), NodeKind::Overlay(layout(5.))),
                sized("first", 1, 40., 20.),
                sized("second", 1, 10., 10.),
                // 4: a size of its own.
                Node {
                    width: Some(100.),
                    height: Some(50.),
                    ..node("", Some(0), NodeKind::Overlay(layout(0.)))
                },
                sized("under", 4, 40., 20.),
                node("", Some(4), text("on top", TextStyle::Caption, 1)),
            ])
        }
        // A canvas of 100 × 50 that a rect fills.
        "canvas" => canvas_frame(Some((100., 50.)), vec![solid_rect(0., 0., 100., 50.)]),
        // The same, animating: it asks for a new frame every frame.
        "anim-canvas" => Frame {
            animate: true,
            ..canvas_frame(Some((100., 50.)), vec![solid_rect(0., 0., 100., 50.)])
        },
        // A canvas smaller than its tile, and a rect past all its edges.
        "small-canvas" => canvas_frame(
            Some((60., 30.)),
            vec![solid_rect(-500., -500., 1000., 1000.)],
        ),
        "mono-text" => frame_of(vec![
            node("", None, column(0.)),
            node(
                "",
                Some(0),
                NodeKind::Text(TextNode {
                    text: NODE_MONO_TEXT.into(),
                    style: TextStyle::Body,
                    color: Color::Token(ColorToken::Text),
                    align: TextAlign::Start,
                    max_lines: 1,
                    mono: true,
                }),
            ),
        ]),
        "canvas-text" => canvas_frame(
            Some((200., 50.)),
            vec![Draw::Text(TextCmd {
                mono: true,
                ..canvas_text(0., 0., CANVAS_MONO_TEXT)
            })],
        ),
        // Larger than its tile: texts of many lines, 1 KiB without a space
        // and emoji, then a rect far past every edge.
        "hostile-canvas" => canvas_frame(
            Some((1000., 500.)),
            vec![
                Draw::Text(canvas_text(0., 0., &"line\n".repeat(40))),
                Draw::Text(canvas_text(0., 30., &"W".repeat(1024))),
                Draw::Text(canvas_text(0., 60., &"😀".repeat(200))),
                solid_rect(-1000., -1000., 5000., 5000.),
            ],
        ),
        // A 60 × 40 image node showing an 8 × 4 PNG sent with the frame.
        "image" => Frame {
            images: vec![WidgetImage {
                id: "pic".into(),
                format: ImageFormat::Png,
                bytes: Arc::from(png(8, 4)),
            }],
            ..frame_of(vec![
                node("", None, NodeKind::Column(layout(0.))),
                Node {
                    width: Some(60.),
                    height: Some(40.),
                    ..node(
                        "pic",
                        Some(0),
                        NodeKind::Image(ImageNode {
                            image: "pic".into(),
                            fit: ImageFit::Contain,
                            radius: 4.,
                        }),
                    )
                },
            ])
        },
        "perf-quads" | "perf-quads-apart" | "perf-rects" | "perf-ellipses" | "perf-texts"
        | "perf-path" => perf_frame(widget),
        // A tile whose primary action is `open`.
        "actions" => Frame {
            actions: vec![open_action()],
            ..frame(false)
        },
        // The same, with a button `reset` (node 1) above a text.
        "control" => Frame {
            actions: vec![open_action()],
            ..frame_of(vec![
                node("", None, column(0.)),
                node(
                    "reset",
                    Some(0),
                    NodeKind::Button(ButtonNode {
                        label: Some("Reset".into()),
                        icon: None,
                        action: "reset".into(),
                    }),
                ),
                node("", Some(0), text("below", TextStyle::Body, 1)),
            ])
        },
        // `canvas` while [`SWAP_CANVAS`] is set, else [`frame`].
        "swap" if SWAP_CANVAS.load(Ordering::SeqCst) => {
            canvas_frame(Some((100., 50.)), vec![solid_rect(0., 0., 100., 50.)])
        }
        // A focusable canvas `anim` (node 1) of 100 × 50.
        "anim" => {
            let mut frame = canvas_frame(Some((100., 50.)), vec![solid_rect(0., 0., 100., 50.)]);
            frame.nodes[1].id = "anim".into();
            if let NodeKind::Canvas(canvas) = &mut frame.nodes[1].kind {
                canvas.focusable = true;
            }
            frame
        }
        _ => frame(false),
    })
}

/// A red PNG of `width` × `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

/// A column holding one canvas of `size` (else filling it) drawing `commands`.
fn canvas_frame(size: Option<(f32, f32)>, commands: Vec<Draw>) -> Frame {
    let column = Layout {
        align: Align::Stretch,
        ..layout(0.)
    };
    frame_of(vec![
        node("", None, NodeKind::Column(column)),
        Node {
            width: size.map(|(w, _)| w),
            height: size.map(|(_, h)| h),
            grow: if size.is_none() { 1. } else { 0. },
            ..node(
                "canvas",
                Some(0),
                NodeKind::Canvas(CanvasNode {
                    focusable: false,
                    commands,
                }),
            )
        },
    ])
}

/// The color the canvas tests fill rects with.
const CANVAS_RED: u32 = 0xd02040ff;
const CANVAS_MONO_TEXT: &str = "canvas mono";
const NODE_MONO_TEXT: &str = "node mono";

fn solid_style() -> ShapeStyle {
    ShapeStyle {
        paint: Paint::Solid(Color::Rgba(CANVAS_RED)),
        stroke: None,
        opacity: 1.,
    }
}

fn solid_rect(x: f32, y: f32, width: f32, height: f32) -> Draw {
    Draw::Rect(RectCmd {
        x,
        y,
        width,
        height,
        radius: 0.,
        style: solid_style(),
    })
}

fn canvas_text(x: f32, y: f32, text: &str) -> TextCmd {
    TextCmd {
        x,
        y,
        text: text.into(),
        size: 28.,
        color: Color::Token(ColorToken::Text),
        font: None,
        mono: false,
        opacity: 1.,
    }
}

/// Commands and path segments of each canvas the paint-time test draws:
/// the cap; fewer in a debug build, which only checks that they draw.
const PERF_COMMANDS: usize = if cfg!(debug_assertions) {
    500
} else {
    MAX_FRAME_COMMANDS
};

/// A canvas filling its tile with [`PERF_COMMANDS`] commands and segments
/// of one kind, under a rotation of 15°: rects without one (quads), stacked
/// ten deep or each in a place of its own; rounded rects (paths); stroked
/// ellipses (paths); texts that all differ (shaped one by one); or one
/// stroked path of that many line segments.
fn perf_frame(widget: &str) -> Frame {
    let shapes = PERF_COMMANDS - 2;
    let at = |ix: usize| match widget {
        "perf-quads-apart" => ((ix % 170) as f32 * 2., (ix / 170) as f32 * 2.),
        _ => ((ix % 50) as f32 * 7., (ix / 50 % 20) as f32 * 6.),
    };
    let quads = widget.starts_with("perf-quads");
    let (rotate, translate) = if quads {
        (0., (0., 0.))
    } else {
        (15., (10., -20.))
    };
    let body: Vec<Draw> = match widget {
        "perf-path" => {
            let point = |x, y| tomakast_core::Point { x, y };
            let mut segments = vec![PathSegment::MoveTo(point(0., 0.))];
            // One segment less: the path is one command. Lines: a stroke of
            // as many tight cubics passes lyon's 65 536 vertices and is not
            // drawn.
            segments.extend((1..shapes - 1).map(|ix| {
                let (x, y) = at(ix);
                PathSegment::LineTo(point(x, y))
            }));
            vec![Draw::Path(PathCmd {
                segments,
                style: ShapeStyle {
                    stroke: Some(1.),
                    ..solid_style()
                },
            })]
        }
        _ => (0..shapes)
            .map(|ix| {
                let (x, y) = at(ix);
                match widget {
                    "perf-quads" => solid_rect(x, y, 12., 8.),
                    "perf-quads-apart" => solid_rect(x, y, 1., 1.),
                    "perf-rects" => Draw::Rect(RectCmd {
                        x,
                        y,
                        width: 12.,
                        height: 8.,
                        radius: 3.,
                        style: solid_style(),
                    }),
                    "perf-ellipses" => Draw::Ellipse(EllipseCmd {
                        cx: x,
                        cy: y,
                        rx: 6.,
                        ry: 4.,
                        style: ShapeStyle {
                            stroke: Some(1.5),
                            ..solid_style()
                        },
                    }),
                    _ => Draw::Text(TextCmd {
                        size: 10.,
                        ..canvas_text(x, y, &format!("t{ix}"))
                    }),
                }
            })
            .collect(),
    };
    let mut commands = vec![Draw::PushTransform(Transform {
        translate: tomakast_core::Point {
            x: translate.0,
            y: translate.1,
        },
        scale: tomakast_core::Point { x: 1., y: 1. },
        rotate,
    })];
    commands.extend(body);
    commands.push(Draw::PopTransform);
    canvas_frame(None, commands)
}

/// A module with single always tiles `widgets` that draw [`named`] frames.
fn named_launcher(cx: &mut TestAppContext, widgets: &[&str]) -> Launcher {
    let module = FakeModule::new(items(), nothing)
        .with_widgets(widgets.iter().map(|id| decl(id)).collect())
        .with_render(named);
    setup_with(cx, module, widgets)
}

/// Delivers the first frame of every tile of `widgets`, then draws.
fn deliver_frames(cx: &mut TestAppContext, launcher: &Launcher, widgets: &[&str]) {
    deliver_until(cx, &launcher.core, |core| {
        widgets
            .iter()
            .all(|id| core.widget_frame(&key(id)).is_some())
    });
    render(cx);
}

/// [`named_launcher`], drawn with every tile's first frame.
fn named_tiles(cx: &mut TestAppContext, widgets: &[&str]) -> Launcher {
    let launcher = named_launcher(cx, widgets);
    render(cx);
    deliver_frames(cx, &launcher, widgets);
    launcher
}

/// Whether element `id` showed in the last frame, inside every clip.
fn visible(cx: &mut TestAppContext, id: &str) -> bool {
    let w = window(cx);
    let id = id.to_string();
    cx.update_window(w, |_, window, _| window.find(id).visible())
        .unwrap()
}

fn node_id(widget: &str, ix: usize) -> String {
    format!("widget-node:fake/{widget}:{ix}")
}

/// Labels of the elements of `role` in `widget`'s tile, top to bottom.
fn tile_labels(cx: &mut TestAppContext, widget: &str, role: Role) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, _| {
        window
            .within(tile_id(widget))
            .find_all_by_role(role)
            .iter()
            .filter_map(|e| e.label().map(str::to_string))
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_column_lays_out_its_texts_top_to_bottom(cx: &mut TestAppContext) {
    named_tiles(cx, &["column"]);

    let hello = bounds(cx, &node_id("column", 1)).expect("Hello");
    let world = bounds(cx, &node_id("column", 2)).expect("World");
    assert!(world.top() >= hello.bottom(), "{hello:?} {world:?}");
    assert_eq!(tile_labels(cx, "column", Role::Label), ["Hello", "World"]);
}

#[gpui_kit::test]
fn a_row_shares_free_space_by_grow(cx: &mut TestAppContext) {
    named_tiles(cx, &["row"]);

    let width = |cx: &mut TestAppContext, ix| {
        f32::from(bounds(cx, &node_id("row", ix)).unwrap().size.width)
    };
    let (one, three) = (width(cx, 1), width(cx, 2));
    assert!((one - 88.75).abs() <= 0.5, "{one}");
    assert!((three - 266.25).abs() <= 0.5, "{three}");
}

#[gpui_kit::test]
fn fixed_sizes_and_padding_apply(cx: &mut TestAppContext) {
    named_tiles(cx, &["padded"]);

    let tile = bounds(cx, &tile_id("padded")).unwrap();
    let spacer = bounds(cx, &node_id("padded", 1)).unwrap();
    assert_eq!(spacer.origin - tile.origin, point(px(10.), px(10.)));
    assert_eq!((spacer.size.width, spacer.size.height), (px(40.), px(20.)));
}

#[gpui_kit::test]
fn buttons_are_focusable_controls_with_labels(cx: &mut TestAppContext) {
    named_tiles(cx, &["button"]);

    assert_eq!(tile_labels(cx, "button", Role::Button), ["Reset"]);
    let w = window(cx);
    let role = cx
        .update_window(w, |_, window, _| window.find(node_id("button", 1)).role())
        .unwrap();
    assert_eq!(role, Some(Role::Button));
}

/// The test platform shapes text with GPUI's `NoopTextSystem`: every
/// glyph is one fixed advance, with no bidi reordering and no emoji fonts.
/// So right-to-left and emoji runs here are laid out as plain glyphs; that
/// they cannot draw outside the tile rests on the clip, which this test pins
/// with a text laid out wholly outside its tile.
#[gpui_kit::test]
fn tile_text_never_leaves_the_tile(cx: &mut TestAppContext) {
    let widgets = ["hostile", "mixed", "lines"];
    let launcher = named_launcher(cx, &widgets);
    // The strip and the list before any frame: placeholders of their size.
    render(cx);
    let strip_height = bounds(cx, "widget-strip").unwrap().size.height;
    let list_top = first_heading(cx).top();

    deliver_frames(cx, &launcher, &widgets);

    let strip = bounds(cx, "widget-strip").unwrap();
    let tile = bounds(cx, &tile_id("hostile")).unwrap();
    assert_eq!((tile.size.width, tile.size.height), (px(355.), px(120.)));
    assert_eq!(strip.size.height, strip_height);
    let next = bounds(cx, &tile_id("mixed")).unwrap();
    assert_eq!(next.left() - strip.left(), px(379.));
    assert_eq!(first_heading(cx).top(), list_top);
    for (ix, (name, _)) in HOSTILE_TEXTS.iter().enumerate() {
        let text = bounds(cx, &node_id("hostile", ix + 1)).expect(name);
        // Its own box fits the tile's width: it truncates or wraps.
        assert!(text.left() >= tile.left(), "{name}: {text:?}");
        assert!(text.right() <= tile.right(), "{name}: {text:?}");
    }
    // Every text shows only where it overlaps its tile: a text laid out
    // wholly outside it, in the strip, is clipped away.
    let outside = bounds(cx, &node_id("mixed", 8)).unwrap();
    assert!(outside.left() >= next.right(), "{outside:?}");
    let texts = (1..=HOSTILE_TEXTS.len())
        .map(|ix| ("hostile", ix, tile))
        .chain([2, 4, 5, 8].map(|ix| ("mixed", ix, next)));
    for (widget, ix, tile) in texts {
        let w = window(cx);
        let snapshot = cx
            .update_window(w, |_, window, _| window.find(node_id(widget, ix)))
            .unwrap();
        let overlap = snapshot.bounds().intersect(&tile);
        let overlaps = overlap.size.width > px(0.) && overlap.size.height > px(0.);
        assert_eq!(snapshot.visible(), overlaps, "{widget} {ix}: {snapshot:?}");
    }
    assert!(!visible(cx, &node_id("mixed", 8)));
    // In a row, the long text shrinks and leaves the fixed width its room.
    for ix in [2, 3] {
        let node = bounds(cx, &node_id("mixed", ix)).unwrap();
        assert!(node.left() >= next.left(), "{ix}: {node:?}");
        assert!(node.right() <= next.right(), "{ix}: {node:?}");
    }
    // Line breaks count against a text's lines: 40 of them show as 2, or
    // as 1 with no limit; so do wrapped lines and a break together.
    let height =
        |cx: &mut TestAppContext, widget, ix| bounds(cx, &node_id(widget, ix)).unwrap().size.height;
    let one_line = height(cx, "lines", 1);
    assert!(one_line > px(0.));
    for (widget, ix) in [("lines", 2), ("mixed", 4), ("mixed", 5)] {
        let lines = height(cx, widget, ix);
        assert!(lines > one_line, "{widget} {ix}: {lines:?}");
        assert!(lines <= one_line * 2. + px(1.), "{widget} {ix}: {lines:?}");
    }
    assert_eq!(height(cx, "lines", 3), one_line);
}

#[gpui_kit::test]
fn a_frame_nested_32_deep_renders(cx: &mut TestAppContext) {
    // On the test thread's stack, a debug build's 2 MiB.
    named_tiles(cx, &["deep"]);

    assert_eq!(tile_labels(cx, "deep", Role::Button), ["Deep"]);
    assert!(visible(cx, &node_id("deep", 32)));
}

#[gpui_kit::test]
fn an_icon_is_at_most_a_tile_high(cx: &mut TestAppContext) {
    named_tiles(cx, &["big-icon"]);

    let w = window(cx);
    let icons = cx
        .update_window(w, |_, window, _| {
            window
                .within(node_id("big-icon", 1))
                .find_all_by_role(Role::Image)
                .iter()
                .map(|icon| icon.bounds().size)
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(icons.len(), 1);
    assert!(
        icons[0].width <= px(120.) && icons[0].height <= px(120.),
        "{icons:?}"
    );
}

#[gpui_kit::test]
fn an_overlay_takes_its_first_childs_size_and_stacks_the_rest(cx: &mut TestAppContext) {
    named_tiles(cx, &["overlay"]);
    let at = |cx: &mut TestAppContext, ix| bounds(cx, &node_id("overlay", ix)).unwrap();

    let (overlay, first, second) = (at(cx, 1), at(cx, 2), at(cx, 3));
    let inset = point(px(5.), px(5.));
    assert_eq!(first.origin, overlay.origin + inset);
    assert_eq!(second.origin, first.origin);
    assert_eq!((second.size.width, second.size.height), (px(10.), px(10.)));
    assert!(
        overlay.size.width >= px(50.) && overlay.size.height >= px(30.),
        "{overlay:?}"
    );

    let (sized, under, on_top) = (at(cx, 4), at(cx, 5), at(cx, 6));
    assert_eq!((sized.size.width, sized.size.height), (px(100.), px(50.)));
    assert_eq!(under.origin, sized.origin);
    assert_eq!(on_top.origin, sized.origin);
}

#[gpui_kit::test]
fn only_tiles_on_screen_draw_their_frames(cx: &mut TestAppContext) {
    let widgets = ["a", "b", "c", "d", "e"];
    let launcher = named_launcher(cx, &widgets);
    render(cx);
    deliver_frames(cx, &launcher, &widgets[..3]);
    // `d` draws once scrolled into view, and keeps its frame.
    let w = window(cx);
    let scroll = |cx: &mut TestAppContext, by: f32| {
        cx.update_window(w, |_, window, cx| {
            window.scroll(
                "widget-strip",
                ScrollDelta::Pixels(point(px(0.), px(by))),
                cx,
            )
        })
        .unwrap();
        cx.run_until_parked();
        render(cx);
    };
    scroll(cx, -400.);
    deliver_frames(cx, &launcher, &["d"]);
    assert!(bounds(cx, &node_id("d", 0)).is_some(), "on screen, d draws");

    scroll(cx, 400.);

    let strip = bounds(cx, "widget-strip").unwrap();
    let tile = bounds(cx, &tile_id("d")).expect("d keeps its box");
    assert_eq!(tile.left() - strip.left(), px(1105.));
    assert_eq!(tile.size.width, px(355.));
    assert!(launcher.core.borrow().widget_frame(&key("d")).is_some());
    assert!(
        bounds(cx, &node_id("d", 0)).is_none(),
        "off screen, d's frame is not built"
    );
    assert!(
        bounds(cx, &node_id("c", 0)).is_some(),
        "the peeking tile draws"
    );
}

/// Bounds of the images drawn inside element `id` in the last frame.
fn images_in(cx: &mut TestAppContext, id: &str) -> Vec<Bounds<Pixels>> {
    let w = window(cx);
    let id = id.to_string();
    cx.update_window(w, |_, window, _| {
        window
            .within(id)
            .find_all_by_role(Role::Image)
            .iter()
            .map(|image| image.bounds())
            .collect()
    })
    .unwrap()
}

/// An image node is decoded off the UI thread: until then it draws nothing,
/// and once decoded the window is drawn again, with no render asked, with
/// the image filling the node.
#[gpui_kit::test]
fn an_image_node_draws_once_decoded(cx: &mut TestAppContext) {
    let launcher = named_launcher(cx, &["image"]);
    render(cx);
    // Applied without the controller, which would draw the window: the
    // first draw that asks for the image is the next render.
    while launcher.core.borrow().widget_frame(&key("image")).is_none() {
        let ev = wait_event(&launcher.core.borrow().events());
        launcher.core.borrow_mut().apply(ev, LocalNow::now());
    }
    render(cx);
    let image = "widget-image:fake/image:1";
    let node = bounds(cx, &node_id("image", 1)).expect("the image node");
    assert_eq!(bounds(cx, image), Some(node));
    assert_eq!(images_in(cx, image), [], "not decoded yet");

    // Decoded on a raster thread; the window draws again on its own.
    let deadline = Instant::now() + POLL_TIMEOUT;
    loop {
        // The UI thread looks for the answer every few milliseconds.
        cx.executor().advance_clock(Duration::from_millis(4));
        cx.run_until_parked();
        if !images_in(cx, image).is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "decoded within {POLL_TIMEOUT:?}");
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(images_in(cx, image), [node]);
}

// The canvas: a frame's draw commands painted natively.

/// Quads painted in the last frame with [`CANVAS_RED`], in scaled pixels,
/// each with the bounds of the content mask it is clipped to.
fn red_quads(
    cx: &mut TestAppContext,
) -> Vec<(
    Bounds<gpui_kit::ScaledPixels>,
    Bounds<gpui_kit::ScaledPixels>,
)> {
    let red = gpui_kit::Hsla::from(gpui_kit::rgba(CANVAS_RED));
    let w = window(cx);
    cx.update_window(w, |_, window, _| {
        window
            .painted_quads()
            .iter()
            .filter(|q| {
                q.background.as_solid().is_some_and(|c| {
                    (c.h - red.h).abs() < 0.01
                        && (c.s - red.s).abs() < 0.01
                        && (c.l - red.l).abs() < 0.01
                        && (c.a - red.a).abs() < 0.01
                })
            })
            .map(|q| (q.bounds, q.content_mask.bounds))
            .collect()
    })
    .unwrap()
}

fn scale_factor(cx: &mut TestAppContext) -> f32 {
    let w = window(cx);
    cx.update_window(w, |_, window, _| window.scale_factor())
        .unwrap()
}

#[gpui_kit::test]
fn a_canvas_takes_its_node_size(cx: &mut TestAppContext) {
    named_tiles(cx, &["canvas"]);

    let canvas = bounds(cx, &node_id("canvas", 1)).expect("the canvas");
    assert_eq!((canvas.size.width, canvas.size.height), (px(100.), px(50.)));
    // The rect that fills it is painted over exactly its box.
    let scale = scale_factor(cx);
    let quads = red_quads(cx);
    assert_eq!(quads.len(), 1, "{quads:?}");
    assert_eq!(quads[0].0, canvas.scale(scale));
}

/// A launcher over a text system that records the family and features of
/// every line.
fn recorded_tiles(widgets: &[&str]) -> (TestAppContext, Arc<FontRecorder>, Launcher) {
    let recorder = Arc::new(FontRecorder::default());
    let mut cx = TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        None,
        recorder.clone(),
    );
    let launcher = named_launcher(&mut cx, widgets);
    render(&mut cx);
    deliver_frames(&mut cx, &launcher, widgets);
    (cx, recorder, launcher)
}

/// Monospace widget text, on a canvas or in a text node, is in the bundled
/// JetBrains Mono with its ligatures on.
#[test]
fn mono_widget_text_is_shaped_in_the_theme_font() {
    let (_cx, recorder, _launcher) = recorded_tiles(&["canvas-text", "mono-text"]);

    let ligatures = vec![vec![("calt".to_owned(), 1), ("liga".to_owned(), 1)]];
    for text in [CANVAS_MONO_TEXT, NODE_MONO_TEXT] {
        assert_eq!(families_of(&recorder, text), ["JetBrains Mono"], "{text}");
        assert_eq!(features_of(&recorder, text), ligatures, "{text}");
    }
}

/// Review focus: a canvas text with line breaks, 1 KiB without a space or
/// a run of emoji, in a canvas larger than its tile, never draws outside
/// the tile nor moves the strip or the list. The test text system paints
/// no glyphs, so the clip is pinned by the rect drawn after the texts in
/// the same canvas: everything the canvas paints is clipped to that mask.
#[test]
fn canvas_text_never_leaves_the_tile() {
    let recorder = Arc::new(FontRecorder::default());
    let mut cx = TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        None,
        recorder.clone(),
    );
    let cx = &mut cx;
    let widgets = ["hostile-canvas", "column"];
    let launcher = named_launcher(cx, &widgets);
    render(cx);
    let strip_height = bounds(cx, "widget-strip").unwrap().size.height;
    let list_top = first_heading(cx).top();

    deliver_frames(cx, &launcher, &widgets);

    let tile = bounds(cx, &tile_id("hostile-canvas")).unwrap();
    assert_eq!((tile.size.width, tile.size.height), (px(355.), px(120.)));
    assert_eq!(
        bounds(cx, "widget-strip").unwrap().size.height,
        strip_height
    );
    assert_eq!(first_heading(cx).top(), list_top);
    let next = bounds(cx, &tile_id("column")).unwrap();
    assert_eq!(next.left() - tile.left(), px(363.));

    // Each text is laid out as one line, line breaks shown as spaces.
    let lines = recorder.lines();
    assert!(lines.iter().all(|line| !line.contains('\n')), "{lines:?}");
    assert!(lines.contains(&"line ".repeat(40)), "{lines:?}");
    assert!(lines.contains(&"W".repeat(1024)), "{lines:?}");
    assert!(lines.contains(&"😀".repeat(200)), "{lines:?}");

    // The rect past every edge is painted only inside the tile.
    let scale = scale_factor(cx);
    let tile = tile.scale(scale);
    let quads = red_quads(cx);
    assert_eq!(quads.len(), 1, "{quads:?}");
    let mask = quads[0].1;
    assert!(
        mask.left() >= tile.left()
            && mask.right() <= tile.right()
            && mask.top() >= tile.top()
            && mask.bottom() <= tile.bottom(),
        "{mask:?} outside {tile:?}"
    );
    assert!(mask.size.width > gpui_kit::ScaledPixels(0.));
}

/// The cost of painting a canvas of [`PERF_COMMANDS`] commands and
/// segments of each kind on the UI thread, one tile at a time: plan, text
/// shaping, path tessellation and the scene. The GPU's rasterizing is not
/// part of it, nor, with the test text system, any glyph's raster. In a
/// release build one path of that many segments, and as many rects each in
/// a place of its own, stay well within a 60 fps frame. Rects stacked ten
/// deep, small paths and texts do not: GPUI orders every quad, path and
/// glyph in the scene at a cost that grows with what it overlaps, and
/// every small path is tessellated on its own. Those are printed, not
/// asserted. A manual benchmark: the budgets are asserted only in a release
/// build (`cargo test --release -p tomakast-ui --test strip
/// canvas_paint_cost -- --nocapture`); a debug build draws 500 of each and
/// only checks that they draw. The strip's paint guard is off, or it would
/// stop the slow kinds.
#[test]
fn canvas_paint_cost_at_the_cap() {
    for (widget, budget) in [
        ("perf-quads", None),
        ("perf-quads-apart", Some(8)),
        ("perf-path", Some(8)),
        ("perf-rects", None),
        ("perf-ellipses", None),
        ("perf-texts", None),
    ] {
        let mut cx = TestAppContext::build(gpui_kit::TestDispatcher::new(0), None);
        let cx = &mut cx;
        let launcher = named_launcher(cx, &[widget]);
        cx.update(|cx| {
            cx.set_global(CanvasPaintBudget {
                slow: Duration::MAX,
                slow_paints: u32::MAX,
                stall: Duration::MAX,
            })
        });
        render(cx);
        deliver_frames(cx, &launcher, &[widget]);
        let canvas = bounds(cx, &node_id(widget, 1)).expect(widget);
        assert_eq!(
            (canvas.size.width, canvas.size.height),
            (px(355.), px(120.))
        );
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let start = Instant::now();
            render(cx);
            best = best.min(start.elapsed());
        }
        eprintln!("{widget}: {PERF_COMMANDS} commands paint in {best:?}");
        if let Some(ms) = budget.filter(|_| !cfg!(debug_assertions)) {
            assert!(best < Duration::from_millis(ms), "{widget}: {best:?}");
        }
    }
}

/// The toast's label in the last frame, if one shows.
fn toast(cx: &mut TestAppContext) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, _| {
        window
            .try_find("launcher-toast")
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

/// A canvas that keeps painting too slowly is stopped, as a widget that
/// keeps drawing late is: a toast says so and its tile leaves the strip. With a
/// budget no paint can meet, the second slow paint in a row stops it.
#[gpui_kit::test]
fn a_canvas_that_paints_too_slowly_is_stopped(cx: &mut TestAppContext) {
    let launcher = named_launcher(cx, &["canvas"]);
    cx.update(|cx| {
        cx.set_global(CanvasPaintBudget {
            slow: Duration::ZERO,
            slow_paints: 2,
            stall: Duration::from_secs(3600),
        })
    });
    render(cx);
    deliver_frames(cx, &launcher, &["canvas"]);
    assert!(bounds(cx, &node_id("canvas", 1)).is_some(), "it paints");
    assert_eq!(toast(cx), None);

    // One slow paint is let go; the second in a row is not.
    render(cx);
    assert!(
        launcher
            .core
            .borrow()
            .widget_frame(&key("canvas"))
            .is_some()
    );
    render(cx);
    cx.run_until_parked();
    render(cx);

    assert!(
        launcher
            .core
            .borrow()
            .widget_frame(&key("canvas"))
            .is_none()
    );
    assert!(
        bounds(cx, &node_id("canvas", 0)).is_none(),
        "its frame is gone"
    );
    assert!(
        bounds(cx, &tile_id("canvas")).is_none(),
        "its tile leaves the strip"
    );
    let toast = toast(cx).expect("a toast");
    assert!(
        toast.starts_with("Widget Widget canvas stopped: "),
        "{toast}"
    );
}

/// One paint past the stall budget stops it at once; but not the first
/// paint of a new frame, which may shape text and load fonts cold.
#[gpui_kit::test]
fn a_canvas_that_stalls_a_paint_is_stopped_at_once(cx: &mut TestAppContext) {
    let launcher = named_launcher(cx, &["canvas"]);
    cx.update(|cx| {
        cx.set_global(CanvasPaintBudget {
            slow: Duration::from_secs(3600),
            slow_paints: 5,
            stall: Duration::ZERO,
        })
    });
    render(cx);
    deliver_frames(cx, &launcher, &["canvas"]);
    // The first paint of the frame is judged: let go.
    render(cx);
    cx.run_until_parked();
    let frame = |launcher: &Launcher| launcher.core.borrow().widget_frame(&key("canvas"));
    assert!(frame(&launcher).is_some());
    assert_eq!(toast(cx), None);

    // The second is not.
    render(cx);
    cx.run_until_parked();
    assert!(frame(&launcher).is_none());
    assert!(toast(cx).is_some());
}

/// A canvas clips what it paints to its own box, not only to its tile.
#[gpui_kit::test]
fn a_canvas_clips_to_its_own_box(cx: &mut TestAppContext) {
    named_tiles(cx, &["small-canvas"]);

    let canvas = bounds(cx, &node_id("small-canvas", 1)).expect("the canvas");
    assert_eq!((canvas.size.width, canvas.size.height), (px(60.), px(30.)));
    let scale = scale_factor(cx);
    let quads = red_quads(cx);
    assert_eq!(quads.len(), 1, "{quads:?}");
    assert_eq!(quads[0].1, canvas.scale(scale));
}

/// The grace of a first paint is once per showing, not per frame: an
/// animated canvas, a new frame before every paint, that starts to stall
/// once it shows is stopped at its first stalled paint.
#[gpui_kit::test]
fn an_animated_canvas_that_stalls_is_stopped(cx: &mut TestAppContext) {
    let widget = "anim-canvas";
    let launcher = named_launcher(cx, &[widget]);
    let budget = |cx: &mut TestAppContext, stall| {
        cx.update(|cx| {
            cx.set_global(CanvasPaintBudget {
                slow: Duration::from_secs(3600),
                slow_paints: 5,
                stall,
            })
        })
    };
    budget(cx, Duration::MAX);
    render(cx);
    deliver_frames(cx, &launcher, &[widget]);
    let frame = |launcher: &Launcher| launcher.core.borrow().widget_frame(&key(widget));
    // A new frame, then a paint of it.
    let next = |cx: &mut TestAppContext| {
        let last = frame(&launcher).expect("a frame");
        cx.executor().advance_clock(Duration::from_millis(17));
        let now = cx.executor().now();
        launcher.core.borrow_mut().render_widget(&key(widget), now);
        // Applied without the controller, which would draw the window:
        // the one paint is the next render's.
        while frame(&launcher).is_some_and(|frame| Arc::ptr_eq(&frame, &last)) {
            let ev = wait_event(&launcher.core.borrow().events());
            launcher.core.borrow_mut().apply(ev, LocalNow::now());
        }
        render(cx);
    };
    // Shown and animating: its first paint has had its grace.
    for _ in 0..3 {
        next(cx);
    }
    assert!(frame(&launcher).is_some());

    // From now on every paint stalls: the next judged one stops it.
    budget(cx, Duration::ZERO);
    next(cx);

    cx.run_until_parked();
    assert!(frame(&launcher).is_none(), "stopped");
    assert!(toast(cx).is_some());
}

// Keyboard and pointer.

fn open_action() -> Action {
    Action {
        id: "open".into(),
        title: "Open".into(),
        shortcut: None,
    }
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

/// Whether element `id` has the keyboard focus.
fn focused(cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> bool {
    let w = window(cx);
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

/// Whether `provider/widget`'s tile shows its focus ring.
fn ring(cx: &mut TestAppContext, tile: &str) -> bool {
    render(cx);
    bounds(cx, &format!("widget-focus:{tile}")).is_some()
}

/// The labels of the highlighted rows of the list.
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

/// The labels of the rows of the root list, top to bottom.
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

/// The footer's hint for Enter.
fn footer_primary(cx: &mut TestAppContext) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find("footer-primary")
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

/// The labels of the open action panel's actions.
fn panel_actions(cx: &mut TestAppContext) -> Vec<String> {
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

/// The widget actions the module received: node, action and payload.
fn widget_actions(events: &[SeenWidgetEvent]) -> Vec<(Option<String>, String, Option<String>)> {
    events
        .iter()
        .filter_map(|(_, payload, event)| match event {
            WidgetEvent::Action { node, action } => {
                Some((node.clone(), action.clone(), payload.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The keys the module received.
fn widget_keys(events: &[SeenWidgetEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|(_, _, event)| match event {
            WidgetEvent::Key(key) => Some(key.key.clone()),
            _ => None,
        })
        .collect()
}

/// The two actions of calc's result tile.
const CALC_ACTIONS: [(&str, &str); 2] =
    [("copy", "Copy Result"), ("copy-all", "Copy Input + Result")];

/// Answers like calc: a sum of integers gets the condition tile `result`,
/// whose payload is the sum, a line break and the query.
fn calc_answers(query: &str) -> Vec<WidgetAnswer> {
    let terms: Option<Vec<i64>> = query.split('+').map(|t| t.trim().parse().ok()).collect();
    match terms {
        Some(terms) if terms.len() > 1 => vec![WidgetAnswer {
            widget: "result".into(),
            payload: format!("{}\n{query}", terms.iter().sum::<i64>()),
            actions: CALC_ACTIONS
                .iter()
                .map(|(id, title)| Action {
                    id: (*id).into(),
                    title: (*title).into(),
                    shortcut: None,
                })
                .collect(),
        }],
        _ => vec![],
    }
}

/// A `FakeModule` whose widget answers follow the query, as calc's do;
/// everything else is the fake's.
struct Calc(FakeModule);

impl Module for Calc {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn start(&mut self, ctx: ModuleContext) {
        self.0.start(ctx);
    }
    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        self.0.run_action(item, action_id)
    }
    fn is_dynamic_source(&self) -> bool {
        self.0.is_dynamic_source()
    }
    fn search(&mut self, query: &str) -> anyhow::Result<SearchAnswer> {
        let answer = self.0.search(query)?;
        Ok(SearchAnswer {
            widgets: calc_answers(query),
            ..answer
        })
    }
    fn widgets(&self) -> Vec<WidgetDecl> {
        self.0.widgets()
    }
    fn render_widget(&mut self, widget: &str, ctx: &WidgetContext) -> anyhow::Result<Frame> {
        self.0.render_widget(widget, ctx)
    }
    fn on_widget_event(
        &mut self,
        widget: &str,
        payload: Option<&str>,
        event: WidgetEvent,
    ) -> anyhow::Result<Vec<Effect>> {
        self.0.on_widget_event(widget, payload, event)
    }
}

/// The calc-like module `calc`: a dynamic source with no rows and the
/// widget `result`.
fn calc() -> FakeModule {
    FakeModule::new(vec![], nothing)
        .with_id("calc")
        .with_search(vec![])
        .with_widgets(vec![decl("result")])
        .with_render(still)
}

const CALC_TILE: &str = "calc/result";

/// The module `echo`: a dynamic source answering every query with the row
/// `Echo`, and no widgets.
fn echo() -> FakeModule {
    let row = Item {
        id: ItemId::new("echo", "echo"),
        ..item("echo", "Echo")
    };
    FakeModule::new(vec![], nothing)
        .with_id("echo")
        .with_search(vec![row])
}

/// Initializes the UI over a core with `modules`, once each has started,
/// and shows the launcher.
fn launch(cx: &mut TestAppContext, modules: Vec<Box<dyn Module>>) -> Rc<RefCell<Core>> {
    let mut core = new_core();
    let started = modules.len();
    for module in modules {
        core.register(module);
    }
    core.start_modules();
    for _ in 0..started {
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

/// Delivers core events until no dynamic source owes the latest search its
/// tiles, then what that queued.
fn deliver_answers(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    deliver_until(cx, core, |core| !core.widget_answers_pending());
    drain(cx, core);
    render(cx);
}

fn view(cx: &mut TestAppContext) -> gpui_kit::Entity<LauncherView> {
    let w = window(cx);
    cx.update(|cx| w.downcast::<LauncherView>().unwrap().root(cx).unwrap())
}

fn root_selection(cx: &mut TestAppContext) -> Option<String> {
    let view = view(cx);
    cx.update(|cx| view.read(cx).root_selection())
}

fn pushed_views(cx: &mut TestAppContext) -> Vec<String> {
    let view = view(cx);
    cx.update(|cx| view.read(cx).pushed_view_ids())
}

/// Ctrl+K, Cmd+K on macOS.
const TOGGLE_ACTIONS_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};

/// Whether no tile of `tiles` shows its focus ring.
fn no_ring(cx: &mut TestAppContext, tiles: &[&str]) -> bool {
    tiles.iter().all(|tile| !ring(cx, tile))
}

const ALWAYS_TILES: [&str; 3] = ["fake/a", "fake/b", "fake/c"];

/// Up climbs the screen: the first row, the strip's first tile, the field,
/// where a typed query has no history.
#[gpui_kit::test]
fn up_on_the_first_row_with_a_typed_query_enters_the_strip_then_the_field(cx: &mut TestAppContext) {
    setup(cx);
    type_text(cx, "f");
    assert!(!ring(cx, "fake/a"));
    assert_eq!(highlighted_rows(cx).len(), 1, "the first row is selected");

    press(cx, "up");

    assert!(ring(cx, "fake/a"));
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(query_value(cx), "f");

    press(cx, "up");
    assert!(no_ring(cx, &ALWAYS_TILES), "in the field");
    assert!(highlighted_rows(cx).is_empty());
    assert!(focused(cx, "launcher-input"));

    press(cx, "up");
    assert_eq!(query_value(cx), "f", "no history for a typed query");
    assert!(no_ring(cx, &ALWAYS_TILES));
    assert!(highlighted_rows(cx).is_empty());
}

/// At an empty query the chain is the same, and Up in the field recalls
/// the newest query.
#[gpui_kit::test]
fn up_at_an_empty_query_climbs_to_the_tile_the_field_then_the_history(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    assert!(launcher.core.borrow().history().record("files", 100));
    render(cx);
    assert_eq!(highlighted_rows(cx).len(), 1);

    press(cx, "up");
    assert!(ring(cx, "fake/a"));
    assert!(highlighted_rows(cx).is_empty());

    press(cx, "up");
    assert!(no_ring(cx, &ALWAYS_TILES));
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(query_value(cx), "", "entering the field recalls nothing");

    press(cx, "up");
    assert_eq!(query_value(cx), "files");
    assert!(no_ring(cx, &ALWAYS_TILES));
    assert!(highlighted_rows(cx).is_empty());
}

/// Down walks the chain back: the field, the strip's first tile, the
/// list's first row.
#[gpui_kit::test]
fn down_from_the_field_reaches_the_tile_then_the_first_row(cx: &mut TestAppContext) {
    setup(cx);
    render(cx);
    let first = row_titles(cx).first().cloned().unwrap();
    press(cx, "up");
    press(cx, "up");
    assert!(no_ring(cx, &ALWAYS_TILES));

    press(cx, "down");
    assert!(ring(cx, "fake/a"));
    assert!(highlighted_rows(cx).is_empty());

    press(cx, "down");
    assert!(no_ring(cx, &ALWAYS_TILES));
    assert_eq!(highlighted_rows(cx), [first]);
}

/// From a selected condition tile Up goes to the field, where it does
/// nothing; Down comes back to the tile, then the first row, from where Up
/// climbs again.
#[gpui_kit::test]
fn a_selected_condition_tile_climbs_to_the_field(cx: &mut TestAppContext) {
    let core = launch(cx, vec![Box::new(Calc(calc())), Box::new(echo())]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    press(cx, "up");
    assert!(!ring(cx, CALC_TILE), "in the field");
    assert!(highlighted_rows(cx).is_empty());
    press(cx, "up");
    assert_eq!(query_value(cx), "2+2");
    assert!(!ring(cx, CALC_TILE));
    assert!(highlighted_rows(cx).is_empty());

    press(cx, "down");
    assert!(ring(cx, CALC_TILE));
    press(cx, "down");
    assert!(!ring(cx, CALC_TILE));
    assert_eq!(highlighted_rows(cx), ["Echo"]);

    press(cx, "up");
    assert!(ring(cx, CALC_TILE));
    assert!(highlighted_rows(cx).is_empty());
    press(cx, "up");
    assert!(!ring(cx, CALC_TILE));
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(query_value(cx), "2+2");
}

/// Enter, Ctrl+K and the footer in the field are the list's first row's.
#[gpui_kit::test]
fn enter_in_the_field_runs_the_first_row(cx: &mut TestAppContext) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let echo = echo();
    let calls = echo.calls.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo)]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    press(cx, "up");
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(footer_primary(cx).as_deref(), Some("⏎ Run"));
    press(cx, TOGGLE_ACTIONS_KEY);
    assert_eq!(panel_actions(cx), ["Run"], "the first row's panel");
    press(cx, "escape");
    assert!(panel_actions(cx).is_empty());
    press(cx, "enter");

    poll("the row's action", || !calls.lock().unwrap().is_empty());
    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("echo", "echo"), "run".to_string())]
    );
    assert!(widget_actions(&events.lock().unwrap()).is_empty());
}

#[gpui_kit::test]
fn tab_at_an_empty_query_reaches_the_always_tiles(cx: &mut TestAppContext) {
    setup(cx);
    render(cx);
    assert_eq!(highlighted_rows(cx).len(), 1);

    press(cx, "tab");

    assert!(ring(cx, "fake/a"));
    assert!(
        highlighted_rows(cx).is_empty(),
        "the first row loses its highlight"
    );
    // Down goes back to the list and its first row.
    press(cx, "down");
    assert!(!ring(cx, "fake/a"));
    assert_eq!(highlighted_rows(cx).len(), 1);
}

#[gpui_kit::test]
fn the_first_row_stays_selected_at_an_empty_query(cx: &mut TestAppContext) {
    let launcher = setup(cx);
    render(cx);
    deliver_until(cx, &launcher.core, all_drawn);
    render(cx);

    let first = row_titles(cx).first().cloned();
    assert!(first.is_some());
    assert_eq!(highlighted_rows(cx), first.into_iter().collect::<Vec<_>>());
    assert!(WIDGETS.iter().all(|id| !ring(cx, &format!("fake/{id}"))));
}

#[gpui_kit::test]
fn enter_on_a_focused_tile_runs_its_primary_action(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["actions"]);

    press(cx, "tab");
    press(cx, "enter");

    poll("the tile's action", || {
        !widget_actions(&launcher.widget_events.lock().unwrap()).is_empty()
    });
    assert_eq!(
        widget_actions(&launcher.widget_events.lock().unwrap()),
        [(None, "open".to_string(), None)]
    );
    assert!(cx.update(|cx| cx.global::<LauncherController>().is_visible()));
}

#[gpui_kit::test]
fn tab_reaches_a_button_which_shows_its_ring_and_enter_presses_it(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["control"]);
    let button_ring = "widget-focus:fake/control:1";

    press(cx, "tab");
    render(cx);
    assert!(bounds(cx, button_ring).is_none());
    press(cx, "tab");
    render(cx);
    assert!(
        bounds(cx, button_ring).is_some(),
        "the focused button's ring"
    );
    assert!(ring(cx, "fake/control"), "and its tile's");
    assert_eq!(selected(cx, &tile_id("control")), Some(true));
    assert_eq!(selected(cx, &node_id("control", 1)), Some(true));
    press(cx, "enter");

    poll("the button's action", || {
        !widget_actions(&launcher.widget_events.lock().unwrap()).is_empty()
    });
    assert_eq!(
        widget_actions(&launcher.widget_events.lock().unwrap()),
        [(Some("reset".to_string()), "reset".to_string(), None)]
    );
}

#[gpui_kit::test]
fn a_condition_tile_starts_selected_and_enter_copies_it(cx: &mut TestAppContext) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let core = launch(cx, vec![Box::new(Calc(calc))]);

    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE), "the condition tile is selected");
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(footer_primary(cx).as_deref(), Some("⏎ Copy Result"));
    press(cx, "enter");

    poll("the copy", || {
        !widget_actions(&events.lock().unwrap()).is_empty()
    });
    assert_eq!(
        widget_actions(&events.lock().unwrap()),
        [(None, "copy".to_string(), Some("4\n2+2".to_string()))]
    );
    assert_eq!(query_value(cx), "2+2");
    assert!(
        focused(cx, "launcher-input"),
        "the keyboard stays in the field"
    );
}

/// While typing on, the selected tile keeps its selection until its new
/// answer lands: the highlight never jumps to the list's row and back.
#[gpui_kit::test]
fn the_selected_tile_keeps_its_selection_while_its_new_answer_is_on_the_way(
    cx: &mut TestAppContext,
) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo())]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    type_text(cx, "3");

    assert!(ring(cx, CALC_TILE), "the stale tile keeps the selection");
    assert!(
        highlighted_rows(cx).is_empty(),
        "{:?}",
        highlighted_rows(cx)
    );
    assert_eq!(
        footer_primary(cx),
        None,
        "no hint: Enter runs the new answer's action or, if it misses the wait, the row's"
    );
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));
    assert!(highlighted_rows(cx).is_empty());
    assert_eq!(footer_primary(cx).as_deref(), Some("⏎ Copy Result"));
    // Its actions waited for the new answer: Enter copies `2+23`'s.
    press(cx, "enter");
    poll("the copy", || {
        !widget_actions(&events.lock().unwrap()).is_empty()
    });
    assert_eq!(
        widget_actions(&events.lock().unwrap()),
        [(None, "copy".to_string(), Some("25\n2+23".to_string()))]
    );
}

/// Hiding drops the last query's tile, and the next showing starts with
/// none; the hide of the window that went never reaches the next one.
#[gpui_kit::test]
fn the_next_showing_has_no_old_tile_and_answers_anew(cx: &mut TestAppContext) {
    let core = launch(cx, vec![Box::new(Calc(calc()))]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    hide_launcher(cx);
    show(cx);

    assert!(core.borrow().strip().is_empty(), "no old tile");
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE), "the new answer's tile is selected");
}

/// A stale tile keeps its selection, but an Enter that waited for its new
/// answer in vain acts on the list's row, never on the tile.
#[gpui_kit::test]
fn enter_on_a_stale_selected_tile_falls_back_to_the_first_row(cx: &mut TestAppContext) {
    let delay = Arc::new(AtomicU64::new(0));
    let calc = calc().with_render_delay(delay.clone());
    let events = calc.widget_events.clone();
    let echo = echo();
    let calls = echo.calls.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo)]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));
    delay.store(300, Ordering::SeqCst);

    type_text(cx, "3");
    // Echo's row for `2+23` arrives; the tile is still being drawn.
    loop {
        let ev = wait_event(&core.borrow().events());
        let echoed = matches!(&ev, tomakast_core::CoreEvent::Instance { provider, .. } if provider == "echo");
        let update = core.borrow_mut().apply(ev, LocalNow::now());
        hand(cx, update);
        cx.run_until_parked();
        if echoed {
            break;
        }
    }
    assert!(ring(cx, CALC_TILE), "the stale tile keeps the selection");
    assert!(core.borrow().widget_answers_pending());
    press(cx, "enter");
    cx.executor().advance_clock(Duration::from_millis(116));
    cx.run_until_parked();

    poll("the row's action", || !calls.lock().unwrap().is_empty());
    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("echo", "echo"), "run".to_string())]
    );
    assert!(widget_actions(&events.lock().unwrap()).is_empty());
    deliver_answers(cx, &core);
    assert!(widget_actions(&events.lock().unwrap()).is_empty());
}

#[gpui_kit::test]
fn enter_waits_for_the_new_answer_and_never_acts_on_an_older_one(cx: &mut TestAppContext) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let echo = echo();
    let calls = echo.calls.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo)]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    // 1. Typed and pressed before the answers to `2+23` are in.
    type_text(cx, "3");
    let typed = events.lock().unwrap().len();
    press(cx, "enter");
    std::thread::sleep(Duration::from_millis(20));
    cx.run_until_parked();
    assert!(widget_actions(&events.lock().unwrap()).is_empty());
    assert!(calls.lock().unwrap().is_empty(), "no row ran");

    // 2. The answers arrive: the Enter acts on the new tile, once.
    deliver_answers(cx, &core);
    poll("the copy", || {
        !widget_actions(&events.lock().unwrap()).is_empty()
    });
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &core);
    assert_eq!(
        widget_actions(&events.lock().unwrap()),
        [(None, "copy".to_string(), Some("25\n2+23".to_string()))]
    );
    assert!(calls.lock().unwrap().is_empty(), "the tile was on screen");

    // 3. Nothing carried the old payload since the `3`.
    let events = events.lock().unwrap();
    assert!(
        events[typed..]
            .iter()
            .all(|(_, payload, _)| payload.as_deref() != Some("4\n2+2")),
        "{events:?}"
    );
}

#[gpui_kit::test]
fn enter_falls_back_to_the_first_row_when_no_tile_arrives_in_time(cx: &mut TestAppContext) {
    let delay = Arc::new(AtomicU64::new(0));
    let calc = calc().with_render_delay(delay.clone());
    let events = calc.widget_events.clone();
    let echo = echo();
    let calls = echo.calls.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo)]);
    delay.store(300, Ordering::SeqCst);

    type_text(cx, "2+2");
    // The row arrives; the tile is still being drawn.
    while root_selection(cx).as_deref() != Some("Echo") {
        let ev = wait_event(&core.borrow().events());
        let update = core.borrow_mut().apply(ev, LocalNow::now());
        hand(cx, update);
        cx.run_until_parked();
    }
    assert!(core.borrow().widget_answers_pending());
    press(cx, "enter");
    cx.executor().advance_clock(Duration::from_millis(115));
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(20));
    assert!(calls.lock().unwrap().is_empty(), "still waiting at 115 ms");

    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();

    poll("the row's action", || !calls.lock().unwrap().is_empty());
    assert_eq!(
        *calls.lock().unwrap(),
        [(ItemId::new("echo", "echo"), "run".to_string())]
    );
    // The late tile comes, and nothing acts on it.
    deliver_answers(cx, &core);
    assert!(widget_actions(&events.lock().unwrap()).is_empty());
}

#[gpui_kit::test]
fn ctrl_k_opens_the_tiles_actions(cx: &mut TestAppContext) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let core = launch(cx, vec![Box::new(Calc(calc))]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);

    press(cx, TOGGLE_ACTIONS_KEY);

    assert_eq!(panel_actions(cx), ["Copy Result", "Copy Input + Result"]);
    press(cx, "down");
    press(cx, "enter");
    poll("the action", || {
        !widget_actions(&events.lock().unwrap()).is_empty()
    });
    assert_eq!(
        widget_actions(&events.lock().unwrap()),
        [(None, "copy-all".to_string(), Some("4\n2+2".to_string()))]
    );
}

#[gpui_kit::test]
fn ctrl_k_waits_like_enter(cx: &mut TestAppContext) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo())]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);

    type_text(cx, "3");
    press(cx, TOGGLE_ACTIONS_KEY);
    assert!(panel_actions(cx).is_empty(), "no panel yet");

    deliver_answers(cx, &core);
    assert_eq!(panel_actions(cx), ["Copy Result", "Copy Input + Result"]);
    press(cx, "enter");
    poll("the action", || {
        !widget_actions(&events.lock().unwrap()).is_empty()
    });
    assert_eq!(
        widget_actions(&events.lock().unwrap()),
        [(None, "copy".to_string(), Some("25\n2+23".to_string()))]
    );
}

#[gpui_kit::test]
fn keys_in_a_focused_canvas_reach_the_plugin(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["anim"]);

    press(cx, "tab");
    press(cx, "tab");
    press(cx, "a");
    press(cx, "left");

    poll("both keys", || {
        widget_keys(&launcher.widget_events.lock().unwrap()).len() >= 2
    });
    assert_eq!(
        widget_keys(&launcher.widget_events.lock().unwrap()),
        ["a", "left"]
    );
    assert_eq!(query_value(cx), "");
    // Escape leaves the canvas for its tile, and typing goes to the field again.
    press(cx, "escape");
    assert!(cx.update(|cx| cx.global::<LauncherController>().is_visible()));
    type_text(cx, "f");
    assert_eq!(query_value(cx), "f");
    assert_eq!(
        widget_keys(&launcher.widget_events.lock().unwrap()),
        ["a", "left"]
    );
}

fn push_form() -> anyhow::Result<Vec<Effect>> {
    let field = |id: &str| tomakast_core::FormField {
        id: id.into(),
        label: id.into(),
        kind: tomakast_core::FieldKind::Checkbox { value: false },
        error: None,
    };
    Ok(vec![Effect::PushView(tomakast_core::View::Form(
        tomakast_core::FormView {
            id: tomakast_core::ViewId("fake/form".into()),
            title: "Form".into(),
            fields: vec![field("one"), field("two")],
            submit_title: "Save".into(),
        },
    ))])
}

#[gpui_kit::test]
fn tab_in_a_form_still_moves_between_fields(cx: &mut TestAppContext) {
    let module = FakeModule::new(items(), push_form)
        .with_widgets(WIDGETS.iter().map(|id| decl(id)).collect())
        .with_render(still);
    let launcher = setup_with(cx, module, &WIDGETS);
    render(cx);
    press(cx, "enter");
    while pushed_views(cx).is_empty() {
        let ev = wait_event(&launcher.core.borrow().events());
        let update = launcher.core.borrow_mut().apply(ev, LocalNow::now());
        hand(cx, update);
        cx.run_until_parked();
    }
    render(cx);
    assert!(focused(cx, ("form-field", 0usize)));

    press(cx, "tab");

    assert!(focused(cx, ("form-field", 1usize)));
    press(cx, "escape");
    render(cx);
    assert!(!ring(cx, "fake/a"), "the strip took no Tab");
    assert_eq!(highlighted_rows(cx).len(), 1);
}

#[gpui_kit::test]
fn clicking_a_button_sends_its_action(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["control"]);
    let w = window(cx);

    cx.update_window(w, |_, window, cx| window.click(node_id("control", 1), cx))
        .unwrap();
    cx.run_until_parked();

    poll("the button's action", || {
        !widget_actions(&launcher.widget_events.lock().unwrap()).is_empty()
    });
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert_eq!(
        widget_actions(&launcher.widget_events.lock().unwrap()),
        [(Some("reset".to_string()), "reset".to_string(), None)],
        "the button's, not the tile's too"
    );
    assert!(
        focused(cx, "launcher-input"),
        "a click leaves the keyboard in the field"
    );

    // Elsewhere on the tile: its primary action.
    cx.update_window(w, |_, window, cx| window.click(node_id("control", 2), cx))
        .unwrap();
    cx.run_until_parked();
    poll("the tile's action", || {
        widget_actions(&launcher.widget_events.lock().unwrap()).len() >= 2
    });
    assert_eq!(
        widget_actions(&launcher.widget_events.lock().unwrap())[1],
        (None, "open".to_string(), None)
    );
    assert!(focused(cx, "launcher-input"));
    type_text(cx, "f");
    assert_eq!(query_value(cx), "f");
}

/// Moves the pointer to `at`, in window coordinates, and draws a frame, as
/// `hover` does.
fn move_pointer(cx: &mut TestAppContext, at: gpui_kit::Point<Pixels>) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: at,
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

#[gpui_kit::test]
fn hover_moves_are_coalesced_per_frame(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["canvas"]);
    let canvas = bounds(cx, &node_id("canvas", 1)).expect("the canvas");
    let pointer = |events: &[SeenWidgetEvent]| -> Vec<(PointerKind, tomakast_core::Point)> {
        events
            .iter()
            .filter_map(|(_, _, event)| match event {
                WidgetEvent::Pointer(p) => Some((p.kind, p.at)),
                _ => None,
            })
            .collect()
    };

    for x in [10., 20., 30.] {
        move_pointer(cx, canvas.origin + point(px(x), px(5.)));
    }
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    render(cx);

    poll("the pointer events", || {
        pointer(&launcher.widget_events.lock().unwrap()).len() >= 2
    });
    std::thread::sleep(Duration::from_millis(20));
    // Where it came onto the canvas, then the last of the three moves.
    let at = |x| tomakast_core::Point { x, y: 5. };
    assert_eq!(
        pointer(&launcher.widget_events.lock().unwrap()),
        [(PointerKind::Enter, at(10.)), (PointerKind::Move, at(30.))]
    );
}

#[gpui_kit::test]
fn up_and_down_walk_the_history_while_a_recalled_tile_is_selected(cx: &mut TestAppContext) {
    let core = launch(cx, vec![Box::new(Calc(calc()))]);
    assert!(core.borrow().history().record("1+1", 100));
    assert!(core.borrow().history().record("2+2", 200));

    // No tiles at the empty query: Up goes from the row-less list to the
    // field, then into the history.
    press(cx, "up");
    assert_eq!(query_value(cx), "");
    press(cx, "up");
    assert_eq!(query_value(cx), "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE), "the recalled query's tile is selected");

    press(cx, "up");
    assert_eq!(query_value(cx), "1+1", "Up is the history's");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    press(cx, "down");
    assert_eq!(query_value(cx), "2+2", "Down is the history's");
    deliver_answers(cx, &core);
    press(cx, "down");
    assert_eq!(query_value(cx), "", "past the newest: the empty field");
    assert!(!ring(cx, CALC_TILE), "which keeps no tile");
    press(cx, "up");
    assert_eq!(query_value(cx), "2+2", "still in the field: Up recalls");
}

/// An emptied field keeps no tile selected, even while the tile selected
/// for the query just cleared is still on the strip; Up climbs through
/// that tile to the field, then the history.
#[gpui_kit::test]
fn up_on_an_emptied_field_recalls_the_history(cx: &mut TestAppContext) {
    let core = launch(cx, vec![Box::new(Calc(calc()))]);
    assert!(core.borrow().history().record("1+1", 100));
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    assert!(ring(cx, CALC_TILE));

    for _ in 0.."2+2".len() {
        press(cx, "backspace");
    }
    assert_eq!(query_value(cx), "");
    assert!(!ring(cx, CALC_TILE), "the emptied field keeps no tile");
    assert!(
        !core.borrow().strip().is_empty(),
        "the old tile still shows"
    );

    press(cx, "up");
    assert!(ring(cx, CALC_TILE));
    press(cx, "up");
    assert!(!ring(cx, CALC_TILE));
    assert_eq!(query_value(cx), "");
    press(cx, "up");

    assert_eq!(query_value(cx), "1+1", "Up in the field is the history's");
}

/// The accessible selected state of element `id`.
fn selected(cx: &mut TestAppContext, id: &str) -> Option<bool> {
    let w = window(cx);
    let id = id.to_string();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).selected()
    })
    .unwrap()
}

#[gpui_kit::test]
fn keys_from_the_field_never_reach_a_canvas(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["anim"]);
    press(cx, "tab");
    press(cx, "tab");
    let w = window(cx);

    // The keyboard goes back to the field by a click.
    cx.update_window(w, |_, window, cx| window.click("launcher-input", cx))
        .unwrap();
    cx.run_until_parked();
    type_text(cx, "f");
    press(cx, "up");
    press(cx, "down");
    press(cx, "enter");

    std::thread::sleep(Duration::from_millis(20));
    assert!(widget_keys(&launcher.widget_events.lock().unwrap()).is_empty());
    assert_eq!(query_value(cx), "f");
    assert!(!ring(cx, "fake/anim"));
}

#[gpui_kit::test]
fn tab_while_enter_waits_drops_the_enter(cx: &mut TestAppContext) {
    let calc = calc();
    let events = calc.widget_events.clone();
    let echo = echo();
    let calls = echo.calls.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo)]);
    type_text(cx, "2+2");
    deliver_answers(cx, &core);
    type_text(cx, "3");
    press(cx, "enter");

    press(cx, "tab");
    deliver_answers(cx, &core);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();

    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &core);
    assert!(widget_actions(&events.lock().unwrap()).is_empty());
    assert!(calls.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn an_older_answer_landing_after_enter_is_never_acted_on(cx: &mut TestAppContext) {
    let delay = Arc::new(AtomicU64::new(50));
    let calc = calc().with_render_delay(delay.clone());
    let (events, renders) = (calc.widget_events.clone(), calc.renders.clone());
    let echo = echo();
    let calls = echo.calls.clone();
    let core = launch(cx, vec![Box::new(Calc(calc)), Box::new(echo)]);

    type_text(cx, "2+2");
    // `2+2`'s tile is being drawn: its answer comes after the Enter.
    poll("the 2+2 render", || renders.load(Ordering::SeqCst) > 0);
    delay.store(0, Ordering::SeqCst);
    type_text(cx, "3");
    press(cx, "enter");
    deliver_answers(cx, &core);

    poll("the copy", || {
        !widget_actions(&events.lock().unwrap()).is_empty()
    });
    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &core);
    assert_eq!(
        widget_actions(&events.lock().unwrap()),
        [(None, "copy".to_string(), Some("25\n2+23".to_string()))]
    );
    assert!(calls.lock().unwrap().is_empty());
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .all(|(_, payload, _)| payload.as_deref() != Some("4\n2+2")),
        "{events:?}"
    );
}

#[gpui_kit::test]
fn arrows_scroll_a_focused_tile_into_view(cx: &mut TestAppContext) {
    // Only the tiles on screen draw.
    let launcher = named_launcher(cx, &["a", "b", "c", "d", "e"]);
    render(cx);
    deliver_frames(cx, &launcher, &["a", "b", "c"]);
    let strip = bounds(cx, "widget-strip").unwrap();
    assert!(
        bounds(cx, &tile_id("e")).unwrap().left() >= strip.right(),
        "off screen"
    );

    press(cx, "tab");
    for _ in 0..4 {
        press(cx, "right");
    }
    render(cx);

    assert!(ring(cx, "fake/e"));
    let e = bounds(cx, &tile_id("e")).unwrap();
    assert!(
        e.left() >= strip.left() && e.right() <= strip.right(),
        "{e:?} in {strip:?}"
    );
    for _ in 0..4 {
        press(cx, "left");
    }
    render(cx);
    let a = bounds(cx, &tile_id("a")).unwrap();
    assert_eq!(a.left() - strip.left(), px(16.));
}

/// The tiles of `ALWAYS_TILES` that show their focus ring.
fn ringed(cx: &mut TestAppContext) -> Vec<&'static str> {
    ALWAYS_TILES
        .into_iter()
        .filter(|tile| ring(cx, tile))
        .collect()
}

/// In the strip, Left and Right move between the tiles at every press,
/// with the caret anywhere in the field: the field never takes them, and
/// the caret stays where it was.
#[gpui_kit::test]
fn left_and_right_in_the_strip_never_move_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    type_text(cx, "fi");
    // The caret between `f` and `i`: the field could move it either way.
    press(cx, "left");
    press(cx, "up");
    assert_eq!(ringed(cx), ["fake/a"]);

    for round in 0..5 {
        press(cx, "right");
        assert_eq!(ringed(cx), ["fake/b"], "right, round {round}");
        press(cx, "left");
        assert_eq!(ringed(cx), ["fake/a"], "left, round {round}");
    }
    press(cx, "right");
    press(cx, "right");
    assert_eq!(ringed(cx), ["fake/c"]);
    press(cx, "right");
    assert_eq!(ringed(cx), ["fake/c"], "the last tile keeps it");
    for _ in 0..3 {
        press(cx, "left");
    }
    assert_eq!(ringed(cx), ["fake/a"], "the first tile keeps it");

    assert!(
        focused(cx, "launcher-input"),
        "the keyboard stays in the field"
    );
    type_text(cx, "X");
    assert_eq!(query_value(cx), "fXi", "the caret did not move");
}

/// With the selection in the list or in the field, Left and Right move
/// the caret, as without a strip.
#[gpui_kit::test]
fn left_and_right_in_the_list_and_the_field_move_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    type_text(cx, "fi");
    assert_eq!(highlighted_rows(cx).len(), 1, "the first row is selected");

    press(cx, "left");
    type_text(cx, "X");
    assert_eq!(query_value(cx), "fXi");
    press(cx, "right");
    type_text(cx, "Y");
    assert_eq!(query_value(cx), "fXiY");
    assert!(ringed(cx).is_empty(), "the strip took neither");

    // Up twice: the strip, then the field.
    press(cx, "up");
    press(cx, "up");
    assert!(ringed(cx).is_empty(), "in the field");
    press(cx, "left");
    press(cx, "left");
    type_text(cx, "Z");
    assert_eq!(query_value(cx), "fXZiY");
    press(cx, "right");
    type_text(cx, "W");
    assert_eq!(query_value(cx), "fXZiWY");
    assert!(ringed(cx).is_empty());
}

#[gpui_kit::test]
fn clicks_on_tiles_do_nothing_while_the_panel_is_open(cx: &mut TestAppContext) {
    let launcher = named_tiles(cx, &["control"]);
    press(cx, TOGGLE_ACTIONS_KEY);
    assert!(!panel_actions(cx).is_empty(), "the list row's panel");
    let w = window(cx);

    cx.update_window(w, |_, window, cx| window.click(node_id("control", 1), cx))
        .unwrap();
    cx.update_window(w, |_, window, cx| window.click(node_id("control", 2), cx))
        .unwrap();
    cx.run_until_parked();

    std::thread::sleep(Duration::from_millis(20));
    drain(cx, &launcher.core);
    assert!(widget_actions(&launcher.widget_events.lock().unwrap()).is_empty());
}

#[gpui_kit::test]
fn a_canvas_that_leaves_under_the_pointer_gets_leave(cx: &mut TestAppContext) {
    SWAP_CANVAS.store(true, Ordering::SeqCst);
    let launcher = named_tiles(cx, &["swap"]);
    let canvas = bounds(cx, &node_id("swap", 1)).expect("the canvas");
    move_pointer(cx, canvas.origin + point(px(10.), px(5.)));
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    render(cx);
    let kinds = |launcher: &Launcher| -> Vec<PointerKind> {
        launcher
            .widget_events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, _, event)| match event {
                WidgetEvent::Pointer(p) => Some(p.kind),
                _ => None,
            })
            .collect()
    };
    poll("enter and move", || kinds(&launcher).len() >= 2);

    // The next frame has no canvas.
    SWAP_CANVAS.store(false, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_millis(17));
    let last = launcher.core.borrow().widget_frame(&key("swap"));
    let now = cx.executor().now();
    launcher.core.borrow_mut().render_widget(&key("swap"), now);
    deliver_until(cx, &launcher.core, |core| {
        core.widget_frame(&key("swap")) != last
    });
    render(cx);

    poll("the leave", || kinds(&launcher).len() >= 3);
    assert_eq!(
        kinds(&launcher),
        [PointerKind::Enter, PointerKind::Move, PointerKind::Leave]
    );
}
