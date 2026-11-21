use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tomakast_core::store::Store;
use tomakast_core::strip::{StoppedWidget, VisibleTile, WidgetEnv};
use tomakast_core::testing::{
    FakeModule, SeenWidgetEvent, burn_cpu, late_render_millis, wait_event,
};
use tomakast_core::{
    Action, Align, Color, ColorToken, Core, CoreEvent, CoreUpdate, Edges, Effect, Frame,
    ImageFormat, Justify, Layout, LocalNow, Module, ModuleContext, Node, NodeKind, Point,
    PointerAt, PointerEvent, PointerKind, RankWeights, SearchAnswer, TextAlign, TextNode,
    TextSizes, TextStyle, ThemeColors, Toast, ToastKind, WidgetAnswer, WidgetContext, WidgetDecl,
    WidgetEvent, WidgetImage, WidgetKey, WidgetSettings, WidgetSize, WidgetTheme,
};

fn now() -> LocalNow {
    LocalNow::from_unix_with_offset(1_700_000_000, 0)
}

fn theme() -> WidgetTheme {
    WidgetTheme {
        dark: true,
        colors: ThemeColors {
            text: 0xffffffff,
            muted: 0x999999ff,
            panel: 0x202020ff,
            border: 0x404040ff,
            selected: 0x303060ff,
            error: 0xff4040ff,
            success: 0x40ff40ff,
        },
        ui_font: "Inter".into(),
        mono_font: "Mono".into(),
        sizes: TextSizes {
            title: 16.,
            value: 28.,
            body: 14.,
            caption: 12.,
        },
    }
}

fn env() -> WidgetEnv {
    WidgetEnv {
        quarter: (173.5, 120.),
        single: (355., 120.),
        double: (718., 120.),
        scale: 2.,
        theme: theme(),
    }
}

fn decl(id: &str) -> WidgetDecl {
    WidgetDecl {
        id: id.into(),
        title: format!("Widget {id}"),
        size: WidgetSize::Single,
    }
}

fn key() -> WidgetKey {
    WidgetKey::new("fake", "w")
}

/// A column holding one text node.
fn frame(text: &str) -> Frame {
    let layout = Layout {
        padding: Edges {
            top: 0.,
            right: 0.,
            bottom: 0.,
            left: 0.,
        },
        gap: 0.,
        align: Align::Start,
        justify: Justify::Start,
    };
    let node = |parent, kind| Node {
        id: String::new(),
        parent,
        grow: 0.,
        width: None,
        height: None,
        kind,
    };
    Frame {
        nodes: vec![
            node(None, NodeKind::Column(layout)),
            node(
                Some(0),
                NodeKind::Text(TextNode {
                    text: text.into(),
                    style: TextStyle::Body,
                    color: Color::Token(ColorToken::Text),
                    align: TextAlign::Start,
                    max_lines: 1,
                    mono: false,
                }),
            ),
        ],
        animate: false,
        actions: vec![],
        images: vec![],
    }
}

fn hello(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    Ok(frame("hello"))
}

fn failing(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    anyhow::bail!("cannot draw")
}

fn nothing() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

fn image(id: &str, bytes: &[u8]) -> WidgetImage {
    WidgetImage {
        id: id.into(),
        format: ImageFormat::Png,
        bytes: Arc::from(bytes),
    }
}

/// A module declaring widget `w`, which draws "hello".
fn module() -> FakeModule {
    FakeModule::new(vec![], nothing)
        .with_widgets(vec![decl("w")])
        .with_render(hello)
}

/// A started core over `m`, with the env set, past `m`'s start-up items.
fn core_with(m: impl Module) -> Core {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    core.register(Box::new(m));
    core.set_widget_env(env());
    core.start_modules();
    core
}

/// The event inside an instance stamp.
fn inner(ev: &CoreEvent) -> &CoreEvent {
    match ev {
        CoreEvent::Instance { event, .. } => inner(event),
        ev => ev,
    }
}

/// Applies events until one matches `wanted`; returns that event's update.
fn apply_until(core: &mut Core, wanted: impl Fn(&CoreEvent) -> bool) -> CoreUpdate {
    let rx = core.events();
    loop {
        let ev = wait_event(&rx);
        let hit = wanted(inner(&ev));
        let update = core.apply(ev, now());
        if hit {
            return update;
        }
    }
}

fn rendered(ev: &CoreEvent) -> bool {
    matches!(ev, CoreEvent::WidgetRendered { .. })
}

fn event_finished(ev: &CoreEvent) -> bool {
    matches!(ev, CoreEvent::WidgetEventFinished { .. })
}

/// Renders `w` at `at` and applies the reply.
fn render_and_apply(core: &mut Core, at: Instant) -> CoreUpdate {
    assert!(core.render_widget(&key(), at));
    apply_until(core, rendered)
}

#[test]
fn a_rendered_frame_is_stored_for_the_widget() {
    let m = module();
    let contexts = m.render_contexts.clone();
    let renders = m.renders.clone();
    let mut core = core_with(m);

    assert!(core.render_widget(&key(), Instant::now()));
    apply_until(&mut core, rendered);

    assert_eq!(core.widget_frame(&key()).as_deref(), Some(&frame("hello")));
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    let contexts = contexts.lock().unwrap();
    let (widget, ctx) = &contexts[0];
    assert_eq!(widget, "w");
    assert_eq!((ctx.width, ctx.height), (355., 120.));
    assert_eq!(ctx.scale, 2.);
    assert_eq!(ctx.millis_shown, 0);
    assert_eq!((ctx.query.as_deref(), ctx.payload.as_deref()), (None, None));
}

#[test]
fn each_size_is_rendered_at_its_own_width() {
    let sized = |id: &str, size| WidgetDecl { size, ..decl(id) };
    let m = FakeModule::new(vec![], nothing)
        .with_widgets(vec![
            sized("q", WidgetSize::Quarter),
            sized("s", WidgetSize::Single),
            sized("d", WidgetSize::Double),
        ])
        .with_render(hello);
    let contexts = m.render_contexts.clone();
    let mut core = core_with(m);

    for widget in ["q", "s", "d"] {
        assert!(core.render_widget(&WidgetKey::new("fake", widget), Instant::now()));
        apply_until(&mut core, rendered);
    }

    let contexts = contexts.lock().unwrap();
    let sizes: Vec<(&str, f32, f32)> = contexts
        .iter()
        .map(|(widget, ctx)| (widget.as_str(), ctx.width, ctx.height))
        .collect();
    assert_eq!(
        sizes,
        [("q", 173.5, 120.), ("s", 355., 120.), ("d", 718., 120.)]
    );
}

/// Draws "frame <n>", counting from 0.
fn numbered(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    static N: AtomicUsize = AtomicUsize::new(0);
    Ok(frame(&format!(
        "frame {}",
        N.fetch_add(1, Ordering::SeqCst)
    )))
}

#[test]
fn a_late_frame_is_dropped_and_the_last_one_kept() {
    let work = Arc::new(AtomicU64::new(0));
    let m = module()
        .with_render(numbered)
        .with_render_work(work.clone());
    let renders = m.renders.clone();
    let mut core = core_with(m);
    let t0 = Instant::now();

    render_and_apply(&mut core, t0);
    assert_eq!(
        core.widget_frame(&key()).as_deref(),
        Some(&frame("frame 0"))
    );

    work.store(late_render_millis(), Ordering::SeqCst);
    let update = render_and_apply(&mut core, t0 + Duration::from_millis(20));

    assert!(matches!(update, CoreUpdate::None), "{update:?}");
    assert_eq!(renders.load(Ordering::SeqCst), 2);
    assert_eq!(
        core.widget_frame(&key()).as_deref(),
        Some(&frame("frame 0"))
    );
}

#[test]
fn five_late_frames_in_a_row_stop_the_widget() {
    let work = Arc::new(AtomicU64::new(late_render_millis()));
    let mut core = core_with(module().with_render_work(work));
    let t0 = Instant::now();

    for i in 0..4 {
        let update = render_and_apply(&mut core, t0 + Duration::from_millis(20 * i));
        assert!(matches!(update, CoreUpdate::None), "{i}: {update:?}");
    }
    let update = render_and_apply(&mut core, t0 + Duration::from_millis(80));

    let CoreUpdate::WidgetStopped { widgets } = update else {
        panic!("not stopped: {update:?}");
    };
    assert_eq!(
        widgets,
        [StoppedWidget {
            title: "Widget w".into(),
            reason: "it took longer than 16 ms to draw 5 times in a row".into(),
        }]
    );
    assert!(!core.render_widget(&key(), t0 + Duration::from_secs(1)));
}

