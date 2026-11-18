//! Headless tests of the scratchpad view.
//!
//! As in `stack.rs`, module calls run on real worker threads and the test
//! thread hands each reply to the controller, as the daemon does. The
//! debounce timer runs on GPUI's simulated clock (`advance_clock`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, Bounds, Pixels, QuitMode, ScrollDelta,
    TestAppContext, point,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreUpdate, Effect, Item, ItemId, ItemKind, LineAnnotation, LocalNow, RankWeights,
    ScratchpadView, View, ViewEvent, ViewId,
};
use tomakast_ui::{LauncherController, LauncherView};

const DEBOUNCE: Duration = Duration::from_millis(30);

fn items() -> Vec<Item> {
    vec![Item {
        id: ItemId::new("fake", "pad"),
        title: "Pad".into(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Command,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
        }],
    }]
}

fn note(line: u32, text: &str) -> LineAnnotation {
    LineAnnotation {
        line,
        text: text.into(),
        is_error: false,
    }
}

fn pad(text: String, revision: u64, annotations: Vec<LineAnnotation>) -> ScratchpadView {
    ScratchpadView {
        id: ViewId("fake/pad".into()),
        title: "Pad".into(),
        text,
        revision,
        annotations,
    }
}

/// Review Focus 5: a short line, an empty one, a line far wider than the
/// view, and one after it.
fn push_mixed_lines() -> anyhow::Result<Vec<Effect>> {
    let text = format!("1+1\n\n{}\n2*3", "9".repeat(300));
    Ok(vec![Effect::PushView(View::Scratchpad(pad(
        text,
        0,
        vec![note(0, "2"), note(3, "6")],
    )))])
}

/// Far more lines than the view shows, each annotated.
fn push_tall() -> anyhow::Result<Vec<Effect>> {
    let text = (0..80)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let annotations = (0..80).map(|i| note(i, &format!("= {i}"))).collect();
    Ok(vec![Effect::PushView(View::Scratchpad(pad(
        text,
        0,
        annotations,
    )))])
}

/// Module text with line breaks and other control characters in it.
fn push_multiline_note() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Scratchpad(pad(
        "1/0\n1+1\n3".into(),
        0,
        vec![
            LineAnnotation {
                line: 0,
                text: "parse error:\nexpected )\r\tat 3".into(),
                is_error: true,
            },
            note(1, "2"),
        ],
    )))])
}

fn push_empty() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Scratchpad(pad(
        String::new(),
        0,
        Vec::new(),
    )))])
}

/// Answers "abc" with an update one revision behind it, anything else with
/// an update at the event's revision. Both try to replace the text.
fn annotate(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::TextChanged { revision, text } = event else {
        return Ok(vec![]);
    };
    let (revision, label) = if text == "abc" {
        (revision - 1, "stale")
    } else {
        (revision, "fresh")
    };
    Ok(vec![Effect::UpdateView(View::Scratchpad(pad(
        "from the module".into(),
        revision,
        vec![note(0, label)],
    )))])
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

fn type_text(cx: &mut TestAppContext, text: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn wait(cx: &mut TestAppContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

fn render(cx: &mut TestAppContext) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.render_frame(cx))
        .unwrap();
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

/// Accessible label and bounds of the annotation of `line`, as last painted.
fn annotation(cx: &mut TestAppContext, line: usize) -> Option<(String, Bounds<Pixels>)> {
    let w = window(cx);
    cx.update_window(w, |_, window, _| {
        window
            .try_find(("scratchpad-annotation", line))
            .map(|e| (e.label().unwrap_or_default().to_string(), e.bounds()))
    })
    .unwrap()
}

/// Where the editor laid out the start of `line`, as last painted.
fn line_bounds(cx: &mut TestAppContext, line: usize) -> Option<Bounds<Pixels>> {
    let w = window(cx);
    cx.update_window(w, |root, _, cx| {
        let view = root.downcast::<LauncherView>().unwrap();
        view.read(cx).scratchpad_line_bounds(line, cx)
    })
    .unwrap()
}

fn bounds_of(cx: &mut TestAppContext, id: &'static str) -> Bounds<Pixels> {
    let w = window(cx);
    cx.update_window(w, |_, window, _| window.find(id).bounds())
        .unwrap()
}

fn assert_aligned(cx: &mut TestAppContext, line: usize) {
    let editor = line_bounds(cx, line).unwrap_or_else(|| panic!("line {line} is laid out"));
    let (_, note) = annotation(cx, line).unwrap_or_else(|| panic!("line {line} has its note"));
    assert!(
        (note.origin.y - editor.origin.y).abs() < Pixels::from(0.5),
        "line {line}: note at {:?}, text at {:?}",
        note.origin.y,
        editor.origin.y
    );
    assert_eq!(note.size.height, editor.size.height, "line {line} height");
}

type Events = Arc<Mutex<Vec<(ViewId, ViewEvent)>>>;

fn pushed(cx: &mut TestAppContext, module: FakeModule) -> (Rc<RefCell<Core>>, Events) {
    let events = module.view_events.clone();
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    // The editor lays out in the first frame; the column reads it.
    render(cx);
    (core, events)
}

fn text_events(events: &Events) -> Vec<ViewEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, e)| e.clone())
        .filter(|e| matches!(e, ViewEvent::TextChanged { .. }))
        .collect()
}

