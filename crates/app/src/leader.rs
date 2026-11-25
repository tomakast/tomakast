//! The leader on the UI thread: runs the leader machine, opens and closes the invisible window,
//! and runs the `Leader+…` items the keys read there name.

use std::time::Instant;

use gpui_kit::{App, BorrowAppContext as _, Global, Task};
use tomakast_core::leader::{LeaderEffect, LeaderInput, LeaderMachine, LeaderMode, window_key};
use tomakast_core::{Hotkey, ItemId};
use tomakast_ui::{CaptureEvent, CaptureSink, LeaderWindow};

/// The window the driver opens and closes; the [`LeaderWindow`] global outside tests.
pub(crate) trait LeaderSurface {
    fn open(&mut self, cx: &mut App);
    fn close(&mut self, cx: &mut App);
}

/// The invisible leader window, reporting to `sink`.
pub(crate) struct SystemLeaderWindow {
    sink: CaptureSink,
}

impl SystemLeaderWindow {
    pub fn new(sink: CaptureSink) -> SystemLeaderWindow {
        SystemLeaderWindow { sink }
    }
}

impl LeaderSurface for SystemLeaderWindow {
    fn open(&mut self, cx: &mut App) {
        let sink = self.sink.clone();
        cx.update_global::<LeaderWindow, _>(|w, cx| w.open(sink, cx));
    }

    fn close(&mut self, cx: &mut App) {
        cx.update_global::<LeaderWindow, _>(|w, cx| w.close(cx));
    }
}

/// Global. The leader machine with the leader, the mode and the `Leader+…` items in force.
pub(crate) struct LeaderDriver {
    machine: LeaderMachine,
    leader: Option<Hotkey>,
    mode: LeaderMode,
    items: Vec<(Hotkey, ItemId)>,
    window: Box<dyn LeaderSurface>,
    /// Counts the timers; only the latest one's tick counts.
    generation: u64,
    /// The pending tick at the machine's deadline; dropped (cancelled) when replaced.
    timer: Option<Task<()>>,
}

impl Global for LeaderDriver {}

impl LeaderDriver {
    pub fn new(window: Box<dyn LeaderSurface>) -> LeaderDriver {
        LeaderDriver {
            machine: LeaderMachine::new(LeaderMode::default()),
            leader: None,
            mode: LeaderMode::default(),
            items: Vec::new(),
            window,
            generation: 0,
            timer: None,
        }
    }

    /// The leader in force; `None` when none is set.
    pub fn leader(&self) -> Option<&Hotkey> {
        self.leader.as_ref()
    }

    /// After each hotkey load: the leader, the effective mode and a snapshot of
    /// `Leader+…` → item. A new leader or mode closes an open window; the same ones leave it
    /// open (only the items change).
    pub fn configure(
        &mut self,
        leader: Option<Hotkey>,
        mode: LeaderMode,
        items: Vec<(Hotkey, ItemId)>,
        cx: &mut App,
    ) {
        self.items = items;
        if leader == self.leader && mode == self.mode {
            return;
        }
        self.leader = leader;
        self.mode = mode;
        let was_open = self.machine.is_open();
        self.machine.set_mode(mode);
        if was_open {
            self.window.close(cx);
        }
        self.cancel_tick();
    }

    /// Runs the machine and carries out its effects: opens or closes the window, runs an item
    /// through `run`. Then waits for the machine's deadline, if it has one.
    pub fn input(
        &mut self,
        input: LeaderInput,
        now: Instant,
        cx: &mut App,
        run: &dyn Fn(&ItemId, &mut App),
    ) {
        if !self.uses_window() {
            return;
        }
        let items = &self.items;
        let bound = |hotkey: &Hotkey| {
            items
                .iter()
                .find(|(h, _)| h == hotkey)
                .map(|(_, item)| item.clone())
        };
        for effect in self.machine.on(input, now, &bound) {
            match effect {
                LeaderEffect::Open => self.window.open(cx),
                LeaderEffect::Close => self.window.close(cx),
                LeaderEffect::Run(item) => run(&item, cx),
            }
        }
        self.cancel_tick();
        if let Some(deadline) = self.machine.deadline() {
            let generation = self.generation;
            let delay = deadline.saturating_duration_since(now);
            self.timer = Some(cx.spawn(async move |cx| {
                cx.background_executor().timer(delay).await;
                cx.update(|cx| tick(generation, deadline, cx));
            }));
        }
    }

