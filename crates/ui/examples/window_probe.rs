//! Task 9 window experiment: measures how fast the launcher window can be
//! shown, and how it looks, before the real UI is built.
//!
//! Findings live in `docs/experiments/2026-11-window-probe.md`.
//!
//! ```text
//! cargo run --release -p tomakast-ui --example window_probe -- [flags]
//!   --strategy recreate|persistent   default: recreate
//!   --prewarm                        open and immediately close a 1x1 window at startup
//!   --prewarm-frame                  like --prewarm, but close it after its first frame
//!   --root kit|plain|kit-transparent default: kit
//!   --cycles N                       default: 10
//!   --interval-ms MS                 default: 700 (time shown, then time hidden)
//!   --idle SECS                      after the cycles, stay hidden for SECS and count CPU time
//!   --with-input                     put an input field in the window (still no keyboard grab)
//!   --interactive                    user-only: keyboard window toggled by Alt+Space
//! ```
//!
//! Every non-interactive window uses `KeyboardInteractivity::None` on Wayland
//! and does not take focus on macOS and Windows. `--interactive` grabs the
//! keyboard (Exclusive on Wayland) unless `TOMAKAST_NO_KEYBOARD=1` is set.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

gpui_kit::actions!(window_probe, [Dismiss]);

const KEY_CONTEXT: &str = "WindowProbe";
const APP_ID: &str = "dev.tomakast.WindowProbe";
const WINDOW_SIZE: (f32, f32) = (720., 460.);
const CORNER_RADIUS: f32 = 14.;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Strategy {
    Recreate,
    Persistent,
}

/// How the window's root view is built.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RootKind {
    /// `gpui_kit::open_window`: content wrapped in the kit's Base Root.
    Kit,
    /// Bare `cx.open_window` with the probe view as the root.
    Plain,
    /// `gpui_kit::open_window`, then the Root's own background is set to
    /// transparent (the candidate fix for the white backdrop).
    KitTransparent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Prewarm {
    Off,
    /// Close in the same tick as the open.
    Immediate,
    /// Close after the window's first frame.
    AfterFrame,
}

#[derive(Clone, Debug)]
struct Args {
    strategy: Strategy,
    prewarm: Prewarm,
    root: RootKind,
    cycles: usize,
    interval: Duration,
    idle: Option<Duration>,
    with_input: bool,
    interactive: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Args {
            strategy: Strategy::Recreate,
            prewarm: Prewarm::Off,
            root: RootKind::Kit,
            cycles: 10,
            interval: Duration::from_millis(700),
            idle: None,
            with_input: false,
            interactive: false,
        };
        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
            match flag.as_str() {
                "--strategy" => {
                    args.strategy = match value("--strategy")?.as_str() {
                        "recreate" => Strategy::Recreate,
                        "persistent" => Strategy::Persistent,
                        other => return Err(format!("unknown strategy {other:?}")),
                    }
                }
                "--prewarm" => args.prewarm = Prewarm::Immediate,
                "--prewarm-frame" => args.prewarm = Prewarm::AfterFrame,
                "--root" => {
                    args.root = match value("--root")?.as_str() {
                        "kit" => RootKind::Kit,
                        "plain" => RootKind::Plain,
                        "kit-transparent" => RootKind::KitTransparent,
                        other => return Err(format!("unknown root {other:?}")),
                    }
                }
                "--cycles" => args.cycles = parse_num(&value("--cycles")?)?,
                "--interval-ms" => {
                    args.interval = Duration::from_millis(parse_num(&value("--interval-ms")?)?)
                }
                "--idle" => args.idle = Some(Duration::from_secs(parse_num(&value("--idle")?)?)),
                "--with-input" => args.with_input = true,
                "--interactive" => {
                    args.interactive = true;
                    args.with_input = true;
                }
                other => return Err(format!("unknown flag {other:?}")),
            }
        }
        Ok(args)
    }

    fn label(&self) -> String {
        format!(
            "strategy={:?} prewarm={:?} root={:?} input={} cycles={} interval={}ms",
            self.strategy,
            self.prewarm,
            self.root,
            self.with_input,
            self.cycles,
            self.interval.as_millis()
        )
    }
}

fn parse_num<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("not a number: {s:?}"))
}

// ---------------------------------------------------------------------------
// Timing