#[test]
fn five_failed_frames_in_a_row_stop_the_widget() {
    let mut core = core_with(module().with_render(failing));
    let t0 = Instant::now();

    let updates: Vec<CoreUpdate> = (0..5)
        .map(|i| render_and_apply(&mut core, t0 + Duration::from_millis(20 * i)))
        .collect();

    assert!(updates[..4].iter().all(|u| matches!(u, CoreUpdate::None)));
    let CoreUpdate::WidgetStopped { widgets } = &updates[4] else {
        panic!("not stopped: {:?}", updates[4]);
    };
    assert_eq!(widgets.len(), 1);
    assert_eq!(widgets[0].reason, "it failed to draw 5 times in a row");
    assert_eq!(core.widget_frame(&key()), None);
}

#[test]
fn an_on_time_frame_resets_the_late_count() {
    let work = Arc::new(AtomicU64::new(late_render_millis()));
    let mut core = core_with(module().with_render_work(work.clone()));
    let t0 = Instant::now();
    let mut at = 0;
    let mut render = |core: &mut Core| {
        at += 20;
        render_and_apply(core, t0 + Duration::from_millis(at))
    };

    for _ in 0..4 {
        render(&mut core);
    }
    work.store(0, Ordering::SeqCst);
    render(&mut core);
    work.store(late_render_millis(), Ordering::SeqCst);
    let updates: Vec<CoreUpdate> = (0..4).map(|_| render(&mut core)).collect();

    assert!(updates.iter().all(|u| matches!(u, CoreUpdate::None)));
    assert!(core.render_widget(&key(), t0 + Duration::from_secs(1)));
}

#[test]
fn renders_are_limited_to_60_per_second_per_tile() {
    let mut core = core_with(module());
    let t0 = Instant::now();

    assert!(core.render_widget(&key(), t0));
    assert!(!core.render_widget(&key(), t0 + Duration::from_millis(14)));
    assert!(core.render_widget(&key(), t0 + Duration::from_micros(15_500)));
    assert!(!core.render_widget(&key(), t0 + Duration::from_micros(29_500)));
    assert!(core.render_widget(&key(), t0 + Duration::from_millis(31)));
}

#[test]
fn a_render_that_cannot_be_sent_changes_nothing() {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    core.register(Box::new(module()));
    core.set_widget_env(env());
    let t0 = Instant::now();

    assert!(!core.render_widget(&key(), t0));
    core.start_modules();

    assert!(core.render_widget(&key(), t0 + Duration::from_millis(1)));
    let rx = core.events();
    let seq = loop {
        if let CoreEvent::WidgetRendered { seq, .. } = inner(&wait_event(&rx)) {
            break *seq;
        }
    };
    assert_eq!(seq, 1);
}

/// Answers every search with tiles of `w` and `v`.
fn answers_w_and_v() -> Vec<WidgetAnswer> {
    ["w", "v"]
        .map(|widget| WidgetAnswer {
            widget: widget.into(),
            payload: "4".into(),
            actions: vec![],
        })
        .to_vec()
}

/// Searches `query` and applies the provider's widget answers.
fn search_and_apply(core: &mut Core, query: &str) -> CoreUpdate {
    core.search(query, now());
    apply_until(core, |ev| matches!(ev, CoreEvent::WidgetAnswers { .. }))
}

#[test]
fn five_late_answer_frames_stop_the_widget_and_its_renders() {
    let work = Arc::new(AtomicU64::new(late_render_millis()));
    let m = module()
        .with_render_work(work)
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let renders = m.renders.clone();
    let mut core = core_with(m);

    let updates: Vec<CoreUpdate> = (0..5)
        .map(|i| search_and_apply(&mut core, &format!("q{i}")))
        .collect();

    assert!(updates[..4].iter().all(|u| matches!(u, CoreUpdate::None)));
    let CoreUpdate::WidgetStopped { widgets } = &updates[4] else {
        panic!("not stopped: {:?}", updates[4]);
    };
    assert_eq!(widgets[0].title, "Widget w");
    assert_eq!(renders.load(Ordering::SeqCst), 5);
    search_and_apply(&mut core, "q5");
    assert_eq!(renders.load(Ordering::SeqCst), 5);
    assert!(!core.render_widget(&key(), Instant::now()));
}

#[test]
fn one_apply_reports_every_widget_it_stops() {
    let work = Arc::new(AtomicU64::new(late_render_millis()));
    let m = FakeModule::new(vec![], nothing)
        .with_widgets(vec![decl("w"), decl("v")])
        .with_render(hello)
        .with_render_work(work)
        .with_search(vec![])
        .with_widget_answers(answers_w_and_v());
    let mut core = core_with(m);

    let updates: Vec<CoreUpdate> = (0..5)
        .map(|i| search_and_apply(&mut core, &format!("q{i}")))
        .collect();

    let CoreUpdate::WidgetStopped { widgets } = &updates[4] else {
        panic!("not stopped: {:?}", updates[4]);
    };
    let titles: Vec<&str> = widgets.iter().map(|w| w.title.as_str()).collect();
    assert_eq!(titles, ["Widget w", "Widget v"]);
}

#[test]
fn a_second_answer_of_the_same_widget_keeps_the_first() {
    let m = module().with_search(vec![]).with_widget_answers(
        ["1", "2"]
            .map(|payload| WidgetAnswer {
                widget: "w".into(),
                payload: payload.into(),
                actions: vec![],
            })
            .to_vec(),
    );
    let renders = m.renders.clone();
    let events = m.widget_events.clone();
    let mut core = core_with(m);

    core.search("x", now());
    let rx = core.events();
    let ev = loop {
        let ev = wait_event(&rx);
        if matches!(inner(&ev), CoreEvent::WidgetAnswers { .. }) {
            break ev;
        }
        core.apply(ev, now());
    };
    let CoreEvent::WidgetAnswers { answers, .. } = inner(&ev) else {
        unreachable!()
    };
    let payloads: Vec<&str> = answers.iter().map(|a| a.answer.payload.as_str()).collect();
    assert_eq!(payloads, ["1"]);
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    core.apply(ev, now());
    core.send_widget_event(&key(), action("copy")).unwrap();
    apply_until(&mut core, event_finished);

    assert_eq!(events.lock().unwrap()[0].1.as_deref(), Some("1"));
}

/// The first call draws "first"; later ones draw "later" with image `pic`.
fn picture_later(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::SeqCst) == 0 {
        return Ok(frame("first"));
    }
    let mut frame = frame("later");
    frame.images.push(image("pic", &[7]));
    Ok(frame)
}

#[test]
fn a_late_frames_images_are_kept() {
    let work = Arc::new(AtomicU64::new(0));
    let m = module()
        .with_render(picture_later)
        .with_render_work(work.clone());
    let mut core = core_with(m);
    let t0 = Instant::now();
    render_and_apply(&mut core, t0);

    work.store(late_render_millis(), Ordering::SeqCst);
    render_and_apply(&mut core, t0 + Duration::from_millis(20));

    assert_eq!(core.widget_frame(&key()).as_deref(), Some(&frame("first")));
    assert_eq!(
        core.widget_image("fake", "pic").map(|(_, image)| image),
        Some(image("pic", &[7]))
    );
}

#[test]
fn the_images_of_an_older_answer_are_kept() {
    let m = module()
        .with_render(with_picture)
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let mut core = core_with(m);
    core.search("a", now());
    let rx = core.events();
    let old = loop {
        let ev = wait_event(&rx);
        if matches!(inner(&ev), CoreEvent::WidgetAnswers { .. }) {
            break ev;
        }
        core.apply(ev, now());
    };

    core.search("b", now());
    core.apply(old, now());

    assert_eq!(core.widget_frame(&key()), None);
    assert!(core.widget_image("fake", "pic").is_some());
}

fn pointer(kind: PointerKind) -> WidgetEvent {
    WidgetEvent::Pointer(PointerEvent {
        node: "n".into(),
        kind,
        at: Point { x: 1., y: 1. },
    })
}

#[test]
fn only_presses_and_releases_of_the_pointer_carry_a_request() {
    let mut core = core_with(module());

    for kind in [PointerKind::Move, PointerKind::Enter, PointerKind::Leave] {
        let request = core.send_widget_event(&key(), pointer(kind)).unwrap();
        assert_eq!(request, None, "{kind:?}");
    }
    for kind in [PointerKind::Down, PointerKind::Up] {
        let request = core.send_widget_event(&key(), pointer(kind)).unwrap();
        assert!(request.is_some(), "{kind:?}");
    }
    let key_event = WidgetEvent::Key(tomakast_core::KeyEvent {
        node: "n".into(),
        key: "enter".into(),
        modifiers: tomakast_core::Modifiers::empty(),
    });
    assert!(core.send_widget_event(&key(), key_event).unwrap().is_some());
}

/// Like [`numbered`], with its own count.
fn counted(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    static N: AtomicUsize = AtomicUsize::new(0);
    Ok(frame(&format!(
        "frame {}",
        N.fetch_add(1, Ordering::SeqCst)
    )))
}

