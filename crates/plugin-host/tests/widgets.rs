//! Widgets through the host: kitchen-sink's tiles rendered, answered and clicked through a
//! core, with the host's caps on frames, widget answers, images and widget events.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::harness::*;
use tomakast_core::widget::{
    Frame, KeyEvent, NodeKind, Point, PointerEvent, PointerKind, WidgetEvent, WidgetKey,
};
use tomakast_core::{Effect, Icon, Modifiers, ToastKind, View};
use tomakast_plugin_host::Opener;

const SINK: &str = "kitchen-sink/indexed";

/// Sets the mode the `probe` widget renders in (see the kitchen-sink crate doc).
fn probe_mode(h: &mut Harness, mode: &str) {
    h.run(SINK, &format!("render:{mode}"))
        .unwrap_or_else(|e| panic!("render:{mode} failed: {e}"));
}

/// The texts of `frame`'s text nodes, in order.
fn texts(frame: &Frame) -> Vec<&str> {
    frame
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// Renders `widget` until the stored frame passes `drawn`, at most 3 times: a frame that
/// misses the 16 ms soft deadline on a loaded machine is dropped, and 3 stay below the 5 late
/// frames that stop a widget.
fn render_until(h: &mut Harness, widget: &str, drawn: impl Fn(&Frame) -> bool) -> Arc<Frame> {
    let mut last = None;
    for _ in 0..3 {
        last = h.render(widget);
        if let Some(frame) = &last
            && drawn(frame)
        {
            return Arc::clone(frame);
        }
    }
    panic!("{widget} was not drawn in 3 renders; the last frame: {last:?}");
}

/// `probe` rendered in `ok` mode: a column holding the text `ok`.
fn ok_frame(h: &mut Harness) -> Arc<Frame> {
    probe_mode(h, "ok");
    render_until(h, "probe", |frame| texts(frame) == ["ok"])
}

/// What reached the opener; nothing is opened.
#[derive(Default)]
struct FakeOpener(Mutex<Vec<String>>);

impl Opener for FakeOpener {
    fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(format!("url {url}"));
        Ok(())
    }

    fn open_path(&self, path: &Path) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push(format!("path {}", path.display()));
        Ok(())
    }
}

/// The plugin still answers: its command runs and `echo:x` is answered.
fn assert_alive(h: &mut Harness) {
    assert!(h.run("kitchen-sink/hello", "run").is_ok());
    assert_eq!(h.search("echo:x").expect("the plugin answers").len(), 1);
}

