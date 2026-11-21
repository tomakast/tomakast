//! WIT 0.1 coverage: a scripted scenario drives kitchen-sink and calc through the host, a
//! [`Recorder`] notes what crossed the WIT boundary, and every import function, export,
//! `event` / `view-event` case, `effect` case and `view` case that `plugin.wit` declares must
//! appear in the recording. The expected names come from the parsed WIT only, so a new case
//! or function in the contract fails this test until a plugin exercises it.
//!
//! The scenario runs at least one behaviour per WIT name, not every kitchen-sink behaviour:
//! the probes and the cap, rate and budget tests have their own suites and add no WIT name.
//! It never runs the behaviours that break a plugin (`spin`, `sleep`, `panic`, `hog`,
//! `big-return`), which would end the shared instance. The `open` imports go to a recording
//! opener, never to the desktop; HTTP goes to a local server.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use support::harness::*;
use support::server::Server;
use tomakast_core::widget::{
    KeyEvent, NodeKind, Point, PointerEvent, PointerKind, WidgetEvent, WidgetImage,
};
use tomakast_core::{Effect, FieldValue, Modifiers, ViewEvent};
use tomakast_plugin_host::{DomainPattern, Opener, PreferencesHandle, Recorder};
use wit_parser::{Resolve, TypeDefKind, WorldItem};

const SINK: &str = "kitchen-sink/indexed";
const SINK_ID: &str = "kitchen-sink";
/// How long a due event may take to arrive on a loaded CI machine.
const ARRIVE: Duration = Duration::from_secs(10);

/// Every name the recorder must see, from `plugin.wit`: `import <interface>.<function>` for
/// the world's imported interfaces, `export <name>` for its exports, the cases of the
/// variants `event`, `view-event`, `effect`, `view`, `widget-event`, `node-kind`, `draw`,
/// `path-segment`, `paint` and `color`, and of the enums `pointer-kind` and `image-format`,
/// each as `<type> <case>`.
fn wit_names() -> BTreeSet<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugin-api/wit/since_v0.1.0");
    let mut resolve = Resolve::default();
    resolve.push_dir(&dir).expect("plugin.wit parses");
    let (_, world) = resolve
        .worlds
        .iter()
        .find(|(_, w)| w.name == "plugin")
        .expect("world `plugin`");
    let mut names = BTreeSet::new();
    for item in world.imports.values() {
        match item {
            WorldItem::Interface { id, .. } => {
                let interface = &resolve.interfaces[*id];
                let iface = interface.name.clone().expect("a named interface");
                for function in interface.functions.keys() {
                    names.insert(format!("import {iface}.{function}"));
                }
            }
            WorldItem::Function(f) => panic!("a bare imported function `{}`", f.name),
            WorldItem::Type { .. } => {}
        }
    }
    for item in world.exports.values() {
        match item {
            WorldItem::Function(f) => {
                names.insert(format!("export {}", f.name));
            }
            other => panic!("an export that is not a function: {other:?}"),
        }
    }
    let (_, types) = resolve
        .interfaces
        .iter()
        .find(|(_, i)| i.name.as_deref() == Some("types"))
        .expect("interface `types`");
    for variant in [
        "event",
        "view-event",
        "effect",
        "view",
        "widget-event",
        "node-kind",
        "draw",
        "path-segment",
        "paint",
        "color",
    ] {
        let id = types.types[variant];
        let TypeDefKind::Variant(v) = &resolve.types[id].kind else {
            panic!("`{variant}` is not a variant");
        };
        for case in &v.cases {
            names.insert(format!("{variant} {}", case.name));
        }
    }
    for enumeration in ["pointer-kind", "image-format"] {
        let id = types.types[enumeration];
        let TypeDefKind::Enum(e) = &resolve.types[id].kind else {
            panic!("`{enumeration}` is not an enum");
        };
        for case in &e.cases {
            names.insert(format!("{enumeration} {}", case.name));
        }
    }
    names
}

/// What reached the opener; nothing is opened.
#[derive(Default)]
struct FakeOpener(Mutex<Vec<String>>);

impl Opener for FakeOpener {
    fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(format!("url {url}"));
        Ok(())
    }

    fn open_path(&self, path: &Path) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push(format!("path {}", path.display()));
        Ok(())
    }
}

fn run(h: &mut Harness, id: &str, action: &str) -> Vec<Effect> {
    h.run(id, action)
        .unwrap_or_else(|e| panic!("{id} {action} failed: {e}"))
}

fn event(h: &mut Harness, view: &str, event: ViewEvent) -> Vec<Effect> {
    h.view_event(view, event)
        .unwrap_or_else(|e| panic!("{view} event failed: {e}"))
}

