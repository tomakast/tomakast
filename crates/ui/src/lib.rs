//! Launcher UI on GPUI: the launcher window and its list view.
//!
//! The daemon calls [`init`] once, sets `cx.set_quit_mode(QuitMode::Explicit)`
//! (the launcher window is closed on every hide, and on Linux GPUI otherwise
//! quits when the last window closes), stores a [`LauncherController`] as a
//! global and calls [`LauncherController::prewarm`] at startup.

mod launcher_view;
mod theme;
mod window;

use gpui_kit::{App, KeyBinding};

pub use launcher_view::{Confirm, Dismiss, LauncherView, SelectNext, SelectPrev};
pub use window::LauncherController;

use launcher_view::KEY_CONTEXT;

/// Initializes GPUI Kit and binds the launcher keys. Call it once, instead of
/// `gpui_kit::init`: the bindings below must come after the Kit's own.
///
/// The theme follows the system appearance: every launcher window syncs it
/// from `window.appearance()` when it opens and tracks its changes.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    cx.bind_keys([
        KeyBinding::new("up", SelectPrev, Some(KEY_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}