fn copies(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CopyText(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn pointer(node: &str, kind: PointerKind) -> WidgetEvent {
    WidgetEvent::Pointer(PointerEvent {
        node: node.into(),
        kind,
        at: Point { x: 4., y: 4. },
    })
}

/// A harness whose `echo` tile answered `tile:hi`.
fn echo_tile() -> Harness {
    let mut h = harness();
    let answer = h.search_answer("tile:hi");
    assert_eq!(answer.widgets.len(), 1, "{answer:?}");
    h
}

/// The next toast is the host's denial of a `copy-text` that is no user action's.
fn assert_copy_denied(h: &mut Harness) {
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    let message = toast.message.unwrap_or_default();
    assert!(
        message.contains("copy-text needs a user action"),
        "{message}"
    );
}

#[test]
fn kitchen_sink_shows_three_always_tiles_from_init() {
    let h = harness();
    assert_eq!(h.core.always_widgets(ID), ["static", "buttons", "anim"]);
}

#[test]
fn a_frame_round_trips_through_the_host() {
    let mut h = harness();
    let frame = render_until(&mut h, "static", |_| true);
    assert!(
        matches!(frame.nodes[0].kind, NodeKind::Column(_)),
        "{frame:?}"
    );
    assert_eq!(frame.nodes.len(), 3, "{frame:?}");
    for node in &frame.nodes[1..] {
        assert_eq!(node.parent, Some(0));
        assert!(matches!(node.kind, NodeKind::Text(_)), "{node:?}");
    }
    assert_eq!(
        texts(&frame),
        ["Static", "A column with a title and a body"]
    );
}

#[test]
fn a_spinning_render_breaks_the_plugin_at_100_ms() {
    let mut h = harness();
    probe_mode(&mut h, "spin");
    let started = Instant::now();
    assert_eq!(h.render("probe"), None);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_broken(&mut h, "render-widget did not answer within 100 ms");
}

/// A render over the soft deadline and under the hard one (100 ms of wall time) is dropped,
/// and the plugin lives. The core holds frames to 2 ms of the worker's CPU time here, not
/// 16 ms (64 ms in a debug build): the probe's 30 ms busy-wait is late in either build however
/// loaded the machine is.
#[test]
fn a_slow_render_is_dropped_but_the_plugin_lives() {
    let mut h = harness();
    let ok = ok_frame(&mut h);
    h.core.set_soft_render_deadline(Duration::from_millis(2));
    probe_mode(&mut h, "slow:30");

    assert_eq!(h.render("probe"), Some(ok), "the late frame is dropped");
    assert_eq!(h.search("echo:x").expect("the plugin answers").len(), 1);
}

#[test]
fn a_frame_over_a_cap_is_dropped_not_fatal() {
    let mut h = harness();
    let ok = ok_frame(&mut h);
    for mode in ["big", "bad-tree", "err"] {
        probe_mode(&mut h, mode);
        assert_eq!(
            h.render("probe"),
            Some(ok.clone()),
            "{mode}: the last frame stays"
        );
    }
    assert!(
        h.plugin_events.try_recv().is_err(),
        "the plugin is not broken"
    );
    assert_alive(&mut h);
    // A good frame is stored again.
    assert_eq!(ok_frame(&mut h), ok);
}

#[test]
fn svg_with_external_references_is_refused_when_sent() {
    let mut h = harness();
    let ok = ok_frame(&mut h);
    for kind in ["href", "script", "huge"] {
        probe_mode(&mut h, &format!("bad-svg:{kind}"));
        assert_eq!(h.render("probe"), Some(ok.clone()), "bad-svg:{kind}");
        assert!(
            h.core.widget_image(ID, "bad-svg").is_none(),
            "bad-svg:{kind}"
        );
    }
    for format in ["svg", "png", "jpeg"] {
        probe_mode(&mut h, &format!("image:{format}"));
        let frame = h.render("probe").unwrap();
        assert_eq!(texts(&frame), [format!("image:{format}")]);
        let id = format!("probe-{format}");
        assert!(h.core.widget_image(ID, &id).is_some(), "{id} is stored");
    }
}

/// Ruling U holds in frames too: a `plugin-file` icon reaches the core as the canonical path
/// of a file in the plugin folder, and one that leaves the folder refuses the frame.
#[test]
fn plugin_file_icons_in_frames_resolve_inside_the_folder() {
    let mut h = harness();
    probe_mode(&mut h, "icon:icons/echo.png");
    let frame = h.render("probe").expect("the icon frame");
    let NodeKind::Icon(node) = &frame.nodes[1].kind else {
        panic!("an icon node: {frame:?}");
    };
    let Icon::PluginFile(path) = &node.icon else {
        panic!("a plugin file: {node:?}");
    };
    let folder = loaded().2.canonicalize().unwrap();
    assert_eq!(*path, folder.join("icons/echo.png"));
    for bad in ["missing.png", "icons"] {
        probe_mode(&mut h, &format!("icon:{bad}"));
        assert_eq!(
            h.render("probe"),
            Some(frame.clone()),
            "{bad}: the last frame stays"
        );
    }
}

#[test]
fn the_image_budget_holds() {
    // `init` already sent kitchen-sink's two `anim` images: 62 more fill the budget.
    let mut h = harness();
    let err = h.run(SINK, "image-flood:63").unwrap_err();
    assert!(err.contains("more than 64 images"), "{err}");
    assert!(
        h.core.widget_image(ID, "flood0").is_none(),
        "a rejected answer stores no image"
    );

    h.run(SINK, "image-flood:62").expect("64 images fit");
    assert!(h.core.widget_image(ID, "flood61").is_some());
    let (before, _) = h.core.widget_image(ID, "flood0").unwrap();
    // Replacing an id frees its old bytes first: twice, and the count stays at 64.
    // (other bytes each time: the same bytes would keep the version).
    for format in ["jpeg", "svg"] {
        h.run(SINK, &format!("set-image:flood0:{format}"))
            .expect("a replaced image fits");
    }
    let (after, _) = h.core.widget_image(ID, "flood0").unwrap();
    assert!(after > before, "flood0 was replaced");
    let err = h.run(SINK, "set-image:one-more:png").unwrap_err();
    assert!(err.contains("more than 64 images"), "{err}");
    assert!(h.core.widget_image(ID, "one-more").is_none());
}

#[test]
fn a_condition_tile_answers_with_its_payload() {
    let mut h = harness();
    let answer = h.search_answer("tile:hi");
    let [tile] = &answer.widgets[..] else {
        panic!("one widget answer: {answer:?}");
    };
    assert_eq!(tile.widget, "echo");
    assert_eq!(tile.payload, "hi");
    let actions: Vec<&str> = tile.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions, ["copy", "copy-twice"]);
    let frame = h
        .core
        .widget_frame(&WidgetKey::new(ID, "echo"))
        .expect("the tile's frame");
    assert_eq!(texts(&frame), ["hi"]);
}

#[test]
fn quarter_tiles_are_drawn_at_a_quarter_of_the_strip() {
    let mut h = harness();
    let answer = h.search_answer("quarters");
    let widgets: Vec<(&str, &str)> = answer
        .widgets
        .iter()
        .map(|w| (w.widget.as_str(), w.payload.as_str()))
        .collect();
    assert_eq!(widgets, [("quarter-a", "A"), ("quarter-b", "B")]);
    for (widget, payload) in [("quarter-a", "A"), ("quarter-b", "B")] {
        let frame = h
            .core
            .widget_frame(&WidgetKey::new(ID, widget))
            .expect("the tile's frame");
        assert_eq!(texts(&frame), ["Quarter", payload, "173.5 x 120"]);
    }
    let sizes: Vec<_> = h
        .core
        .strip()
        .into_iter()
        .map(|tile| (tile.key.widget, tile.size))
        .collect();
    use tomakast_core::WidgetSize::{Quarter, Single};
    assert_eq!(
        sizes,
        [
            ("quarter-a".to_string(), Quarter),
            ("quarter-b".to_string(), Quarter),
            ("static".to_string(), Single),
            ("buttons".to_string(), Single),
            ("anim".to_string(), Single),
        ]
    );
}

#[test]
fn answering_an_undeclared_widget_rejects_the_search() {
    let mut h = harness();
    let answer = h.search_answer("tile-undeclared");
    assert!(answer.items.is_empty(), "{answer:?}");
    assert!(answer.widgets.is_empty(), "{answer:?}");
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    let message = toast.message.unwrap_or_default();
    assert!(message.contains("\"nope\""), "{message}");
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none(), "one denial toast");
}

