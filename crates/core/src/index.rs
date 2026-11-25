//! Item model and the in-memory index shared by all providers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::matcher::ItemKeys;
use crate::script::Scripts;
use crate::settings::{Hotkey, HotkeyError, Modifiers};

/// Longest valid item id, in bytes.
const MAX_ID_LEN: usize = 256;

/// Stable item identifier of the form `<provider-id>/<item-id>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(String);

impl ItemId {
    pub fn new(provider: &str, local: &str) -> ItemId {
        ItemId(format!("{provider}/{local}"))
    }

    /// Parses a full identifier; returns `None` when there is no `/`.
    pub fn parse(s: &str) -> Option<ItemId> {
        s.contains('/').then(|| ItemId(s.to_string()))
    }

    pub fn provider(&self) -> &str {
        self.0.split_once('/').map_or(self.0.as_str(), |(p, _)| p)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What the item is called when no title is known (it is not indexed now): its id
    /// without its provider, an application's also without `.desktop`
    /// (`apps/org.foo.desktop` is "org.foo"). An id that is nothing but its provider stays
    /// whole.
    pub fn fallback_title(&self) -> String {
        let local = self
            .0
            .split_once('/')
            .map_or(self.as_str(), |(_, local)| local);
        let local = match self.provider() {
            "apps" => local.strip_suffix(".desktop").unwrap_or(local),
            _ => local,
        };
        if local.is_empty() {
            self.0.clone()
        } else {
            local.to_string()
        }
    }

    /// Whether an id coming from a module may be indexed: it contains `/`,
    /// has no control character and no Unicode format character (bidi
    /// overrides, zero-width characters), and is at most 256 bytes.
    pub fn is_valid(&self) -> bool {
        self.0.contains('/') && is_valid_id(&self.0)
    }
}

/// Whether a module-supplied id (of an item, action, view, row, tile or field) is at most
/// 256 bytes and has no control character and no Unicode format character.
pub fn is_valid_id(id: &str) -> bool {
    id.len() <= MAX_ID_LEN && !id.chars().any(is_hidden)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Application,
    Command,
    File,
    Folder,
    /// A command that carries the query on to a fuller search ("Search Files for …"),
    /// from a dynamic source. The root list keeps it only when every other hit is a file
    /// or folder; it shows as a command.
    Fallback,
}

impl ItemKind {
    /// A file or a folder: ranked as a match when a dynamic source answers.
    pub fn is_path(self) -> bool {
        matches!(self, ItemKind::File | ItemKind::Folder)
    }
}

/// Unicode format characters (general category Cf, Unicode 16): bidi
/// controls, zero-width characters, tags. They change how text around them
/// looks without being seen, so ids may not contain them, and emoji only
/// the ones emoji sequences are built from (see [`is_emoji_part`]).
const FORMAT_CHARS: &[(char, char)] = &[
    ('\u{00ad}', '\u{00ad}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061c}', '\u{061c}'),
    ('\u{06dd}', '\u{06dd}'),
    ('\u{070f}', '\u{070f}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08e2}', '\u{08e2}'),
    ('\u{180e}', '\u{180e}'),
    ('\u{200b}', '\u{200f}'),
    ('\u{202a}', '\u{202e}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206f}'),
    ('\u{feff}', '\u{feff}'),
    ('\u{fff9}', '\u{fffb}'),
    ('\u{110bd}', '\u{110bd}'),
    ('\u{110cd}', '\u{110cd}'),
    ('\u{13430}', '\u{1343f}'),
    ('\u{1bca0}', '\u{1bca3}'),
    ('\u{1d173}', '\u{1d17a}'),
    ('\u{e0001}', '\u{e0001}'),
    ('\u{e0020}', '\u{e007f}'),
];

/// Whether `c` is a control or a Unicode format character (category Cf):
/// invisible, or able to disguise the text around it (bidi overrides,
/// zero-width characters, tags).
pub fn is_hidden_char(c: char) -> bool {
    is_hidden(c)
}

/// A control or format character: invisible, or able to disguise the text
/// around it.
fn is_hidden(c: char) -> bool {
    c.is_control() || FORMAT_CHARS.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// The format characters an emoji may contain: the zero-width joiner of ZWJ
/// sequences (👨‍💻) and the tag characters of subdivision flags.
fn is_emoji_part(c: char) -> bool {
    c == '\u{200d}' || ('\u{e0020}'..='\u{e007f}').contains(&c)
}

/// Longest valid [`Icon::Emoji`], in bytes.
const MAX_EMOJI_LEN: usize = 32;

/// An icon or a tile image. WIT `icon`: `named`, `plugin-file` (a plugin's
/// relative path, which the host resolves to `File`), `emoji`, `color`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    /// A theme icon name.
    Named(String),
    /// An image file; trusted built-in modules give any path.
    File(PathBuf),
    /// A plugin's image file (WIT `plugin-file`): relative to the plugin folder as the plugin
    /// sent it, then the canonical path once the plugin host checked it. The UI opens it only
    /// through its bounded loader (no symbolic link, at most 1 MiB, PNG or JPEG of at most
    /// 1024x1024).
    PluginFile(PathBuf),
    /// An emoji or symbol, at most 32 bytes, drawn as text.
    Emoji(String),
    /// A colour swatch, `#rrggbb`.
    Color(String),
}

impl Icon {
    /// Whether an `Emoji` is at most 32 bytes, has at least one visible
    /// character (neither control nor format), and is free of control and
    /// format characters other than the zero-width joiner and the tag
    /// characters; and whether a `Color` is `#` and six hex digits. `Named` and
    /// `File` are always valid here; the plugin host checks plugin paths.
    pub fn is_valid(&self) -> bool {
        match self {
            Icon::Named(_) | Icon::File(_) | Icon::PluginFile(_) => true,
            Icon::Emoji(s) => {
                s.len() <= MAX_EMOJI_LEN
                    && s.chars().any(|c| !is_hidden(c))
                    && !s.chars().any(|c| is_hidden(c) && !is_emoji_part(c))
            }
            Icon::Color(s) => s
                .strip_prefix('#')
                .is_some_and(|hex| hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub title: String,
    /// Key combination that runs the action on the selected row, in the
    /// [`Hotkey::parse`] syntax (`ctrl+shift+c`).
    pub shortcut: Option<String>,
}

/// Why an [`Action::shortcut`] is refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ShortcutError {
    #[error(transparent)]
    Parse(#[from] HotkeyError),
    #[error("shortcut `{0}` needs ctrl, alt or super")]
    NoModifier(String),
    #[error("shortcut `{0}` uses a key the launcher or text editing needs")]
    Reserved(String),
    #[error("shortcut `{0}` does not name a key")]
    UnknownKey(String),
    #[error("shortcut `{0}` cannot use the leader key")]
    Leader(String),
}

/// Longest key name in an action shortcut (`pagedown`, `f24`), in bytes.
const MAX_KEY_NAME_LEN: usize = 16;

/// A key an action shortcut can name: one visible character (`c`, `1`, `é`), or a short
/// ASCII key name (`f2`, `pageup`). Anything else cannot be a key: a module must not slip
/// control characters or long text into a shortcut.
fn is_key_name(key: &str) -> bool {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => !is_hidden(c) && !c.is_whitespace(),
        _ => {
            key.len() <= MAX_KEY_NAME_LEN
                && key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        }
    }
}

/// Keys an action shortcut may not use with any modifiers: the launcher's
/// navigation and the text field's editing keys. Names as `Hotkey::parse`
/// leaves them (lowercase), with their aliases.
const RESERVED_KEYS: &[&str] = &[
    "tab",
    "backspace",
    "delete",
    "del",
    "home",
    "end",
    "up",
    "down",
    "left",
    "right",
    "arrowup",
    "arrowdown",
    "arrowleft",
    "arrowright",
    "enter",
    "return",
    "escape",
    "esc",
    "space",
];

/// Keys an action shortcut may not use with Ctrl alone: select all, copy,
/// paste, cut, undo, redo, and the action panel (Ctrl+K).
const RESERVED_CTRL_KEYS: &[&str] = &["a", "c", "v", "x", "z", "y", "k"];

impl Action {
    /// The parsed [`shortcut`](Self::shortcut): `None` without one, an error
    /// when it does not parse or is refused. A shortcut needs Ctrl, Alt or
    /// Super (Shift alone is not enough), may not use the leader key or a
    /// navigation or editing key (Tab, Backspace, Delete, Home, End, the
    /// arrows, Enter, Esc, Space) with any modifiers, and may not be Ctrl
    /// alone with A, C, V, X, Z, Y or K: a module must not take keys the user
    /// types or navigates with.
    pub fn hotkey(&self) -> Option<Result<Hotkey, ShortcutError>> {
        self.shortcut.as_deref().map(parse_shortcut)
    }
}

fn parse_shortcut(s: &str) -> Result<Hotkey, ShortcutError> {
    let hotkey = Hotkey::parse(s)?;
    if hotkey.mods.contains(Modifiers::LEADER) {
        return Err(ShortcutError::Leader(s.to_string()));
    }
    if !hotkey
        .mods
        .intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(ShortcutError::NoModifier(s.to_string()));
    }
    let key = hotkey.key.as_str();
    if !is_key_name(key) {
        return Err(ShortcutError::UnknownKey(s.to_string()));
    }
    if RESERVED_KEYS.contains(&key)
        || (hotkey.mods == Modifiers::CTRL && RESERVED_CTRL_KEYS.contains(&key))
    {
        return Err(ShortcutError::Reserved(s.to_string()));
    }
    Ok(hotkey)
}

/// A launchable entry. The first action is the primary one; `actions` is non-empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub title: String,
    pub subtitle: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<Icon>,
    pub kind: ItemKind,
    pub actions: Vec<Action>,
    /// Where the item comes from, shown on its row instead of the kind label.
    /// The plugin host sets it to the plugin's manifest name; WIT `item` has
    /// no such field, so a plugin cannot set it.
    pub source: Option<String>,
}

/// Every indexed item with its match keys, computed once when the item is indexed.
#[derive(Default)]
pub struct Index {
    items: HashMap<ItemId, (Arc<Item>, ItemKeys)>,
    /// The scripts of every item's title and keywords.
    scripts: Scripts,
    /// Counts the changes: what was built from the items is current while it is the same.
    generation: u64,
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    /// Removes every item of `provider` and inserts `items` in their place.
    /// The same items again (in any order) change nothing, the generation neither.
    pub fn replace_provider(&mut self, provider: &str, items: Vec<Item>) {
        if self.holds_exactly(provider, &items) {
            return;
        }
        self.generation += 1;
        self.items.retain(|id, _| id.provider() != provider);
        for item in items {
            let keys = ItemKeys::new(&item);
            self.items.insert(item.id.clone(), (Arc::new(item), keys));
        }
        self.scripts = self
            .items
            .values()
            .fold(Scripts::default(), |acc, (_, keys)| {
                acc.union(keys.scripts())
            });
    }

