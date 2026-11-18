//! Manual check: registers Alt+Space through the portal and prints activations.
use tomakast_core::Hotkey;
use tomakast_platform::{HotkeyBinding, register_global_hotkeys};

fn main() {
    let backend = register_global_hotkeys(
        vec![HotkeyBinding {
            id: "launcher".into(),
            description: "Open tomakast".into(),
            hotkey: Hotkey::parse("Alt+Space").unwrap(),
        }],
        |id| println!("activated {id}"),
    );
    println!("backend: {backend:?}");
    loop {
        std::thread::park();
    }
}