/// One show: time from the show request to the first `render` of the view,
/// and to the first `on_next_frame` callback registered in that render
/// (it runs when the platform asks for the frame after the rendered one,
/// i.e. once the first frame has been handed to the compositor).
#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    render: Option<Duration>,
    frame: Option<Duration>,
}

#[derive(Default)]
struct Timing {
    /// The show being measured: its index and request time.
    pending: Option<(usize, Instant)>,
    samples: Vec<Sample>,
}

type SharedTiming = Rc<RefCell<Timing>>;

fn start_show(timing: &SharedTiming) -> usize {
    let mut t = timing.borrow_mut();
    let ix = t.samples.len();
    t.samples.push(Sample::default());
    t.pending = Some((ix, Instant::now()));
    ix
}

/// Called from `render`: closes the "render" metric of a pending show and
/// arms the "frame" metric.
fn record_render(timing: &SharedTiming, window: &Window) {
    let Some((ix, requested)) = timing.borrow_mut().pending.take() else {
        return;
    };
    timing.borrow_mut().samples[ix].render = Some(requested.elapsed());
    let timing = timing.clone();
    window.on_next_frame(move |_, _| {
        let frame = requested.elapsed();
        timing.borrow_mut().samples[ix].frame = Some(frame);
        if let Some(on_frame) = ON_FRAME.with(|f| f.borrow_mut().take()) {
            on_frame(ix, frame);
        }
    });
}

type FrameHook = Box<dyn FnOnce(usize, Duration)>;

thread_local! {
    /// Optional hook run when a show's first frame is measured (interactive mode).
    static ON_FRAME: RefCell<Option<FrameHook>> = const { RefCell::new(None) };
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn fmt_ms(d: Option<Duration>) -> String {
    d.map_or_else(|| "-".into(), |d| format!("{:.1}", ms(d)))
}

/// `first`, median and p95 (nearest rank) of one metric.
fn summarize(
    values: &[Option<Duration>],
) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
    let first = values.first().copied().flatten();
    let mut rest: Vec<Duration> = values.iter().skip(1).flatten().copied().collect();
    rest.sort();
    if rest.is_empty() {
        return (first, None, None);
    }
    let median = rest[(rest.len() - 1) / 2];
    let p95_rank = ((rest.len() as f64) * 0.95).ceil() as usize;
    let p95 = rest[p95_rank.clamp(1, rest.len()) - 1];
    (first, Some(median), Some(p95))
}

fn mb(bytes: Option<u64>) -> String {
    bytes.map_or_else(
        || "-".into(),
        |b| format!("{:.0} MB", b as f64 / 1_048_576.0),
    )
}

fn print_report(args: &Args, samples: &[Sample], rss: &RssMarks) {
    let render: Vec<_> = samples.iter().map(|s| s.render).collect();
    let frame: Vec<_> = samples.iter().map(|s| s.frame).collect();
    let (rf, rm, rp) = summarize(&render);
    let (ff, fm, fp) = summarize(&frame);
    println!();
    println!("window_probe: {}", args.label());
    println!("per show (ms, request -> render / on_next_frame):");
    for (i, s) in samples.iter().enumerate() {
        println!("  #{i:<2} {:>7} {:>7}", fmt_ms(s.render), fmt_ms(s.frame));
    }
    println!("| metric (ms)    | first | median (warm) | p95 (warm) |");
    println!("|----------------|-------|---------------|------------|");
    println!(
        "| render         | {} | {} | {} |",
        fmt_ms(rf),
        fmt_ms(rm),
        fmt_ms(rp)
    );
    println!(
        "| on_next_frame  | {} | {} | {} |",
        fmt_ms(ff),
        fmt_ms(fm),
        fmt_ms(fp)
    );
    println!(
        "RSS: before first show {}, after first show {}, after last hide {}",
        mb(rss.before),
        mb(rss.after_first_show),
        mb(rss.after_last_hide)
    );
}

#[derive(Default)]
struct RssMarks {
    before: Option<u64>,
    after_first_show: Option<u64>,
    after_last_hide: Option<u64>,
}

// ---------------------------------------------------------------------------
// The view

struct ProbeView {
    timing: SharedTiming,
    shows: usize,
    input: Option<Entity<InputState>>,
}

