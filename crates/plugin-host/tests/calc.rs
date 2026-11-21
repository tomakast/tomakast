//! The calc plugin (`plugins/calc`) driven through the core, as the daemon drives it: the
//! search answer's tile, the scratchpad with its saves, the sheet history, the storage limits
//! and a reload.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use support::harness::{
    Harness, PluginSetup, home_entry, loaded_plugin, offline, plugin_harness, plugin_harness_with,
};
use tomakast_core::widget::{Frame, NodeKind, Params, WidgetEvent, params};
use tomakast_core::{Effect, LineAnnotation, ListView, ScratchpadView, Toast, View, ViewEvent};
use tomakast_plugin_host::{
    Decision, Grant, GrantDecision, Package, Permissions, PreferencesHandle, Responder,
    WidgetGrant, decide,
};

const ID: &str = "calc";
const KIB: usize = 1024;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Calc with empty storage.
fn start() -> Harness {
    seeded(&[])
}

/// Calc whose storage holds `entries` before it starts.
fn seeded(entries: &[(&str, &str)]) -> Harness {
    let entries: Vec<(&str, &[u8])> = entries.iter().map(|(k, v)| (*k, v.as_bytes())).collect();
    plugin_harness(ID, &entries)
}

/// Calc with `prefs` as the user's values.
fn with_prefs(prefs: &[(&str, &str)]) -> Harness {
    zoned(prefs, None)
}

/// Calc with `prefs` as the user's values, on a computer set to `zone` (`None`: one that
/// cannot tell).
fn zoned(prefs: &[(&str, &str)], zone: Option<&'static str>) -> Harness {
    let prefs = prefs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    plugin_harness_with(
        ID,
        PluginSetup {
            prefs,
            ..PluginSetup::default()
        },
        move |plugin| plugin.with_time_zone(Arc::new(move || zone.map(Into::into))),
    )
}

/// The widget of calc's answer.
const ANSWER: &str = "answer";

/// The value of the param `key` of a tile's `params`.
fn param<'a>(params: &'a Params, key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// What the answer tile shows for a query: the result and the input of its params, and the
/// captions of the frame the core drew for it.
#[derive(Debug)]
struct Shown {
    result: String,
    input: String,
    /// Under the input: the zone or rates note, or what was computed.
    caption: String,
    /// Under the result: its kind in words.
    kind: String,
}

