//! The leader key: a key the user holds (or taps) so the next key runs that key's hotkey.

use std::time::{Duration, Instant};

use crate::ItemId;
use crate::settings::{Hotkey, Modifiers};

/// The leader window closes by itself after this long without a key event.
pub const SAFETY_CLOSE: Duration = Duration::from_secs(5);

/// How the leader key is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeaderMode {
    /// Hold the leader and press a key.
    #[default]
    Hold,
    /// Press the leader, release it, then press a key.
    Tap,
}

impl LeaderMode {
    /// The value in `config.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            LeaderMode::Hold => "hold",
            LeaderMode::Tap => "tap",
        }
    }
}

/// What the leader machine is told.
#[derive(Clone, Debug)]
pub enum LeaderInput {
    /// The leader's global shortcut fired (a press; in tap mode with a modifier leader, a tap).
    Pressed,
    Released,
    /// A key-down in the leader window, already in `Leader+…` form (see [`window_key`]);
    /// `None`: a key that names no supported key. `repeat`: auto-repeat.
    Key {
        hotkey: Option<Hotkey>,
        repeat: bool,
    },
    Escape,
    FocusLost,
    /// Time passed; the driver sends it at [`LeaderMachine::deadline`].
    Tick,
}

/// What the driver must do.
#[derive(Debug, PartialEq, Eq)]
pub enum LeaderEffect {
    Open,
    Close,
    Run(ItemId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// Hold mode: the leader is down and the window is open.
    Held,
    /// Tap mode: the window is open and waits for one key.
    Armed,
}

/// The leader's state machine; pure, the daemon drives it.
pub struct LeaderMachine {
    mode: LeaderMode,
    state: State,
    last_event: Instant,
}

impl LeaderMachine {
    pub fn new(mode: LeaderMode) -> LeaderMachine {
        LeaderMachine {
            mode,
            state: State::Idle,
            last_event: Instant::now(),
        }
    }

    /// Changes the mode. An open state goes idle without an effect: the driver closes the window.
    pub fn set_mode(&mut self, mode: LeaderMode) {
        self.mode = mode;
        self.state = State::Idle;
    }

    /// `bound`: the item of a `Leader+…` hotkey, if any.
    pub fn on(
        &mut self,
        input: LeaderInput,
        now: Instant,
        bound: &dyn Fn(&Hotkey) -> Option<ItemId>,
    ) -> Vec<LeaderEffect> {
        if !matches!(input, LeaderInput::Tick) {
            self.last_event = now;
        }
        match (self.state, input) {
            (State::Idle, LeaderInput::Pressed) => {
                self.state = match self.mode {
                    LeaderMode::Hold => State::Held,
                    LeaderMode::Tap => State::Armed,
                };
                vec![LeaderEffect::Open]
            }
            (State::Held, LeaderInput::Released) => self.close(),
            (State::Armed, LeaderInput::Pressed) => self.close(),
            (State::Held | State::Armed, LeaderInput::Escape | LeaderInput::FocusLost) => {
                self.close()
            }
            (State::Held | State::Armed, LeaderInput::Tick) => {
                if now >= self.last_event + SAFETY_CLOSE {
                    self.close()
                } else {
                    Vec::new()
                }
            }
            (_, LeaderInput::Key { repeat: true, .. }) => Vec::new(),
            (State::Held, LeaderInput::Key { hotkey, .. }) => hotkey
                .as_ref()
                .and_then(bound)
                .map(LeaderEffect::Run)
                .into_iter()
                .collect(),
            (State::Armed, LeaderInput::Key { hotkey, .. }) => {
                let mut effects: Vec<LeaderEffect> = hotkey
                    .as_ref()
                    .and_then(bound)
                    .map(LeaderEffect::Run)
                    .into_iter()
                    .collect();
                effects.extend(self.close());
                effects
            }
            _ => Vec::new(),
        }
    }

    fn close(&mut self) -> Vec<LeaderEffect> {
        self.state = State::Idle;
        vec![LeaderEffect::Close]
    }

    /// When to send [`LeaderInput::Tick`]; `None` while idle.
    pub fn deadline(&self) -> Option<Instant> {
        self.is_open().then(|| self.last_event + SAFETY_CLOSE)
    }

    pub fn is_open(&self) -> bool {
        self.state != State::Idle
    }
}

/// A window key-down as a `Leader+…` hotkey: the leader's own modifiers taken off, the key mapped to
/// its US name ([`crate::keys::gpui_alias`], then [`crate::keys::us_key`] when it is not supported),
/// `LEADER` added. `None` when the key is not supported.
pub fn window_key(pressed: &Hotkey, leader: &Hotkey) -> Option<Hotkey> {
    let alias = crate::keys::gpui_alias(&pressed.key);
    let (key, shift) = if crate::keys::is_supported(alias) {
        (alias.to_owned(), false)
    } else {
        crate::keys::us_key_shifted(alias)?
    };
    let mut mods = (pressed.mods - leader.mods) | Modifiers::LEADER;
    if shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Hotkey { mods, key })
}

