//! The daemon's hotkeys: binds the launcher set and the item set, holds one pending change
//! (from the recorder or the settings window), and writes `config.toml` only once the system
//! accepted it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tomakast_core::hotkeys::{
    HotkeyOrigin, HotkeyState, HotkeyTable, HotkeyTarget, ItemHotkey, Refusal, check,
    check_launcher, item_hotkeys,
};
use tomakast_core::settings_edit::{set_item_hotkey, set_key, write_config};
use tomakast_core::{Hotkey, ItemId, Settings};
use tomakast_platform::{BindState, HotkeyBinding, HotkeyEvent, Registrar, SetKind};
use tracing::{info, warn};

/// The binding id of the launcher hotkey; item hotkeys use their item id.
pub(crate) const LAUNCHER_ID: &str = "launcher";
const LAUNCHER_DESCRIPTION: &str = "Open tomakast";

/// What the daemon does with the result of a call.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// The rows' hotkeys changed.
    Table(HotkeyTable),
    /// A notice for the user about the change of `target`, shown where it was asked for.
    Notice {
        text: String,
        origin: HotkeyOrigin,
        target: HotkeyTarget,
    },
    /// A hotkey was pressed: run it, or hand it to an open recorder.
    Pressed { id: String, hotkey: Hotkey },
}

/// A change sent to the system and not answered yet.
struct Pending {
    generation: u64,
    target: HotkeyTarget,
    origin: HotkeyOrigin,
    /// What goes into the file: `None` clears an item's hotkey, or resets the launcher's.
    hotkey: Option<Hotkey>,
    title: String,
    /// The set before the change, to bind back.
    previous: Vec<HotkeyBinding>,
    /// The set with the change.
    bindings: Vec<HotkeyBinding>,
}

impl Pending {
    fn set(&self) -> SetKind {
        set_of(&self.target)
    }

    fn id(&self) -> &str {
        match &self.target {
            HotkeyTarget::Launcher => LAUNCHER_ID,
            HotkeyTarget::Item(item) => item.as_str(),
        }
    }

    /// The hotkey the change binds; `None` when it clears an item's.
    fn bound(&self) -> Option<&Hotkey> {
        self.bindings
            .iter()
            .find(|b| b.id == self.id())
            .map(|b| &b.hotkey)
    }
}

fn set_of(target: &HotkeyTarget) -> SetKind {
    match target {
        HotkeyTarget::Launcher => SetKind::Launcher,
        HotkeyTarget::Item(_) => SetKind::Items,
    }
}

pub(crate) struct Hotkeys<R: Registrar> {
    registrar: R,
    config_file: PathBuf,
    generation: u64,
    /// The sets as last sent from the file (or from an accepted change).
    applied: BTreeMap<SetKind, Vec<HotkeyBinding>>,
    /// The generation of the last set sent per set, whose answer sets the states.
    latest: BTreeMap<SetKind, u64>,
    states: BTreeMap<String, HotkeyState>,
    pending: Option<Pending>,
    unavailable: Option<String>,
}

fn same(a: &[HotkeyBinding], b: &[HotkeyBinding]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.id == y.id && x.hotkey == y.hotkey)
}

fn launcher_bindings(hotkey: &Hotkey) -> Vec<HotkeyBinding> {
    vec![HotkeyBinding {
        id: LAUNCHER_ID.into(),
        description: LAUNCHER_DESCRIPTION.into(),
        hotkey: hotkey.clone(),
    }]
}

fn item_bindings(
    items: &[(ItemId, Hotkey)],
    title: &dyn Fn(&ItemId) -> Option<String>,
) -> Vec<HotkeyBinding> {
    items
        .iter()
        .map(|(item, hotkey)| HotkeyBinding {
            id: item.as_str().to_string(),
            description: title(item).unwrap_or_else(|| item.fallback_title()),
            hotkey: hotkey.clone(),
        })
        .collect()
}

impl<R: Registrar> Hotkeys<R> {
    pub fn new(registrar: R, config_file: PathBuf) -> Self {
        Hotkeys {
            registrar,
            config_file,
            generation: 0,
            applied: BTreeMap::new(),
            latest: BTreeMap::new(),
            states: BTreeMap::new(),
            pending: None,
            unavailable: None,
        }
    }

    fn send(&mut self, set: SetKind, bindings: Vec<HotkeyBinding>) -> u64 {
        self.generation += 1;
        let generation = self.generation;
        self.latest.insert(set, generation);
        info!(?set, generation, count = bindings.len(), "binding hotkeys");
        self.registrar.set(set, generation, bindings);
        generation
    }

    /// Sends `bindings` as `set` when they differ from what was last sent.
    fn apply(&mut self, set: SetKind, bindings: Vec<HotkeyBinding>) {
        if self
            .applied
            .get(&set)
            .is_some_and(|old| same(old, &bindings))
        {
            return;
        }
        self.applied.insert(set, bindings.clone());
        self.send(set, bindings);
    }

