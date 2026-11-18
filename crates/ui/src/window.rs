//! The launcher window, following the Task 9 findings
//! (`docs/experiments/2026-11-window-probe.md`):
//! - the window is recreated on every show, because GPUI cannot hide a
//!   window on Linux;
//! - it is opened with bare `cx.open_window`, not `gpui_kit::open_window`,
//!   whose Root paints an opaque backdrop and a client-side frame around
//!   layer surfaces; the view draws the rounded panel itself;
//! - on Wayland it is a layer-shell `Overlay` surface, elsewhere a pop-up;
//! - the renderer is prewarmed once at startup.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    App, AppContext as _, BorrowAppContext as _, Bounds, Empty, Entity, Global, Pixels, Size,
    Subscription, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowId,
    WindowKind, WindowOptions, px, size,
};
use tomakast_core::Core;

use crate::launcher_view::LauncherView;

/// Wayland app id and X11 class of the launcher window.
const APP_ID: &str = "dev.tomakast.Tomakast";
const WINDOW_SIZE: (f32, f32) = (720., 460.);

/// `TOMAKAST_NO_KEYBOARD=1` opens the window without taking the keyboard
/// (layer-shell `KeyboardInteractivity::None`, no focus), so it can be shown
/// on a live session during development without grabbing input.
fn keyboard_allowed() -> bool {
    std::env::var_os("TOMAKAST_NO_KEYBOARD").is_none_or(|v| v != "1")
}

fn options(size: Size<Pixels>, keyboard: bool, cx: &App) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        focus: keyboard,
        show: true,
        kind: kind(keyboard),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(APP_ID.into()),
        ..Default::default()
    }
}

#[cfg(target_os = "linux")]
fn kind(keyboard: bool) -> WindowKind {
    use gpui_kit::layer_shell::{KeyboardInteractivity, Layer, LayerShellOptions};
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return WindowKind::PopUp;
    }
    WindowKind::LayerShell(LayerShellOptions {
        namespace: "tomakast-launcher".into(),
        layer: Layer::Overlay,
        keyboard_interactivity: if keyboard {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        },
        ..Default::default()
    })
}

/// On macOS GPUI makes a pop-up a non-activating panel; on Windows a
/// topmost tool window without a taskbar entry.
#[cfg(not(target_os = "linux"))]
fn kind(_keyboard: bool) -> WindowKind {
    WindowKind::PopUp
}

/// Opens and immediately closes a 1x1 window that never takes the keyboard,
/// so the first real show does not pay for renderer start-up (~200 ms).
/// Blocks the main thread for about that long.
pub(crate) fn prewarm(cx: &mut App) {
    let started = Instant::now();
    let opts = options(size(px(1.), px(1.)), false, cx);
    match cx.open_window(opts, |_, cx| cx.new(|_| Empty)) {
        Ok(handle) => {
            if let Err(e) = handle.update(cx, |_, window, _| window.remove_window()) {
                tracing::warn!(error = %e, "prewarm window could not be closed");
            }
            tracing::debug!(
                ms = started.elapsed().as_secs_f64() * 1000.0,
                "renderer prewarmed"
            );
        }
        Err(e) => tracing::warn!(error = %e, "prewarm window could not be opened"),
    }
}

/// Opens the launcher window with `build` as its root view.
pub(crate) fn open(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> gpui_kit::Entity<LauncherView>,
) -> gpui_kit::Result<WindowHandle<LauncherView>> {
    let keyboard = keyboard_allowed();
    let opts = options(size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), keyboard, cx);
    cx.open_window(opts, build)
}

/// Owns the launcher window: opens it on show, closes it on hide.
/// Stored in a [`gpui_kit::Global`]; call it through `cx.update_global`.
pub struct LauncherController {
    core: Rc<RefCell<Core>>,
    open: Option<OpenWindow>,
}