#[test]
fn widget_action_is_a_user_action() {
    let mut h = echo_tile();
    let copy = |action: &str| WidgetEvent::Action {
        node: None,
        action: action.into(),
    };
    let effects = h.widget_event("echo", copy("copy")).unwrap();
    assert_eq!(copies(&effects), ["hi"]);
    let effects = h.widget_event("echo", copy("copy-twice")).unwrap();
    assert_eq!(copies(&effects), ["hi", "hi"]);
}

#[test]
fn a_key_is_a_user_action() {
    let mut h = echo_tile();
    let key = WidgetEvent::Key(KeyEvent {
        node: "tile".into(),
        key: "c".into(),
        modifiers: Modifiers::empty(),
    });
    let effects = h.widget_event("echo", key).unwrap();
    assert_eq!(copies(&effects), ["hi"]);
}

#[test]
fn hover_is_not_a_user_action() {
    let mut h = echo_tile();
    let effects = h
        .widget_event("echo", pointer("tile", PointerKind::Move))
        .unwrap();
    assert!(copies(&effects).is_empty(), "{effects:?}");
    assert_copy_denied(&mut h);
}

#[test]
fn a_press_alone_is_not_a_user_action() {
    let mut h = echo_tile();
    let effects = h
        .widget_event("echo", pointer("tile", PointerKind::Down))
        .unwrap();
    assert!(copies(&effects).is_empty(), "{effects:?}");
    assert_copy_denied(&mut h);
}

#[test]
fn a_completed_click_is_a_user_action() {
    let mut h = echo_tile();
    let down = h
        .widget_event("echo", pointer("tile", PointerKind::Down))
        .unwrap();
    assert!(copies(&down).is_empty(), "{down:?}");
    let up = h
        .widget_event("echo", pointer("tile", PointerKind::Up))
        .unwrap();
    assert_eq!(copies(&up), ["hi"]);
    // The click is over: another up completes none.
    let again = h
        .widget_event("echo", pointer("tile", PointerKind::Up))
        .unwrap();
    assert!(copies(&again).is_empty(), "{again:?}");
}

#[test]
fn an_up_without_a_down_is_not_a_user_action() {
    let mut h = echo_tile();
    let up = h
        .widget_event("echo", pointer("tile", PointerKind::Up))
        .unwrap();
    assert!(copies(&up).is_empty(), "{up:?}");
    assert_copy_denied(&mut h);
}

#[test]
fn an_up_on_another_node_is_not_one() {
    let mut h = echo_tile();
    h.widget_event("echo", pointer("a", PointerKind::Down))
        .unwrap();
    let up = h
        .widget_event("echo", pointer("b", PointerKind::Up))
        .unwrap();
    assert!(copies(&up).is_empty(), "{up:?}");
    // Any up ends the press: a later up on the pressed node completes no click either.
    let late = h
        .widget_event("echo", pointer("a", PointerKind::Up))
        .unwrap();
    assert!(copies(&late).is_empty(), "{late:?}");
}

