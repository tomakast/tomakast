//! User settings (`config.toml`) and hotkey parsing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::index::ItemId;
use crate::leader::LeaderMode;
use crate::settings_schema::{
    ENABLED_PLUGINS, KeyValue, NOT_A_TABLE, OnInvalid, SettingKey, ValueKind, builtin_keys,
    read_key, unrecognized,
};
use crate::widget::{Params, WidgetKey, WidgetSize, params};
use anyhow::Context;
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        const CTRL = 1;
        const ALT = 1 << 1;
        const SHIFT = 1 << 2;
        const SUPER = 1 << 3;
        /// The leader key stands in for these: a hotkey that starts with `Leader` runs while
        /// the leader is held. Never sent to a hotkey backend.
        const LEADER = 1 << 4;
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
    #[error("a leader key cannot contain `Leader`")]
    LeaderInLeader,
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
        "leader" => Some(Modifiers::LEADER),
        _ => None,
    }
}

/// Splits `Ctrl+Alt+T` into modifiers and the key; a trailing known modifier
/// name (`Ctrl+Alt`) is a missing key, not a key.
impl Hotkey {
    pub fn parse(s: &str) -> Result<Hotkey, HotkeyError> {
        Hotkey::parse_parts(s, false)
    }

    /// Like [`Hotkey::parse`], but modifiers alone (`Ctrl`, `Ctrl+Alt`) are a hotkey too, with an
    /// empty key, and `Leader` is refused.
    pub fn parse_leader(s: &str) -> Result<Hotkey, HotkeyError> {
        let hotkey = Hotkey::parse_parts(s, true)?;
        if hotkey.mods.contains(Modifiers::LEADER) {
            return Err(HotkeyError::LeaderInLeader);
        }
        Ok(hotkey)
    }

    /// Whether the hotkey has no key, only modifiers (a leader key may be that).
    pub fn is_modifiers_only(&self) -> bool {
        self.key.is_empty()
    }

    fn parse_parts(s: &str, modifiers_alone: bool) -> Result<Hotkey, HotkeyError> {
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
        if let Some(m) = modifier(last) {
            if !modifiers_alone {
                return Err(HotkeyError::MissingKey(s.to_string()));
            }
            return Ok(Hotkey {
                mods: mods | m,
                key: String::new(),
            });
        }
        Ok(Hotkey {
            mods,
            key: last.to_ascii_lowercase(),
        })
    }