    /// Settings (re)loaded from the file: binds each set that changed. While a change is
    /// pending this waits: the change re-reads the file when it is answered.
    pub fn load(
        &mut self,
        settings: &Settings,
        title: &dyn Fn(&ItemId) -> Option<String>,
    ) -> Vec<Outcome> {
        if self.pending.is_some() {
            return Vec::new();
        }
        self.apply(
            SetKind::Launcher,
            launcher_bindings(&settings.launcher_hotkey),
        );
        let (items, dropped) = item_hotkeys(settings);
        for line in dropped {
            warn!("{line}");
        }
        self.apply(SetKind::Items, item_bindings(&items, title));
        vec![Outcome::Table(self.table())]
    }

    /// A recorder's Enter (`Some`) or Backspace (`None`) on `target`: checked against the
    /// file as it is now, then sent as the pending change. `None` clears an item's hotkey and
    /// resets the launcher's to the default; the launcher hotkey it already has changes
    /// nothing, except that a reset of a file holding the default takes the key out of it
    /// without asking the system.
    pub fn request(
        &mut self,
        target: &HotkeyTarget,
        hotkey: Option<Hotkey>,
        origin: HotkeyOrigin,
        title: &dyn Fn(&ItemId) -> Option<String>,
    ) -> Result<Vec<Outcome>, Refusal> {
        if self.pending.is_some() {
            return Err(Refusal::Pending);
        }
        let settings = Settings::load(&self.config_file)
            .map_err(|e| Refusal::BrokenConfig(format!("{e:#}")))?;
        let (mut items, _) = item_hotkeys(&settings);
        let (bindings, title) = match target {
            HotkeyTarget::Launcher => {
                let bound = hotkey
                    .clone()
                    .unwrap_or_else(|| Settings::default().launcher_hotkey);
                if bound == settings.launcher_hotkey {
                    return Ok(match hotkey {
                        None => self.remove_launcher_key(origin),
                        Some(_) => Vec::new(),
                    });
                }
                check_launcher(&items, &bound)?;
                (launcher_bindings(&bound), "the launcher".to_string())
            }
            HotkeyTarget::Item(item) => {
                if let Some(hotkey) = &hotkey {
                    check(&items, &settings.launcher_hotkey, item, hotkey)?;
                }
                items.retain(|(other, _)| other != item);
                if let Some(hotkey) = &hotkey {
                    items.push((item.clone(), hotkey.clone()));
                }
                (
                    item_bindings(&items, title),
                    title(item).unwrap_or_else(|| item.fallback_title()),
                )
            }
        };
        let set = set_of(target);
        let previous = self.applied.get(&set).cloned().unwrap_or_default();
        let generation = self.send(set, bindings.clone());
        self.pending = Some(Pending {
            generation,
            target: target.clone(),
            origin,
            hotkey,
            title,
            previous,
            bindings,
        });
        Ok(vec![Outcome::Table(self.table())])
    }

    pub fn on_event(&mut self, event: HotkeyEvent) -> Vec<Outcome> {
        match event {
            HotkeyEvent::Activated(id) => self
                .applied
                .values()
                .flatten()
                .find(|b| b.id == id)
                .map(|b| Outcome::Pressed {
                    id,
                    hotkey: b.hotkey.clone(),
                })
                .into_iter()
                .collect(),
            HotkeyEvent::Unavailable { reason } => {
                warn!(
                    "global hotkeys are unavailable ({reason}); \
                     bind `tomakast toggle` and `tomakast run <id>` in your system settings"
                );
                self.unavailable = Some(reason);
                vec![Outcome::Table(self.table())]
            }
            HotkeyEvent::Applied {
                set,
                generation,
                outcome,
            } => {
                let pending = self
                    .pending
                    .take_if(|p| p.set() == set && p.generation == generation);
                match pending {
                    Some(pending) => self.answer(pending, outcome),
                    None => {
                        if let Err(reason) = &outcome {
                            warn!(?set, "hotkeys not bound: {reason}");
                        }
                        if self.latest.get(&set) == Some(&generation) {
                            self.record(set, &outcome);
                        }
                        vec![Outcome::Table(self.table())]
                    }
                }
            }
        }
    }

    /// Stores the per-binding states of an answer.
    fn record(&mut self, set: SetKind, outcome: &Result<Vec<(String, BindState)>, String>) {
        match outcome {
            Ok(states) => {
                for (id, state) in states {
                    let state = match state {
                        BindState::Active { trigger } => HotkeyState::Active {
                            trigger: trigger.clone(),
                        },
                        BindState::Later => HotkeyState::AfterRestart,
                        BindState::Refused { reason } => HotkeyState::Refused {
                            reason: reason.clone(),
                        },
                    };
                    self.states.insert(id.clone(), state);
                }
            }
            Err(reason) => {
                for b in self.applied.get(&set).into_iter().flatten() {
                    self.states.insert(
                        b.id.clone(),
                        HotkeyState::Refused {
                            reason: reason.clone(),
                        },
                    );
                }
            }
        }
    }