/// The texts of `frame`'s text nodes, in order.
fn texts(frame: &Frame) -> Vec<&str> {
    frame
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// Calc's own calls, on the shared harness. They panic on a failed call.
trait Calc {
    fn get(&self, key: &str) -> Option<String>;
    fn answers(&mut self, query: &str) -> Vec<Shown>;
    fn drawn(&mut self, result: &str, input: &str) -> Arc<Frame>;
    fn act(&mut self, id: &str, action: &str) -> Vec<Effect>;
    fn event(&mut self, view: &str, event: ViewEvent) -> Vec<Effect>;
    fn open_calculator(&mut self) -> ScratchpadView;
    fn type_text(&mut self, view: &str, revision: u64, text: &str) -> Vec<Effect>;
    fn choose(&mut self, view: &str, row: &str, action: &str) -> Vec<Effect>;
    fn close(&mut self, view: &str);
}

impl Calc for Harness {
    fn get(&self, key: &str) -> Option<String> {
        self.stored(key)
    }

    /// Searches `query`: calc answers no rows, and at most its answer tile, drawn.
    fn answers(&mut self, query: &str) -> Vec<Shown> {
        let answer = self.search_answer(query);
        assert!(answer.items.is_empty(), "{query:?}: {answer:?}");
        assert!(answer.widgets.len() <= 1, "{query:?}: {answer:?}");
        answer
            .widgets
            .into_iter()
            .map(|tile| {
                assert_eq!(tile.widget, ANSWER, "{tile:?}");
                let (Some(result), Some(input)) =
                    (param(&tile.params, "result"), param(&tile.params, "input"))
                else {
                    panic!("the params result and input: {tile:?}");
                };
                let (result, input) = (result.to_owned(), input.to_owned());
                let (result, input) = (result.as_str(), input.as_str());
                let frame = self.drawn(result, input);
                let [_, caption, _, kind] = texts(&frame)[..] else {
                    panic!("four texts: {frame:?}");
                };
                Shown {
                    result: result.into(),
                    input: input.into(),
                    caption: caption.into(),
                    kind: kind.into(),
                }
            })
            .collect()
    }

    /// The answer tile's frame showing `result` and `input`. The search drew it; a frame
    /// late on a loaded machine is dropped, so it is drawn again, at most 3 times (below the 5
    /// late frames that stop a widget).
    fn drawn(&mut self, result: &str, input: &str) -> Arc<Frame> {
        let shows = |frame: &Frame| {
            let texts = texts(frame);
            texts.len() == 4 && texts[0] == input && texts[2] == result
        };
        let mut frame = self.frame(ANSWER);
        for _ in 0..3 {
            if let Some(frame) = frame.as_ref().filter(|f| shows(f)) {
                return Arc::clone(frame);
            }
            frame = self.render(ANSWER);
        }
        match frame {
            Some(frame) if shows(&frame) => frame,
            other => panic!("the tile of {result:?} was not drawn: {other:?}"),
        }
    }

    fn act(&mut self, id: &str, action: &str) -> Vec<Effect> {
        self.run(id, action)
            .unwrap_or_else(|e| panic!("{id} {action} failed: {e}"))
    }

    fn event(&mut self, view: &str, event: ViewEvent) -> Vec<Effect> {
        self.view_event(view, event)
            .unwrap_or_else(|e| panic!("{view} event failed: {e}"))
    }

    /// Opens the Calculator and returns the pad it shows.
    fn open_calculator(&mut self) -> ScratchpadView {
        pushed_pad(&self.act("calc/calculator", "run"))
    }

    fn type_text(&mut self, view: &str, revision: u64, text: &str) -> Vec<Effect> {
        let event = ViewEvent::TextChanged {
            revision,
            text: text.into(),
        };
        self.event(view, event)
    }

    fn choose(&mut self, view: &str, row: &str, action: &str) -> Vec<Effect> {
        let event = ViewEvent::ActionChosen {
            row_id: row.into(),
            action_id: action.into(),
        };
        self.event(view, event)
    }

    fn close(&mut self, view: &str) {
        let effects = self.event(view, ViewEvent::Closed);
        assert!(effects.is_empty(), "{effects:?}");
    }
}

fn pushed_pad(effects: &[Effect]) -> ScratchpadView {
    match effects {
        [Effect::PushView(View::Scratchpad(pad))] => pad.clone(),
        other => panic!("expected one pushed scratchpad, got {other:?}"),
    }
}

fn updated_pad(effects: &[Effect]) -> ScratchpadView {
    match effects {
        [Effect::UpdateView(View::Scratchpad(pad))] => pad.clone(),
        other => panic!("expected one scratchpad update, got {other:?}"),
    }
}

fn pushed_list(effects: &[Effect]) -> ListView {
    match effects {
        [Effect::PushView(View::List(list))] => list.clone(),
        other => panic!("expected one pushed list, got {other:?}"),
    }
}

fn toasts(effects: &[Effect]) -> Vec<&Toast> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Toast(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn note(line: u32, text: &str) -> LineAnnotation {
    LineAnnotation {
        line,
        text: text.into(),
        is_error: false,
    }
}

fn action_ids(actions: &[tomakast_core::Action]) -> Vec<(&str, &str)> {
    actions
        .iter()
        .map(|a| (a.id.as_str(), a.title.as_str()))
        .collect()
}

const PAD_ACTIONS: [(&str, &str); 3] = [
    ("copy-last", "Copy Last Result"),
    ("new", "New Sheet"),
    ("history", "History"),
];

fn rows(list: &ListView) -> Vec<(&str, &str, &str)> {
    list.rows
        .iter()
        .map(|r| {
            let subtitle = r.subtitle.as_deref().unwrap_or("");
            (r.id.as_str(), r.title.as_str(), subtitle)
        })
        .collect()
}

/// Ids in base 36, as calc counts them.
fn base36(mut n: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

#[test]
fn calc_answers_math_with_a_double_tile() {
    let mut calc = start();
    let answer = calc.search_answer("2+2*3");
    assert!(answer.items.is_empty(), "{answer:?}");
    let [tile] = &answer.widgets[..] else {
        panic!("one widget answer: {answer:?}");
    };
    assert_eq!(tile.widget, ANSWER);
    assert_eq!(
        tile.params,
        params([
            ("input".to_string(), "2+2*3".to_string()),
            ("result".to_string(), "8".to_string()),
        ]),
        "{tile:?}"
    );
    assert_eq!(
        action_ids(&tile.actions),
        [
            ("copy", "Copy Result"),
            ("copy-both", "Copy Input + Result")
        ]
    );
    for (query, result, caption, kind) in [
        ("2+2*3", "8", "Expression", "Number"),
        ("15% of 80", "12", "Expression", "Number"),
        ("1000/5", "200", "Expression", "Number"),
        ("5 km in mi", "3.106855961 mi", "Conversion", "Length"),
    ] {
        let shown = calc.answers(query);
        let [shown] = &shown[..] else {
            panic!("{query}: {shown:?}");
        };
        assert_eq!(
            (shown.result.as_str(), shown.input.as_str()),
            (result, query)
        );
        assert_eq!(
            (shown.caption.as_str(), shown.kind.as_str()),
            (caption, kind)
        );
    }
    // A query with a tab still answers on one line (a text node may not hold control
    // characters but line breaks).
    assert_eq!(calc.answers("2\t+ 2")[0].input, "2 + 2");
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
}

#[test]
fn enter_on_the_tile_copies_the_result() {
    let mut calc = start();
    let action = |action: &str| WidgetEvent::Action {
        node: None,
        action: action.into(),
    };
    let tile = &calc.search_answer("2+2*3").widgets[0];
    assert_eq!(param(&tile.params, "result"), Some("8"), "{tile:?}");
    let effects = calc.widget_event(ANSWER, action("copy")).unwrap();
    assert_eq!(effects, [Effect::CopyText("8".into())]);
    let effects = calc.widget_event(ANSWER, action("copy-both")).unwrap();
    assert_eq!(effects, [Effect::CopyText("2+2*3 = 8".into())]);
    // The answer for the latest query is the one copied.
    calc.answers("5 km in mi");
    let effects = calc.widget_event(ANSWER, action("copy")).unwrap();
    assert_eq!(effects, [Effect::CopyText("3.106855961 mi".into())]);
    // Anything else on the tile does nothing.
    assert!(
        calc.widget_event(ANSWER, action("nope"))
            .unwrap()
            .is_empty()
    );
    assert!(
        calc.widget_event(ANSWER, WidgetEvent::Shown)
            .unwrap()
            .is_empty()
    );
}

/// Added to Home, the answer tile evaluates its input afresh on each render (here the stored
/// result is old), with no search before, and Enter copies what it shows.
#[test]
fn the_answer_tile_on_home_evaluates_its_input_afresh() {
    let mut calc = start();
    calc.set_home(vec![home_entry(
        ID,
        ANSWER,
        &[("result", "5"), ("input", "2+2*3")],
    )]);
    let frame = calc.render(ANSWER).expect("the tile's frame");
    assert_eq!(texts(&frame), ["2+2*3", "Expression", "8", "Number"]);
    let actions: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        actions,
        ["copy-live", "copy-both-live"],
        "Enter on Home copies"
    );
    let action = |id: &str| WidgetEvent::Action {
        node: None,
        action: id.into(),
    };
    let effects = calc.widget_event(ANSWER, action("copy-live")).unwrap();
    assert_eq!(effects, [Effect::CopyText("8".into())]);
    let effects = calc.widget_event(ANSWER, action("copy-both-live")).unwrap();
    assert_eq!(effects, [Effect::CopyText("2+2*3 = 8".into())]);
}

/// A Home tile whose input does not evaluate now (a conversion before any rates, offline)
/// falls back to the stored result, without captions, and Enter copies that.
#[test]
fn a_home_tile_without_rates_still_draws() {
    let mut calc = start();
    calc.set_home(vec![home_entry(
        ID,
        ANSWER,
        &[("result", "1.08 USD"), ("input", "1 eur in usd")],
    )]);
    let frame = calc.render(ANSWER).expect("the tile's frame");
    assert_eq!(texts(&frame), ["1 eur in usd", "", "1.08 USD", ""]);
    let copy = WidgetEvent::Action {
        node: None,
        action: "copy-live".into(),
    };
    let effects = calc.widget_event(ANSWER, copy).unwrap();
    assert_eq!(effects, [Effect::CopyText("1.08 USD".into())]);
}

#[test]
fn the_answer_tile_renders_well_inside_16_ms() {
    // The soft deadline of a render is 16 ms of CPU time on the worker, around the call. Here
    // the wall time of the whole round trip through the core, once the 60/s slot is free;
    // best of 5. A debug build of the host gets 4 times as long, as the deadline does.
    let bound = if cfg!(debug_assertions) {
        Duration::from_millis(16 * 4)
    } else {
        Duration::from_millis(16)
    };
    let mut calc = start();
    let query = "sqrt(2) * 3 km in mi";
    assert_eq!(calc.answers(query).len(), 1);
    let best = (0..5)
        .map(|_| {
            std::thread::sleep(Duration::from_millis(20));
            let start = Instant::now();
            let frame = calc.render(ANSWER).expect("the tile's frame");
            let took = start.elapsed();
            assert_eq!(texts(&frame)[0], query);
            took
        })
        .min()
        .unwrap();
    eprintln!("the answer tile, rendered through the core: {best:?}");
    assert!(best < bound, "a render took {best:?}");
}

#[test]
fn search_ignores_non_math() {
    let mut calc = start();
    for query in [
        "2024 report",
        "10 km run",
        "1/2 price",
        "42",
        "hello",
        "1/0",
        "9^9^9^9",
        "2026-11-19",
        "11/19/2026",
        "555-1234",
    ] {
        assert!(calc.answers(query).is_empty(), "{query:?}");
    }
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
}

#[test]
fn calculator_restores_last_sheet() {
    // First run: a new, empty sheet with the pad's actions.
    let mut calc = start();
    let pad = calc.open_calculator();
    assert_eq!(pad.id.0, "sheet:0");
    assert_eq!(pad.title, "Sheet");
    assert_eq!(pad.text, "");
    assert_eq!(pad.revision, 0);
    assert!(pad.annotations.is_empty());
    assert_eq!(action_ids(&pad.actions), PAD_ACTIONS);
    calc.type_text("sheet:0", 1, "a = 2\na * 3");
    calc.close("sheet:0");
    // Opening the Calculator again shows the same sheet, evaluated.
    let pad = calc.open_calculator();
    assert_eq!(pad.id.0, "sheet:0");
    assert_eq!(pad.text, "a = 2\na * 3");
    assert_eq!(pad.annotations, [note(0, "2"), note(1, "6")]);

    // The current sheet is the one restored, not the newest.
    let mut calc = seeded(&[
        ("sheets", "b,a"),
        ("current", "a"),
        ("sheet:a", "1+1"),
        ("sheet:b", "2+2"),
        ("next-id", "c"),
    ]);
    let pad = calc.open_calculator();
    assert_eq!(pad.id.0, "sheet:a");
    assert_eq!(pad.text, "1+1");
    assert_eq!(pad.annotations, [note(0, "2")]);
}

#[test]
fn text_changed_annotates_and_saves() {
    let mut calc = start();
    calc.open_calculator();
    let before = unix_now();
    let text = "1+1\n\nx\n2 *\n";
    let pad = updated_pad(&calc.type_text("sheet:0", 3, text));
    assert_eq!(pad.id.0, "sheet:0");
    assert_eq!(pad.revision, 3, "the update answers the event's revision");
    // One annotation per non-empty line; errors are marked.
    let lines: Vec<_> = pad
        .annotations
        .iter()
        .map(|a| (a.line, a.is_error))
        .collect();
    assert_eq!(lines, [(0, false), (2, true), (3, true)]);
    assert_eq!(pad.annotations[0].text, "2");
    // An update replaces the pad's actions: they are sent again.
    assert_eq!(action_ids(&pad.actions), PAD_ACTIONS);
    // Saved at once: the text, when, and the sheet as current and newest.
    assert_eq!(calc.get("sheet:0").as_deref(), Some(text));
    let edited: i64 = calc.get("sheet-edited:0").unwrap().parse().unwrap();
    assert!((before..=unix_now()).contains(&edited), "{edited}");
    assert_eq!(calc.get("sheets").as_deref(), Some("0"));
    assert_eq!(calc.get("current").as_deref(), Some("0"));
}

#[test]
fn closed_saves_the_final_text() {
    // Review Focus: 100 keystrokes in about a second. Each one saved would pass the
    // 20-writes-per-second limit; the last edits must still reach storage by `closed`.
    let mut calc = start();
    calc.open_calculator();
    let mut text = String::new();
    let mut last = None;
    for revision in 1..=100u64 {
        text.push_str(if revision % 10 == 0 { "1\n" } else { "1+" });
        last = Some(updated_pad(&calc.type_text("sheet:0", revision, &text)));
    }
    text.push('1');
    let last_pad = updated_pad(&calc.type_text("sheet:0", 101, &text));
    // The last revision's annotations answer the last revision.
    assert_eq!(last_pad.revision, 101);
    assert_ne!(last.unwrap().annotations, last_pad.annotations);
    assert_eq!(last_pad.annotations.first(), Some(&note(0, "10")));
    assert_eq!(last_pad.annotations.last(), Some(&note(10, "1")));
    // Esc: the final text arrives only before `closed`, which saves it.
    calc.close("sheet:0");
    assert_eq!(calc.get("sheet:0").as_deref(), Some(text.as_str()));
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
    // And the next opening shows it.
    assert_eq!(calc.open_calculator().text, text);
}

#[test]
fn history_lists_sheets_newest_first() {
    let now = unix_now();
    let edited = |ago: i64| (now - ago).to_string();
    let (c, b, a, d) = (
        edited(30),
        edited(2 * 3600 + 60),
        edited(3 * 86400 + 60),
        edited(30 * 60),
    );
    let long_line = format!("{}\t{}", "9".repeat(800), "8".repeat(800));
    let mut calc = seeded(&[
        ("sheets", "c,b,d,a"),
        ("current", "c"),
        ("next-id", "e"),
        ("sheet:c", "\n  total = 5\n"),
        ("sheet:b", "\n \n"),
        ("sheet:d", &long_line),
        ("sheet:a", "x"),
        ("sheet-edited:c", &c),
        ("sheet-edited:b", &b),
        ("sheet-edited:d", &d),
        ("sheet-edited:a", &a),
    ]);
    let list = pushed_list(&calc.act("calc/history", "run"));
    assert_eq!(list.id.0, "history");
    assert_eq!(list.title, "History");
    let got = rows(&list);
    assert_eq!(got[0], ("c", "total = 5", "Edited just now"));
    assert_eq!(got[1], ("b", "Empty sheet", "Edited 2 h ago"));
    assert_eq!(got[3], ("a", "x", "Edited 3 days ago"));
    // A long first line is cut to fit a title (1 KiB, one line).
    let (id, title, subtitle) = got[2];
    assert_eq!((id, subtitle), ("d", "Edited 30 min ago"));
    assert!(title.len() <= KIB && !title.contains('\t'), "{title}");
    assert!(title.starts_with("999"));
    for row in &list.rows {
        assert_eq!(
            action_ids(&row.actions),
            [("open", "Open Sheet"), ("delete", "Delete Sheet")]
        );
    }

    // Sheets made in the launcher list newest first too, also from the pad's action.
    let mut calc = start();
    calc.open_calculator();
    calc.type_text("sheet:0", 1, "first");
    let effects = calc.choose("sheet:0", "sheet:0", "new");
    assert!(
        matches!(&effects[..], [Effect::PopView, Effect::PushView(View::Scratchpad(p))] if p.id.0 == "sheet:1" && p.text.is_empty()),
        "{effects:?}"
    );
    calc.close("sheet:0");
    calc.type_text("sheet:1", 1, "second");
    let list = pushed_list(&calc.choose("sheet:1", "sheet:1", "history"));
    let titles: Vec<_> = rows(&list).into_iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(titles, [("1", "second"), ("0", "first")]);
}

#[test]
fn open_from_history_makes_current() {
    let mut calc = seeded(&[
        ("sheets", "b,a"),
        ("current", "b"),
        ("next-id", "c"),
        ("sheet:a", "2^10"),
        ("sheet:b", "1"),
    ]);
    let pad = calc.open_calculator();
    assert_eq!(pad.id.0, "sheet:b");
    pushed_list(&calc.choose("sheet:b", "sheet:b", "history"));
    let pad = pushed_pad(&calc.choose("history", "a", "open"));
    assert_eq!(pad.id.0, "sheet:a");
    assert_eq!(pad.text, "2^10");
    assert_eq!(pad.annotations, [note(0, "1024")]);
    assert_eq!(action_ids(&pad.actions), PAD_ACTIONS);
    assert_eq!(calc.get("current").as_deref(), Some("a"));
    // Opening does not reorder the history; editing does.
    assert_eq!(calc.get("sheets").as_deref(), Some("b,a"));
    calc.type_text("sheet:a", 1, "2^11");
    assert_eq!(calc.get("sheets").as_deref(), Some("a,b"));
    // The late `closed` of the sheet below does not change what is current.
    calc.close("sheet:b");
    assert_eq!(calc.get("current").as_deref(), Some("a"));
    assert_eq!(calc.get("sheet:b").as_deref(), Some("1"));
    assert_eq!(calc.open_calculator().id.0, "sheet:a");
}

#[test]
fn delete_sheet() {
    let mut calc = seeded(&[
        ("sheets", "a,b,c"),
        ("current", "a"),
        ("next-id", "d"),
        ("sheet:a", "1"),
        ("sheet:b", "2"),
        ("sheet:c", "3"),
        ("sheet-edited:b", "1800000000"),
    ]);
    let list = pushed_list(&calc.act("calc/history", "run"));
    assert_eq!(list.rows.len(), 3);
    let effects = calc.choose("history", "b", "delete");
    let list = match &effects[..] {
        [Effect::UpdateView(View::List(list))] => list,
        other => panic!("expected the history again, got {other:?}"),
    };
    assert_eq!(list.id.0, "history");
    let ids: Vec<_> = list.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["a", "c"]);
    assert_eq!(calc.get("sheet:b"), None);
    assert_eq!(calc.get("sheet-edited:b"), None);
    assert_eq!(calc.get("sheets").as_deref(), Some("a,c"));
    // Deleting the current sheet makes the newest one left current.
    calc.choose("history", "a", "delete");
    assert_eq!(calc.get("current").as_deref(), Some("c"));
    assert_eq!(calc.open_calculator().id.0, "sheet:c");
    // With every sheet gone, the Calculator starts a new one.
    calc.choose("history", "c", "delete");
    assert_eq!(calc.get("sheets").as_deref(), Some(""));
    let pad = calc.open_calculator();
    assert_eq!((pad.id.0.as_str(), pad.text.as_str()), ("sheet:d", ""));
}

#[test]
fn a_deleted_id_is_never_reused() {
    // `next-id` is lost, and the newest sheet is deleted: going by the list alone, the next
    // sheet would take the deleted one's id, and calc would never save its edits.
    let mut calc = seeded(&[
        ("sheets", "1,0"),
        ("current", "0"),
        ("sheet:0", "a"),
        ("sheet:1", "b"),
    ]);
    pushed_list(&calc.act("calc/history", "run"));
    calc.choose("history", "1", "delete");
    assert_eq!(calc.get("sheets").as_deref(), Some("0"));
    let pad = calc.open_calculator();
    assert_eq!(pad.id.0, "sheet:0");
    let effects = calc.choose("sheet:0", "sheet:0", "new");
    let new = match &effects[..] {
        [Effect::PopView, Effect::PushView(View::Scratchpad(pad))] => pad.id.0.clone(),
        other => panic!("expected the new pad, got {other:?}"),
    };
    assert_ne!(new, "sheet:1", "the deleted sheet's id was reused");
    let effects = calc.type_text(&new, 1, "7");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Toast(_))),
        "{effects:?}"
    );
    assert_eq!(calc.get(&new).as_deref(), Some("7"));
}