    /// Whether `provider`'s items are `items`, each once, and no others.
    fn holds_exactly(&self, provider: &str, items: &[Item]) -> bool {
        let held = self
            .items
            .keys()
            .filter(|id| id.provider() == provider)
            .count();
        let ids: std::collections::HashSet<&ItemId> = items.iter().map(|item| &item.id).collect();
        held == items.len()
            && ids.len() == items.len()
            && items.iter().all(|item| {
                item.id.provider() == provider
                    && self
                        .items
                        .get(&item.id)
                        .is_some_and(|(held, _)| **held == *item)
            })
    }

    pub fn get(&self, id: &ItemId) -> Option<&Arc<Item>> {
        self.items.get(id).map(|(item, _)| item)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<Item>> {
        self.items.values().map(|(item, _)| item)
    }

    /// Every item with its match keys.
    pub fn iter_keyed(&self) -> impl Iterator<Item = (&Arc<Item>, &ItemKeys)> {
        self.items.values().map(|(item, keys)| (item, keys))
    }

    /// The scripts present in the index, over all titles and keywords.
    pub fn scripts(&self) -> Scripts {
        self.scripts
    }

    /// A number that changes whenever the items do.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_without_a_title_is_named_by_its_local_id() {
        let name = |id: &str| ItemId::parse(id).unwrap().fallback_title();
        assert_eq!(name("apps/org.kde.konsole.desktop"), "org.kde.konsole");
        assert_eq!(name("weather/refresh"), "refresh");
        assert_eq!(name("files/notes.desktop"), "notes.desktop");
        assert_eq!(name("weather/"), "weather/");
    }

