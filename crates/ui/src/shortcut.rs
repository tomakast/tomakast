//! Action shortcuts: a module's key combination that runs an action on the
//! selected row (`Action::shortcut`).
//!
//! The core refuses shortcuts that take the keys the user types and
//! navigates with (Ruling J). A valid one may still be a key the launcher
//! binds on this platform (Cmd+K on macOS is Super+K) or a text-editing key
//! of the kit's inputs (Ctrl+E, Ctrl+Shift+Z); such a shortcut, and one that
//! is invalid after all, is ignored: it runs nothing and shows no chips, and
//! the action is reached through the action panel.

use gpui_kit::{Keystroke, Modifiers};
use tomakast_core::settings::Modifiers as Mods;
use tomakast_core::{Action, Hotkey, View};

/// The text-editing keys with Ctrl, Alt or Cmd that the kit's inputs bind
/// (`gpui-base` 0.10, `input/base/state.rs`), for every platform at once:
/// undo and redo, select all, copy, cut, paste, word and line moves and
/// selections, word deletes, indent, search. Cmd is Super off macOS, where
/// the kit binds Ctrl instead, so a module's shortcut is refused the same
/// way everywhere. The core's Ruling J validation already refuses Ctrl
/// alone with A, C, V, X, Z, Y and K, and every navigation and editing key
/// (arrows, Home, End, Backspace, Delete) with any modifiers (core tests
/// `shortcut_refuses_ctrl_editing_keys` and `shortcut_refuses_reserved_keys`);
/// those are repeated here so the table reads as the kit's.
const TEXT_EDITING_KEYS: &[&str] = &[
    // Undo, redo.
    "ctrl-z",
    "ctrl-y",
    "ctrl-shift-z",
    "cmd-z",
    "cmd-shift-z",
    "cmd-y",
    // Select all, copy, cut, paste.
    "ctrl-a",
    "ctrl-c",
    "ctrl-x",
    "ctrl-v",
    "cmd-a",
    "cmd-c",
    "cmd-x",
    "cmd-v",
    // Line start and end, and selecting to them.
    "ctrl-e",
    "ctrl-shift-a",
    "ctrl-shift-e",
    "cmd-left",
    "cmd-right",
    "shift-cmd-left",
    "shift-cmd-right",
    "ctrl-home",
    "ctrl-end",
    "ctrl-shift-home",
    "ctrl-shift-end",
    "cmd-up",
    "cmd-down",
    "cmd-shift-up",
    "cmd-shift-down",
    // Word moves and selections.
    "ctrl-left",
    "ctrl-right",
    "alt-left",
    "alt-right",
    "ctrl-shift-left",
    "ctrl-shift-right",
    "alt-shift-left",
    "alt-shift-right",
    // Extra cursors.
    "ctrl-alt-up",
    "ctrl-alt-down",
    "cmd-alt-up",
    "cmd-alt-down",
    "alt-shift-up",
    "alt-shift-down",
    // Word and line deletes.
    "ctrl-backspace",
    "ctrl-delete",
    "alt-backspace",
    "alt-delete",
    "cmd-backspace",
    "cmd-delete",
    // Indent, outdent, search, replace, code actions, character palette.
    "ctrl-]",
    "ctrl-[",
    "cmd-]",
    "cmd-[",
    "ctrl-f",
    "ctrl-h",
    "cmd-f",
    "cmd-shift-f",
    "ctrl-.",
    "cmd-.",
    "ctrl-cmd-space",
];

/// The keys a shortcut may not take: the launcher's bindings (see
/// [`crate::init`]), the form's Tab and Shift+Tab, and the kit's
/// text-editing keys.
pub(crate) fn launcher_keys() -> impl Iterator<Item = &'static str> {
    crate::LAUNCHER_KEYS
        .into_iter()
        .chain(crate::FORM_FOCUS_KEYS)
        .chain(TEXT_EDITING_KEYS.iter().copied())
}

/// Most ignored shortcuts logged one by one per push; the rest are counted.
const WARNED_SHORTCUTS: usize = 5;

/// The keystroke of `hotkey`: Super is the platform key (Cmd on macOS).
fn keystroke(hotkey: &Hotkey) -> Keystroke {
    Keystroke {
        modifiers: Modifiers {
            control: hotkey.mods.contains(Mods::CTRL),
            alt: hotkey.mods.contains(Mods::ALT),
            shift: hotkey.mods.contains(Mods::SHIFT),
            platform: hotkey.mods.contains(Mods::SUPER),
            function: false,
        },
        key: hotkey.key.clone(),
        key_char: None,
    }
}