#[test]
fn the_reserve_covers_a_whole_save() {
    // After a burst of typing in sheet `b`, an edit of sheet `a` waits. `closed` then saves
    // it whole: its text, `sheet-edited`, `sheets` and `current`, 4 writes of the reserve.
    let mut calc = seeded(&[
        ("sheets", "b,a"),
        ("current", "b"),
        ("next-id", "c"),
        ("sheet:a", "1"),
        ("sheet:b", "2"),
    ]);
    let began = Instant::now();
    let mut text = String::from("1");
    for revision in 1..=30u64 {
        text.push_str("+1");
        calc.type_text("sheet:b", revision, &text);
    }
    calc.type_text("sheet:a", 1, "5");
    let outran = within_window(began);
    if outran {
        assert_eq!(calc.get("sheet:a").as_deref(), Some("1"), "the edit waits");
    }
    calc.close("sheet:a");
    assert_eq!(calc.get("sheet:a").as_deref(), Some("5"));
    if outran {
        assert_eq!(calc.get("current").as_deref(), Some("a"));
        assert_eq!(calc.get("sheets").as_deref(), Some("a,b"));
    }
}

/// Calc with 100 sheets, `0` (current) to `2r`, newest first; sheet `<id>`'s text is
/// `text(id)`, and `next-id` is `2s`.
fn hundred_sheets(text: impl Fn(&str) -> String) -> Harness {
    let ids: Vec<String> = (0..100).rev().map(base36).collect();
    let sheets = ids.join(",");
    let texts: Vec<(String, String)> = ids
        .iter()
        .map(|id| (format!("sheet:{id}"), text(id)))
        .collect();
    let mut entries = vec![
        ("sheets", sheets.as_str()),
        ("current", "0"),
        ("next-id", "2s"),
    ];
    entries.extend(texts.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    seeded(&entries)
}

fn listed(calc: &Harness) -> Vec<String> {
    let stored = calc.get("sheets").unwrap_or_default();
    stored
        .split(',')
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The one toast of `effects` that tells of dropped sheets: its message.
fn drop_notice(effects: &[Effect]) -> String {
    match toasts(effects)[..] {
        [toast] => {
            assert!(toast.title.contains("Deleted"), "{toast:?}");
            toast.message.clone().unwrap_or_default()
        }
        _ => panic!("expected one toast, got {effects:?}"),
    }
}

#[test]
fn a_new_sheet_is_listed_by_its_first_text() {
    // Ruling 13: a sheet the user never types in leaves nothing behind.
    let mut calc = start();
    assert_eq!(calc.open_calculator().id.0, "sheet:0");
    // Blank text does not list it.
    let effects = calc.type_text("sheet:0", 1, " \n\t\n");
    assert!(toasts(&effects).is_empty(), "{effects:?}");
    calc.close("sheet:0");
    for key in ["sheets", "current", "sheet:0", "sheet-edited:0"] {
        assert_eq!(calc.get(key), None, "{key}");
    }
    assert!(
        pushed_list(&calc.act("calc/history", "run"))
            .rows
            .is_empty()
    );
    calc.close("history");
    // The first text does: listed, saved and current.
    assert_eq!(calc.open_calculator().id.0, "sheet:1");
    calc.type_text("sheet:1", 1, "1+1");
    assert_eq!(calc.get("sheets").as_deref(), Some("1"));
    assert_eq!(calc.get("sheet:1").as_deref(), Some("1+1"));
    assert_eq!(calc.get("current").as_deref(), Some("1"));
    // New Sheet, left without typing: not in the history, and the Calculator reopens the
    // sheet last edited.
    let effects = calc.choose("sheet:1", "sheet:1", "new");
    assert!(
        matches!(&effects[..], [Effect::PopView, Effect::PushView(View::Scratchpad(p))] if p.id.0 == "sheet:2"),
        "{effects:?}"
    );
    calc.close("sheet:1");
    calc.close("sheet:2");
    let list = pushed_list(&calc.act("calc/history", "run"));
    assert_eq!(rows(&list).iter().map(|r| r.0).collect::<Vec<_>>(), ["1"]);
    calc.close("history");
    assert_eq!(calc.get("next-id").as_deref(), Some("3"));
    let pad = calc.open_calculator();
    assert_eq!((pad.id.0.as_str(), pad.text.as_str()), ("sheet:1", "1+1"));
    // A listed sheet that is cleared stays listed: its blank text is saved.
    calc.type_text("sheet:1", 2, "");
    assert_eq!(calc.get("sheet:1").as_deref(), Some(""));
    assert_eq!(calc.get("sheets").as_deref(), Some("1"));
}

#[test]
fn hundred_sheets_cap_drops_oldest_non_current() {
    // 100 sheets, none empty; the current one is the oldest.
    let mut calc = hundred_sheets(|id| match id {
        "1" => "rent = 1200\nfood = 300".into(),
        id => id.into(),
    });
    assert_eq!(calc.open_calculator().id.0, "sheet:0");
    // New Sheet drops nothing: the new sheet is not listed yet.
    let before = calc.get("sheets");
    let effects = calc.choose("sheet:0", "sheet:0", "new");
    assert!(
        matches!(&effects[..], [Effect::PopView, Effect::PushView(View::Scratchpad(p))] if p.id.0 == "sheet:2s"),
        "{effects:?}"
    );
    assert_eq!(calc.get("sheets"), before);
    assert_eq!(calc.get("next-id").as_deref(), Some("2t"));
    assert_eq!(calc.get("current").as_deref(), Some("0"));
    // Its first text lists it: the oldest sheet is the current one, so the oldest other one
    // goes, and the user is told which.
    let effects = calc.type_text("sheet:2s", 1, "7");
    let message = drop_notice(&effects);
    assert!(message.contains("“rent = 1200”"), "{message}");
    assert!(message.contains("100 sheets"), "{message}");
    let stored = listed(&calc);
    assert_eq!(stored.len(), 100);
    assert_eq!(stored[0], "2s");
    assert!(stored.iter().any(|id| id == "0"));
    assert!(!stored.iter().any(|id| id == "1"));
    assert_eq!(calc.get("sheet:1"), None);
    assert_eq!(calc.get("sheet:0").as_deref(), Some("0"));
    assert_eq!(calc.get("sheet:2s").as_deref(), Some("7"));
    assert_eq!(calc.get("current").as_deref(), Some("2s"));
    // Listed now: the next edits drop nothing.
    let effects = calc.type_text("sheet:2s", 2, "7+1");
    assert!(toasts(&effects).is_empty(), "{effects:?}");
    assert_eq!(listed(&calc).len(), 100);
}

#[test]
fn the_cap_drops_an_empty_sheet_first() {
    let mut calc = hundred_sheets(|id| match id {
        "a" => " \n".into(),
        "b" => "\t".into(),
        id => id.into(),
    });
    calc.open_calculator();
    calc.choose("sheet:0", "sheet:0", "new");
    let effects = calc.type_text("sheet:2s", 1, "7");
    assert_eq!(
        drop_notice(&effects),
        "An empty sheet was deleted to make room. Calculator keeps at most 100 sheets."
    );
    let stored = listed(&calc);
    assert_eq!(stored.len(), 100);
    // The oldest empty one: `a` is older than `b`.
    assert!(!stored.iter().any(|id| id == "a"));
    assert!(stored.iter().any(|id| id == "b") && stored.iter().any(|id| id == "1"));
    assert_eq!(calc.get("sheet:a"), None);
}

#[test]
fn search_saves_what_waits() {
    // M3: an edit left waiting (here by a list calc cannot read) is saved by the next
    // search, which the launcher sends on every keystroke, with no user action.
    let mut calc = seeded(&[
        ("sheets", "b,a"),
        ("current", "a"),
        ("next-id", "c"),
        ("sheet:a", "1"),
        ("sheet:b", "2"),
    ]);
    calc.open_calculator();
    calc.db()
        .kv_set(ID, "sheets", &[0xff, 0xfe], unix_now())
        .unwrap();
    calc.type_text("sheet:a", 1, "1+1");
    assert_eq!(calc.get("sheet:a").as_deref(), Some("1"), "the edit waits");
    calc.db().kv_set(ID, "sheets", b"b,a", unix_now()).unwrap();
    assert!(calc.answers("2024 report").is_empty());
    assert_eq!(calc.get("sheet:a").as_deref(), Some("1+1"));
    assert_eq!(calc.get("sheets").as_deref(), Some("a,b"));
}

#[test]
fn a_save_that_needs_a_drop_waits_for_a_call_that_is_not_search() {
    // M-2: a save that search makes must not drop a sheet (picking one reads every sheet,
    // under search's 100 ms deadline). The new sheet's first text, at the cap, waits; the
    // next call that is not a search lists it, drops the oldest and tells.
    let mut calc = hundred_sheets(|id| id.into());
    calc.open_calculator();
    calc.choose("sheet:0", "sheet:0", "new");
    let good = calc.get("sheets").unwrap();
    with_unreadable_list(&mut calc, &good, |calc| {
        let effects = calc.type_text("sheet:2s", 1, "7");
        assert!(toasts(&effects).is_empty(), "{effects:?}");
    });
    assert!(calc.answers("2024 report").is_empty());
    assert_eq!(calc.get("sheets"), Some(good));
    assert_eq!(calc.get("sheet:1").as_deref(), Some("1"));
    assert_eq!(calc.get("sheet:2s"), None, "the edit waits");
    let effects = calc.type_text("sheet:2s", 2, "8");
    assert!(drop_notice(&effects).contains("“1”"), "{effects:?}");
    assert_eq!(calc.get("sheet:2s").as_deref(), Some("8"));
    assert_eq!(calc.get("sheet:1"), None);
    assert_eq!(listed(&calc).len(), 100);
}

/// Makes `sheets` unreadable while `f` runs, then stores `good` again: an edit made in `f`
/// waits in calc's memory, with nothing of it written.
fn with_unreadable_list(calc: &mut Harness, good: &str, f: impl FnOnce(&mut Harness)) {
    calc.db()
        .kv_set(ID, "sheets", &[0xff, 0xfe], unix_now())
        .unwrap();
    f(calc);
    calc.db()
        .kv_set(ID, "sheets", good.as_bytes(), unix_now())
        .unwrap();
}

/// Fills calc's storage quota to the last byte with keys calc does not know.
fn fill_quota(calc: &Harness) {
    const ROW: usize = 32;
    for n in 1.. {
        let key = format!("filler:{n}");
        let left = (1024 * KIB) as u64 - calc.db().kv_usage(ID).unwrap();
        let Some(room) = (left as usize).checked_sub(key.len() + ROW) else {
            break;
        };
        if room == 0 {
            break;
        }
        let value = "x".repeat(room.min(250 * KIB));
        calc.db()
            .kv_set(ID, &key, value.as_bytes(), unix_now())
            .unwrap();
    }
    assert_eq!(calc.db().kv_usage(ID).unwrap(), (1024 * KIB) as u64);
}

#[test]
fn a_drop_toast_comes_after_the_calls_own_toast() {
    // M-1: an action's own toast must not replace the toast of a drop its flush made.
    let mut calc = hundred_sheets(|id| id.into());
    calc.open_calculator();
    calc.choose("sheet:0", "sheet:0", "new");
    let good = calc.get("sheets").unwrap();
    with_unreadable_list(&mut calc, &good, |calc| {
        calc.type_text("sheet:2s", 1, "no math here");
    });
    let effects = calc.choose("sheet:2s", "sheet:2s", "copy-last");
    assert_eq!(calc.get("sheet:2s").as_deref(), Some("no math here"));
    let shown = toasts(&effects);
    assert_eq!(shown.len(), 2, "{effects:?}");
    assert_eq!(shown[0].title, "Calculator: No result to copy");
    let last = shown[1];
    assert!(last.title.contains("Deleted"), "{last:?}");
    assert!(last.message.as_deref().unwrap_or_default().contains("“1”"));
    assert!(matches!(effects.last(), Some(Effect::Toast(t)) if t == last));
}

#[test]
fn search_makes_no_room_when_storage_is_full() {
    // M-2: a save that search makes and that does not fit the quota waits, with no sheet
    // dropped. The next calls that are not searches drop what it needs, at most three per
    // save, and tell each drop.
    let victim = |id: &str| format!("{id}\n{}", "1+1\n".repeat(10 * KIB / 4));
    let texts: Vec<(String, String)> = (1..=5)
        .map(|n| (format!("sheet:{n}"), victim(&n.to_string())))
        .collect();
    let mut entries = vec![
        ("sheets", "0,5,4,3,2,1"),
        ("current", "0"),
        ("next-id", "6"),
        ("sheet:0", "1"),
    ];
    entries.extend(texts.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let mut calc = seeded(&entries);
    calc.open_calculator();
    // More than three sheets must go to fit it.
    let text = "2+2\n".repeat(35 * KIB / 4);
    with_unreadable_list(&mut calc, "0,5,4,3,2,1", |calc| {
        calc.type_text("sheet:0", 1, &text);
    });
    fill_quota(&calc);
    calc.answers("2024 report");
    calc.answers("2024 report");
    assert_eq!(calc.get("sheets").as_deref(), Some("0,5,4,3,2,1"));
    for (key, text) in &texts {
        assert_eq!(calc.get(key).as_ref(), Some(text), "{key}");
    }
    assert_eq!(calc.get("sheet:0").as_deref(), Some("1"), "the edit waits");
    // Three drops, and the edit still waits.
    let message = drop_notice(&calc.type_text("sheet:0", 2, &text));
    for name in ["“1”", "“2”", "“3”"] {
        assert!(message.contains(name), "{name}: {message}");
    }
    assert_eq!(calc.get("sheet:0").as_deref(), Some("1"));
    // In a new write window, the fourth, and the edit is saved.
    std::thread::sleep(Duration::from_millis(1100));
    let effects = calc.type_text("sheet:0", 3, &text);
    let message = drop_notice(&effects);
    assert!(message.starts_with("“4” was deleted"), "{message}");
    assert!(
        matches!(effects.last(), Some(Effect::Toast(_))),
        "{effects:?}"
    );
    assert_eq!(calc.get("sheet:0").as_deref(), Some(text.as_str()));
    assert_eq!(calc.get("sheets").as_deref(), Some("0,5"));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a timing test: run with --release")]
fn search_stays_within_its_deadline_at_the_cap_with_storage_full() {
    // M-2's worst case: 100 sheets of 10 KiB fill the quota, and a new sheet's first text of
    // 50 KiB waits (behind an unreadable list). Listing it needs the cap's drop and more for
    // the quota, and picking each sheet to drop reads every sheet. Search leaves all of that
    // to the next call, whose deadline is 1000 ms. Each sample on fresh storage: a search
    // after the first would have nothing left to do. Best of 3.
    let text = "2+2\n".repeat(50 * KIB / 4);
    let mut searches = Vec::new();
    let mut saves = Vec::new();
    for _ in 0..3 {
        let mut calc = hundred_sheets(|id| format!("{id}\n{}", "1+1\n".repeat(10 * KIB / 4)));
        calc.open_calculator();
        calc.choose("sheet:0", "sheet:0", "new");
        let good = calc.get("sheets").unwrap();
        with_unreadable_list(&mut calc, &good, |calc| {
            calc.type_text("sheet:2s", 1, &text);
        });
        fill_quota(&calc);
        let start = Instant::now();
        let items = calc.answers("1+1");
        searches.push(start.elapsed());
        assert_eq!(items.len(), 1);
        let start = Instant::now();
        let effects = calc.type_text("sheet:2s", 2, &text);
        saves.push(start.elapsed());
        assert!(drop_notice(&effects).contains("“1”"), "{effects:?}");
    }
    let search = searches.into_iter().min().unwrap();
    let save = saves.into_iter().min().unwrap();
    eprintln!("at the cap with storage full: search {search:?}, the next edit {save:?}");
    assert!(
        search <= Duration::from_millis(10),
        "the search took {search:?}"
    );
    assert!(
        save <= Duration::from_millis(500),
        "the edit that makes room took {save:?}"
    );
}

#[test]
fn a_pad_over_the_cap_is_pushed_cut() {
    // M4: the live text of a pad may be longer than a pushed pad may hold (256 KiB);
    // opening its sheet again pushes the first 256 KiB, not a view the host rejects.
    let mut calc = start();
    calc.open_calculator();
    let text = huge_paste();
    calc.type_text("sheet:0", 1, &text);
    pushed_list(&calc.choose("sheet:0", "sheet:0", "history"));
    let pad = pushed_pad(&calc.choose("history", "0", "open"));
    assert_eq!(pad.id.0, "sheet:0");
    assert!(pad.text.len() <= 256 * KIB, "{}", pad.text.len());
    assert!(pad.text.len() > 255 * KIB, "{}", pad.text.len());
    assert!(text.starts_with(&pad.text));
    assert_eq!(pad.annotations.len(), 10_000);
}

#[test]
fn storage_full_warns_once_and_keeps_working() {
    // 15 old sheets of 64 KiB nearly fill the 1 MiB quota.
    let old: Vec<String> = (1..=15).rev().map(base36).collect();
    let big = "1+1\n".repeat(16 * KIB);
    let mut sheets = vec!["0".to_string()];
    sheets.extend(old.iter().cloned());
    let sheets = sheets.join(",");
    let keys: Vec<String> = old.iter().map(|id| format!("sheet:{id}")).collect();
    let mut entries = vec![
        ("sheets", sheets.as_str()),
        ("current", "0"),
        ("next-id", "g"),
        ("sheet:0", "1"),
    ];
    entries.extend(keys.iter().map(|k| (k.as_str(), big.as_str())));
    let mut calc = seeded(&entries);
    calc.open_calculator();

    // A sheet of 65 lines of 1000 bytes does not fit: the oldest other sheet is dropped,
    // and the user is told which.
    let line = format!("1+1{}\n", " ".repeat(996));
    let text = line.repeat(65);
    let left = (1024 * KIB) as u64 - calc.db().kv_usage(ID).unwrap();
    assert!(text.len() as u64 > left && text.len() <= 64 * KIB);
    let effects = calc.type_text("sheet:0", 1, &text);
    let message = drop_notice(&effects);
    assert!(
        message.contains("“1+1”") && message.contains("full"),
        "{message}"
    );
    assert_eq!(calc.get("sheet:0").as_deref(), Some(text.as_str()));
    assert_eq!(calc.get("sheet:1"), None, "the oldest goes first");
    assert!(calc.get("sheet:2").is_some());
    assert!(!calc.get("sheets").unwrap().split(',').any(|id| id == "1"));

    // Full again (filled with a key calc does not know): the next oldest goes, told again
    // (every drop is), and the pad keeps answering.
    let used = calc.db().kv_usage(ID).unwrap();
    let left = (1024 * KIB) as u64 - used;
    let filler = "x".repeat(left as usize - "filler".len() - 32);
    calc.db()
        .kv_set(ID, "filler", filler.as_bytes(), unix_now())
        .unwrap();
    let text = format!("{text}4*4\n");
    let effects = calc.type_text("sheet:0", 2, &text);
    assert!(drop_notice(&effects).contains("“1+1”"), "{effects:?}");
    let pad = match &effects[..] {
        [Effect::UpdateView(View::Scratchpad(pad)), Effect::Toast(_)] => pad,
        other => panic!("{other:?}"),
    };
    assert_eq!(pad.annotations[0], note(0, "2"));
    assert_eq!(pad.annotations.last(), Some(&note(65, "16")));
    assert_eq!(calc.get("sheet:0").as_deref(), Some(text.as_str()));
    assert_eq!(calc.get("sheet:2"), None);
    assert!(calc.get("sheet:3").is_some());

    // Full with nothing calc may drop: the sheet is not saved, the user is told once, and
    // the pad keeps computing.
    // A value is at most 256 KiB: four of 250 KiB.
    let filler = "x".repeat(250 * KIB);
    let mut calc = seeded(&[
        ("sheets", "0"),
        ("current", "0"),
        ("next-id", "1"),
        ("sheet:0", "1"),
        ("filler:1", &filler),
        ("filler:2", &filler),
        ("filler:3", &filler),
        ("filler:4", &filler),
    ]);
    calc.open_calculator();
    let text = line.repeat(60);
    let effects = calc.type_text("sheet:0", 1, &text);
    let warnings = toasts(&effects);
    assert_eq!(warnings.len(), 1, "{effects:?}");
    let message = warnings[0].message.as_deref().unwrap_or_default();
    assert!(message.contains("not saved"), "{message}");
    assert_eq!(calc.get("sheet:0").as_deref(), Some("1"));
    let effects = calc.type_text("sheet:0", 2, &format!("{text}5*5"));
    assert!(toasts(&effects).is_empty(), "{effects:?}");
    assert_eq!(
        updated_pad(&effects).annotations.last(),
        Some(&note(60, "25"))
    );
    // Filled to the last byte, a new sheet opens (nothing of it is stored yet), but its
    // first text cannot be listed: the user gets calc's own message, not the host's text,
    // and nothing is left behind.
    let used = calc.db().kv_usage(ID).unwrap();
    let left = (1024 * KIB) as u64 - used;
    let last = "x".repeat(left as usize - "filler:5".len() - 32);
    calc.db()
        .kv_set(ID, "filler:5", last.as_bytes(), unix_now())
        .unwrap();
    let effects = calc.choose("sheet:0", "sheet:0", "new");
    assert!(
        matches!(&effects[..], [Effect::PopView, Effect::PushView(View::Scratchpad(p))] if p.id.0 == "sheet:1"),
        "{effects:?}"
    );
    let effects = calc.type_text("sheet:1", 1, "7");
    let warnings = toasts(&effects);
    assert_eq!(warnings.len(), 1, "{effects:?}");
    let message = warnings[0].message.as_deref().unwrap_or_default();
    assert_eq!(
        message,
        "This sheet is not saved. Delete sheets in History to make room."
    );
    assert_eq!(calc.get("sheets").as_deref(), Some("0"));
    assert_eq!(calc.get("sheet:1"), None);
}

#[test]
fn new_sheet_makes_room_when_storage_is_full() {
    // An old sheet fills what the quota leaves: listing a new sheet, at its first text,
    // drops it, and says so.
    let big = "1+1\n".repeat(16 * KIB);
    let filler = "x".repeat(250 * KIB);
    let mut calc = seeded(&[
        ("sheets", "0,1"),
        ("current", "0"),
        ("next-id", "2"),
        ("sheet:0", "1"),
        ("sheet:1", &big),
        ("filler:1", &filler),
        ("filler:2", &filler),
        ("filler:3", &filler),
    ]);
    let used = calc.db().kv_usage(ID).unwrap();
    let left = (1024 * KIB) as u64 - used;
    let last = "x".repeat(left as usize - "filler:4".len() - 32);
    calc.db()
        .kv_set(ID, "filler:4", last.as_bytes(), unix_now())
        .unwrap();
    calc.open_calculator();
    let effects = calc.choose("sheet:0", "sheet:0", "new");
    assert!(
        matches!(&effects[..], [Effect::PopView, Effect::PushView(View::Scratchpad(p))] if p.id.0 == "sheet:2"),
        "{effects:?}"
    );
    assert_eq!(calc.get("sheets").as_deref(), Some("0,1"));
    let message = drop_notice(&calc.type_text("sheet:2", 1, "5"));
    assert!(
        message.contains("“1+1”") && message.contains("full"),
        "{message}"
    );
    assert_eq!(calc.get("sheets").as_deref(), Some("2,0"));
    assert_eq!(calc.get("sheet:2").as_deref(), Some("5"));
    assert_eq!(calc.get("sheet:1"), None);
    assert_eq!(calc.get("current").as_deref(), Some("2"));
}

#[test]
fn oversized_sheet_saves_truncated_and_warns_once() {
    let mut calc = start();
    calc.open_calculator();
    // Over 64 KiB, with a multi-byte character across the cut.
    let text = format!("{}é\n1+1", "1".repeat(64 * KIB - 1));
    let effects = calc.type_text("sheet:0", 1, &text);
    let warnings = toasts(&effects);
    assert_eq!(warnings.len(), 1, "{effects:?}");
    assert!(warnings[0].title.contains("too long"), "{:?}", warnings[0]);
    // Computed in full, saved cut on a character boundary.
    let pad = effects
        .iter()
        .find_map(|e| match e {
            Effect::UpdateView(View::Scratchpad(p)) => Some(p),
            _ => None,
        })
        .unwrap();
    assert_eq!(pad.annotations.last(), Some(&note(1, "2")));
    let saved = calc.get("sheet:0").unwrap();
    assert_eq!(saved, "1".repeat(64 * KIB - 1));
    // The same sheet warns only once.
    let effects = calc.type_text("sheet:0", 2, &format!("{text}\n2+2"));
    assert!(toasts(&effects).is_empty(), "{effects:?}");
}

#[test]
fn reload_restores_text() {
    // The plugin is reloaded while a sheet is open: no `closed` reaches the old instance.
    let mut calc = start();
    calc.open_calculator();
    calc.type_text("sheet:0", 1, "2^10");
    calc.reload();
    let pad = calc.open_calculator();
    assert_eq!(pad.id.0, "sheet:0");
    assert_eq!(pad.text, "2^10");
    assert_eq!(pad.annotations, [note(0, "1024")]);
}

#[test]
fn quick_edits_are_saved_without_closed() {
    // Ruling 8: at a normal typing pace every edit is saved at once, so a reload (or a
    // break) with no `closed` keeps the last one.
    let mut calc = start();
    calc.open_calculator();
    calc.type_text("sheet:0", 1, "2^1");
    std::thread::sleep(Duration::from_millis(40));
    calc.type_text("sheet:0", 2, "2^10");
    assert_eq!(calc.get("sheet:0").as_deref(), Some("2^10"));
    calc.reload();
    assert_eq!(calc.open_calculator().text, "2^10");
}

/// Whether the writes since `began` all fall in calc's 1 s write window (with a margin), so
/// a burst really outran the budget. On a slow machine or a loaded debug run it may not:
/// then only what holds at any speed is checked (the final text is saved).
fn within_window(began: Instant) -> bool {
    let within = began.elapsed() < Duration::from_millis(700);
    if !within {
        eprintln!(
            "the burst took {:?}: not checking that it outran the budget",
            began.elapsed()
        );
    }
    within
}

#[test]
fn a_burst_of_actions_and_typing_still_saves_on_closed() {
    // Two new sheets (each several writes) and fast typing within a second use the write
    // budget up; the reserve still lets `closed` save the final text.
    let mut calc = start();
    calc.open_calculator();
    calc.type_text("sheet:0", 1, "1");
    let began = Instant::now();
    calc.choose("sheet:0", "sheet:0", "new");
    calc.choose("sheet:1", "sheet:1", "new");
    let mut text = String::new();
    for revision in 1..=30u64 {
        text.push_str("1+");
        calc.type_text("sheet:2", revision, &format!("{text}1"));
    }
    let text = format!("{text}1");
    if within_window(began) {
        assert_ne!(
            calc.get("sheet:2").as_deref(),
            Some(text.as_str()),
            "the burst outran the budget"
        );
    }
    calc.close("sheet:2");
    assert_eq!(calc.get("sheet:2").as_deref(), Some(text.as_str()));
    assert_eq!(calc.get("sheet:0").as_deref(), Some("1"));
    // Sheet 1 was never typed in, so it was never listed.
    assert_eq!(calc.get("sheets").as_deref(), Some("2,0"));
}

#[test]
fn copy_last_saves_what_waits() {
    // Typing faster than the budget leaves the newest text waiting; an action saves it
    // before it acts.
    let mut calc = start();
    calc.open_calculator();
    let began = Instant::now();
    let mut text = String::from("1");
    for revision in 1..=30u64 {
        text.push_str("+1");
        calc.type_text("sheet:0", revision, &text);
    }
    if within_window(began) {
        assert_ne!(calc.get("sheet:0").as_deref(), Some(text.as_str()));
    }
    let effects = calc.choose("sheet:0", "sheet:0", "copy-last");
    assert_eq!(effects, [Effect::CopyText("31".into())]);
    assert_eq!(calc.get("sheet:0").as_deref(), Some(text.as_str()));
}

#[test]
fn unreadable_storage_writes_nothing() {
    // A read error (here a value that is not UTF-8) must not let calc write from a list it
    // could not read: that would drop every other sheet from the history.
    let mut calc = seeded(&[
        ("sheets", "b,a"),
        ("current", "a"),
        ("next-id", "c"),
        ("sheet:a", "1"),
        ("sheet:b", "2"),
    ]);
    calc.open_calculator();
    calc.db()
        .kv_set(ID, "sheets", &[0xff, 0xfe], unix_now())
        .unwrap();
    calc.type_text("sheet:a", 1, "1+1");
    let raw = |calc: &Harness| calc.db().kv_get(ID, "sheets").unwrap().unwrap();
    assert_eq!(raw(&calc), [0xff, 0xfe], "sheets is not rewritten");
    assert_eq!(calc.get("sheet:a").as_deref(), Some("1"), "the edit waits");
    // Actions fail with a toast and write nothing.
    for (view, row, action) in [
        ("sheet:a", "sheet:a", "new"),
        ("history", "b", "delete"),
        ("sheet:a", "sheet:a", "history"),
    ] {
        let effects = calc.choose(view, row, action);
        let warnings = toasts(&effects);
        assert_eq!(warnings.len(), 1, "{action}: {effects:?}");
        let message = warnings[0].message.as_deref().unwrap_or_default();
        assert_eq!(message, "Calculator storage could not be read.", "{action}");
    }
    assert_eq!(calc.get("next-id").as_deref(), Some("c"));
    assert_eq!(calc.get("sheet:b").as_deref(), Some("2"));
    // Readable again: the waiting edit is saved by the next call.
    calc.db().kv_set(ID, "sheets", b"b,a", unix_now()).unwrap();
    calc.close("sheet:a");
    assert_eq!(calc.get("sheet:a").as_deref(), Some("1+1"));
    assert_eq!(calc.get("sheets").as_deref(), Some("a,b"));
}

/// 3 MiB of short lines, and a last line, far past line 10 000, that would be 4.
fn huge_paste() -> String {
    "12345 * 6789 + 1\n".repeat(3 * 1024 * KIB / 17) + "2+2"
}

#[test]
fn a_huge_paste_is_evaluated_only_as_far_as_shown() {
    let mut calc = start();
    calc.open_calculator();
    let effects = calc.type_text("sheet:0", 1, &huge_paste());
    let pad = effects
        .iter()
        .find_map(|e| match e {
            Effect::UpdateView(View::Scratchpad(p)) => Some(p),
            _ => None,
        })
        .expect("the pad is updated");
    assert_eq!(pad.annotations.len(), 10_000);
    assert_eq!(pad.annotations[9_999], note(9_999, "83810206"));
    // Copy Last Result copies the last annotated value: the last line is not evaluated.
    let effects = calc.choose("sheet:0", "sheet:0", "copy-last");
    assert_eq!(effects, [Effect::CopyText("83810206".into())]);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a timing test: run with --release")]
fn a_huge_paste_stays_within_the_deadline() {
    // Evaluating only the 10 000 lines that get annotations takes a few tens of ms; all
    // 185 000 lines would take a quarter of a second. Best of 3 round trips.
    let mut calc = start();
    calc.open_calculator();
    let text = huge_paste();
    let best = (1..=3)
        .map(|revision| {
            let start = Instant::now();
            calc.type_text("sheet:0", revision, &text);
            start.elapsed()
        })
        .min()
        .unwrap();
    assert!(
        best <= Duration::from_millis(100),
        "a 3 MiB paste took {best:?}"
    );
}

#[test]
fn calculator_reopens_the_sheet_last_edited() {
    let mut calc = seeded(&[
        ("sheets", "b,a"),
        ("current", "b"),
        ("next-id", "c"),
        ("sheet:a", "1"),
        ("sheet:b", "2"),
    ]);
    assert_eq!(calc.open_calculator().id.0, "sheet:b");
    // Pad b, then History, open a, Esc, Esc: back on pad b.
    pushed_list(&calc.choose("sheet:b", "sheet:b", "history"));
    assert_eq!(
        pushed_pad(&calc.choose("history", "a", "open")).id.0,
        "sheet:a"
    );
    calc.close("sheet:a");
    calc.close("history");
    assert_eq!(calc.get("current").as_deref(), Some("a"));
    // Editing b makes it current again: the Calculator reopens it.
    calc.type_text("sheet:b", 1, "2+2");
    assert_eq!(calc.get("current").as_deref(), Some("b"));
    calc.close("sheet:b");
    let pad = calc.open_calculator();
    assert_eq!((pad.id.0.as_str(), pad.text.as_str()), ("sheet:b", "2+2"));
}

#[test]
fn a_deleted_sheet_stays_deleted() {
    let mut calc = seeded(&[
        ("sheets", "a,b"),
        ("current", "a"),
        ("next-id", "c"),
        ("sheet:a", "1"),
        ("sheet:b", "2"),
    ]);
    calc.open_calculator();
    pushed_list(&calc.choose("sheet:a", "sheet:a", "history"));
    calc.choose("history", "a", "delete");
    calc.close("history");
    // Back on the pad of the deleted sheet: it still computes, but edits are not saved
    // (they would bring the sheet back), and the user is told once.
    let effects = calc.type_text("sheet:a", 1, "1+1");
    let warnings = toasts(&effects);
    assert_eq!(warnings.len(), 1, "{effects:?}");
    assert_eq!(warnings[0].title, "Calculator: This sheet was deleted");
    let pad = effects
        .iter()
        .find_map(|e| match e {
            Effect::UpdateView(View::Scratchpad(p)) => Some(p),
            _ => None,
        })
        .expect("the pad is updated");
    assert_eq!(pad.annotations, [note(0, "2")]);
    let effects = calc.type_text("sheet:a", 2, "1+2");
    assert!(toasts(&effects).is_empty(), "{effects:?}");
    calc.close("sheet:a");
    assert_eq!(calc.get("sheet:a"), None);
    assert_eq!(calc.get("sheets").as_deref(), Some("b"));
    assert_eq!(calc.get("current").as_deref(), Some("b"));
    assert_eq!(calc.open_calculator().id.0, "sheet:b");
}

#[test]
fn history_ignores_rows_that_are_not_sheets() {
    let mut calc = seeded(&[("sheets", "a"), ("current", "a"), ("sheet:a", "1")]);
    for (row, action) in [("../x", "open"), ("A", "delete"), ("a,b", "open")] {
        assert!(calc.choose("history", row, action).is_empty(), "{row}");
    }
    assert_eq!(calc.get("current").as_deref(), Some("a"));
    assert_eq!(calc.get("sheets").as_deref(), Some("a"));
}

#[test]
fn copy_last_needs_user_action() {
    let mut calc = start();
    calc.open_calculator();
    let effects = calc.type_text("sheet:0", 1, "1+1\n3*3\nnope\n");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::CopyText(_))),
        "{effects:?}"
    );
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
    // Chosen from the action panel: the last result, skipping errors and empty lines.
    let effects = calc.choose("sheet:0", "sheet:0", "copy-last");
    assert_eq!(effects, [Effect::CopyText("9".into())]);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a timing test: run with --release")]
fn pad_and_search_stay_within_budget() {
    // The plan's margins: a 1000-line sheet recomputes in at most 50 ms and a 1 KiB query
    // answers in at most 10 ms. Measured here around the whole round trip through the core
    // (the guest's share is smaller); the best of a few runs, to keep a busy machine out.
    let mut calc = start();
    calc.open_calculator();
    let text: String = (0..1000)
        .map(|i| format!("x{i} = {i} * 2 + sqrt({i}) km in m\n"))
        .collect();
    let best = (1..=5)
        .map(|revision| {
            let start = Instant::now();
            let pad = updated_pad(&calc.type_text("sheet:0", revision, &text));
            let took = start.elapsed();
            assert_eq!(pad.annotations.len(), 1000);
            took
        })
        .min()
        .unwrap();
    eprintln!("1000-line sheet, round trip through the core: {best:?}");
    assert!(
        best <= Duration::from_millis(50),
        "1000 lines took {best:?}"
    );
    // Spaces inside, so that trimming keeps the query at 1 KiB.
    let tail = " * 3 km in mi";
    let query = format!("sqrt(2){}{tail}", " ".repeat(1024 - 7 - tail.len()));
    assert_eq!(query.len(), 1024);
    let query = query.as_str();
    let best = (0..5)
        .map(|_| {
            let start = Instant::now();
            let items = calc.answers(query);
            let took = start.elapsed();
            assert_eq!(items.len(), 1);
            took
        })
        .min()
        .unwrap();
    eprintln!("1 KiB search query, round trip through the core: {best:?}");
    assert!(
        best <= Duration::from_millis(10),
        "a 1 KiB query took {best:?}"
    );
}

#[test]
fn now_uses_the_time_zone_preference() {
    for (zone, suffix) in [
        (None, " UTC"),
        (Some("Asia/Tokyo"), " JST"),
        (Some("Moscow"), " MSK"),
        (Some("europe/moscow"), " MSK"),
        (Some("Mars/Base"), " UTC"),
        (Some(""), " UTC"),
    ] {
        let mut calc = match zone {
            None => start(),
            Some(z) => with_prefs(&[("timezone", z)]),
        };
        let title = calc.answers("now + 0 min")[0].result.clone();
        assert!(title.ends_with(suffix), "{zone:?}: {title}");
        assert_eq!(title.split(", ").count(), 3, "{title}");
    }
}

#[test]
fn a_preference_change_moves_now() {
    let handle: Arc<Mutex<Option<PreferencesHandle>>> = Arc::default();
    let keep = Arc::clone(&handle);
    let mut calc = plugin_harness_with(ID, PluginSetup::default(), move |plugin| {
        *keep.lock().unwrap() = Some(plugin.preferences());
        plugin
    });
    assert!(calc.answers("now + 0 min")[0].result.ends_with(" UTC"));
    let handle = handle
        .lock()
        .unwrap()
        .take()
        .expect("the preferences handle");
    handle.set(BTreeMap::from([(
        "timezone".into(),
        "Europe/Moscow".into(),
    )]));
    calc.settle(Duration::from_millis(50));
    assert!(calc.answers("now + 0 min")[0].result.ends_with(" MSK"));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
}

#[test]
fn date_answers_name_their_zone() {
    let mut calc = start();
    assert_eq!(calc.answers("days until 1 jan")[0].caption, "UTC");
    let mut tokyo = with_prefs(&[("timezone", "Asia/Tokyo")]);
    assert_eq!(tokyo.answers("days until 1 jan")[0].caption, "Asia/Tokyo");
}

#[test]
fn search_answers_time_zones() {
    let mut calc = start();
    assert!(calc.answers("now in Tokyo")[0].result.ends_with(" JST"));
    assert!(calc.answers("now")[0].result.ends_with(" UTC"));
    assert_eq!(calc.answers("today")[0].caption, "UTC");
    assert_eq!(calc.answers("today")[0].kind, "Date and time");
    assert!(calc.answers("31 dec").is_empty() && calc.answers("5pm").is_empty());
    assert_eq!(calc.answers("days until 1 jan").len(), 1);
}

#[test]
fn an_empty_timezone_preference_uses_the_computers_zone() {
    for prefs in [&[][..], &[("timezone", "")], &[("timezone", "  ")]] {
        let mut calc = zoned(prefs, Some("Asia/Tokyo"));
        let now = calc.answers("now")[0].result.clone();
        assert!(now.ends_with(" JST"), "{prefs:?}: {now}");
        // A date names the zone it read.
        assert_eq!(calc.answers("today")[0].caption, "Asia/Tokyo", "{prefs:?}");
    }
}

#[test]
fn the_timezone_preference_overrides_the_computers_zone() {
    let mut calc = zoned(&[("timezone", "Europe/Moscow")], Some("Asia/Tokyo"));
    let now = calc.answers("now")[0].result.clone();
    assert!(now.ends_with(" MSK"), "{now}");
    // A preference that names no zone is UTC, whatever the computer's.
    let mut calc = zoned(&[("timezone", "Mars/Base")], Some("Asia/Tokyo"));
    let now = calc.answers("now")[0].result.clone();
    assert!(now.ends_with(" UTC"), "{now}");
}

#[test]
fn an_unknown_computer_zone_falls_back_to_utc() {
    for zone in [None, Some("Mars/Base")] {
        let mut calc = zoned(&[], zone);
        let now = calc.answers("now")[0].result.clone();
        assert!(now.ends_with(" UTC"), "{zone:?}: {now}");
    }
}

#[test]
fn a_change_of_the_computers_zone_is_followed() {
    let zone = Arc::new(Mutex::new("Asia/Tokyo"));
    let source = Arc::clone(&zone);
    let mut calc = plugin_harness_with(ID, PluginSetup::default(), move |plugin| {
        plugin.with_time_zone(Arc::new(move || Some((*source.lock().unwrap()).into())))
    });
    assert!(calc.answers("now")[0].result.ends_with(" JST"));
    *zone.lock().unwrap() = "Europe/Moscow";
    assert!(calc.answers("now")[0].result.ends_with(" MSK"));
}

/// The two rate tables the tests share, as calc stores them, fetched `age` seconds ago.
fn seeded_rates(age: i64) -> Vec<(&'static str, String)> {
    let fetched = unix_now() - age;
    vec![
        (
            "rates:fiat",
            format!("{fetched} USD=1 EUR=0.92 RUB=93.125 GBP=0.79 JPY=150"),
        ),
        ("rates:crypto", format!("{fetched} BTC=67000 ETH=3500")),
    ]
}

fn with_rates(age: i64) -> Harness {
    seeded_with(&seeded_rates(age))
}

fn seeded_with(rates: &[(&str, String)]) -> Harness {
    let entries: Vec<(&str, &str)> = rates.iter().map(|(k, v)| (*k, v.as_str())).collect();
    seeded(&entries)
}

#[test]
fn stored_rates_convert_in_search_and_sheets() {
    let rates = seeded_rates(60);
    let mut calc = seeded_with(&rates);
    let items = calc.answers("100 usd in rub");
    assert_eq!(items[0].result, "9 312.50 RUB");
    assert_eq!(
        (items[0].input.as_str(), items[0].caption.as_str()),
        ("100 usd in rub", "Conversion")
    );
    assert_eq!(items[0].kind, "Currency");
    assert_eq!(calc.answers("1 eth in btc")[0].result, "0.052238806 BTC");
    // A bare amount is not an answer.
    assert!(calc.answers("100 usd").is_empty() && calc.answers("$100").is_empty());
    calc.open_calculator();
    let pad = updated_pad(&calc.type_text("sheet:0", 1, "100 usd in rub\n$100 to €"));
    assert_eq!(
        pad.annotations,
        [note(0, "9 312.50 RUB"), note(1, "92.00 EUR")]
    );
    // Search wrote nothing for rates, and nothing came unasked.
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
    assert_eq!(calc.get("rates:fiat"), Some(rates[0].1.clone()));
    assert_eq!(calc.get("rates:crypto"), Some(rates[1].1.clone()));
}

/// A sheet answers what search answers, with the rates and the zone search reads, and keeps
/// money, units and percentages in its variables; a bare value shows its value there.
#[test]
fn a_sheet_computes_what_search_does_across_its_lines() {
    let seed = seeded_rates(60)
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into_bytes()))
        .collect();
    let prefs = [("timezone".to_string(), "Europe/Moscow".to_string())].into();
    let mut calc = plugin_harness_with(
        ID,
        PluginSetup {
            seed,
            prefs,
            ..PluginSetup::default()
        },
        |plugin| plugin,
    );
    calc.open_calculator();
    let text = "rent = 1200 eur\nfood = 300 usd\nrent + food in rub\n10 km\n\
                speed = 90 km/h\nt = 2 h\nspeed * t in km\nnow\n5pm UTC in Tokyo\n\
                salary = 5000 usd\ntax = 13%\nsalary - tax";
    let pad = updated_pad(&calc.type_text("sheet:0", 1, text));
    let shown: Vec<_> = pad
        .annotations
        .iter()
        .map(|a| (a.line, a.text.as_str(), a.is_error))
        .collect();
    assert_eq!(shown.len(), 12, "{shown:?}");
    assert!(shown.iter().all(|(_, _, error)| !error), "{shown:?}");
    let line = |n: usize| shown[n].1;
    assert_eq!(line(2), "149 404.89 RUB");
    assert_eq!(line(3), "10 km", "a bare value shows in a sheet");
    assert_eq!(line(6), "180 km");
    assert!(
        line(7).ends_with(" MSK"),
        "the zone preference: {}",
        line(7)
    );
    assert!(line(8).ends_with(", 02:00 JST"), "{}", line(8));
    assert_eq!(line(11), "4 350.00 USD");
    // The same computations on one line, in search.
    for (query, n) in [
        ("1200 eur + 300 usd in rub", 2),
        ("90 km/h * 2 h in km", 6),
        ("5pm UTC in Tokyo", 8),
        ("5000 usd - 13%", 11),
    ] {
        assert_eq!(calc.answers(query)[0].result, line(n), "{query}");
    }
    assert!(
        calc.answers("10 km").is_empty(),
        "search hides a bare value"
    );
    // Opened again, the pushed pad reads the same.
    calc.close("sheet:0");
    let reopened = calc.open_calculator();
    assert_eq!(reopened.annotations, pad.annotations);
}

#[test]
fn rates_older_than_a_day_say_so() {
    let mut calc = with_rates(3 * 86_400 + 60);
    assert_eq!(
        calc.answers("100 usd in rub")[0].caption,
        "rates from 3 days ago"
    );
    let mut fresh = with_rates(3600);
    assert_eq!(fresh.answers("100 usd in rub")[0].caption, "Conversion");
}

#[test]
fn without_rates_a_currency_query_has_no_answer() {
    let mut calc = start();
    assert!(calc.answers("100 usd in rub").is_empty());
    assert_eq!(calc.answers("2+2")[0].result, "4");
    calc.open_calculator();
    let pad = updated_pad(&calc.type_text("sheet:0", 1, "100 usd in rub"));
    assert_eq!(pad.annotations.len(), 1);
    assert!(pad.annotations[0].is_error);
    assert_eq!(pad.annotations[0].text, "no exchange rates yet");
}

#[test]
fn malformed_stored_rates_are_ignored() {
    let mut calc = seeded(&[("rates:fiat", "garbage"), ("rates:crypto", "1 BTC=x")]);
    assert!(calc.answers("100 usd in rub").is_empty());
    assert!(calc.answers("1 btc in usd").is_empty());
    assert_eq!(calc.answers("2+2")[0].result, "4");
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
}

const FIAT_URL: &str = "https://open.er-api.com/v6/latest/USD";
const CRYPTO_PREFIX: &str = "https://api.coingecko.com/api/v3/simple/price?ids=";
const FIAT_BODY: &str = r#"{"result":"success","time_last_update_unix":1795046401,"rates":{"USD":1,"EUR":0.92,"RUB":93.125,"GBP":0.79,"JPY":150}}"#;
const CRYPTO_BODY: &str = r#"{"bitcoin":{"usd":67000},"ethereum":{"usd":3500}}"#;

/// Requests a responder saw, as `"<method> <url>"`.
type Seen = Arc<Mutex<Vec<String>>>;

/// The two rate APIs, answered with [`FIAT_BODY`] and [`CRYPTO_BODY`]; every request is
/// recorded.
fn fake_rates() -> (Responder, Seen) {
    gated_rates(Arc::new(AtomicBool::new(true)))
}

/// [`fake_rates`], failing every request (still recorded) while `open` is false.
fn gated_rates(open: Arc<AtomicBool>) -> (Responder, Seen) {
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        log.lock().unwrap().push(format!("{method} {url}"));
        if !open.load(Ordering::SeqCst) {
            return Err("offline".into());
        }
        if url == FIAT_URL {
            Ok((200, FIAT_BODY.into()))
        } else if url.starts_with(CRYPTO_PREFIX) {
            Ok((200, CRYPTO_BODY.into()))
        } else {
            Err(format!("no fake for {url}"))
        }
    });
    (responder, seen)
}

