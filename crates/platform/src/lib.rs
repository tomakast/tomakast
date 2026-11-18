//! OS integration: global hotkeys and autostart.

pub mod autostart;
pub mod hotkeys;

pub use autostart::{set_autostart, set_autostart_in};
pub use hotkeys::{HotkeyBackend, HotkeyBinding, portal_shortcuts, register_global_hotkeys};
