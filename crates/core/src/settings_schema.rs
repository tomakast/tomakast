//! The one description of every built-in `config.toml` key: its kind, default, title and
//! page. [`crate::Settings`] reads the file by walking it; the settings window shows it.

use std::sync::LazyLock;

use crate::settings::{Hotkey, Modifiers};

/// What a value is, and so which control edits it.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    Bool,
    Text,
    Number(NumberRange),
    Choice(Vec<ChoiceOption>),
    Hotkey,
    /// A leader key: a hotkey that may be modifiers alone, or empty for none.
    Leader,
    /// A city of the built-in list; empty: find the location automatically.
    City,
    /// An IANA time zone name (`Europe/Berlin`), picked from a list; unset: the computer's
    /// zone. Any string fits, since a plugin may accept more (a city).
    TimeZone,
    /// A font family, picked from the installed ones: the bundled family (the key's default),
    /// `system` for the desktop's, or a family name. Any string fits; a family that is not
    /// installed falls back to the bundled one.
    Font,
}

/// The bounds of a number value; `integer`: a float does not fit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NumberRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub integer: bool,
}

impl NumberRange {
    fn contains(&self, n: f64) -> bool {
        self.min.is_none_or(|min| n >= min) && self.max.is_none_or(|max| n <= max)
    }
}

/// One value of a [`ValueKind::Choice`]: `value` is what the file holds, `title` what the
/// user sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: String,
    pub title: String,
}

/// The settings window's page a key shows on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Launcher,
    Location,
    Appearance,
}

/// What the parser does with a value of the wrong type: refuse the file, or warn and use the
/// default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnInvalid {
    Error,
    Warn,
}

/// One built-in scalar key of `config.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingKey {
    /// The table and the key, `["launcher", "hotkey"]`.
    pub path: &'static [&'static str],
    pub kind: ValueKind,
    /// The value in effect when the file has none, as the window shows it.
    pub default: &'static str,
    pub title: &'static str,
    pub description: Option<&'static str>,
    pub page: Page,
    /// More words the window's search finds the key by.
    pub keywords: &'static [&'static str],
    pub on_invalid: OnInvalid,
}

