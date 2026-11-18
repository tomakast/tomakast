use std::time::Instant;

use tomakast_core::index::Action;
use tomakast_core::{Item, ItemId, ItemKind, LocalNow, RankWeights, SearchEngine, UsageModel};

const DAY: i64 = 86_400;
const BASE_DAY: i64 = 19_000;

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("apps", local),
        title: title.to_string(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        actions: vec![Action {
            id: "launch".into(),
            title: "Launch".into(),
        }],
    }
}

fn engine(items: Vec<Item>) -> SearchEngine {
    let mut e = SearchEngine::new(UsageModel::default(), RankWeights::default());
    e.index_mut().replace_provider("apps", items);
    e
}

/// `LocalNow` at `minute` of day `BASE_DAY + day_offset`, UTC offset 0.
fn at(day_offset: i64, minute: i64) -> LocalNow {
    LocalNow::from_unix_with_offset((BASE_DAY + day_offset) * DAY + minute * 60, 0)
}

fn titles(hits: &[tomakast_core::Hit]) -> Vec<&str> {
    hits.iter().map(|h| h.item.title.as_str()).collect()
}

#[test]
fn habit_beats_slightly_better_match() {
    let mut e = engine(vec![
        item("firefox", "Firefox"),
        item("fire-alarm", "Fire Alarm"),
    ]);
    let now = at(0, 12 * 60);
    let an_hour_ago = LocalNow::from_unix_with_offset(now.unix - 3600, 0);
    let alarm = ItemId::new("apps", "fire-alarm");
    for _ in 0..5 {
        e.record_launch(&alarm, "fi", an_hour_ago);
    }
    let hits = e.search("fi", now);
    assert_eq!(hits[0].item.title, "Fire Alarm");
}

#[test]
fn empty_query_orders_by_usage_then_title() {
    let mut e = engine(vec![
        item("zed", "Zed"),
        item("alacritty", "Alacritty"),
        item("btop", "Btop"),
    ]);
    let now = at(0, 12 * 60);
    let zed = ItemId::new("apps", "zed");
    e.record_launch(&zed, "", at(-1, 12 * 60));
    e.record_launch(&zed, "", at(-2, 12 * 60));
    let hits = e.search("", now);
    assert_eq!(titles(&hits), ["Zed", "Alacritty", "Btop"]);
    assert!(hits[0].score > 0.0);
    assert_eq!(hits[1].score, 0.0);
}

#[test]
fn ties_break_by_case_insensitive_title() {
    let mut e = engine(vec![
        item("b", "beta"),
        item("a", "Alpha"),
        item("c", "Charlie"),
    ]);
    assert_eq!(
        titles(&e.search("", at(0, 0))),
        ["Alpha", "beta", "Charlie"]
    );
}

#[test]
fn time_of_day_prefers_morning_app_in_morning() {
    let mut e = engine(vec![item("a", "Morning"), item("b", "Evening")]);
    let a = ItemId::new("apps", "a");
    let b = ItemId::new("apps", "b");
    for d in 1..=10 {
        e.record_launch(&a, "", at(-d, 9 * 60));
        e.record_launch(&b, "", at(-d, 21 * 60));
    }
    let morning = e.search("", at(0, 9 * 60 + 10));
    assert_eq!(morning[0].item.title, "Morning");
    let evening = e.search("", at(0, 21 * 60 + 5));
    assert_eq!(evening[0].item.title, "Evening");
}

#[test]
fn non_matching_items_are_excluded() {
    let mut e = engine(vec![item("firefox", "Firefox"), item("zed", "Zed")]);
    assert!(e.search("zzz", at(0, 0)).is_empty());
}

#[test]
fn record_launch_returns_the_stored_record() {
    let mut e = engine(vec![item("zed", "Zed")]);
    let id = ItemId::new("apps", "zed");
    let now = at(0, 0);
    let rec = e.record_launch(&id, "ZE", now);
    assert_eq!(rec.item, id);
    assert_eq!(rec.query, "ze");
    assert_eq!(rec.at, now);
}

#[test]
#[ignore]
fn search_10k_under_5ms() {
    let items: Vec<Item> = (0..10_000)
        .map(|i| {
            item(
                &format!("app-{i}"),
                &format!("Fire Application {i} Foo Bar"),
            )
        })
        .collect();
    let mut e = engine(items);
    for i in 0..1_000 {
        let id = ItemId::new("apps", &format!("app-{}", i * 7));
        e.record_launch(&id, "fi", at(-(i % 30), (i * 13) % 1440));
    }
    let now = at(0, 12 * 60);
    for q in ["f", "fi", "fir", "fire"] {
        let mut runs: Vec<_> = (0..20)
            .map(|_| {
                let t = Instant::now();
                let hits = e.search(q, now);
                let d = t.elapsed();
                assert!(!hits.is_empty());
                d
            })
            .collect();
        runs.sort();
        let median = runs[runs.len() / 2];
        println!("query {q:?}: median {median:?}");
        assert!(median.as_millis() < 5, "query {q:?} median {median:?}");
    }
}

#[test]
fn hits_tell_dynamic_answers_from_index_matches() {
    let mut e = engine(vec![item("files", "Files")]);
    let answer = std::sync::Arc::new(Item {
        id: ItemId::new("calc", "answer"),
        ..item("answer", "= 42")
    });

    let hits = e.search_with("f", at(0, 0), &[answer]);

    let flags: Vec<(&str, bool)> = hits
        .iter()
        .map(|h| (h.item.title.as_str(), h.dynamic))
        .collect();
    assert_eq!(flags, [("= 42", true), ("Files", false)]);
}
