use tomakast_core::Scripts;
use tomakast_core::readings::{
    MAX_READINGS, QueryReading, ReadingKind, normalize_layout, query_readings,
};

const ALL: Scripts = Scripts::LATIN
    .union(Scripts::CYRILLIC)
    .union(Scripts::GEORGIAN)
    .union(Scripts::ARMENIAN);

#[test]
fn non_latin_query_gets_latin_layout_and_translit() {
    let r = query_readings("ашкуащч", ALL);
    assert_eq!(
        r[0],
        QueryReading {
            kind: ReadingKind::Layout,
            text: "firefox".into(),
            script: Scripts::LATIN
        }
    );
    assert!(
        r.iter()
            .any(|x| x.kind == ReadingKind::Translit && x.text == "ashkuashchch")
    );
}

#[test]
fn latin_query_gets_layouts_of_index_scripts_only() {
    let r = query_readings("afqks", Scripts::LATIN.union(Scripts::CYRILLIC));
    assert!(r.iter().any(|x| x.text == "файлы"));
    assert!(
        r.iter()
            .all(|x| x.script == Scripts::CYRILLIC && x.kind == ReadingKind::Layout)
    );
}

#[test]
fn readings_are_distinct_and_capped() {
    let r = query_readings("sdfg", ALL);
    assert!(r.len() <= MAX_READINGS);
    let mut t: Vec<_> = r.iter().map(|x| x.text.to_lowercase()).collect();
    t.sort();
    t.dedup();
    assert_eq!(t.len(), r.len());
    assert!(r.iter().all(|x| x.text != "sdfg"));
}

#[test]
fn readings_ignore_case() {
    assert_eq!(
        query_readings("АШКУАЩЧ", ALL)[0].text.to_lowercase(),
        "firefox"
    );
}

#[test]
fn one_letter_mixed_or_other_has_none() {
    assert!(query_readings("ф", ALL).is_empty());
    assert!(query_readings("фa", ALL).is_empty());
    assert!(query_readings("αβγ", ALL).is_empty());
}

#[test]
fn no_readings_without_letters() {
    assert!(query_readings("2+2*3", ALL).is_empty());
    assert!(query_readings("15%", ALL).is_empty());
}

#[test]
fn long_query_has_no_readings() {
    assert!(query_readings(&"ф".repeat(300), ALL).is_empty());
}

#[test]
fn normalize_layout_maps_wrong_layout() {
    assert_eq!(normalize_layout("Ашкуащч"), "firefox");
    assert_eq!(normalize_layout("Fire"), "fire");
}

#[test]
fn ukrainian_modifier_apostrophe_is_cyrillic() {
    use tomakast_core::{romanize, skeleton};
    assert_eq!(Scripts::of("мʼята").single(), Some(Scripts::CYRILLIC));
    assert!(!query_readings("мʼята", ALL).is_empty());
    for s in [romanize("мʼята"), skeleton("мʼята")] {
        assert!(s.is_ascii() && !s.contains('ʼ'), "stray char in {s:?}");
    }
}
