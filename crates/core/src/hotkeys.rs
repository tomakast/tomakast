//! Rules for item hotkeys and the launcher's, shared by the daemon (on the file) and the
//! recorder (on a press), and the table of hotkeys the launcher shows.

use std::fmt;

use crate::index::ItemId;
use crate::keys;
use crate::settings::{Hotkey, Settings};

/// Whose hotkey a change is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyTarget {
    /// The hotkey that opens the launcher.
    Launcher,
    Item(ItemId),
}

/// Where a hotkey change was asked for, so its notice is shown there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyOrigin {
    /// The launcher's recorder, or the file.
    Launcher,
    /// The settings window.
    Settings,
}

/// Why a hotkey cannot be given to an item or the launcher.
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

impl Refusal {
    /// The refusal in the user's words, with `title_of` naming the item that has the hotkey
    /// (its readable id when that gives `None`).
    pub fn text(&self, title_of: &dyn Fn(&ItemId) -> Option<String>) -> String {
        match self {
            Refusal::TakenBy(item) => {
                let title = title_of(item).unwrap_or_else(|| item.fallback_title());
                format!("Already set on \"{title}\". Clear it there first.")
            }
            other => other.to_string(),
        }
    }
}

/// The refusal in the user's words; an item that has the hotkey is named by its readable id
/// (see [`Refusal::text`] for its title).
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoModifier => f.write_str("Add Ctrl, Alt or Super."),
            Refusal::UnsupportedKey(key) => {
                write!(f, "{} cannot be a hotkey.", keys::display_name(key))
            }
            Refusal::LauncherHotkey => f.write_str("That is the launcher hotkey."),
            Refusal::TakenBy(_) => f.write_str(&self.text(&|_| None)),
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

/// Whether the launcher may take `hotkey`, given the usable item hotkeys `assigned`.
pub fn check_launcher(assigned: &[(ItemId, Hotkey)], hotkey: &Hotkey) -> Result<(), Refusal> {
    usable(hotkey)?;
    match assigned.iter().find(|(_, h)| h == hotkey) {
        Some((other, _)) => Err(Refusal::TakenBy(other.clone())),
        None => Ok(()),
    }
}

/// A `[hotkeys]` entry the launcher does not use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedHotkey {
    /// Its index in [`Settings::hotkeys`], which holds the entries in the order of the
    /// file's `[hotkeys]` table read as a [`toml::Table`].
    pub position: usize,
    pub hotkey: Hotkey,
    pub item: ItemId,
    pub reason: DropReason,
}

/// Why a `[hotkeys]` entry is not used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The hotkey cannot be the item's: unusable, the launcher's, or another item's.
    Refused(Refusal),
    /// The item already has this hotkey from an earlier entry.
    ItemHasOne(Hotkey),
}

/// The usable `[hotkeys]` in the order `settings` holds them, and a log line for each entry
/// dropped: unusable, the launcher's, already bound, or a second hotkey of one item. The first
/// binding wins.
pub fn item_hotkeys(settings: &Settings) -> (Vec<(ItemId, Hotkey)>, Vec<String>) {
    let (items, dropped) = sort_hotkeys(settings);
    let dropped = dropped
        .into_iter()
        .map(|d| {
            let problem = match d.reason {
                DropReason::Refused(Refusal::TakenBy(_)) => {
                    "it is already bound to another item".to_string()
                }
                DropReason::Refused(refusal) => refusal.to_string(),
                DropReason::ItemHasOne(_) => "the item already has a hotkey".to_string(),
            };
            format!(
                "ignoring hotkey {} for {}: {problem}",
                d.hotkey,
                d.item.as_str()
            )
        })
        .collect();
    (items, dropped)
}

/// The `[hotkeys]` entries [`item_hotkeys`] drops, with where they are and why.
pub fn dropped_hotkeys(settings: &Settings) -> Vec<DroppedHotkey> {
    sort_hotkeys(settings).1
}

fn sort_hotkeys(settings: &Settings) -> (Vec<(ItemId, Hotkey)>, Vec<DroppedHotkey>) {
    let mut items: Vec<(ItemId, Hotkey)> = Vec::new();
    let mut dropped = Vec::new();
    for (position, (hotkey, item)) in settings.hotkeys.iter().enumerate() {
        let reason = match check(&items, &settings.launcher_hotkey, item, hotkey) {
            Err(refusal) => Some(DropReason::Refused(refusal)),
            Ok(()) => items
                .iter()
                .find(|(other, _)| other == item)
                .map(|(_, kept)| DropReason::ItemHasOne(kept.clone())),
        };
        match reason {
            Some(reason) => dropped.push(DroppedHotkey {
                position,
                hotkey: hotkey.clone(),
                item: item.clone(),
                reason,
            }),
            None => items.push((item.clone(), hotkey.clone())),
        }
    }
    (items, dropped)
}

