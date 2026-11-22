//! Rules for item hotkeys, shared by the daemon (on the file) and the recorder (on a press),
//! and the table of item hotkeys the launcher shows.

use std::fmt;

use crate::index::ItemId;
use crate::keys;
use crate::settings::{Hotkey, Settings};

/// Why a hotkey cannot be given to an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Neither Ctrl, Alt nor Super (Shift alone does not count).
    NoModifier,
    /// The key (config name) is not one a hotkey can use.
    UnsupportedKey(String),
    LauncherHotkey,
    /// Another item has it.
    TakenBy(ItemId),
    /// Another change is still being applied.
    Pending,
    /// `config.toml` does not load; the text is the error.
    BrokenConfig(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoModifier => f.write_str("Add Ctrl, Alt or Super."),
            Refusal::UnsupportedKey(key) => write!(f, "{key} cannot be a hotkey."),
            Refusal::LauncherHotkey => f.write_str("That is the launcher hotkey."),
            Refusal::TakenBy(item) => {
                write!(f, "Already set on {}. Clear it there first.", item.as_str())
            }
            Refusal::Pending => f.write_str("Waiting for the last change."),
            Refusal::BrokenConfig(e) => write!(f, "config.toml has an error: {e}"),
        }
    }
}

/// Whether `hotkey` can be a global hotkey at all, apart from who else has it.
fn usable(hotkey: &Hotkey) -> Result<(), Refusal> {
    use crate::settings::Modifiers;
    if !hotkey
        .mods
        .intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(Refusal::NoModifier);
    }
    if !keys::is_supported(&hotkey.key) {
        return Err(Refusal::UnsupportedKey(hotkey.key.clone()));
    }
    Ok(())
}

/// Whether `item` may take `hotkey`, given the usable item hotkeys `assigned` and the
/// launcher's.
pub fn check(
    assigned: &[(ItemId, Hotkey)],
    launcher: &Hotkey,
    item: &ItemId,
    hotkey: &Hotkey,
) -> Result<(), Refusal> {
    usable(hotkey)?;
    if hotkey == launcher {
        return Err(Refusal::LauncherHotkey);
    }
    match assigned
        .iter()
        .find(|(other, h)| h == hotkey && other != item)
    {
        Some((other, _)) => Err(Refusal::TakenBy(other.clone())),
        None => Ok(()),
    }
}

/// The usable `[hotkeys]` in the order `settings` holds them, and a log line for each entry
/// dropped: unusable, the launcher's, already bound, or a second hotkey of one item. The first
/// binding wins.
pub fn item_hotkeys(settings: &Settings) -> (Vec<(ItemId, Hotkey)>, Vec<String>) {
    let mut items: Vec<(ItemId, Hotkey)> = Vec::new();
    let mut dropped = Vec::new();
    for (hotkey, item) in &settings.hotkeys {
        let problem = match check(&items, &settings.launcher_hotkey, item, hotkey) {
            Err(Refusal::TakenBy(_)) => Some("it is already bound to another item".to_string()),
            Err(refusal) => Some(refusal.to_string()),
            Ok(()) if items.iter().any(|(other, _)| other == item) => {
                Some("the item already has a hotkey".to_string())
            }
            Ok(()) => None,
        };
        match problem {
            Some(problem) => dropped.push(format!(
                "ignoring hotkey {hotkey} for {}: {problem}",
                item.as_str()
            )),
            None => items.push((item.clone(), hotkey.clone())),
        }
    }
    (items, dropped)
}

/// What became of an item's hotkey.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyState {
    /// Bound. `trigger` is the system's own name for it when that differs.
    Active { trigger: Option<String> },
    /// Saved; this desktop binds hotkeys only at start.
    AfterRestart,
    /// In the file, but the system did not take it.
    Refused { reason: String },
    /// No hotkeys in this run.
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemHotkey {
    pub item: ItemId,
    pub hotkey: Hotkey,
    pub state: HotkeyState,
}

/// The item hotkeys as the daemon applied them, for the launcher's rows and recorder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotkeyTable {
    pub launcher: Option<Hotkey>,
    pub items: Vec<ItemHotkey>,
    /// A change is being applied; another is refused until it is done.
    pub pending: bool,
    /// Why there are no hotkeys in this run, if there are none.
    pub unavailable: Option<String>,
}

impl HotkeyTable {
    pub fn of(&self, item: &ItemId) -> Option<&ItemHotkey> {
        self.items.iter().find(|h| &h.item == item)
    }