/// A responder that gives every request the same answer.
fn answering(status: u16, body: &'static str) -> Responder {
    Arc::new(move |_: &str, _: &str| Ok((status, body.into())))
}

/// The two good answers, padded with spaces (still valid JSON) to just over the 8 KiB calc
/// reads of an answer.
fn oversized() -> Responder {
    let pad = |body: &str| format!("{body:<width$}", width = 8 * KIB + 1).into_bytes();
    Arc::new(move |_: &str, url: &str| {
        let body = if url == FIAT_URL {
            FIAT_BODY
        } else {
            CRYPTO_BODY
        };
        Ok((200, pad(body)))
    })
}

/// Calc over `seed`, its requests answered by `responder`.
fn served(seed: &[(&str, String)], responder: Responder) -> Harness {
    let seed = seed
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone().into_bytes()))
        .collect();
    plugin_harness_with(
        ID,
        PluginSetup {
            seed,
            responder,
            ..PluginSetup::default()
        },
        |plugin| plugin,
    )
}

/// Settles for up to 5 s until `key` is stored; its value.
fn wait_stored(calc: &mut Harness, key: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = calc.get(key) {
            return value;
        }
        assert!(Instant::now() < deadline, "{key} was never stored");
        calc.settle(Duration::from_millis(10));
    }
}

