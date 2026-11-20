//! Acceptance tests for search across keyboard layouts and scripts, over a fixed corpus.

mod corpus;

use std::collections::BTreeSet;

use corpus::corpus;
use tomakast_core::{
    Hit, Item, ItemKeys, LocalNow, Matcher, Pattern, RankWeights, Scripts, SearchEngine,
    UsageModel, UsageStats, combined_score, usage_value,
};

fn engine(items: Vec<Item>) -> SearchEngine {
    let mut e = SearchEngine::new(UsageModel::default(), RankWeights::default());
    e.index_mut().replace_provider("apps", items);
    e
}

fn now() -> LocalNow {
    LocalNow::from_unix_with_offset(19_000 * 86_400 + 12 * 3_600, 0)
}

fn titles(hits: &[Hit]) -> Vec<&str> {
    hits.iter().map(|h| h.item.title.as_str()).collect()
}

/// Level 0 (layout and as typed), 1 (letter for letter), 2 (`c/k`, `x`) and 3 (digraphs,
/// dropped letters) of the spike note's "Full pair list": query, expected item.
const PAIRS: &[(&str, &str)] = &[
    // Level 0.
    ("ашкуащч", "Firefox"),
    ("еудупкфь", "Telegram"),
    ("ыдфсл", "Slack"),
    ("сфдс", "Calculator"),
    ("лщтыщду", "Konsole"),
    ("afqks", "Файлы"),
    ("yfcnhjqrb", "Настройки"),
    ("j,kfrj", "Облако"),
    ("gjxnf", "Почта"),
    ("фай", "Файлы"),
    ("настр", "Настройки"),
    // Level 1.
    ("опен", "Open"),
    ("телеграм", "Telegram"),
    ("дискорд", "Discord"),
    ("слак", "Slack"),
    ("терминал", "Terminal"),
    ("фирефокс", "Firefox"),
    ("гимп", "GIMP"),
    ("блендер", "Blender"),
    ("крита", "Krita"),
    ("постман", "Postman"),
    ("обсидиан", "Obsidian"),
    ("докер", "Docker Desktop"),
    ("дольфин", "Dolphin"),
    ("консоле", "Konsole"),
    ("fajly", "Файлы"),
    ("faily", "Файлы"),
    ("fayly", "Файлы"),
    ("nastrojki", "Настройки"),
    ("nastroiki", "Настройки"),
    ("nastroyki", "Настройки"),
    ("kalkulyator", "Калькулятор"),
    ("dokumenty", "Документы"),
    ("pochta", "Почта"),
    ("failebi", "ფაილები"),
    ("parametrebi", "პარამეტრები"),
    ("kargavorumner", "Կարգավորումներ"),
    ("fayler", "Ֆայլեր"),
    ("файлеби", "ფაილები"),
    // Level 2.
    ("калк", "Calculator"),
    ("калькулятор", "Calculator"),
    ("вскод", "Visual Studio Code"),
    ("спектакл", "Spectacle"),
    ("долфин", "Dolphin"),
    ("фотошоп", "Photoshop"),
    ("yandex", "Яндекс Музыка"),
    // Level 3.
    ("хром", "Google Chrome"),
    ("хромиум", "Chromium"),
    ("джира", "Jira"),
    ("ютуб", "YouTube"),
    ("гугл", "Google"),
    ("зум", "Zoom"),
    ("либреофис", "LibreOffice Writer"),
];

/// Pairs whose expected item is beaten by another item, as the spike saw it: query, first hit.
const FIRST_HITS: &[(&str, &str)] = &[
    ("калк", "Калькулятор"),
    ("хром", "Chromium"),
    ("yandex", "Yandex Disk"),
];

fn top(hits: &[Hit], n: usize) -> String {
    hits.iter()
        .take(n)
        .map(|h| format!("{} ({:.0})", h.item.title, h.score))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn pairs_levels_0_to_3() {
    let mut e = engine(corpus());
    let mut failures = Vec::new();
    for &(query, expected) in PAIRS {
        let hits = e.search(query, now());
        if !titles(&hits).iter().take(3).any(|t| *t == expected) {
            failures.push(format!("{query:?} → {expected:?}: {}", top(&hits, 5)));
        }
    }
    for &(query, first) in FIRST_HITS {
        let hits = e.search(query, now());
        if hits.first().map(|h| h.item.title.as_str()) != Some(first) {
            failures.push(format!("{query:?} first {first:?}: {}", top(&hits, 5)));
        }
    }
    assert!(
        failures.is_empty(),
        "failing pairs:\n{}",
        failures.join("\n")
    );
}

#[test]
fn multi_word_layout_query() {
    let mut e = engine(corpus());
    let hits = e.search("мшыгфд ыегвшщ", now());
    assert_eq!(
        titles(&hits).first(),
        Some(&"Visual Studio Code"),
        "{}",
        top(&hits, 5)
    );
}

fn latin_only(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|it| ItemKeys::new(it).scripts() == Scripts::LATIN)
        .cloned()
        .collect()
}