    /// `xdg-desktop-portal` shortcut trigger: `CTRL+ALT+SHIFT+LOGO+<xkb keysym>`; `None` when
    /// the key is not one a hotkey can use ([`crate::keys`]).
    pub fn to_xdg_trigger(&self) -> Option<String> {
        if self.mods.contains(Modifiers::LEADER) {
            return None;
        }
        let keysym = crate::keys::xkb_keysym(&self.key)?;
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
        out.push_str(keysym);
        Some(out)
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

impl Hotkey {
    /// The hotkey as the user reads it: `Ctrl+Alt+Page Down`, `Leader+Page Up`, `Ctrl+Alt` for
    /// a modifiers-only one. [`Display`](fmt::Display) is the form `config.toml` keeps
    /// (`Ctrl+Alt+Pagedown`).
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (flag, name) in [
            (Modifiers::LEADER, "Leader"),
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::SUPER, "Super"),
        ] {
            if self.mods.contains(flag) {
                out.push_str(name);
                out.push('+');
            }
        }
        if self.is_modifiers_only() {
            out.pop();
        } else {
            out.push_str(&crate::keys::display_name(&self.key));
        }
        out
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = [
            (Modifiers::LEADER, "Leader"),
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::SUPER, "Super"),
        ]
        .into_iter()
        .filter(|(flag, _)| self.mods.contains(*flag))
        .map(|(_, name)| name.to_owned())
        .collect();
        if !self.is_modifiers_only() {
            parts.push(capitalize(&self.key));
        }
        f.write_str(&parts.join("+"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub launcher_hotkey: Hotkey,
    pub autostart: bool,
    /// The leader key, from `[launcher] leader`; `None`: no leader.
    pub leader: Option<Hotkey>,
    pub leader_mode: LeaderMode,
    pub hotkeys: Vec<(Hotkey, ItemId)>,
    /// The user's plugin preferences, from the `[plugins.<id>]` tables, by plugin id and
    /// key. Values are strings; a boolean is `"true"` or `"false"`.
    pub plugin_preferences: BTreeMap<String, BTreeMap<String, String>>,
    /// The plugins the user turned off, by id: the `false` entries of `[enabled_plugins]`.
    /// An off plugin is not run at all; everything else of it stays in the file.
    pub disabled_plugins: BTreeSet<String>,
    /// The user's Home, from the `[[home.widgets]]` entries, in order. The same widget may
    /// appear more than once.
    pub home: Vec<HomeEntry>,
    pub location: LocationSettings,
    /// The `[appearance]` `theme`.
    pub appearance: ThemeChoice,
    /// The `[appearance]` fonts and text sizes.
    pub text: TextSettings,
    /// How the launcher behaves when it is shown and hidden (`[launcher]`).
    pub launcher: LauncherSettings,
    pub files: FilesSettings,
}

/// A font family setting: the family the launcher bundles, the desktop's, or a family by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FontChoice {
    #[default]
    Bundled,
    /// The desktop's font of this kind (its UI font, or its monospace font).
    System,
    Family(String),
}

impl FontChoice {
    /// The value of a font key: `system`, the bundled family's name `bundled`, or any other
    /// family name; an empty value is the bundled family.
    pub fn parse(value: &str, bundled: &str) -> FontChoice {
        match value.trim() {
            "" => FontChoice::Bundled,
            "system" => FontChoice::System,
            family if family == bundled => FontChoice::Bundled,
            family => FontChoice::Family(family.to_owned()),
        }
    }
}

/// How large the launcher's and the settings window's text is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextSize {
    Small,
    #[default]
    Default,
    Large,
}

impl TextSize {
    pub fn parse(s: &str) -> Option<TextSize> {
        match s {
            "small" => Some(TextSize::Small),
            "default" => Some(TextSize::Default),
            "large" => Some(TextSize::Large),
            _ => None,
        }
    }

    /// The factor every text size is multiplied by: row text is 13, 14 or 16 px.
    pub fn scale(self) -> f32 {
        match self {
            TextSize::Small => 13. / 14.,
            TextSize::Default => 1.,
            TextSize::Large => 16. / 14.,
        }
    }
}

/// The `[appearance]` keys of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSettings {
    pub font: FontChoice,
    pub mono_font: FontChoice,
    pub size: TextSize,
    /// The size of monospace text (sheets, code blocks), in pixels; not scaled by `size`.
    pub mono_size: u8,
}

impl Default for TextSettings {
    fn default() -> TextSettings {
        TextSettings {
            font: FontChoice::System,
            mono_font: FontChoice::Bundled,
            size: TextSize::Default,
            mono_size: 14,
        }
    }
}

/// The bundled UI family: the UI font when the desktop's (the default of `[appearance] font`)
/// cannot be read.
pub const BUNDLED_FONT: &str = "Inter";
/// The bundled monospace family, the default of `[appearance] mono_font`.
pub const BUNDLED_MONO_FONT: &str = "JetBrains Mono";

/// How long after hiding the launcher opens again on what it showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopToRoot {
    Immediately,
    After(std::time::Duration),
    Never,
}

impl Default for PopToRoot {
    fn default() -> PopToRoot {
        PopToRoot::After(std::time::Duration::from_secs(90))
    }
}

impl PopToRoot {
    pub fn parse(s: &str) -> Option<PopToRoot> {
        let after = |secs| Some(PopToRoot::After(std::time::Duration::from_secs(secs)));
        match s {
            "immediately" => Some(PopToRoot::Immediately),
            "30s" => after(30),
            "90s" => after(90),
            "3m" => after(180),
            "10m" => after(600),
            "never" => Some(PopToRoot::Never),
            _ => None,
        }
    }
}

/// Which screen the launcher opens on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenChoice {
    #[default]
    Mouse,
    ActiveWindow,
    Primary,
}

