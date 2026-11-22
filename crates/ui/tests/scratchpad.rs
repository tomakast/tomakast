//! Headless tests of the scratchpad view.
//!
//! As in `stack.rs`, module calls run on real worker threads and the test
//! thread hands each reply to the controller, as the daemon does. The
//! debounce timer runs on GPUI's simulated clock (`advance_clock`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, Bounds, Pixels, QuitMode, ScrollDelta,
    TestAppContext, point,
};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreUpdate, Effect, Item, ItemId, ItemKind, LineAnnotation, LocalNow, MenuOption,
    RankWeights, ScratchpadView, ToolbarButton, View, ViewEvent, ViewId,
};
use tomakast_ui::{LauncherController, LauncherView};

#[path = "support/font_recorder.rs"]
mod font_recorder;
use font_recorder::{FontRecorder, families_of, features_of};

const DEBOUNCE: Duration = Duration::from_millis(30);

fn items() -> Vec<Item> {
    vec![Item {
        id: ItemId::new("fake", "pad"),
        title: "Pad".into(),
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
    }]
}

fn note(line: u32, text: &str) -> LineAnnotation {
    LineAnnotation {
        line,
        text: text.into(),
        is_error: false,
        swatch: None,
    }
}

fn pad(text: String, revision: u64, annotations: Vec<LineAnnotation>) -> ScratchpadView {
    ScratchpadView {
        id: ViewId("fake/pad".into()),
        title: "Pad".into(),
        text,
        revision,
        annotations,
        actions: Vec::new(),
        output: None,
        toolbar: Vec::new(),
        busy: false,
        placeholder: String::new(),
        detail: None,
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
                swatch: None,
            },
            note(1, "2"),
        ],
    )))])
}

/// A colour line with a swatch, then a number line without one.
fn push_swatch() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Scratchpad(pad(
        "#ff8800\n2+2".into(),
        0,
        vec![
            LineAnnotation {
                swatch: Some("#ff8800".into()),
                ..note(0, "rgb(255, 136, 0)")
            },
            note(1, "4"),
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

#[gpui_kit::test]
fn annotation_swatch_is_drawn(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_swatch).with_view_events(nothing),
    );
    let w = window(cx);
    let (swatch, text, other) = cx
        .update_window(w, |_, window, cx| {
            window.render_frame(cx);
            (
                window
                    .try_find(("scratchpad-annotation-swatch", 0usize))
                    .map(|e| e.bounds()),
                window.find(("scratchpad-annotation-text", 0usize)).bounds(),
                window
                    .try_find(("scratchpad-annotation-swatch", 1usize))
                    .map(|e| e.bounds()),
            )
        })
        .unwrap();
    let swatch = swatch.expect("line 0 has a swatch");
    assert!(other.is_none(), "line 1 has no swatch");
    let (_, row) = annotation(cx, 0).unwrap();
    assert!(
        swatch.top() >= row.top() && swatch.bottom() <= row.bottom(),
        "swatch {swatch:?} outside its row {row:?}"
    );
    assert!(
        swatch.right() <= text.left(),
        "swatch {swatch:?} after {text:?}"
    );
    assert!(swatch.size.width > Pixels::ZERO && swatch.size.height > Pixels::ZERO);
}

/// Ruling AQ: a pad with actions. Enter still types a newline; the action
/// panel (Ctrl+K) and the actions' shortcuts run them, with the view's id as
/// the row id, as for a detail view.
fn push_pad_with_actions() -> anyhow::Result<Vec<Effect>> {
    let mut view = pad("1+1".into(), 0, vec![note(0, "2")]);
    view.actions = vec![
        Action {
            id: "copy-result".into(),
            title: "Copy result".into(),
            shortcut: None,
        },
        Action {
            id: "history".into(),
            title: "History".into(),
            shortcut: Some("ctrl+shift+h".into()),
        },
    ];
    Ok(vec![Effect::PushView(View::Scratchpad(view))])
}

fn chosen(events: &Events) -> Vec<ViewEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, e)| e.clone())
        .filter(|e| matches!(e, ViewEvent::ActionChosen { .. }))
        .collect()
}

