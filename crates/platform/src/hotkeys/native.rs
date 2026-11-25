//! Hotkeys the app registers itself through `global-hotkey`: Carbon `RegisterEventHotKey` on
//! macOS, `RegisterHotKey` on Windows, `XGrabKey` on X11.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tomakast_core::Hotkey;
use tomakast_core::settings::Modifiers as Mods;

use super::{BindState, EventSink, HotkeyBinding, HotkeyEvent, MODIFIER_ALONE, Registrar, SetKind};

/// The `global-hotkey` code of `key` (a config name, see `tomakast_core::keys`).
#[rustfmt::skip]
fn code(key: &str) -> Option<Code> {
    use Code::*;
    Some(match key {
        "a" => KeyA, "b" => KeyB, "c" => KeyC, "d" => KeyD, "e" => KeyE, "f" => KeyF,
        "g" => KeyG, "h" => KeyH, "i" => KeyI, "j" => KeyJ, "k" => KeyK, "l" => KeyL,
        "m" => KeyM, "n" => KeyN, "o" => KeyO, "p" => KeyP, "q" => KeyQ, "r" => KeyR,
        "s" => KeyS, "t" => KeyT, "u" => KeyU, "v" => KeyV, "w" => KeyW, "x" => KeyX,
        "y" => KeyY, "z" => KeyZ,
        "0" => Digit0, "1" => Digit1, "2" => Digit2, "3" => Digit3, "4" => Digit4,
        "5" => Digit5, "6" => Digit6, "7" => Digit7, "8" => Digit8, "9" => Digit9,
        "f1" => F1, "f2" => F2, "f3" => F3, "f4" => F4, "f5" => F5, "f6" => F6,
        "f7" => F7, "f8" => F8, "f9" => F9, "f10" => F10, "f11" => F11, "f12" => F12,
        "space" => Space, "enter" => Enter, "tab" => Tab, "backspace" => Backspace,
        "insert" => Insert, "delete" => Delete, "home" => Home, "end" => End,
        "left" => ArrowLeft, "up" => ArrowUp, "right" => ArrowRight, "down" => ArrowDown,
        "pageup" => PageUp, "pagedown" => PageDown,
        "-" => Minus, "=" => Equal, "[" => BracketLeft, "]" => BracketRight,
        "\\" => Backslash, ";" => Semicolon, "'" => Quote, "," => Comma, "." => Period,
        "/" => Slash, "`" => Backquote,
        "pause" => Pause, "scrolllock" => ScrollLock, "menu" => ContextMenu,
        "f13" => F13, "f14" => F14, "f15" => F15, "f16" => F16, "f17" => F17, "f18" => F18,
        "f19" => F19, "f20" => F20, "f21" => F21, "f22" => F22, "f23" => F23, "f24" => F24,
        _ => return None,
    })
}

/// `hotkey` for `global-hotkey`; Super (`META`) is Cmd on macOS and the Windows key on Windows.
fn native_hotkey(hotkey: &Hotkey) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    for (flag, native) in [
        (Mods::CTRL, Modifiers::CONTROL),
        (Mods::ALT, Modifiers::ALT),
        (Mods::SHIFT, Modifiers::SHIFT),
        (Mods::SUPER, Modifiers::META),
    ] {
        if hotkey.mods.contains(flag) {
            mods |= native;
        }
    }
    Some(HotKey::new(Some(mods), code(&hotkey.key)?))
}

/// The event of a press or release of the binding `id`.
fn event_of(state: HotKeyState, id: String) -> HotkeyEvent {
    match state {
        HotKeyState::Pressed => HotkeyEvent::Activated(id),
        HotKeyState::Released => HotkeyEvent::Released(id),
    }
}

/// What to register and unregister to go from `current` to `wanted` (id, hotkey).
#[derive(Debug, Default, PartialEq, Eq)]
struct Diff {
    register: Vec<(String, Hotkey)>,
    unregister: Vec<(String, Hotkey)>,
}