impl Render for ProbeView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        record_render(&self.timing, window);
        let panel = div()
            .key_context(KEY_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_5()
            .rounded(px(CORNER_RADIUS))
            .overflow_hidden()
            // Semi-transparent content fill; everything outside the rounded
            // rectangle must stay fully transparent.
            .bg(hsla(0.62, 0.15, 0.10, 0.78))
            .border_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.18))
            .text_color(hsla(0.0, 0.0, 0.96, 1.0))
            .child(div().text_xl().child("tomakast window probe"))
            .child(format!("show #{}", self.shows));
        match &self.input {
            Some(input) => panel.child(Input::new(input)).child(
                div()
                    .text_sm()
                    .text_color(hsla(0.0, 0.0, 0.8, 1.0))
                    .child("Type to test input. Esc or Alt+Space hides; try clicking outside."),
            ),
            None => panel,
        }
    }
}

// ---------------------------------------------------------------------------
// Windows

fn window_options(size: Size<Pixels>, interactive: bool, cx: &App) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        focus: keyboard_allowed(interactive),
        show: true,
        kind: window_kind(interactive),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(APP_ID.into()),
        ..Default::default()
    }
}

fn keyboard_allowed(interactive: bool) -> bool {
    interactive && std::env::var_os("TOMAKAST_NO_KEYBOARD").is_none_or(|v| v != "1")
}

#[cfg(target_os = "linux")]
fn window_kind(interactive: bool) -> WindowKind {
    use gpui_kit::layer_shell::{KeyboardInteractivity, Layer, LayerShellOptions};
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return WindowKind::PopUp;
    }
    WindowKind::LayerShell(LayerShellOptions {
        namespace: "tomakast-window-probe".into(),
        layer: Layer::Overlay,
        keyboard_interactivity: if keyboard_allowed(interactive) {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        },
        ..Default::default()
    })
}

#[cfg(not(target_os = "linux"))]
fn window_kind(_interactive: bool) -> WindowKind {
    WindowKind::PopUp
}

fn open_probe_window(
    args: &Args,
    timing: &SharedTiming,
    cx: &mut App,
) -> Option<(AnyWindowHandle, Entity<ProbeView>)> {
    let (interactive, with_input) = (args.interactive, args.with_input);
    let opts = window_options(size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), interactive, cx);
    let timing = timing.clone();
    let build = move |window: &mut Window, cx: &mut App| {
        let input = with_input.then(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type here…"));
            input.update(cx, |s, cx| s.focus(window, cx));
            input
        });
        cx.new(|_| ProbeView {
            timing,
            shows: 1,
            input,
        })
    };
    let opened = match args.root {
        RootKind::Plain => {
            let mut view = None;
            cx.open_window(opts, |window, cx| {
                let v = build(window, cx);
                view = Some(v.clone());
                v
            })
            .map(|handle| {
                (
                    handle.into(),
                    view.expect("open_window ran its build closure"),
                )
            })
        }
        RootKind::Kit | RootKind::KitTransparent => gpui_kit::open_window(opts, cx, build),
    };
    let (handle, view) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("[probe] open_window failed: {e:?}");
            return None;
        }
    };
    if args.root == RootKind::KitTransparent {
        let _ = handle.update(cx, |_, window, cx| {
            if let Some(Some(root)) = window.root::<gpui_kit::base::Root>() {
                root.update(cx, |root, cx| {
                    root.style().background = Some(transparent_black().into());
                    cx.notify();
                });
            }
        });
    }
    Some((handle, view))
}

/// Fails (and logs) when called from inside an update of the same window.
fn remove_window(handle: AnyWindowHandle, cx: &mut App) -> bool {
    match handle.update(cx, |_, window, _| window.remove_window()) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[probe] remove_window failed: {e:?}");
            false
        }
    }
}

/// Opens a 1x1 window (never focused, no keyboard) and closes it.
fn prewarm(mode: Prewarm, cx: &mut App) {
    let opts = window_options(size(px(1.), px(1.)), false, cx);
    let started = Instant::now();
    let handle = match cx.open_window(opts, |_, cx| cx.new(|_| gpui_kit::Empty)) {
        Ok(handle) => AnyWindowHandle::from(handle),
        Err(e) => {
            eprintln!("[probe] prewarm open failed: {e:?}");
            return;
        }
    };
    match mode {
        Prewarm::Off => {}
        Prewarm::Immediate => {
            remove_window(handle, cx);
            eprintln!(
                "[probe] prewarm: opened and closed in {:.1} ms",
                ms(started.elapsed())
            );
        }
        Prewarm::AfterFrame => {
            let _ = handle.update(cx, |_, window, _| {
                window.on_next_frame(move |window, _| {
                    eprintln!(
                        "[probe] prewarm: first frame after {:.1} ms",
                        ms(started.elapsed())
                    );
                    window.remove_window();
                });
            });
        }
    }
}