#[gpui_kit::test]
fn pad_actions_run_from_the_panel_not_from_enter(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_pad_with_actions).with_view_events(nothing),
    );
    // The footer names the pad's first action with the key that runs it, Ctrl+Enter (Enter
    // types a newline), and the action panel.
    let enter = if cfg!(target_os = "macos") {
        "⌘+↵"
    } else {
        "Ctrl+↵"
    };
    assert_eq!(
        label_of(cx, "footer-primary").as_deref(),
        Some(format!("{enter} Copy result").as_str())
    );
    assert!(label_of(cx, "footer-actions").is_some(), "the Actions hint");
    press(cx, "enter");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    assert!(chosen(&events).is_empty(), "Enter ran a pad action");

    press(cx, "ctrl-k");
    assert!(label_of(cx, "action-panel").is_some(), "the panel opens");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(
        chosen(&events),
        [ViewEvent::ActionChosen {
            row_id: "fake/pad".into(),
            action_id: "copy-result".into(),
        }]
    );

    press(cx, "ctrl-shift-h");
    deliver(cx, &core);
    assert_eq!(
        chosen(&events).last(),
        Some(&ViewEvent::ActionChosen {
            row_id: "fake/pad".into(),
            action_id: "history".into(),
        })
    );
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

/// Ruling 9: a daemon run (a hotkey or `tomakast run`) that pushes the pad while it is
/// open leaves one pad. The old one's waiting edit reaches the module first; it is not
/// closed, and the new content starts fresh, at its own revision.
#[gpui_kit::test]
fn daemon_run_of_an_open_pad_leaves_one_pad(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_empty).with_view_events(nothing),
    );
    type_text(cx, "abc");
    core.borrow_mut()
        .request_action(&ItemId::new("fake", "pad"), "run", "")
        .unwrap();
    deliver(cx, &core);
    // The pending edit, sent before the old content went.
    deliver(cx, &core);
    assert_eq!(pushed_ids(cx), ["fake/pad"]);
    assert_eq!(
        text_events(&events),
        [ViewEvent::TextChanged {
            revision: 3,
            text: "abc".into()
        }]
    );
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, e)| *e == ViewEvent::Closed),
        "the pad is replaced, not closed"
    );

    // The new pad counts its own revisions from its push.
    type_text(cx, "z");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    assert_eq!(
        text_events(&events).last(),
        Some(&ViewEvent::TextChanged {
            revision: 1,
            text: "z".into()
        })
    );
    press(cx, "escape");
    assert_eq!(label_of(cx, "view-header"), None, "back on the root list");
}

/// Feedback (calc): a sheet reads like code, so its text and its results are
/// in the bundled monospace font, with its ligatures, and its lines line up
/// in columns.
#[test]
fn pad_text_and_results_are_monospace() {
    let recorder = Arc::new(FontRecorder::default());
    let mut cx = TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        None,
        recorder.clone(),
    );
    let cx = &mut cx;
    let core = setup(
        cx,
        FakeModule::new(items(), push_mixed_lines).with_view_events(nothing),
    );
    press(cx, "enter");
    deliver(cx, &core);
    render(cx);
    render(cx);
    let mono = "JetBrains Mono";
    let ligatures = vec![vec![("calt".to_owned(), 1), ("liga".to_owned(), 1)]];
    assert_eq!(families_of(&recorder, "1+1"), [mono], "the editor's text");
    assert_eq!(
        features_of(&recorder, "1+1"),
        ligatures,
        "the editor's text"
    );
    assert_eq!(families_of(&recorder, "6"), [mono], "an annotation");
    assert_eq!(features_of(&recorder, "6"), ligatures, "an annotation");
}

/// The launcher adds its ten bundled faces, and every text of the root list
/// and of a pushed view is in a bundled family: none is left in the
/// platform's `.SystemUIFont`, which names whatever the machine has (and may
/// lack the weights the launcher draws).
#[test]
fn launcher_text_is_in_the_bundled_fonts() {
    let recorder = Arc::new(FontRecorder::default());
    let mut cx = TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        None,
        recorder.clone(),
    );
    let cx = &mut cx;
    let core = setup(
        cx,
        FakeModule::new(items(), push_mixed_lines).with_view_events(nothing),
    );
    assert_eq!(recorder.added(), 10, "the bundled faces");
    render(cx);
    assert_eq!(families_of(&recorder, "Pad"), ["Inter"], "a root row");
    press(cx, "enter");
    deliver(cx, &core);
    render(cx);
    render(cx);
    assert_eq!(recorder.families(), ["Inter", "JetBrains Mono"]);
}

