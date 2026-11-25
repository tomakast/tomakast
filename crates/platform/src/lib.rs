//! OS integration: global hotkeys, autostart, opening URLs and files, icon themes, KWin's
//! title-bar buttons, the desktop's fonts and its reduced-motion setting.

pub mod autostart;
pub mod hotkeys;
pub mod icon_theme;
pub mod kwin_buttons;
pub mod motion;
pub mod open;
pub mod reveal;
pub mod screens;
pub mod system_fonts;

pub use autostart::{set_autostart, set_autostart_in};
pub use hotkeys::{
    BindState, EventSink, HotkeyBinding, HotkeyEvent, HotkeyService, Registrar, SetKind,
    portal_shortcuts, release_hotkeys,
};
pub use kwin_buttons::{FrameButton, kwin_button_layout, kwin_buttons};
pub use motion::reduced_motion;
#[cfg(target_os = "linux")]
pub use reveal::file_uri;
pub use reveal::reveal_path;
pub use screens::screen_choices;
pub use system_fonts::{FontKind, system_font};