/// At least one kitchen-sink behaviour per WIT name: the imports, the exports, every event
/// and view event, and the effects calc does not return. The other behaviours (the probes, the
/// cap and rate tests) have their own tests and add no WIT name.
fn kitchen_sink(recorder: &Recorder) {
    let server = Server::start();
    let opener = Arc::new(FakeOpener::default());
    let fake: Arc<dyn Opener> = opener.clone();
    let prefs: Arc<Mutex<Option<PreferencesHandle>>> = Arc::default();
    let handle = Arc::clone(&prefs);
    let recording = recorder.clone();
    // `init` (replace-items) runs while the harness starts.
    let mut h = harness_built(
        |package| {
            package.manifest.permissions.http = vec![DomainPattern::parse("localhost").unwrap()];
        },
        BTreeMap::new(),
        move |plugin| {
            *handle.lock().unwrap() = Some(plugin.preferences());
            plugin.with_opener(fake).with_recorder(recording)
        },
    );

    // search, run.
    let echo = h.search("echo:x").expect("search answers");
    assert_eq!(echo.len(), 1);
    assert!(!run(&mut h, "kitchen-sink/hello", "run").is_empty());

    // storage and preferences, answered as detail views (push-view).
    let stored = metadata(&run(&mut h, SINK, "storage"));
    assert_eq!(stored["get"], "Ok(Some(\"v\"))", "{stored:?}");
    let read = metadata(&run(&mut h, SINK, "prefs"));
    assert_eq!(read["greeting"], "Some(\"hello\")", "{read:?}");

    // Every kind of view.
    for kind in ["list", "grid", "detail", "form", "scratchpad"] {
        assert_eq!(
            run(&mut h, SINK, &format!("view:{kind}")).len(),
            1,
            "{kind}"
        );
    }

    // The other effects: update-view, copy-text, toast, close-window (replace-items: `init`).
    run(&mut h, SINK, "updates:1");
    run(&mut h, SINK, "copy:copied");
    run(&mut h, SINK, "toasts:1");
    run(&mut h, SINK, "close-window");

    // log.
    run(&mut h, SINK, "log:covered");

    // locale; the harness's zone source cannot tell.
    let zone = metadata(&run(&mut h, SINK, "tz"));
    assert_eq!(zone["zone"], "UTC", "{zone:?}");

    // open, through the fake opener.
    let opened = metadata(&run(&mut h, SINK, "open-url:https://example.com/"));
    assert_eq!(opened["result"], "Ok(())", "{opened:?}");
    run(&mut h, SINK, "open-path:/");
    assert!(
        opener
            .0
            .lock()
            .unwrap()
            .contains(&"url https://example.com/".to_string()),
        "the fake opener got the URL"
    );

    // timers: start, the firing, cancel.
    let started = metadata(&run(&mut h, SINK, "timer:10:once"));
    assert_eq!(started["result"], "Ok(1)", "{started:?}");
    next_detail(&mut h, "timer-fired", ARRIVE).expect("the timer fires");
    run(&mut h, SINK, "cancel:1");

    // location: the request and its answer (the harness's source refuses: no real service).
    assert!(run(&mut h, SINK, "locate").is_empty());
    let located = next_detail(&mut h, "location", ARRIVE).expect("the location event arrives");
    assert!(located.contains_key("error"), "{located:?}");

    // http: fetch and its answer.
    let url = server.url("localhost", "/hello");
    let fetched = metadata(&run(&mut h, SINK, &format!("fetch:GET {url}")));
    assert!(fetched["result"].starts_with("Ok("), "{fetched:?}");
    let done = next_detail(&mut h, "http-done", ARRIVE).expect("http-done arrives");
    assert_eq!(done["status"], "200", "{done:?}");

    // Every view event; kitchen-sink answers each with a toast.
    let view = "covered";
    for view_event in [
        ViewEvent::QueryChanged {
            revision: 1,
            text: "q".into(),
            readings: vec![],
        },
        ViewEvent::ActionChosen {
            row_id: "r".into(),
            action_id: "a".into(),
        },
        ViewEvent::FormSubmitted {
            values: vec![("f".into(), FieldValue::Text("v".into()))],
        },
        ViewEvent::TextChanged {
            revision: 1,
            text: "t".into(),
        },
        ViewEvent::Closed,
    ] {
        assert!(!event(&mut h, view, view_event).is_empty());
    }

    widgets(&mut h);

    // preferences-changed, answered with an unsolicited toast.
    let handle = prefs
        .lock()
        .unwrap()
        .take()
        .expect("the preferences handle");
    handle.set(BTreeMap::from([("greeting".into(), "hi".into())]));
    loop {
        if h.toast().title.ends_with("preferences-changed") {
            break;
        }
    }
}