#[test]
fn a_frame_for_an_older_render_is_dropped() {
    let mut core = core_with(module().with_render(counted));
    let t0 = Instant::now();

    assert!(core.render_widget(&key(), t0));
    assert!(core.render_widget(&key(), t0 + Duration::from_millis(20)));
    apply_until(&mut core, rendered);
    assert_eq!(core.widget_frame(&key()), None);
    apply_until(&mut core, rendered);

    assert_eq!(
        core.widget_frame(&key()).as_deref(),
        Some(&frame("frame 1"))
    );
}

#[test]
fn search_renders_answered_widgets_on_the_same_turn() {
    let answer = WidgetAnswer {
        widget: "w".into(),
        payload: "4".into(),
        actions: vec![],
    };
    let m = module()
        .with_search(vec![])
        .with_widget_answers(vec![answer.clone()]);
    let contexts = m.render_contexts.clone();
    let mut core = core_with(m);
    apply_until(&mut core, |ev| matches!(ev, CoreEvent::ReplaceItems { .. }));

    core.search("2+2", now());
    let rx = core.events();
    let first = wait_event(&rx);
    let second = wait_event(&rx);

    assert!(matches!(inner(&first), CoreEvent::SearchResults { .. }));
    let CoreEvent::WidgetAnswers { answers, .. } = inner(&second) else {
        panic!("not widget answers: {second:?}");
    };
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].answer, answer);
    assert_eq!(answers[0].frame, Ok(frame("hello")));
    let contexts = contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].1.query.as_deref(), Some("2+2"));
    assert_eq!(contexts[0].1.payload.as_deref(), Some("4"));
    assert_eq!(contexts[0].1.width, 355.);
    assert_eq!(contexts[0].1.focused, None);
    assert_eq!(contexts[0].1.pointer, None);
}

#[test]
fn a_condition_tile_is_rendered_with_its_query_and_payload() {
    let m = module()
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let contexts = m.render_contexts.clone();
    let mut core = core_with(m);

    core.search("2+2", now());
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::WidgetAnswers { .. })
    });
    assert_eq!(core.widget_frame(&key()).as_deref(), Some(&frame("hello")));
    render_and_apply(&mut core, Instant::now());

    let contexts = contexts.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[1].1.query.as_deref(), Some("2+2"));
    assert_eq!(contexts[1].1.payload.as_deref(), Some("4"));
}

#[test]
fn an_answer_naming_an_undeclared_widget_is_dropped() {
    let m = module()
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "nope".into(),
            payload: "x".into(),
            actions: vec![],
        }]);
    let renders = m.renders.clone();
    let mut core = core_with(m);

    core.search("x", now());
    let update = apply_until(
        &mut core,
        |ev| matches!(ev, CoreEvent::WidgetAnswers { answers, .. } if answers.is_empty()),
    );

    assert!(matches!(update, CoreUpdate::None), "{update:?}");
    assert_eq!(renders.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failed_search_sends_empty_widget_answers() {
    let m = module()
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let panic = m.panic_on_search.clone();
    let mut core = core_with(m);
    core.search("2+2", now());
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::WidgetAnswers { .. })
    });
    assert!(core.widget_frame(&key()).is_some());

    panic.store(true, Ordering::SeqCst);
    let generation = core.search("2+23", now()).generation;
    let rx = core.events();
    let ev = loop {
        let ev = wait_event(&rx);
        if !matches!(inner(&ev), CoreEvent::StripChanged) {
            break ev;
        }
        core.apply(ev, now());
    };

    let CoreEvent::WidgetAnswers {
        generation: answered,
        answers,
        ..
    } = inner(&ev)
    else {
        panic!("not widget answers: {ev:?}");
    };
    assert_eq!(*answered, generation);
    assert!(answers.is_empty());
    core.apply(ev, now());
    assert_eq!(core.widget_frame(&key()), None);
}

#[test]
fn no_env_no_widget_answers() {
    let m = module()
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let renders = m.renders.clone();
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    core.register(Box::new(m));
    core.start_modules();

    core.search("2+2", now());
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::SearchResults { .. })
    });
    std::thread::sleep(Duration::from_millis(20));

    assert!(core.events().is_empty());
    assert_eq!(renders.load(Ordering::SeqCst), 0);
    assert!(!core.render_widget(&key(), Instant::now()));
}

/// A module that shows its tiles from `start`, as a plugin's `init` does.
struct ShowsFromStart;

impl Module for ShowsFromStart {
    fn id(&self) -> &str {
        "fake"
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.emit(vec![Effect::ShowWidgets(vec!["w".into(), "nope".into()])]);
    }

    fn run_action(&mut self, _: &tomakast_core::Item, _: &str) -> anyhow::Result<Vec<Effect>> {
        Ok(vec![])
    }

    fn widgets(&self) -> Vec<WidgetDecl> {
        vec![decl("w")]
    }
}

#[test]
fn show_widgets_from_init_is_applied_while_no_window_is_open() {
    let mut core = core_with(ShowsFromStart);

    let update = apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::ModuleEffects { .. })
    });

    let CoreUpdate::ModuleEffects { effects, .. } = update else {
        panic!("not module effects: {update:?}");
    };
    assert!(effects.is_empty());
    assert_eq!(core.always_widgets("fake"), ["w"]);
    let next = apply_until(&mut core, |ev| matches!(ev, CoreEvent::StripChanged));
    assert!(matches!(next, CoreUpdate::Widgets));
}

/// Answers action `a` with image `i` = [1] and any other with `i` = [2, 2].
fn set_image(_: &str, _: Option<&str>, event: WidgetEvent) -> anyhow::Result<Vec<Effect>> {
    let bytes: &[u8] = match event {
        WidgetEvent::Action { action, .. } if action == "a" => &[1],
        _ => &[2, 2],
    };
    Ok(vec![Effect::SetWidgetImage(image("i", bytes))])
}

fn action(name: &str) -> WidgetEvent {
    WidgetEvent::Action {
        node: None,
        action: name.into(),
    }
}

#[test]
fn set_widget_image_bumps_the_version() {
    let mut core = core_with(module().with_widget_events(set_image));

    core.send_widget_event(&key(), action("a")).unwrap();
    apply_until(&mut core, event_finished);
    assert_eq!(core.widget_image("fake", "i"), Some((1, image("i", &[1]))));
    core.send_widget_event(&key(), action("b")).unwrap();
    apply_until(&mut core, event_finished);

    assert_eq!(
        core.widget_image("fake", "i"),
        Some((2, image("i", &[2, 2])))
    );
}

#[test]
fn storing_identical_image_bytes_keeps_the_version() {
    let mut core = core_with(module().with_widget_events(set_image));

    core.send_widget_event(&key(), action("a")).unwrap();
    apply_until(&mut core, event_finished);
    core.send_widget_event(&key(), action("a")).unwrap();
    apply_until(&mut core, event_finished);
    assert_eq!(core.widget_image("fake", "i"), Some((1, image("i", &[1]))));

    core.send_widget_event(&key(), action("b")).unwrap();
    apply_until(&mut core, event_finished);
    assert_eq!(
        core.widget_image("fake", "i"),
        Some((2, image("i", &[2, 2])))
    );
}

/// Draws a frame carrying image `pic`.
fn with_picture(_: &str, _: &WidgetContext) -> anyhow::Result<Frame> {
    let mut frame = frame("pic");
    frame.images.push(image("pic", &[7]));
    Ok(frame)
}

#[test]
fn a_frames_images_are_stored() {
    let mut core = core_with(module().with_render(with_picture));

    render_and_apply(&mut core, Instant::now());

    assert_eq!(
        core.widget_image("fake", "pic"),
        Some((1, image("pic", &[7])))
    );
    assert_eq!(core.widget_frame(&key()).as_deref(), Some(&frame("pic")));
}

fn show_and_toast(_: &str, _: Option<&str>, _: WidgetEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![
        Effect::ShowWidgets(vec!["w".into()]),
        Effect::Toast(toast()),
    ])
}

fn toast() -> Toast {
    Toast {
        kind: ToastKind::Success,
        title: "done".into(),
        message: None,
    }
}

#[test]
fn widget_effects_never_reach_the_ui() {
    let mut core = core_with(module().with_widget_events(show_and_toast));

    let request = core.send_widget_event(&key(), action("go")).unwrap();
    let update = apply_until(&mut core, event_finished);

    assert!(request.is_some());
    let CoreUpdate::WidgetEffects {
        request: answered,
        provider,
        effects,
        items_changed,
        ..
    } = update
    else {
        panic!("not widget effects: {update:?}");
    };
    assert_eq!(answered, request);
    assert_eq!(provider, "fake");
    assert_eq!(effects, [Effect::Toast(toast())]);
    assert!(!items_changed);
    assert_eq!(core.always_widgets("fake"), ["w"]);
}

#[test]
fn shown_and_hidden_carry_no_request() {
    let m = module();
    let events = m.widget_events.clone();
    let mut core = core_with(m);

    let request = core.send_widget_event(&key(), WidgetEvent::Shown).unwrap();
    let update = apply_until(&mut core, event_finished);

    assert_eq!(request, None);
    assert!(matches!(
        update,
        CoreUpdate::WidgetEffects { request: None, .. }
    ));
    assert_eq!(
        *events.lock().unwrap(),
        [("w".to_string(), None, WidgetEvent::Shown)]
    );
}

