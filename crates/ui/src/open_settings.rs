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