/// The recorders' "leader down" state.
#[derive(Default)]
pub struct LeaderRecording {
    down: bool,
}

impl LeaderRecording {
    pub fn pressed(&mut self) {
        self.down = true;
    }

    /// Hold: the leader is up. Tap: it stays down until a key.
    pub fn released(&mut self, mode: LeaderMode) {
        if mode == LeaderMode::Hold {
            self.down = false;
        }
    }

    /// The hotkey a recorder shows for a key-down `pressed`. If the leader is down, it is
    /// [`window_key`] (and in tap mode the state goes up). With a modifiers-only leader in hold
    /// mode, a press whose mods are exactly the leader's becomes `Leader+<key>`. Otherwise
    /// `pressed` unchanged.
    pub fn key(&mut self, pressed: &Hotkey, leader: Option<&Hotkey>, mode: LeaderMode) -> Hotkey {
        let Some(leader) = leader else {
            return pressed.clone();
        };
        let held_modifiers =
            mode == LeaderMode::Hold && leader.is_modifiers_only() && pressed.mods == leader.mods;
        if !self.down && !held_modifiers {
            return pressed.clone();
        }
        if mode == LeaderMode::Tap {
            self.down = false;
        }
        window_key(pressed, leader).unwrap_or_else(|| pressed.clone())
    }

    pub fn is_down(&self) -> bool {
        self.down
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

    fn b(h: &Hotkey) -> Option<ItemId> {
        (*h == hk("leader+f")).then(|| id("apps/ff"))
    }

    fn key(s: &str) -> LeaderInput {
        LeaderInput::Key {
            hotkey: Some(hk(s)),
            repeat: false,
        }
    }

    #[test]
    fn hold_runs_each_key_until_release() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        assert_eq!(m.on(LeaderInput::Pressed, t0, &b), [LeaderEffect::Open]);
        assert_eq!(
            m.on(key("leader+f"), t0, &b),
            [LeaderEffect::Run(id("apps/ff"))]
        );
        assert_eq!(
            m.on(key("leader+f"), t0, &b),
            [LeaderEffect::Run(id("apps/ff"))]
        );
        assert_eq!(m.on(key("leader+x"), t0, &b), []);
        assert_eq!(m.on(LeaderInput::Released, t0, &b), [LeaderEffect::Close]);
        assert!(!m.is_open());
    }