/// Whether `pressed` is the key combination `expected`: the same key with
/// the same modifiers; what it would type does not matter.
fn same(pressed: &Keystroke, expected: &Keystroke) -> bool {
    pressed.modifiers == expected.modifiers && pressed.key.eq_ignore_ascii_case(&expected.key)
}

/// Whether `hotkey` is one of `keys` (GPUI keystroke syntax).
fn collides(hotkey: &Hotkey, keys: impl IntoIterator<Item = &'static str>) -> bool {
    let wanted = keystroke(hotkey);
    keys.into_iter()
        .filter_map(|key| Keystroke::parse(key).ok())
        .any(|key| same(&key, &wanted))
}

/// Why a shortcut is not usable, for the log.
#[derive(Debug, PartialEq, Eq)]
enum Unusable {
    Invalid,
    LauncherKey,
}

/// The shortcut of `action` if it is valid and not a launcher key.
fn check(action: &Action) -> Option<Result<Hotkey, Unusable>> {
    Some(match action.hotkey()? {
        Err(_) => Err(Unusable::Invalid),
        Ok(hotkey) if collides(&hotkey, launcher_keys()) => Err(Unusable::LauncherKey),
        Ok(hotkey) => Ok(hotkey),
    })
}

/// The usable shortcut of `action`, if it has one.
pub(crate) fn usable(action: &Action) -> Option<Hotkey> {
    check(action)?.ok()
}

/// The first of `actions` whose usable shortcut is `pressed`.
pub(crate) fn find<'a>(
    actions: impl IntoIterator<Item = &'a Action>,
    pressed: &Keystroke,
) -> Option<&'a Action> {
    actions
        .into_iter()
        .find(|action| usable(action).is_some_and(|hotkey| same(pressed, &keystroke(&hotkey))))
}

/// The first [`WARNED_SHORTCUTS`] ignored shortcuts of `view`, with why,
/// and how many more there are.
fn ignored_shortcuts(view: &View) -> (Vec<(&Action, Unusable)>, usize) {
    let actions: Box<dyn Iterator<Item = &Action>> = match view {
        View::List(v) => Box::new(v.rows.iter().flat_map(|r| &r.actions)),
        View::Grid(v) => Box::new(v.tiles.iter().flat_map(|t| &t.actions)),
        View::Detail(v) => Box::new(v.actions.iter()),
        View::Scratchpad(v) => Box::new(v.actions.iter()),
        View::Form(_) => return (Vec::new(), 0),
    };
    let mut ignored = actions.filter_map(|action| match check(action)? {
        Err(why) => Some((action, why)),
        Ok(_) => None,
    });
    let first = ignored.by_ref().take(WARNED_SHORTCUTS).collect();
    (first, ignored.count())
}

/// Logs the shortcuts of `view` that are ignored: the first few one by
/// one, then how many more.
pub(crate) fn warn_unusable(provider: &str, view: &View) {
    let (first, more) = ignored_shortcuts(view);
    for (action, why) in first {
        let why = match why {
            Unusable::Invalid => "is invalid",
            Unusable::LauncherKey => "is a launcher or text-editing key",
        };
        tracing::warn!(
            module = %provider,
            action = ?action.id,
            shortcut = ?action.shortcut,
            "action shortcut {why}; ignored"
        );
    }
    if more > 0 {
        tracing::warn!(module = %provider, count = more, "more action shortcuts ignored");
    }
}

/// The key chips of `hotkey`: its modifiers, then its key.
/// The chips of the key that runs a pad's first action: Ctrl+Enter, Cmd+Enter on macOS.
pub(crate) fn submit_chips() -> Vec<String> {
    let key = if cfg!(target_os = "macos") {
        "super+enter"
    } else {
        "ctrl+enter"
    };
    Hotkey::parse(key)
        .map(|hotkey| chips(&hotkey))
        .unwrap_or_default()
}

pub(crate) fn chips(hotkey: &Hotkey) -> Vec<String> {
    let names: [(Mods, &str, &str); 4] = [
        (Mods::CTRL, "Ctrl", "⌃"),
        (Mods::ALT, "Alt", "⌥"),
        (Mods::SHIFT, "Shift", "⇧"),
        (Mods::SUPER, "Super", "⌘"),
    ];
    let mut chips: Vec<String> = names
        .iter()
        .filter(|(flag, _, _)| hotkey.mods.contains(*flag))
        .map(|&(_, other, mac)| {
            if cfg!(target_os = "macos") {
                mac
            } else {
                other
            }
            .to_string()
        })
        .collect();
    chips.push(key_label(&hotkey.key));
    chips
}