    /// Whether the leader opens the window: a leader is set, and it is not modifiers only in
    /// hold mode (then its `Leader+…` hotkeys are system hotkeys of their own).
    fn uses_window(&self) -> bool {
        self.leader
            .as_ref()
            .is_some_and(|l| !l.is_modifiers_only() || self.mode == LeaderMode::Tap)
    }

    fn cancel_tick(&mut self) {
        self.generation += 1;
        self.timer = None;
    }
}

/// The timer of `generation`, set for `deadline`, is due: the machine is told, as of that
/// deadline, if that timer is still the latest.
fn tick(generation: u64, deadline: Instant, cx: &mut App) {
    if !cx.has_global::<LeaderDriver>() {
        return;
    }
    cx.update_global::<LeaderDriver, _>(|driver, cx| {
        if driver.generation == generation {
            // Time alone runs nothing.
            driver.input(LeaderInput::Tick, deadline, cx, &|_, _| {});
        }
    });
}

/// What the machine is told for a [`CaptureEvent`] of the leader window, under `leader`: a key
/// in `Leader+…` form, Esc, the loss of focus, and the key-up of the leader's own key as its
/// release (for when the system's release event is missed). Other key-ups mean nothing.
pub(crate) fn capture_input(event: CaptureEvent, leader: Option<&Hotkey>) -> Option<LeaderInput> {
    match event {
        CaptureEvent::Key { hotkey, repeat } => Some(LeaderInput::Key {
            hotkey: hotkey.and_then(|h| window_key(&h, leader?)),
            repeat,
        }),
        CaptureEvent::KeyUp { hotkey } => {
            let (up, leader) = (hotkey?, leader?);
            (!leader.is_modifiers_only() && up.key == leader.key).then_some(LeaderInput::Released)
        }
        CaptureEvent::Escape => Some(LeaderInput::Escape),
        CaptureEvent::FocusLost => Some(LeaderInput::FocusLost),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui_kit::TestAppContext;
    use tomakast_core::leader::SAFETY_CLOSE;
    use tomakast_core::settings::Modifiers;

    use super::*;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn leader(s: &str) -> Hotkey {
        Hotkey::parse_leader(s).unwrap()
    }

    fn id(s: &str) -> ItemId {
        ItemId::parse(s).unwrap()
    }

    /// Counts the window's opens and closes.
    #[derive(Clone, Default)]
    struct FakeWindow(Rc<RefCell<(u32, u32)>>);

    impl FakeWindow {
        fn opens(&self) -> u32 {
            self.0.borrow().0
        }

        fn closes(&self) -> u32 {
            self.0.borrow().1
        }
    }

    impl LeaderSurface for FakeWindow {
        fn open(&mut self, _: &mut App) {
            self.0.borrow_mut().0 += 1;
        }

        fn close(&mut self, _: &mut App) {
            self.0.borrow_mut().1 += 1;
        }
    }

    struct Setup {
        driver: LeaderDriver,
        window: FakeWindow,
        ran: Rc<RefCell<Vec<ItemId>>>,
        t0: Instant,
    }

    impl Setup {
        fn new(cx: &mut TestAppContext, leader_key: &str, mode: LeaderMode) -> Setup {
            let window = FakeWindow::default();
            let mut driver = LeaderDriver::new(Box::new(window.clone()));
            cx.update(|cx| {
                driver.configure(
                    Some(leader(leader_key)),
                    mode,
                    vec![
                        (hk("leader+f"), id("apps/firefox")),
                        (hk("leader+t"), id("apps/terminal")),
                    ],
                    cx,
                )
            });
            Setup {
                driver,
                window,
                ran: Rc::default(),
                t0: Instant::now(),
            }
        }

        fn input(&mut self, cx: &mut TestAppContext, input: LeaderInput, after: Duration) {
            let ran = self.ran.clone();
            let now = self.t0 + after;
            cx.update(|cx| {
                self.driver.input(input, now, cx, &|item, _| {
                    ran.borrow_mut().push(item.clone())
                })
            });
        }

        fn key(&mut self, cx: &mut TestAppContext, key: &str, after: Duration) {
            let hotkey = Some(hk(key));
            self.input(
                cx,
                LeaderInput::Key {
                    hotkey,
                    repeat: false,
                },
                after,
            );
        }

        fn ran(&self) -> Vec<ItemId> {
            self.ran.borrow().clone()
        }
    }

    const NOW: Duration = Duration::ZERO;

    #[gpui_kit::test]
    fn press_opens_key_runs_release_closes(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Hold);
        s.input(cx, LeaderInput::Pressed, NOW);
        assert_eq!((s.window.opens(), s.window.closes()), (1, 0));
        s.key(cx, "leader+f", Duration::from_millis(100));
        s.key(cx, "leader+t", Duration::from_millis(200));
        s.key(cx, "leader+x", Duration::from_millis(300));
        assert_eq!(s.ran(), [id("apps/firefox"), id("apps/terminal")]);
        assert_eq!(s.window.closes(), 0);
        s.input(cx, LeaderInput::Released, Duration::from_millis(400));
        assert_eq!((s.window.opens(), s.window.closes()), (1, 1));
        assert!(s.driver.timer.is_none());
    }

    #[gpui_kit::test]
    fn a_tick_after_five_seconds_closes(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Hold);
        s.input(cx, LeaderInput::Pressed, NOW);
        assert!(s.driver.timer.is_some());
        s.input(
            cx,
            LeaderInput::Tick,
            SAFETY_CLOSE - Duration::from_millis(1),
        );
        assert_eq!(s.window.closes(), 0);
        s.input(cx, LeaderInput::Tick, SAFETY_CLOSE);
        assert_eq!(s.window.closes(), 1);
        assert!(s.driver.timer.is_none());
    }

    /// The driver as the daemon holds it: a global, so its own timer can reach it.
    fn global_driver(cx: &mut TestAppContext, mode: LeaderMode) -> (FakeWindow, Instant) {
        let window = FakeWindow::default();
        let driver = LeaderDriver::new(Box::new(window.clone()));
        cx.update(|cx| {
            cx.set_global(driver);
            cx.update_global::<LeaderDriver, _>(|d, cx| {
                d.configure(
                    Some(leader("f12")),
                    mode,
                    vec![(hk("leader+f"), id("apps/firefox"))],
                    cx,
                )
            });
        });
        (window, Instant::now())
    }

    fn global_input(cx: &mut TestAppContext, input: LeaderInput, now: Instant) {
        cx.update(|cx| {
            cx.update_global::<LeaderDriver, _>(|d, cx| d.input(input, now, cx, &|_, _| {}))
        });
    }

    fn wait(cx: &mut TestAppContext, time: Duration) {
        cx.executor().advance_clock(time);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn the_driver_closes_by_itself_after_five_seconds(cx: &mut TestAppContext) {
        for mode in [LeaderMode::Hold, LeaderMode::Tap] {
            let (window, t0) = global_driver(cx, mode);
            global_input(cx, LeaderInput::Pressed, t0);
            assert_eq!(window.opens(), 1, "{mode:?}");
            wait(cx, SAFETY_CLOSE - Duration::from_millis(1));
            assert_eq!(window.closes(), 0, "{mode:?}");
            wait(cx, Duration::from_millis(1));
            assert_eq!(window.closes(), 1, "{mode:?}");
            wait(cx, SAFETY_CLOSE * 2);
            assert_eq!(window.closes(), 1, "{mode:?}");
        }
    }

    #[gpui_kit::test]
    fn a_key_event_pushes_the_close_back(cx: &mut TestAppContext) {
        let (window, t0) = global_driver(cx, LeaderMode::Hold);
        global_input(cx, LeaderInput::Pressed, t0);
        wait(cx, Duration::from_secs(3));
        let key = LeaderInput::Key {
            hotkey: Some(hk("leader+x")),
            repeat: false,
        };
        global_input(cx, key, t0 + Duration::from_secs(3));
        wait(cx, Duration::from_secs(2));
        assert_eq!(window.closes(), 0, "not at 5 s");
        wait(cx, Duration::from_secs(3) - Duration::from_millis(1));
        assert_eq!(window.closes(), 0);
        wait(cx, Duration::from_millis(1));
        assert_eq!(window.closes(), 1, "at 8 s");
    }

    #[gpui_kit::test]
    fn a_stale_timer_does_not_close_a_newer_hold(cx: &mut TestAppContext) {
        let (window, t0) = global_driver(cx, LeaderMode::Hold);
        global_input(cx, LeaderInput::Pressed, t0);
        let stale = cx.update(|cx| cx.global::<LeaderDriver>().generation);
        global_input(cx, LeaderInput::Released, t0 + Duration::from_millis(100));
        global_input(cx, LeaderInput::Pressed, t0 + Duration::from_millis(200));
        assert_eq!((window.opens(), window.closes()), (2, 1));
        // The old timer's tick, even long after its deadline, is not this hold's.
        cx.update(|cx| tick(stale, t0 + Duration::from_secs(60), cx));
        assert_eq!(window.closes(), 1);
    }

    #[gpui_kit::test]
    fn tap_mode_runs_one_key_and_closes(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Tap);
        s.input(cx, LeaderInput::Pressed, NOW);
        s.input(cx, LeaderInput::Released, Duration::from_millis(50));
        assert_eq!(s.window.closes(), 0);
        s.key(cx, "leader+f", Duration::from_millis(100));
        assert_eq!(s.ran(), [id("apps/firefox")]);
        assert_eq!((s.window.opens(), s.window.closes()), (1, 1));
    }

    #[gpui_kit::test]
    fn reconfigure_while_open_closes(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Hold);
        s.input(cx, LeaderInput::Pressed, NOW);
        cx.update(|cx| {
            s.driver
                .configure(Some(leader("f12")), LeaderMode::Tap, vec![], cx)
        });
        assert_eq!(s.window.closes(), 1);
        assert!(s.driver.timer.is_none());
        // Closed: the next press opens again, in the new mode.
        s.input(cx, LeaderInput::Pressed, Duration::from_secs(1));
        assert_eq!(s.window.opens(), 2);
        s.input(cx, LeaderInput::Released, Duration::from_secs(1));
        assert_eq!(s.window.closes(), 1);
    }

    #[gpui_kit::test]
    fn another_leader_while_open_closes(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Hold);
        s.input(cx, LeaderInput::Pressed, NOW);
        cx.update(|cx| {
            s.driver
                .configure(Some(leader("f11")), LeaderMode::Hold, vec![], cx)
        });
        assert_eq!(s.window.closes(), 1);
        assert_eq!(s.driver.leader(), Some(&leader("f11")));
    }

    #[gpui_kit::test]
    fn an_unchanged_reconfigure_keeps_the_window_open(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Hold);
        s.input(cx, LeaderInput::Pressed, NOW);
        // Another item set under the same leader and mode: the hold goes on, with the new items.
        cx.update(|cx| {
            s.driver.configure(
                Some(leader("f12")),
                LeaderMode::Hold,
                vec![(hk("leader+g"), id("apps/gimp"))],
                cx,
            )
        });
        assert_eq!(s.window.closes(), 0);
        s.key(cx, "leader+f", Duration::from_millis(100));
        s.key(cx, "leader+g", Duration::from_millis(200));
        assert_eq!(s.ran(), [id("apps/gimp")]);
        s.input(cx, LeaderInput::Released, Duration::from_millis(300));
        assert_eq!(s.window.closes(), 1);
    }

    #[gpui_kit::test]
    fn a_modifier_leader_in_hold_mode_opens_nothing(cx: &mut TestAppContext) {
        // Its `Leader+…` hotkeys are system hotkeys of their own: no window takes part.
        let mut s = Setup::new(cx, "ctrl", LeaderMode::Hold);
        s.input(cx, LeaderInput::Pressed, NOW);
        s.key(cx, "leader+f", Duration::from_millis(100));
        assert_eq!(s.window.opens(), 0);
        assert!(s.ran().is_empty());
        assert!(s.driver.timer.is_none());
    }

    #[gpui_kit::test]
    fn a_modifier_leader_in_tap_mode_opens_the_window(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "ctrl", LeaderMode::Tap);
        s.input(cx, LeaderInput::Pressed, NOW);
        assert_eq!(s.window.opens(), 1);
    }

    #[gpui_kit::test]
    fn no_leader_opens_nothing(cx: &mut TestAppContext) {
        let mut s = Setup::new(cx, "f12", LeaderMode::Hold);
        cx.update(|cx| s.driver.configure(None, LeaderMode::Hold, vec![], cx));
        s.input(cx, LeaderInput::Pressed, NOW);
        assert_eq!(s.window.opens(), 0);
    }

    #[test]
    fn a_window_key_becomes_a_leader_hotkey() {
        let input = capture_input(
            CaptureEvent::Key {
                hotkey: Some(hk("ctrl+f")),
                repeat: true,
            },
            Some(&leader("ctrl+space")),
        );
        assert!(matches!(
            input,
            Some(LeaderInput::Key { hotkey: Some(h), repeat: true }) if h == hk("leader+f")
        ));
        let input = capture_input(
            CaptureEvent::Key {
                hotkey: None,
                repeat: false,
            },
            Some(&leader("f12")),
        );
        assert!(matches!(
            input,
            Some(LeaderInput::Key {
                hotkey: None,
                repeat: false
            })
        ));
    }

    #[test]
    fn the_key_up_of_the_leader_key_is_its_release() {
        let up = |key: &str| CaptureEvent::KeyUp {
            hotkey: Some(hk(key)),
        };
        assert!(matches!(
            capture_input(up("f12"), Some(&leader("f12"))),
            Some(LeaderInput::Released)
        ));
        // The leader's key with modifiers changed meanwhile is still its key.
        assert!(matches!(
            capture_input(up("shift+space"), Some(&leader("ctrl+space"))),
            Some(LeaderInput::Released)
        ));
        // As the window reports them (lowercase GPUI key names) against leaders as typed in
        // the config file.
        for (key, leader_text) in [("f12", "F12"), ("space", "Ctrl+Space"), ("pause", "Pause")] {
            let up = CaptureEvent::KeyUp {
                hotkey: Some(Hotkey {
                    mods: Modifiers::empty(),
                    key: key.into(),
                }),
            };
            assert!(matches!(
                capture_input(up, Some(&leader(leader_text))),
                Some(LeaderInput::Released)
            ));
        }
        assert!(capture_input(up("f"), Some(&leader("f12"))).is_none());
        assert!(capture_input(up("escape"), Some(&leader("f12"))).is_none());
        assert!(
            capture_input(CaptureEvent::KeyUp { hotkey: None }, Some(&leader("f12"))).is_none()
        );
        // A modifiers-only leader has no key to come up.
        assert!(capture_input(up("f"), Some(&leader("ctrl"))).is_none());
        assert!(capture_input(up("f12"), None).is_none());
    }

    #[test]
    fn escape_and_focus_loss_pass_through() {
        assert!(matches!(
            capture_input(CaptureEvent::Escape, Some(&leader("f12"))),
            Some(LeaderInput::Escape)
        ));
        assert!(matches!(
            capture_input(CaptureEvent::FocusLost, Some(&leader("f12"))),
            Some(LeaderInput::FocusLost)
        ));
    }
}