impl ScreenChoice {
    pub fn parse(s: &str) -> Option<ScreenChoice> {
        match s {
            "mouse" => Some(ScreenChoice::Mouse),
            "active-window" => Some(ScreenChoice::ActiveWindow),
            "primary" => Some(ScreenChoice::Primary),
            _ => None,
        }
    }

    /// The value the file holds.
    pub fn value(self) -> &'static str {
        match self {
            ScreenChoice::Mouse => "mouse",
            ScreenChoice::ActiveWindow => "active-window",
            ScreenChoice::Primary => "primary",
        }
    }
}

/// The `[launcher]` keys of showing and hiding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherSettings {
    pub pop_to_root: PopToRoot,
    pub screen: ScreenChoice,
    /// Record the queries searched; off, nothing new is recorded.
    pub history: bool,
}

impl Default for LauncherSettings {
    fn default() -> LauncherSettings {
        LauncherSettings {
            pop_to_root: PopToRoot::default(),
            screen: ScreenChoice::default(),
            history: true,
        }
    }
}

/// Which colors the launcher uses: the system's setting, or always light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    /// `"system"`, `"light"` or `"dark"`.
    pub fn parse(s: &str) -> Option<ThemeChoice> {
        match s {
            "system" => Some(ThemeChoice::System),
            "light" => Some(ThemeChoice::Light),
            "dark" => Some(ThemeChoice::Dark),
            _ => None,
        }
    }
}

/// The `[location]` table: where the launcher takes the computer's location from, for plugins
/// with the `location` permission.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocationSettings {
    /// A city of the built-in list, trimmed; empty: find the location automatically.
    pub city: String,
    /// Look the location up from the Wi-Fi networks around (sent to BeaconDB) when the
    /// operating system cannot tell. Off unless the user turns it on.
    pub wifi_lookup: bool,
    /// The time zone plugins are told, trimmed; empty: the computer's.
    pub timezone: String,
}

/// The `[files]` table: what the file search looks through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesSettings {
    /// Files and folders may answer in the root list.
    pub in_root: bool,
    /// Folders to look through, as written (`~` is not expanded here).
    pub roots: Vec<String>,
    /// Paths and patterns to leave out, as written.
    pub exclude: Vec<String>,
    /// Include hidden entries.
    pub hidden: bool,
    /// Inside git repositories, leave out what `.gitignore` files, `.git/info/exclude` and the
    /// global gitignore list.
    pub gitignore: bool,
}

impl Default for FilesSettings {
    fn default() -> FilesSettings {
        FilesSettings {
            in_root: true,
            roots: vec!["~".into()],
            exclude: Vec::new(),
            hidden: false,
            gitignore: true,
        }
    }
}

/// One tile of the user's Home: a widget, optionally a size of the user's choosing, and the
/// widget's own settings (a city, units).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeEntry {
    pub widget: WidgetKey,
    /// `None`: the size the widget declares.
    pub size: Option<WidgetSize>,
    pub params: Params,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            launcher_hotkey: Hotkey {
                mods: Modifiers::ALT,
                key: "space".into(),
            },
            autostart: true,
            leader: None,
            leader_mode: LeaderMode::default(),
            hotkeys: Vec::new(),
            plugin_preferences: BTreeMap::new(),
            disabled_plugins: BTreeSet::new(),
            home: Vec::new(),
            location: LocationSettings::default(),
            appearance: ThemeChoice::default(),
            text: TextSettings::default(),
            launcher: LauncherSettings::default(),
            files: FilesSettings::default(),
        }
    }
}