/// The probe window and the strategy that shows and hides it.
struct Shower {
    args: Args,
    timing: SharedTiming,
    window: Option<(AnyWindowHandle, Entity<ProbeView>)>,
    visible: bool,
    shows: usize,
}

impl Shower {
    fn show(&mut self, cx: &mut App) {
        if self.visible {
            return;
        }
        self.shows += 1;
        start_show(&self.timing);
        match (self.args.strategy, &self.window) {
            (Strategy::Persistent, Some((handle, view))) => {
                let shows = self.shows;
                view.update(cx, |v, cx| {
                    v.shows = shows;
                    cx.notify();
                });
                let activate = keyboard_allowed(self.args.interactive);
                let _ = handle.update(cx, |_, window, _| {
                    os::show_native(window, activate);
                    window.refresh();
                });
            }
            _ => {
                self.window = open_probe_window(&self.args, &self.timing, cx);
                if let Some((_, view)) = &self.window {
                    let shows = self.shows;
                    view.update(cx, |v, _| v.shows = shows);
                }
            }
        }
        self.visible = self.window.is_some();
    }

    fn hide(&mut self, cx: &mut App) {
        if !self.visible {
            return;
        }
        let Some((handle, _)) = self.window.clone() else {
            self.visible = false;
            return;
        };
        // Keep the state unchanged if the window could not be reached, so a
        // later hide can retry instead of orphaning a visible window.
        let hidden = match self.args.strategy {
            Strategy::Recreate => remove_window(handle, cx),
            Strategy::Persistent => handle
                .update(cx, |_, window, _| os::hide_native(window))
                .inspect_err(|e| eprintln!("[probe] native hide failed: {e:?}"))
                .is_ok(),
        };
        if hidden {
            self.visible = false;
            if self.args.strategy == Strategy::Recreate {
                self.window = None;
            }
        }
    }

    fn toggle(&mut self, cx: &mut App) {
        if self.visible {
            self.hide(cx)
        } else {
            self.show(cx)
        }
    }
}

// ---------------------------------------------------------------------------
// Runs