/// A compose pad: `output` set, so the editor wraps and the output shows below it.
fn compose(text: String, revision: u64, output: &str) -> ScratchpadView {
    let mut view = pad(text, revision, Vec::new());
    view.output = Some(output.into());
    view
}

fn push_compose() -> anyhow::Result<Vec<Effect>> {
    let long = "word ".repeat(80);
    Ok(vec![Effect::PushView(View::Scratchpad(compose(
        format!("{long}\nnext"),
        0,
        "**Hola**",
    )))])
}

fn push_empty_compose() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Scratchpad(compose(
        String::new(),
        0,
        "Type to translate",
    )))])
}

/// Answers "abc" with an output one revision behind, anything else with an output at the
/// event's revision that names the text.
fn translate(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::TextChanged { revision, text } = event else {
        return Ok(vec![]);
    };
    let (revision, label) = if text == "abc" {
        (revision - 1, "stale".to_string())
    } else {
        (revision, format!("fresh {text}"))
    };
    Ok(vec![Effect::UpdateView(View::Scratchpad(compose(
        "from the module".into(),
        revision,
        &label,
    )))])
}

#[gpui_kit::test]
fn compose_pad_shows_output_below_the_editor(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_compose).with_view_events(nothing),
    );
    let output = label_of(cx, "scratchpad-output").expect("the output pane");
    assert!(output.contains("Hola"), "{output:?}");
    assert!(
        label_of(cx, "scratchpad-annotations").is_none(),
        "no annotation column"
    );
    // Two columns, each the whole height: the editor, then the output beside it.
    let editor = bounds_of(cx, "scratchpad-editor");
    let pane = bounds_of(cx, "scratchpad-output");
    assert!(pane.left() >= editor.right(), "{pane:?} beside {editor:?}");
    assert!(
        (pane.top() - editor.top()).abs() < Pixels::from(1.),
        "{pane:?} level with {editor:?}"
    );
    assert!(
        (pane.size.width - editor.size.width).abs() < Pixels::from(24.),
        "about half each: {pane:?} {editor:?}"
    );
}

#[gpui_kit::test]
fn compose_pad_wraps(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_compose).with_view_events(nothing),
    );
    let first = line_bounds(cx, 0).unwrap();
    let second = line_bounds(cx, 1).unwrap();
    assert!(
        second.origin.y - first.origin.y > first.size.height * 1.5,
        "the 400-character line wraps: {first:?} then {second:?}"
    );
}

#[gpui_kit::test]
fn compose_pad_output_updates_at_the_revision(cx: &mut TestAppContext) {
    let (core, _) = pushed(
        cx,
        FakeModule::new(items(), push_empty_compose).with_view_events(translate),
    );
    type_text(cx, "abc");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    let output = label_of(cx, "scratchpad-output").unwrap();
    assert!(
        output.contains("Type to translate"),
        "stale ignored: {output:?}"
    );

    type_text(cx, "d");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    let output = label_of(cx, "scratchpad-output").unwrap();
    assert!(output.contains("fresh abcd"), "{output:?}");
}

fn view_events(events: &Events) -> Vec<ViewEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, e)| e.clone())
        .collect()
}

#[gpui_kit::test]
fn ctrl_enter_runs_the_pads_first_action(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_pad_with_actions).with_view_events(nothing),
    );
    press(cx, "end");
    press(cx, "ctrl-enter");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    assert_eq!(
        chosen(&events),
        [ViewEvent::ActionChosen {
            row_id: "fake/pad".into(),
            action_id: "copy-result".into(),
        }]
    );
    assert!(
        text_events(&events).is_empty(),
        "Ctrl+Enter typed nothing: {:?}",
        view_events(&events)
    );
}

#[gpui_kit::test]
fn pad_action_sends_the_waiting_edit_first(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_pad_with_actions).with_view_events(nothing),
    );
    press(cx, "end");
    type_text(cx, "x");
    // Within the debounce: the edit is still waiting when the action runs.
    press(cx, "ctrl-enter");
    // Two answers: the edit's and the action's.
    deliver(cx, &core);
    deliver(cx, &core);
    let seen = view_events(&events);
    assert!(
        matches!(
            seen.as_slice(),
            [ViewEvent::TextChanged { text, .. }, ViewEvent::ActionChosen { .. }] if text == "1+1x"
        ),
        "{seen:?}"
    );

    // From the panel too.
    type_text(cx, "y");
    press(cx, "ctrl-k");
    press(cx, "enter");
    deliver(cx, &core);
    deliver(cx, &core);
    let seen = view_events(&events);
    assert!(
        matches!(
            &seen[2..],
            [ViewEvent::TextChanged { text, .. }, ViewEvent::ActionChosen { .. }] if text == "1+1xy"
        ),
        "{seen:?}"
    );
}

