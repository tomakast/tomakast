//! The one way into the settings window: the launcher's Settings item, Ctrl+, and the
//! daemon's IPC request all end in [`open_settings`], which calls the hook the app sets.

use std::rc::Rc;

use gpui_kit::{App, Global};

/// Opens the settings window on a page id (`None`: the last page).
pub type OpenSettingsFn = Rc<dyn Fn(Option<String>, &mut App)>;

/// Holder of the settings window's entry point; set by the app once the window can be built.
pub struct OpenSettingsHook(pub OpenSettingsFn);

impl Global for OpenSettingsHook {}

/// Asks for the settings window; logs when the app has set no hook yet.
pub fn open_settings(page: Option<String>, cx: &mut App) {
    match cx.try_global::<OpenSettingsHook>().map(|h| h.0.clone()) {
        Some(open) => open(page, cx),
        None => tracing::info!("settings window not built"),
    }
}

/// Opens the settings window on `page` from the launcher, then hides the launcher. In that
/// order: the window asks for the focus on behalf of the key or click that opened it, which
/// Wayland compositors grant only while the window it was pressed in still exists.
pub(crate) fn open_settings_from_launcher(page: Option<String>, cx: &mut App) {
    open_settings(page, cx);
    crate::window::hide_launcher(cx);
}