fn run_cycles(args: Args, cx: &mut App) {
    let timing = SharedTiming::default();
    let shower = Rc::new(RefCell::new(Shower {
        args: args.clone(),
        timing: timing.clone(),
        window: None,
        visible: false,
        shows: 0,
    }));
    cx.spawn(async move |cx| {
        let timer = |d: Duration| cx.background_executor().timer(d);
        if args.prewarm != Prewarm::Off {
            cx.update(|cx| prewarm(args.prewarm, cx));
        }
        // Same gap before the first show with and without prewarm.
        timer(args.interval).await;
        let mut rss = RssMarks {
            before: os::rss_bytes(),
            ..Default::default()
        };
        for i in 0..args.cycles {
            cx.update(|cx| shower.borrow_mut().show(cx));
            timer(args.interval).await;
            if i == 0 {
                rss.after_first_show = os::rss_bytes();
            }
            cx.update(|cx| shower.borrow_mut().hide(cx));
            timer(args.interval).await;
        }
        rss.after_last_hide = os::rss_bytes();
        print_report(&args, &timing.borrow().samples, &rss);
        if let Some(idle) = args.idle {
            let (cpu0, wall0) = (os::cpu_time(), Instant::now());
            timer(idle).await;
            let (cpu1, wall) = (os::cpu_time(), wall0.elapsed());
            match (cpu0, cpu1) {
                (Some(a), Some(b)) => println!(
                    "idle {:.1} s with the window hidden: CPU {} {} ({:.1} ms), RSS {}",
                    wall.as_secs_f64(),
                    b.raw - a.raw,
                    b.unit,
                    ms(b.time.saturating_sub(a.time)),
                    mb(os::rss_bytes())
                ),
                _ => println!("idle: CPU time unavailable on this platform"),
            }
        }
        println!("[probe] done; the process stayed alive after the last window closed");
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn run_interactive(args: Args, cx: &mut App) {
    if !keyboard_allowed(true) {
        eprintln!("[probe] TOMAKAST_NO_KEYBOARD=1: the window will not take the keyboard");
    }
    cx.bind_keys([KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT))]);
    let timing = SharedTiming::default();
    let shower = Rc::new(RefCell::new(Shower {
        args,
        timing,
        window: None,
        visible: false,
        shows: 0,
    }));
    cx.on_action({
        let shower = shower.clone();
        move |_: &Dismiss, cx| {
            println!("[probe] Esc -> hide");
            // Actions run inside the window's update; hide after it returns.
            let shower = shower.clone();
            cx.defer(move |cx| shower.borrow_mut().hide(cx));
        }
    });

    let (tx, rx) = async_channel::unbounded::<HotkeyEvent>();
    hotkey::register(tx, cx);

    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            let received = Instant::now();
            println!("[probe] hotkey: {}", event.describe());
            cx.update(|cx| {
                let mut s = shower.borrow_mut();
                let was_visible = s.visible;
                s.toggle(cx);
                if was_visible || !s.visible {
                    return;
                }
                ON_FRAME.with(|f| {
                    *f.borrow_mut() = Some(Box::new(move |ix, frame| {
                        println!(
                            "[probe] show #{ix}: request -> on_next_frame {:.1} ms; hotkey received -> frame {:.1} ms",
                            ms(frame),
                            ms(received.elapsed())
                        );
                    }));
                });
                // A persistent window keeps its observer from the first show.
                if s.args.strategy == Strategy::Recreate || s.shows == 1 {
                    watch_deactivation(&s, shower.clone(), cx);
                }
                auto_hide(s.shows, shower.clone(), cx);
            });
        }
    })
    .detach();
    println!(
        "[probe] interactive: press Alt+Space to toggle the window, Esc hides it. \
         Each show auto-hides after {}s. Ctrl+C quits.",
        AUTO_HIDE.as_secs()
    );
}

/// Safety net for an Exclusive keyboard grab: hide a show that stays open too long.
const AUTO_HIDE: Duration = Duration::from_secs(30);

fn auto_hide(show: usize, shower: Rc<RefCell<Shower>>, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(AUTO_HIDE).await;
        cx.update(|cx| {
            let mut s = shower.borrow_mut();
            if s.visible && s.shows == show {
                println!("[probe] auto-hide after {}s", AUTO_HIDE.as_secs());
                s.hide(cx);
            }
        });
    })
    .detach();
}

/// Hides the window when the platform reports it lost activation (a click
/// outside, on platforms that report it).
fn watch_deactivation(shower: &Shower, shared: Rc<RefCell<Shower>>, cx: &mut App) {
    let Some((handle, view)) = shower.window.clone() else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        view.update(cx, |_, cx| {
            cx.observe_window_activation(window, move |_, window, cx| {
                let active = window.is_window_active();
                println!("[probe] window active = {active}");
                if !active {
                    let shared = shared.clone();
                    cx.defer(move |cx| shared.borrow_mut().hide(cx));
                }
            })
            .detach();
        });
    });
}

struct HotkeyEvent {
    /// Timestamp carried by the event, if the platform provides one.
    timestamp: Option<Duration>,
    detail: String,
}

impl HotkeyEvent {
    fn describe(&self) -> String {
        let clocks = os::clocks();
        match self.timestamp {
            Some(ts) => format!(
                "{} timestamp={} ms; now: monotonic={} ms, realtime={} ms",
                self.detail,
                ts.as_millis(),
                clocks.0.as_millis(),
                clocks.1.as_millis()
            ),
            None => format!("{} (no timestamp)", self.detail),
        }
    }
}

#[cfg(target_os = "linux")]
mod hotkey {
    use super::{APP_ID, HotkeyEvent};
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use gpui_kit::{App, AppContext as _};

    /// Temporary portal code modeled on Task 11.
    pub fn register(tx: async_channel::Sender<HotkeyEvent>, cx: &mut App) {
        cx.background_spawn(async move {
            if let Err(e) = run(tx).await {
                eprintln!("[probe] portal hotkey failed: {e}");
            }
        })
        .detach();
    }