fn option(id: &str, title: &str, native: &str, russian: &str) -> MenuOption {
    MenuOption {
        id: id.into(),
        title: title.into(),
        subtitle: Some(native.into()),
        icon: None,
        keywords: vec![russian.into()],
    }
}

/// A compose pad with a toolbar: `swap`, a plain button on Ctrl+S, and `lang`, a menu of
/// three languages on Ctrl+P with English current.
fn toolbar_pad(revision: u64, output: &str) -> ScratchpadView {
    let mut view = compose("hello".into(), revision, output);
    view.toolbar = vec![
        ToolbarButton {
            id: "swap".into(),
            label: "⇄".into(),
            icon: None,
            shortcut: Some("ctrl+s".into()),
            options: vec![],
            selected: None,
        },
        ToolbarButton {
            id: "lang".into(),
            label: "English".into(),
            icon: None,
            shortcut: Some("ctrl+p".into()),
            options: vec![
                option("de", "German", "Deutsch", "немецкий"),
                option("en", "English", "English", "английский"),
                option("fr", "French", "Français", "французский"),
            ],
            selected: Some("en".into()),
        },
    ];
    view
}

fn push_toolbar() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Scratchpad(toolbar_pad(
        0, "out",
    )))])
}

/// A sheet (no output) with a toolbar: the toolbar is not shown.
fn push_sheet_with_toolbar() -> anyhow::Result<Vec<Effect>> {
    let mut view = toolbar_pad(0, "out");
    view.output = None;
    Ok(vec![Effect::PushView(View::Scratchpad(view))])
}

/// Answers every edit with the toolbar pad at the edit's revision, its output naming the text.
fn retoolbar(_: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    let ViewEvent::TextChanged { revision, text } = event else {
        return Ok(vec![]);
    };
    Ok(vec![Effect::UpdateView(View::Scratchpad(toolbar_pad(
        revision,
        &format!("updated {text}"),
    )))])
}

fn click(cx: &mut TestAppContext, id: (&'static str, usize)) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn bounds_at(cx: &mut TestAppContext, id: (&'static str, usize)) -> Bounds<Pixels> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).bounds()
    })
    .unwrap()
}

/// The titles of the menu's rows, as shown.
fn menu_rows(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        (0..)
            .map_while(|i| window.try_find(("action", i as usize)))
            .map(|e| e.label().unwrap_or_default().to_string())
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn toolbar_shows_in_a_compose_pad_only(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    assert!(label_of(cx, "scratchpad-toolbar").is_some());
    // In the header, over both columns: the first button over the editor, the last over
    // the output.
    let header = bounds_of(cx, "view-header");
    let bar = bounds_of(cx, "scratchpad-toolbar");
    let editor = bounds_of(cx, "scratchpad-editor");
    let pane = bounds_of(cx, "scratchpad-output");
    assert!(
        bar.top() >= header.top() && bar.bottom() <= header.bottom(),
        "{bar:?} in {header:?}"
    );
    assert!(bar.bottom() <= editor.top());
    let first = bounds_at(cx, ("toolbar-button", 0));
    let last = bounds_at(cx, ("toolbar-button", 1));
    assert!(first.right() <= pane.left(), "{first:?} over the editor");
    assert!(
        last.left() >= editor.right() - Pixels::from(40.),
        "{last:?} over the output"
    );
}

#[gpui_kit::test]
fn a_sheet_shows_no_toolbar(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_sheet_with_toolbar).with_view_events(nothing),
    );
    assert!(label_of(cx, "scratchpad-toolbar").is_none());
}

/// Review Focus 5: the waiting edit goes before the button's action.
#[gpui_kit::test]
fn plain_button_click_sends_the_action_after_the_edit(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    press(cx, "end");
    type_text(cx, "x");
    click(cx, ("toolbar-button", 0));
    deliver(cx, &core);
    deliver(cx, &core);
    let seen = view_events(&events);
    assert!(
        matches!(
            seen.as_slice(),
            [ViewEvent::TextChanged { text, .. }, ViewEvent::ActionChosen { row_id, action_id }]
                if text == "hellox" && row_id == "fake/pad" && action_id == "swap"
        ),
        "{seen:?}"
    );
    // Its shortcut runs it too.
    press(cx, "ctrl-s");
    deliver(cx, &core);
    assert!(matches!(
        view_events(&events).last(),
        Some(ViewEvent::ActionChosen { action_id, .. }) if action_id == "swap"
    ));
}