/// Settles for up to 5 s until `seen` holds `n` requests.
fn wait_requests(calc: &mut Harness, seen: &Seen, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while seen.lock().unwrap().len() < n {
        assert!(
            Instant::now() < deadline,
            "{:?}, expected {n}",
            seen.lock().unwrap()
        );
        calc.settle(Duration::from_millis(10));
    }
}

fn no_toasts(calc: &Harness) {
    assert!(
        toasts(&calc.unsolicited).is_empty(),
        "{:?}",
        calc.unsolicited
    );
}

#[test]
fn init_fetches_rates_and_search_uses_them() {
    let (responder, seen) = fake_rates();
    let mut calc = served(&[], responder);
    let fiat = wait_stored(&mut calc, "rates:fiat");
    let crypto = wait_stored(&mut calc, "rates:crypto");
    assert!(fiat.contains("RUB=93.125"), "{fiat}");
    assert!(crypto.contains("BTC=67000"), "{crypto}");
    // Their time is calc's, not the API's update time.
    for stored in [&fiat, &crypto] {
        let fetched: i64 = stored.split(' ').next().unwrap().parse().unwrap();
        assert!((unix_now() - fetched).abs() <= 60, "{stored}");
    }
    assert_eq!(calc.answers("100 usd in rub")[0].result, "9 312.50 RUB");
    assert_eq!(calc.answers("1 eth in btc")[0].result, "0.052238806 BTC");
    // The two requests, in either order (the host's workers run them side by side).
    let mut requests = seen.lock().unwrap().clone();
    requests.sort_by_key(|r| !r.contains("er-api"));
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(requests[0], format!("GET {FIAT_URL}"));
    assert!(
        requests[1]
            .starts_with("GET https://api.coingecko.com/api/v3/simple/price?ids=bitcoin,ethereum,")
            && requests[1].ends_with("&vs_currencies=usd"),
        "{}",
        requests[1]
    );
    calc.settle(Duration::from_millis(20));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
}