/// What became of an item's hotkey, or the launcher's.
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
    /// What became of the launcher hotkey; `None` when there is none.
    pub launcher_state: Option<HotkeyState>,
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
        self.check_target(&HotkeyTarget::Item(item.clone()), hotkey)
    }

    /// [`check`] or [`check_launcher`] against this table; refused while a change is pending.
    pub fn check_target(&self, target: &HotkeyTarget, hotkey: &Hotkey) -> Result<(), Refusal> {
        if self.pending {
            return Err(Refusal::Pending);
        }
        let assigned: Vec<(ItemId, Hotkey)> = self
            .items
            .iter()
            .map(|h| (h.item.clone(), h.hotkey.clone()))
            .collect();
        let item = match target {
            HotkeyTarget::Launcher => return check_launcher(&assigned, hotkey),
            HotkeyTarget::Item(item) => item,
        };
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
    fn dropped_entries_say_where_they_are_and_why() {
        let dropped = dropped_hotkeys(&settings(&[
            ("Ctrl+Alt+T", "apps/konsole.desktop"),
            ("F1", "apps/b.desktop"),
            ("ctrl+alt+t", "apps/c.desktop"),
            ("Ctrl+Alt+K", "apps/konsole.desktop"),
            ("Alt+Space", "apps/d.desktop"),
        ]));
        assert_eq!(
            dropped,
            [
                DroppedHotkey {
                    position: 1,
                    hotkey: hk("f1"),
                    item: id("apps/b.desktop"),
                    reason: DropReason::Refused(Refusal::NoModifier),
                },
                DroppedHotkey {
                    position: 2,
                    hotkey: hk("ctrl+alt+t"),
                    item: id("apps/c.desktop"),
                    reason: DropReason::Refused(Refusal::TakenBy(id("apps/konsole.desktop"))),
                },
                DroppedHotkey {
                    position: 3,
                    hotkey: hk("ctrl+alt+k"),
                    item: id("apps/konsole.desktop"),
                    reason: DropReason::ItemHasOne(hk("ctrl+alt+t")),
                },
                DroppedHotkey {
                    position: 4,
                    hotkey: hk("alt+space"),
                    item: id("apps/d.desktop"),
                    reason: DropReason::Refused(Refusal::LauncherHotkey),
                },
            ]
        );
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
    fn check_launcher_refuses_an_items_hotkey() {
        let assigned = [(id("apps/konsole"), hk("ctrl+alt+t"))];
        assert_eq!(
            check_launcher(&assigned, &hk("ctrl+alt+t")),
            Err(Refusal::TakenBy(id("apps/konsole")))
        );
        assert_eq!(
            check_launcher(&assigned, &hk("shift+a")),
            Err(Refusal::NoModifier)
        );
        assert_eq!(check_launcher(&assigned, &hk("super+space")), Ok(()));
    }

    #[test]
    fn table_check_target_checks_the_launcher_against_items() {
        let table = HotkeyTable {
            launcher: Some(launcher()),
            items: vec![ItemHotkey {
                item: id("apps/konsole"),
                hotkey: hk("ctrl+alt+t"),
                state: HotkeyState::Active { trigger: None },
            }],
            ..HotkeyTable::default()
        };
        assert_eq!(
            table.check_target(&HotkeyTarget::Launcher, &hk("ctrl+alt+t")),
            Err(Refusal::TakenBy(id("apps/konsole")))
        );
        assert_eq!(
            table.check_target(&HotkeyTarget::Launcher, &hk("super+space")),
            Ok(())
        );
        assert_eq!(
            table.check_target(&HotkeyTarget::Item(id("apps/kitty")), &hk("alt+space")),
            Err(Refusal::LauncherHotkey)
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
        // Without a title, the item's readable name, never its raw id.
        assert_eq!(
            Refusal::TakenBy(id("apps/org.kde.konsole.desktop")).to_string(),
            "Already set on \"org.kde.konsole\". Clear it there first."
        );
        assert_eq!(
            Refusal::TakenBy(id("apps/konsole")).text(&|_| Some("Konsole".into())),
            "Already set on \"Konsole\". Clear it there first."
        );
        assert_eq!(
            Refusal::TakenBy(id("apps/konsole")).text(&|_| None),
            "Already set on \"konsole\". Clear it there first."
        );
        assert_eq!(
            Refusal::LauncherHotkey.to_string(),
            "That is the launcher hotkey."
        );
        assert_eq!(Refusal::NoModifier.to_string(), "Add Ctrl, Alt or Super.");
        assert_eq!(
            Refusal::UnsupportedKey("escape".into()).to_string(),
            "Esc cannot be a hotkey."
        );
        assert_eq!(Refusal::Pending.to_string(), "Waiting for the last change.");
        assert_eq!(
            Refusal::BrokenConfig("bad".into()).to_string(),
            "config.toml has an error: bad"
        );
    }
}
