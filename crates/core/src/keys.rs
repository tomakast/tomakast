//! The keys a global hotkey can use, and their names in the APIs that bind them.
//!
//! A key is named as in `config.toml` (`t`, `f1`, `space`), the lowercase form
//! [`Hotkey::parse`](crate::Hotkey::parse) keeps. Each has an xkb keysym (the
//! GlobalShortcuts portal's trigger syntax) and a Qt key code (KDE's kglobalaccel).

use crate::settings::{Hotkey, Modifiers};

/// (config name, xkb keysym, Qt key code), for every key a hotkey may use.
#[rustfmt::skip]
const TABLE: &[(&str, &str, u32)] = &[
    ("a", "a", 0x41), ("b", "b", 0x42), ("c", "c", 0x43), ("d", "d", 0x44),
    ("e", "e", 0x45), ("f", "f", 0x46), ("g", "g", 0x47), ("h", "h", 0x48),
    ("i", "i", 0x49), ("j", "j", 0x4a), ("k", "k", 0x4b), ("l", "l", 0x4c),
    ("m", "m", 0x4d), ("n", "n", 0x4e), ("o", "o", 0x4f), ("p", "p", 0x50),
    ("q", "q", 0x51), ("r", "r", 0x52), ("s", "s", 0x53), ("t", "t", 0x54),
    ("u", "u", 0x55), ("v", "v", 0x56), ("w", "w", 0x57), ("x", "x", 0x58),
    ("y", "y", 0x59), ("z", "z", 0x5a),
    ("0", "0", 0x30), ("1", "1", 0x31), ("2", "2", 0x32), ("3", "3", 0x33),
    ("4", "4", 0x34), ("5", "5", 0x35), ("6", "6", 0x36), ("7", "7", 0x37),
    ("8", "8", 0x38), ("9", "9", 0x39),
    ("f1", "F1", 0x0100_0030), ("f2", "F2", 0x0100_0031), ("f3", "F3", 0x0100_0032),
    ("f4", "F4", 0x0100_0033), ("f5", "F5", 0x0100_0034), ("f6", "F6", 0x0100_0035),
    ("f7", "F7", 0x0100_0036), ("f8", "F8", 0x0100_0037), ("f9", "F9", 0x0100_0038),
    ("f10", "F10", 0x0100_0039), ("f11", "F11", 0x0100_003a), ("f12", "F12", 0x0100_003b),
    ("space", "space", 0x20),
    ("enter", "Return", 0x0100_0004),
    ("tab", "Tab", 0x0100_0001),
    ("backspace", "BackSpace", 0x0100_0003),
    ("insert", "Insert", 0x0100_0006),
    ("delete", "Delete", 0x0100_0007),
    ("home", "Home", 0x0100_0010),
    ("end", "End", 0x0100_0011),
    ("left", "Left", 0x0100_0012),
    ("up", "Up", 0x0100_0013),
    ("right", "Right", 0x0100_0014),
    ("down", "Down", 0x0100_0015),
    ("pageup", "Prior", 0x0100_0016),
    ("pagedown", "Next", 0x0100_0017),
    ("-", "minus", 0x2d),
    ("=", "equal", 0x3d),
    ("[", "bracketleft", 0x5b),
    ("]", "bracketright", 0x5d),
    ("\\", "backslash", 0x5c),
    (";", "semicolon", 0x3b),
    ("'", "apostrophe", 0x27),
    (",", "comma", 0x2c),
    (".", "period", 0x2e),
    ("/", "slash", 0x2f),
    ("`", "grave", 0x60),
];

/// The config names of the supported keys, in table order.
#[rustfmt::skip]
pub const SUPPORTED_KEYS: &[&str] = &[
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
    "s", "t", "u", "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "space", "enter",
    "tab", "backspace", "insert", "delete", "home", "end", "left", "up", "right", "down",
    "pageup", "pagedown", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/", "`",
];

/// US-layout shifted symbols and the key that types them.
#[rustfmt::skip]
const SHIFTED: &[(&str, &str)] = &[
    ("!", "1"), ("@", "2"), ("#", "3"), ("$", "4"), ("%", "5"), ("^", "6"), ("&", "7"),
    ("*", "8"), ("(", "9"), (")", "0"), ("_", "-"), ("+", "="), ("{", "["), ("}", "]"),
    ("|", "\\"), (":", ";"), ("\"", "'"), ("<", ","), (">", "."), ("?", "/"), ("~", "`"),
];

fn entry(key: &str) -> Option<&'static (&'static str, &'static str, u32)> {
    TABLE.iter().find(|(name, _, _)| *name == key)
}

/// Whether `key` (a config name) can be a hotkey's key.
pub fn is_supported(key: &str) -> bool {
    entry(key).is_some()
}

/// The xkb keysym name of `key`, as the portal's trigger syntax wants it.
pub fn xkb_keysym(key: &str) -> Option<&'static str> {
    entry(key).map(|(_, keysym, _)| *keysym)
}

/// The Qt key code of `key`, without modifiers.
pub fn qt_key(key: &str) -> Option<u32> {
    entry(key).map(|(_, _, code)| *code)
}

/// `hotkey` as one Qt key code with its modifiers, as kglobalaccel stores it.
pub fn qt_code(hotkey: &Hotkey) -> Option<i32> {
    let mut code = qt_key(&hotkey.key)?;
    for (flag, bit) in [
        (Modifiers::SHIFT, 0x0200_0000),
        (Modifiers::CTRL, 0x0400_0000),
        (Modifiers::ALT, 0x0800_0000),
        (Modifiers::SUPER, 0x1000_0000),
    ] {
        if hotkey.mods.contains(flag) {
            code |= bit;
        }
    }
    i32::try_from(code).ok()
}