static BUILTIN_KEYS: LazyLock<Vec<SettingKey>> = LazyLock::new(|| {
    vec![
        SettingKey {
            path: &["launcher", "hotkey"],
            kind: ValueKind::Hotkey,
            default: "Alt+Space",
            title: "Launcher hotkey",
            description: None,
            page: Page::Launcher,
            keywords: &["shortcut", "keyboard", "open"],
            on_invalid: OnInvalid::Error,
        },
        SettingKey {
            path: &["launcher", "leader"],
            kind: ValueKind::Leader,
            default: "",
            title: "Leader key",
            description: Some("Press it with another key to run that key's hotkey."),
            page: Page::Launcher,
            keywords: &["leader", "shortcut", "keyboard"],
            on_invalid: OnInvalid::Error,
        },
        SettingKey {
            path: &["launcher", "leader_mode"],
            kind: ValueKind::Choice(vec![
                ChoiceOption {
                    value: "hold".into(),
                    title: "Hold the leader".into(),
                },
                ChoiceOption {
                    value: "tap".into(),
                    title: "Press, then press a key".into(),
                },
            ]),
            default: "hold",
            title: "Leader mode",
            description: None,
            page: Page::Launcher,
            keywords: &["leader", "shortcut", "keyboard"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["launcher", "autostart"],
            kind: ValueKind::Bool,
            default: "true",
            title: "Start at login",
            description: None,
            page: Page::General,
            keywords: &["autostart", "startup", "boot"],
            on_invalid: OnInvalid::Error,
        },
        SettingKey {
            path: &["launcher", "pop_to_root"],
            kind: ValueKind::Choice(vec![
                ChoiceOption {
                    value: "immediately".into(),
                    title: "Immediately".into(),
                },
                ChoiceOption {
                    value: "30s".into(),
                    title: "After 30 seconds".into(),
                },
                ChoiceOption {
                    value: "90s".into(),
                    title: "After 90 seconds".into(),
                },
                ChoiceOption {
                    value: "3m".into(),
                    title: "After 3 minutes".into(),
                },
                ChoiceOption {
                    value: "10m".into(),
                    title: "After 10 minutes".into(),
                },
                ChoiceOption {
                    value: "never".into(),
                    title: "Never".into(),
                },
            ]),
            default: "90s",
            title: "Return back home",
            description: Some(
                "How long to wait after the launcher is hidden before its last search is \
                 cleared.",
            ),
            page: Page::Launcher,
            keywords: &[
                "return", "home", "pop", "root", "reset", "clear", "search", "timeout", "hide",
            ],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["launcher", "screen"],
            kind: ValueKind::Choice(vec![
                ChoiceOption {
                    value: "mouse".into(),
                    title: "With the mouse pointer".into(),
                },
                ChoiceOption {
                    value: "active-window".into(),
                    title: "With the active window".into(),
                },
                ChoiceOption {
                    value: "primary".into(),
                    title: "Primary screen".into(),
                },
            ]),
            default: "mouse",
            title: "Show on screen",
            description: None,
            page: Page::Launcher,
            keywords: &[
                "screen", "display", "monitor", "output", "mouse", "pointer", "primary",
            ],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["launcher", "history"],
            kind: ValueKind::Bool,
            default: "true",
            title: "Remember search history",
            description: None,
            page: Page::Launcher,
            keywords: &["history", "queries", "recent", "privacy"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["location", "city"],
            kind: ValueKind::City,
            default: "",
            title: "City",
            description: None,
            page: Page::Location,
            keywords: &["location", "weather", "place", "town"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["location", "wifi_lookup"],
            kind: ValueKind::Bool,
            default: "false",
            title: "Look up location by Wi-Fi",
            description: Some(
                "When the system cannot tell where you are, the Wi-Fi networks around are \
                 sent to BeaconDB to find out.",
            ),
            page: Page::Location,
            keywords: &["location", "wifi", "network", "beacondb", "privacy"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["location", "timezone"],
            kind: ValueKind::TimeZone,
            default: "",
            title: "Time zone",
            description: None,
            page: Page::Location,
            keywords: &["time", "zone", "timezone", "clock", "utc", "offset"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["appearance", "theme"],
            kind: ValueKind::Choice(vec![
                ChoiceOption {
                    value: "system".into(),
                    title: "System".into(),
                },
                ChoiceOption {
                    value: "light".into(),
                    title: "Light".into(),
                },
                ChoiceOption {
                    value: "dark".into(),
                    title: "Dark".into(),
                },
            ]),
            default: "system",
            title: "Theme",
            description: None,
            page: Page::Appearance,
            keywords: &["theme", "dark", "light", "appearance", "colors"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["appearance", "font"],
            kind: ValueKind::Font,
            default: "system",
            title: "Font",
            description: None,
            page: Page::Appearance,
            keywords: &["font", "typeface", "family", "text"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["appearance", "text_size"],
            kind: ValueKind::Choice(vec![
                ChoiceOption {
                    value: "small".into(),
                    title: "Small".into(),
                },
                ChoiceOption {
                    value: "default".into(),
                    title: "Default".into(),
                },
                ChoiceOption {
                    value: "large".into(),
                    title: "Large".into(),
                },
            ]),
            default: "default",
            title: "Text size",
            description: None,
            page: Page::Appearance,
            keywords: &["font", "size", "text", "bigger", "smaller", "zoom", "scale"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["appearance", "mono_font"],
            kind: ValueKind::Font,
            default: "JetBrains Mono",
            title: "Monospace font",
            description: None,
            page: Page::Appearance,
            keywords: &["font", "monospace", "mono", "code", "fixed"],
            on_invalid: OnInvalid::Warn,
        },
        SettingKey {
            path: &["appearance", "mono_size"],
            kind: ValueKind::Number(NumberRange {
                min: Some(8.),
                max: Some(32.),
                step: Some(1.),
                integer: true,
            }),
            default: "14",
            title: "Monospace size",
            description: None,
            page: Page::Appearance,
            keywords: &["font", "monospace", "mono", "code", "size"],
            on_invalid: OnInvalid::Warn,
        },
    ]
});

/// Every built-in scalar key, in the order the window shows them.
pub fn builtin_keys() -> &'static [SettingKey] {
    &BUILTIN_KEYS
}

/// The table of plugin switches: `<id> = false` turns a plugin off. Its own table, not a key
/// of `[plugins.<id>]`, whose keys are the plugin's preferences (one may be `enabled`).
pub const ENABLED_PLUGINS: &str = "enabled_plugins";

/// The keys that hold many entries, each with an editor of its own: `[hotkeys]`,
/// `[[home.widgets]]`, the `[plugins.<id>]` tables and the plugin switches.
pub const COLLECTIONS: &[&[&str]] = &[
    &["hotkeys"],
    &["home", "widgets"],
    &["plugins"],
    &[ENABLED_PLUGINS],
];

/// The keys of `[files]`: read by [`crate::Settings`] (`FilesSettings`, its parser warns about
/// a wrong type), known to [`unrecognized`], but with no control in the settings window.
pub const FILES_KEYS: &[&[&str]] = &[
    &["files", "in_root"],
    &["files", "roots"],
    &["files", "exclude"],
    &["files", "hidden"],
    &["files", "gitignore"],
];

/// A key's value in a file.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyValue {
    /// Not in the file, or a table on its path is not a table.
    Missing,
    Valid(toml::Value),
    /// There, but not of the key's kind.
    Invalid(toml::Value),
}

/// `key`'s value in `file` (a whole config file).
pub fn read_key(file: &toml::Table, key: &SettingKey) -> KeyValue {
    let (last, tables) = key.path.split_last().expect("a key path is never empty");
    let mut table = file;
    for name in tables {
        match table.get(*name) {
            Some(toml::Value::Table(t)) => table = t,
            _ => return KeyValue::Missing,
        }
    }
    match table.get(*last) {
        None => KeyValue::Missing,
        Some(value) if value_fits(&key.kind, value) => KeyValue::Valid(value.clone()),
        Some(value) => KeyValue::Invalid(value.clone()),
    }
}

/// Whether `value` is one of `kind`: a boolean for `Bool`, a string for `Text`, `City`,
/// `TimeZone` and `Font`, a string that parses as a hotkey without `Leader` for `Hotkey` (a
/// leader key, or blank, for `Leader`), an integer (or a float unless `integer`) within bounds
/// for `Number`, one of the option values for `Choice`.
pub fn value_fits(kind: &ValueKind, value: &toml::Value) -> bool {
    use toml::Value;
    match (kind, value) {
        (ValueKind::Bool, Value::Boolean(_)) => true,
        (
            ValueKind::Text | ValueKind::City | ValueKind::TimeZone | ValueKind::Font,
            Value::String(_),
        ) => true,
        (ValueKind::Hotkey, Value::String(s)) => {
            Hotkey::parse(s).is_ok_and(|h| !h.mods.contains(Modifiers::LEADER))
        }
        (ValueKind::Leader, Value::String(s)) => {
            s.trim().is_empty() || Hotkey::parse_leader(s).is_ok()
        }
        (ValueKind::Number(range), Value::Integer(n)) => range.contains(*n as f64),
        (ValueKind::Number(range), Value::Float(f)) => !range.integer && range.contains(*f),
        (ValueKind::Choice(options), Value::String(s)) => options.iter().any(|o| o.value == *s),
        _ => false,
    }
}

/// A key of the file the launcher gives no meaning to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unrecognized {
    pub path: Vec<String>,
    /// The value as TOML text.
    pub value: String,
    pub reason: String,
}

/// The keys of `file` (a whole config file) outside the schema, sorted by path: top-level
/// keys that are neither a built-in table nor a collection (the legacy `widgets` among them,
/// as "no longer a setting"), and unknown keys inside `[launcher]`, `[location]` and
/// `[home]`. A built-in table that is not a table is listed as [`NOT_A_TABLE`] when the
/// parser skips it (`location`, `home`); one it refuses (`launcher`, `hotkeys`, `plugins`)
/// makes the file an error instead.
pub fn unrecognized(file: &toml::Table) -> Vec<Unrecognized> {
    let entry = |path: Vec<String>, value: &toml::Value, reason: &str| Unrecognized {
        path,
        value: value.to_string(),
        reason: reason.to_owned(),
    };
    let mut out = Vec::new();
    for (name, value) in file {
        if name == "widgets" {
            out.push(entry(vec![name.clone()], value, "no longer a setting"));
            continue;
        }
        if !known_paths().any(|p| p[0] == name) {
            out.push(entry(vec![name.clone()], value, "not a setting"));
            continue;
        }
        let toml::Value::Table(table) = value else {
            if skipped_when_not_a_table(name) {
                out.push(entry(vec![name.clone()], value, NOT_A_TABLE));
            }
            continue;
        };
        let children: Vec<&str> = known_paths()
            .filter(|p| p.len() == 2 && p[0] == name)
            .map(|p| p[1])
            .collect();
        if children.is_empty() {
            continue;
        }
        for (key, value) in table {
            if !children.contains(&key.as_str()) {
                let reason = format!("not a setting of [{name}]");
                out.push(entry(vec![name.clone(), key.clone()], value, &reason));
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The reason [`unrecognized`] gives a built-in table that is not a table.
pub const NOT_A_TABLE: &str = "not a table";

/// Whether the parser skips the built-in table `name` when it is not a table, rather than
/// refusing the file: neither a top-level collection nor the table of a key that is an error
/// when wrong.
fn skipped_when_not_a_table(name: &str) -> bool {
    !COLLECTIONS.iter().any(|c| *c == [name])
        && !builtin_keys()
            .iter()
            .any(|k| k.path[0] == name && k.on_invalid == OnInvalid::Error)
}

/// The path of every built-in key and collection.
fn known_paths() -> impl Iterator<Item = &'static [&'static str]> {
    builtin_keys()
        .iter()
        .map(|k| k.path)
        .chain(COLLECTIONS.iter().copied())
        .chain(FILES_KEYS.iter().copied())
}
