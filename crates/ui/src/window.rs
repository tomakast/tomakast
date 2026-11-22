//! The launcher window, following the Task 9 findings
//! (`docs/experiments/2026-11-window-probe.md`):
//! - the window is recreated on every show, because GPUI cannot hide a
//!   window on Linux;
//! - it is opened with bare `cx.open_window`, not `gpui_kit::open_window`,
//!   whose Root paints an opaque backdrop and a client-side frame around
//!   layer surfaces; the view draws the rounded panel itself;
//! - on Wayland it is a layer-shell `Overlay` surface, elsewhere a pop-up;
//! - the renderer is prewarmed once at startup;
//! - the background is blurred, with the blur region rounded to the panel;
//!   on Wayland a transparent client inset around the panel holds its shadow
//!   and is left out of the blur.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    App, AppContext as _, BorrowAppContext as _, Bounds, Corners, Empty, Entity, Global, Pixels,
    Size, Subscription, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowId,
    WindowKind, WindowOptions, point, px, size,
};
use tomakast_core::hotkeys::HotkeyTable;
use tomakast_core::{Core, CoreUpdate, Hotkey, LocalNow};

use crate::hotkeys::{HotkeyRequests, SharedHotkeys};
use crate::launcher_view::LauncherView;
use crate::markdown::{AllowLinks, LinkPolicy};
use crate::stack::{self, Launcher, OpenTarget, SharedLedger};
use crate::theme;

/// Wayland app id and X11 class of the launcher window.
const APP_ID: &str = "dev.tomakast.Tomakast";
/// Layer-shell namespace of the launcher surface; see [`kind`].
#[cfg(target_os = "linux")]
const LAYER_NAMESPACE: &str = "utility";

/// `TOMAKAST_NO_KEYBOARD=1` opens the window without taking the keyboard
/// (layer-shell `KeyboardInteractivity::None`, no focus), so it can be shown
/// on a live session during development without grabbing input.
fn keyboard_allowed() -> bool {
    std::env::var_os("TOMAKAST_NO_KEYBOARD").is_none_or(|v| v != "1")
}

/// Whether the launcher runs as a Wayland layer surface: a Linux build in a
/// Wayland session. Elsewhere it is a pop-up.
pub(crate) fn is_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
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
        window_background: WindowBackgroundAppearance::Blurred,
        app_id: Some(APP_ID.into()),
        ..Default::default()
    }
}

