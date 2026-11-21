//! `plugin.toml` parsing and validation.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};
use tomakast_core::{is_hidden_char, is_valid_id};

pub use tomakast_core::widget::{MAX_WIDGETS, WidgetSize};

/// WIT versions this host can run.
pub const SUPPORTED_API_VERSIONS: &[&str] = &["0.1.0"];

/// Ids of built-in modules; a plugin may not take them.
pub const RESERVED_IDS: &[&str] = &["apps", "demo", "plugins"];

const MAX_NAME_CHARS: usize = 64;
const MAX_TITLE_CHARS: usize = 128;
/// A widget's title, which the approval prompt shows.
const MAX_WIDGET_TITLE_CHARS: usize = 64;
pub const MAX_COMMANDS: usize = 64;
pub const MAX_PREFERENCES: usize = 32;
/// Parameters one widget may declare.
pub const MAX_WIDGET_PARAMS: usize = 8;
/// Keys a widget parameter may not have: an entry of the user's Home uses them for itself.
pub const RESERVED_PARAM_KEYS: &[&str] = &["widget", "size"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("invalid plugin.toml: {0}")]
    Syntax(String),
    #[error("invalid id {0:?}: use 1-32 characters from a-z, 0-9 and '-'")]
    BadId(String),
    #[error("id {id:?} must equal the folder name {folder:?}")]
    IdFolderMismatch { id: String, folder: String },
    #[error("id {0:?} is reserved for a built-in module")]
    ReservedId(String),
    #[error("unsupported api version {0:?}")]
    UnsupportedApi(String),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PrefKind {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "bool")]
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    pub id: String,
    pub title: String,
    /// A theme icon name; it follows the id rule.
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferenceDecl {
    pub key: String,
    pub title: String,
    pub kind: PrefKind,
    #[serde(default)]
    pub default: Option<String>,
}

/// A widget the plugin can draw, a tile of the launcher (`[[widgets]]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetEntry {
    /// Follows the id rule of `id`; unique within the manifest.
    pub id: String,
    /// Shown in the approval prompt.
    pub title: String,
    /// The sizes a tile of it can have, without repeats; the first is its default. From
    /// `sizes`, or from the one-size shorthand `size`.
    pub sizes: Vec<WidgetSize>,
    /// The parameters a tile of it takes (`[[widgets.params]]`), at most
    /// [`MAX_WIDGET_PARAMS`].
    pub params: Vec<ParamEntry>,
}

/// A parameter of a widget's tiles (`[[widgets.params]]`): a city, units.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamEntry {
    /// Follows the id rule of `id`, unique within the widget, and is none of
    /// [`RESERVED_PARAM_KEYS`].
    pub key: String,
    /// For the parameter editor.
    pub title: String,
}

/// `[[widgets]]` as written: `size` and `sizes` are checked against each other in
/// [`Manifest::parse`], where the error can name the widget.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWidget {
    id: String,
    title: String,
    #[serde(default, deserialize_with = "one_size")]
    size: Option<WidgetSize>,
    #[serde(default, deserialize_with = "some_sizes")]
    sizes: Option<Vec<WidgetSize>>,
    #[serde(default)]
    params: Vec<ParamEntry>,
}

/// `"quarter"`, `"single"` or `"double"`; anything else does not parse.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Size {
    Quarter,
    Single,
    Double,
}

impl From<Size> for WidgetSize {
    fn from(size: Size) -> WidgetSize {
        match size {
            Size::Quarter => WidgetSize::Quarter,
            Size::Single => WidgetSize::Single,
            Size::Double => WidgetSize::Double,
        }
    }
}

fn one_size<'de, D: Deserializer<'de>>(de: D) -> Result<Option<WidgetSize>, D::Error> {
    Ok(Some(Size::deserialize(de)?.into()))
}

fn some_sizes<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Vec<WidgetSize>>, D::Error> {
    let sizes = Vec::<Size>::deserialize(de)?;
    Ok(Some(sizes.into_iter().map(Into::into).collect()))
}

/// A host allowlist entry: an exact host (`example.com`) or a wildcard over subdomains
/// (`*.example.org`). Stored lowercase; non-ASCII names must be written as punycode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DomainPattern {
    wildcard: bool,
    /// Lowercase host, without the `*.` prefix.
    host: String,
}