/// The launcher window while it is open.
struct OpenWindow {
    handle: WindowHandle<LauncherView>,
    view: Entity<LauncherView>,
    /// Forgets this window when it closes without `hide`: the compositor
    /// closing the layer surface (e.g. its output went away), Alt+F4 on a
    /// pop-up, and the like.
    _on_closed: Subscription,
}

impl Global for LauncherController {}

impl LauncherController {
    pub fn new(core: Rc<RefCell<Core>>) -> LauncherController {
        LauncherController { core, open: None }
    }

    /// Warms up the renderer; the daemon calls it once at startup.
    pub fn prewarm(&mut self, cx: &mut App) {
        prewarm(cx);
    }

    /// Opens the launcher window with an empty, focused query. No-op when it is open.
    pub fn show(&mut self, cx: &mut App) {
        self.show_at(Instant::now(), cx);
    }

    /// Like [`show`](Self::show), with the moment the request was received
    /// (hotkey activation or IPC) as the start of the logged open latency.
    pub fn show_at(&mut self, requested_at: Instant, cx: &mut App) {
        if self.open.is_some() {
            return;
        }
        let core = self.core.clone();
        let mut built = None;
        let opened = open(cx, |window, cx| {
            let view = cx.new(|cx| LauncherView::new(core, requested_at, window, cx));
            built = Some(view.clone());
            view
        });
        match (opened, built) {
            (Ok(handle), Some(view)) => {
                let id = handle.window_id();
                // Close observers run inside the closing window's update, where
                // the global may be leased (a `hide`), so the forget is deferred.
                let on_closed = cx.on_window_closed(move |cx, closed| {
                    if closed == id {
                        cx.defer(move |cx| forget_closed_window(id, cx));
                    }
                });
                self.open = Some(OpenWindow {
                    handle,
                    view,
                    _on_closed: on_closed,
                });
            }
            (Ok(_), None) => unreachable!("open_window runs its build closure"),
            (Err(e), _) => tracing::error!(error = %e, "launcher window could not be opened"),
        }
    }

    /// Closes the launcher window. Must not be called from inside an update
    /// of that window; the view defers its own hides.
    pub fn hide(&mut self, cx: &mut App) {
        let Some(OpenWindow { handle, .. }) = self.open.take() else {
            return;
        };
        // An error means the window is already gone (e.g. closed by the compositor).
        if let Err(e) = handle.update(cx, |_, window, _| window.remove_window()) {
            tracing::debug!(error = %e, "launcher window already closed");
        }
    }

    pub fn toggle(&mut self, cx: &mut App) {
        self.toggle_at(Instant::now(), cx);
    }

    /// Like [`toggle`](Self::toggle); `requested_at` is passed to [`show_at`](Self::show_at).
    pub fn toggle_at(&mut self, requested_at: Instant, cx: &mut App) {
        if self.is_visible() {
            self.hide(cx);
        } else {
            self.show_at(requested_at, cx);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.open.is_some()
    }

    /// Recomputes the results of the open window after the core's items changed.
    pub fn core_changed(&mut self, cx: &mut App) {
        if let Some(open) = &self.open {
            open.view.update(cx, |view, cx| view.core_changed(cx));
        }
    }
}

/// Drops the controller's state for window `id` if it is still the open one,
/// so a window closed without `hide` reads as hidden and `show` reopens it.
fn forget_closed_window(id: WindowId, cx: &mut App) {
    if cx.has_global::<LauncherController>() {
        cx.update_global::<LauncherController, _>(|c, _| {
            if c.open.as_ref().is_some_and(|o| o.handle.window_id() == id) {
                tracing::debug!("launcher window closed outside hide");
                c.open = None;
            }
        });
    }
}

/// Hides the launcher through the global controller; used by the view, deferred
/// out of its own window update.
pub(crate) fn hide_launcher(cx: &mut App) {
    if cx.has_global::<LauncherController>() {
        cx.update_global::<LauncherController, _>(|c, cx| c.hide(cx));
    }
}