    /// The system's answer to the pending change.
    fn answer(
        &mut self,
        pending: Pending,
        outcome: Result<Vec<(String, BindState)>, String>,
    ) -> Vec<Outcome> {
        let set = pending.set();
        let id = pending.id();
        let refusal = match &outcome {
            Err(reason) => Some((reason.clone(), true)),
            Ok(states) => states
                .iter()
                .find(|(s, _)| s == id)
                .and_then(|(_, state)| match state {
                    BindState::Refused { reason } => Some((reason.clone(), false)),
                    _ => None,
                }),
        };
        let origin = pending.origin;
        let mut out = Vec::new();
        if let Some((reason, rebind)) = refusal {
            // The backend keeps the old binding of a refused id; after a failed set it
            // gets the previous set again.
            if rebind {
                self.send(set, pending.previous.clone());
            }
            let what = match pending.bound() {
                Some(hotkey) => format!("{} for {}", hotkey.label(), pending.title),
                None => format!("Clearing the hotkey of {}", pending.title),
            };
            out.push(Outcome::Notice {
                text: format!("{what}: {reason}; nothing changed"),
                origin,
                target: pending.target.clone(),
            });
        } else {
            match self.save(&pending) {
                Ok(()) => {
                    self.applied.insert(set, pending.bindings.clone());
                    self.states.remove(id);
                    self.record(set, &outcome);
                }
                Err(text) => {
                    self.send(set, pending.previous.clone());
                    self.applied.insert(set, pending.previous.clone());
                    out.push(Outcome::Notice {
                        text,
                        origin,
                        target: pending.target.clone(),
                    });
                }
            }
        }
        self.follow_file(&pending);
        out.push(Outcome::Table(self.table()));
        out
    }

    /// Binds both sets as the file says now: `load` waited while the change was pending, so
    /// a hand edit meanwhile (to either set) is taken here. An unchanged set is not sent.
    fn follow_file(&mut self, pending: &Pending) {
        let settings = match Settings::load(&self.config_file) {
            Ok(settings) => settings,
            Err(e) => {
                warn!("{e:#}; hotkeys stay as they were");
                return;
            }
        };
        // Titles of the items known here; an item new to the file is named by its id.
        let titles: BTreeMap<String, String> = self
            .applied
            .values()
            .flatten()
            .chain(&pending.previous)
            .chain(&pending.bindings)
            .map(|b| (b.id.clone(), b.description.clone()))
            .collect();
        self.apply(
            SetKind::Launcher,
            launcher_bindings(&settings.launcher_hotkey),
        );
        let (items, _) = item_hotkeys(&settings);
        self.apply(
            SetKind::Items,
            item_bindings(&items, &|item| titles.get(item.as_str()).cloned()),
        );
    }

    /// Takes `launcher.hotkey` out of the file, whose value is the default bound already: a
    /// notice when that cannot be written, nothing otherwise.
    fn remove_launcher_key(&self, origin: HotkeyOrigin) -> Vec<Outcome> {
        let text = match std::fs::read_to_string(&self.config_file) {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => Err(format!("Cannot read config.toml: {e}")),
        };
        let removed = text.and_then(|text| {
            let edited = set_key(&text, &["launcher", "hotkey"], None)
                .map_err(|e| format!("Cannot save config.toml: {e:#}"))?;
            if edited == text {
                return Ok(());
            }
            write_config(&self.config_file, &edited)
                .map_err(|e| format!("Cannot save config.toml: {e:#}"))
        });
        match removed {
            Ok(()) => Vec::new(),
            Err(text) => vec![Outcome::Notice {
                text,
                origin,
                target: HotkeyTarget::Launcher,
            }],
        }
    }