/// Review Focus 5.
#[gpui_kit::test]
fn annotations_align_with_lines(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_mixed_lines).with_view_events(nothing),
    );
    assert_eq!(label_of(cx, "view-header").as_deref(), Some("Pad"));

    assert_aligned(cx, 0);
    assert_aligned(cx, 3);
    assert_eq!(annotation(cx, 0).unwrap().0, "2");
    assert_eq!(annotation(cx, 3).unwrap().0, "6");
    assert!(annotation(cx, 1).is_none(), "line 1 has no annotation");
    assert!(annotation(cx, 2).is_none(), "line 2 has no annotation");

    // No wrap: the 300-character line takes one row, so line 3 is three
    // rows below line 0.
    let first = line_bounds(cx, 0).unwrap();
    let fourth = line_bounds(cx, 3).unwrap();
    assert_eq!(fourth.origin.y - first.origin.y, first.size.height * 3.);

    // The column is 30% of the view, beside the editor.
    let editor = bounds_of(cx, "scratchpad-editor");
    let column = bounds_of(cx, "scratchpad-annotations");
    let view = column.right() - editor.left();
    assert!(
        (column.size.width - view * 0.3).abs() < Pixels::from(1.),
        "column {:?} of {view:?}",
        column.size.width
    );
    assert!(editor.right() <= column.left());
}

#[gpui_kit::test]
fn long_lines_scroll_horizontally(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_mixed_lines).with_view_events(nothing),
    );
    let text_x = line_bounds(cx, 0).unwrap().origin.x;
    let (_, note) = annotation(cx, 0).unwrap();

    // To the end of the 300-character line: the text scrolls left, the
    // column stays where it is.
    press(cx, "down");
    press(cx, "down");
    press(cx, "end");
    render(cx);
    let scrolled_x = line_bounds(cx, 0).unwrap().origin.x;
    assert!(scrolled_x < text_x, "text at {text_x:?} -> {scrolled_x:?}");
    assert_eq!(annotation(cx, 0).unwrap().1, note);
    assert_aligned(cx, 0);
    assert_aligned(cx, 3);
}