    #[test]
    fn auto_repeat_runs_once() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(
            m.on(key("leader+f"), t0, &b),
            [LeaderEffect::Run(id("apps/ff"))]
        );
        let repeat = LeaderInput::Key {
            hotkey: Some(hk("leader+f")),
            repeat: true,
        };
        assert_eq!(m.on(repeat, t0, &b), []);
    }

    #[test]
    fn leader_repeat_while_held_is_ignored() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(m.on(LeaderInput::Pressed, t0, &b), []);
        assert!(m.is_open());
    }

    #[test]
    fn release_before_open_closes_at_once() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        assert_eq!(m.on(LeaderInput::Pressed, t0, &b), [LeaderEffect::Open]);
        assert_eq!(m.on(LeaderInput::Released, t0, &b), [LeaderEffect::Close]);
    }

    #[test]
    fn tap_runs_one_key_and_closes() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Tap);
        assert_eq!(m.on(LeaderInput::Pressed, t0, &b), [LeaderEffect::Open]);
        assert_eq!(m.on(LeaderInput::Released, t0, &b), []);
        assert!(m.is_open());
        assert_eq!(
            m.on(key("leader+f"), t0, &b),
            [LeaderEffect::Run(id("apps/ff")), LeaderEffect::Close]
        );
        assert!(!m.is_open());
    }

    #[test]
    fn tap_unknown_key_or_escape_closes() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Tap);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(m.on(key("leader+x"), t0, &b), [LeaderEffect::Close]);
        m.on(LeaderInput::Pressed, t0, &b);
        let none = LeaderInput::Key {
            hotkey: None,
            repeat: false,
        };
        assert_eq!(m.on(none, t0, &b), [LeaderEffect::Close]);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(m.on(LeaderInput::Escape, t0, &b), [LeaderEffect::Close]);
        assert!(!m.is_open());
    }

    #[test]
    fn escape_closes_hold() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(m.on(LeaderInput::Escape, t0, &b), [LeaderEffect::Close]);
    }

    #[test]
    fn tap_leader_again_closes() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Tap);
        assert_eq!(m.on(LeaderInput::Pressed, t0, &b), [LeaderEffect::Open]);
        assert_eq!(m.on(LeaderInput::Pressed, t0, &b), [LeaderEffect::Close]);
    }

    #[test]
    fn safety_timeout_closes_held() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        assert_eq!(m.deadline(), None);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(m.deadline(), Some(t0 + SAFETY_CLOSE));
        assert_eq!(m.on(LeaderInput::Tick, t0 + Duration::from_secs(4), &b), []);
        assert_eq!(
            m.on(LeaderInput::Tick, t0 + Duration::from_secs(5), &b),
            [LeaderEffect::Close]
        );
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn safety_timeout_closes_armed() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Tap);
        m.on(LeaderInput::Pressed, t0, &b);
        assert_eq!(m.deadline(), Some(t0 + SAFETY_CLOSE));
        assert_eq!(m.on(LeaderInput::Tick, t0 + Duration::from_secs(4), &b), []);
        assert_eq!(
            m.on(LeaderInput::Tick, t0 + Duration::from_secs(5), &b),
            [LeaderEffect::Close]
        );
    }

    #[test]
    fn a_key_event_pushes_the_deadline() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        m.on(LeaderInput::Pressed, t0, &b);
        let t3 = t0 + Duration::from_secs(3);
        m.on(key("leader+x"), t3, &b);
        assert_eq!(m.deadline(), Some(t3 + SAFETY_CLOSE));
        assert_eq!(m.on(LeaderInput::Tick, t0 + Duration::from_secs(5), &b), []);
    }

    #[test]
    fn focus_lost_closes() {
        let t0 = Instant::now();
        for mode in [LeaderMode::Hold, LeaderMode::Tap] {
            let mut m = LeaderMachine::new(mode);
            m.on(LeaderInput::Pressed, t0, &b);
            assert_eq!(m.on(LeaderInput::FocusLost, t0, &b), [LeaderEffect::Close]);
            assert!(!m.is_open());
        }
    }

    #[test]
    fn set_mode_goes_idle() {
        let t0 = Instant::now();
        let mut m = LeaderMachine::new(LeaderMode::Hold);
        m.on(LeaderInput::Pressed, t0, &b);
        m.set_mode(LeaderMode::Tap);
        assert!(!m.is_open());
    }

    #[test]
    fn russian_layout_keys_map_to_us() {
        let l = Hotkey::parse_leader("F12").unwrap();
        assert_eq!(window_key(&hk("а"), &l), Some(hk("leader+f")));
        assert_eq!(window_key(&hk("shift+ф"), &l), Some(hk("leader+shift+a")));
        assert_eq!(window_key(&hk("."), &l), Some(hk("leader+.")));
        assert_eq!(
            window_key(&hk("ctrl+f"), &Hotkey::parse_leader("Ctrl+Space").unwrap()),
            Some(hk("leader+f"))
        );
        assert_eq!(window_key(&hk("€"), &l), None);
        assert_eq!(window_key(&hk("shift+№"), &l), Some(hk("leader+shift+3")));
        assert_eq!(window_key(&hk("№"), &l), Some(hk("leader+shift+3")));
    }

    #[test]
    fn recording_follows_the_mode() {
        let mut r = LeaderRecording::default();
        let l = Hotkey::parse_leader("F12").unwrap();
        r.pressed();
        assert_eq!(r.key(&hk("f"), Some(&l), LeaderMode::Hold), hk("leader+f"));
        r.released(LeaderMode::Hold);
        assert_eq!(
            r.key(&hk("ctrl+alt+t"), Some(&l), LeaderMode::Hold),
            hk("ctrl+alt+t")
        );
        r.pressed();
        r.released(LeaderMode::Tap);
        assert_eq!(r.key(&hk("f"), Some(&l), LeaderMode::Tap), hk("leader+f"));
        assert!(!r.is_down());
        let ctrl = Hotkey::parse_leader("Ctrl").unwrap();
        assert_eq!(
            r.key(&hk("ctrl+f"), Some(&ctrl), LeaderMode::Hold),
            hk("leader+f")
        );
        // Only modifiers exactly the leader's are the leader.
        assert_eq!(
            r.key(&hk("ctrl+shift+f"), Some(&ctrl), LeaderMode::Hold),
            hk("ctrl+shift+f")
        );
        assert_eq!(
            r.key(&hk("ctrl+alt+t"), Some(&ctrl), LeaderMode::Hold),
            hk("ctrl+alt+t")
        );
        assert_eq!(
            r.key(&hk("ctrl+f"), Some(&ctrl), LeaderMode::Tap),
            hk("ctrl+f")
        );
    }
}