#[cfg(target_os = "linux")]
fn kind(keyboard: bool) -> WindowKind {
    use gpui_kit::layer_shell::{KeyboardInteractivity, Layer, LayerShellOptions};
    if !is_wayland() {
        return WindowKind::PopUp;
    }
    WindowKind::LayerShell(LayerShellOptions {
        // KWin derives the surface's window type from the namespace; an
        // unknown one is a normal window, which its `scale` effect animates on
        // map and unmap. `utility` is the type neither `scale` nor
        // `fadingpopups` animates (checked on KWin 6.8).
        namespace: LAYER_NAMESPACE.into(),
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
    build: impl FnOnce(&mut Window, &mut App) -> Entity<LauncherView>,
) -> gpui_kit::Result<WindowHandle<LauncherView>> {
    let keyboard = keyboard_allowed();
    let opts = options(surface_size(theme::shadow_margin()), keyboard, cx);
    cx.open_window(opts, |window, cx| {
        window.set_client_inset(px(theme::shadow_margin()));
        // Clicks on the transparent shadow margin reach the window below.
        if let Some(panel) = input_region(theme::shadow_margin()) {
            window.set_input_region(Some(&[panel]));
        }
        window.set_client_corner_radii(Corners::all(px(theme::PANEL_RADIUS)));
        build(window, cx)
    })
}

/// Owns the launcher window: opens it on show, closes it on hide.
/// The size of the launcher surface: the panel and the shadow margin on each
/// side. The patched gpui-pre-linux (`third_party/gpui-pre-linux`) gives a
/// layer surface this whole size, so the compositor centres the panel.
fn surface_size(margin: f32) -> Size<Pixels> {
    let (width, height) = theme::PANEL_SIZE;
    size(px(width + 2. * margin), px(height + 2. * margin))
}

/// The part of the surface that takes pointer input: the panel, without the
/// shadow margin around it, in surface coordinates. `None` (the whole
/// surface) where there is no margin.
fn input_region(margin: f32) -> Option<Bounds<Pixels>> {
    let (width, height) = theme::PANEL_SIZE;
    (margin > 0.).then(|| Bounds::new(point(px(margin), px(margin)), size(px(width), px(height))))
}

/// Stored in a [`gpui_kit::Global`]; call it through `cx.update_global`.
pub struct LauncherController {
    core: Rc<RefCell<Core>>,
    /// Requests issued by launcher windows, to route their replies.
    ledger: SharedLedger,
    /// Decides whether a module's links open; see [`set_link_policy`](Self::set_link_policy).
    links: Rc<dyn LinkPolicy>,
    /// Item hotkeys, shared with the open window.
    hotkeys: SharedHotkeys,
    open: Option<OpenWindow>,
}

/// The launcher window while it is open.
struct OpenWindow {
    handle: WindowHandle<LauncherView>,
    /// The window's number in the ledger.
    number: u64,
    /// Forgets this window when it closes without `hide`: the compositor
    /// closing the layer surface (e.g. its output went away), Alt+F4 on a
    /// pop-up, and the like.
    _on_closed: Subscription,
}

impl Global for LauncherController {}

impl LauncherController {
    pub fn new(core: Rc<RefCell<Core>>) -> LauncherController {
        LauncherController {
            core,
            ledger: SharedLedger::default(),
            links: Rc::new(AllowLinks),
            hotkeys: SharedHotkeys::default(),
            open: None,
        }
    }

    /// Turns on hotkey assignment: root rows get "Assign Hotkey…", whose recorder sends its
    /// changes to `requests`.
    pub fn set_hotkey_requests(&mut self, requests: Rc<dyn HotkeyRequests>) {
        self.hotkeys.borrow_mut().requests = Some(requests);
    }

    /// The item hotkeys as the daemon applied them; the open window shows them at once.
    pub fn set_hotkey_table(&mut self, table: HotkeyTable, cx: &mut App) {
        self.hotkeys.borrow_mut().table = table;
        if let Some(open) = &self.open {
            let _ = open.handle.update(cx, |_, _, cx| cx.notify());
        }
    }

    /// A message about a hotkey change: a toast in the open window, or at the next open.
    pub fn hotkey_notice(&mut self, text: String, cx: &mut App) {
        match &self.open {
            Some(open) => {
                let shown = open
                    .handle
                    .update(cx, |view, _, cx| view.show_error_toast(text.clone(), cx));
                if shown.is_err() {
                    self.hotkeys.borrow_mut().notices.push(text);
                }
            }
            None => self.hotkeys.borrow_mut().notices.push(text),
        }
    }

    /// Whether the open window is recording a hotkey: presses of tomakast's own hotkeys
    /// then go to [`recorded_press`](Self::recorded_press) instead of running.
    pub fn is_recording(&self, cx: &App) -> bool {
        self.open
            .as_ref()
            .and_then(|open| open.handle.read(cx).ok())
            .is_some_and(|view| view.is_recording())
    }

    /// A tomakast hotkey the system fired while the recorder was open: recorded as if
    /// pressed in it.
    pub fn recorded_press(&mut self, hotkey: Hotkey, cx: &mut App) {
        if let Some(open) = &self.open {
            let _ = open
                .handle
                .update(cx, |view, _, cx| view.recorded_press(hotkey, cx));
        }
    }

    /// Replaces the policy that decides whether the links in a module's
    /// views open (by default every module's do). It is asked on every click;
    /// windows opened from now on use it.
    pub fn set_link_policy(&mut self, policy: Rc<dyn LinkPolicy>) {
        self.links = policy;
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
        let ledger = self.ledger.clone();
        let links = self.links.clone();
        let hotkeys = self.hotkeys.clone();
        let number = self.ledger.borrow_mut().open_window();
        let opened = open(cx, |window, cx| {
            cx.new(|cx| {
                LauncherView::new(
                    core,
                    ledger,
                    number,
                    links,
                    hotkeys,
                    requested_at,
                    window,
                    cx,
                )
            })
        });
        match opened {
            Ok(handle) => {
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
                    number,
                    _on_closed: on_closed,
                });
            }
            Err(e) => tracing::error!(error = %e, "launcher window could not be opened"),
        }
    }

    /// Closes the launcher window, after its pushed views were told they go
    /// (`ViewEvent::Closed`). Must not be called from inside an update of
    /// that window; the view defers its own hides.
    pub fn hide(&mut self, cx: &mut App) {
        let Some(OpenWindow { handle, .. }) = self.open.take() else {
            return;
        };
        // An error means the window is already gone (e.g. closed by the compositor).
        let closed = handle.update(cx, |view, window, cx| {
            let query = view.root_query(cx);
            view.close_all(cx);
            window.remove_window();
            query
        });
        match closed {
            // Recorded after the window is gone, so it never delays the close.
            Ok(query) => match self.core.try_borrow_mut() {
                Ok(mut core) => {
                    core.record_hidden_query(&query, LocalNow::now());
                }
                Err(_) => tracing::warn!("core busy; query not recorded"),
            },
            Err(e) => tracing::debug!(error = %e, "launcher window already closed"),
        }
        // Their window is gone: nothing draws the images until it opens again, and then at
        // once, from the cache.
        crate::strip::images::stop_polls();
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

    /// Applies a [`Core::apply`] result: new results to the open window,
    /// replies to the window that requested them. Replies to requests of a
    /// window closed since never touch the current one. Replies to requests
    /// of no window (hotkeys, `tomakast run`) may close the window, open it on
    /// a pushed view, or toast in it. Effects a module sends on its own
    /// (`ModuleEffects`) never open, hide or pop: they update the module's
    /// views, push over the module's own top view, toast while the window is
    /// open, and copy text.
    pub fn core_update(&mut self, update: CoreUpdate, cx: &mut App) {
        stack::apply_update(self, update, cx);
    }
}

impl Launcher for LauncherController {
    fn ledger(&self) -> SharedLedger {
        self.ledger.clone()
    }

    fn window(&self) -> Option<OpenTarget> {
        self.open.as_ref().map(|o| OpenTarget {
            number: o.number,
            handle: o.handle,
        })
    }

    fn show_window(&mut self, cx: &mut App) -> Option<OpenTarget> {
        self.show(cx);
        self.window()
    }

    fn hide_window(&mut self, cx: &mut App) {
        self.hide(cx);
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

#[cfg(test)]
mod tests {
    use super::{input_region, surface_size};
    use crate::theme;
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn only_the_panel_takes_pointer_input() {
        let (width, height) = theme::PANEL_SIZE;
        assert_eq!(input_region(0.), None);
        assert_eq!(
            input_region(44.),
            Some(Bounds::new(
                point(px(44.), px(44.)),
                size(px(width), px(height))
            ))
        );
    }

    #[test]
    fn surface_holds_the_panel_and_its_margin() {
        let (width, height) = theme::PANEL_SIZE;
        assert_eq!(surface_size(0.), size(px(width), px(height)));
        assert_eq!(surface_size(44.), size(px(width + 88.), px(height + 88.)));
    }
}
