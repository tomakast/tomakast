//! The translator plugin (`plugins/translator`) driven through the core, as the daemon
//! drives it, with Google faked by the request's URL: the pad opens with its hint, a text
//! rests and is translated once, Ctrl+Enter before the answer says "Still translating", the
//! root phrase shows a tile that Enter copies, other queries send nothing, and no network is
//! a calm line in the pane.

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::harness::{Harness, PluginSetup, home_entry, now, plugin_harness_with};
use tomakast_core::widget::{Frame, NodeKind, Params, Slot, TileKey, WidgetEvent, WidgetKey};
use tomakast_core::{Effect, ScratchpadView, ToastKind, View, ViewEvent};
use tomakast_plugin_host::Responder;

const ID: &str = "translator";
const RUN: &str = include_str!("../../../plugins/translator/tests/fixtures/word-run-en-ru.json");
const SENTENCE: &str =
    include_str!("../../../plugins/translator/tests/fixtures/sentence-en-ru.json");
const BEZHAT: &str =
    include_str!("../../../plugins/translator/tests/fixtures/word-bezhat-ru-en.json");
const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single?";

/// Requests the fake saw, as `"<method> <url>"`.
type Seen = Arc<Mutex<Vec<String>>>;

/// Google: a word into Russian answers `run`'s dictionary, a longer text into Russian the
/// sentence, anything into English `бежать`'s answer.
fn google() -> (Responder, Seen) {
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        log.lock().unwrap().push(format!("{method} {url}"));
        let body = if !url.starts_with(ENDPOINT) {
            return Err(format!("no fake for {url}"));
        } else if url.contains("&tl=en&") {
            BEZHAT
        } else if url.contains("&dt=bd") {
            RUN
        } else {
            SENTENCE
        };
        Ok((200, body.as_bytes().to_vec()))
    });
    (responder, seen)
}

/// The translator with the system's languages English then Russian: the pair (ru, en).
fn start(responder: Responder) -> Harness {
    plugin_harness_with(
        ID,
        PluginSetup {
            responder,
            ..PluginSetup::default()
        },
        |plugin| plugin.with_languages(Arc::new(|| vec!["en_US.UTF-8".into(), "ru".into()])),
    )
}

fn open(h: &mut Harness) -> ScratchpadView {
    let effects = h.run("translator/translate", "run").unwrap();
    match &effects[..] {
        [Effect::PushView(View::Scratchpad(pad))] => pad.clone(),
        other => panic!("one pushed pad: {other:?}"),
    }
}

fn type_text(h: &mut Harness, revision: u64, text: &str) -> Vec<Effect> {
    let event = ViewEvent::TextChanged {
        revision,
        text: text.into(),
    };
    h.view_event("pad", event).unwrap()
}

fn choose(h: &mut Harness, action: &str) -> Vec<Effect> {
    let event = ViewEvent::ActionChosen {
        row_id: "pad".into(),
        action_id: action.into(),
    };
    h.view_event("pad", event).unwrap()
}

/// What a compose pad shows in its second column: the output, then the detail.
fn shown(pad: &ScratchpadView) -> String {
    pad.output.clone().unwrap_or_default() + pad.detail.as_deref().unwrap_or_default()
}

/// The next unsolicited pad update whose output (and detail) satisfies `want`, within 5 s.
fn wait_pad(h: &mut Harness, want: impl Fn(&str) -> bool) -> ScratchpadView {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = h.unsolicited.iter().position(|e| {
            matches!(e, Effect::UpdateView(View::Scratchpad(pad))
                if pad.output.is_some() && want(&shown(pad)))
        });
        if let Some(at) = found {
            let Effect::UpdateView(View::Scratchpad(pad)) = h.unsolicited.remove(at) else {
                unreachable!()
            };
            return pad;
        }
        assert!(
            Instant::now() < deadline,
            "no such pad update: {:?}",
            h.unsolicited
        );
        h.settle(Duration::from_millis(10));
    }
}