    fn titled(provider: &str, local: &str, title: &str) -> Item {
        Item {
            id: ItemId::new(provider, local),
            title: title.into(),
            subtitle: None,
            keywords: vec![],
            icon: None,
            kind: ItemKind::Application,
            actions: vec![],
            source: None,
        }
    }

    #[test]
    fn the_generation_changes_with_the_items() {
        let mut index = Index::new();
        let before = index.generation();
        assert_eq!(index.generation(), before, "reading changes nothing");
        index.replace_provider("apps", vec![titled("apps", "f", "Firefox")]);
        let after = index.generation();
        assert_ne!(after, before);
        index.replace_provider("apps", vec![]);
        assert_ne!(index.generation(), after);
    }

    /// A provider indexing the same items again (each refresh of the applications) changes
    /// nothing, the generation neither; any difference in one of them does.
    #[test]
    fn the_same_items_again_keep_the_generation() {
        let mut index = Index::new();
        let items = || {
            vec![
                titled("apps", "f", "Firefox"),
                titled("apps", "t", "Terminal"),
            ]
        };
        index.replace_provider("apps", items());
        index.replace_provider("demo", vec![titled("demo", "d", "Demo")]);
        let before = index.generation();
        let mut reversed = items();
        reversed.reverse();
        index.replace_provider("apps", reversed);
        assert_eq!(index.generation(), before);
        index.replace_provider("other", vec![]);
        assert_eq!(index.generation(), before, "an empty provider stays empty");
        let mut renamed = items();
        renamed[1].title = "Konsole".into();
        index.replace_provider("apps", renamed);
        assert_ne!(index.generation(), before);
        assert_eq!(
            index.get(&ItemId::new("apps", "t")).unwrap().title,
            "Konsole"
        );
        let before = index.generation();
        index.replace_provider("apps", vec![titled("apps", "f", "Firefox")]);
        assert_ne!(index.generation(), before, "one item fewer");
    }