fn failed_event(_: &str, _: Option<&str>, _: WidgetEvent) -> anyhow::Result<Vec<Effect>> {
    anyhow::bail!("no such action")
}

#[test]
fn a_failed_widget_action_fails_its_request() {
    let mut core = core_with(module().with_widget_events(failed_event));

    let request = core
        .send_widget_event(&key(), action("go"))
        .unwrap()
        .unwrap();
    let update = apply_until(&mut core, event_finished);

    let CoreUpdate::ActionFailed {
        request: failed,
        message,
    } = update
    else {
        panic!("not failed: {update:?}");
    };
    assert_eq!(failed, request);
    assert!(message.contains("no such action"), "{message}");
}

#[test]
fn a_widget_event_reaches_the_module_with_the_answers_payload() {
    let m = module()
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let events = m.widget_events.clone();
    let mut core = core_with(m);
    core.search("2+2", now());
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::WidgetAnswers { .. })
    });

    core.send_widget_event(&key(), action("copy")).unwrap();
    apply_until(&mut core, event_finished);

    assert_eq!(
        *events.lock().unwrap(),
        [("w".to_string(), Some("4".to_string()), action("copy"))]
    );
}

fn show_and_set_image(_: &str, _: Option<&str>, _: WidgetEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![
        Effect::ShowWidgets(vec!["w".into()]),
        Effect::SetWidgetImage(image("i", &[1])),
    ])
}

#[test]
fn an_unregistered_provider_forgets_its_widgets() {
    let m = module()
        .with_widget_events(show_and_set_image)
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let mut core = core_with(m);
    render_and_apply(&mut core, Instant::now());
    core.send_widget_event(&key(), action("a")).unwrap();
    apply_until(&mut core, event_finished);
    core.search("2+2", now());
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::WidgetAnswers { .. })
    });
    assert_eq!(core.always_widgets("fake"), ["w"]);
    assert!(core.widget_image("fake", "i").is_some());
    assert!(core.widget_frame(&key()).is_some());

    core.unregister("fake");

    assert_eq!(core.widget_frame(&key()), None);
    assert_eq!(core.widget_image("fake", "i"), None);
    assert!(core.always_widgets("fake").is_empty());
    assert!(!core.render_widget(&key(), Instant::now() + Duration::from_secs(1)));
    let err = core.send_widget_event(&key(), action("a")).unwrap_err();
    assert!(err.to_string().contains("no module for provider"), "{err}");
}

#[test]
fn a_queued_widget_action_of_a_stopped_module_fails() {
    let delay = Arc::new(AtomicU64::new(200));
    let m = module().with_render_delay(delay);
    let renders = m.renders.clone();
    let events = m.widget_events.clone();
    let mut core = core_with(m);
    assert!(core.render_widget(&key(), Instant::now()));
    while renders.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(1));
    }
    let request = core
        .send_widget_event(&key(), action("go"))
        .unwrap()
        .unwrap();
    core.send_widget_event(&key(), WidgetEvent::Hidden).unwrap();

    core.unregister("fake");
    let update = apply_until(&mut core, event_finished);

    let CoreUpdate::ActionFailed {
        request: failed,
        message,
    } = update
    else {
        panic!("not failed: {update:?}");
    };
    assert_eq!(failed, request);
    assert_eq!(message, "module fake was stopped");
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn a_module_without_widgets_has_none() {
    let mut core = core_with(FakeModule::new(vec![], nothing));

    let err = core.send_widget_event(&key(), action("go")).unwrap_err();

    assert!(err.to_string().contains("no widget fake/w"), "{err}");
    assert!(!core.render_widget(&key(), Instant::now()));
}

#[test]
fn a_replaced_instance_forgets_the_old_always_set() {
    let mut core = core_with(ShowsFromStart);
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::ModuleEffects { .. })
    });
    assert_eq!(core.always_widgets("fake"), ["w"]);

    core.register(Box::new(module()));

    assert!(core.always_widgets("fake").is_empty());
}

/// The widget answers of a query.
type Answers = fn(&str) -> anyhow::Result<Vec<WidgetAnswer>>;

/// A module with widgets that shows `shown` from `start`; with `answer`, a
/// dynamic source whose answers come from it. Its frames say
/// `<widget>:<payload>` (`-` for none).
struct Tiles {
    id: String,
    approved_at: Option<i64>,
    widgets: Vec<WidgetDecl>,
    shown: Vec<String>,
    answer: Option<Answers>,
    animate: bool,
    /// `start` also asks every shown tile to draw.
    redraw_on_start: bool,
    /// Milliseconds every render sleeps; may change any time.
    render_delay: Arc<AtomicU64>,
    /// Milliseconds of CPU time every render burns; may change any time.
    render_work: Arc<AtomicU64>,
    /// Effects answered to every widget event.
    on_event: Vec<Effect>,
    /// `start` waits while this is set (at most 5 s).
    start_gate: Option<Arc<AtomicBool>>,
    renders: Arc<AtomicUsize>,
    contexts: Arc<Mutex<Vec<(String, WidgetContext)>>>,
    events: Arc<Mutex<Vec<SeenWidgetEvent>>>,
}

impl Tiles {
    /// Provider `id` declaring `widgets` (single tiles), showing none.
    fn new(id: &str, widgets: &[&str]) -> Tiles {
        Tiles {
            id: id.into(),
            approved_at: None,
            widgets: widgets.iter().map(|w| decl(w)).collect(),
            shown: vec![],
            answer: None,
            animate: false,
            redraw_on_start: false,
            render_delay: Arc::default(),
            render_work: Arc::default(),
            on_event: vec![],
            start_gate: None,
            renders: Arc::default(),
            contexts: Arc::default(),
            events: Arc::default(),
        }
    }

    fn approved(mut self, at: i64) -> Tiles {
        self.approved_at = Some(at);
        self
    }

    fn showing(mut self, ids: &[&str]) -> Tiles {
        self.shown = ids.iter().map(|id| id.to_string()).collect();
        self
    }

    fn answering(mut self, answer: Answers) -> Tiles {
        self.answer = Some(answer);
        self
    }

    fn animating(mut self) -> Tiles {
        self.animate = true;
        self
    }

    fn redrawing(mut self) -> Tiles {
        self.redraw_on_start = true;
        self
    }

    /// `start` waits while `gate` is set.
    fn gated(mut self, gate: Arc<AtomicBool>) -> Tiles {
        self.start_gate = Some(gate);
        self
    }
}

impl Module for Tiles {
    fn id(&self) -> &str {
        &self.id
    }

