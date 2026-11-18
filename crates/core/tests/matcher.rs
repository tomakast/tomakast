use tomakast_core::{Action, Item, ItemId, ItemKind, Matcher, Pattern};

fn item(title: &str, keywords: &[&str]) -> Item {
    Item {
        id: ItemId::new("apps", title),
        title: title.to_string(),
        subtitle: None,
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
        icon: None,
        kind: ItemKind::Application,
        actions: vec![Action {
            id: "open".into(),
            title: "Open".into(),
        }],
    }
}

#[test]
fn title_match_scores_and_non_match_is_none() {
    let mut m = Matcher::new();
    let it = item("Firefox", &[]);
    assert!(m.score(&Pattern::parse("fire"), &it).is_some());
    assert!(m.score(&Pattern::parse("zzz"), &it).is_none());
}

#[test]
fn keyword_match_found_but_below_equal_title_match() {
    let mut m = Matcher::new();
    let p = Pattern::parse("browser");
    let a = m.score(&p, &item("Browser", &[])).expect("title match");
    let b = m
        .score(&p, &item("Firefox", &["browser"]))
        .expect("keyword match");
    assert!(a > b);
}

#[test]
fn matches_cyrillic_and_case_insensitive() {
    let mut m = Matcher::new();
    assert!(
        m.score(&Pattern::parse("фай"), &item("Файлы", &[]))
            .is_some()
    );
    assert!(
        m.score(&Pattern::parse("FIRE"), &item("Firefox", &[]))
            .is_some()
    );
}

#[test]
fn empty_pattern_matches_everything_with_zero() {
    let mut m = Matcher::new();
    let p = Pattern::parse("");
    assert!(p.is_empty());
    assert_eq!(m.score(&p, &item("Firefox", &[])), Some(0));
}
