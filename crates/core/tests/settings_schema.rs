use tomakast_core::settings_schema::{
    ChoiceOption, KeyValue, NumberRange, OnInvalid, Page, ValueKind, builtin_keys, read_key,
    unrecognized, value_fits,
};
use tomakast_core::{FontChoice, Hotkey, PopToRoot, ScreenChoice, Settings, TextSize, ThemeChoice};

#[test]
fn every_schema_key_reaches_settings() {
    let text = r#"
[launcher]
hotkey = "Ctrl+Alt+K"
autostart = false
pop_to_root = "3m"
screen = "primary"
history = false
[location]
city = "Berlin"
wifi_lookup = true
timezone = "Asia/Tokyo"
[appearance]
theme = "light"
font = "system"
text_size = "small"
mono_font = "Iosevka"
mono_size = 12
"#;
    let s = Settings::parse(text).unwrap();
    assert_eq!(s.launcher_hotkey, Hotkey::parse("Ctrl+Alt+K").unwrap());
    assert!(!s.autostart);
    assert_eq!(s.location.city, "Berlin");
    assert!(s.location.wifi_lookup);
    assert_eq!(s.appearance, ThemeChoice::Light);
    assert_eq!(
        s.launcher.pop_to_root,
        PopToRoot::After(std::time::Duration::from_secs(180))
    );
    assert_eq!(s.launcher.screen, ScreenChoice::Primary);
    assert!(!s.launcher.history);
    assert_eq!(s.location.timezone, "Asia/Tokyo");
    assert_eq!(s.text.font, FontChoice::System);
    assert_eq!(s.text.size, TextSize::Small);
    assert_eq!(s.text.mono_font, FontChoice::Family("Iosevka".into()));
    assert_eq!(s.text.mono_size, 12);
    // One assertion per schema key: adding a key without a line here fails.
    assert_eq!(builtin_keys().len(), 13);
}

#[test]
fn wrong_type_is_invalid_not_missing() {
    let file: toml::Table = "[location]\ncity = 5\n".parse().unwrap();
    let city = builtin_keys()
        .iter()
        .find(|k| k.path == ["location", "city"])
        .unwrap();
    assert!(matches!(
        read_key(&file, city),
        KeyValue::Invalid(toml::Value::Integer(5))
    ));
    let s = Settings::parse("[location]\ncity = 5\n").unwrap();
    assert_eq!(s.location.city, "");
}

#[test]
fn wrong_typed_autostart_is_still_an_error() {
    assert!(Settings::parse("[launcher]\nautostart = \"yes\"\n").is_err());
}

#[test]
fn unrecognized_lists_unknown_keys_at_every_level() {
    let file: toml::Table =
        "zz = 1\nwidgets = []\n[launcher]\nfoo = true\n[location]\nbar = \"x\"\n[home]\nbaz = 2\n"
            .parse()
            .unwrap();
    let mut paths: Vec<String> = unrecognized(&file)
        .iter()
        .map(|u| u.path.join("."))
        .collect();
    paths.sort();
    let mut expected = ["zz", "widgets", "launcher.foo", "location.bar", "home.baz"];
    expected.sort();
    assert_eq!(paths, expected);
}

#[test]
fn numbers_in_plugin_tables_reach_plugins_as_text() {
    let s = Settings::parse("[plugins.p]\nn = 10\nf = 2.5\n").unwrap();
    assert_eq!(s.plugin_preferences["p"]["n"], "10");
    assert_eq!(s.plugin_preferences["p"]["f"], "2.5");
}

fn number(min: Option<f64>, max: Option<f64>, integer: bool) -> ValueKind {
    ValueKind::Number(NumberRange {
        min,
        max,
        step: None,
        integer,
    })
}

#[test]
fn number_values_fit_by_type_and_bounds() {
    use toml::Value::{Float, Integer, String as Str};
    let cases = [
        // (kind, value, fits)
        (number(None, None, true), Integer(5), true),
        (number(None, None, true), Float(5.0), false),
        (number(None, None, true), Float(2.5), false),
        (number(None, None, false), Float(2.5), true),
        (number(None, None, false), Integer(2), true),
        (number(Some(1.0), Some(20.0), true), Integer(1), true),
        (number(Some(1.0), Some(20.0), true), Integer(20), true),
        (number(Some(1.0), Some(20.0), true), Integer(0), false),
        (number(Some(1.0), Some(20.0), true), Integer(21), false),
        (number(Some(0.5), Some(1.5), false), Float(0.4), false),
        (number(Some(0.5), Some(1.5), false), Float(0.5), true),
        (number(Some(0.5), Some(1.5), false), Float(1.6), false),
        (number(Some(1.0), None, true), Integer(1000), true),
        (number(None, Some(1.0), true), Integer(-1000), true),
        (number(None, None, false), Str("5".into()), false),
        (number(None, None, false), toml::Value::Boolean(true), false),
    ];
    for (kind, value, fits) in cases {
        assert_eq!(value_fits(&kind, &value), fits, "{kind:?} with {value}");
    }
}

#[test]
fn choice_values_fit_only_when_an_option() {
    use toml::Value::{Integer, String as Str};
    let kind = ValueKind::Choice(
        ["metric", "imperial"]
            .map(|v| ChoiceOption {
                value: v.into(),
                title: v.into(),
            })
            .to_vec(),
    );
    let cases = [
        (Str("metric".into()), true),
        (Str("imperial".into()), true),
        (Str("kelvin".into()), false),
        (Str("Metric".into()), false),
        (Str(String::new()), false),
        (Integer(1), false),
    ];
    for (value, fits) in cases {
        assert_eq!(value_fits(&kind, &value), fits, "{value}");
    }
}

#[test]
fn unrecognized_lists_location_and_home_that_are_not_tables() {
    let file: toml::Table = "location = 5\nhome = \"x\"\n".parse().unwrap();
    let listed: Vec<(String, String)> = unrecognized(&file)
        .into_iter()
        .map(|u| (u.path.join("."), u.reason))
        .collect();
    assert_eq!(
        listed,
        [
            ("home".to_string(), "not a table".to_string()),
            ("location".to_string(), "not a table".to_string()),
        ]
    );
}

#[test]
fn theme_is_a_choice_on_appearance() {
    let key = builtin_keys()
        .iter()
        .find(|k| k.path == ["appearance", "theme"])
        .unwrap();
    assert_eq!(key.page, Page::Appearance);
    assert_eq!(key.default, "system");
    assert_eq!(key.on_invalid, OnInvalid::Warn);
    let ValueKind::Choice(options) = &key.kind else {
        panic!()
    };
    let pairs: Vec<_> = options
        .iter()
        .map(|o| (o.value.as_str(), o.title.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [("system", "System"), ("light", "Light"), ("dark", "Dark")]
    );
}

#[test]
fn any_string_fits_a_time_zone() {
    use toml::Value::{Integer, String as Str};
    for zone in ["Europe/Berlin", "Moscow", "", "not a zone"] {
        assert!(
            value_fits(&ValueKind::TimeZone, &Str(zone.into())),
            "{zone:?}"
        );
    }
    assert!(!value_fits(&ValueKind::TimeZone, &Integer(3)));
}
