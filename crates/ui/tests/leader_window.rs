//! Headless tests of the invisible leader window: the keys it takes reach its sink, and it
//! opens and closes as the leader state machine asks.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, TestAppContext,
    VisualTestContext,
};
use tomakast_core::Hotkey;
use tomakast_ui::{CaptureEvent, CaptureSink, LeaderWindow};

type Events = Rc<RefCell<Vec<CaptureEvent>>>;

fn recording_sink() -> (CaptureSink, Events) {
    let events = Events::default();
    let seen = events.clone();
    let sink: CaptureSink = Rc::new(move |event, _| seen.borrow_mut().push(event));
    (sink, events)
}

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        // The launcher's own bindings (its escape among them) must not take the window's keys.
        tomakast_ui::init(cx);
        cx.set_global(LeaderWindow::new());
    });
}

fn open(cx: &mut TestAppContext, sink: CaptureSink) {
    cx.update(|cx| cx.update_global::<LeaderWindow, _>(|w, cx| w.open(sink, cx)));
    cx.run_until_parked();
}

fn close(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LeaderWindow, _>(|w, cx| w.close(cx)));
    cx.run_until_parked();
}

fn is_open(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<LeaderWindow>().is_open())
}

fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "expected exactly the leader window");
    windows[0]
}

fn hk(s: &str) -> Hotkey {
    Hotkey::parse(s).unwrap()
}

#[gpui_kit::test]
fn keys_reach_the_sink(cx: &mut TestAppContext) {
    setup(cx);
    let (sink, events) = recording_sink();
    open(cx, sink);
    let w = window(cx);

    cx.simulate_keystrokes(w, "f");
    assert_eq!(
        *events.borrow(),
        [CaptureEvent::Key {
            hotkey: Some(hk("f")),
            repeat: false
        }]
    );

    cx.simulate_keystrokes(w, "escape");
    assert_eq!(events.borrow().last(), Some(&CaptureEvent::Escape));
    assert_eq!(events.borrow().len(), 2);
}

#[gpui_kit::test]
fn close_removes_the_window(cx: &mut TestAppContext) {
    setup(cx);
    let (sink, events) = recording_sink();
    open(cx, sink);
    let w = window(cx);
    assert!(is_open(cx));

    close(cx);
    assert!(!cx.update(|cx| cx.windows()).contains(&w));
    assert!(!is_open(cx));
    // Closed as asked: nothing to report.
    assert!(events.borrow().is_empty());
}

/// The compositor may close the surface on its own; the sink hears it as a loss of focus, so
/// the leader goes idle and the next press opens again.
#[gpui_kit::test]
fn a_window_closed_outside_close_reports_lost_focus(cx: &mut TestAppContext) {
    setup(cx);
    let (sink, events) = recording_sink();
    open(cx, sink.clone());
    let w = window(cx);

    cx.update_window(w, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(!is_open(cx));
    assert_eq!(*events.borrow(), [CaptureEvent::FocusLost]);

    open(cx, sink);
    assert!(is_open(cx));
}

/// The sink may close the window while hearing of its loss (the driver does): no double
/// borrow of the global.
#[gpui_kit::test]
fn the_sink_may_close_when_told_of_a_compositor_close(cx: &mut TestAppContext) {
    setup(cx);
    let heard = Events::default();
    let seen = heard.clone();
    let sink: CaptureSink = Rc::new(move |event, cx| {
        seen.borrow_mut().push(event);
        cx.update_global::<LeaderWindow, _>(|w, cx| w.close(cx));
    });
    open(cx, sink);
    let w = window(cx);
    cx.update_window(w, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*heard.borrow(), [CaptureEvent::FocusLost]);
    assert!(!is_open(cx));
}

#[gpui_kit::test]
fn a_second_open_is_a_no_op(cx: &mut TestAppContext) {
    setup(cx);
    let (first, first_events) = recording_sink();
    let (second, second_events) = recording_sink();
    open(cx, first);
    open(cx, second);
    let w = window(cx);

    cx.simulate_keystrokes(w, "g");
    assert_eq!(first_events.borrow().len(), 1);
    assert!(second_events.borrow().is_empty());
}

/// The sink runs outside the window's own update, so it can close the window, as the leader
/// state machine does on Escape.
#[gpui_kit::test]
fn the_sink_can_close_the_window(cx: &mut TestAppContext) {
    setup(cx);
    let sink: CaptureSink = Rc::new(|event, cx| {
        if event == CaptureEvent::Escape {
            cx.update_global::<LeaderWindow, _>(|w, cx| w.close(cx));
        }
    });
    open(cx, sink);
    let w = window(cx);

    cx.simulate_keystrokes(w, "escape");
    assert!(cx.update(|cx| cx.windows()).is_empty());
    assert!(!is_open(cx));
}

#[gpui_kit::test]
fn losing_focus_after_having_it_reaches_the_sink(cx: &mut TestAppContext) {
    setup(cx);
    let (sink, events) = recording_sink();
    open(cx, sink);
    let w = window(cx);

    // Never active yet: losing nothing sends nothing.
    VisualTestContext::from_window(w, cx).deactivate_window();
    cx.run_until_parked();
    assert!(events.borrow().is_empty());

    // The test platform does not activate new windows; do what the compositor does when the
    // window gets the keyboard.
    cx.update_window(w, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty());

    VisualTestContext::from_window(w, cx).deactivate_window();
    cx.run_until_parked();
    assert_eq!(*events.borrow(), [CaptureEvent::FocusLost]);
}

/// A press is a key-down and a key-up; the daemon reads the leader key's own key-up as its
/// release.
#[gpui_kit::test]
fn key_ups_reach_the_sink(cx: &mut TestAppContext) {
    setup(cx);
    let (sink, events) = recording_sink();
    open(cx, sink);
    let w = window(cx);

    cx.update_window(w, |_, window, cx| window.press("f", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        [
            CaptureEvent::Key {
                hotkey: Some(hk("f")),
                repeat: false
            },
            CaptureEvent::KeyUp {
                hotkey: Some(hk("f"))
            },
        ]
    );
}