impl Settings {
    /// Loads `path`; a missing file yields the defaults.
    pub fn load(path: &Path) -> anyhow::Result<Settings> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Settings::parse(&text).map_err(|e| anyhow::anyhow!("loading {}: {e:#}", path.display()))
    }

    /// Never fails: on error returns the defaults together with the error.
    pub fn load_or_default(path: &Path) -> (Settings, Option<anyhow::Error>) {
        match Settings::load(path) {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(e)),
        }
    }

    /// Reads the text of a `config.toml`. Fails on invalid TOML, a malformed `[hotkeys]`
    /// entry and a built-in key of the wrong type that is an error ([`OnInvalid::Error`]);
    /// anything else the launcher cannot use is skipped with a warning.
    pub fn parse(text: &str) -> anyhow::Result<Settings> {
        let mut file = toml_table(text)?;
        if file.contains_key("widgets") {
            tracing::warn!("ignoring [widgets]: the strip's order is no longer a setting");
        }
        for u in unrecognized(&file) {
            // A table that is not a table is warned about where it is read.
            if u.path != ["widgets"] && u.reason != NOT_A_TABLE {
                let key = u.path.join(".");
                tracing::warn!(key, value = u.value, "ignoring {key}: {}", u.reason);
            }
        }
        check_tables(&file)?;
        let mut settings = Settings::default();
        for key in builtin_keys() {
            let Some(value) = builtin_value(&file, key)? else {
                continue;
            };
            match (key.path, value) {
                (["launcher", "hotkey"], toml::Value::String(s)) => {
                    settings.launcher_hotkey = Hotkey::parse(&s).expect("a valid value parses");
                }
                (["launcher", "leader"], toml::Value::String(s)) => {
                    settings.leader = (!s.trim().is_empty())
                        .then(|| Hotkey::parse_leader(&s).expect("a valid value parses"));
                }
                (["launcher", "leader_mode"], toml::Value::String(s)) => {
                    settings.leader_mode = match s.as_str() {
                        "tap" => LeaderMode::Tap,
                        _ => LeaderMode::Hold,
                    };
                }
                (["launcher", "autostart"], toml::Value::Boolean(on)) => settings.autostart = on,
                (["location", "city"], toml::Value::String(city)) => {
                    settings.location.city = city.trim().to_owned();
                }
                (["location", "wifi_lookup"], toml::Value::Boolean(on)) => {
                    settings.location.wifi_lookup = on;
                }
                (["appearance", "theme"], toml::Value::String(s)) => {
                    settings.appearance = ThemeChoice::parse(&s).expect("a valid value parses");
                }
                (["location", "timezone"], toml::Value::String(zone)) => {
                    settings.location.timezone = zone.trim().to_owned();
                }
                (["launcher", "pop_to_root"], toml::Value::String(s)) => {
                    settings.launcher.pop_to_root =
                        PopToRoot::parse(&s).expect("a valid value parses");
                }
                (["launcher", "screen"], toml::Value::String(s)) => {
                    settings.launcher.screen =
                        ScreenChoice::parse(&s).expect("a valid value parses");
                }
                (["launcher", "history"], toml::Value::Boolean(on)) => {
                    settings.launcher.history = on;
                }
                (["appearance", "font"], toml::Value::String(s)) => {
                    settings.text.font = FontChoice::parse(&s, BUNDLED_FONT);
                }
                (["appearance", "mono_font"], toml::Value::String(s)) => {
                    settings.text.mono_font = FontChoice::parse(&s, BUNDLED_MONO_FONT);
                }
                (["appearance", "text_size"], toml::Value::String(s)) => {
                    settings.text.size = TextSize::parse(&s).expect("a valid value parses");
                }
                (["appearance", "mono_size"], toml::Value::Integer(n)) => {
                    settings.text.mono_size = u8::try_from(n).expect("a valid value is 8 to 32");
                }
                (path, value) => unreachable!("schema key {path:?} = {value} has no field"),
            }
        }
        settings.hotkeys = match file.remove("hotkeys") {
            None => Vec::new(),
            Some(toml::Value::Table(table)) => hotkey_entries(table)?,
            Some(other) => anyhow::bail!("[hotkeys] is not a table ({})", other.type_str()),
        };
        settings.plugin_preferences = match file.remove("plugins") {
            None => BTreeMap::new(),
            Some(toml::Value::Table(table)) => plugin_preferences(table),
            Some(other) => anyhow::bail!("[plugins] is not a table ({})", other.type_str()),
        };
        settings.disabled_plugins = match file.remove(ENABLED_PLUGINS) {
            None => BTreeSet::new(),
            Some(toml::Value::Table(table)) => disabled_plugins(table),
            Some(other) => {
                anyhow::bail!("[{ENABLED_PLUGINS}] is not a table ({})", other.type_str())
            }
        };
        settings.home = home_entries(file.remove("home"));
        settings.files = files_settings(file.remove("files"));
        Ok(settings)
    }
}

