//! Manual check: binds Alt+Space through this desktop's hotkey backend and prints every
//! event (presses, outcomes). On KDE it registers under tomakast's own component.
use tomakast_core::Hotkey;
use tomakast_platform::{HotkeyBinding, HotkeyService, Registrar as _, SetKind};

fn main() {
    let mut service = HotkeyService::start(|event| println!("{event:?}"));
    service.set(
        SetKind::Launcher,
        1,
        vec![HotkeyBinding {
            id: "launcher".into(),
            description: "Open tomakast".into(),
            hotkey: Hotkey::parse("Alt+Space").unwrap(),
        }],
    );
    service.set(SetKind::Items, 2, Vec::new());
    loop {
        std::thread::park();
    }
}