    /// Writes the accepted change, if the file still allows it.
    fn save(&self, pending: &Pending) -> Result<(), String> {
        let text = match std::fs::read_to_string(&self.config_file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("Cannot read config.toml: {e}")),
        };
        if let Some(hotkey) = pending.bound() {
            let settings = Settings::load(&self.config_file)
                .map_err(|e| format!("Cannot save config.toml: {e:#}"))?;
            let (items, _) = item_hotkeys(&settings);
            let allowed = match &pending.target {
                HotkeyTarget::Launcher => check_launcher(&items, hotkey),
                HotkeyTarget::Item(item) => check(&items, &settings.launcher_hotkey, item, hotkey),
            };
            if allowed.is_err() {
                return Err("config.toml changed meanwhile; nothing was saved".into());
            }
        }
        let text = match &pending.target {
            HotkeyTarget::Launcher => {
                let value = pending.hotkey.as_ref().map(|h| h.to_string().into());
                set_key(&text, &["launcher", "hotkey"], value)
            }
            HotkeyTarget::Item(item) => set_item_hotkey(&text, item, pending.hotkey.as_ref()),
        }
        .map_err(|e| format!("Cannot save config.toml: {e:#}"))?;
        write_config(&self.config_file, &text)
            .map_err(|e| format!("Cannot save config.toml: {e:#}"))
    }

    pub fn table(&self) -> HotkeyTable {
        let launcher = self
            .applied
            .get(&SetKind::Launcher)
            .and_then(|b| b.first())
            .map(|b| b.hotkey.clone());
        let state = |id: &str| {
            if self.unavailable.is_some() {
                HotkeyState::Off
            } else {
                self.states
                    .get(id)
                    .cloned()
                    .unwrap_or(HotkeyState::Active { trigger: None })
            }
        };
        let items = self
            .applied
            .get(&SetKind::Items)
            .into_iter()
            .flatten()
            .filter_map(|b| {
                Some(ItemHotkey {
                    item: ItemId::parse(&b.id)?,
                    hotkey: b.hotkey.clone(),
                    state: state(&b.id),
                })
            })
            .collect();
        HotkeyTable {
            launcher_state: launcher.as_ref().map(|_| state(LAUNCHER_ID)),
            launcher,
            items,
            pending: self.pending.is_some(),
            unavailable: self.unavailable.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    type Calls = Rc<RefCell<Vec<(SetKind, u64, Vec<(String, String)>)>>>;

    #[derive(Default)]
    struct Fake {
        calls: Calls,
    }

    impl Registrar for Fake {
        fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
            let pairs = bindings
                .into_iter()
                .map(|b| (b.id, b.hotkey.to_string()))
                .collect();
            self.calls.borrow_mut().push((set, generation, pairs));
        }
    }

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn id(s: &str) -> ItemId {
        ItemId::parse(s).unwrap()
    }

    fn no_title(_: &ItemId) -> Option<String> {
        None
    }

    struct Setup {
        _dir: tempfile::TempDir,
        file: PathBuf,
        calls: Calls,
        hotkeys: Hotkeys<Fake>,
    }

    fn setup(config: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        std::fs::write(&file, config).unwrap();
        let fake = Fake::default();
        let calls = fake.calls.clone();
        let hotkeys = Hotkeys::new(fake, file.clone());
        Setup {
            _dir: dir,
            file,
            calls,
            hotkeys,
        }
    }

    fn load(s: &mut Setup) -> Vec<Outcome> {
        let settings = Settings::load(&s.file).unwrap();
        s.hotkeys.load(&settings, &no_title)
    }

    fn last(s: &Setup) -> (SetKind, u64, Vec<(String, String)>) {
        s.calls.borrow().last().cloned().unwrap()
    }

    fn active(ids: &[&str]) -> Result<Vec<(String, BindState)>, String> {
        Ok(ids
            .iter()
            .map(|i| (i.to_string(), BindState::Active { trigger: None }))
            .collect())
    }

    fn applied(
        set: SetKind,
        generation: u64,
        outcome: Result<Vec<(String, BindState)>, String>,
    ) -> HotkeyEvent {
        HotkeyEvent::Applied {
            set,
            generation,
            outcome,
        }
    }

    fn notices(outcomes: &[Outcome]) -> Vec<String> {
        outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Notice { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn item(s: &str) -> HotkeyTarget {
        HotkeyTarget::Item(id(s))
    }

    /// A request from the launcher's recorder.
    fn request(
        s: &mut Setup,
        target: HotkeyTarget,
        hotkey: Option<Hotkey>,
    ) -> Result<Vec<Outcome>, Refusal> {
        s.hotkeys
            .request(&target, hotkey, HotkeyOrigin::Launcher, &no_title)
    }

    const KONSOLE: &str = "[hotkeys]\n\"Ctrl+Alt+K\" = \"apps/konsole\"\n";

    #[test]
    fn startup_binds_both_sets_once() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        {
            let calls = s.calls.borrow();
            assert_eq!(calls.len(), 2, "{calls:?}");
            assert_eq!(calls[0].0, SetKind::Launcher);
            assert_eq!(
                calls[0].2,
                [("launcher".to_string(), "Alt+Space".to_string())]
            );
            assert_eq!(calls[1].0, SetKind::Items);
            assert_eq!(
                calls[1].2,
                [("apps/konsole".to_string(), "Ctrl+Alt+K".to_string())]
            );
        }
        load(&mut s);
        assert_eq!(s.calls.borrow().len(), 2, "an unchanged file binds nothing");
    }

    #[test]
    fn an_accepted_change_is_written_after_acceptance() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&s.file).unwrap(),
            KONSOLE,
            "not yet"
        );
        assert!(s.hotkeys.table().pending);
        let (set, generation, pairs) = last(&s);
        assert_eq!(set, SetKind::Items);
        assert_eq!(pairs.len(), 2);
        s.hotkeys.on_event(applied(
            set,
            generation,
            active(&["apps/konsole", "apps/kitty"]),
        ));
        let text = std::fs::read_to_string(&s.file).unwrap();
        assert!(text.contains("\"Ctrl+Alt+T\" = \"apps/kitty\""), "{text}");
        let table = s.hotkeys.table();
        assert!(!table.pending);
        assert_eq!(
            table.of(&id("apps/kitty")).map(|h| (&h.hotkey, &h.state)),
            Some((&hk("ctrl+alt+t"), &HotkeyState::Active { trigger: None }))
        );
    }

    /// The notice of a refused change names an item no module titles by its readable id,
    /// not its raw one.
    #[test]
    fn a_refused_change_of_an_untitled_item_names_it_readably() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        s.hotkeys
            .request(
                &item("apps/org.kitty.desktop"),
                Some(hk("ctrl+alt+t")),
                HotkeyOrigin::Launcher,
                &no_title,
            )
            .unwrap();
        let (set, generation, _) = last(&s);
        let out = s
            .hotkeys
            .on_event(applied(set, generation, Err("D-Bus is gone".into())));
        assert_eq!(
            notices(&out),
            ["Ctrl+Alt+T for org.kitty: D-Bus is gone; nothing changed"]
        );
    }

    #[test]
    fn a_refused_change_writes_nothing_and_keeps_the_previous_set() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        s.hotkeys
            .request(
                &item("apps/kitty"),
                Some(hk("ctrl+alt+t")),
                HotkeyOrigin::Launcher,
                &|_| Some("Kitty".into()),
            )
            .unwrap();
        let (set, generation, _) = last(&s);
        let calls_before = s.calls.borrow().len();
        let out = s.hotkeys.on_event(applied(
            set,
            generation,
            Ok(vec![
                ("apps/konsole".into(), BindState::Active { trigger: None }),
                (
                    "apps/kitty".into(),
                    BindState::Refused {
                        reason: "used by kitty: Launch".into(),
                    },
                ),
            ]),
        ));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), KONSOLE);
        assert_eq!(
            notices(&out),
            ["Ctrl+Alt+T for Kitty: used by kitty: Launch; nothing changed"]
        );
        assert!(!s.hotkeys.table().pending);
        assert_eq!(s.hotkeys.table().of(&id("apps/kitty")), None);
        assert_eq!(
            s.calls.borrow().len(),
            calls_before,
            "the backend kept the old binding"
        );
    }

    #[test]
    fn a_failed_set_rebinds_the_previous_one() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        let (set, generation, _) = last(&s);
        s.hotkeys
            .on_event(applied(set, generation, Err("D-Bus is gone".into())));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), KONSOLE);
        assert_eq!(
            last(&s).2,
            [("apps/konsole".to_string(), "Ctrl+Alt+K".to_string())]
        );
    }

    #[test]
    fn a_second_request_while_pending_is_refused() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        assert_eq!(
            request(&mut s, item("apps/files"), Some(hk("ctrl+alt+f"))).err(),
            Some(Refusal::Pending)
        );
    }

    #[test]
    fn a_conflicting_hand_edit_skips_the_write() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        let (set, generation, _) = last(&s);
        let edited = "[hotkeys]\n\"Ctrl+Alt+T\" = \"apps/konsole\"\n";
        std::fs::write(&s.file, edited).unwrap();
        let out = s.hotkeys.on_event(applied(
            set,
            generation,
            active(&["apps/konsole", "apps/kitty"]),
        ));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), edited);
        assert_eq!(
            notices(&out),
            ["config.toml changed meanwhile; nothing was saved"]
        );
        // Bound back to what the file now says.
        assert_eq!(
            last(&s).2,
            [("apps/konsole".to_string(), "Ctrl+Alt+T".to_string())]
        );
    }

    #[test]
    fn a_broken_file_refuses_the_change_and_keeps_the_sets() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        std::fs::write(&s.file, "[hotkeys\n").unwrap();
        let calls_before = s.calls.borrow().len();
        assert!(matches!(
            request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).err(),
            Some(Refusal::BrokenConfig(_))
        ));
        assert_eq!(s.calls.borrow().len(), calls_before);
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), "[hotkeys\n");
    }

    #[test]
    fn the_launcher_set_is_never_rebound_by_an_item_change() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        let (set, generation, _) = last(&s);
        s.hotkeys.on_event(applied(
            set,
            generation,
            active(&["apps/konsole", "apps/kitty"]),
        ));
        let launcher_calls = s
            .calls
            .borrow()
            .iter()
            .filter(|(set, _, _)| *set == SetKind::Launcher)
            .count();
        assert_eq!(launcher_calls, 1);
    }

    #[test]
    fn presses_are_reported_with_their_hotkey() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        let out = s
            .hotkeys
            .on_event(HotkeyEvent::Activated("apps/konsole".into()));
        assert!(matches!(
            out.as_slice(),
            [Outcome::Pressed { id, hotkey }] if id == "apps/konsole" && *hotkey == hk("ctrl+alt+k")
        ));
        let out = s
            .hotkeys
            .on_event(HotkeyEvent::Activated("launcher".into()));
        assert!(
            matches!(out.as_slice(), [Outcome::Pressed { hotkey, .. }] if *hotkey == hk("alt+space"))
        );
        assert!(
            s.hotkeys
                .on_event(HotkeyEvent::Activated("apps/gone".into()))
                .is_empty()
        );
    }

    #[test]
    fn clearing_is_a_pending_change_too() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/konsole"), None).unwrap();
        let (set, generation, pairs) = last(&s);
        assert!(pairs.is_empty());
        s.hotkeys.on_event(applied(set, generation, Ok(vec![])));
        let text = std::fs::read_to_string(&s.file).unwrap();
        assert!(!text.contains("apps/konsole"), "{text}");
    }

    #[test]
    fn a_change_for_later_is_written_and_shown_as_after_restart() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        let (set, generation, _) = last(&s);
        s.hotkeys.on_event(applied(
            set,
            generation,
            Ok(vec![
                ("apps/konsole".into(), BindState::Active { trigger: None }),
                ("apps/kitty".into(), BindState::Later),
            ]),
        ));
        assert!(
            std::fs::read_to_string(&s.file)
                .unwrap()
                .contains("apps/kitty")
        );
        assert_eq!(
            s.hotkeys
                .table()
                .of(&id("apps/kitty"))
                .map(|h| h.state.clone()),
            Some(HotkeyState::AfterRestart)
        );
    }

    #[test]
    fn unavailable_hotkeys_show_as_off() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        s.hotkeys.on_event(HotkeyEvent::Unavailable {
            reason: "TOMAKAST_NO_KEYBOARD=1".into(),
        });
        let table = s.hotkeys.table();
        assert_eq!(table.unavailable.as_deref(), Some("TOMAKAST_NO_KEYBOARD=1"));
        assert_eq!(
            table.of(&id("apps/konsole")).map(|h| h.state.clone()),
            Some(HotkeyState::Off)
        );
    }

    #[test]
    fn a_hand_edited_hotkey_the_system_refuses_shows_as_refused() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        let (set, generation, _) = last(&s);
        s.hotkeys.on_event(applied(
            set,
            generation,
            Ok(vec![(
                "apps/konsole".into(),
                BindState::Refused {
                    reason: "used by kitty: Launch".into(),
                },
            )]),
        ));
        assert_eq!(
            s.hotkeys
                .table()
                .of(&id("apps/konsole"))
                .map(|h| h.state.clone()),
            Some(HotkeyState::Refused {
                reason: "used by kitty: Launch".into()
            })
        );
    }

    fn launcher_calls(s: &Setup) -> Vec<(u64, Vec<(String, String)>)> {
        s.calls
            .borrow()
            .iter()
            .filter(|(set, _, _)| *set == SetKind::Launcher)
            .map(|(_, generation, pairs)| (*generation, pairs.clone()))
            .collect()
    }

    fn applied_launcher(s: &Setup) -> Option<Hotkey> {
        s.hotkeys
            .applied
            .get(&SetKind::Launcher)
            .and_then(|b| b.first())
            .map(|b| b.hotkey.clone())
    }

    #[test]
    fn launcher_hotkey_is_written_after_the_system_accepts() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&s.file).unwrap(),
            KONSOLE,
            "not yet"
        );
        assert!(s.hotkeys.table().pending);
        let (set, generation, pairs) = last(&s);
        assert_eq!(set, SetKind::Launcher);
        assert_eq!(pairs, [("launcher".to_string(), "Super+Space".to_string())]);
        s.hotkeys
            .on_event(applied(set, generation, active(&["launcher"])));
        let text = std::fs::read_to_string(&s.file).unwrap();
        assert_eq!(
            Settings::parse(&text).unwrap().launcher_hotkey,
            hk("super+space"),
            "{text}"
        );
        assert!(text.contains("\"Ctrl+Alt+K\" = \"apps/konsole\""), "{text}");
        let table = s.hotkeys.table();
        assert!(!table.pending);
        assert_eq!(table.launcher, Some(hk("super+space")));
        assert_eq!(launcher_calls(&s).len(), 2, "the item set is not touched");

        // Back to the default: the key leaves the file and Alt+Space is bound.
        request(&mut s, HotkeyTarget::Launcher, None).unwrap();
        let (set, generation, pairs) = last(&s);
        assert_eq!(pairs, [("launcher".to_string(), "Alt+Space".to_string())]);
        s.hotkeys
            .on_event(applied(set, generation, active(&["launcher"])));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), KONSOLE);
        assert_eq!(s.hotkeys.table().launcher, Some(hk("alt+space")));
    }

    #[test]
    fn the_launcher_hotkey_it_already_has_is_a_no_op() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        let calls_before = s.calls.borrow().len();
        let out = request(&mut s, HotkeyTarget::Launcher, Some(hk("alt+space"))).unwrap();
        assert!(out.is_empty(), "{out:?}");
        // A reset of a file without the key has nothing to remove.
        let out = request(&mut s, HotkeyTarget::Launcher, None).unwrap();
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(s.calls.borrow().len(), calls_before);
        assert!(!s.hotkeys.table().pending);
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), KONSOLE);
    }

    /// A reset of a file that spells out the default takes the key out at once: the binding
    /// stays as it is, so the system is not asked.
    #[test]
    fn resetting_a_written_default_removes_the_key() {
        let mut s = setup(&format!("[launcher]\nhotkey = \"Alt+Space\"\n\n{KONSOLE}"));
        load(&mut s);
        let calls_before = s.calls.borrow().len();
        let out = request(&mut s, HotkeyTarget::Launcher, None).unwrap();
        assert!(notices(&out).is_empty(), "{out:?}");
        let text = std::fs::read_to_string(&s.file).unwrap();
        assert!(!text.contains("hotkey = \"Alt+Space\""), "{text}");
        assert!(text.contains("\"Ctrl+Alt+K\" = \"apps/konsole\""), "{text}");
        assert_eq!(s.calls.borrow().len(), calls_before);
        assert!(!s.hotkeys.table().pending);
    }

    #[test]
    fn refused_launcher_hotkey_is_not_written() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        s.hotkeys
            .request(
                &HotkeyTarget::Launcher,
                Some(hk("super+space")),
                HotkeyOrigin::Settings,
                &no_title,
            )
            .unwrap();
        let (set, generation, _) = last(&s);
        let calls_before = s.calls.borrow().len();
        let out = s.hotkeys.on_event(applied(
            set,
            generation,
            Ok(vec![(
                "launcher".into(),
                BindState::Refused {
                    reason: "used by krunner: Run".into(),
                },
            )]),
        ));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), KONSOLE);
        assert_eq!(applied_launcher(&s), Some(hk("alt+space")));
        let notices: Vec<_> = out
            .iter()
            .filter_map(|o| match o {
                Outcome::Notice { text, origin, .. } => Some((text.as_str(), *origin)),
                _ => None,
            })
            .collect();
        assert_eq!(
            notices,
            [(
                "Super+Space for the launcher: used by krunner: Run; nothing changed",
                HotkeyOrigin::Settings
            )]
        );
        assert!(!s.hotkeys.table().pending);
        assert_eq!(
            s.calls.borrow().len(),
            calls_before,
            "the backend kept the old binding"
        );
    }

    #[test]
    fn a_failed_launcher_set_rebinds_the_previous_one() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).unwrap();
        let (set, generation, _) = last(&s);
        s.hotkeys
            .on_event(applied(set, generation, Err("D-Bus is gone".into())));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), KONSOLE);
        let (set, _, pairs) = last(&s);
        assert_eq!(set, SetKind::Launcher);
        assert_eq!(pairs, [("launcher".to_string(), "Alt+Space".to_string())]);
        assert_eq!(applied_launcher(&s), Some(hk("alt+space")));
    }

    #[test]
    fn launcher_hotkey_taken_by_an_item_is_refused() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        let calls_before = s.calls.borrow().len();
        assert_eq!(
            request(&mut s, HotkeyTarget::Launcher, Some(hk("ctrl+alt+k"))).err(),
            Some(Refusal::TakenBy(id("apps/konsole")))
        );
        assert_eq!(s.calls.borrow().len(), calls_before);
        assert!(!s.hotkeys.table().pending);
    }

    #[test]
    fn a_launcher_hotkey_an_item_took_meanwhile_is_not_written() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).unwrap();
        let (set, generation, _) = last(&s);
        let edited = "[hotkeys]\n\"Super+Space\" = \"apps/konsole\"\n";
        std::fs::write(&s.file, edited).unwrap();
        let out = s
            .hotkeys
            .on_event(applied(set, generation, active(&["launcher"])));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), edited);
        assert_eq!(
            notices(&out),
            ["config.toml changed meanwhile; nothing was saved"]
        );
        // Bound back to what the file now says.
        assert_eq!(
            launcher_calls(&s).last().map(|(_, pairs)| pairs.clone()),
            Some(vec![("launcher".to_string(), "Alt+Space".to_string())])
        );
        assert_eq!(applied_launcher(&s), Some(hk("alt+space")));
        assert_eq!(
            last(&s),
            (
                SetKind::Items,
                s.hotkeys.latest[&SetKind::Items],
                vec![("apps/konsole".to_string(), "Super+Space".to_string())]
            ),
            "the item set follows the file too"
        );
    }

    fn items_bound(s: &Setup) -> Vec<(String, String)> {
        s.calls
            .borrow()
            .iter()
            .rev()
            .find(|(set, _, _)| *set == SetKind::Items)
            .map(|(_, _, pairs)| pairs.clone())
            .unwrap()
    }

    #[test]
    fn the_item_set_follows_the_file_after_a_launcher_change() {
        let refused = || {
            Ok(vec![(
                "launcher".to_string(),
                BindState::Refused {
                    reason: "used by krunner: Run".into(),
                },
            )])
        };
        type Answer = Result<Vec<(String, BindState)>, String>;
        let answers: [fn() -> Answer; 3] = [
            || active(&["launcher"]),
            refused,
            || Err("D-Bus is gone".into()),
        ];
        for answer in answers {
            let mut s = setup(KONSOLE);
            load(&mut s);
            request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).unwrap();
            let (set, generation, _) = last(&s);
            std::fs::write(&s.file, "[hotkeys]\n\"Ctrl+Alt+J\" = \"apps/konsole\"\n").unwrap();
            assert!(load(&mut s).is_empty(), "waits for the pending change");
            assert_eq!(
                items_bound(&s),
                [("apps/konsole".to_string(), "Ctrl+Alt+K".to_string())]
            );
            s.hotkeys.on_event(applied(set, generation, answer()));
            assert_eq!(
                items_bound(&s),
                [("apps/konsole".to_string(), "Ctrl+Alt+J".to_string())]
            );
            let text = std::fs::read_to_string(&s.file).unwrap();
            assert!(text.contains("\"Ctrl+Alt+J\" = \"apps/konsole\""), "{text}");
        }
    }

    #[test]
    fn a_launcher_hotkey_for_later_is_written_and_shown_as_after_restart() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).unwrap();
        let (set, generation, _) = last(&s);
        s.hotkeys.on_event(applied(
            set,
            generation,
            Ok(vec![("launcher".into(), BindState::Later)]),
        ));
        let text = std::fs::read_to_string(&s.file).unwrap();
        assert_eq!(
            Settings::parse(&text).unwrap().launcher_hotkey,
            hk("super+space"),
            "{text}"
        );
        assert_eq!(applied_launcher(&s), Some(hk("super+space")));
        assert_eq!(
            s.hotkeys.states.get(LAUNCHER_ID),
            Some(&HotkeyState::AfterRestart)
        );
        assert_eq!(
            s.hotkeys.table().launcher_state,
            Some(HotkeyState::AfterRestart)
        );
    }

    #[test]
    fn the_table_carries_the_launcher_state() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        assert_eq!(
            s.hotkeys.table().launcher_state,
            Some(HotkeyState::Active { trigger: None })
        );
        let (set, generation, _) = s
            .calls
            .borrow()
            .iter()
            .rev()
            .find(|(set, _, _)| *set == SetKind::Launcher)
            .cloned()
            .unwrap();
        s.hotkeys.on_event(applied(
            set,
            generation,
            Ok(vec![(
                "launcher".into(),
                BindState::Refused {
                    reason: "used by krunner: Run".into(),
                },
            )]),
        ));
        assert_eq!(
            s.hotkeys.table().launcher_state,
            Some(HotkeyState::Refused {
                reason: "used by krunner: Run".into()
            })
        );
        s.hotkeys.on_event(HotkeyEvent::Unavailable {
            reason: "TOMAKAST_NO_KEYBOARD=1".into(),
        });
        assert_eq!(s.hotkeys.table().launcher_state, Some(HotkeyState::Off));
    }

    #[test]
    fn a_launcher_request_while_an_item_change_is_pending_is_refused() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).unwrap();
        assert_eq!(
            request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).err(),
            Some(Refusal::Pending)
        );
    }

    #[test]
    fn an_item_request_while_a_launcher_change_is_pending_is_refused() {
        let mut s = setup(KONSOLE);
        load(&mut s);
        request(&mut s, HotkeyTarget::Launcher, Some(hk("super+space"))).unwrap();
        assert_eq!(
            request(&mut s, item("apps/kitty"), Some(hk("ctrl+alt+t"))).err(),
            Some(Refusal::Pending)
        );
    }

    #[test]
    fn item_notice_carries_its_origin_and_target() {
        for origin in [HotkeyOrigin::Launcher, HotkeyOrigin::Settings] {
            let mut s = setup(KONSOLE);
            load(&mut s);
            s.hotkeys
                .request(
                    &item("apps/kitty"),
                    Some(hk("ctrl+alt+t")),
                    origin,
                    &no_title,
                )
                .unwrap();
            let (set, generation, _) = last(&s);
            let out = s
                .hotkeys
                .on_event(applied(set, generation, Err("D-Bus is gone".into())));
            let origins: Vec<(HotkeyOrigin, &HotkeyTarget)> = out
                .iter()
                .filter_map(|o| match o {
                    Outcome::Notice { origin, target, .. } => Some((*origin, target)),
                    _ => None,
                })
                .collect();
            assert_eq!(origins, [(origin, &item("apps/kitty"))]);
        }
    }
}
