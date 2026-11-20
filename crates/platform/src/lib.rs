//! OS integration: global hotkeys, autostart, opening URLs and files, and icon themes.

pub mod autostart;
pub mod hotkeys;
pub mod icon_theme;
pub mod open;

pub use autostart::{set_autostart, set_autostart_in};
pub use hotkeys::{HotkeyBackend, HotkeyBinding, portal_shortcuts, register_global_hotkeys};