    async fn run(tx: async_channel::Sender<HotkeyEvent>) -> ashpd::Result<()> {
        match ashpd::AppID::try_from(APP_ID) {
            Ok(id) => match ashpd::register_host_app(id).await {
                Ok(()) => println!("[probe] registered host app id {APP_ID}"),
                Err(e) => println!("[probe] host app registration failed ({e}); continuing"),
            },
            Err(e) => println!("[probe] bad app id: {e}"),
        }
        let portal = GlobalShortcuts::new().await?;
        println!(
            "[probe] GlobalShortcuts portal version {}",
            portal.version()
        );
        let session = portal.create_session(Default::default()).await?;
        let shortcut = NewShortcut::new("toggle", "Toggle the tomakast window probe")
            .preferred_trigger("ALT+space");
        let bound = portal
            .bind_shortcuts(&session, &[shortcut], None, Default::default())
            .await?
            .response()?;
        for s in bound.shortcuts() {
            println!(
                "[probe] bound {:?}: {:?} (trigger {:?})",
                s.id(),
                s.description(),
                s.trigger_description()
            );
        }
        let mut activated = portal.receive_activated().await?;
        while let Some(ev) = activated.next().await {
            let event = HotkeyEvent {
                timestamp: Some(ev.timestamp()),
                detail: format!("portal Activated {:?}", ev.shortcut_id()),
            };
            if tx.send(event).await.is_err() {
                break;
            }
        }
        drop(session);
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod hotkey {
    use super::HotkeyEvent;
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use gpui_kit::App;

    pub fn register(tx: async_channel::Sender<HotkeyEvent>, _cx: &mut App) {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => return eprintln!("[probe] global-hotkey manager failed: {e}"),
        };
        if let Err(e) = manager.register(HotKey::new(Some(Modifiers::ALT), Code::Space)) {
            return eprintln!("[probe] registering Alt+Space failed: {e}");
        }
        // The manager unregisters on drop; keep it for the life of the process.
        std::mem::forget(manager);
        GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
            if ev.state == HotKeyState::Pressed {
                let _ = tx.send_blocking(HotkeyEvent {
                    timestamp: None,
                    detail: "global-hotkey Alt+Space".into(),
                });
            }
        }));
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod hotkey {
    pub fn register(_: async_channel::Sender<super::HotkeyEvent>, _: &mut gpui_kit::App) {
        eprintln!("[probe] no global hotkey on this platform");
    }
}

// ---------------------------------------------------------------------------
// OS specifics: RSS, CPU time, clocks, native hide/show.

struct CpuTime {
    raw: u64,
    unit: &'static str,
    time: Duration,
}

#[cfg(target_os = "linux")]
mod os {
    use super::CpuTime;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub const PERSISTENT_SUPPORTED: bool = false;

    pub fn rss_bytes() -> Option<u64> {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        Some(pages * page_size())
    }

    fn page_size() -> u64 {
        // SAFETY: sysconf has no preconditions.
        (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }) as u64
    }

    /// utime + stime of the whole process, in clock ticks.
    pub fn cpu_time() -> Option<CpuTime> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        // Fields after the `(comm)`; utime and stime are fields 14 and 15.
        let after = &stat[stat.rfind(')')? + 2..];
        let fields: Vec<&str> = after.split_whitespace().collect();
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        // SAFETY: sysconf has no preconditions.
        let hz = (unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).max(1) as u64;
        let raw = utime + stime;
        Some(CpuTime {
            raw,
            unit: "ticks",
            time: Duration::from_millis(raw * 1000 / hz),
        })
    }

    /// (CLOCK_MONOTONIC, realtime since the Unix epoch).
    pub fn clocks() -> (Duration, Duration) {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: ts is a valid out-pointer.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        let mono = Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32);
        let real = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        (mono, real)
    }

    pub fn hide_native(_: &gpui_kit::Window) {}
    pub fn show_native(_: &gpui_kit::Window, _activate: bool) {}
}

#[cfg(target_os = "macos")]
mod os {
    use super::CpuTime;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    pub const PERSISTENT_SUPPORTED: bool = true;

