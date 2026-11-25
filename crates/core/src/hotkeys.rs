//! Rules for item hotkeys and the launcher's, shared by the daemon (on the file) and the
//! recorder (on a press), and the table of hotkeys the launcher shows.

use std::fmt;

use crate::index::ItemId;
use crate::keys;
use crate::leader::LeaderMode;
use crate::settings::{Hotkey, Modifiers, Settings};

/// Whose hotkey a change is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyTarget {
    /// The hotkey that opens the launcher.
    Launcher,
    /// The leader key.
    Leader,
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
    /// A `Leader+…` hotkey whose key is the leader's own key.
    LeaderKey,
    /// The leader key itself.
    LeaderHotkey,
    /// The launcher hotkey cannot start with `Leader`.
    LauncherLeader,
    /// With the new leader, `item`'s `Leader+…` hotkey becomes `leader_form`, which `other`
    /// has.
    LeaderClash {
        leader_form: Hotkey,
        item: ItemId,
        other: ItemId,
    },
    /// With the new leader, `item`'s `Leader+…` hotkey becomes `leader_form`, the launcher
    /// hotkey.
    LauncherClash {
        leader_form: Hotkey,
        item: ItemId,
    },
}

impl Refusal {
    /// The refusal in the user's words, with `title_of` naming the item that has the hotkey or
    /// clashes with the leader (its readable id when that gives `None`).
    pub fn text(&self, title_of: &dyn Fn(&ItemId) -> Option<String>) -> String {
        match self {
            Refusal::TakenBy(item) => {
                let title = title_of(item).unwrap_or_else(|| item.fallback_title());
                format!("Already set on \"{title}\". Clear it there first.")
            }
            Refusal::LeaderClash {
                leader_form, item, ..
            } => {
                let title = title_of(item).unwrap_or_else(|| item.fallback_title());
                format!(
                    "With this leader, {} is set on \"{title}\" twice.",
                    leader_form.label()
                )
            }
            Refusal::LauncherClash { leader_form, item } => {
                let title = title_of(item).unwrap_or_else(|| item.fallback_title());
                format!(
                    "With this leader, {} on \"{title}\" is the launcher hotkey.",
                    leader_form.label()
                )
            }
            other => other.to_string(),
        }
    }
}

/// The refusal in the user's words; an item that has the hotkey (or clashes with the leader) is
/// named by its readable id (see [`Refusal::text`] for its title).
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
            Refusal::LeaderKey => f.write_str("That key is the leader's own."),
            Refusal::LeaderHotkey => f.write_str("That is the leader key."),
            Refusal::LauncherLeader => f.write_str("The launcher hotkey cannot use the leader."),
            Refusal::LeaderClash { .. } | Refusal::LauncherClash { .. } => {
                f.write_str(&self.text(&|_| None))
            }
        }
    }
}

/// The hotkeys of one configuration: the launcher's and the leader, which decide which hotkeys
/// are the same keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyspace {
    pub launcher: Hotkey,
    /// `None`: no leader set.
    pub leader: Option<Hotkey>,
    pub mode: LeaderMode,
}

impl Keyspace {
    pub fn of(settings: &Settings) -> Keyspace {
        Keyspace {
            launcher: settings.launcher_hotkey.clone(),
            leader: settings.leader.clone(),
            mode: settings.leader_mode,
        }
    }
}

/// What the system registers for `hotkey`: itself without `Leader`; with a modifiers-only
/// leader in hold mode, the leader's modifiers in place of `Leader`. `None`: the leader window
/// reads it (a leader with a key, tap mode), or no leader is set.
pub fn system_form(hotkey: &Hotkey, keyspace: &Keyspace) -> Option<Hotkey> {
    if !hotkey.mods.contains(Modifiers::LEADER) {
        return Some(hotkey.clone());
    }
    match &keyspace.leader {
        Some(leader) if leader.is_modifiers_only() && keyspace.mode == LeaderMode::Hold => {
            Some(Hotkey {
                mods: (hotkey.mods - Modifiers::LEADER) | leader.mods,
                key: hotkey.key.clone(),
            })
        }
        _ => None,
    }
}