impl DomainPattern {
    pub fn parse(text: &str) -> Result<DomainPattern, ManifestError> {
        let bad = |why: &str| ManifestError::Invalid(format!("bad http domain {text:?}: {why}"));
        let lower = text.trim().to_ascii_lowercase();
        let (wildcard, host) = match lower.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, lower.as_str()),
        };
        if !host.is_ascii() {
            return Err(bad("write international names as punycode (xn--...)"));
        }
        if host.is_empty() || host.len() > 253 {
            return Err(bad("empty or too long"));
        }
        for label in host.split('.') {
            let ok = !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !ok {
                return Err(bad(
                    "only host names (letters, digits, '-', dots) are allowed",
                ));
            }
        }
        if wildcard && !host.contains('.') {
            return Err(bad(
                "a wildcard needs at least two labels after `*.`, for example `*.example.org`",
            ));
        }
        Ok(DomainPattern {
            wildcard,
            host: host.to_owned(),
        })
    }

    /// Does this pattern allow `host`? `*.example.org` matches `a.example.org` and
    /// `a.b.example.org`, never `example.org` or `evil-example.org`.
    pub fn matches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        let host = host.strip_suffix('.').unwrap_or(&host);
        if host.split('.').any(str::is_empty) {
            return false;
        }
        if self.wildcard {
            host.len() > self.host.len() + 1
                && host.ends_with(&self.host)
                && host.as_bytes()[host.len() - self.host.len() - 1] == b'.'
        } else {
            host == self.host
        }
    }

    /// Does this pattern allow the IPv4 address `ip`? Only an exact entry written as that
    /// address does: a wildcard never matches an address, and a name is not resolved.
    pub fn matches_ip(&self, ip: std::net::Ipv4Addr) -> bool {
        !self.wildcard && self.host.parse::<std::net::Ipv4Addr>() == Ok(ip)
    }

    /// Is everything this pattern allows also allowed by `other`?
    fn covered_by(&self, other: &DomainPattern) -> bool {
        if self.wildcard {
            // Every host under `*.self` must be under `*.other`.
            other.wildcard
                && (self.host == other.host || self.host.ends_with(&format!(".{}", other.host)))
        } else {
            other.matches(&self.host) || (!other.wildcard && other.host == self.host)
        }
    }
}

impl std::fmt::Display for DomainPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.wildcard {
            write!(f, "*.")?;
        }
        write!(f, "{}", self.host)
    }
}

impl From<DomainPattern> for String {
    fn from(p: DomainPattern) -> String {
        p.to_string()
    }
}

impl TryFrom<String> for DomainPattern {
    type Error = ManifestError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        DomainPattern::parse(&s)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Permissions {
    pub storage: bool,
    pub clipboard: bool,
    pub open: bool,
    pub timers: bool,
    pub index: bool,
    /// The plugin's `search` export is called with every query typed in the launcher search
    /// (Ruling AJ; the top-level `dynamic` key of earlier drafts).
    pub search: bool,
    /// The `location` import: the computer's approximate location.
    pub location: bool,
    pub http: Vec<DomainPattern>,
}

impl Permissions {
    /// True when `self` asks for nothing that `other` does not already grant.
    pub fn is_subset_of(&self, other: &Permissions) -> bool {
        (!self.storage || other.storage)
            && (!self.clipboard || other.clipboard)
            && (!self.open || other.open)
            && (!self.timers || other.timers)
            && (!self.index || other.index)
            && (!self.search || other.search)
            && (!self.location || other.location)
            && self
                .http
                .iter()
                .all(|p| other.http.iter().any(|o| p.covered_by(o)))
    }

