//! KDE's own shortcut service, kglobalaccel, over D-Bus: the app sets, changes and removes its
//! shortcuts itself, silently (`docs/experiments/2026-11-item-hotkeys-portal-probe.md`).
//!
//! kglobalaccel does not refuse a combination another app holds (both would then hold it),
//! so every new combination is looked up first and refused with the holder's name. The
//! launcher hotkey is the user's own setting and is set anyway, with a warning.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;

use tomakast_core::Hotkey;
use tomakast_core::keys::qt_code;
use tomakast_core::settings::Modifiers;
use zbus::blocking::{Connection, MessageIterator};

use super::{BindState, EventSink, HotkeyBinding, HotkeyEvent, Registrar, SetKind};

const SERVICE: &str = "org.kde.kglobalaccel";
const PATH: &str = "/kglobalaccel";
const IFACE: &str = "org.kde.KGlobalAccel";
const COMPONENT_IFACE: &str = "org.kde.kglobalaccel.Component";
/// The component name of tomakast's shortcuts in KDE's settings.
pub(super) const COMPONENT: &str = "dev.tomakast.Tomakast";
const COMPONENT_NAME: &str = "tomakast";
/// `SetPresent | NoAutoloading`: the requested keys win over stored ones.
const SET_FLAGS: u32 = 6;

/// An action that holds a combination.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Holder {
    action: String,
    action_name: String,
    component: String,
    component_name: String,
}

/// The first holder outside `own_component`, as a reason to show.
fn holders_conflict(holders: &[Holder], own_component: &str) -> Option<String> {
    holders
        .iter()
        .find(|h| h.component != own_component)
        .map(|h| format!("used by {}: {}", h.component_name, h.action_name))
}

/// The bindings to set (new or changed) and the ids to unregister (gone).
fn plan(
    current: &BTreeMap<String, Hotkey>,
    wanted: &[HotkeyBinding],
) -> (Vec<HotkeyBinding>, Vec<String>) {
    let set = wanted
        .iter()
        .filter(|b| current.get(&b.id) != Some(&b.hotkey))
        .cloned()
        .collect();
    let unregister = current
        .keys()
        .filter(|id| !wanted.iter().any(|b| &b.id == *id))
        .cloned()
        .collect();
    (set, unregister)
}

/// Actions of the component that neither set wants: left over from an earlier run.
fn strays(names: &[String], wanted: &BTreeSet<String>) -> Vec<String> {
    names
        .iter()
        .filter(|n| !wanted.contains(*n))
        .cloned()
        .collect()
}

/// The D-Bus path of `component`'s object.
fn component_path(component: &str) -> String {
    let name: String = component
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("/component/{name}")
}

/// `hotkey` as kglobalaccel's key list. KDE names a shifted digit or symbol by the symbol,
/// without Shift (its own settings hold `Meta++`), so such a hotkey is set in both forms.
fn keys_of(hotkey: &Hotkey) -> Result<Vec<(Vec<i32>,)>, String> {
    let code = qt_code(hotkey).ok_or_else(|| format!("{hotkey} cannot be a hotkey"))?;
    let mut keys = Vec::new();
    if hotkey.mods.contains(Modifiers::SHIFT)
        && let Some(symbol) = tomakast_core::keys::shifted(&hotkey.key)
        && let Ok(symbol) = i32::try_from(u32::from(symbol.chars().next().unwrap_or(' ')))
    {
        let mods = code & !0x01ff_ffff & !0x0200_0000;
        keys.push((vec![mods | symbol, 0, 0, 0],));
    }
    keys.push((vec![code, 0, 0, 0],));
    Ok(keys)
}

/// Every action holding any form of `hotkey`.
fn holders_of(conn: &Connection, hotkey: &Hotkey) -> Result<Vec<Holder>, String> {
    type Info = (
        String,
        String,
        String,
        String,
        String,
        String,
        Vec<i32>,
        Vec<i32>,
    );
    let mut holders = Vec::new();
    for key in keys_of(hotkey)? {
        let reply = conn
            .call_method(
                Some(SERVICE),
                PATH,
                Some(IFACE),
                "globalShortcutsByKey",
                &(key, (0i32,)),
            )
            .map_err(|e| e.to_string())?;
        let infos: Vec<Info> = reply.body().deserialize().map_err(|e| e.to_string())?;
        holders.extend(infos.into_iter().map(
            |(action, action_name, component, component_name, ..)| Holder {
                action,
                action_name,
                component,
                component_name,
            },
        ));
    }
    Ok(holders)
}

