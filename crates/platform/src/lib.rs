//! OS integration: global hotkeys, autostart, opening URLs and files, icon themes, and the
//! desktop's reduced-motion setting.

pub mod autostart;
pub mod hotkeys;
pub mod icon_theme;
pub mod motion;
pub mod open;

pub use autostart::{set_autostart, set_autostart_in};
pub use hotkeys::{
    BindState, EventSink, HotkeyBinding, HotkeyEvent, HotkeyService, Registrar, SetKind,
    portal_shortcuts, release_hotkeys,
};
pub use motion::reduced_motion;