    /// One plain-English line per permission, shown in the approval prompt.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.storage {
            out.push("Store data on this computer (up to 1 MiB)".to_owned());
        }
        if self.clipboard {
            out.push("Copy text to the clipboard".to_owned());
        }
        if self.open {
            out.push("Open links and files".to_owned());
        }
        if self.timers {
            out.push("Run timers in the background".to_owned());
        }
        if self.index {
            out.push("Add items to search".to_owned());
        }
        if self.search {
            out.push("See what you type in the launcher search".to_owned());
        }
        if self.location {
            out.push("Know your approximate location (within about 10 km)".to_owned());
        }
        if !self.http.is_empty() {
            let hosts: Vec<String> = self.http.iter().map(DomainPattern::to_string).collect();
            out.push(format!("Make web requests to {}", hosts.join(", ")));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    name: String,
    version: String,
    api: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    commands: Vec<CommandDecl>,
    #[serde(default)]
    preferences: Vec<PreferenceDecl>,
    #[serde(default)]
    widgets: Vec<RawWidget>,
    #[serde(default)]
    permissions: Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    pub author: Option<String>,
    pub commands: Vec<CommandDecl>,
    pub preferences: Vec<PreferenceDecl>,
    pub widgets: Vec<WidgetEntry>,
    pub permissions: Permissions,
}

fn valid_ident(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn check_text(what: &str, s: &str, max_chars: usize) -> Result<(), ManifestError> {
    if s.trim().is_empty() || s.chars().count() > max_chars || s.chars().any(char::is_control) {
        return Err(ManifestError::Invalid(format!(
            "{what} must be 1-{max_chars} characters without control characters"
        )));
    }
    Ok(())
}

/// [`check_text`] for text the approval prompt shows as the plugin's identity (name, author):
/// it also refuses Unicode format characters (bidi overrides, zero-width characters) and the
/// line and paragraph separators U+2028/U+2029, which could disguise or reshape the prompt.
fn check_display_text(what: &str, s: &str, max_chars: usize) -> Result<(), ManifestError> {
    check_text(what, s, max_chars)?;
    if s.chars()
        .any(|c| is_hidden_char(c) || c == '\u{2028}' || c == '\u{2029}')
    {
        return Err(ManifestError::Invalid(format!(
            "{what} must not contain invisible formatting characters or line separators"
        )));
    }
    Ok(())
}

impl Manifest {
    pub fn parse(text: &str, folder_name: &str) -> Result<Manifest, ManifestError> {
        let raw: RawManifest =
            toml::from_str(text).map_err(|e| ManifestError::Syntax(e.to_string()))?;
        if !valid_ident(&raw.id) {
            return Err(ManifestError::BadId(raw.id));
        }
        if raw.id != folder_name {
            return Err(ManifestError::IdFolderMismatch {
                id: raw.id,
                folder: folder_name.to_owned(),
            });
        }
        if RESERVED_IDS.contains(&raw.id.as_str()) {
            return Err(ManifestError::ReservedId(raw.id));
        }
        if !SUPPORTED_API_VERSIONS.contains(&raw.api.as_str()) {
            return Err(ManifestError::UnsupportedApi(raw.api));
        }
        check_display_text("name", &raw.name, MAX_NAME_CHARS)?;
        check_text("version", &raw.version, 32)?;
        if let Some(author) = &raw.author {
            check_display_text("author", author, MAX_NAME_CHARS)?;
        }
        if raw.commands.len() > MAX_COMMANDS {
            return Err(ManifestError::Invalid(format!(
                "too many commands (at most {MAX_COMMANDS})"
            )));
        }
        if raw.preferences.len() > MAX_PREFERENCES {
            return Err(ManifestError::Invalid(format!(
                "too many preferences (at most {MAX_PREFERENCES})"
            )));
        }
        let mut seen = HashSet::new();
        for c in &raw.commands {
            if !valid_ident(&c.id) {
                return Err(ManifestError::BadId(c.id.clone()));
            }
            if !seen.insert(c.id.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate command id {:?}",
                    c.id
                )));
            }
            check_text("command title", &c.title, MAX_TITLE_CHARS)?;
            if let Some(icon) = &c.icon
                && (icon.is_empty() || !is_valid_id(icon))
            {
                return Err(ManifestError::Invalid(format!(
                    "command icon of {:?} must be 1-256 bytes without control or format characters",
                    c.id
                )));
            }
        }
        let mut seen = HashSet::new();
        for p in &raw.preferences {
            if !valid_ident(&p.key) {
                return Err(ManifestError::Invalid(format!(
                    "invalid preference key {:?}: use 1-32 characters from a-z, 0-9 and '-'",
                    p.key
                )));
            }
            if !seen.insert(p.key.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate preference key {:?}",
                    p.key
                )));
            }
            check_text("preference title", &p.title, MAX_TITLE_CHARS)?;
        }
        if raw.widgets.len() > MAX_WIDGETS {
            return Err(ManifestError::Invalid(format!(
                "too many widgets (at most {MAX_WIDGETS})"
            )));
        }
        let mut seen = HashSet::new();
        for w in &raw.widgets {
            if !valid_ident(&w.id) {
                return Err(ManifestError::Invalid(format!(
                    "invalid widget id {:?}: use 1-32 characters from a-z, 0-9 and '-'",
                    w.id
                )));
            }
            if !seen.insert(w.id.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate widget id {:?}",
                    w.id
                )));
            }
            check_display_text("widget title", &w.title, MAX_WIDGET_TITLE_CHARS)?;
        }
        let widgets = raw
            .widgets
            .into_iter()
            .map(widget_entry)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Manifest {
            id: raw.id,
            name: raw.name,
            version: raw.version,
            api: raw.api,
            author: raw.author,
            commands: raw.commands,
            preferences: raw.preferences,
            widgets,
            permissions: raw.permissions,
        })
    }
}