/// Widgets: the tiles' renders, every widget event, and the widget effects.
fn widgets(h: &mut Harness) {
    // `static` and `anim` render; `probe` in the mode `all` holds every node kind.
    assert!(h.render("static").is_some());
    let anim = h.render("anim").expect("the anim frame");
    assert!(anim.animate);
    // The images are sent once (by `init`): another frame leaves their versions alone.
    fn versions(h: &Harness) -> [Option<(u64, WidgetImage)>; 2] {
        [
            h.core.widget_image(SINK_ID, "anim-png"),
            h.core.widget_image(SINK_ID, "anim-svg"),
        ]
    }
    let before = versions(h);
    assert!(before.iter().all(Option::is_some), "{before:?}");
    h.render("anim").expect("a second anim frame");
    assert_eq!(versions(h), before);
    run(h, SINK, "render:all");
    let all = h.render("probe").expect("the probe frame");
    assert!(
        all.nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Canvas(_))),
        "{all:?}"
    );

    // Widget events: shown, hidden, an action, every kind of pointer, a key.
    let pointer = |kind| {
        WidgetEvent::Pointer(PointerEvent {
            node: "anim".into(),
            kind,
            at: Point { x: 4., y: 4. },
        })
    };
    let events = vec![
        WidgetEvent::Shown,
        WidgetEvent::Hidden,
        WidgetEvent::Action {
            node: Some("inc".into()),
            action: "inc".into(),
        },
        pointer(PointerKind::Down),
        pointer(PointerKind::Up),
        pointer(PointerKind::Move),
        pointer(PointerKind::Enter),
        pointer(PointerKind::Leave),
        WidgetEvent::Key(KeyEvent {
            node: "anim".into(),
            key: "a".into(),
            modifiers: Modifiers::empty(),
        }),
    ];
    for event in events {
        let widget = if matches!(event, WidgetEvent::Action { .. }) {
            "buttons"
        } else {
            "anim"
        };
        h.widget_event(widget, event)
            .unwrap_or_else(|e| panic!("a widget event failed: {e}"));
    }

    // The widget effects: show-widgets, redraw-widget, set-widget-image of each format.
    run(h, SINK, "show:static");
    run(h, SINK, "redraw:anim");
    for format in ["png", "jpeg", "svg"] {
        run(h, SINK, &format!("set-image:covered:{format}"));
    }

    // A widget answer of `search`.
    let tile = h.search_answer("tile:x");
    assert_eq!(tile.widgets.len(), 1, "{tile:?}");
}

/// Calc: a search answer and its copy, the Calculator with an edit, a new sheet, the history,
/// and closing the pad.
fn calc(recorder: &Recorder) {
    let recording = recorder.clone();
    let mut h = plugin_harness_built("calc", &[], move |plugin| plugin.with_recorder(recording));
    let answer = h.search_answer("2+2*3");
    assert!(answer.widgets[0].payload.starts_with("8"), "{answer:?}");
    let copy = WidgetEvent::Action {
        node: None,
        action: "copy".into(),
    };
    let copied = h.widget_event("answer", copy).expect("the tile copies");
    assert_eq!(copied, [Effect::CopyText("8".into())]);

    let pushed = run(&mut h, "calc/calculator", "run");
    let Some(Effect::PushView(tomakast_core::View::Scratchpad(pad))) = pushed.last() else {
        panic!("expected the pad, got {pushed:?}");
    };
    let pad = pad.id.0.clone();
    let typed = ViewEvent::TextChanged {
        revision: 1,
        text: "x = 5\nx * 2".into(),
    };
    assert!(!event(&mut h, &pad, typed).is_empty());
    let chosen = |action: &str| ViewEvent::ActionChosen {
        row_id: pad.clone(),
        action_id: action.into(),
    };
    // A new sheet pops the pad and pushes the new one (pop-view).
    event(&mut h, &pad, chosen("new"));
    event(&mut h, &pad, chosen("history"));
    run(&mut h, "calc/history", "run");
    event(&mut h, &pad, ViewEvent::Closed);
}

#[test]
fn every_wit_name_is_exercised() {
    let recorder = Recorder::default();
    kitchen_sink(&recorder);
    calc(&recorder);

    let expected = wit_names();
    let recorded = recorder.entries();
    let missing: Vec<&String> = expected.difference(&recorded).collect();
    assert!(
        missing.is_empty(),
        "WIT names no plugin exercised (extend kitchen-sink): {missing:?}"
    );
    // The recorder spells every name as the WIT does.
    let unknown: Vec<&String> = recorded.difference(&expected).collect();
    assert!(
        unknown.is_empty(),
        "recorded names the WIT does not declare: {unknown:?}"
    );
}