fn texts(frame: &Frame) -> Vec<String> {
    frame
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// The value of the param `key` of a tile's `params`.
fn param<'a>(params: &'a Params, key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// `effects` copy `text` and nothing else: the launcher says "Copied to clipboard" itself.
fn assert_copied(effects: &[Effect], text: &str) {
    assert!(
        matches!(effects, [Effect::CopyText(t)] if t == text),
        "{effects:?}"
    );
}

fn requests(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().clone()
}

#[test]
fn translate_opens_a_compose_pad() {
    let (responder, seen) = google();
    let mut h = start(responder);
    let pad = open(&mut h);
    assert_eq!(pad.title, "Translate");
    assert_eq!(pad.output.as_deref(), Some(""), "a compose pad, empty");
    assert_eq!(pad.placeholder, "Type or paste text to translate.");
    let actions: Vec<&str> = pad.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions[0], "copy");
    assert!(requests(&seen).is_empty());
}

#[test]
fn typing_translates_after_rest() {
    let (responder, seen) = google();
    let mut h = start(responder);
    open(&mut h);
    let effects = type_text(&mut h, 1, "hello");
    assert!(
        matches!(&effects[..], [Effect::UpdateView(View::Scratchpad(p))] if p.busy && p.title == "Translate"),
        "{effects:?}"
    );
    let pad = wait_pad(&mut h, |out| out.starts_with("бегать"));
    assert_eq!(pad.revision, 1);
    assert_eq!(pad.title, "Translate");
    assert!(!pad.busy, "the answer is in");
    let from = pad.toolbar.iter().find(|b| b.id == "from-lang").unwrap();
    assert_eq!(from.label, "English (detected)");
    let seen = requests(&seen);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0].starts_with("POST https://translate.googleapis.com/"),
        "{seen:?}"
    );
    // Copy now has the translation, and Recent keeps the text.
    assert_copied(&choose(&mut h, "copy"), "бегать");
    assert_copied(&choose(&mut h, "copy-source"), "hello");
    assert!(h.stored("recent").unwrap().contains("\"hello\""));
}

/// Review Focus 1: Ctrl+Enter right after typing, before the answer.
#[test]
fn copy_before_the_answer_says_still_translating() {
    let (responder, _) = google();
    let mut h = start(responder);
    open(&mut h);
    type_text(&mut h, 1, "hello");
    let effects = choose(&mut h, "copy");
    assert!(
        // A toast that goes by itself (review finding 5): a progress one would stay.
        matches!(&effects[..], [Effect::Toast(t)]
            if t.title == "Still translating" && t.kind == ToastKind::Success),
        "{effects:?}"
    );
    wait_pad(&mut h, |out| out.starts_with("бегать"));
    // A new edit: its translation is not in yet.
    type_text(&mut h, 2, "hello there friend now");
    assert!(matches!(&choose(&mut h, "copy")[..], [Effect::Toast(_)]));
}

#[test]
fn a_russian_text_goes_to_english() {
    let (responder, seen) = google();
    let mut h = start(responder);
    open(&mut h);
    type_text(&mut h, 1, "бежать");
    let pad = wait_pad(&mut h, |out| out.starts_with("run"));
    let to = pad.toolbar.iter().find(|b| b.id == "to-lang").unwrap();
    assert_eq!(to.label, "English");
    assert!(requests(&seen)[0].contains("&tl=en&"));
}

