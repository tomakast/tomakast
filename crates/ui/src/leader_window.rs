//! The invisible leader window: while the leader is down, a 1x1 transparent
//! surface takes the keyboard and reports each key-down, each key-up and the
//! loss of focus to a sink (the daemon's leader state machine). The key-up of
//! the leader's own key is how the daemon sees the release when the system's
//! release event is missed. It draws nothing and takes
//! no pointer input. Opened through the launcher's window options
//! ([`crate::window::leader_options`]), so it rides the prewarmed renderer.

use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    App, AppContext as _, BorrowAppContext as _, Context, FocusHandle, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Render,
    Subscription, Window, WindowHandle, WindowId, div,
};
use tomakast_core::Hotkey;
use tomakast_core::keys::gpui_alias;

use crate::views::hotkey_recorder::hotkey_of;
use crate::window::{keyboard_allowed, leader_options};

/// What the leader window reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureEvent {
    /// A key-down as a hotkey (the recorder's `hotkey_of` with `keys::gpui_alias`); `None`: a
    /// modifier alone or a key with no name.
    Key {
        hotkey: Option<Hotkey>,
        repeat: bool,
    },
    /// A key-up, converted as [`Key`](Self::Key) is. The window does not know the leader: the
    /// daemon reads a key-up of the leader's key as its release.
    KeyUp {
        hotkey: Option<Hotkey>,
    },
    Escape,
    FocusLost,
}

/// Receives the window's [`CaptureEvent`]s. It runs outside the window's own
/// update, so it may close the window.
pub type CaptureSink = Rc<dyn Fn(CaptureEvent, &mut App)>;

/// Global. Owns at most one leader window.
#[derive(Default)]
pub struct LeaderWindow {
    open: Option<OpenLeader>,
}

/// The leader window while it is open.
struct OpenLeader {
    handle: WindowHandle<CaptureView>,
    /// Told of a close the window did not ask for.
    sink: CaptureSink,
    /// Forgets the window when it closes without [`LeaderWindow::close`] (the
    /// compositor closing the surface), so the next `open` opens again.
    _on_closed: Subscription,
}

impl Global for LeaderWindow {}

impl LeaderWindow {
    pub fn new() -> LeaderWindow {
        LeaderWindow::default()
    }

    /// Opens the 1×1 transparent window that takes the keyboard; no-op when open.
    /// Under `TOMAKAST_NO_KEYBOARD=1` the window opens without the keyboard (tests rely on it).
    pub fn open(&mut self, sink: CaptureSink, cx: &mut App) {
        if self.open.is_some() {
            return;
        }
        let opened_at = Instant::now();
        let opts = leader_options(keyboard_allowed(), cx);
        let view_sink = sink.clone();
        let opened = cx.open_window(opts, |window, cx| {
            // Clicks go through to the window below.
            window.set_input_region(Some(&[]));
            cx.new(|cx| CaptureView::new(view_sink, opened_at, window, cx))
        });
        match opened {
            Ok(handle) => {
                let id = handle.window_id();
                // Close observers run inside the closing window's update, where
                // the global may be leased (a `close`), so the forget is deferred.
                let on_closed = cx.on_window_closed(move |cx, closed| {
                    if closed == id {
                        cx.defer(move |cx| forget_closed_window(id, cx));
                    }
                });
                self.open = Some(OpenLeader {
                    handle,
                    sink,
                    _on_closed: on_closed,
                });
            }
            Err(e) => tracing::error!(error = %e, "leader window could not be opened"),
        }
    }

    /// Closes the window; no-op when it is closed. Must not be called from
    /// inside an update of that window; the sink runs outside it.
    pub fn close(&mut self, cx: &mut App) {
        let Some(OpenLeader { handle, .. }) = self.open.take() else {
            return;
        };
        // An error means the window is already gone (e.g. closed by the compositor).
        if let Err(e) = handle.update(cx, |_, window, _| window.remove_window()) {
            tracing::debug!(error = %e, "leader window already closed");
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

/// Drops the state for window `id` if it is still the open one, and tells its sink the
/// keyboard is lost (outside the global's update: the sink may close).
fn forget_closed_window(id: WindowId, cx: &mut App) {
    if !cx.has_global::<LeaderWindow>() {
        return;
    }
    let sink = cx.update_global::<LeaderWindow, _>(|w, _| {
        if !w.open.as_ref().is_some_and(|o| o.handle.window_id() == id) {
            return None;
        }
        tracing::debug!("leader window closed outside close");
        w.open.take().map(|o| o.sink)
    });
    if let Some(sink) = sink {
        sink(CaptureEvent::FocusLost, cx);
    }
}

/// The leader window's view: an empty, focused element that forwards keys.
struct CaptureView {
    sink: CaptureSink,
    focus: FocusHandle,
    /// When `open` was called; taken when the first activation is logged.
    opened_at: Option<Instant>,
    was_active: bool,
    _activation: Subscription,
}

impl CaptureView {
    fn new(
        sink: CaptureSink,
        opened_at: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CaptureView {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let active = window.is_window_active();
            if active && let Some(opened_at) = this.opened_at.take() {
                tracing::debug!(
                    ms = opened_at.elapsed().as_secs_f64() * 1000.0,
                    "leader window focused"
                );
            }
            if this.was_active && !active {
                this.send(CaptureEvent::FocusLost, cx);
            }
            this.was_active = active;
        });
        CaptureView {
            sink,
            focus,
            opened_at: Some(opened_at),
            was_active: false,
            _activation: activation,
        }
    }

    /// Hands `event` to the sink once the window's update is over.
    fn send(&self, event: CaptureEvent, cx: &mut Context<Self>) {
        let sink = self.sink.clone();
        cx.defer(move |cx| sink(event, cx));
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let keystroke = &event.keystroke;
        let captured = if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            CaptureEvent::Escape
        } else {
            CaptureEvent::Key {
                hotkey: hotkey_in_window(keystroke),
                repeat: event.is_held,
            }
        };
        self.send(captured, cx);
    }

    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let hotkey = hotkey_in_window(&event.keystroke);
        self.send(CaptureEvent::KeyUp { hotkey }, cx);
    }
}

/// `keystroke` as a hotkey: the recorder's [`hotkey_of`], with the key under its config name
/// ([`gpui_alias`]).
fn hotkey_in_window(keystroke: &Keystroke) -> Option<Hotkey> {
    hotkey_of(keystroke).map(|h| Hotkey {
        key: gpui_alias(&h.key).to_owned(),
        ..h
    })
}

impl Render for CaptureView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
    }
}