#[test]
fn unsolicited_widget_effects_are_rate_limited() {
    let mut h = harness();
    // `init`'s show-widgets counts in the same 1 s window: let it pass first.
    std::thread::sleep(Duration::from_millis(1100));
    h.run(SINK, "timer-reply:10:image-flood:31").unwrap();
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    let message = toast.message.unwrap_or_default();
    assert!(
        message.contains("more than 30 unsolicited show-widgets and set-widget-image per s"),
        "{message}"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.core.widget_image(ID, "flood29").is_none() {
        assert!(Instant::now() < deadline, "the first 30 images arrive");
        h.update();
    }
    assert!(
        h.core.widget_image(ID, "flood30").is_none(),
        "the 31st is dropped"
    );
}

/// The always tiles `init` shows draw valid frames, their buttons act, and `show-widgets` and
/// `redraw-widget` from `run` follow the declared ids.
#[test]
fn the_always_tiles_draw_and_act() {
    let mut h = harness();
    let buttons = h.render("buttons").expect("the buttons frame");
    let actions: Vec<&str> = buttons
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Button(button) => Some(button.action.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(actions, ["inc", "reset"]);
    let anim = h.render("anim").expect("the anim frame");
    assert!(anim.animate);
    assert!(
        anim.nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Canvas(c) if c.focusable)),
        "{anim:?}"
    );

    let inc = WidgetEvent::Action {
        node: Some("inc".into()),
        action: "inc".into(),
    };
    let effects = h.widget_event("buttons", inc).unwrap();
    assert!(
        matches!(&effects[..], [Effect::Toast(t)] if t.title.ends_with("inc")),
        "{effects:?}"
    );

    h.run(SINK, "show:probe,echo").unwrap();
    assert_eq!(h.core.always_widgets(ID), ["probe", "echo"]);
    h.run(SINK, "redraw:probe").unwrap();
    for bad in ["show:nope", "redraw:nope", "show:probe,probe"] {
        let err = h.run(SINK, bad).unwrap_err();
        assert!(err.contains("invalid answer"), "{bad}: {err}");
    }
    assert_eq!(h.core.always_widgets(ID), ["probe", "echo"]);
}

/// Hovering and pressing are no user actions: what an answer to them may do only in answer to
/// a user action (close the launcher, push or pop a view) is dropped. The click they complete
/// is one, and its answer passes.
#[test]
fn only_a_completed_click_closes_pushes_or_pops() {
    let mut h = harness();
    for node in ["close", "push", "pop"] {
        for kind in [PointerKind::Move, PointerKind::Down] {
            let effects = h.widget_event("probe", pointer(node, kind)).unwrap();
            assert!(effects.is_empty(), "{node} {kind:?}: {effects:?}");
        }
        let up = h
            .widget_event("probe", pointer(node, PointerKind::Up))
            .unwrap();
        let passed = match node {
            "close" => matches!(&up[..], [Effect::CloseWindow]),
            "push" => matches!(&up[..], [Effect::PushView(View::Detail(d))] if d.id.0 == "pushed"),
            _ => matches!(&up[..], [Effect::PopView]),
        };
        assert!(passed, "{node}: {up:?}");
    }
    let toast = h.toast();
    let message = toast.message.unwrap_or_default();
    assert!(
        message
            .contains("close-window needs a user action (an action, a key or a completed click)"),
        "{message}"
    );
}

/// `open` needs a user action: refused while a tile draws, allowed in answer to its action.
#[test]
fn open_needs_a_widget_user_action() {
    let opener = Arc::new(FakeOpener::default());
    let fake: Arc<dyn Opener> = opener.clone();
    let mut h = harness_built(
        |_| {},
        BTreeMap::new(),
        move |plugin| plugin.with_opener(fake),
    );
    let url = "https://example.com/";
    probe_mode(&mut h, &format!("open-url:{url}"));
    let frame = render_until(&mut h, "probe", |frame| texts(frame) != ["ok"]);
    let [result] = texts(&frame)[..] else {
        panic!("one text: {frame:?}");
    };
    assert!(
        result.starts_with("Err(") && result.contains("needs a user action"),
        "{result}"
    );
    assert!(
        opener.0.lock().unwrap().is_empty(),
        "nothing opened while drawing"
    );

    let action = WidgetEvent::Action {
        node: None,
        action: format!("open-url:{url}"),
    };
    let effects = h.widget_event("probe", action).unwrap();
    assert_eq!(metadata(&effects)["result"], "Ok(())");
    assert_eq!(*opener.0.lock().unwrap(), [format!("url {url}")]);
}