    #[test]
    fn scripts_follow_replace_provider() {
        let mut index = Index::new();
        assert_eq!(index.scripts(), Scripts::default());
        index.replace_provider("apps", vec![titled("apps", "f", "Firefox")]);
        index.replace_provider(
            "ru",
            vec![Item {
                keywords: vec!["ფაილები".into()],
                ..titled("ru", "files", "Файлы")
            }],
        );
        assert_eq!(
            index.scripts(),
            Scripts::LATIN
                .union(Scripts::CYRILLIC)
                .union(Scripts::GEORGIAN)
        );

        index.replace_provider("ru", vec![]);

        assert_eq!(index.scripts(), Scripts::LATIN);
    }

    #[test]
    fn iter_keyed_pairs_items_with_their_keys() {
        let mut index = Index::new();
        index.replace_provider("apps", vec![titled("apps", "f", "Файлы")]);
        let keyed: Vec<_> = index.iter_keyed().collect();
        assert_eq!(keyed.len(), 1);
        assert_eq!(keyed[0].0.title, "Файлы");
        assert_eq!(keyed[0].1.title.scripts(), Scripts::CYRILLIC);
    }

    #[test]
    fn item_id_validation() {
        assert!(ItemId::new("apps", "firefox.desktop").is_valid());
        // Not constructible through the public API; modules could only send it over a boundary.
        assert!(!ItemId("nofslash".into()).is_valid());
        assert!(!ItemId::new("a", "b\nc").is_valid());
        assert!(!ItemId::new("a", "b\u{7f}").is_valid());
        let at_limit = ItemId::new("a", &"x".repeat(MAX_ID_LEN - 2));
        assert_eq!(at_limit.as_str().len(), MAX_ID_LEN);
        assert!(at_limit.is_valid());
        assert!(!ItemId::new("a", &"x".repeat(MAX_ID_LEN - 1)).is_valid());
    }