fn plan(current: &[(String, Hotkey)], wanted: &[(String, Hotkey)]) -> Diff {
    Diff {
        register: wanted
            .iter()
            .filter(|w| !current.contains(w))
            .cloned()
            .collect(),
        unregister: current
            .iter()
            .filter(|c| !wanted.contains(c))
            .cloned()
            .collect(),
    }
}

/// The OS side of `global-hotkey`, so the bookkeeping can be tested without a desktop.
trait OsHotkeys {
    /// Fails when the combination is registered already, by this process or another.
    fn register(&mut self, hotkey: HotKey) -> Result<(), String>;
    fn unregister(&mut self, hotkey: HotKey);
}

impl OsHotkeys for GlobalHotKeyManager {
    fn register(&mut self, hotkey: HotKey) -> Result<(), String> {
        GlobalHotKeyManager::register(self, hotkey).map_err(|e| e.to_string())
    }

    fn unregister(&mut self, hotkey: HotKey) {
        if let Err(e) = GlobalHotKeyManager::unregister(self, hotkey) {
            tracing::warn!("cannot unregister a hotkey: {e}");
        }
    }
}

pub(super) struct NativeBackend {
    os: Box<dyn OsHotkeys>,
    /// What is registered, per set: (id, hotkey).
    current: BTreeMap<SetKind, Vec<(String, Hotkey)>>,
    /// `global-hotkey` ids to binding ids, for presses.
    ids: Arc<Mutex<HashMap<u32, String>>>,
    sink: EventSink,
}

impl NativeBackend {
    pub(super) fn new(sink: EventSink) -> Result<NativeBackend, String> {
        let manager =
            GlobalHotKeyManager::new().map_err(|e| format!("cannot start global hotkeys: {e}"))?;
        let backend = NativeBackend::with_os(Box::new(manager), sink.clone());
        let (press_ids, press_sink) = (backend.ids.clone(), sink);
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            let id = press_ids
                .lock()
                .ok()
                .and_then(|ids| ids.get(&event.id()).cloned());
            if let Some(id) = id {
                press_sink(event_of(event.state(), id));
            }
        }));
        Ok(backend)
    }

    fn with_os(os: Box<dyn OsHotkeys>, sink: EventSink) -> NativeBackend {
        NativeBackend {
            os,
            current: BTreeMap::new(),
            ids: Arc::default(),
            sink,
        }
    }

    fn register(&mut self, id: &str, hotkey: &Hotkey) -> Result<(), String> {
        let native = native_hotkey(hotkey).ok_or_else(|| format!("{hotkey} cannot be a hotkey"))?;
        self.os
            .register(native)
            .map_err(|_| format!("another app holds {hotkey}"))?;
        if let Ok(mut ids) = self.ids.lock() {
            ids.insert(native.id(), id.to_string());
        }
        Ok(())
    }

    fn unregister(&mut self, hotkey: &Hotkey) {
        if let Some(native) = native_hotkey(hotkey) {
            self.os.unregister(native);
            if let Ok(mut ids) = self.ids.lock() {
                ids.remove(&native.id());
            }
        }
    }
}