/// The symbol Shift+`key` types on a US layout (`1` → `!`).
pub fn shifted(key: &str) -> Option<&'static str> {
    SHIFTED
        .iter()
        .find(|(_, base)| *base == key)
        .map(|(symbol, _)| *symbol)
}

/// The key that types the shifted symbol `symbol` on a US layout (`!` → `1`).
pub fn unshifted(symbol: &str) -> Option<&'static str> {
    SHIFTED
        .iter()
        .find(|(shifted, _)| *shifted == symbol)
        .map(|(_, base)| *base)
}

/// How `key` (a config name) is named to the user: `pagedown` -> `Page Down`, `f2` -> `F2`,
/// `c` -> `C`, `left` -> `Left`. Not the file's form, which stays the config name.
pub fn display_name(key: &str) -> String {
    let named = match key {
        "pageup" => Some("Page Up"),
        "pagedown" => Some("Page Down"),
        "backspace" => Some("Backspace"),
        "capslock" => Some("Caps Lock"),
        "numlock" => Some("Num Lock"),
        "scrolllock" => Some("Scroll Lock"),
        "print" | "printscreen" => Some("Print Screen"),
        "esc" | "escape" => Some("Esc"),
        "return" => Some("Enter"),
        _ => None,
    };
    if let Some(named) = named {
        return named.to_string();
    }
    let function_key = key
        .strip_prefix('f')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if key.chars().count() == 1 || function_key {
        return key.to_uppercase();
    }
    let mut chars = key.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_have_their_written_names() {
        for (key, name) in [
            ("pageup", "Page Up"),
            ("pagedown", "Page Down"),
            ("backspace", "Backspace"),
            ("capslock", "Caps Lock"),
            ("numlock", "Num Lock"),
            ("scrolllock", "Scroll Lock"),
            ("print", "Print Screen"),
            ("escape", "Esc"),
            ("left", "Left"),
            ("up", "Up"),
            ("right", "Right"),
            ("down", "Down"),
            ("home", "Home"),
            ("end", "End"),
            ("insert", "Insert"),
            ("delete", "Delete"),
            ("space", "Space"),
            ("tab", "Tab"),
            ("enter", "Enter"),
            ("f12", "F12"),
            ("k", "K"),
            ("/", "/"),
        ] {
            assert_eq!(display_name(key), name, "{key}");
        }
    }

    #[test]
    fn a_hotkey_reads_with_the_keys_name_and_keeps_its_file_form() {
        let hotkey = hk("ctrl+alt+pagedown");
        assert_eq!(hotkey.label(), "Ctrl+Alt+Page Down");
        assert_eq!(hotkey.to_string(), "Ctrl+Alt+Pagedown");
        assert_eq!(Hotkey::parse(&hotkey.to_string()), Ok(hotkey));
    }

    #[test]
    fn every_supported_key_has_a_name_without_lowercase_words() {
        for key in SUPPORTED_KEYS {
            let name = display_name(key);
            assert!(
                name.split(' ')
                    .all(|word| !word.starts_with(char::is_lowercase)),
                "{key}: {name}"
            );
        }
    }

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    #[test]
    fn the_supported_keys_are_the_table() {
        let names: Vec<&str> = TABLE.iter().map(|(name, _, _)| *name).collect();
        assert_eq!(names, SUPPORTED_KEYS);
    }

    #[test]
    fn every_supported_key_has_a_keysym() {
        for key in SUPPORTED_KEYS {
            assert!(xkb_keysym(key).is_some(), "{key}");
        }
    }

    #[test]
    fn every_supported_key_has_a_qt_code() {
        for key in SUPPORTED_KEYS {
            assert!(qt_key(key).is_some(), "{key}");
        }
    }

    #[test]
    fn named_keys_map_to_xkb_names() {
        for (key, keysym) in [
            ("t", "t"),
            ("7", "7"),
            ("f1", "F1"),
            ("enter", "Return"),
            ("pageup", "Prior"),
            ("pagedown", "Next"),
            ("-", "minus"),
            ("[", "bracketleft"),
            ("`", "grave"),
            ("'", "apostrophe"),
            ("space", "space"),
        ] {
            assert_eq!(xkb_keysym(key), Some(keysym), "{key}");
        }
    }

    #[test]
    fn qt_codes_match_kde() {
        // Read back from kglobalaccel: kitty's Ctrl+Alt+T and KWin's Alt+F4.
        assert_eq!(qt_code(&hk("ctrl+alt+t")), Some(201326676));
        assert_eq!(qt_code(&hk("alt+f4")), Some(150994995));
        assert_eq!(qt_code(&hk("ctrl+alt+shift+k")), Some(234881099));
        assert_eq!(qt_key("enter"), Some(0x0100_0004));
        assert_eq!(qt_key("pagedown"), Some(0x0100_0017));
    }

    #[test]
    fn shifted_symbols_map_to_their_base_key() {
        assert_eq!(unshifted("!"), Some("1"));
        assert_eq!(unshifted(")"), Some("0"));
        assert_eq!(unshifted("?"), Some("/"));
        assert_eq!(unshifted("~"), Some("`"));
        assert_eq!(unshifted("a"), None);
    }

    #[test]
    fn base_keys_map_to_their_shifted_symbol() {
        assert_eq!(shifted("1"), Some("!"));
        assert_eq!(shifted("/"), Some("?"));
        assert_eq!(shifted("a"), None);
    }

    #[test]
    fn non_latin_and_reserved_keys_are_not_supported() {
        assert!(!is_supported("е"));
        assert!(!is_supported("escape"));
        assert!(is_supported("f12"));
        assert!(!is_supported("f13"));
    }
}