#[gpui_kit::test]
fn annotations_follow_the_editor_scroll(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_tall).with_view_events(nothing),
    );
    assert!(line_bounds(cx, 79).is_none(), "the last line is off screen");

    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.scroll("scratchpad-editor", ScrollDelta::Lines(point(0., -20.)), cx)
    })
    .unwrap();
    assert!(line_bounds(cx, 0).is_none(), "the editor scrolled down");

    // Every line the editor laid out has its note beside it; no other does.
    let mut shown = 0;
    for line in 0..80 {
        match line_bounds(cx, line) {
            Some(_) => {
                assert_aligned(cx, line);
                shown += 1;
            }
            None => assert!(annotation(cx, line).is_none(), "line {line} is not shown"),
        }
    }
    assert!(shown > 0);

    // The wheel over the annotation column scrolls the editor too.
    let top = (0..80).find(|&l| line_bounds(cx, l).is_some()).unwrap();
    cx.update_window(w, |_, window, cx| {
        window.scroll(
            "scratchpad-annotations",
            ScrollDelta::Lines(point(0., -10.)),
            cx,
        )
    })
    .unwrap();
    let new_top = (0..80).find(|&l| line_bounds(cx, l).is_some()).unwrap();
    assert!(new_top > top, "top line {top} -> {new_top}");
    assert_aligned(cx, new_top + 2);
}

#[gpui_kit::test]
fn stale_annotations_ignored(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_empty).with_view_events(annotate),
    );

    // Revision 3; the module answers for revision 2.
    type_text(cx, "abc");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    assert_eq!(
        text_events(&events),
        [ViewEvent::TextChanged {
            revision: 3,
            text: "abc".into()
        }]
    );
    assert!(annotation(cx, 0).is_none(), "the stale update is ignored");

    // At the current revision it applies, and leaves the text alone.
    type_text(cx, "d");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    assert_eq!(annotation(cx, 0).map(|a| a.0).as_deref(), Some("fresh"));
    type_text(cx, "e");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    assert_eq!(
        text_events(&events).last(),
        Some(&ViewEvent::TextChanged {
            revision: 5,
            text: "abcde".into()
        })
    );
}

#[gpui_kit::test]
fn text_changed_debounced(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_empty).with_view_events(nothing),
    );

    // Five keys within 20 ms.
    for ch in ["h", "e", "l", "l", "o"] {
        type_text(cx, ch);
        wait(cx, Duration::from_millis(4));
    }
    // 30 ms after the last key, not before.
    wait(cx, Duration::from_millis(25));
    std::thread::sleep(Duration::from_millis(50));
    assert!(text_events(&events).is_empty(), "nothing before 30 ms");

    wait(cx, Duration::from_millis(1));
    deliver(cx, &core);
    wait(cx, Duration::from_millis(100));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        text_events(&events),
        [ViewEvent::TextChanged {
            revision: 5,
            text: "hello".into()
        }]
    );
}

#[gpui_kit::test]
fn enter_inserts_a_newline_and_escape_pops(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_empty).with_view_events(nothing),
    );
    type_text(cx, "1");
    press(cx, "enter");
    type_text(cx, "2");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    assert_eq!(
        text_events(&events).last(),
        Some(&ViewEvent::TextChanged {
            revision: 3,
            text: "1\n2".into()
        })
    );

    press(cx, "escape");
    assert_eq!(label_of(cx, "view-header"), None, "back on the root list");
}

#[gpui_kit::test]
fn a_multiline_annotation_stays_on_its_line(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_multiline_note).with_view_events(nothing),
    );
    assert_aligned(cx, 0);
    assert_aligned(cx, 1);
    // The text itself stays inside its row.
    let w = window(cx);
    let text = cx
        .update_window(w, |_, window, _| {
            window.find(("scratchpad-annotation-text", 0usize)).bounds()
        })
        .unwrap();
    let (_, row) = annotation(cx, 0).unwrap();
    assert!(
        text.top() >= row.top() && text.bottom() <= row.bottom(),
        "text {text:?} outside its row {row:?}"
    );
    let (_, error) = annotation(cx, 0).unwrap();
    let (_, next) = annotation(cx, 1).unwrap();
    assert!(error.bottom() <= next.top(), "{error:?} overlaps {next:?}");
    // One row: the breaks are shown as spaces.
    assert_eq!(
        annotation(cx, 0).unwrap().0,
        "parse error: expected )  at 3"
    );
}