    fn start(&mut self, ctx: ModuleContext) {
        if let Some(gate) = &self.start_gate {
            let deadline = Instant::now() + Duration::from_secs(5);
            while gate.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let mut effects = vec![Effect::ShowWidgets(self.shown.clone())];
        if self.redraw_on_start {
            effects.extend(self.shown.iter().cloned().map(Effect::RedrawWidget));
        }
        ctx.emit(effects);
    }

    fn run_action(&mut self, _: &tomakast_core::Item, _: &str) -> anyhow::Result<Vec<Effect>> {
        Ok(vec![])
    }

    fn is_dynamic_source(&self) -> bool {
        self.answer.is_some()
    }

    fn search(&mut self, query: &str) -> anyhow::Result<SearchAnswer> {
        let answer = self.answer.expect("searched a static module");
        Ok(SearchAnswer {
            items: vec![],
            widgets: answer(query)?,
        })
    }

    fn widgets(&self) -> Vec<WidgetDecl> {
        self.widgets.clone()
    }

    fn approved_at(&self) -> Option<i64> {
        self.approved_at
    }

    fn render_widget(&mut self, widget: &str, ctx: &WidgetContext) -> anyhow::Result<Frame> {
        self.renders.fetch_add(1, Ordering::SeqCst);
        self.contexts
            .lock()
            .unwrap()
            .push((widget.to_string(), ctx.clone()));
        std::thread::sleep(Duration::from_millis(
            self.render_delay.load(Ordering::SeqCst),
        ));
        burn_cpu(Duration::from_millis(
            self.render_work.load(Ordering::SeqCst),
        ));
        let mut frame = frame(&format!(
            "{widget}:{}",
            ctx.payload.as_deref().unwrap_or("-")
        ));
        frame.animate = self.animate;
        frame.actions = vec![copy_action()];
        Ok(frame)
    }

    fn on_widget_event(
        &mut self,
        widget: &str,
        payload: Option<&str>,
        event: WidgetEvent,
    ) -> anyhow::Result<Vec<Effect>> {
        self.events
            .lock()
            .unwrap()
            .push((widget.to_string(), payload.map(str::to_string), event));
        Ok(self.on_event.clone())
    }
}

fn copy_action() -> Action {
    Action {
        id: "copy".into(),
        title: "Copy".into(),
        shortcut: None,
    }
}

/// The answer of a calculator: `2+2` is 4, `2+23` is 25, `bad` fails, and
/// anything else has no tile.
fn calc(query: &str) -> anyhow::Result<Vec<WidgetAnswer>> {
    let payload = match query {
        "2+2" => "4",
        "2+23" => "25",
        "bad" => anyhow::bail!("cannot compute"),
        _ => return Ok(vec![]),
    };
    Ok(vec![WidgetAnswer {
        widget: "w".into(),
        payload: payload.into(),
        actions: vec![copy_action()],
    }])
}

/// A started core over `modules`, with the env set, once every module's
/// `ShowWidgets` from `start` is applied.
fn core_of(modules: Vec<Tiles>) -> Core {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    let n = modules.len();
    for m in modules {
        core.register(Box::new(m));
    }
    core.set_widget_env(env());
    core.start_modules();
    for _ in 0..n {
        apply_until(&mut core, |ev| {
            matches!(ev, CoreEvent::ModuleEffects { .. })
        });
    }
    core
}

fn keys(core: &Core) -> Vec<String> {
    core.strip()
        .iter()
        .map(|tile| tile.key.to_string())
        .collect()
}

fn wkey(s: &str) -> WidgetKey {
    WidgetKey::parse(s).unwrap()
}

fn settings(order: &[&str], hidden: &[&str]) -> WidgetSettings {
    WidgetSettings {
        order: order.iter().map(|k| wkey(k)).collect(),
        hidden: hidden.iter().map(|k| wkey(k)).collect(),
    }
}

fn visible(keys: &[&str]) -> Vec<VisibleTile> {
    keys.iter()
        .map(|k| VisibleTile {
            key: wkey(k),
            focused: None,
            pointer: None,
        })
        .collect()
}

fn answers_applied(ev: &CoreEvent) -> bool {
    matches!(ev, CoreEvent::WidgetAnswers { .. })
}

#[test]
fn always_tiles_follow_the_settings_order_then_approval_order() {
    let mut core = core_of(vec![
        Tiles::new("b", &["x"]).approved(10).showing(&["x"]),
        Tiles::new("a", &["y"]).approved(20).showing(&["y"]),
    ]);

    assert_eq!(keys(&core), ["b/x", "a/y"]);
    core.set_widget_settings(settings(&["a/y"], &[]));
    assert_eq!(keys(&core), ["a/y", "b/x"]);
}

#[test]
fn built_ins_lead_the_approval_order_and_ties_follow_ids_then_declarations() {
    let core = core_of(vec![
        Tiles::new("b", &["x"]).approved(10).showing(&["x"]),
        Tiles::new("c", &["z"]).approved(10).showing(&["z"]),
        Tiles::new("n", &["1", "2"]).showing(&["2", "1"]),
    ]);

    assert_eq!(keys(&core), ["n/1", "n/2", "b/x", "c/z"]);
}

#[test]
fn hidden_tiles_are_not_in_the_strip() {
    let mut core = core_of(vec![
        Tiles::new("a", &["x", "y", "w"])
            .showing(&["x", "y"])
            .answering(calc),
    ]);
    search_and_apply(&mut core, "2+2");
    assert_eq!(keys(&core), ["a/w", "a/x", "a/y"]);

    core.set_widget_settings(settings(&["a/x"], &["a/x", "a/w"]));

    assert_eq!(keys(&core), ["a/y"]);
}

#[test]
fn condition_tiles_come_first_and_an_answered_always_tile_shows_once() {
    let mut core = core_of(vec![
        Tiles::new("calc", &["w", "v"])
            .showing(&["v", "w"])
            .answering(calc),
        Tiles::new("aaa", &["t"]).showing(&["t"]),
    ]);

    search_and_apply(&mut core, "2+2");

    assert_eq!(keys(&core), ["calc/w", "aaa/t", "calc/v"]);
    let strip = core.strip();
    let condition = strip[0].condition.as_ref().unwrap();
    assert_eq!(condition.payload, "4");
    assert_eq!(condition.actions, [copy_action()]);
    assert!(!condition.stale);
    assert_eq!(strip[0].frame.as_deref(), Some(&frame_of("w:4")));
    assert_eq!(strip[0].title, "Widget w");
    assert_eq!(strip[0].size, WidgetSize::Single);
    assert!(strip[1].condition.is_none());
    assert_eq!(strip[1].frame, None);
}

/// What [`Tiles`] draws for `text`.
fn frame_of(text: &str) -> Frame {
    let mut frame = frame(text);
    frame.actions = vec![copy_action()];
    frame
}

#[test]
fn a_new_answer_replaces_the_condition_tiles_atomically() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    search_and_apply(&mut core, "2+2");

    core.search("2+23", now());

    let strip = core.strip();
    assert_eq!(strip.len(), 1);
    let condition = strip[0].condition.as_ref().unwrap();
    assert!(condition.stale);
    assert_eq!(condition.payload, "4");
    assert_eq!(strip[0].frame.as_deref(), Some(&frame_of("w:4")));

    apply_until(&mut core, answers_applied);
    let strip = core.strip();
    let condition = strip[0].condition.as_ref().unwrap();
    assert!(!condition.stale);
    assert_eq!(condition.payload, "25");
    assert_eq!(strip[0].frame.as_deref(), Some(&frame_of("w:25")));

    search_and_apply(&mut core, "nothing");
    assert!(core.strip().is_empty());
}

#[test]
fn a_stale_tile_takes_no_user_actions() {
    let m = Tiles::new("calc", &["w"]).answering(calc);
    let events = m.events.clone();
    let mut core = core_of(vec![m]);
    let w = wkey("calc/w");
    search_and_apply(&mut core, "2+2");
    assert!(!core.widget_answers_pending());
    assert_eq!(core.widget_actions(&w), [copy_action()]);

    core.search("2+23", now());

    assert!(core.widget_answers_pending());
    let key_event = WidgetEvent::Key(tomakast_core::KeyEvent {
        node: "n".into(),
        key: "enter".into(),
        modifiers: tomakast_core::Modifiers::empty(),
    });
    for event in [action("copy"), key_event, pointer(PointerKind::Up)] {
        let err = core.send_widget_event(&w, event).unwrap_err();
        assert!(err.to_string().contains("older query"), "{err}");
    }
    assert!(core.widget_actions(&w).is_empty());

    apply_until(&mut core, answers_applied);

    assert!(!core.widget_answers_pending());
    assert_eq!(core.widget_actions(&w), [copy_action()]);
    core.send_widget_event(&w, action("copy")).unwrap();
    apply_until(&mut core, event_finished);
    assert_eq!(
        *events.lock().unwrap(),
        [("w".to_string(), Some("25".to_string()), action("copy"))]
    );
}

/// A module that declared widgets and has not finished `start` may be a
/// dynamic source: a search then waits for its tiles, until it answers or
/// turns out not to answer searches.
#[test]
fn answers_are_pending_while_a_module_with_widgets_starts() {
    let gate = Arc::new(AtomicBool::new(true));
    let static_gate = Arc::new(AtomicBool::new(true));
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    core.register(Box::new(
        Tiles::new("calc", &["w"])
            .answering(calc)
            .gated(gate.clone()),
    ));
    core.register(Box::new(
        Tiles::new("clock", &["w"]).gated(static_gate.clone()),
    ));
    core.set_widget_env(env());
    core.start_modules();

    core.search("2+2", now());
    assert!(core.widget_answers_pending(), "both are starting");

    gate.store(false, Ordering::SeqCst);
    apply_until(&mut core, answers_applied);
    assert!(core.widget_answers_pending(), "clock is still starting");
    assert_eq!(core.strip()[0].condition.as_ref().unwrap().payload, "4");

    // Started, `clock` turns out not to answer searches.
    static_gate.store(false, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while core.widget_answers_pending() {
        assert!(Instant::now() < deadline, "clock never started");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn answers_are_pending_only_from_a_dynamic_source_with_an_env() {
    let mut core = core_of(vec![Tiles::new("clock", &["w"]).showing(&["w"])]);
    core.search("2+2", now());
    assert!(!core.widget_answers_pending());

    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        now(),
    )
    .unwrap();
    core.register(Box::new(Tiles::new("calc", &["w"]).answering(calc)));
    core.start_modules();
    core.search("2+2", now());
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::SearchResults { .. })
    });
    core.search("2+23", now());
    assert!(!core.widget_answers_pending());
}

#[test]
fn a_failed_search_clears_its_providers_condition_tiles() {
    let mut core = core_of(vec![
        Tiles::new("calc", &["w", "v"])
            .showing(&["v"])
            .answering(calc),
    ]);
    search_and_apply(&mut core, "2+2");
    assert_eq!(keys(&core), ["calc/w", "calc/v"]);

    core.search("bad", now());
    let update = apply_until(
        &mut core,
        |ev| matches!(ev, CoreEvent::WidgetAnswers { answers, .. } if answers.is_empty()),
    );

    assert!(matches!(update, CoreUpdate::None), "{update:?}");
    assert_eq!(keys(&core), ["calc/v"]);
    assert!(!core.widget_answers_pending());
}