impl Registrar for NativeBackend {
    /// Combinations that move to another id (or swap) are released first, since the OS
    /// refuses one this process still holds. Then what is new is registered; an id whose new
    /// combination fails keeps its old one. Then the rest of what is gone is released.
    /// Modifiers alone are refused: the OS APIs take modifiers plus one key.
    fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
        let current = self.current.remove(&set).unwrap_or_default();
        let mut refused: HashMap<String, String> = HashMap::new();
        let mut wanted: Vec<(String, Hotkey)> = Vec::new();
        for b in &bindings {
            if b.hotkey.is_modifiers_only() {
                refused.insert(b.id.clone(), MODIFIER_ALONE.into());
            } else {
                wanted.push((b.id.clone(), b.hotkey.clone()));
            }
        }
        let diff = plan(&current, &wanted);
        let moving: Vec<(String, Hotkey)> = diff
            .unregister
            .iter()
            .filter(|(_, hotkey)| diff.register.iter().any(|(_, h)| h == hotkey))
            .cloned()
            .collect();
        for (_, hotkey) in &moving {
            self.unregister(hotkey);
        }
        for (id, hotkey) in &diff.register {
            if let Err(reason) = self.register(id, hotkey) {
                refused.insert(id.clone(), reason);
            }
        }
        let mut now: Vec<(String, Hotkey)> = Vec::new();
        for (id, hotkey) in &current {
            let entry = (id.clone(), hotkey.clone());
            if !diff.unregister.contains(&entry) {
                now.push(entry);
            } else if moving.contains(&entry) {
                // Released already; an id that kept nothing new gets it back if it is free.
                let taken = now.iter().any(|(_, h)| h == hotkey)
                    || diff
                        .register
                        .iter()
                        .any(|(i, h)| h == hotkey && !refused.contains_key(i));
                if refused.contains_key(id) && !taken && self.register(id, hotkey).is_ok() {
                    now.push(entry);
                }
            } else if refused.contains_key(id) {
                now.push(entry);
            } else {
                self.unregister(hotkey);
            }
        }
        for (id, hotkey) in &diff.register {
            if !refused.contains_key(id) {
                now.push((id.clone(), hotkey.clone()));
            }
        }
        self.current.insert(set, now);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_core::Hotkey;
    use tomakast_core::keys::SUPPORTED_KEYS;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn pair(id: &str, s: &str) -> (String, Hotkey) {
        (id.into(), hk(s))
    }

    #[test]
    fn every_supported_key_has_a_native_code() {
        for key in SUPPORTED_KEYS {
            assert!(code(key).is_some(), "{key}");
        }
    }