/// `raw` as a [`WidgetEntry`], its id and title already checked: one or more sizes, from
/// either `sizes` or `size`, without repeats; at most [`MAX_WIDGET_PARAMS`] params, each key
/// by the id rule, once, and none of [`RESERVED_PARAM_KEYS`], each with a title. The errors
/// name the widget.
fn widget_entry(raw: RawWidget) -> Result<WidgetEntry, ManifestError> {
    let RawWidget {
        id,
        title,
        size,
        sizes,
        params,
    } = raw;
    let bad = |why: String| ManifestError::Invalid(format!("widget {id:?}: {why}"));
    let sizes = match (size, sizes) {
        (Some(_), Some(_)) => {
            return Err(bad(
                "has both `size` and `sizes`; use `sizes` (the first is the default)".into(),
            ));
        }
        (Some(size), None) => vec![size],
        (None, Some(sizes)) => sizes,
        (None, None) => {
            return Err(bad(
                "needs `sizes` (\"quarter\", \"single\", \"double\"; the first is the default)"
                    .into(),
            ));
        }
    };
    if sizes.is_empty() {
        return Err(bad("`sizes` is empty".into()));
    }
    for (at, size) in sizes.iter().enumerate() {
        if sizes[..at].contains(size) {
            return Err(bad(format!("`sizes` names {:?} twice", size.as_str())));
        }
    }
    if params.len() > MAX_WIDGET_PARAMS {
        return Err(bad(format!(
            "too many params (at most {MAX_WIDGET_PARAMS})"
        )));
    }
    for (at, param) in params.iter().enumerate() {
        if !valid_ident(&param.key) {
            return Err(bad(format!(
                "invalid param key {:?}: use 1-32 characters from a-z, 0-9 and '-'",
                param.key
            )));
        }
        if RESERVED_PARAM_KEYS.contains(&param.key.as_str()) {
            return Err(bad(format!(
                "param key {:?} is reserved (a Home entry uses `widget` and `size` itself)",
                param.key
            )));
        }
        if params[..at].iter().any(|p| p.key == param.key) {
            return Err(bad(format!("duplicate param key {:?}", param.key)));
        }
        check_display_text("param title", &param.title, MAX_WIDGET_TITLE_CHARS).map_err(
            |e| match e {
                ManifestError::Invalid(why) => bad(why),
                other => other,
            },
        )?;
    }
    Ok(WidgetEntry {
        id,
        title,
        sizes,
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
id = "kitchen-sink"
name = "Kitchen Sink"
version = "0.1.0"
api = "0.1.0"
author = "tomakast"

[[commands]]
id = "open"
title = "Kitchen Sink"

[[commands]]
id = "ping"
title = "Ping"
icon = "bell"

[[preferences]]
key = "greeting"
title = "Greeting"
kind = "text"
default = "hello"

[[preferences]]
key = "loud"
title = "Loud"
kind = "bool"

[permissions]
storage = true
clipboard = true
open = true
timers = true
index = true
search = true
location = true
http = ["example.com", "*.example.org"]
"#;

    fn with(id: &str, extra: &str) -> String {
        format!("id = \"{id}\"\nname = \"X\"\nversion = \"1\"\napi = \"0.1.0\"\n{extra}")
    }

    #[test]
    fn manifest_parses_full_example() {
        let m = Manifest::parse(FULL, "kitchen-sink").unwrap();
        assert_eq!(m.id, "kitchen-sink");
        assert_eq!(m.name, "Kitchen Sink");
        assert_eq!(m.author.as_deref(), Some("tomakast"));
        assert_eq!(m.commands.len(), 2);
        assert_eq!(m.commands[1].icon.as_deref(), Some("bell"));
        assert_eq!(m.preferences[0].kind, PrefKind::Text);
        assert_eq!(m.preferences[0].default.as_deref(), Some("hello"));
        assert_eq!(m.preferences[1].kind, PrefKind::Bool);
        let p = &m.permissions;
        assert!(p.storage && p.clipboard && p.open && p.timers && p.index && p.search);
        assert_eq!(p.http.len(), 2);
        assert!(p.http[1].matches("a.example.org"));
    }

    #[test]
    fn manifest_rejects_id_mismatch_with_folder() {
        let err = Manifest::parse(&with("foo", ""), "bar").unwrap_err();
        assert!(matches!(err, ManifestError::IdFolderMismatch { .. }));
    }

    #[test]
    fn manifest_rejects_bad_id_chars() {
        for bad in [
            "Foo",
            "foo_bar",
            "foo bar",
            "",
            "../x",
            "é",
            &"a".repeat(33),
        ] {
            let err = Manifest::parse(&with(bad, ""), bad).unwrap_err();
            assert!(matches!(err, ManifestError::BadId(_)), "{bad:?}: {err:?}");
        }
        let cmd = with("ok", "[[commands]]\nid = \"A b\"\ntitle = \"t\"\n");
        assert!(matches!(
            Manifest::parse(&cmd, "ok").unwrap_err(),
            ManifestError::BadId(_)
        ));
    }

    #[test]
    fn manifest_rejects_reserved_ids() {
        for id in RESERVED_IDS {
            let err = Manifest::parse(&with(id, ""), id).unwrap_err();
            assert_eq!(err, ManifestError::ReservedId((*id).to_owned()));
        }
    }

    #[test]
    fn manifest_rejects_unknown_api_version() {
        let text = FULL.replace("api = \"0.1.0\"", "api = \"9.9.9\"");
        assert!(matches!(
            Manifest::parse(&text, "kitchen-sink").unwrap_err(),
            ManifestError::UnsupportedApi(_)
        ));
    }

    #[test]
    fn manifest_rejects_unknown_permission_key() {
        let text = format!("{}\nfilesystem = true\n", FULL);
        assert!(matches!(
            Manifest::parse(&text, "kitchen-sink").unwrap_err(),
            ManifestError::Syntax(_)
        ));
    }

    #[test]
    fn manifest_rejects_unknown_top_level_key_and_long_name() {
        let text = with("ok", "surprise = 1\n");
        assert!(Manifest::parse(&text, "ok").is_err());
        let long =
            with("ok", "").replace("name = \"X\"", &format!("name = \"{}\"", "n".repeat(65)));
        assert!(matches!(
            Manifest::parse(&long, "ok").unwrap_err(),
            ManifestError::Invalid(_)
        ));
    }

    #[test]
    fn manifest_name_and_author_reject_format_characters_and_line_separators() {
        // The characters go into the TOML strings raw (TOML has no `\u{..}` escape).
        let named =
            |name: &str| with("ok", "").replace("name = \"X\"", &format!("name = \"{name}\""));
        let authored = |author: &str| format!("{}author = \"{author}\"\n", with("ok", ""));
        for bad in [
            "Evil\u{202e}gnp.exe", // right-to-left override
            "Zero\u{200b}width",
            "Joiner\u{200d}x",
            "Tag\u{e0041}",
            "Line\u{2028}break",
            "Para\u{2029}break",
        ] {
            for text in [named(bad), authored(bad)] {
                assert!(
                    matches!(Manifest::parse(&text, "ok"), Err(ManifestError::Invalid(_))),
                    "{bad:?} in {text}"
                );
            }
        }
        // Ordinary non-Latin text, combining marks and emoji without joiners stay valid.
        for ok in ["Файлы", "Cafe\u{301}", "Weather \u{2600}\u{fe0f}"] {
            assert!(Manifest::parse(&named(ok), "ok").is_ok(), "{ok:?}");
            assert!(Manifest::parse(&authored(ok), "ok").is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn manifest_has_no_dynamic_or_mode_key() {
        // Ruling AJ: `search` is a permission; Ruling AP: commands have no `mode`.
        let dynamic = with("ok", "dynamic = true\n");
        assert!(matches!(
            Manifest::parse(&dynamic, "ok"),
            Err(ManifestError::Syntax(_))
        ));
        let mode = with(
            "ok",
            "[[commands]]\nid = \"a\"\ntitle = \"t\"\nmode = \"view\"\n",
        );
        assert!(matches!(
            Manifest::parse(&mode, "ok"),
            Err(ManifestError::Syntax(_))
        ));
        let search = with("ok", "[permissions]\nsearch = true\n");
        assert!(Manifest::parse(&search, "ok").unwrap().permissions.search);
        assert!(
            !Manifest::parse(&with("ok", ""), "ok")
                .unwrap()
                .permissions
                .search
        );
    }

    #[test]
    fn manifest_command_icon_follows_the_id_rule() {
        let icon = |name: &str| {
            with(
                "ok",
                &format!("[[commands]]\nid = \"a\"\ntitle = \"t\"\nicon = \"{name}\"\n"),
            )
        };
        // An icon name longer than the old 32-character text limit but within 256 bytes.
        let long = "x".repeat(200);
        for ok in ["bell", "face-smile", long.as_str()] {
            assert!(Manifest::parse(&icon(ok), "ok").is_ok(), "{ok:?}");
        }
        let too_long = "x".repeat(257);
        for bad in ["", "zero\u{200b}width", "rtl\u{202e}x", too_long.as_str()] {
            assert!(
                matches!(
                    Manifest::parse(&icon(bad), "ok"),
                    Err(ManifestError::Invalid(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn manifest_rejects_duplicate_commands() {
        let c = "[[commands]]\nid = \"a\"\ntitle = \"t\"\n";
        let text = with("ok", &format!("{c}{c}"));
        assert!(matches!(
            Manifest::parse(&text, "ok").unwrap_err(),
            ManifestError::Invalid(_)
        ));
    }

    #[test]
    fn manifest_caps_commands_and_preferences() {
        let cmds = |n: usize| {
            (0..n)
                .map(|i| format!("[[commands]]\nid = \"c{i}\"\ntitle = \"t\"\n"))
                .collect::<String>()
        };
        let prefs = |n: usize| {
            (0..n)
                .map(|i| {
                    format!("[[preferences]]\nkey = \"p{i}\"\ntitle = \"t\"\nkind = \"bool\"\n")
                })
                .collect::<String>()
        };
        assert!(Manifest::parse(&with("ok", &cmds(MAX_COMMANDS)), "ok").is_ok());
        assert!(matches!(
            Manifest::parse(&with("ok", &cmds(MAX_COMMANDS + 1)), "ok"),
            Err(ManifestError::Invalid(_))
        ));
        assert!(Manifest::parse(&with("ok", &prefs(MAX_PREFERENCES)), "ok").is_ok());
        assert!(matches!(
            Manifest::parse(&with("ok", &prefs(MAX_PREFERENCES + 1)), "ok"),
            Err(ManifestError::Invalid(_))
        ));
    }

    fn widgets(entries: &[(&str, &str, &str)]) -> String {
        entries
            .iter()
            .map(|(id, title, size)| {
                format!("[[widgets]]\nid = \"{id}\"\ntitle = \"{title}\"\nsize = \"{size}\"\n")
            })
            .collect()
    }

    #[test]
    fn manifest_parses_widgets() {
        let text = with(
            "ok",
            &widgets(&[
                ("clock", "Clock", "single"),
                ("world-map", "World map", "double"),
                ("weather", "Weather", "quarter"),
            ]),
        );
        let m = Manifest::parse(&text, "ok").unwrap();
        assert_eq!(
            m.widgets,
            vec![
                WidgetEntry {
                    id: "clock".into(),
                    title: "Clock".into(),
                    sizes: vec![WidgetSize::Single],
                    params: vec![],
                },
                WidgetEntry {
                    id: "world-map".into(),
                    title: "World map".into(),
                    sizes: vec![WidgetSize::Double],
                    params: vec![],
                },
                WidgetEntry {
                    id: "weather".into(),
                    title: "Weather".into(),
                    sizes: vec![WidgetSize::Quarter],
                    params: vec![],
                },
            ]
        );
        assert!(
            Manifest::parse(&with("ok", ""), "ok")
                .unwrap()
                .widgets
                .is_empty()
        );
        let most: Vec<(String, &str, &str)> = (0..MAX_WIDGETS)
            .map(|i| (format!("w{i}"), "W", "single"))
            .collect();
        let most: Vec<(&str, &str, &str)> =
            most.iter().map(|(i, t, s)| (i.as_str(), *t, *s)).collect();
        assert_eq!(
            Manifest::parse(&with("ok", &widgets(&most)), "ok")
                .unwrap()
                .widgets
                .len(),
            MAX_WIDGETS
        );
    }

    #[test]
    fn manifest_rejects_bad_widgets() {
        let parse =
            |entries: &[(&str, &str, &str)]| Manifest::parse(&with("ok", &widgets(entries)), "ok");
        let long = "t".repeat(65);
        for bad in [
            vec![("A", "W", "single")],
            vec![("w", "W", "single"), ("w", "Again", "double")],
            vec![("w", "", "single")],
            vec![("w", long.as_str(), "single")],
            vec![("w", "Evil\u{202e}gnp", "single")],
        ] {
            assert!(
                matches!(parse(&bad), Err(ManifestError::Invalid(_))),
                "{bad:?}: {:?}",
                parse(&bad)
            );
        }
        for size in ["triple", "Quarter", "half"] {
            assert!(
                matches!(parse(&[("w", "W", size)]), Err(ManifestError::Syntax(_))),
                "{size}: {:?}",
                parse(&[("w", "W", size)])
            );
        }
        let many: Vec<String> = (0..=MAX_WIDGETS).map(|i| format!("w{i}")).collect();
        let many: Vec<(&str, &str, &str)> =
            many.iter().map(|id| (id.as_str(), "W", "single")).collect();
        assert!(matches!(parse(&many), Err(ManifestError::Invalid(_))));
        // Each key is required; a size, as `size` or `sizes`.
        for missing in ["id = \"w\"\n", "title = \"W\"\n"] {
            let text = with("ok", &widgets(&[("w", "W", "single")]).replace(missing, ""));
            assert!(
                matches!(Manifest::parse(&text, "ok"), Err(ManifestError::Syntax(_))),
                "without {missing:?}"
            );
        }
        let text = with(
            "ok",
            &widgets(&[("w", "W", "single")]).replace("size = \"single\"\n", ""),
        );
        let err = Manifest::parse(&text, "ok").unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("\"w\"")),
            "without a size: {err:?}"
        );
    }

    /// One `[[widgets]]` entry `w` with the lines `body`.
    fn widget_with(body: &str) -> Result<Manifest, ManifestError> {
        Manifest::parse(
            &with(
                "ok",
                &format!("[[widgets]]\nid = \"w\"\ntitle = \"W\"\n{body}"),
            ),
            "ok",
        )
    }

    /// `n` `[[widgets.params]]` of `w`, keys `p0`, `p1`…
    fn params(n: usize) -> String {
        (0..n)
            .map(|i| format!("[[widgets.params]]\nkey = \"p{i}\"\ntitle = \"P{i}\"\n"))
            .collect()
    }

    /// `err` is an invalid manifest whose message names the widget `w`.
    fn names_w(err: Result<Manifest, ManifestError>) {
        assert!(
            matches!(&err, Err(ManifestError::Invalid(m)) if m.contains("\"w\"")),
            "{err:?}"
        );
    }

    #[test]
    fn sizes_list_first_is_default() {
        let m = widget_with(
            "sizes = [\"single\", \"quarter\"]\n[[widgets.params]]\nkey = \"city\"\ntitle = \"City\"\n",
        )
        .unwrap();
        assert_eq!(
            m.widgets[0].sizes,
            [WidgetSize::Single, WidgetSize::Quarter]
        );
        assert_eq!(
            m.widgets[0].params,
            [ParamEntry {
                key: "city".into(),
                title: "City".into(),
            }]
        );
        let m = widget_with("sizes = [\"double\", \"quarter\", \"single\"]\n").unwrap();
        assert_eq!(m.widgets[0].sizes[0], WidgetSize::Double);
        // None, a repeat, or an unknown size.
        names_w(widget_with("sizes = []\n"));
        names_w(widget_with("sizes = [\"single\", \"single\"]\n"));
        assert!(matches!(
            widget_with("sizes = [\"single\", \"half\"]\n"),
            Err(ManifestError::Syntax(_))
        ));
    }

    #[test]
    fn size_shorthand_still_parses() {
        let m = widget_with("size = \"double\"\n").unwrap();
        assert_eq!(m.widgets[0].sizes, [WidgetSize::Double]);
        assert!(m.widgets[0].params.is_empty());
    }

    #[test]
    fn size_and_sizes_together_are_rejected() {
        names_w(widget_with("size = \"single\"\nsizes = [\"single\"]\n"));
    }

    #[test]
    fn params_declared_with_reserved_key_are_rejected() {
        for key in ["widget", "size"] {
            names_w(widget_with(&format!(
                "size = \"single\"\n[[widgets.params]]\nkey = \"{key}\"\ntitle = \"T\"\n"
            )));
        }
        // Not the id rule, twice, or without a title.
        for bad in ["City", "", "a b"] {
            names_w(widget_with(&format!(
                "size = \"single\"\n[[widgets.params]]\nkey = \"{bad}\"\ntitle = \"T\"\n"
            )));
        }
        names_w(widget_with(&format!(
            "size = \"single\"\n{}{}",
            params(1),
            params(1)
        )));
        names_w(widget_with(
            "size = \"single\"\n[[widgets.params]]\nkey = \"city\"\ntitle = \"\"\n",
        ));
        assert!(matches!(
            widget_with("size = \"single\"\n[[widgets.params]]\nkey = \"city\"\n"),
            Err(ManifestError::Syntax(_))
        ));
    }

    #[test]
    fn more_than_eight_params_are_rejected() {
        let m = widget_with(&format!("size = \"single\"\n{}", params(MAX_WIDGET_PARAMS))).unwrap();
        assert_eq!(m.widgets[0].params.len(), 8);
        names_w(widget_with(&format!(
            "size = \"single\"\n{}",
            params(MAX_WIDGET_PARAMS + 1)
        )));
    }

    #[test]
    fn domain_pattern_matches() {
        let wild = DomainPattern::parse("*.example.org").unwrap();
        assert!(wild.matches("a.example.org"));
        assert!(wild.matches("a.b.example.org"));
        assert!(wild.matches("A.EXAMPLE.ORG"));
        assert!(!wild.matches("example.org"));
        assert!(!wild.matches("evil-example.org"));
        assert!(!wild.matches("example.org.evil.com"));
        assert!(!wild.matches(".example.org"));
        assert!(!wild.matches("a..example.org"));
        assert!(
            !DomainPattern::parse("example.com")
                .unwrap()
                .matches("..example.com")
        );
        let exact = DomainPattern::parse("example.com").unwrap();
        assert!(exact.matches("example.com"));
        assert!(exact.matches("EXAMPLE.com"));
        assert!(!exact.matches("a.example.com"));
        assert!(!exact.matches("evil-example.com"));
        // IDNs are compared as lowercase punycode; raw Unicode is refused.
        let idn = DomainPattern::parse("XN--E1AFMKFD.xn--p1ai").unwrap();
        assert!(idn.matches("xn--e1afmkfd.xn--p1ai"));
        assert!(DomainPattern::parse("пример.рф").is_err());
        for bad in [
            "",
            "*.com",
            "*",
            "a b.com",
            "http://a.com",
            "a.com/x",
            "a.com:80",
            "-a.com",
            "a..com",
            "*.*.a.com",
        ] {
            assert!(DomainPattern::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn permissions_subset() {
        let pat = |s: &str| DomainPattern::parse(s).unwrap();
        let base = Permissions {
            storage: true,
            http: vec![pat("example.com"), pat("*.example.org")],
            ..Permissions::default()
        };
        assert!(Permissions::default().is_subset_of(&base));
        assert!(base.is_subset_of(&base));
        let more = Permissions {
            clipboard: true,
            ..base.clone()
        };
        assert!(!more.is_subset_of(&base));
        // Seeing what is typed is a permission like any other (Ruling AJ).
        let sees = Permissions {
            search: true,
            ..base.clone()
        };
        assert!(!sees.is_subset_of(&base));
        assert!(base.is_subset_of(&sees));
        // An update that starts asking for the location is reviewed again.
        let locates = Permissions {
            location: true,
            ..base.clone()
        };
        assert!(!locates.is_subset_of(&base));
        assert!(base.is_subset_of(&locates));
        assert!(base.is_subset_of(&more));
        let new_host = Permissions {
            http: vec![pat("other.com")],
            ..Permissions::default()
        };
        assert!(!new_host.is_subset_of(&base));
        let sub_host = Permissions {
            http: vec![pat("a.example.org"), pat("*.b.example.org")],
            ..Permissions::default()
        };
        assert!(sub_host.is_subset_of(&base));
        // A wildcard is not covered by an exact host, nor the bare domain by its wildcard.
        let wide = Permissions {
            http: vec![pat("*.example.com")],
            ..Permissions::default()
        };
        assert!(!wide.is_subset_of(&base));
        let bare = Permissions {
            http: vec![pat("example.org")],
            ..Permissions::default()
        };
        assert!(!bare.is_subset_of(&base));
    }

    #[test]
    fn permissions_describe_is_plain_english() {
        let m = Manifest::parse(FULL, "kitchen-sink").unwrap();
        assert_eq!(
            m.permissions.describe(),
            vec![
                "Store data on this computer (up to 1 MiB)",
                "Copy text to the clipboard",
                "Open links and files",
                "Run timers in the background",
                "Add items to search",
                "See what you type in the launcher search",
                "Know your approximate location (within about 10 km)",
                "Make web requests to example.com, *.example.org",
            ]
        );
        assert!(Permissions::default().describe().is_empty());
    }
}