/// Counts the `render` events of target `widgets`: the log line the
/// acceptance count reads.
#[derive(Clone, Default)]
struct RenderLog(Arc<AtomicUsize>);

impl RenderLog {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl tracing::Subscriber for RenderLog {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "widgets"
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Message(bool);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" && format!("{value:?}") == "render" {
                    self.0 = true;
                }
            }
        }
        let mut message = Message(false);
        event.record(&mut message);
        if event.metadata().target() == "widgets" && message.0 {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Runs `f` with `log` counting the renders issued on this thread.
fn logged<T>(log: &RenderLog, f: impl FnOnce() -> T) -> T {
    tracing::subscriber::with_default(log.clone(), f)
}

/// Ticks `visible` at `at`; when that issued renders, applies their replies.
/// Returns how many it issued.
fn tick_and_apply(core: &mut Core, log: &RenderLog, at: Instant, visible: &[VisibleTile]) -> usize {
    let before = log.count();
    logged(log, || core.widget_tick(at, visible));
    let issued = log.count() - before;
    for _ in 0..issued {
        apply_until(core, rendered);
    }
    issued
}

/// Shows `visible` at `t0` and settles: the first renders and the replies to
/// `Shown` are applied, then the redraw those replies ask for is rendered and
/// applied too. Returns the time of the last tick.
fn show_and_settle(
    core: &mut Core,
    log: &RenderLog,
    t0: Instant,
    visible: &[VisibleTile],
) -> Instant {
    // A worker answers in order, so the replies to `Shown` came before the frames.
    assert_eq!(tick_and_apply(core, log, t0, visible), visible.len());
    let t1 = t0 + Duration::from_millis(20);
    assert_eq!(tick_and_apply(core, log, t1, visible), visible.len());
    t1
}

#[test]
fn ticks_render_at_most_60_per_second() {
    let m = Tiles::new("anim", &["w"]).showing(&["w"]).animating();
    let renders = m.renders.clone();
    let mut core = core_of(vec![m]);
    let log = RenderLog::default();
    let visible = visible(&["anim/w"]);
    let t0 = Instant::now();

    let mut asked_again = true;
    for ms in 0..100 {
        tick_and_apply(&mut core, &log, t0 + Duration::from_millis(ms), &visible);
        asked_again &= logged(&log, || {
            core.widget_tick(t0 + Duration::from_millis(ms), &visible)
        });
    }

    assert!((6..=7).contains(&log.count()), "{} renders", log.count());
    assert_eq!(renders.load(Ordering::SeqCst), log.count());
    assert!(asked_again, "an animating tile asks for every frame");
}

#[test]
fn no_render_while_one_is_in_flight() {
    let m = Tiles::new("slow", &["w"]).showing(&["w"]);
    m.render_delay.store(50, Ordering::SeqCst);
    let mut core = core_of(vec![m]);
    let log = RenderLog::default();
    let visible = visible(&["slow/w"]);
    let t0 = Instant::now();

    logged(&log, || core.widget_tick(t0, &visible));
    assert_eq!(log.count(), 1);
    // The reply to `Shown` asks for a redraw while the render is in flight.
    apply_until(&mut core, event_finished);
    let busy = logged(&log, || {
        core.widget_tick(t0 + Duration::from_millis(20), &visible)
    });

    assert!(busy);
    assert_eq!(log.count(), 1);
    apply_until(&mut core, rendered);
    logged(&log, || {
        core.widget_tick(t0 + Duration::from_millis(40), &visible)
    });
    assert_eq!(log.count(), 2);
}

#[test]
fn shown_and_hidden_events_follow_visibility() {
    let m = Tiles::new("t", &["w", "v"]).showing(&["w", "v"]);
    let events = m.events.clone();
    let mut core = core_of(vec![m]);
    let t0 = Instant::now();

    core.widget_tick(t0, &visible(&["t/w"]));
    let update = apply_until(&mut core, event_finished);
    assert!(
        matches!(update, CoreUpdate::WidgetEffects { request: None, .. }),
        "{update:?}"
    );
    core.widget_tick(t0 + Duration::from_millis(5), &visible(&["t/w"]));
    core.widget_tick(t0 + Duration::from_millis(10), &[]);
    apply_until(&mut core, event_finished);

    assert_eq!(
        *events.lock().unwrap(),
        [
            ("w".to_string(), None, WidgetEvent::Shown),
            ("w".to_string(), None, WidgetEvent::Hidden),
        ]
    );
}

#[test]
fn a_shown_tile_is_rendered_with_its_visibility() {
    let m = Tiles::new("t", &["w"]).showing(&["w"]);
    let contexts = m.contexts.clone();
    let mut core = core_of(vec![m]);
    let log = RenderLog::default();
    let t0 = Instant::now();
    let pointer = PointerAt {
        node: Some("n".into()),
        at: Point { x: 3., y: 4. },
    };
    let tile = VisibleTile {
        key: wkey("t/w"),
        focused: Some("b".into()),
        pointer: Some(pointer.clone()),
    };

    show_and_settle(&mut core, &log, t0, std::slice::from_ref(&tile));

    let contexts = contexts.lock().unwrap();
    let (widget, ctx) = &contexts[1];
    assert_eq!(widget, "w");
    assert_eq!(ctx.millis_shown, 20);
    assert_eq!(ctx.focused.as_deref(), Some("b"));
    assert_eq!(ctx.pointer, Some(pointer));
    assert_eq!((ctx.width, ctx.height), (355., 120.));
    assert_eq!((ctx.query.as_deref(), ctx.payload.as_deref()), (None, None));
}

#[test]
fn hidden_launcher_means_zero_renders() {
    let mut m = Tiles::new("t", &["w"]).showing(&["w"]).animating();
    m.on_event = vec![Effect::RedrawWidget("w".into())];
    let renders = m.renders.clone();
    let events = m.events.clone();
    let mut core = core_of(vec![m]);
    let log = RenderLog::default();
    tick_and_apply(&mut core, &log, Instant::now(), &visible(&["t/w"]));
    let before = renders.load(Ordering::SeqCst);

    logged(&log, || core.widgets_hidden());
    let rx = core.events();
    let until = Instant::now() + Duration::from_millis(50);
    while Instant::now() < until {
        match rx.try_recv() {
            Ok(ev) => {
                logged(&log, || core.apply(ev, now()));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }

    assert_eq!(renders.load(Ordering::SeqCst), before);
    assert_eq!(log.count(), before);
    let seen =
        || -> Vec<WidgetEvent> { events.lock().unwrap().iter().map(|e| e.2.clone()).collect() };
    assert_eq!(seen(), [WidgetEvent::Shown, WidgetEvent::Hidden]);
    // The UI's frame clock asks for no frame: no tile is on screen.
    let later = Instant::now() + Duration::from_millis(100);
    assert!(!logged(&log, || core.widget_tick(later, &[])));
    assert_eq!(log.count(), before);

    // Opened again, the tile is told `Shown` again and draws.
    let reopened = later + Duration::from_millis(20);
    assert_eq!(
        tick_and_apply(&mut core, &log, reopened, &visible(&["t/w"])),
        1
    );
    assert_eq!(
        seen(),
        [WidgetEvent::Shown, WidgetEvent::Hidden, WidgetEvent::Shown]
    );
}

#[test]
fn an_env_change_redraws_visible_tiles() {
    let m = Tiles::new("t", &["w", "v"]).showing(&["w", "v"]);
    let contexts = m.contexts.clone();
    let mut core = core_of(vec![m]);
    let log = RenderLog::default();
    let visible = visible(&["t/w"]);
    let t1 = show_and_settle(&mut core, &log, Instant::now(), &visible);

    core.set_widget_env(env());
    let busy = logged(&log, || {
        core.widget_tick(t1 + Duration::from_millis(20), &visible)
    });
    assert!(!busy);
    assert_eq!(log.count(), 2);

    let mut wider = env();
    wider.scale = 1.;
    wider.theme.dark = false;
    core.set_widget_env(wider);
    let issued = tick_and_apply(&mut core, &log, t1 + Duration::from_millis(40), &visible);

    assert_eq!(issued, 1);
    let contexts = contexts.lock().unwrap();
    let (widget, ctx) = contexts.last().unwrap();
    assert_eq!(widget, "w");
    assert_eq!(ctx.scale, 1.);
    assert!(!ctx.theme.dark);
    assert!(contexts.iter().all(|(widget, _)| widget == "w"));
}

#[test]
fn a_broken_or_reloaded_plugins_tiles_leave_the_strip() {
    let plugin = || Tiles::new("p", &["w", "v"]).showing(&["w"]).answering(calc);
    let log = RenderLog::default();

    // Broken: the daemon unregisters it.
    let mut core = core_of(vec![plugin()]);
    search_and_apply(&mut core, "2+2");
    assert_eq!(keys(&core), ["p/w"]);
    core.unregister("p");
    assert!(keys(&core).is_empty());
    let err = core
        .send_widget_event(&wkey("p/w"), action("copy"))
        .unwrap_err();
    assert!(err.to_string().contains("no module for provider"), "{err}");

    // Reloaded: a new instance replaces it, at once or after an unregister.
    for unregister_first in [false, true] {
        let mut core = core_of(vec![plugin().showing(&["w", "v"])]);
        search_and_apply(&mut core, "2+2");
        assert_eq!(keys(&core), ["p/w", "p/v"]);
        let t0 = Instant::now();
        tick_and_apply(&mut core, &log, t0, &visible(&["p/w", "p/v"]));

        if unregister_first {
            core.unregister("p");
        }
        let reloaded = Tiles::new("p", &["w", "v"]).showing(&["v"]);
        let renders = reloaded.renders.clone();
        let events = reloaded.events.clone();
        core.register(Box::new(reloaded));

        assert!(keys(&core).is_empty());
        for key in ["p/w", "p/v"] {
            let err = core
                .send_widget_event(&wkey(key), action("copy"))
                .unwrap_err();
            assert!(err.to_string().contains("plugin reloaded"), "{key}: {err}");
        }
        let before = log.count();
        logged(&log, || {
            core.widget_tick(t0 + Duration::from_millis(20), &visible(&["p/w", "p/v"]))
        });
        assert_eq!(
            log.count(),
            before,
            "the old tiles render on the new instance"
        );

        // Once the new instance shows `v`, it takes actions, with no old payload.
        apply_until(&mut core, |ev| {
            matches!(ev, CoreEvent::ModuleEffects { .. })
        });
        assert_eq!(keys(&core), ["p/v"]);
        core.send_widget_event(&wkey("p/v"), action("copy"))
            .unwrap();
        apply_until(&mut core, event_finished);
        assert_eq!(
            *events.lock().unwrap(),
            [("v".to_string(), None, action("copy"))]
        );
        assert_eq!(renders.load(Ordering::SeqCst), 0);
        let err = core
            .send_widget_event(&wkey("p/w"), action("copy"))
            .unwrap_err();
        assert!(err.to_string().contains("plugin reloaded"), "{err}");
    }

    // The new instance shows the answered tile `w` again, as an always tile:
    // Enter on it reaches the new instance without the old answer's payload.
    let mut core = core_of(vec![plugin()]);
    search_and_apply(&mut core, "2+2");
    assert_eq!(core.strip()[0].condition.as_ref().unwrap().payload, "4");
    let reloaded = Tiles::new("p", &["w", "v"]).showing(&["w"]);
    let events = reloaded.events.clone();
    core.register(Box::new(reloaded));
    let err = core
        .send_widget_event(&wkey("p/w"), action("copy"))
        .unwrap_err();
    assert!(err.to_string().contains("plugin reloaded"), "{err}");
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::ModuleEffects { .. })
    });
    assert_eq!(keys(&core), ["p/w"]);
    assert!(core.strip()[0].condition.is_none());
    core.send_widget_event(&wkey("p/w"), action("copy"))
        .unwrap();
    apply_until(&mut core, event_finished);
    assert_eq!(
        *events.lock().unwrap(),
        [("w".to_string(), None, action("copy"))]
    );
}