    #[test]
    fn native_hotkeys_carry_the_modifiers() {
        let native = native_hotkey(&hk("ctrl+alt+shift+super+t")).unwrap();
        assert_eq!(
            native,
            global_hotkey::hotkey::HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::META),
                Code::KeyT
            )
        );
    }

    #[test]
    fn plan_registers_added_and_unregisters_removed() {
        let current = [pair("a", "ctrl+1")];
        let wanted = [pair("a", "ctrl+2"), pair("b", "ctrl+3")];
        assert_eq!(
            plan(&current, &wanted),
            Diff {
                register: vec![pair("a", "ctrl+2"), pair("b", "ctrl+3")],
                unregister: vec![pair("a", "ctrl+1")],
            }
        );
    }

    /// Refuses a combination already registered (by this process or by `foreign`), as the OS
    /// APIs do.
    #[derive(Default)]
    struct FakeOs {
        held: std::rc::Rc<std::cell::RefCell<Vec<HotKey>>>,
        foreign: Vec<HotKey>,
    }

    impl OsHotkeys for FakeOs {
        fn register(&mut self, hotkey: HotKey) -> Result<(), String> {
            let mut held = self.held.borrow_mut();
            if held.contains(&hotkey) || self.foreign.contains(&hotkey) {
                return Err("already registered".into());
            }
            held.push(hotkey);
            Ok(())
        }

        fn unregister(&mut self, hotkey: HotKey) {
            self.held.borrow_mut().retain(|h| *h != hotkey);
        }
    }

    fn binding(id: &str, s: &str) -> HotkeyBinding {
        HotkeyBinding {
            id: id.into(),
            description: id.into(),
            hotkey: hk(s),
        }
    }

    type Events = Arc<Mutex<Vec<HotkeyEvent>>>;

    fn backend(
        foreign: &[&str],
    ) -> (
        NativeBackend,
        std::rc::Rc<std::cell::RefCell<Vec<HotKey>>>,
        Events,
    ) {
        let os = FakeOs {
            foreign: foreign
                .iter()
                .map(|s| native_hotkey(&hk(s)).unwrap())
                .collect(),
            ..FakeOs::default()
        };
        let held = os.held.clone();
        let events: Events = Arc::default();
        let sink = events.clone();
        let backend = NativeBackend::with_os(
            Box::new(os),
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        );
        (backend, held, events)
    }

    fn owner(backend: &NativeBackend, s: &str) -> Option<String> {
        let native = native_hotkey(&hk(s)).unwrap();
        backend.ids.lock().unwrap().get(&native.id()).cloned()
    }

    fn last_states(events: &Events) -> Vec<(String, BindState)> {
        match events.lock().unwrap().last() {
            Some(HotkeyEvent::Applied {
                outcome: Ok(states),
                ..
            }) => states.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_combination_moves_to_another_item() {
        let (mut backend, held, events) = backend(&[]);
        backend.set(SetKind::Items, 1, vec![binding("konsole", "ctrl+alt+t")]);
        backend.set(SetKind::Items, 2, vec![binding("kitty", "ctrl+alt+t")]);
        assert_eq!(
            last_states(&events),
            [("kitty".to_string(), BindState::Active { trigger: None })]
        );
        assert_eq!(held.borrow().len(), 1);
        assert_eq!(owner(&backend, "ctrl+alt+t").as_deref(), Some("kitty"));
    }

    #[test]
    fn two_items_swap_their_combinations() {
        let (mut backend, held, events) = backend(&[]);
        backend.set(
            SetKind::Items,
            1,
            vec![binding("a", "ctrl+alt+1"), binding("b", "ctrl+alt+2")],
        );
        backend.set(
            SetKind::Items,
            2,
            vec![binding("a", "ctrl+alt+2"), binding("b", "ctrl+alt+1")],
        );
        assert!(
            last_states(&events)
                .iter()
                .all(|(_, s)| matches!(s, BindState::Active { .. }))
        );
        assert_eq!(held.borrow().len(), 2);
        assert_eq!(owner(&backend, "ctrl+alt+1").as_deref(), Some("b"));
        assert_eq!(owner(&backend, "ctrl+alt+2").as_deref(), Some("a"));
    }

    #[test]
    fn a_combination_another_app_holds_keeps_the_old_one() {
        let (mut backend, held, events) = backend(&["ctrl+alt+y"]);
        backend.set(SetKind::Items, 1, vec![binding("kitty", "ctrl+alt+t")]);
        backend.set(SetKind::Items, 2, vec![binding("kitty", "ctrl+alt+y")]);
        assert!(matches!(
            &last_states(&events)[0].1,
            BindState::Refused { .. }
        ));
        assert_eq!(owner(&backend, "ctrl+alt+t").as_deref(), Some("kitty"));
        assert_eq!(held.borrow().len(), 1);
    }

    #[test]
    fn a_release_is_forwarded() {
        assert_eq!(
            event_of(HotKeyState::Pressed, "leader".into()),
            HotkeyEvent::Activated("leader".into())
        );
        assert_eq!(
            event_of(HotKeyState::Released, "leader".into()),
            HotkeyEvent::Released("leader".into())
        );
    }

    #[test]
    fn a_modifier_alone_is_refused() {
        let (mut backend, held, events) = backend(&[]);
        backend.set(
            SetKind::Leader,
            1,
            vec![HotkeyBinding {
                id: "leader".into(),
                description: "Leader".into(),
                hotkey: Hotkey::parse_leader("Ctrl").unwrap(),
            }],
        );
        assert_eq!(
            last_states(&events),
            [(
                "leader".to_string(),
                BindState::Refused {
                    reason: "a modifier alone is not available on this system".into()
                }
            )]
        );
        assert!(held.borrow().is_empty());
    }

    #[test]
    fn plan_of_an_unchanged_set_is_empty() {
        let set = [pair("a", "ctrl+1")];
        assert_eq!(plan(&set, &set), Diff::default());
    }
}