/// Review Focus 1: the menu sits under its button, opened by a click or by the shortcut.
#[gpui_kit::test]
fn menu_opens_under_its_button(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    let button = bounds_at(cx, ("toolbar-button", 1));
    for open in ["click", "shortcut"] {
        match open {
            "click" => click(cx, ("toolbar-button", 1)),
            _ => press(cx, "ctrl-p"),
        }
        assert_eq!(menu_rows(cx), ["German", "English", "French"], "{open}");
        let menu = bounds_of(cx, "action-panel");
        assert!(
            menu.top() >= button.bottom() && menu.top() - button.bottom() < Pixels::from(16.),
            "{open}: menu {menu:?} under {button:?}"
        );
        press(cx, "escape");
        assert!(
            label_of(cx, "action-panel").is_none(),
            "{open}: Esc closes it"
        );
    }
}

/// Review Focus 2: other names and the wrong layout find the language.
#[gpui_kit::test]
fn menu_filters_by_keyword_and_layout(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    for query in ["deut", "немец", "ytvtw"] {
        press(cx, "ctrl-p");
        type_text(cx, query);
        assert_eq!(menu_rows(cx), ["German"], "{query}");
        press(cx, "escape");
    }
}

#[gpui_kit::test]
fn choosing_sends_the_option_and_the_button(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    press(cx, "ctrl-p");
    type_text(cx, "fren");
    press(cx, "enter");
    deliver(cx, &core);
    assert_eq!(
        chosen(&events),
        [ViewEvent::ActionChosen {
            row_id: "fr".into(),
            action_id: "lang".into(),
        }]
    );
    assert!(label_of(cx, "action-panel").is_none(), "choosing closes it");
    click(cx, ("toolbar-button", 1));
    click(cx, ("action", 0));
    deliver(cx, &core);
    assert_eq!(
        chosen(&events).last(),
        Some(&ViewEvent::ActionChosen {
            row_id: "de".into(),
            action_id: "lang".into(),
        })
    );
}

/// Review Focus 4: Esc closes the menu, and the pad stays.
#[gpui_kit::test]
fn esc_closes_the_menu_only(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    press(cx, "ctrl-p");
    press(cx, "escape");
    assert!(label_of(cx, "action-panel").is_none());
    assert!(
        label_of(cx, "scratchpad-output").is_some(),
        "the pad is still there"
    );
}

/// Review Focus 3: an update of the pad keeps the menu open with its filter.
#[gpui_kit::test]
fn menu_survives_an_update(cx: &mut TestAppContext) {
    let (core, _) = pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(retoolbar),
    );
    press(cx, "end");
    type_text(cx, "x");
    press(cx, "ctrl-p");
    type_text(cx, "ger");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    assert!(
        label_of(cx, "scratchpad-output")
            .unwrap()
            .contains("updated hellox")
    );
    assert_eq!(menu_rows(cx), ["German"], "the menu and its filter stay");
}

/// The toolbar pad with a full-length menu: 135 options, the 36th current.
fn push_long_menu() -> anyhow::Result<Vec<Effect>> {
    let mut view = toolbar_pad(0, "out");
    view.toolbar[1].options = (0..135)
        .map(|i| {
            option(
                &format!("l{i}"),
                &format!("Language {i:03}"),
                "Native",
                "русский",
            )
        })
        .collect();
    view.toolbar[1].selected = Some("l35".into());
    Ok(vec![Effect::PushView(View::Scratchpad(view))])
}

fn row_bounds(cx: &mut TestAppContext, title: &str) -> Bounds<Pixels> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find_by_label(title).bounds()
    })
    .unwrap()
}

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.top() >= outer.top() && inner.bottom() <= outer.bottom()
}

