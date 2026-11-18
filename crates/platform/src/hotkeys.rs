//! Global hotkeys. Linux uses the `org.freedesktop.portal.GlobalShortcuts` portal;
//! other OSes return `Unsupported` until plans 3 and 4.

use tomakast_core::Hotkey;

#[cfg(target_os = "linux")]
mod linux;

/// One shortcut to register. `id` is `"launcher"` or `ItemId::as_str()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub id: String,
    pub description: String,
    pub hotkey: Hotkey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyBackend {
    Portal,
    Unsupported { reason: String },
}

/// Pure mapping to portal shortcuts: `(id, description, preferred trigger)`.
pub fn portal_shortcuts(bindings: &[HotkeyBinding]) -> Vec<(String, String, String)> {
    bindings
        .iter()
        .map(|b| {
            (
                b.id.clone(),
                b.description.clone(),
                b.hotkey.to_xdg_trigger(),
            )
        })
        .collect()
}

/// Registers global hotkeys on a background thread and calls `on_activate(id)` on each press.
/// Returns once the portal has accepted (or refused) the binding.
///
/// This blocks (on KDE possibly until the user answers the confirmation dialog), so callers
/// must not call it on the UI thread; use a background thread.
#[cfg(target_os = "linux")]
pub fn register_global_hotkeys(
    bindings: Vec<HotkeyBinding>,
    on_activate: impl Fn(String) + Send + 'static,
) -> HotkeyBackend {
    linux::register(bindings, on_activate)
}

#[cfg(not(target_os = "linux"))]
pub fn register_global_hotkeys(
    _bindings: Vec<HotkeyBinding>,
    _on_activate: impl Fn(String) + Send + 'static,
) -> HotkeyBackend {
    HotkeyBackend::Unsupported {
        reason: "global hotkeys are not implemented on this OS yet".into(),
    }
}