/// The keys pressed for `hotkey`: its [`system_form`], except that under a leader with a key
/// and modifiers in hold mode a `Leader+…` hotkey is pressed with the leader's modifiers held
/// (holding Ctrl+Space and pressing F sends Ctrl+F), though nothing is registered for it.
fn pressed_form(hotkey: &Hotkey, keyspace: &Keyspace) -> Option<Hotkey> {
    match &keyspace.leader {
        Some(leader)
            if hotkey.mods.contains(Modifiers::LEADER)
                && keyspace.mode == LeaderMode::Hold
                && !leader.is_modifiers_only()
                && !leader.mods.is_empty() =>
        {
            Some(Hotkey {
                mods: (hotkey.mods - Modifiers::LEADER) | leader.mods,
                key: hotkey.key.clone(),
            })
        }
        _ => system_form(hotkey, keyspace),
    }
}

/// Whether `a` and `b` are the same keys under `keyspace`: equal, or pressed as the same
/// keys ([`pressed_form`]).
pub fn same_keys(a: &Hotkey, b: &Hotkey, keyspace: &Keyspace) -> bool {
    a == b
        || matches!(
            (pressed_form(a, keyspace), pressed_form(b, keyspace)),
            (Some(x), Some(y)) if x == y
        )
}

/// Whether `hotkey` can be a global hotkey at all, apart from who else has it.
fn usable(hotkey: &Hotkey) -> Result<(), Refusal> {
    if !hotkey
        .mods
        .intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::SUPER | Modifiers::LEADER)
    {
        return Err(Refusal::NoModifier);
    }
    if !keys::is_supported(&hotkey.key) {
        return Err(Refusal::UnsupportedKey(hotkey.key.clone()));
    }
    Ok(())
}

/// Whether `item` may take `hotkey`, given the usable item hotkeys `assigned` and the
/// launcher hotkey and leader of `keyspace`.
pub fn check(
    assigned: &[(ItemId, Hotkey)],
    keyspace: &Keyspace,
    item: &ItemId,
    hotkey: &Hotkey,
) -> Result<(), Refusal> {
    if keyspace.leader.as_ref() == Some(hotkey) {
        return Err(Refusal::LeaderHotkey);
    }
    usable(hotkey)?;
    if hotkey.mods.contains(Modifiers::LEADER)
        && let Some(leader) = &keyspace.leader
        && !leader.is_modifiers_only()
        && hotkey.key == leader.key
    {
        return Err(Refusal::LeaderKey);
    }
    if same_keys(hotkey, &keyspace.launcher, keyspace) {
        return Err(Refusal::LauncherHotkey);
    }
    match assigned
        .iter()
        .find(|(other, h)| other != item && same_keys(h, hotkey, keyspace))
    {
        Some((other, _)) => Err(Refusal::TakenBy(other.clone())),
        None => Ok(()),
    }
}

/// Whether the launcher may take `hotkey`, given the usable item hotkeys `assigned` and the
/// leader of `keyspace`.
pub fn check_launcher(
    assigned: &[(ItemId, Hotkey)],
    keyspace: &Keyspace,
    hotkey: &Hotkey,
) -> Result<(), Refusal> {
    if keyspace.leader.as_ref() == Some(hotkey) {
        return Err(Refusal::LeaderHotkey);
    }
    if hotkey.mods.contains(Modifiers::LEADER) {
        return Err(Refusal::LauncherLeader);
    }
    usable(hotkey)?;
    match assigned
        .iter()
        .find(|(_, h)| same_keys(h, hotkey, keyspace))
    {
        Some((other, _)) => Err(Refusal::TakenBy(other.clone())),
        None => Ok(()),
    }
}

