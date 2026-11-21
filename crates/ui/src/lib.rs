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
mod fonts;
mod icons;
mod launcher_view;
pub mod markdown;
mod named_icon;
pub mod plugin_image;
mod root_sections;
mod shortcut;
mod stack;
mod strip;
mod theme;
mod toast;
mod views;
mod window;

use gpui_kit::{App, KeyBinding};

pub use icons::assets;
pub use launcher_view::{
    Confirm, Dismiss, LauncherView, SelectLeft, SelectNext, SelectPrev, SelectRight, StripNext,
    StripPrev, SubmitForm, ToggleActions,
};
pub use markdown::LinkPolicy;
pub use strip::images::{raster_helper, set_raster_helper};
pub use strip::{CanvasPaintBudget, ReducedMotionSource, tile_name};
pub use window::LauncherController;

use launcher_view::{KEY_CONTEXT, ROOT_KEY_CONTEXT};

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

/// Key context of the kit's select (a form dropdown), as gpui-base names it.
const SELECT_KEY_CONTEXT: &str = "Select";

/// The keys [`init`] binds, in its order: up, down, left, right, enter,
/// escape, the action panel, form submit. Action shortcuts may not take them.
const LAUNCHER_KEYS: [&str; 8] = [
    "up",
    "down",
    "left",
    "right",
    "enter",
    "escape",
    TOGGLE_ACTIONS_KEY,
    SUBMIT_FORM_KEY,
];

/// The keys that move between form fields; the form view handles them
/// itself, and action shortcuts may not take them either. On the root list
/// they move through the widget strip, bound in [`ROOT_KEY_CONTEXT`] only, so
/// a form keeps its own.
const FORM_FOCUS_KEYS: [&str; 2] = ["tab", "shift-tab"];

/// Initializes GPUI Kit and binds the launcher keys. Call it once, instead of
/// `gpui_kit::init`: the bindings below must come after the Kit's own.
///
/// The theme follows the system appearance: every launcher window syncs it
/// from `window.appearance()` when it opens and tracks its changes. Its text
/// is in the bundled Inter and JetBrains Mono, whatever the machine has.
pub fn init(cx: &mut App) {
    fonts::register(cx);
    gpui_kit::init(cx);
    fonts::name_on_theme(cx);
    let [up, down, left, right, enter, escape, toggle, submit] = LAUNCHER_KEYS;
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new(up, SelectPrev, context),
        KeyBinding::new(down, SelectNext, context),
        KeyBinding::new(left, SelectLeft, context),
        KeyBinding::new(right, SelectRight, context),
        KeyBinding::new(enter, Confirm, context),
        KeyBinding::new(escape, Dismiss, context),
        KeyBinding::new(toggle, ToggleActions, context),
        KeyBinding::new(submit, SubmitForm, context),
        // The kit's select takes the submit key to open itself; on a form
        // dropdown it submits the form, as on every other field. Its open
        // list's own binding is deeper and still chooses the option.
        KeyBinding::new(submit, SubmitForm, Some(SELECT_KEY_CONTEXT)),
    ]);
    theme::paint_kit_popups(cx);
    let [tab, shift_tab] = FORM_FOCUS_KEYS;
    let root = Some(ROOT_KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new(tab, StripNext, root),
        KeyBinding::new(shift_tab, StripPrev, root),
    ]);
}

/// Draws theme-named icons (`Icon::Named`) that are not bundled Lucide names as the files
/// they resolve to in the desktop's icon theme, from now on: the app calls it once, after
/// [`init`]. Without it such icons are monograms, as in the headless tests, which must not
/// depend on the machine's icon theme.
pub fn resolve_named_icons(cx: &mut App) {
    named_icon::start(cx);
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