/// A built-in key's table that is not a table: an error when one of its keys is an error
/// when wrong ([`OnInvalid::Error`]), otherwise skipped with a warning.
fn check_tables(file: &toml::Table) -> anyhow::Result<()> {
    let mut seen: Vec<&str> = Vec::new();
    for key in builtin_keys() {
        let name = key.path[0];
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        let Some(value) = file.get(name).filter(|v| !v.is_table()) else {
            continue;
        };
        let kind = value.type_str();
        let refuse = builtin_keys()
            .iter()
            .any(|k| k.path[0] == name && k.on_invalid == OnInvalid::Error);
        if refuse {
            anyhow::bail!("[{name}] is not a table ({kind})");
        }
        tracing::warn!(kind, "ignoring [{name}]: not a table");
    }
    Ok(())
}

/// `key`'s valid value in `file`; `None` when missing or skipped with a warning.
fn builtin_value(file: &toml::Table, key: &SettingKey) -> anyhow::Result<Option<toml::Value>> {
    let value = match read_key(file, key) {
        KeyValue::Missing => return Ok(None),
        KeyValue::Valid(value) => return Ok(Some(value)),
        KeyValue::Invalid(value) => value,
    };
    let (name, tables) = key.path.split_last().expect("a key path is never empty");
    let tables = tables.join(".");
    let expected = expected(&key.kind);
    match key.on_invalid {
        OnInvalid::Error => {
            let shown = match &value {
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let cause = match (&key.kind, &value) {
                (ValueKind::Hotkey, toml::Value::String(s)) => match Hotkey::parse(s) {
                    Err(e) => anyhow::Error::from(e),
                    Ok(h) if h.mods.contains(Modifiers::LEADER) => {
                        anyhow::anyhow!("a hotkey here cannot use `Leader`")
                    }
                    Ok(_) => anyhow::anyhow!("not {expected}"),
                },
                (ValueKind::Leader, toml::Value::String(s)) => match Hotkey::parse_leader(s) {
                    Err(e) => anyhow::Error::from(e),
                    Ok(_) => anyhow::anyhow!("not {expected}"),
                },
                _ => anyhow::anyhow!("not {expected}"),
            };
            Err(cause.context(format!("[{tables}] {name} `{shown}`")))
        }
        OnInvalid::Warn => {
            let kind = value.type_str();
            tracing::warn!(
                key = name,
                kind,
                "ignoring [{tables}] entry: not {expected}"
            );
            Ok(None)
        }
    }
}

/// What a value of `kind` is, for messages: "a boolean".
fn expected(kind: &ValueKind) -> &'static str {
    match kind {
        ValueKind::Bool => "a boolean",
        ValueKind::Text | ValueKind::City | ValueKind::TimeZone | ValueKind::Font => "a string",
        ValueKind::Number(_) => "a number within bounds",
        ValueKind::Choice(_) => "one of the choices",
        ValueKind::Hotkey => "a hotkey",
        ValueKind::Leader => "a leader key",
    }
}

/// The `[hotkeys]` entries; a key that is not a hotkey or a value that is not an item id is an
/// error naming the key.
fn hotkey_entries(table: toml::Table) -> anyhow::Result<Vec<(Hotkey, ItemId)>> {
    let mut hotkeys = Vec::new();
    for (key, id) in table {
        let hotkey = Hotkey::parse(&key).with_context(|| format!("[hotkeys] key `{key}`"))?;
        let toml::Value::String(id) = id else {
            anyhow::bail!(
                "[hotkeys] `{key}`: the item id is not a string ({})",
                id.type_str()
            );
        };
        let id = ItemId::parse(&id)
            .with_context(|| format!("[hotkeys] `{key}`: invalid item id `{id}`"))?;
        hotkeys.push((hotkey, id));
    }
    Ok(hotkeys)
}