/// Review finding 1: a full-length menu still hangs under its button, inside the panel.
#[gpui_kit::test]
fn a_long_menu_hangs_under_its_button(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_long_menu).with_view_events(nothing),
    );
    let button = bounds_at(cx, ("toolbar-button", 1));
    for open in ["click", "shortcut"] {
        match open {
            "click" => click(cx, ("toolbar-button", 1)),
            _ => press(cx, "ctrl-p"),
        }
        render(cx);
        let menu = bounds_of(cx, "action-panel");
        let pane = bounds_of(cx, "scratchpad-output");
        assert!(
            menu.top() >= button.bottom(),
            "{open}: menu {menu:?} under {button:?}"
        );
        assert!(
            menu.bottom() <= pane.bottom() + Pixels::from(1.),
            "{open}: {menu:?} in {pane:?}"
        );
        press(cx, "escape");
    }
}

/// Review finding 2: the current choice is in view when the menu opens, and the first row
/// after filtering.
#[gpui_kit::test]
fn the_selected_row_stays_in_view(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_long_menu).with_view_events(nothing),
    );
    press(cx, "ctrl-p");
    render(cx);
    render(cx);
    let list = bounds_of(cx, "action-list");
    assert!(
        inside(row_bounds(cx, "Language 035"), list),
        "the current row shows"
    );
    for _ in 0..40 {
        press(cx, "down");
    }
    render(cx);
    assert!(
        inside(row_bounds(cx, "Language 075"), list),
        "the selection scrolls along"
    );
    type_text(cx, "1");
    render(cx);
    render(cx);
    assert!(
        inside(row_bounds(cx, "Language 001"), list),
        "after filtering, the first row"
    );
}

/// Review finding 4: a click outside closes the menu; its button toggles it, and so does
/// its shortcut.
#[gpui_kit::test]
fn the_menu_closes_like_a_popup(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    click(cx, ("toolbar-button", 1));
    assert!(label_of(cx, "action-panel").is_some());
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.click("scratchpad-editor", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        label_of(cx, "action-panel").is_none(),
        "a click outside closes it"
    );
    click(cx, ("toolbar-button", 1));
    click(cx, ("toolbar-button", 1));
    assert!(
        label_of(cx, "action-panel").is_none(),
        "its button toggles it"
    );
    press(cx, "ctrl-p");
    press(cx, "ctrl-p");
    assert!(
        label_of(cx, "action-panel").is_none(),
        "its shortcut toggles it"
    );
}

/// The toolbar pad with actions, as the translator has them: Copy first, and
/// "Change Language…" named as the `lang` button, on its shortcut.
fn push_toolbar_with_actions() -> anyhow::Result<Vec<Effect>> {
    let mut view = toolbar_pad(0, "out");
    view.actions = vec![
        Action {
            id: "copy".into(),
            title: "Copy Translation".into(),
            shortcut: None,
        },
        Action {
            id: "lang".into(),
            title: "Change Language…".into(),
            shortcut: Some("ctrl+p".into()),
        },
    ];
    Ok(vec![Effect::PushView(View::Scratchpad(view))])
}

/// Ctrl+K on a pad shows Ctrl+Enter on its first action, which that key runs.
#[gpui_kit::test]
fn the_first_pad_action_shows_ctrl_enter(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar_with_actions).with_view_events(nothing),
    );
    press(cx, "ctrl-k");
    let keys = |cx: &mut TestAppContext, ix: usize| {
        let w = window(cx);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(("action-keys", ix))
                .map(|e| e.label().unwrap_or_default().to_string())
        })
        .unwrap()
    };
    let enter = if cfg!(target_os = "macos") {
        "⌘+↵"
    } else {
        "Ctrl+↵"
    };
    assert_eq!(keys(cx, 0).as_deref(), Some(enter));
    assert!(
        keys(cx, 1).is_some(),
        "the language action shows its shortcut"
    );
    let enter_hint = |cx: &mut TestAppContext, ix: usize| {
        let w = window(cx);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(("action-enter", ix)).is_some()
        })
        .unwrap()
    };
    assert!(
        !enter_hint(cx, 0),
        "the selected row shows its own keys, not the Enter hint as well"
    );
}

/// A pad action named as a toolbar button opens that button's menu: from Ctrl+K, and by
/// its shortcut while Ctrl+K is open.
#[gpui_kit::test]
fn an_action_named_as_a_button_opens_its_menu(cx: &mut TestAppContext) {
    let (_, events) = pushed(
        cx,
        FakeModule::new(items(), push_toolbar_with_actions).with_view_events(nothing),
    );
    press(cx, "ctrl-k");
    press(cx, "down");
    press(cx, "enter");
    assert_eq!(menu_rows(cx), ["German", "English", "French"]);
    press(cx, "escape");
    press(cx, "ctrl-k");
    press(cx, "ctrl-p");
    assert_eq!(menu_rows(cx), ["German", "English", "French"]);
    assert!(chosen(&events).is_empty(), "nothing went to the module");
}