#[test]
fn only_visible_tiles_render_among_two_hundred() {
    let renders = Arc::new(AtomicUsize::new(0));
    let ids: Vec<String> = (0..16).map(|i| format!("w{i:02}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let modules = (0..16)
        .map(|i| {
            let mut m = Tiles::new(&format!("p{i:02}"), &ids)
                .showing(&ids)
                .redrawing();
            m.renders = renders.clone();
            m
        })
        .collect();
    // Every tile is asked to draw: only the two on screen may.
    let mut core = core_of(modules);
    assert_eq!(core.strip().len(), 256);
    let log = RenderLog::default();

    let issued = tick_and_apply(
        &mut core,
        &log,
        Instant::now(),
        &visible(&["p00/w00", "p00/w01"]),
    );

    assert_eq!(issued, 2);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(renders.load(Ordering::SeqCst), 2);
}

#[test]
fn an_always_tile_no_longer_answered_draws_again() {
    let mut core = core_of(vec![
        Tiles::new("calc", &["w"]).showing(&["w"]).answering(calc),
    ]);
    let log = RenderLog::default();
    let visible = visible(&["calc/w"]);
    search_and_apply(&mut core, "2+2");
    let t1 = show_and_settle(&mut core, &log, Instant::now(), &visible);

    search_and_apply(&mut core, "nothing");
    let issued = tick_and_apply(&mut core, &log, t1 + Duration::from_millis(20), &visible);

    assert_eq!(issued, 1);
    let strip = core.strip();
    assert!(strip[0].condition.is_none());
    assert_eq!(strip[0].frame.as_deref(), Some(&frame_of("w:-")));
}

#[test]
fn a_stopped_tile_wants_no_frames() {
    let m = Tiles::new("t", &["w"]).showing(&["w"]).animating();
    let work = m.render_work.clone();
    let mut core = core_of(vec![m]);
    let log = RenderLog::default();
    let visible = visible(&["t/w"]);
    let t0 = Instant::now();
    tick_and_apply(&mut core, &log, t0, &visible);
    work.store(late_render_millis(), Ordering::SeqCst);

    // The animating frame stays while 5 late ones are dropped.
    let mut update = CoreUpdate::None;
    for i in 1..=5 {
        let at = t0 + Duration::from_millis(20 * i);
        assert!(logged(&log, || core.widget_tick(at, &visible)), "{i}");
        update = apply_until(&mut core, rendered);
    }

    assert!(
        matches!(update, CoreUpdate::WidgetStopped { .. }),
        "{update:?}"
    );
    assert!(!logged(&log, || {
        core.widget_tick(t0 + Duration::from_millis(200), &visible)
    }));
    assert_eq!(log.count(), 6);
}

/// The UI stops a widget whose canvas paints too slowly: like a widget
/// that drew late too often, with a toast and a log line, and its tile
/// leaves the strip with its last frame, so nothing of it paints again.
#[test]
fn stop_widget_clears_an_always_tiles_frame_and_reports_it() {
    let mut core = core_of(vec![Tiles::new("fake", &["w"]).showing(&["w"])]);
    assert!(core.render_widget(&key(), Instant::now()));
    apply_until(&mut core, rendered);
    assert!(core.widget_frame(&key()).is_some());

    let update = core.stop_widget(&key(), "it paints too slowly");

    let CoreUpdate::WidgetStopped { widgets } = update else {
        panic!("not stopped: {update:?}");
    };
    assert_eq!(
        widgets,
        [StoppedWidget {
            title: "Widget w".into(),
            reason: "it paints too slowly".into(),
        }]
    );
    assert!(core.widget_frame(&key()).is_none());
    let tiles = core.strip();
    assert!(tiles.is_empty(), "the tile leaves the strip: {tiles:?}");
    assert!(!core.render_widget(&key(), Instant::now() + Duration::from_secs(1)));
    // Once is enough.
    assert!(matches!(
        core.stop_widget(&key(), "again"),
        CoreUpdate::None
    ));
    assert!(matches!(
        core.stop_widget(&wkey("fake/nope"), "unknown"),
        CoreUpdate::None
    ));
}

/// A widget stopped for drawing late keeps its last good frame: it paints
/// cheaply, and its actions still work. Only the UI's paint guard
/// (`stop_widget`) takes the frame away.
#[test]
fn a_widget_stopped_for_late_frames_keeps_its_last_frame() {
    let tiles = Tiles::new("fake", &["w"]).showing(&["w"]);
    let work = tiles.render_work.clone();
    let mut core = core_of(vec![tiles]);
    let t0 = Instant::now();
    render_and_apply(&mut core, t0);
    let drawn = core.widget_frame(&key()).expect("a frame");

    work.store(late_render_millis(), Ordering::SeqCst);
    let updates: Vec<CoreUpdate> = (1..=5)
        .map(|i| render_and_apply(&mut core, t0 + Duration::from_millis(20 * i)))
        .collect();

    assert!(
        matches!(updates[4], CoreUpdate::WidgetStopped { .. }),
        "{updates:?}"
    );
    assert_eq!(core.widget_frame(&key()), Some(drawn.clone()));
    let tiles = core.strip();
    assert_eq!(tiles[0].frame, Some(drawn));
}

/// The soft deadline is the worker thread's CPU time: a render that waits
/// (a loaded machine keeps the thread off its core) but computes little is
/// on time, however long it takes on the wall clock.
#[cfg(unix)]
#[test]
fn a_render_that_waits_but_uses_little_cpu_is_not_late() {
    let delay = Arc::new(AtomicU64::new(late_render_millis() + 20));
    let mut core = core_with(module().with_render_delay(delay));
    let started = Instant::now();

    render_and_apply(&mut core, Instant::now());

    assert!(started.elapsed() > Duration::from_millis(late_render_millis()));
    assert_eq!(
        core.widget_frame(&key()).as_deref(),
        Some(&frame("hello")),
        "the slow but idle frame is kept"
    );
}

/// A condition widget stopped for drawing late (a load spike while typing)
/// is stopped until the launcher shows again, not for the session: the
/// next search after a hide answers with its tile again. An always widget
/// stopped the same way stays stopped.
#[test]
fn a_stopped_condition_widget_draws_again_after_hide_and_show() {
    let work = Arc::new(AtomicU64::new(late_render_millis()));
    let m = FakeModule::new(vec![], nothing)
        .with_widgets(vec![decl("w")])
        .with_render(hello)
        .with_render_work(work.clone())
        .with_search(vec![])
        .with_widget_answers(vec![WidgetAnswer {
            widget: "w".into(),
            payload: "4".into(),
            actions: vec![],
        }]);
    let renders = m.renders.clone();
    let mut core = core_with(m);
    let updates: Vec<CoreUpdate> = (0..5)
        .map(|i| search_and_apply(&mut core, &format!("q{i}")))
        .collect();
    assert!(
        matches!(updates[4], CoreUpdate::WidgetStopped { .. }),
        "{:?}",
        updates[4]
    );
    work.store(0, Ordering::SeqCst);
    search_and_apply(&mut core, "q5");
    assert_eq!(renders.load(Ordering::SeqCst), 5, "stopped while shown");

    core.widgets_hidden();
    search_and_apply(&mut core, "q6");

    assert_eq!(renders.load(Ordering::SeqCst), 6);
    let strip = core.strip();
    assert_eq!(strip.len(), 1, "{strip:?}");
    assert_eq!(strip[0].condition.as_ref().unwrap().payload, "4");
    assert!(core.render_widget(&key(), Instant::now() + Duration::from_secs(1)));
}

#[test]
fn a_stopped_always_widget_stays_stopped_after_hide_and_show() {
    let tiles = Tiles::new("fake", &["w"]).showing(&["w"]);
    let work = tiles.render_work.clone();
    let mut core = core_of(vec![tiles]);
    work.store(late_render_millis(), Ordering::SeqCst);
    let t0 = Instant::now();
    let updates: Vec<CoreUpdate> = (0..5)
        .map(|i| render_and_apply(&mut core, t0 + Duration::from_millis(20 * i)))
        .collect();
    assert!(
        matches!(updates[4], CoreUpdate::WidgetStopped { .. }),
        "{updates:?}"
    );

    core.widgets_hidden();
    core.search("", now());

    assert!(!core.render_widget(&key(), t0 + Duration::from_secs(1)));
}

fn condition_tiles(core: &Core) -> Vec<String> {
    core.strip()
        .iter()
        .filter(|tile| tile.condition.is_some())
        .map(|tile| tile.key.to_string())
        .collect()
}

/// Hiding drops the last query's condition tiles: the next showing, which
/// searches the empty query, never shows the old `4` before calc answers.
#[test]
fn hiding_drops_the_condition_tiles() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    search_and_apply(&mut core, "2+2");
    assert_eq!(condition_tiles(&core), ["calc/w"]);

    core.widgets_hidden();

    assert!(condition_tiles(&core).is_empty());
    core.search("", now());
    assert!(condition_tiles(&core).is_empty());
    search_and_apply(&mut core, "");
    assert!(condition_tiles(&core).is_empty());
}

/// An answer for the last query that lands after the hide is not kept
/// either; the next search's answers are.
#[test]
fn an_answer_landing_after_the_hide_is_not_kept() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    core.search("2+2", now());
    core.widgets_hidden();

    apply_until(&mut core, answers_applied);

    assert!(condition_tiles(&core).is_empty());
    search_and_apply(&mut core, "2+2");
    assert_eq!(condition_tiles(&core), ["calc/w"]);
}

