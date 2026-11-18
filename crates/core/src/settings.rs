//! User settings (`config.toml`) and hotkey parsing.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::Context;
use bitflags::bitflags;
use serde::Deserialize;

use crate::index::ItemId;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        const CTRL = 1;
        const ALT = 1 << 1;
        const SHIFT = 1 << 2;
        const SUPER = 1 << 3;
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("empty hotkey")]
    Empty,
    #[error("hotkey `{0}` has no key")]
    MissingKey(String),
    #[error("hotkey `{0}` has more than one key")]
    MultipleKeys(String),
    #[error("unknown modifier `{0}`")]
    UnknownModifier(String),
}

/// A key combination; `key` is the lowercase key name (`"space"`, `"t"`, `"f1"`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub mods: Modifiers,
    pub key: String,
}

fn modifier(name: &str) -> Option<Modifiers> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifiers::CTRL),
        "alt" => Some(Modifiers::ALT),
        "shift" => Some(Modifiers::SHIFT),
        "super" | "meta" | "logo" | "win" => Some(Modifiers::SUPER),
        _ => None,
    }
}

/// Splits `Ctrl+Alt+T` into modifiers and the key; a trailing known modifier
/// name (`Ctrl+Alt`) is a missing key, not a key.
impl Hotkey {
    pub fn parse(s: &str) -> Result<Hotkey, HotkeyError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(HotkeyError::Empty);
        }
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (last, rest) = parts.split_last().expect("split yields at least one part");
        if last.is_empty() {
            return Err(HotkeyError::MissingKey(s.to_string()));
        }
        let mut mods = Modifiers::empty();
        let mut extra_key = false;
        for p in rest {
            match modifier(p) {
                Some(m) => mods |= m,
                None if p.is_empty() => return Err(HotkeyError::MissingKey(s.to_string())),
                // A non-modifier before the last part: a second key if it looks
                // like a key name, otherwise an unknown modifier.
                None if p.chars().count() == 1 || is_named_key(p) => extra_key = true,
                None => return Err(HotkeyError::UnknownModifier(p.to_string())),
            }
        }
        if extra_key {
            return Err(HotkeyError::MultipleKeys(s.to_string()));
        }
        if modifier(last).is_some() {
            return Err(HotkeyError::MissingKey(s.to_string()));
        }
        Ok(Hotkey {
            mods,
            key: last.to_ascii_lowercase(),
        })
    }

    /// `xdg-desktop-portal` shortcut trigger: `CTRL+ALT+SHIFT+LOGO+<xkb key>`.
    pub fn to_xdg_trigger(&self) -> String {
        let mut out = String::new();
        for (flag, name) in [
            (Modifiers::CTRL, "CTRL"),
            (Modifiers::ALT, "ALT"),
            (Modifiers::SHIFT, "SHIFT"),
            (Modifiers::SUPER, "LOGO"),
        ] {
            if self.mods.contains(flag) {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&self.key);
        out
    }
}

fn is_named_key(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "space" | "enter" | "return" | "tab" | "escape" | "esc" | "backspace"
    ) || (s.len() > 1 && s.starts_with(['f', 'F']) && s[1..].chars().all(|c| c.is_ascii_digit()))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in [
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::SUPER, "Super"),
        ] {
            if self.mods.contains(flag) {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&capitalize(&self.key))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub launcher_hotkey: Hotkey,
    pub autostart: bool,
    pub hotkeys: Vec<(Hotkey, ItemId)>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            launcher_hotkey: Hotkey {
                mods: Modifiers::ALT,
                key: "space".into(),
            },
            autostart: true,
            hotkeys: Vec::new(),
        }
    }
}

#[derive(Deserialize, Default)]
struct RawSettings {
    #[serde(default)]
    launcher: RawLauncher,
    #[serde(default)]
    hotkeys: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct RawLauncher {
    hotkey: Option<String>,
    autostart: Option<bool>,
}

impl Settings {
    /// Loads `path`; a missing file yields the defaults.
    pub fn load(path: &Path) -> anyhow::Result<Settings> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Settings::from_toml(&text).map_err(|e| anyhow::anyhow!("loading {}: {e:#}", path.display()))
    }

    /// Never fails: on error returns the defaults together with the error.
    pub fn load_or_default(path: &Path) -> (Settings, Option<anyhow::Error>) {
        match Settings::load(path) {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(e)),
        }
    }

    fn from_toml(text: &str) -> anyhow::Result<Settings> {
        let raw: RawSettings = toml::from_str(text).context("invalid TOML")?;
        let defaults = Settings::default();
        let launcher_hotkey = match raw.launcher.hotkey {
            Some(s) => Hotkey::parse(&s).with_context(|| format!("[launcher] hotkey `{s}`"))?,
            None => defaults.launcher_hotkey,
        };
        let mut hotkeys = Vec::new();
        for (key, id) in raw.hotkeys {
            let hotkey = Hotkey::parse(&key).with_context(|| format!("[hotkeys] key `{key}`"))?;
            let id = ItemId::parse(&id)
                .with_context(|| format!("[hotkeys] `{key}`: invalid item id `{id}`"))?;
            hotkeys.push((hotkey, id));
        }
        Ok(Settings {
            launcher_hotkey,
            autostart: raw.launcher.autostart.unwrap_or(defaults.autostart),
            hotkeys,
        })
    }
}