    pub fn rss_bytes() -> Option<u64> {
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        // SAFETY: info is a valid buffer of `size` bytes.
        let n = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTASKINFO,
                0,
                (&mut info as *mut libc::proc_taskinfo).cast(),
                size,
            )
        };
        (n == size).then_some(info.pti_resident_size)
    }

    /// User + system time from getrusage, in microseconds.
    pub fn cpu_time() -> Option<CpuTime> {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: ru is a valid out-pointer.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
            return None;
        }
        let us = |t: libc::timeval| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64;
        let raw = us(ru.ru_utime) + us(ru.ru_stime);
        Some(CpuTime {
            raw,
            unit: "us",
            time: Duration::from_micros(raw),
        })
    }

    pub fn clocks() -> (Duration, Duration) {
        thread_local!(static START: Instant = Instant::now());
        let mono = START.with(|s| s.elapsed());
        let real = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        (mono, real)
    }

    fn ns_window(window: &gpui_kit::Window) -> Option<*mut AnyObject> {
        let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        let view = h.ns_view.as_ptr().cast::<AnyObject>();
        // SAFETY: ns_view is a live NSView owned by GPUI.
        let ns_window: *mut AnyObject = unsafe { msg_send![view, window] };
        (!ns_window.is_null()).then_some(ns_window)
    }

    /// `orderOut:` keeps the NSPanel alive but off screen.
    pub fn hide_native(window: &gpui_kit::Window) {
        if let Some(w) = ns_window(window) {
            // SAFETY: w is a live NSWindow; called on the main thread.
            let _: () = unsafe { msg_send![w, orderOut: std::ptr::null_mut::<AnyObject>()] };
        }
    }

    pub fn show_native(window: &gpui_kit::Window, activate: bool) {
        if let Some(w) = ns_window(window) {
            // SAFETY: w is a live NSWindow; called on the main thread.
            unsafe {
                if activate {
                    let _: () =
                        msg_send![w, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
                } else {
                    let _: () = msg_send![w, orderFrontRegardless];
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod os {
    use super::CpuTime;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::{FILETIME, HWND};
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SetForegroundWindow, ShowWindow,
    };

    pub const PERSISTENT_SUPPORTED: bool = true;

    pub fn rss_bytes() -> Option<u64> {
        let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: c is a valid buffer of `size` bytes.
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, size) };
        (ok != 0).then_some(c.WorkingSetSize as u64)
    }

    /// Kernel + user time from GetProcessTimes, in 100 ns units.
    pub fn cpu_time() -> Option<CpuTime> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: all out-pointers are valid.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if ok == 0 {
            return None;
        }
        let v = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
        let raw = v(kernel) + v(user);
        Some(CpuTime {
            raw,
            unit: "x100ns",
            time: Duration::from_nanos(raw * 100),
        })
    }

    pub fn clocks() -> (Duration, Duration) {
        thread_local!(static START: Instant = Instant::now());
        let mono = START.with(|s| s.elapsed());
        let real = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        (mono, real)
    }

    fn hwnd(window: &gpui_kit::Window) -> Option<HWND> {
        let RawWindowHandle::Win32(h) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        Some(h.hwnd.get() as HWND)
    }

    pub fn hide_native(window: &gpui_kit::Window) {
        if let Some(h) = hwnd(window) {
            // SAFETY: h is a live window owned by GPUI.
            unsafe { ShowWindow(h, SW_HIDE) };
        }
    }

    pub fn show_native(window: &gpui_kit::Window, activate: bool) {
        if let Some(h) = hwnd(window) {
            // SAFETY: h is a live window owned by GPUI.
            unsafe {
                if activate {
                    ShowWindow(h, SW_SHOW);
                    SetForegroundWindow(h);
                } else {
                    ShowWindow(h, SW_SHOWNOACTIVATE);
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod os {
    use super::CpuTime;
    use std::time::Duration;
    pub const PERSISTENT_SUPPORTED: bool = false;
    pub fn rss_bytes() -> Option<u64> {
        None
    }
    pub fn cpu_time() -> Option<CpuTime> {
        None
    }
    pub fn clocks() -> (Duration, Duration) {
        (Duration::ZERO, Duration::ZERO)
    }
    pub fn hide_native(_: &gpui_kit::Window) {}
    pub fn show_native(_: &gpui_kit::Window, _activate: bool) {}
}

// ---------------------------------------------------------------------------

fn main() {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("window_probe: {e}");
            std::process::exit(2);
        }
    };
    if args.strategy == Strategy::Persistent && !os::PERSISTENT_SUPPORTED {
        println!("unsupported");
        return;
    }
    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        cx.set_quit_mode(QuitMode::Explicit);
        if args.interactive {
            run_interactive(args, cx);
        } else {
            run_cycles(args, cx);
        }
    });
}