    /// Unicode format characters (category Cf) that can disguise an id.
    const FORMAT_CHARS: [char; 8] = [
        '\u{00ad}',  // soft hyphen
        '\u{200b}',  // zero-width space
        '\u{200d}',  // zero-width joiner
        '\u{202e}',  // right-to-left override
        '\u{2066}',  // left-to-right isolate
        '\u{feff}',  // zero-width no-break space
        '\u{e0041}', // tag latin capital letter a
        '\u{110bd}', // kaithi number sign
    ];

    #[test]
    fn item_id_rejects_format_characters() {
        for c in FORMAT_CHARS {
            assert!(!ItemId::new("a", &format!("b{c}c")).is_valid(), "{c:?}");
        }
        // Not format characters: a combining mark, a variation selector, a non-Latin letter.
        for ok in ["e\u{301}", "\u{2764}\u{fe0f}", "файл"] {
            assert!(ItemId::new("a", ok).is_valid(), "{ok:?}");
        }
    }

    #[test]
    fn emoji_allows_only_joiner_and_tag_format_characters() {
        let emoji_parts = |c: &char| *c == '\u{200d}' || ('\u{e0020}'..='\u{e007f}').contains(c);
        for c in FORMAT_CHARS.iter().filter(|c| !emoji_parts(c)) {
            assert!(!Icon::Emoji(format!("🍎{c}")).is_valid(), "{c:?}");
        }
        assert!(!Icon::Emoji("🍎\u{202e}".into()).is_valid());
        assert!(Icon::Emoji("\u{2764}\u{fe0f}".into()).is_valid());
        // A ZWJ sequence: man, joiner, laptop (11 bytes).
        assert!(Icon::Emoji("👨\u{200d}💻".into()).is_valid());
        // Tag characters, as in subdivision flags (not a real flag).
        assert!(Icon::Emoji("🏴\u{e0067}\u{e007f}".into()).is_valid());
        // The England flag: a subdivision flag, 28 bytes.
        let england = "🏴\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}";
        assert_eq!(england.len(), 28);
        assert!(Icon::Emoji(england.into()).is_valid());
    }

    #[test]
    fn emoji_needs_a_visible_character() {
        for invisible in ["\u{e0041}", "\u{200d}", "\u{200d}\u{e0067}\u{e007f}"] {
            assert!(!Icon::Emoji(invisible.into()).is_valid(), "{invisible:?}");
        }
        assert!(Icon::Emoji("\u{200d}🍎".into()).is_valid());
    }

    #[test]
    fn icon_validation() {
        assert!(Icon::Named("folder".into()).is_valid());
        assert!(Icon::Color("#1a2B3c".into()).is_valid());
        for bad in [
            "1a2b3c",
            "#1a2b3",
            "#1a2b3c4",
            "#1a2b3g",
            "#",
            "",
            "#ｆｆｆｆｆｆ",
        ] {
            assert!(!Icon::Color(bad.into()).is_valid(), "{bad:?}");
        }
        assert!(Icon::Emoji("🍎".into()).is_valid());
        // A family emoji with joiners: 25 bytes, within the cap.
        assert!(Icon::Emoji("👨\u{200d}👩\u{200d}👧\u{200d}👦".into()).is_valid());
        assert!(Icon::Emoji("x".repeat(MAX_EMOJI_LEN)).is_valid());
        assert!(!Icon::Emoji("x".repeat(MAX_EMOJI_LEN + 1)).is_valid());
        // The cap the WIT states.
        assert!(Icon::Emoji("x".repeat(32)).is_valid());
        assert!(!Icon::Emoji("x".repeat(33)).is_valid());
        assert!(!Icon::Emoji(String::new()).is_valid());
        assert!(!Icon::Emoji("a\nb".into()).is_valid());
    }

