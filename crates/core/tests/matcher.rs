use std::time::Instant;

use tomakast_core::{
    Action, Item, ItemId, ItemKeys, ItemKind, Matcher, Pattern, Scripts, TextKeys,
};

fn item(title: &str, keywords: &[&str]) -> Item {
    Item {
        id: ItemId::new("apps", title),
        title: title.to_string(),
        subtitle: None,
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
        icon: None,
        kind: ItemKind::Application,
        source: None,
        actions: vec![Action {
            id: "open".into(),
            title: "Open".into(),
            shortcut: None,
        }],
    }
}

fn score(m: &mut Matcher, p: &Pattern, it: &Item) -> Option<u32> {
    m.score(p, it, &ItemKeys::new(it))
}

fn score_text(m: &mut Matcher, p: &Pattern, text: &str) -> Option<u32> {
    m.score_text(p, text, &TextKeys::new(text))
}

#[test]
fn title_match_scores_and_non_match_is_none() {
    let mut m = Matcher::new();
    let it = item("Firefox", &[]);
    assert!(score(&mut m, &Pattern::parse("fire"), &it).is_some());
    assert!(score(&mut m, &Pattern::parse("zzz"), &it).is_none());
}

#[test]
fn keyword_match_found_but_below_equal_title_match() {
    let mut m = Matcher::new();
    let p = Pattern::parse("browser");
    let a = score(&mut m, &p, &item("Browser", &[])).expect("title match");
    let b = score(&mut m, &p, &item("Firefox", &["browser"])).expect("keyword match");
    assert!(a > b);
}

#[test]
fn matches_cyrillic_and_case_insensitive() {
    let mut m = Matcher::new();
    assert!(score(&mut m, &Pattern::parse("фай"), &item("Файлы", &[])).is_some());
    assert!(score(&mut m, &Pattern::parse("FIRE"), &item("Firefox", &[])).is_some());
}

#[test]
fn empty_pattern_matches_everything_with_zero() {
    let mut m = Matcher::new();
    let p = Pattern::parse("");
    assert!(p.is_empty());
    assert_eq!(score(&mut m, &p, &item("Firefox", &[])), Some(0));
}

#[test]
fn score_text_matches_plain_text() {
    let mut m = Matcher::new();
    assert!(score_text(&mut m, &Pattern::parse("gam"), "Gamma").is_some());
    assert!(score_text(&mut m, &Pattern::parse("zzz"), "Gamma").is_none());
    assert_eq!(score_text(&mut m, &Pattern::parse(""), "Gamma"), Some(0));
}

#[test]
fn layout_reading_matches_with_penalty() {
    let mut m = Matcher::new();
    let ff = item("Firefox", &[]);
    let typed = m
        .score(&Pattern::parse("firefox"), &ff, &ItemKeys::new(&ff))
        .unwrap();
    let p = Pattern::across("ашкуащч", Scripts::LATIN, &mut m);
    assert_eq!(m.score(&p, &ff, &ItemKeys::new(&ff)), Some(typed * 7 / 8));
}

#[test]
fn skeleton_matches_across_scripts_only() {
    let mut m = Matcher::new();
    let chrome = item("Google Chrome", &[]);
    let p = Pattern::across("хром", Scripts::LATIN, &mut m);
    assert!(m.score(&p, &chrome, &ItemKeys::new(&chrome)).is_some());
    let calc = item("calc", &[]);
    let p = Pattern::across("kalk", Scripts::LATIN, &mut m); // Latin vs Latin: no skeleton
    assert!(m.score(&p, &calc, &ItemKeys::new(&calc)).is_none());
}

#[test]
fn as_typed_wins_at_equal_quality() {
    let mut m = Matcher::new();
    let files = item("Файлы", &[]);
    let p = Pattern::across("фай", Scripts::CYRILLIC.union(Scripts::LATIN), &mut m);
    let typed = m
        .score(&Pattern::parse("фай"), &files, &ItemKeys::new(&files))
        .unwrap();
    assert_eq!(m.score(&p, &files, &ItemKeys::new(&files)), Some(typed));
}

#[test]
fn scattered_cross_script_hit_is_dropped() {
    let mut m = Matcher::new();
    let both = Scripts::CYRILLIC.union(Scripts::LATIN);
    let scattered = item("Азбука сосна", &[]); // skeleton "azbuka sosna": a..b..s
    let contiguous = item("Абсент", &[]); // skeleton "absent"
    let p = Pattern::across("abs", both, &mut m);
    assert!(
        m.score(&p, &scattered, &ItemKeys::new(&scattered))
            .is_none()
    );
    assert!(
        m.score(&p, &contiguous, &ItemKeys::new(&contiguous))
            .is_some()
    );
}