/// The `[plugins.<id>]` tables as strings; a number is its TOML text (`"10"`, `"2.5"`). A
/// mistake in one plugin's table must not cost the user their launcher settings, so what is
/// not a string, a boolean or a number is skipped with a warning.
fn plugin_preferences(raw: toml::Table) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut all = BTreeMap::new();
    for (plugin, table) in raw {
        let toml::Value::Table(table) = table else {
            tracing::warn!(plugin, "ignoring [plugins] entry: not a table");
            continue;
        };
        let mut prefs = BTreeMap::new();
        for (key, value) in table {
            let value = match value {
                toml::Value::String(s) => s,
                toml::Value::Boolean(b) => b.to_string(),
                number @ (toml::Value::Integer(_) | toml::Value::Float(_)) => number.to_string(),
                other => {
                    let kind = other.type_str();
                    tracing::warn!(
                        plugin,
                        key,
                        kind,
                        "ignoring plugin preference: not a string, a boolean or a number"
                    );
                    continue;
                }
            };
            prefs.insert(key, value);
        }
        all.insert(plugin, prefs);
    }
    all
}

/// The ids `[enabled_plugins]` turns off: those set to `false`. `true` is on, as is a plugin
/// the table does not name; a value that is not a boolean is skipped with a warning (on).
fn disabled_plugins(raw: toml::Table) -> BTreeSet<String> {
    let mut off = BTreeSet::new();
    for (plugin, value) in raw {
        match value {
            toml::Value::Boolean(false) => {
                off.insert(plugin);
            }
            toml::Value::Boolean(true) => {}
            other => {
                let kind = other.type_str();
                tracing::warn!(
                    plugin,
                    kind,
                    "ignoring [{ENABLED_PLUGINS}] entry: not a boolean"
                );
            }
        }
    }
    off
}

/// The `[[home.widgets]]` entries. A mistake in one costs only that entry: it is skipped with
/// a warning (a missing or malformed `widget`, a parameter that is not a string or a boolean);
/// an unknown `size` is ignored and the widget's own size is used. A `[home]` that is not a
/// table, or `widgets` that is not an array of tables, yields no entries.
fn home_entries(raw: Option<toml::Value>) -> Vec<HomeEntry> {
    let mut table = match raw {
        None => return Vec::new(),
        Some(toml::Value::Table(table)) => table,
        Some(other) => {
            let kind = other.type_str();
            tracing::warn!(kind, "ignoring [home]: not a table");
            return Vec::new();
        }
    };
    let list = match table.remove("widgets") {
        None => return Vec::new(),
        Some(toml::Value::Array(list)) => list,
        Some(other) => {
            let kind = other.type_str();
            tracing::warn!(kind, "ignoring [[home.widgets]]: not an array of tables");
            return Vec::new();
        }
    };
    list.into_iter().filter_map(home_entry).collect()
}

fn home_entry(raw: toml::Value) -> Option<HomeEntry> {
    let toml::Value::Table(table) = raw else {
        tracing::warn!("ignoring [[home.widgets]] entry: not a table");
        return None;
    };
    parse_home_entry(table)
}

/// Each element of `home.widgets` of `file` (a whole config file), in the file's order: the
/// entry it reads as by the rules of [`Settings`], or `None` for one the launcher skips.
/// Fails, naming the key, when `home` is not a table or `widgets` is not an array: a file whose
/// Home the launcher cannot change.
pub(crate) fn home_elements(file: &toml::Table) -> Result<Vec<Option<HomeEntry>>, &'static str> {
    let home = match file.get("home") {
        None => return Ok(Vec::new()),
        Some(toml::Value::Table(home)) => home,
        Some(_) => return Err("home"),
    };
    match home.get("widgets") {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(list)) => Ok(list.iter().cloned().map(home_entry).collect()),
        Some(_) => Err("home.widgets"),
    }
}

/// `text` as a TOML table; when it is not TOML, the error in one line, with where it is:
/// `invalid TOML at line 3, column 6: key with no value, expected `=``.
fn toml_table(text: &str) -> anyhow::Result<toml::Table> {
    text.parse::<toml::Table>().map_err(|e| {
        let message = e.message().split_whitespace().collect::<Vec<_>>().join(" ");
        match e.span().and_then(|span| toml_place(text, span.start)) {
            Some((line, column)) => {
                anyhow::anyhow!("invalid TOML at line {line}, column {column}: {message}")
            }
            None => anyhow::anyhow!("invalid TOML: {message}"),
        }
    })
}