    #[test]
    fn action_shortcut_uses_hotkey_syntax() {
        let action = |shortcut: Option<&str>| Action {
            id: "copy".into(),
            title: "Copy".into(),
            shortcut: shortcut.map(str::to_string),
        };
        assert_eq!(action(None).hotkey(), None);
        let parsed = action(Some("ctrl+shift+c")).hotkey().unwrap().unwrap();
        assert_eq!(parsed, Hotkey::parse("Ctrl+Shift+C").unwrap());
        assert!(action(Some("ctrl+")).hotkey().unwrap().is_err());
        assert!(action(Some("hyper+c")).hotkey().unwrap().is_err());
    }

    fn shortcut(s: &str) -> Option<Result<Hotkey, ShortcutError>> {
        Action {
            id: "a".into(),
            title: "A".into(),
            shortcut: Some(s.into()),
        }
        .hotkey()
    }

    #[test]
    fn shortcut_needs_ctrl_alt_or_super() {
        for bare in ["c", "f2", "shift+c", "shift+f2"] {
            assert_eq!(
                shortcut(bare),
                Some(Err(ShortcutError::NoModifier(bare.into()))),
                "{bare}"
            );
        }
        for ok in ["ctrl+shift+c", "alt+c", "super+f2", "ctrl+alt+x", "ctrl+1"] {
            assert!(shortcut(ok).unwrap().is_ok(), "{ok}");
        }
    }

    #[test]
    fn shortcut_may_not_use_the_leader() {
        for s in ["leader+f", "Leader+Ctrl+F", "ctrl+leader+shift+x"] {
            assert_eq!(
                shortcut(s),
                Some(Err(ShortcutError::Leader(s.into()))),
                "{s}"
            );
        }
    }

    #[test]
    fn shortcut_refuses_reserved_keys() {
        for key in [
            "tab",
            "backspace",
            "delete",
            "home",
            "end",
            "up",
            "down",
            "left",
            "right",
            "enter",
            "return",
            "esc",
            "escape",
            "space",
        ] {
            for mods in ["ctrl", "alt+shift", "super"] {
                let s = format!("{mods}+{key}");
                assert_eq!(
                    shortcut(&s),
                    Some(Err(ShortcutError::Reserved(s.clone()))),
                    "{s}"
                );
            }
        }
    }

    #[test]
    fn shortcut_key_is_a_character_or_a_short_key_name() {
        for ok in [
            "ctrl+shift+c",
            "alt+é",
            "ctrl+f24",
            "super+pagedown",
            "ctrl+kp_add",
            "alt+1",
        ] {
            assert!(shortcut(ok).unwrap().is_ok(), "{ok}");
        }
        let long = format!("ctrl+{}", "x".repeat(MAX_KEY_NAME_LEN + 1));
        for bad in [
            "ctrl+x\nx",
            "ctrl+\u{202e}",
            "alt+\u{7}",
            "ctrl+a b",
            "ctrl+pgup!",
            long.as_str(),
        ] {
            assert_eq!(
                shortcut(bad),
                Some(Err(ShortcutError::UnknownKey(bad.trim().to_string()))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn shortcut_refuses_ctrl_editing_keys() {
        for key in ["a", "c", "v", "x", "z", "y", "k"] {
            for s in [
                format!("ctrl+{key}"),
                format!("Ctrl+{}", key.to_uppercase()),
            ] {
                assert_eq!(
                    shortcut(&s),
                    Some(Err(ShortcutError::Reserved(s.clone()))),
                    "{s}"
                );
            }
            // Another modifier with Ctrl makes it a different combination.
            assert!(shortcut(&format!("ctrl+shift+{key}")).unwrap().is_ok());
        }
    }
}