#[test]
fn the_phrase_shows_a_tile_and_enter_copies() {
    let (responder, seen) = google();
    let mut h = start(responder);
    let answer = h.search_answer("hello in russian");
    assert!(answer.items.is_empty());
    let [tile] = &answer.widgets[..] else {
        panic!("one tile: {answer:?}");
    };
    assert_eq!(tile.widget, "translation");
    assert_eq!(param(&tile.params, "text"), Some("hello"));
    assert_eq!(param(&tile.params, "to"), Some("ru"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let shown = loop {
        let shown = h
            .render("translation")
            .map(|f| texts(&f))
            .unwrap_or_default();
        if shown.contains(&"бегать".to_string()) {
            break shown;
        }
        assert!(Instant::now() < deadline, "the tile shows {shown:?}");
        h.settle(Duration::from_millis(20));
    };
    assert_eq!(shown, ["hello", "English (detected)", "бегать", "Russian"]);
    let copy = WidgetEvent::Action {
        node: None,
        action: "copy".into(),
    };
    assert_copied(&h.widget_event("translation", copy).unwrap(), "бегать");
    assert_eq!(requests(&seen).len(), 1);
}

#[test]
fn other_queries_send_nothing() {
    let (responder, seen) = google();
    let mut h = start(responder);
    for query in ["firefox", "go to port", "hello", "log in", "check in gym"] {
        let answer = h.search_answer(query);
        assert!(answer.widgets.is_empty(), "{query}: {answer:?}");
    }
    h.settle(Duration::from_millis(500));
    assert!(requests(&seen).is_empty(), "{:?}", requests(&seen));
}

#[test]
fn offline_is_calm() {
    let mut h = start(support::harness::offline());
    open(&mut h);
    type_text(&mut h, 1, "hello");
    let pad = wait_pad(&mut h, |out| out == "No connection.");
    assert_eq!(pad.title, "Translate");
    assert!(h.take_toast().is_none(), "no toast");
    // Still alive: recent translations open.
    let effects = choose(&mut h, "recent");
    assert!(
        matches!(&effects[..], [Effect::PushView(View::List(_))]),
        "{effects:?}"
    );
}

/// The texts the Home tile showing `text` draws once it shows `want`, within 5 s.
fn wait_home_tile(h: &mut Harness, text: &str, want: &str) -> Vec<String> {
    let params = h
        .core
        .home()
        .iter()
        .find(|e| e.params.iter().any(|(k, v)| k == "text" && v == text))
        .map(|e| e.params.clone())
        .expect("the tile is on Home");
    let key = TileKey {
        widget: WidgetKey::new(ID, "translation"),
        params,
        slot: Slot::Home(0),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let shown = h.render_tile(&key).map(|f| texts(&f)).unwrap_or_default();
        if shown.iter().any(|t| t == want) {
            return shown;
        }
        assert!(Instant::now() < deadline, "{text} shows {shown:?}");
        h.settle(Duration::from_millis(20));
    }
}

/// Review findings 2 and 6: six Home tiles at once, more than the host's four requests, all
/// translate (none says "Offline"), are stored, and draw from storage after a reload with no
/// request.
#[test]
fn many_home_tiles_translate_and_keep() {
    let (responder, seen) = google();
    let mut h = start(responder);
    let words = ["one", "two", "three", "four", "five", "six"];
    let home = words
        .iter()
        .map(|w| {
            home_entry(
                ID,
                "translation",
                &[("text", w), ("from", ""), ("to", "ru")],
            )
        })
        .collect();
    h.set_home(home);
    h.core.search("", now());
    for w in words {
        wait_home_tile(&mut h, w, "бегать");
    }
    assert_eq!(requests(&seen).len(), 6, "{:?}", requests(&seen));
    assert_eq!(h.stored("home").unwrap().lines().count(), 6);

    h.reload();
    h.core.search("", now());
    for w in words {
        wait_home_tile(&mut h, w, "бегать");
    }
    assert_eq!(requests(&seen).len(), 6, "drawn from storage");
}

/// A language chosen from the pad's bar translates the text again into it, and the bar shows
/// it.
#[test]
fn choosing_a_target_from_the_bar() {
    let (responder, seen) = google();
    let mut h = start(responder);
    open(&mut h);
    type_text(&mut h, 1, "hello");
    wait_pad(&mut h, |out| out.starts_with("бегать"));
    let event = ViewEvent::ActionChosen {
        row_id: "de".into(),
        action_id: "to-lang".into(),
    };
    let effects = h.view_event("pad", event).unwrap();
    let [Effect::UpdateView(View::Scratchpad(pad))] = &effects[..] else {
        panic!("the pad, updated: {effects:?}");
    };
    let to = pad.toolbar.iter().find(|b| b.id == "to-lang").unwrap();
    assert_eq!(
        (to.label.as_str(), to.selected.as_deref()),
        ("German", Some("de"))
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !requests(&seen).iter().any(|r| r.contains("&tl=de&")) {
        assert!(Instant::now() < deadline, "{:?}", requests(&seen));
        h.settle(Duration::from_millis(10));
    }
    assert_eq!(h.stored("target").as_deref(), Some("de"));
}