/// `c` -> `C`, `f2` -> `F2`, `pageup` -> `Page Up`; `enter` -> `↵`, as the Enter hints draw it.
fn key_label(key: &str) -> String {
    if key == "enter" {
        return "↵".into();
    }
    tomakast_core::keys::display_name(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkey(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn with_shortcut(shortcut: &str) -> Action {
        Action {
            id: "a".into(),
            title: "A".into(),
            shortcut: Some(shortcut.into()),
        }
    }

    #[test]
    fn super_k_collides_with_the_macos_panel_key() {
        let mac = ["up", "down", "enter", "escape", "cmd-k", "cmd-enter"];
        assert!(collides(&hotkey("super+k"), mac));
        assert!(!collides(&hotkey("super+j"), mac));
        let other = ["up", "down", "enter", "escape", "ctrl-k", "ctrl-enter"];
        assert!(!collides(&hotkey("super+k"), other));
        assert!(collides(&hotkey("ctrl+k"), other));
    }

    #[test]
    fn text_editing_keys_are_not_usable() {
        // Valid for the core, but the kit's inputs bind them.
        for taken in ["ctrl+shift+z", "ctrl+e", "ctrl+f", "alt+left", "super+c"] {
            let hotkey = hotkey(taken);
            assert!(collides(&hotkey, launcher_keys()), "{taken}");
            assert_eq!(usable(&with_shortcut(taken)), None, "{taken}");
        }
        assert!(Action::hotkey(&with_shortcut("ctrl+shift+z")).is_some_and(|h| h.is_ok()));
    }

    #[test]
    fn at_most_five_ignored_shortcuts_are_listed() {
        let mut actions: Vec<Action> = (0..7)
            .map(|n| Action {
                id: format!("bad{n}"),
                ..with_shortcut("shift+c")
            })
            .collect();
        actions.push(with_shortcut("ctrl+shift+c"));
        let view = View::Detail(tomakast_core::DetailView {
            id: tomakast_core::ViewId("m/v".into()),
            title: String::new(),
            markdown: String::new(),
            metadata: Vec::new(),
            actions,
        });
        let (first, more) = ignored_shortcuts(&view);
        let ids: Vec<&str> = first.iter().map(|(a, _)| a.id.as_str()).collect();
        assert_eq!(ids, ["bad0", "bad1", "bad2", "bad3", "bad4"]);
        assert!(first.iter().all(|(_, why)| *why == Unusable::Invalid));
        assert_eq!(more, 2);
    }

    #[test]
    fn every_listed_key_parses() {
        for key in launcher_keys() {
            assert!(Keystroke::parse(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn this_platforms_panel_key_is_not_usable() {
        let panel = if cfg!(target_os = "macos") {
            "super+k"
        } else {
            "ctrl+k"
        };
        assert_eq!(usable(&with_shortcut(panel)), None);
        assert_eq!(
            usable(&with_shortcut("ctrl+shift+c")),
            Some(hotkey("ctrl+shift+c"))
        );
    }

    #[test]
    fn file_search_shortcuts_are_usable() {
        // Show in File Manager, Copy Path, Open With… and the grid toggle of the files module.
        for key in ["ctrl+shift+f", "ctrl+shift+c", "ctrl+o", "ctrl+g"] {
            assert_eq!(usable(&with_shortcut(key)), Some(hotkey(key)), "{key}");
        }
    }

    #[test]
    fn invalid_shortcuts_are_not_usable() {
        for bad in ["", "ctrl+", "shift+c", "ctrl+enter", "c", "ctrl+foo+bar"] {
            assert_eq!(usable(&with_shortcut(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn find_matches_modifiers_exactly() {
        let actions = [
            with_shortcut("ctrl+shift+c"),
            Action {
                id: "b".into(),
                ..with_shortcut("alt+f2")
            },
        ];
        let press = |s: &'static str| Keystroke::parse(s).unwrap();
        assert_eq!(
            find(&actions, &press("ctrl-shift-c")).map(|a| &*a.id),
            Some("a")
        );
        assert_eq!(find(&actions, &press("alt-f2")).map(|a| &*a.id), Some("b"));
        assert_eq!(find(&actions, &press("ctrl-c")), None);
        assert_eq!(find(&actions, &press("ctrl-alt-shift-c")), None);
    }

    #[test]
    fn chips_name_modifiers_then_the_key() {
        let expected: &[&str] = if cfg!(target_os = "macos") {
            &["⌃", "⇧", "C"]
        } else {
            &["Ctrl", "Shift", "C"]
        };
        assert_eq!(chips(&hotkey("shift+ctrl+c")), expected);
        assert_eq!(
            chips(&hotkey("alt+f2")).last().map(String::as_str),
            Some("F2")
        );
        assert_eq!(
            chips(&hotkey("ctrl+pageup")).last().map(String::as_str),
            Some("Page Up")
        );
        assert_eq!(
            chips(&hotkey("ctrl+enter")).last().map(String::as_str),
            Some("↵")
        );
    }
}
