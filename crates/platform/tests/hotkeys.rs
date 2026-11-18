use tomakast_core::Hotkey;
use tomakast_platform::{HotkeyBinding, portal_shortcuts};

#[test]
fn portal_shortcuts_use_xdg_triggers() {
    let bindings = vec![
        HotkeyBinding {
            id: "launcher".into(),
            description: "Open tomakast".into(),
            hotkey: Hotkey::parse("Alt+Space").unwrap(),
        },
        HotkeyBinding {
            id: "apps/firefox".into(),
            description: "Firefox".into(),
            hotkey: Hotkey::parse("Ctrl+Shift+F").unwrap(),
        },
    ];
    assert_eq!(
        portal_shortcuts(&bindings),
        vec![
            (
                "launcher".to_string(),
                "Open tomakast".to_string(),
                "ALT+space".to_string()
            ),
            (
                "apps/firefox".to_string(),
                "Firefox".to_string(),
                "CTRL+SHIFT+f".to_string()
            ),
        ]
    );
}