fn push_busy() -> anyhow::Result<Vec<Effect>> {
    let mut view = toolbar_pad(0, "out");
    view.busy = true;
    Ok(vec![Effect::PushView(View::Scratchpad(view))])
}

/// A busy pad shows a line under the header while it works; an idle one a plain divider.
#[gpui_kit::test]
fn a_busy_pad_shows_it_under_the_header(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_busy).with_view_events(nothing),
    );
    assert!(label_of(cx, "view-busy").is_some(), "the busy line");
    assert!(
        label_of(cx, "view-busy-bloom").is_some(),
        "the glow around the running light"
    );
    let header = bounds_of(cx, "view-header");
    let line = bounds_of(cx, "view-busy");
    assert!(line.top() >= header.top() && line.bottom() <= header.bottom() + Pixels::from(2.));
}

#[gpui_kit::test]
fn an_idle_pad_shows_no_busy_line(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_toolbar).with_view_events(nothing),
    );
    assert!(label_of(cx, "view-busy").is_none());
}

fn push_compose_text(text: String) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Scratchpad(compose(
        text.clone(),
        0,
        &text,
    )))])
}

/// The height of the editor's first line, a measure of its text size.
fn first_line_height(cx: &mut TestAppContext) -> f32 {
    render(cx);
    render(cx);
    f32::from(line_bounds(cx, 0).unwrap().size.height)
}

/// A short text reads large; a long one at the body size; the size is fitted to the
/// columns, the same for the text and its output.
#[gpui_kit::test]
fn the_text_size_fits_the_text(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), || push_compose_text("Hola".into())).with_view_events(nothing),
    );
    let short = first_line_height(cx);
    assert!(short >= 26. * 1.4, "a short text is large: {short}");
}

#[gpui_kit::test]
fn a_long_text_is_at_the_body_size(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), || push_compose_text("palabra ".repeat(600)))
            .with_view_events(nothing),
    );
    let long = first_line_height(cx);
    assert!(long <= 14. * 1.5, "a long text is at the body size: {long}");
}

/// Three buttons, as the translator has them: source, swap, target.
fn push_three_buttons() -> anyhow::Result<Vec<Effect>> {
    let mut view = toolbar_pad(0, "out");
    let source = ToolbarButton {
        id: "from".into(),
        label: "Spanish (detected)".into(),
        ..view.toolbar[1].clone()
    };
    view.toolbar = vec![source, view.toolbar[0].clone(), view.toolbar[1].clone()];
    Ok(vec![Effect::PushView(View::Scratchpad(view))])
}

/// The middle button sits over the line between the columns; the last starts over the
/// output's text.
#[gpui_kit::test]
fn the_middle_button_is_over_the_divider(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_three_buttons).with_view_events(nothing),
    );
    render(cx);
    render(cx);
    let editor = bounds_of(cx, "scratchpad-editor");
    let pane = bounds_of(cx, "scratchpad-output");
    let divider = (editor.right() + pane.left()) / 2.;
    let swap = bounds_at(cx, ("toolbar-button", 1));
    let centre = swap.left() + swap.size.width / 2.;
    assert!(
        (centre - divider).abs() < Pixels::from(2.),
        "swap at {centre:?}, divider at {divider:?}"
    );
    let target = bounds_at(cx, ("toolbar-button", 2));
    assert!(
        target.left() >= swap.right(),
        "{target:?} clear of {swap:?}"
    );
    assert!(
        target.left() - pane.left() < Pixels::from(40.),
        "{target:?} at the output's start"
    );
}

/// Each column's text is inset alike on both sides: by the divider as by the window's edge.
#[gpui_kit::test]
fn the_output_is_inset_alike_on_both_sides(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_three_buttons).with_view_events(nothing),
    );
    render(cx);
    let pane = bounds_of(cx, "scratchpad-output");
    let text = bounds_of(cx, "scratchpad-output-text");
    let left = text.left() - pane.left();
    let right = pane.right() - text.right();
    assert!(
        (left - right).abs() < Pixels::from(1.),
        "inset {left:?} by the divider, {right:?} by the edge"
    );
}