/// Whether KDE's shortcut service runs on the session bus.
pub(super) fn available() -> bool {
    let Ok(conn) = Connection::session() else {
        return false;
    };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(SERVICE,),
    )
    .and_then(|reply| reply.body().deserialize::<bool>())
    .unwrap_or(false)
}

/// Unregisters every action of `component`.
pub(super) fn release(component: &str) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;
    let names: Vec<String> = match conn.call_method(
        Some(SERVICE),
        component_path(component).as_str(),
        Some(COMPONENT_IFACE),
        "shortcutNames",
        &(),
    ) {
        Ok(reply) => reply.body().deserialize().map_err(|e| e.to_string())?,
        // No component: nothing was ever registered.
        Err(_) => return Ok(()),
    };
    for id in strays(&names, &BTreeSet::new()) {
        conn.call_method(
            Some(SERVICE),
            PATH,
            Some(IFACE),
            "unregister",
            &(component, id.as_str()),
        )
        .map_err(|e| format!("kglobalaccel unregister: {e}"))?;
    }
    Ok(())
}

pub(super) struct KdeBackend {
    commands: mpsc::Sender<(SetKind, u64, Vec<HotkeyBinding>)>,
}

impl KdeBackend {
    /// Connects, listens for presses of `component`'s shortcuts, and starts the worker that
    /// applies sets.
    pub(super) fn start(component: &str, sink: EventSink) -> Result<KdeBackend, String> {
        let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;
        let rule = format!(
            "type='signal',interface='{COMPONENT_IFACE}',member='globalShortcutPressed',path='{}'",
            component_path(component)
        );
        let presses = MessageIterator::for_match_rule(rule.as_str(), &conn, Some(64))
            .map_err(|e| format!("listening to kglobalaccel: {e}"))?;
        let (own, press_sink) = (component.to_string(), sink.clone());
        std::thread::Builder::new()
            .name("tomakast-kde-presses".into())
            .spawn(move || {
                for message in presses.flatten() {
                    if let Ok((comp, action, _)) =
                        message.body().deserialize::<(String, String, i64)>()
                        && comp == own
                    {
                        press_sink(HotkeyEvent::Activated(action));
                    }
                }
                tracing::warn!("kglobalaccel press stream ended; hotkeys no longer reach tomakast");
            })
            .map_err(|e| format!("cannot start the hotkey thread: {e}"))?;
        let (commands, rx) = mpsc::channel();
        let mut worker = Worker {
            conn,
            component: component.to_string(),
            current: BTreeMap::new(),
            strays_removed: false,
            sink,
        };
        std::thread::Builder::new()
            .name("tomakast-kde-hotkeys".into())
            .spawn(move || {
                while let Ok((set, generation, bindings)) = rx.recv() {
                    worker.apply(set, generation, bindings);
                }
            })
            .map_err(|e| format!("cannot start the hotkey thread: {e}"))?;
        Ok(KdeBackend { commands })
    }
}

impl Registrar for KdeBackend {
    fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
        // Fails only when the worker is gone, which it logs.
        let _ = self.commands.send((set, generation, bindings));
    }
}

struct Worker {
    conn: Connection,
    component: String,
    /// What is set, per set: id → hotkey.
    current: BTreeMap<SetKind, BTreeMap<String, Hotkey>>,
    strays_removed: bool,
    sink: EventSink,
}

impl Worker {
    fn action_id(&self, id: &str, description: &str) -> Vec<String> {
        vec![
            self.component.clone(),
            id.to_string(),
            COMPONENT_NAME.to_string(),
            description.to_string(),
        ]
    }