#[test]
fn bad_rate_answers_store_nothing() {
    for (name, responder) in [
        ("offline", offline()),
        ("status 500", answering(500, FIAT_BODY)),
        ("html", answering(200, "<html>")),
        ("error", answering(200, r#"{"result":"error"}"#)),
        ("empty", answering(200, "{}")),
        ("over 8 KiB", oversized()),
    ] {
        let mut calc = served(&[], responder);
        calc.settle(Duration::from_millis(300));
        assert_eq!(calc.get("rates:fiat"), None, "{name}");
        assert_eq!(calc.get("rates:crypto"), None, "{name}");
        assert!(calc.answers("100 usd in rub").is_empty(), "{name}");
        assert_eq!(calc.answers("2+2")[0].result, "4", "{name}");
        assert!(
            calc.unsolicited.is_empty(),
            "{name}: {:?}",
            calc.unsolicited
        );
    }
}

#[test]
fn fresh_stored_rates_are_not_fetched_again() {
    let (responder, seen) = fake_rates();
    let mut calc = served(&seeded_rates(60), responder);
    calc.settle(Duration::from_millis(300));
    assert!(
        seen.lock().unwrap().is_empty(),
        "{:?}",
        seen.lock().unwrap()
    );
    // Fiat two hours old, crypto a minute old: only fiat is fetched.
    let (responder, seen) = fake_rates();
    let fiat = seeded_rates(2 * 3600).swap_remove(0);
    let crypto = seeded_rates(60).swap_remove(1);
    let mut calc = served(&[fiat, crypto], responder);
    wait_requests(&mut calc, &seen, 1);
    calc.settle(Duration::from_millis(300));
    assert_eq!(*seen.lock().unwrap(), [format!("GET {FIAT_URL}")]);
}

/// [`fake_rates`] whose answers wait (up to 10 s) until `open` is true: the requests stay in
/// flight meanwhile.
fn held_rates(open: Arc<AtomicBool>) -> (Responder, Seen) {
    let (answer, _) = fake_rates();
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method: &str, url: &str| {
        log.lock().unwrap().push(format!("{method} {url}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !open.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        answer(method, url)
    });
    (responder, seen)
}

/// Settles for up to 5 s until an unsolicited update of a pad matches `want`; that update.
/// Other unsolicited effects stay in `unsolicited`.
fn wait_pad_update(calc: &mut Harness, want: impl Fn(&ScratchpadView) -> bool) -> ScratchpadView {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = calc
            .unsolicited
            .iter()
            .position(|e| matches!(e, Effect::UpdateView(View::Scratchpad(pad)) if want(pad)));
        if let Some(at) = found {
            let Effect::UpdateView(View::Scratchpad(pad)) = calc.unsolicited.remove(at) else {
                unreachable!()
            };
            return pad;
        }
        assert!(
            Instant::now() < deadline,
            "no such pad update: {:?}",
            calc.unsolicited
        );
        calc.settle(Duration::from_millis(10));
    }
}

#[test]
fn new_rates_update_the_open_sheet() {
    let open = Arc::new(AtomicBool::new(false));
    let (responder, seen) = held_rates(Arc::clone(&open));
    let mut calc = served(&[], responder);
    // `init`'s two requests wait for their answers while the sheet is typed in.
    wait_requests(&mut calc, &seen, 2);
    assert_eq!(calc.open_calculator().id.0, "sheet:0");
    let pad = updated_pad(&calc.type_text("sheet:0", 1, "100 usd in rub\n2+2"));
    assert_eq!(
        pad.annotations,
        [
            LineAnnotation {
                line: 0,
                text: "no exchange rates yet".into(),
                is_error: true,
            },
            note(1, "4"),
        ]
    );
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
    open.store(true, Ordering::SeqCst);
    // The answer updates the pad unasked, at the revision the launcher last sent.
    let pad = wait_pad_update(&mut calc, |pad| {
        pad.annotations
            .first()
            .is_some_and(|a| a.text == "9 312.50 RUB")
    });
    assert_eq!(pad.id.0, "sheet:0");
    assert_eq!(pad.revision, 1);
    assert_eq!(pad.annotations, [note(0, "9 312.50 RUB"), note(1, "4")]);
    assert_eq!(
        action_ids(&pad.actions),
        [
            ("copy-last", "Copy Last Result"),
            ("new", "New Sheet"),
            ("history", "History")
        ]
    );
    calc.settle(Duration::from_millis(50));
    assert!(
        calc.unsolicited
            .iter()
            .all(|e| matches!(e, Effect::UpdateView(View::Scratchpad(_)))),
        "{:?}",
        calc.unsolicited
    );
    no_toasts(&calc);
}

#[test]
fn a_preference_change_updates_the_open_sheet() {
    let handle: Arc<Mutex<Option<PreferencesHandle>>> = Arc::default();
    let keep = Arc::clone(&handle);
    let seed = vec![
        ("sheets".to_owned(), b"0".to_vec()),
        ("current".to_owned(), b"0".to_vec()),
        ("next-id".to_owned(), b"1".to_vec()),
        ("sheet:0".to_owned(), b"now + 0 min".to_vec()),
    ];
    let mut calc = plugin_harness_with(
        ID,
        PluginSetup {
            seed,
            ..PluginSetup::default()
        },
        move |plugin| {
            *keep.lock().unwrap() = Some(plugin.preferences());
            plugin
        },
    );
    let handle = handle
        .lock()
        .unwrap()
        .take()
        .expect("the preferences handle");
    // Opened, not typed in: the pad is at revision 0.
    let pad = calc.open_calculator();
    assert!(
        pad.annotations[0].text.ends_with(" UTC"),
        "{:?}",
        pad.annotations
    );
    handle.set(BTreeMap::from([(
        "timezone".into(),
        "Europe/Moscow".into(),
    )]));
    let pad = wait_pad_update(&mut calc, |pad| {
        pad.annotations
            .first()
            .is_some_and(|a| a.text.ends_with(" MSK"))
    });
    assert_eq!((pad.id.0.as_str(), pad.revision), ("sheet:0", 0));
    // Typed in, then closed: no update.
    calc.type_text("sheet:0", 1, "now + 0 min\n1+1");
    calc.close("sheet:0");
    calc.settle(Duration::from_millis(20));
    calc.unsolicited.clear();
    handle.set(BTreeMap::from([("timezone".into(), "Asia/Tokyo".into())]));
    calc.settle(Duration::from_millis(100));
    assert!(calc.unsolicited.is_empty(), "{:?}", calc.unsolicited);
    assert!(calc.answers("now + 0 min")[0].result.ends_with(" JST"));
}

#[test]
fn rates_never_drop_a_sheet_when_storage_is_full() {
    let open = Arc::new(AtomicBool::new(false));
    let (responder, seen) = gated_rates(Arc::clone(&open));
    // An older sheet, which is not current once a new one is typed in: one a drop could take.
    let old = [
        ("sheets", "1".to_owned()),
        ("current", "1".to_owned()),
        ("next-id", "2".to_owned()),
        ("sheet:1", "rent = 1200".to_owned()),
    ];
    let mut calc = served(&old, responder);
    // The first init's two requests fail.
    wait_requests(&mut calc, &seen, 2);
    assert_eq!(calc.open_calculator().id.0, "sheet:1");
    calc.choose("sheet:1", "sheet:1", "new");
    calc.close("sheet:1");
    calc.type_text("sheet:2", 1, "1+1");
    calc.close("sheet:2");
    let before = listed(&calc);
    assert_eq!(before, ["2", "1"]);
    assert_eq!(calc.get("current").as_deref(), Some("2"));
    fill_quota(&calc);
    open.store(true, Ordering::SeqCst);
    calc.reload();
    wait_requests(&mut calc, &seen, 4);
    // The rates are in memory once an answer has been handled.
    let deadline = Instant::now() + Duration::from_secs(5);
    let title = loop {
        if let Some(item) = calc.answers("100 usd in rub").first() {
            break item.result.clone();
        }
        assert!(Instant::now() < deadline, "the rates never arrived");
        calc.settle(Duration::from_millis(10));
    };
    assert_eq!(title, "9 312.50 RUB");
    calc.settle(Duration::from_millis(100));
    assert_eq!(calc.get("rates:fiat"), None);
    assert_eq!(calc.get("rates:crypto"), None);
    assert_eq!(listed(&calc), before);
    assert_eq!(calc.get("sheet:2").as_deref(), Some("1+1"));
    assert_eq!(calc.get("sheet:1").as_deref(), Some("rent = 1200"));
    no_toasts(&calc);
}

#[test]
fn calc_asks_for_web_requests_and_timers() {
    let (_, _, dir) = loaded_plugin(ID);
    let package = Package::read(dir).unwrap();
    assert_eq!(
        package.manifest.permissions.describe(),
        [
            "Store data on this computer (up to 1 MiB)",
            "Copy text to the clipboard",
            "Run timers in the background",
            "See what you type in the launcher search",
            "Make web requests to open.er-api.com, api.coingecko.com",
        ]
    );
    // An approval of the set calc held before (another content hash) is asked again.
    let old = Permissions {
        storage: true,
        clipboard: true,
        search: true,
        ..Permissions::default()
    };
    let grant = Grant {
        plugin_id: ID.into(),
        content_hash: [!package.content_hash[0]; 32],
        permissions: old,
        name: "Calculator".into(),
        author: None,
        widgets: Vec::new(),
        decision: GrantDecision::Allow,
        decided_at: 0,
        updated_at: 0,
    };
    assert_ne!(grant.content_hash, package.content_hash);
    assert_eq!(decide(Some(&grant), &package), Decision::Ask);
    // The same grant for the current set runs: only the new permissions ask.
    let current = Grant {
        permissions: package.manifest.permissions.clone(),
        widgets: vec![WidgetGrant {
            id: ANSWER.into(),
            title: "Calculator answer".into(),
        }],
        ..grant
    };
    assert_eq!(decide(Some(&current), &package), Decision::Run);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a timing test: run with --release")]
fn zone_and_currency_queries_stay_within_budget() {
    let mut calc = with_rates(60);
    for query in ["now in New York", "100 usd in rub"] {
        let best = (0..5)
            .map(|_| {
                let start = Instant::now();
                let items = calc.answers(query);
                let took = start.elapsed();
                assert_eq!(items.len(), 1, "{query}");
                took
            })
            .min()
            .unwrap();
        eprintln!("{query:?}, round trip through the core: {best:?}");
        assert!(best <= Duration::from_millis(10), "{query:?} took {best:?}");
    }
}