/// Every 1 to 6 character prefix of every word of every title, lowercased.
fn latin_prefixes(items: &[Item]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for it in items {
        for word in it.title.split_whitespace() {
            let chars: Vec<char> = word.to_lowercase().chars().collect();
            for n in 1..=chars.len().min(6) {
                out.insert(chars[..n].iter().collect());
            }
        }
    }
    out
}

/// The ranking with the query as typed only: the engine's score (`combined_score`, no usage)
/// and its tie-break (lowercase title).
fn reference(items: &[Item], query: &str) -> Vec<String> {
    let mut m = Matcher::new();
    let pattern = Pattern::parse(query);
    let weights = RankWeights::default();
    let usage = usage_value(&UsageStats::default(), &weights);
    let mut hits: Vec<(f64, String, String)> = items
        .iter()
        .filter_map(|it| {
            let s = m.score(&pattern, it, &ItemKeys::new(it))?;
            Some((
                combined_score(s, usage, &weights),
                it.title.to_lowercase(),
                it.title.clone(),
            ))
        })
        .collect();
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, _, t)| t).collect()
}

#[test]
fn latin_only_corpus_is_unchanged() {
    let items = latin_only(&corpus());
    let queries = latin_prefixes(&items);
    assert!(items.len() > 100 && queries.len() > 500);
    let mut e = engine(items.clone());
    for q in &queries {
        let got = titles(&e.search(q, now()))
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(got, reference(&items, q), "query {q:?}");
    }
}

#[test]
fn no_top1_change_for_latin_prefixes() {
    let all = corpus();
    let queries = latin_prefixes(&latin_only(&all));
    let mut e = engine(all.clone());
    let mut changed = Vec::new();
    for q in &queries {
        let got = e.search(q, now()).first().map(|h| h.item.title.clone());
        let want = reference(&all, q).into_iter().next();
        if got != want {
            changed.push(format!("{q:?}: {got:?}, as typed {want:?}"));
        }
    }
    assert!(changed.is_empty(), "top 1 changed:\n{}", changed.join("\n"));
}

/// Stress corpus of the spike: 10k items, five texts each (title and four keywords), built by
/// repeating the corpus. This is a measurement, not a gate: the 5 ms budget is checked by
/// `engine.rs`. Run: `cargo test --release -p tomakast-core -- --ignored --nocapture stress_across`.
#[test]
#[ignore]
fn stress_across_scripts() {
    use std::time::Instant;
    use tomakast_core::ItemId;

    let base = corpus();
    let items: Vec<Item> = (0..10_000)
        .map(|i| {
            let mut it = base[i % base.len()].clone();
            it.id = ItemId::new("apps", &format!("stress-{i}"));
            it.title = format!("{} {i}", it.title);
            it.keywords = (1..=4)
                .map(|k| base[(i * 7 + k * 13) % base.len()].title.clone())
                .collect();
            it
        })
        .collect();
    let mut e = engine(items);
    // `STRESS_NO_USAGE=1` skips the 1,000 launch records, to measure what usage scoring costs.
    let records = if std::env::var_os("STRESS_NO_USAGE").is_some() {
        0
    } else {
        1_000
    };
    println!("stress usage records: {records}");
    for i in 0..records {
        let id = ItemId::new("apps", &format!("stress-{}", i * 7));
        e.record_launch(
            &id,
            "fi",
            LocalNow::from_unix_with_offset(
                (19_000 - (i as i64 % 30)) * 86_400 + ((i as i64 * 13) % 1440) * 60,
                0,
            ),
        );
    }
    for word in ["firefox", "ашкуащч", "калькулятор", "fajly", "j,kfrj"] {
        let chars: Vec<char> = word.chars().collect();
        let mut medians = Vec::new();
        for n in 2..=chars.len() {
            let q: String = chars[..n].iter().collect();
            let mut runs: Vec<_> = (0..20)
                .map(|_| {
                    let t = Instant::now();
                    let hits = e.search(&q, now());
                    let d = t.elapsed();
                    std::hint::black_box(&hits);
                    d
                })
                .collect();
            runs.sort();
            medians.push(runs[runs.len() / 2]);
        }
        println!("stress {word:?}: per-prefix medians {medians:?}");
    }
}