    fn call<B>(&self, method: &str, body: &B) -> Result<zbus::message::Message, String>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(SERVICE), PATH, Some(IFACE), method, body)
            .map_err(|e| format!("kglobalaccel {method}: {e}"))
    }

    /// Sets `binding`'s keys; `Err` is the reason it is not bound.
    fn bind(&self, set: SetKind, binding: &HotkeyBinding) -> Result<(), String> {
        let holders = holders_of(&self.conn, &binding.hotkey)?;
        if let Some(reason) = holders_conflict(&holders, &self.component) {
            if set == SetKind::Items {
                return Err(reason);
            }
            tracing::warn!(hotkey = %binding.hotkey, "launcher hotkey is also {reason}");
        }
        let id = self.action_id(&binding.id, &binding.description);
        self.call("doRegister", &(id.clone(),))?;
        let keys = keys_of(&binding.hotkey)?;
        let reply = self.call("setShortcutKeys", &(id, keys.clone(), SET_FLAGS))?;
        let mut set_keys: Vec<(Vec<i32>,)> =
            reply.body().deserialize().map_err(|e| e.to_string())?;
        let mut asked = keys.clone();
        // KDE may list the forms in another order.
        set_keys.sort();
        asked.sort();
        if set_keys != asked {
            return Err("KDE kept another key".into());
        }
        Ok(())
    }

    fn unbind(&self, id: &str) {
        if let Err(e) = self.call("unregister", &(self.component.as_str(), id)) {
            tracing::warn!(id, "{e}");
        }
    }

    fn apply(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
        let mut current = self.current.remove(&set).unwrap_or_default();
        let (to_set, gone) = plan(&current, &bindings);
        let mut refused: BTreeMap<String, String> = BTreeMap::new();
        for binding in &to_set {
            match self.bind(set, binding) {
                Ok(()) => {
                    current.insert(binding.id.clone(), binding.hotkey.clone());
                }
                Err(reason) => {
                    refused.insert(binding.id.clone(), reason);
                }
            }
        }
        for id in gone {
            self.unbind(&id);
            current.remove(&id);
        }
        self.current.insert(set, current);
        if !self.strays_removed && self.current.len() == 2 {
            self.strays_removed = true;
            self.remove_strays();
        }
        let states = bindings
            .into_iter()
            .map(|b| {
                let state = match refused.remove(&b.id) {
                    Some(reason) => BindState::Refused { reason },
                    None => BindState::Active { trigger: None },
                };
                (b.id, state)
            })
            .collect();
        (self.sink)(HotkeyEvent::Applied {
            set,
            generation,
            outcome: Ok(states),
        });
    }

    /// Unregisters the component's actions that neither set holds.
    fn remove_strays(&self) {
        let names: Vec<String> = match self.conn.call_method(
            Some(SERVICE),
            component_path(&self.component).as_str(),
            Some(COMPONENT_IFACE),
            "shortcutNames",
            &(),
        ) {
            Ok(reply) => reply.body().deserialize().unwrap_or_default(),
            // No component yet: nothing was ever registered.
            Err(_) => return,
        };
        let wanted: BTreeSet<String> = self
            .current
            .values()
            .flat_map(|m| m.keys().cloned())
            .collect();
        for id in strays(&names, &wanted) {
            tracing::info!(id, "removing a stray hotkey from an earlier run");
            self.unbind(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use tomakast_core::Hotkey;

    fn holder(action: &str, action_name: &str, component: &str, component_name: &str) -> Holder {
        Holder {
            action: action.into(),
            action_name: action_name.into(),
            component: component.into(),
            component_name: component_name.into(),
        }
    }

    fn binding(id: &str, s: &str) -> HotkeyBinding {
        HotkeyBinding {
            id: id.into(),
            description: id.into(),
            hotkey: Hotkey::parse(s).unwrap(),
        }
    }

    #[test]
    fn a_combination_held_by_another_app_is_refused_with_its_name() {
        let holders = [holder("_launch", "Launch", "kitty.desktop", "kitty")];
        assert_eq!(
            holders_conflict(&holders, "dev.tomakast.Tomakast"),
            Some("used by kitty: Launch".to_string())
        );
    }

    #[test]
    fn our_own_actions_are_not_a_conflict() {
        let holders = [holder(
            "apps/kitty",
            "Kitty",
            "dev.tomakast.Tomakast",
            "tomakast",
        )];
        assert_eq!(holders_conflict(&holders, "dev.tomakast.Tomakast"), None);
    }

    #[test]
    fn plan_sets_changed_and_unregisters_dropped() {
        let current: BTreeMap<String, Hotkey> = [
            ("a".to_string(), Hotkey::parse("ctrl+alt+1").unwrap()),
            ("b".to_string(), Hotkey::parse("ctrl+alt+2").unwrap()),
        ]
        .into();
        let wanted = [binding("a", "ctrl+alt+1"), binding("c", "ctrl+alt+3")];
        let (set, unregister) = plan(&current, &wanted);
        assert_eq!(set, [binding("c", "ctrl+alt+3")]);
        assert_eq!(unregister, ["b"]);
    }

    #[test]
    fn shift_with_a_digit_is_also_registered_as_its_symbol() {
        // KDE stores shifted symbols without Shift (`Meta++`), so both forms are set.
        let ctrl = 0x0400_0000;
        let shift = 0x0200_0000;
        assert_eq!(
            keys_of(&Hotkey::parse("ctrl+shift+1").unwrap()).unwrap(),
            [
                (vec![ctrl | 0x21, 0, 0, 0],),
                (vec![ctrl | shift | 0x31, 0, 0, 0],)
            ]
        );
        assert_eq!(
            keys_of(&Hotkey::parse("ctrl+shift+t").unwrap()).unwrap(),
            [(vec![ctrl | shift | 0x54, 0, 0, 0],)]
        );
    }

    #[test]
    fn strays_are_actions_in_neither_set() {
        let names = [
            "launcher".to_string(),
            "apps/old".to_string(),
            "apps/kitty".to_string(),
        ];
        let wanted: BTreeSet<String> = ["launcher".to_string(), "apps/kitty".to_string()].into();
        assert_eq!(strays(&names, &wanted), ["apps/old"]);
    }

    /// Run by hand on a KDE session: `cargo test -p tomakast-platform -- --ignored kde_live`.
    /// Uses its own component and removes what it made; presses come from `invokeShortcut`.
    #[test]
    #[ignore]
    fn kde_live() {
        use std::sync::mpsc;
        use std::time::Duration;
        let component = "dev.tomakast.KdeLiveTest";
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let sink: EventSink = std::sync::Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        let mut backend = KdeBackend::start(component, sink).expect("kglobalaccel");
        let next = |rx: &mpsc::Receiver<HotkeyEvent>| {
            rx.recv_timeout(Duration::from_secs(3)).expect("an event")
        };
        backend.set(SetKind::Launcher, 1, vec![]);
        assert!(matches!(
            next(&rx),
            HotkeyEvent::Applied {
                generation: 1,
                outcome: Ok(_),
                ..
            }
        ));
        backend.set(SetKind::Items, 2, vec![binding("t1", "ctrl+alt+shift+k")]);
        assert_eq!(
            next(&rx),
            HotkeyEvent::Applied {
                set: SetKind::Items,
                generation: 2,
                outcome: Ok(vec![("t1".into(), BindState::Active { trigger: None })]),
            }
        );
        let conn = zbus::blocking::Connection::session().unwrap();
        conn.call_method(
            Some(SERVICE),
            component_path(component).as_str(),
            Some(COMPONENT_IFACE),
            "invokeShortcut",
            &("t1",),
        )
        .unwrap();
        assert_eq!(next(&rx), HotkeyEvent::Activated("t1".into()));
        // A shifted digit is set in both of KDE's forms.
        backend.set(
            SetKind::Items,
            5,
            vec![
                binding("t1", "ctrl+alt+shift+k"),
                binding("t3", "ctrl+alt+shift+2"),
            ],
        );
        match next(&rx) {
            HotkeyEvent::Applied {
                outcome: Ok(states),
                ..
            } => assert!(
                states
                    .iter()
                    .all(|(_, s)| matches!(s, BindState::Active { .. })),
                "{states:?}"
            ),
            other => panic!("{other:?}"),
        }
        // A second component asking the same keys is refused with our name.
        let (tx2, rx2) = mpsc::channel();
        let tx2 = std::sync::Mutex::new(tx2);
        let mut other = KdeBackend::start(
            "dev.tomakast.KdeLiveTest2",
            std::sync::Arc::new(move |e| {
                let _ = tx2.lock().unwrap().send(e);
            }),
        )
        .unwrap();
        other.set(SetKind::Items, 3, vec![binding("t2", "ctrl+alt+shift+k")]);
        match next(&rx2) {
            HotkeyEvent::Applied {
                outcome: Ok(states),
                ..
            } => {
                assert!(
                    matches!(&states[0].1, BindState::Refused { .. }),
                    "{states:?}"
                )
            }
            other => panic!("{other:?}"),
        }
        backend.set(SetKind::Items, 4, vec![]);
        assert!(matches!(
            next(&rx),
            HotkeyEvent::Applied { generation: 4, .. }
        ));
        let holders = holders_of(&conn, &Hotkey::parse("ctrl+alt+shift+k").unwrap()).unwrap();
        assert!(
            holders.iter().all(|h| h.component != component),
            "{holders:?}"
        );
    }
}
