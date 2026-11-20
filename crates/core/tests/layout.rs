use tomakast_core::layout::Layout;
use tomakast_core::{LAYOUTS, Scripts};

fn layout(name: &str) -> &'static Layout {
    LAYOUTS.iter().find(|l| l.name == name).unwrap()
}

#[test]
fn ru_to_latin() {
    assert_eq!(layout("ru").to_latin("ашкуащч").as_deref(), Some("firefox"));
}
#[test]
fn ru_punctuation_keys() {
    assert_eq!(layout("ru").from_latin("j,kfrj"), "облако");
}
#[test]
fn ru_shift_level() {
    assert_eq!(layout("ru").to_latin("АШКУАЩЧ").as_deref(), Some("FIREFOX"));
}
#[test]
fn national_letters_differ() {
    assert_eq!(layout("ua").from_latin("s"), "і");
    assert_eq!(layout("ru").from_latin("s"), "ы");
}
#[test]
fn unknown_letter_is_none() {
    assert_eq!(layout("ru").to_latin("файлі"), None);
}
#[test]
fn every_script_has_a_table() {
    for s in [Scripts::CYRILLIC, Scripts::GEORGIAN, Scripts::ARMENIAN] {
        assert!(LAYOUTS.iter().any(|l| l.script == s));
    }
}
#[test]
fn scripts_of_text() {
    assert_eq!(
        Scripts::of("Параметры NVIDIA"),
        Scripts::CYRILLIC.union(Scripts::LATIN)
    );
    assert_eq!(Scripts::of("2+2"), Scripts::default());
    assert_eq!(Scripts::of("ფაილი").single(), Some(Scripts::GEORGIAN));
}
#[test]
fn tables_are_sorted_and_ordered() {
    for l in LAYOUTS {
        assert!(l.to_us.windows(2).all(|w| w[0].0 < w[1].0), "{}", l.name);
    }
    let names: Vec<_> = LAYOUTS.iter().map(|l| l.name).collect();
    assert_eq!(names[..7], ["ru", "ua", "by", "kz", "kg", "tj", "uz"]);
    let pos = |n| names.iter().position(|x| *x == n).unwrap();
    assert!(pos("ge") < pos("am"));
}
#[test]
fn const_scripts_and_letter_count() {
    const ALL: Scripts = Scripts::LATIN.union(Scripts::CYRILLIC);
    assert!(ALL.contains(Scripts::LATIN));
    assert!(!ALL.contains(Scripts::GEORGIAN));
    assert_eq!(ALL.single(), None);
    assert_eq!(Scripts::of("日本").single(), Some(Scripts::OTHER));
    assert_eq!(tomakast_core::letter_count("a1ж-"), 2);
}
#[test]
fn iso_extra_key_prefers_the_letter() {
    assert_eq!(layout("ru").from_latin(">"), "Ю");
    assert_eq!(layout("ua").from_latin("<"), "Б");
    assert_eq!(layout("ru").to_latin("Ю").as_deref(), Some(">"));
}
