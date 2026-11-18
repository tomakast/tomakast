//! Launcher UI on GPUI: the launcher window, its root search list and the
//! stack of module-owned views above it.
//!
//! The daemon calls [`init`] once, sets `cx.set_quit_mode(QuitMode::Explicit)`
//! (the launcher window is closed on every hide, and on Linux GPUI otherwise
//! quits when the last window closes), stores a [`LauncherController`] as a
//! global and calls [`LauncherController::prewarm`] at startup.
//!
//! # Dev-only environment
//!
//! For development runs and screenshots, not user settings; they may change
//! or go without notice:
//!
//! - `TOMAKAST_DEV_QUERY=<text>`: the root search box starts with `<text>`
//!   each time the launcher opens, so a typed query can be shown without
//!   the keyboard.
//! - `TOMAKAST_DEV_ACTIONS=1`: the action panel (Ctrl+K) opens by itself as
//!   soon as there is a selected row with actions, and reopens after it is
//!   closed or a view is pushed, so it can be screenshotted without the
//!   keyboard.
//! - `TOMAKAST_DEV_LIGHT=1`: the launcher uses its light colors whatever the
//!   system appearance, so the light look can be checked on a dark desktop.
//!
//! A flag counts as set when its value is neither empty nor `0`.

mod chrome;
mod launcher_view;
pub mod markdown;
mod root_sections;
mod stack;
mod theme;
mod toast;
mod views;
mod window;

use gpui_kit::{App, KeyBinding};

pub use launcher_view::{
    Confirm, Dismiss, LauncherView, SelectLeft, SelectNext, SelectPrev, SelectRight, SubmitForm,
    ToggleActions,
};
pub use window::LauncherController;

use launcher_view::KEY_CONTEXT;

/// Whether the dev-only flag `name` is set (see the crate docs).
pub(crate) fn dev_flag(name: &str) -> bool {
    dev_flag_value(std::env::var(name).ok().as_deref())
}

fn dev_flag_value(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty() && v != "0")
}

/// The "secondary" modifier plus K: Cmd+K on macOS, Ctrl+K elsewhere. The
/// pinned GPUI does not parse the `secondary-` alias, so it is spelled out.
const TOGGLE_ACTIONS_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};

/// Cmd+Enter on macOS, Ctrl+Enter elsewhere: submits the form on top. The
/// pinned GPUI does not parse the `secondary-` alias either.
const SUBMIT_FORM_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-enter"
} else {
    "ctrl-enter"
};

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
        KeyBinding::new("left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
        KeyBinding::new(TOGGLE_ACTIONS_KEY, ToggleActions, Some(KEY_CONTEXT)),
        KeyBinding::new(SUBMIT_FORM_KEY, SubmitForm, Some(KEY_CONTEXT)),
    ]);
}

#[cfg(test)]
mod tests {
    use super::dev_flag_value;

    #[test]
    fn dev_flag_reads_the_env_value() {
        assert!(dev_flag_value(Some("1")));
        assert!(dev_flag_value(Some("true")));
        assert!(!dev_flag_value(Some("")));
        assert!(!dev_flag_value(Some("0")));
        assert!(!dev_flag_value(None));
    }
}