/// The placeholder is one line: it takes a size at which it fits its column's width.
#[gpui_kit::test]
fn the_placeholder_fits_its_column(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), || {
            let mut view = compose(String::new(), 0, "");
            view.placeholder = "Type or paste text to translate, a long placeholder".into();
            Ok(vec![Effect::PushView(View::Scratchpad(view))])
        })
        .with_view_events(nothing),
    );
    let height = first_line_height(cx);
    assert!(
        height < 26. * 1.4,
        "a long placeholder is not at the largest size: {height}"
    );
}

/// The size moves in a few steps, not pixel by pixel: typing does not shake the text.
#[gpui_kit::test]
fn the_size_moves_in_steps(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), || push_compose_text("palabra ".repeat(40)))
            .with_view_events(nothing),
    );
    let height = first_line_height(cx);
    let sizes = [26., 21., 17., 14.].map(|s: f32| s * 1.45);
    assert!(
        sizes.iter().any(|s| (s - height).abs() < 1.),
        "{height} is one of the steps {sizes:?}"
    );
}

/// The busy line takes no room: the header is as high with it as without it, so nothing
/// moves when it comes and goes.
#[gpui_kit::test]
fn the_busy_line_moves_nothing(cx: &mut TestAppContext) {
    // Busy when pushed; the answer to an edit (`retoolbar`) is idle.
    let (core, _) = pushed(
        cx,
        FakeModule::new(items(), push_busy).with_view_events(retoolbar),
    );
    assert!(label_of(cx, "view-busy").is_some());
    let busy = (
        bounds_of(cx, "view-header"),
        bounds_of(cx, "scratchpad-editor"),
    );
    type_text(cx, "x");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    // The line finishes its sweep, then goes.
    wait(cx, Duration::from_millis(1700));
    render(cx);
    render(cx);
    assert!(label_of(cx, "view-busy").is_none(), "idle now");
    let idle = (
        bounds_of(cx, "view-header"),
        bounds_of(cx, "scratchpad-editor"),
    );
    assert_eq!(
        busy.0.size.height, idle.0.size.height,
        "the header's height"
    );
    assert_eq!(busy.1.top(), idle.1.top(), "the editor's top");
}

/// The busy line finishes its sweep to the right edge when the pad stops being busy, then
/// goes: it never vanishes halfway.
#[gpui_kit::test]
fn the_busy_line_finishes_its_sweep(cx: &mut TestAppContext) {
    let (core, _) = pushed(
        cx,
        FakeModule::new(items(), push_busy).with_view_events(retoolbar),
    );
    render(cx);
    wait(cx, Duration::from_millis(500));
    render(cx);
    // The answer: idle, a third of the way through the sweep. The sweep hurries to the edge,
    // the whole line lights up and fades: a full stop, quickly.
    type_text(cx, "x");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    assert!(
        label_of(cx, "view-busy").is_some(),
        "still sweeping to the edge"
    );
    wait(cx, Duration::from_millis(400));
    render(cx);
    assert_eq!(
        label_of(cx, "view-busy").as_deref(),
        Some("Done"),
        "the line lights up as the sweep ends"
    );
    assert!(
        label_of(cx, "view-busy-glow").is_some(),
        "the lit line glows too"
    );
    wait(cx, Duration::from_millis(400));
    render(cx);
    render(cx);
    assert!(
        label_of(cx, "view-busy").is_none(),
        "gone, well before a sweep's time"
    );
}

/// When the work is done the lit part grows out from where the light was, not from the
/// left edge: early on it leaves the line's start unlit.
#[gpui_kit::test]
fn the_lit_part_grows_from_the_light(cx: &mut TestAppContext) {
    let (core, _) = pushed(
        cx,
        FakeModule::new(items(), push_busy).with_view_events(retoolbar),
    );
    render(cx);
    wait(cx, Duration::from_millis(800));
    render(cx);
    // Done halfway through a sweep: the light is in the middle of the line.
    type_text(cx, "x");
    wait(cx, DEBOUNCE);
    deliver(cx, &core);
    render(cx);
    wait(cx, Duration::from_millis(40));
    render(cx);
    let line = bounds_of(cx, "view-busy");
    let lit = bounds_of(cx, "view-busy-fill");
    assert!(
        lit.left() > line.left() + line.size.width * 0.2,
        "the lit part starts near the light: {lit:?} in {line:?}"
    );
}