/// The line and column (in characters, from 1) of byte `at` of `text`; `None` when `at` is
/// past the text or inside a character.
fn toml_place(text: &str, at: usize) -> Option<(usize, usize)> {
    let before = text.get(..at)?;
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    Some((line, column))
}

/// Each `[[home.widgets]]` element of the config file text `text`, in the file's order: the
/// entry it reads as, or `None` for one the launcher skips. Fails when the text is not valid
/// TOML or its `home` is not a table or `home.widgets` not a list.
pub fn home_rows(text: &str) -> anyhow::Result<Vec<Option<HomeEntry>>> {
    let file = toml_table(text)?;
    home_elements(&file)
        .map_err(|key| anyhow::anyhow!("`{key}` has the wrong form to read the Home list"))
}

fn parse_home_entry(mut table: toml::Table) -> Option<HomeEntry> {
    let Some(widget) = table
        .remove("widget")
        .and_then(|v| v.as_str().and_then(WidgetKey::parse))
    else {
        tracing::warn!("ignoring [[home.widgets]] entry: `widget` is not <plugin-id>/<widget-id>");
        return None;
    };
    let size = match table.remove("size") {
        None => None,
        Some(value) => {
            let size = value.as_str().and_then(WidgetSize::parse);
            if size.is_none() {
                tracing::warn!(%widget, "ignoring [[home.widgets]] size: not quarter, single or double");
            }
            size
        }
    };
    let mut pairs = Vec::with_capacity(table.len());
    for (key, value) in table {
        match value {
            toml::Value::String(s) => pairs.push((key, s)),
            toml::Value::Boolean(b) => pairs.push((key, b.to_string())),
            other => {
                let kind = other.type_str();
                tracing::warn!(%widget, key, kind, "ignoring [[home.widgets]] entry: parameter is not a string or a boolean");
                return None;
            }
        }
    }
    Some(HomeEntry {
        widget,
        size,
        params: params(pairs),
    })
}

/// The `[files]` table. Like `[location]`, a value of the wrong type is skipped with a warning
/// (that key keeps its default, the others are read); so is a `[files]` that is not a table.
fn files_settings(raw: Option<toml::Value>) -> FilesSettings {
    let mut settings = FilesSettings::default();
    let table = match raw {
        None => return settings,
        Some(toml::Value::Table(table)) => table,
        Some(other) => {
            let kind = other.type_str();
            tracing::warn!(kind, "ignoring [files]: not a table");
            return settings;
        }
    };
    for (key, value) in table {
        match (key.as_str(), value) {
            ("in_root", toml::Value::Boolean(on)) => settings.in_root = on,
            ("hidden", toml::Value::Boolean(on)) => settings.hidden = on,
            ("gitignore", toml::Value::Boolean(on)) => settings.gitignore = on,
            ("roots", toml::Value::Array(list)) => match strings(list) {
                Some(list) => settings.roots = list,
                None => tracing::warn!("ignoring files.roots: not an array of strings"),
            },
            ("exclude", toml::Value::Array(list)) => match strings(list) {
                Some(list) => settings.exclude = list,
                None => tracing::warn!("ignoring files.exclude: not an array of strings"),
            },
            (key @ ("in_root" | "hidden" | "gitignore" | "roots" | "exclude"), value) => {
                let kind = value.type_str();
                tracing::warn!(key, kind, "ignoring [files] entry: wrong type");
            }
            (key, _) => tracing::warn!(key, "ignoring unknown [files] key"),
        }
    }
    settings
}

fn strings(list: Vec<toml::Value>) -> Option<Vec<String>> {
    list.into_iter()
        .map(|v| match v {
            toml::Value::String(s) => Some(s),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_is_its_line_and_column_in_characters() {
        let text = "a = 1\nключ = 2\n";
        assert_eq!(toml_place(text, 0), Some((1, 1)));
        assert_eq!(toml_place(text, "a = 1\nключ".len()), Some((2, 5)));
    }

    #[test]
    fn a_place_inside_a_character_or_past_the_text_has_none() {
        let text = "ключ = 2";
        assert_eq!(toml_place(text, 1), None);
        assert_eq!(toml_place(text, text.len() + 1), None);
        let error = toml_table("ключ").unwrap_err().to_string();
        assert!(error.starts_with("invalid TOML"), "{error}");
    }
}