/// Whether `leader` may be the leader key, given the usable item hotkeys `assigned` and the
/// launcher hotkey of `keyspace`: a supported key (or modifiers alone), not the launcher
/// hotkey, and making no two hotkeys the same keys. Whether this system can use it is not
/// decided here.
pub fn check_leader(
    assigned: &[(ItemId, Hotkey)],
    keyspace: &Keyspace,
    leader: &Hotkey,
) -> Result<(), Refusal> {
    if *leader == keyspace.launcher {
        return Err(Refusal::LauncherHotkey);
    }
    if !leader.is_modifiers_only() && !keys::is_supported(&leader.key) {
        return Err(Refusal::UnsupportedKey(leader.key.clone()));
    }
    if let Some((item, _)) = assigned.iter().find(|(_, h)| h == leader) {
        return Err(Refusal::TakenBy(item.clone()));
    }
    let is_leader = |h: &Hotkey| h.mods.contains(Modifiers::LEADER);
    if !leader.is_modifiers_only()
        && assigned
            .iter()
            .any(|(_, h)| is_leader(h) && h.key == leader.key)
    {
        return Err(Refusal::LeaderKey);
    }
    let changed = Keyspace {
        leader: Some(leader.clone()),
        ..keyspace.clone()
    };
    if let Some((item, hotkey)) = assigned
        .iter()
        .find(|(_, h)| same_keys(h, &changed.launcher, &changed))
    {
        return Err(Refusal::LauncherClash {
            leader_form: pressed_form(hotkey, &changed).unwrap_or_else(|| hotkey.clone()),
            item: item.clone(),
        });
    }
    for (i, (a, ha)) in assigned.iter().enumerate() {
        for (b, hb) in &assigned[i + 1..] {
            if a == b || !same_keys(ha, hb, &changed) {
                continue;
            }
            // Name the `Leader+…` hotkey's item first: its hotkey is the one the leader changes.
            let ((item, hotkey), other) = if is_leader(ha) {
                ((a, ha), b)
            } else {
                ((b, hb), a)
            };
            return Err(Refusal::LeaderClash {
                leader_form: pressed_form(hotkey, &changed).unwrap_or_else(|| hotkey.clone()),
                item: item.clone(),
                other: other.clone(),
            });
        }
    }
    Ok(())
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
    let keyspace = Keyspace::of(settings);
    let mut items: Vec<(ItemId, Hotkey)> = Vec::new();
    let mut dropped = Vec::new();
    for (position, (hotkey, item)) in settings.hotkeys.iter().enumerate() {
        let reason = match check(&items, &keyspace, item, hotkey) {
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
    /// A `Leader+…` hotkey while no leader is set.
    NoLeader,
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
    /// The leader key; `None` when none is set.
    pub leader: Option<Hotkey>,
    /// What became of the leader key; `None` when there is none.
    pub leader_state: Option<HotkeyState>,
    pub leader_mode: LeaderMode,
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

    /// The launcher hotkey, leader and mode of this table.
    pub fn keyspace(&self) -> Keyspace {
        Keyspace {
            // No launcher hotkey known: nothing equals an empty key.
            launcher: self.launcher.clone().unwrap_or(Hotkey {
                mods: Modifiers::empty(),
                key: String::new(),
            }),
            leader: self.leader.clone(),
            mode: self.leader_mode,
        }
    }

    /// [`check`] against this table; refused while a change is pending.
    pub fn check(&self, item: &ItemId, hotkey: &Hotkey) -> Result<(), Refusal> {
        self.check_target(&HotkeyTarget::Item(item.clone()), hotkey)
    }

    /// [`check`], [`check_launcher`] or [`check_leader`] against this table; refused while a
    /// change is pending.
    pub fn check_target(&self, target: &HotkeyTarget, hotkey: &Hotkey) -> Result<(), Refusal> {
        if self.pending {
            return Err(Refusal::Pending);
        }
        let assigned: Vec<(ItemId, Hotkey)> = self
            .items
            .iter()
            .map(|h| (h.item.clone(), h.hotkey.clone()))
            .collect();
        let keyspace = self.keyspace();
        match target {
            HotkeyTarget::Launcher => check_launcher(&assigned, &keyspace, hotkey),
            HotkeyTarget::Leader => check_leader(&assigned, &keyspace, hotkey),
            HotkeyTarget::Item(item) => check(&assigned, &keyspace, item, hotkey),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leader::LeaderMode;

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

    /// The default keyspace: launcher Alt+Space, no leader.
    fn plain() -> Keyspace {
        ks(None, LeaderMode::Hold)
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
            check(&[], &plain(), &id("apps/k"), &hk("shift+a")),
            Err(Refusal::NoModifier)
        );
    }

    #[test]
    fn check_refuses_the_launcher_hotkey() {
        assert_eq!(
            check(&[], &plain(), &id("apps/k"), &hk("alt+space")),
            Err(Refusal::LauncherHotkey)
        );
    }

    #[test]
    fn check_refuses_another_items_hotkey() {
        let assigned = [(id("apps/konsole"), hk("ctrl+alt+t"))];
        assert_eq!(
            check(&assigned, &plain(), &id("apps/kitty"), &hk("ctrl+alt+t")),
            Err(Refusal::TakenBy(id("apps/konsole")))
        );
    }

    #[test]
    fn check_launcher_refuses_an_items_hotkey() {
        let assigned = [(id("apps/konsole"), hk("ctrl+alt+t"))];
        assert_eq!(
            check_launcher(&assigned, &plain(), &hk("ctrl+alt+t")),
            Err(Refusal::TakenBy(id("apps/konsole")))
        );
        assert_eq!(
            check_launcher(&assigned, &plain(), &hk("shift+a")),
            Err(Refusal::NoModifier)
        );
        assert_eq!(
            check_launcher(&assigned, &plain(), &hk("super+space")),
            Ok(())
        );
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
            check(&assigned, &plain(), &id("apps/konsole"), &hk("ctrl+alt+t")),
            Ok(())
        );
    }

    #[test]
    fn check_refuses_unsupported_keys() {
        assert_eq!(
            check(&[], &plain(), &id("apps/k"), &hk("ctrl+escape")),
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

    fn ks(leader: Option<&str>, mode: LeaderMode) -> Keyspace {
        Keyspace {
            launcher: hk("alt+space"),
            leader: leader.map(|l| Hotkey::parse_leader(l).unwrap()),
            mode,
        }
    }

    #[test]
    fn leader_hotkeys_need_no_other_modifier() {
        assert_eq!(
            check(
                &[],
                &ks(Some("F12"), LeaderMode::Hold),
                &id("apps/f"),
                &hk("leader+f")
            ),
            Ok(())
        );
    }

    #[test]
    fn with_a_modifier_leader_in_hold_mode_leader_f_is_ctrl_f() {
        let assigned = [(id("apps/kitty"), hk("ctrl+f"))];
        let k = ks(Some("Ctrl"), LeaderMode::Hold);
        assert_eq!(
            check(&assigned, &k, &id("apps/ff"), &hk("leader+f")),
            Err(Refusal::TakenBy(id("apps/kitty")))
        );
        assert_eq!(
            system_form(&hk("leader+shift+f"), &k),
            Some(hk("ctrl+shift+f"))
        );
        // In tap mode they are different gestures.
        assert_eq!(
            check(
                &assigned,
                &ks(Some("Ctrl"), LeaderMode::Tap),
                &id("apps/ff"),
                &hk("leader+f")
            ),
            Ok(())
        );
        assert_eq!(
            system_form(&hk("leader+f"), &ks(Some("F12"), LeaderMode::Hold)),
            None
        );
    }

    #[test]
    fn leader_key_of_own_hotkey_is_refused() {
        assert_eq!(
            check(
                &[],
                &ks(Some("Ctrl+Space"), LeaderMode::Hold),
                &id("apps/a"),
                &hk("leader+space")
            ),
            Err(Refusal::LeaderKey)
        );
        let assigned = [(id("apps/a"), hk("leader+f"))];
        assert_eq!(
            check_leader(
                &assigned,
                &ks(Some("F12"), LeaderMode::Hold),
                &Hotkey::parse_leader("F").unwrap()
            ),
            Err(Refusal::LeaderKey)
        );
    }

    #[test]
    fn leader_change_clash_is_refused() {
        let assigned = [
            (id("apps/ff"), hk("leader+f")),
            (id("apps/kitty"), hk("ctrl+f")),
        ];
        let r = check_leader(
            &assigned,
            &ks(Some("F12"), LeaderMode::Hold),
            &Hotkey::parse_leader("Ctrl").unwrap(),
        );
        assert!(matches!(r, Err(Refusal::LeaderClash { .. })), "{r:?}");
        assert_eq!(
            check_leader(
                &assigned,
                &ks(Some("F12"), LeaderMode::Tap),
                &Hotkey::parse_leader("Ctrl").unwrap()
            ),
            Ok(())
        );
    }

    #[test]
    fn a_leader_clash_names_the_leader_hotkey_first_in_either_order() {
        let expected = Err(Refusal::LeaderClash {
            leader_form: hk("ctrl+f"),
            item: id("apps/ff"),
            other: id("apps/kitty"),
        });
        let ctrl = Hotkey::parse_leader("Ctrl").unwrap();
        let k = ks(None, LeaderMode::Hold);
        let both = [
            (id("apps/ff"), hk("leader+f")),
            (id("apps/kitty"), hk("ctrl+f")),
        ];
        assert_eq!(check_leader(&both, &k, &ctrl), expected);
        let reversed = [both[1].clone(), both[0].clone()];
        assert_eq!(check_leader(&reversed, &k, &ctrl), expected);
    }

    #[test]
    fn the_leader_may_not_be_the_launcher_hotkey() {
        assert_eq!(
            check_leader(&[], &ks(None, LeaderMode::Hold), &hk("alt+space")),
            Err(Refusal::LauncherHotkey)
        );
        assert_eq!(
            check_launcher(
                &[],
                &ks(Some("F12"), LeaderMode::Hold),
                &Hotkey::parse_leader("F12").unwrap()
            ),
            Err(Refusal::LeaderHotkey)
        );
    }

    #[test]
    fn the_launcher_hotkey_cannot_use_the_leader() {
        let k = ks(Some("F12"), LeaderMode::Hold);
        assert_eq!(
            check_launcher(&[], &k, &hk("leader+space")),
            Err(Refusal::LauncherLeader)
        );
        assert_eq!(
            check_launcher(&[], &k, &hk("leader+ctrl+space")),
            Err(Refusal::LauncherLeader)
        );
    }

    #[test]
    fn the_launcher_compares_by_the_keys_a_modifier_leader_makes() {
        let k = Keyspace {
            launcher: hk("ctrl+space"),
            ..ks(Some("Ctrl"), LeaderMode::Hold)
        };
        assert_eq!(
            check(&[], &k, &id("apps/a"), &hk("leader+space")),
            Err(Refusal::LauncherHotkey)
        );
        let assigned = [(id("apps/ff"), hk("leader+f"))];
        assert_eq!(
            check_launcher(&assigned, &k, &hk("ctrl+f")),
            Err(Refusal::TakenBy(id("apps/ff")))
        );
    }

    #[test]
    fn same_keys_under_a_keyspace() {
        let hold = ks(Some("Ctrl"), LeaderMode::Hold);
        assert!(same_keys(&hk("leader+f"), &hk("ctrl+f"), &hold));
        assert!(same_keys(&hk("leader+f"), &hk("leader+f"), &hold));
        assert!(!same_keys(&hk("leader+f"), &hk("ctrl+g"), &hold));
        assert!(!same_keys(
            &hk("leader+f"),
            &hk("ctrl+f"),
            &ks(Some("Ctrl"), LeaderMode::Tap)
        ));
        assert!(!same_keys(
            &hk("leader+f"),
            &hk("ctrl+f"),
            &ks(None, LeaderMode::Hold)
        ));
    }

    #[test]
    fn a_combination_leader_in_hold_mode_makes_leader_f_its_modifiers_and_f() {
        // Holding Ctrl+Space and pressing F sends Ctrl+F.
        let hold = ks(Some("Ctrl+Space"), LeaderMode::Hold);
        assert!(same_keys(&hk("leader+f"), &hk("ctrl+f"), &hold));
        assert!(same_keys(&hk("leader+shift+f"), &hk("ctrl+shift+f"), &hold));
        assert!(!same_keys(&hk("leader+f"), &hk("ctrl+g"), &hold));
        // Nothing is registered for it: the leader window reads it.
        assert_eq!(system_form(&hk("leader+f"), &hold), None);
        let kitty = [(id("apps/kitty"), hk("ctrl+f"))];
        assert_eq!(
            check(&kitty, &hold, &id("apps/ff"), &hk("leader+f")),
            Err(Refusal::TakenBy(id("apps/kitty")))
        );
        let ff = [(id("apps/ff"), hk("leader+f"))];
        assert_eq!(
            check(&ff, &hold, &id("apps/kitty"), &hk("ctrl+f")),
            Err(Refusal::TakenBy(id("apps/ff")))
        );
        let alt_leader = ks(Some("Alt+F12"), LeaderMode::Hold);
        assert_eq!(
            check(&[], &alt_leader, &id("apps/a"), &hk("leader+space")),
            Err(Refusal::LauncherHotkey)
        );
        let both = [
            (id("apps/kitty"), hk("ctrl+f")),
            (id("apps/ff"), hk("leader+f")),
        ];
        assert_eq!(
            check_leader(
                &both,
                &ks(Some("F12"), LeaderMode::Hold),
                &Hotkey::parse_leader("Ctrl+Space").unwrap()
            ),
            Err(Refusal::LeaderClash {
                leader_form: hk("ctrl+f"),
                item: id("apps/ff"),
                other: id("apps/kitty"),
            })
        );
        let mut s = settings(&[
            ("Leader+F", "apps/ff.desktop"),
            ("Ctrl+F", "apps/kitty.desktop"),
        ]);
        s.leader = Some(Hotkey::parse_leader("Ctrl+Space").unwrap());
        let (items, dropped) = item_hotkeys(&s);
        assert_eq!(items, [(id("apps/ff.desktop"), hk("leader+f"))]);
        assert_eq!(dropped.len(), 1, "{dropped:?}");

        // Tap mode: the leader is up before the key, so they are different gestures.
        let tap = ks(Some("Ctrl+Space"), LeaderMode::Tap);
        assert!(!same_keys(&hk("leader+f"), &hk("ctrl+f"), &tap));
        assert_eq!(check(&kitty, &tap, &id("apps/ff"), &hk("leader+f")), Ok(()));
        assert_eq!(
            check_leader(
                &both,
                &ks(Some("F12"), LeaderMode::Tap),
                &Hotkey::parse_leader("Ctrl+Space").unwrap()
            ),
            Ok(())
        );
        s.leader_mode = LeaderMode::Tap;
        assert_eq!(item_hotkeys(&s).0.len(), 2);
    }

    #[test]
    fn a_leader_whose_key_is_not_supported_is_refused() {
        assert_eq!(
            check_leader(&[], &plain(), &Hotkey::parse_leader("Ctrl+Escape").unwrap()),
            Err(Refusal::UnsupportedKey("escape".into()))
        );
        assert_eq!(
            check_leader(&[], &plain(), &Hotkey::parse_leader("Ctrl").unwrap()),
            Ok(())
        );
        assert_eq!(
            check_leader(&[], &plain(), &Hotkey::parse_leader("Pause").unwrap()),
            Ok(())
        );
    }

    #[test]
    fn the_keyspace_of_settings() {
        let s = Settings::parse(
            "[launcher]\nhotkey = \"Super+Space\"\nleader = \"Ctrl\"\nleader_mode = \"tap\"\n",
        )
        .unwrap();
        assert_eq!(
            Keyspace::of(&s),
            Keyspace {
                launcher: hk("super+space"),
                leader: Some(Hotkey::parse_leader("Ctrl").unwrap()),
                mode: LeaderMode::Tap,
            }
        );
    }

    #[test]
    fn the_file_rules_follow_a_modifier_leader() {
        let mut s = settings(&[
            ("Leader+F", "apps/ff.desktop"),
            ("Ctrl+F", "apps/kitty.desktop"),
        ]);
        s.leader = Some(Hotkey::parse_leader("Ctrl").unwrap());
        let (items, dropped) = item_hotkeys(&s);
        assert_eq!(items, [(id("apps/ff.desktop"), hk("leader+f"))]);
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        s.leader_mode = LeaderMode::Tap;
        assert_eq!(item_hotkeys(&s).0.len(), 2);
    }

    #[test]
    fn table_check_target_knows_the_leader() {
        let table = HotkeyTable {
            launcher: Some(launcher()),
            leader: Some(Hotkey::parse_leader("Ctrl").unwrap()),
            items: vec![ItemHotkey {
                item: id("apps/kitty"),
                hotkey: hk("ctrl+f"),
                state: HotkeyState::Active { trigger: None },
            }],
            ..HotkeyTable::default()
        };
        assert_eq!(
            table.check(&id("apps/ff"), &hk("leader+f")),
            Err(Refusal::TakenBy(id("apps/kitty")))
        );
        assert_eq!(
            table.check_target(&HotkeyTarget::Leader, &hk("alt+space")),
            Err(Refusal::LauncherHotkey)
        );
        assert_eq!(
            table.check_target(&HotkeyTarget::Leader, &Hotkey::parse_leader("F12").unwrap()),
            Ok(())
        );
    }

    #[test]
    fn a_leader_that_turns_a_leader_hotkey_into_the_launcher_hotkey_is_refused() {
        let k = Keyspace {
            launcher: hk("ctrl+space"),
            ..ks(Some("F12"), LeaderMode::Hold)
        };
        let assigned = [(id("apps/a"), hk("leader+space"))];
        assert_eq!(
            check_leader(&assigned, &k, &Hotkey::parse_leader("Ctrl").unwrap()),
            Err(Refusal::LauncherClash {
                leader_form: hk("ctrl+space"),
                item: id("apps/a"),
            })
        );
        let tap = Keyspace {
            mode: LeaderMode::Tap,
            ..k
        };
        assert_eq!(
            check_leader(&assigned, &tap, &Hotkey::parse_leader("Ctrl").unwrap()),
            Ok(())
        );
    }

    #[test]
    fn a_leader_that_is_an_items_hotkey_is_refused() {
        let assigned = [(id("apps/kitty"), hk("ctrl+alt+t"))];
        assert_eq!(
            check_leader(
                &assigned,
                &plain(),
                &Hotkey::parse_leader("Ctrl+Alt+T").unwrap()
            ),
            Err(Refusal::TakenBy(id("apps/kitty")))
        );
    }

    #[test]
    fn an_item_may_not_take_the_leader_key() {
        let k = ks(Some("Ctrl+Space"), LeaderMode::Hold);
        assert_eq!(
            check(&[], &k, &id("apps/a"), &hk("ctrl+space")),
            Err(Refusal::LeaderHotkey)
        );
        assert_eq!(
            check(
                &[],
                &ks(Some("F12"), LeaderMode::Hold),
                &id("apps/a"),
                &hk("f12")
            ),
            Err(Refusal::LeaderHotkey)
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
        assert_eq!(
            Refusal::LeaderKey.to_string(),
            "That key is the leader's own."
        );
        assert_eq!(Refusal::LeaderHotkey.to_string(), "That is the leader key.");
        assert_eq!(
            Refusal::LeaderClash {
                leader_form: hk("ctrl+f"),
                item: id("apps/ff"),
                other: id("apps/kitty"),
            }
            .to_string(),
            "With this leader, Ctrl+F is set on \"ff\" twice."
        );
        assert_eq!(
            Refusal::LauncherClash {
                leader_form: hk("ctrl+space"),
                item: id("apps/a"),
            }
            .to_string(),
            "With this leader, Ctrl+Space on \"a\" is the launcher hotkey."
        );
        assert_eq!(
            Refusal::LauncherLeader.to_string(),
            "The launcher hotkey cannot use the leader."
        );
    }
}
