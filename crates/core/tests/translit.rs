use tomakast_core::{romanize, skeleton, skeleton_alt, skeleton_alts};

#[test]
fn skeleton_examples() {
    for (a, b) in [
        ("хром", "hrom"),
        ("chrome", "hrome"),
        ("калькулятор", "kalkulator"),
        ("Calculator", "kalkulator"),
        ("ютуб", "iutub"),
        ("YouTube", "iutube"),
        ("джира", "jira"),
        ("Jira", "jira"),
        ("гугл", "gugl"),
        ("Google", "gugle"),
        ("опен", "open"),
        ("Файлы", "faili"),
        ("fayly", "faili"),
        ("ფაილები", "pailebi"),
        ("Կարգավորումներ", "kargavorumner"),
    ] {
        assert_eq!(skeleton(a), b, "{a}");
    }
}

#[test]
fn alternate_skeleton() {
    assert_eq!(skeleton_alt("Настройки").as_deref(), Some("nastrojki"));
    assert_eq!(skeleton_alt("ფაილები").as_deref(), Some("failebi"));
    assert_eq!(skeleton_alt("ᲤᲐᲘᲚᲔᲑᲘ").as_deref(), Some("failebi"));
    assert_eq!(skeleton_alt("НАСТРОЙКИ").as_deref(), Some("nastrojki"));
    assert_eq!(skeleton_alt("Firefox"), None);
    assert_eq!(skeleton_alt("Хром"), None);
}

#[test]
fn alternate_skeletons_are_independent() {
    // `й` as `j` and `ё` as `е` are separate alternates, and both together.
    assert_eq!(skeleton_alts("Настройки"), vec!["nastrojki"]);
    assert_eq!(skeleton_alts("Отчёт"), vec!["ohet"]);
    assert_eq!(skeleton_alts("ЁЛКА"), vec!["elka"]);
    assert_eq!(
        skeleton_alts("Зелёный"),
        vec!["zelonij", "zeleni", "zelenij"]
    );
    assert_eq!(skeleton_alts("ფაილები"), vec!["failebi"]);
    assert!(skeleton_alts("Хром").is_empty());
    assert!(skeleton_alts("").is_empty());
    // The first alternate is `skeleton_alt`, unchanged: `ё` stays `yo`.
    assert_eq!(skeleton_alt("Зелёный").as_deref(), Some("zelonij"));
    assert_eq!(skeleton_alt("Отчёт"), None);
}

#[test]
fn romanize_is_readable() {
    assert_eq!(romanize("Хром"), "hrom");
    assert_eq!(romanize("щука"), "shchuka");
}

#[test]
fn spaces_digits_and_punctuation_pass_through() {
    assert_eq!(romanize("Яндекс Музыка 2"), "yandeks muzyka 2");
    assert_eq!(skeleton("Файлы и папки"), "faili i papki");
    assert_eq!(skeleton("KDE-Connect"), "kde-konekt");
}

#[test]
fn hard_and_soft_signs_vanish() {
    assert_eq!(romanize("объём"), "obyom");
    assert_eq!(romanize("пароль"), "parol");
}

#[test]
fn national_cyrillic_letters() {
    assert_eq!(romanize("їжак"), "yizhak");
    assert_eq!(romanize("ґанок"), "ganok");
    assert_eq!(romanize("ўзбек"), "uzbek");
    assert_eq!(romanize("қазақ"), "kazak");
    assert_eq!(romanize("ҳамид"), "hamid");
    assert_eq!(romanize("өнер"), "oner");
    assert_eq!(romanize("үй"), "uy");
}

#[test]
fn georgian_capitals_and_letters() {
    assert_eq!(romanize("პარამეტრები"), "parametrebi");
    assert_eq!(romanize("ᲤᲐᲘᲚᲔᲑᲘ"), "pailebi");
    assert_eq!(romanize("შეტყობინება"), "shetqobineba");
}

#[test]
fn armenian_digraph_and_letters() {
    assert_eq!(romanize("Կարգավորումներ"), "kargavorumner");
    assert_eq!(romanize("Ֆայլեր"), "fayler");
    assert_eq!(skeleton("Ֆայլեր"), "failer");
}

#[test]
fn latin_goes_straight_to_folding() {
    assert_eq!(romanize("Firefox"), "firefox");
    assert_eq!(skeleton("Firefox"), "firefoks");
    assert_eq!(skeleton("school"), "shul");
    assert_eq!(skeleton("Thunderbird"), "tunderbird");
    assert_eq!(skeleton("Wireshark"), "vireshark");
    assert_eq!(skeleton("Spotify"), "spotifi");
}

#[test]
fn empty_and_non_letters() {
    assert_eq!(skeleton(""), "");
    assert_eq!(skeleton("42 !?"), "42 !?");
    assert_eq!(skeleton_alt(""), None);
}