/// Runs `copy` on `key`'s tile and applies its answer.
fn copy_on(core: &mut Core, key: &str) -> CoreUpdate {
    core.send_widget_event(
        &wkey(key),
        WidgetEvent::Action {
            node: None,
            action: "copy".into(),
        },
    )
    .unwrap()
    .expect("a request");
    apply_until(core, event_finished)
}

/// A query is recorded when an action runs from it, and a tile's action is
/// one: each tile records the query it was answered for.
#[test]
fn a_tile_action_records_its_query_in_the_history() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    search_and_apply(&mut core, "2+2");
    copy_on(&mut core, "calc/w");
    search_and_apply(&mut core, "2+23");
    copy_on(&mut core, "calc/w");

    core.widgets_hidden();
    core.record_hidden_query("2+23", now());

    let queries: Vec<String> = core
        .history()
        .entries()
        .into_iter()
        .map(|entry| entry.query)
        .collect();
    assert_eq!(queries, ["2+23", "2+2"]);
}

fn pointer_on(node: &str, kind: PointerKind) -> WidgetEvent {
    WidgetEvent::Pointer(PointerEvent {
        node: node.into(),
        kind,
        at: Point { x: 1., y: 1. },
    })
}

/// Sends `event` to `key`'s tile and applies its answer.
fn press_on(core: &mut Core, key: &str, event: WidgetEvent) {
    core.send_widget_event(&wkey(key), event)
        .unwrap()
        .expect("a request");
    apply_until(core, event_finished);
}

fn history_queries(core: &Core) -> Vec<String> {
    core.history()
        .entries()
        .into_iter()
        .map(|entry| entry.query)
        .collect()
}

/// A click is a press and a release on the same node: a release on a node
/// other than the one pressed is no user action and records nothing.
#[test]
fn a_release_on_another_node_records_nothing() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    search_and_apply(&mut core, "2+2");
    press_on(&mut core, "calc/w", pointer_on("a", PointerKind::Down));
    press_on(&mut core, "calc/w", pointer_on("b", PointerKind::Up));
    // A stray release with no press before it is not a click either.
    press_on(&mut core, "calc/w", pointer_on("a", PointerKind::Up));

    assert!(
        history_queries(&core).is_empty(),
        "{:?}",
        history_queries(&core)
    );
}

/// A press and a release on the same node record the tile's query.
#[test]
fn a_click_on_a_node_records_its_query() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    search_and_apply(&mut core, "2+2");
    press_on(&mut core, "calc/w", pointer_on("a", PointerKind::Down));
    press_on(&mut core, "calc/w", pointer_on("a", PointerKind::Up));

    assert_eq!(history_queries(&core), ["2+2"]);
}

/// Hovering and an exempt provider's tile record nothing; the exempt run's
/// query is skipped by the hide that follows, as for an item's run.
#[test]
fn a_tile_action_of_an_exempt_provider_records_nothing() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    core.exempt_from_history("calc");
    search_and_apply(&mut core, "2+2");
    copy_on(&mut core, "calc/w");
    core.send_widget_event(&wkey("calc/w"), pointer(PointerKind::Move))
        .unwrap();

    core.widgets_hidden();
    assert!(!core.record_hidden_query("2+2", now()));

    assert!(core.history().entries().is_empty());
}

/// A plugin reloaded while the launcher shows a query is asked that query
/// once it starts: calc answers `2+2` again after a reload, and Enter
/// waits for it meanwhile.
#[test]
fn a_reloaded_module_answers_the_latest_search() {
    let mut core = core_of(vec![Tiles::new("calc", &["w"]).answering(calc)]);
    search_and_apply(&mut core, "2+2");

    core.register(Box::new(Tiles::new("calc", &["w"]).answering(calc)));

    assert!(core.widget_answers_pending());
    apply_until(&mut core, answers_applied);
    let strip = core.strip();
    assert_eq!(strip.len(), 1, "{strip:?}");
    let condition = strip[0].condition.as_ref().expect("a condition tile");
    assert_eq!((condition.payload.as_str(), condition.stale), ("4", false));
    assert!(!core.widget_answers_pending());
}

/// Reloaded while the launcher is hidden, it is not asked: the next showing
/// searches anew.
#[test]
fn a_module_reloaded_while_hidden_is_not_asked() {
    let tiles = Tiles::new("calc", &["w"]).answering(calc);
    let contexts = tiles.contexts.clone();
    let mut core = core_of(vec![tiles]);
    search_and_apply(&mut core, "2+2");
    core.widgets_hidden();

    let reloaded = Tiles::new("calc", &["w"]).answering(calc);
    let renders = reloaded.renders.clone();
    core.register(Box::new(reloaded));
    apply_until(&mut core, |ev| {
        matches!(ev, CoreEvent::ModuleEffects { .. })
    });
    std::thread::sleep(Duration::from_millis(20));

    assert!(!core.widget_answers_pending());
    assert_eq!(renders.load(Ordering::SeqCst), 0);
    assert_eq!(contexts.lock().unwrap().len(), 1);
}