    /// [`check`] against this table; refused while a change is pending.
    pub fn check(&self, item: &ItemId, hotkey: &Hotkey) -> Result<(), Refusal> {
        if self.pending {
            return Err(Refusal::Pending);
        }
        let assigned: Vec<(ItemId, Hotkey)> = self
            .items
            .iter()
            .map(|h| (h.item.clone(), h.hotkey.clone()))
            .collect();
        // No launcher hotkey known: nothing equals an empty key.
        let none = Hotkey {
            mods: crate::settings::Modifiers::empty(),
            key: String::new(),
        };
        check(
            &assigned,
            self.launcher.as_ref().unwrap_or(&none),
            item,
            hotkey,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn id(s: &str) -> ItemId {
        ItemId::parse(s).unwrap()
    }

    fn settings(hotkeys: &[(&str, &str)]) -> Settings {
        Settings {
            hotkeys: hotkeys.iter().map(|(k, i)| (hk(k), id(i))).collect(),
            ..Settings::default()
        }
    }

    fn launcher() -> Hotkey {
        hk("alt+space")
    }

    #[test]
    fn items_keep_their_hotkeys() {
        let (items, dropped) = item_hotkeys(&settings(&[("Ctrl+Alt+T", "apps/konsole.desktop")]));
        assert_eq!(items, [(id("apps/konsole.desktop"), hk("ctrl+alt+t"))]);
        assert!(dropped.is_empty());
    }

    #[test]
    fn unusable_hotkeys_are_dropped_and_the_first_binding_wins() {
        let (items, dropped) = item_hotkeys(&settings(&[
            // Equal to the launcher hotkey after normalisation.
            ("alt+SPACE", "apps/a.desktop"),
            ("F1", "apps/b.desktop"),
            ("Ctrl+Alt+T", "apps/konsole.desktop"),
            // Same hotkey as above, spelled differently.
            ("ctrl+alt+t", "apps/c.desktop"),
            // Second hotkey for the same item.
            ("Ctrl+Alt+K", "apps/konsole.desktop"),
            ("Super+F", "apps/firefox.desktop"),
            ("Ctrl+Escape", "apps/d.desktop"),
        ]));
        assert_eq!(
            items,
            [
                (id("apps/konsole.desktop"), hk("ctrl+alt+t")),
                (id("apps/firefox.desktop"), hk("super+f")),
            ]
        );
        assert_eq!(dropped.len(), 5, "{dropped:?}");
    }

    #[test]
    fn check_refuses_without_ctrl_alt_or_super() {
        assert_eq!(
            check(&[], &launcher(), &id("apps/k"), &hk("shift+a")),
            Err(Refusal::NoModifier)
        );
    }

    #[test]
    fn check_refuses_the_launcher_hotkey() {
        assert_eq!(
            check(&[], &launcher(), &id("apps/k"), &hk("alt+space")),
            Err(Refusal::LauncherHotkey)
        );
    }

    #[test]
    fn check_refuses_another_items_hotkey() {
        let assigned = [(id("apps/konsole"), hk("ctrl+alt+t"))];
        assert_eq!(
            check(&assigned, &launcher(), &id("apps/kitty"), &hk("ctrl+alt+t")),
            Err(Refusal::TakenBy(id("apps/konsole")))
        );
    }

    #[test]
    fn check_allows_the_items_own_hotkey() {
        let assigned = [(id("apps/konsole"), hk("ctrl+alt+t"))];
        assert_eq!(
            check(
                &assigned,
                &launcher(),
                &id("apps/konsole"),
                &hk("ctrl+alt+t")
            ),
            Ok(())
        );
    }

    #[test]
    fn check_refuses_unsupported_keys() {
        assert_eq!(
            check(&[], &launcher(), &id("apps/k"), &hk("ctrl+escape")),
            Err(Refusal::UnsupportedKey("escape".into()))
        );
    }

    #[test]
    fn table_check_refuses_while_pending() {
        let table = HotkeyTable {
            launcher: Some(launcher()),
            pending: true,
            ..HotkeyTable::default()
        };
        assert_eq!(
            table.check(&id("apps/k"), &hk("ctrl+alt+t")),
            Err(Refusal::Pending)
        );
    }

    #[test]
    fn table_check_counts_every_state_as_taken() {
        let table = HotkeyTable {
            launcher: Some(launcher()),
            items: vec![ItemHotkey {
                item: id("apps/konsole"),
                hotkey: hk("ctrl+alt+t"),
                state: HotkeyState::Refused {
                    reason: "used by kitty: Launch".into(),
                },
            }],
            ..HotkeyTable::default()
        };
        assert_eq!(
            table.check(&id("apps/kitty"), &hk("ctrl+alt+t")),
            Err(Refusal::TakenBy(id("apps/konsole")))
        );
        assert_eq!(
            table.of(&id("apps/konsole")).map(|h| &h.hotkey),
            Some(&hk("ctrl+alt+t"))
        );
    }

    #[test]
    fn refusal_texts() {
        assert_eq!(
            Refusal::TakenBy(id("apps/konsole")).to_string(),
            "Already set on apps/konsole. Clear it there first."
        );
        assert_eq!(
            Refusal::LauncherHotkey.to_string(),
            "That is the launcher hotkey."
        );
        assert_eq!(Refusal::NoModifier.to_string(), "Add Ctrl, Alt or Super.");
        assert_eq!(
            Refusal::UnsupportedKey("escape".into()).to_string(),
            "escape cannot be a hotkey."
        );
        assert_eq!(Refusal::Pending.to_string(), "Waiting for the last change.");
        assert_eq!(
            Refusal::BrokenConfig("bad".into()).to_string(),
            "config.toml has an error: bad"
        );
    }
}