#[test]
fn blank_query_has_no_readings() {
    let mut m = Matcher::new();
    let p = Pattern::across("   ", Scripts::LATIN.union(Scripts::CYRILLIC), &mut m);
    assert!(p.is_empty() && p.readings().is_empty());
}

#[test]
fn keyword_layout_hit_gets_both_factors() {
    let mut m = Matcher::new();
    let it = item("Web", &["firefox"]);
    let typed = score_text(&mut m, &Pattern::parse("firefox"), "firefox").unwrap();
    let p = Pattern::across("ашкуащч", Scripts::LATIN, &mut m);
    assert_eq!(score(&mut m, &p, &it), Some(typed * 7 / 8 * 3 / 4));
}

#[test]
fn alt_skeleton_matches() {
    // `й` romanizes as `j` only in the alternate skeleton.
    let mut m = Matcher::new();
    let p = Pattern::across("nastrojki", Scripts::LATIN.union(Scripts::CYRILLIC), &mut m);
    assert!(score_text(&mut m, &p, "Настройки").is_some());
}

#[test]
fn yo_written_as_e_matches() {
    // `ё` is often written `е`: `otchet` finds `Отчёт`.
    let mut m = Matcher::new();
    let p = Pattern::across("otchet", Scripts::LATIN.union(Scripts::CYRILLIC), &mut m);
    assert!(score_text(&mut m, &p, "Отчёт.pdf").is_some());
}

#[test]
fn yo_and_short_i_alternates_all_match() {
    // `ё` as `yo` with `й` as `j`, `ё` as `е`, and both.
    let mut m = Matcher::new();
    let scripts = Scripts::LATIN.union(Scripts::CYRILLIC);
    for (query, text) in [
        ("zelyonyj", "Зелёный"),
        ("zelenyj", "Зелёный"),
        ("zelenyy", "Зелёный"),
        ("tyomnyj", "Тёмный"),
        ("temnyj", "Тёмный"),
        ("nastrojki", "Настройки"),
    ] {
        let p = Pattern::across(query, scripts, &mut m);
        assert!(score_text(&mut m, &p, text).is_some(), "{query} -> {text}");
    }
}

#[test]
fn query_with_empty_skeleton_matches_nothing_across() {
    let mut m = Matcher::new();
    let p = Pattern::across("ьъ", Scripts::LATIN.union(Scripts::CYRILLIC), &mut m);
    assert!(score_text(&mut m, &p, "Firefox").is_none());
}

#[test]
fn skeleton_skips_the_query_script_part_of_a_mixed_text() {
    let mut m = Matcher::new();
    let both = Scripts::LATIN.union(Scripts::CYRILLIC);
    // The whole-text skeleton would be "kalk test": a Latin-to-Latin phonetic hit.
    let p = Pattern::across("kalk", both, &mut m);
    assert!(score_text(&mut m, &p, "Calc Тест").is_none());
}

#[test]
fn skeleton_reaches_the_other_script_part_of_a_mixed_text() {
    let mut m = Matcher::new();
    let both = Scripts::LATIN.union(Scripts::CYRILLIC);
    let p = Pattern::across("хром", both, &mut m);
    assert!(score_text(&mut m, &p, "Google Chrome Тест").is_some());
    let p = Pattern::across("calc", both, &mut m);
    assert!(score_text(&mut m, &p, "Калькулятор Pro").is_some());
}

#[test]
#[ignore]
fn score_text_2000_rows_under_5ms() {
    let texts: Vec<String> = (0..2_000)
        .map(|i| {
            if i % 10 == 0 {
                format!("Файлы окно {i} настройки")
            } else {
                format!("Firefox Application {i} Foo Bar")
            }
        })
        .collect();
    let keys: Vec<TextKeys> = texts.iter().map(|t| TextKeys::new(t)).collect();
    let scripts = keys
        .iter()
        .fold(Scripts::default(), |acc, k| acc.union(k.scripts()));
    let mut m = Matcher::new();
    let query: Vec<char> = "ашкуащч".chars().collect();
    for n in 2..=query.len() {
        let q: String = query[..n].iter().collect();
        let mut runs: Vec<_> = (0..20)
            .map(|_| {
                let t = Instant::now();
                let p = Pattern::across(&q, scripts, &mut m);
                let hits = texts
                    .iter()
                    .zip(&keys)
                    .filter(|(text, k)| m.score_text(&p, text, k).is_some())
                    .count();
                let d = t.elapsed();
                assert!(hits > 0, "query {q:?} matched nothing");
                d
            })
            .collect();
        runs.sort();
        let median = runs[runs.len() / 2];
        println!("query {q:?}: median {median:?}");
        assert!(median.as_millis() < 5, "query {q:?} median {median:?}");
    }
}
