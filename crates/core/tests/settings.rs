use std::ffi::OsString;
use std::path::PathBuf;

use tomakast_core::{Hotkey, ItemId, Paths, Settings, WidgetKey, WidgetSettings};

#[test]
fn parse_and_format_hotkey() {
    let h = Hotkey::parse("ctrl+alt+t").unwrap();
    assert_eq!(h.to_string(), "Ctrl+Alt+T");
    assert_eq!(h.to_xdg_trigger(), "CTRL+ALT+t");
    assert_eq!(
        Hotkey::parse("Super+Space").unwrap().to_xdg_trigger(),
        "LOGO+space"
    );
}

#[test]
fn parse_rejects_bad_hotkeys() {
    for s in ["", "Ctrl+", "Ctrl+A+B", "Hyper+A"] {
        assert!(Hotkey::parse(s).is_err(), "{s}");
    }
}

#[test]
fn tomakast_home_overrides_paths() {
    let paths =
        Paths::detect_with(|k| (k == "TOMAKAST_HOME").then(|| OsString::from("/tmp/x"))).unwrap();
    assert_eq!(paths.config_file, PathBuf::from("/tmp/x/config.toml"));
    assert_eq!(paths.db_file, PathBuf::from("/tmp/x/tomakast.db"));
    assert_eq!(paths.data_dir, PathBuf::from("/tmp/x"));
    assert_eq!(paths.plugins_dir, PathBuf::from("/tmp/x/plugins"));
    assert_eq!(
        paths.plugin_cache_dir,
        PathBuf::from("/tmp/x/cache/plugins")
    );
}

#[test]
fn ensure_dirs_creates_data_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("nested/home");
    let h = home.clone().into_os_string();
    let paths = Paths::detect_with(|k| (k == "TOMAKAST_HOME").then(|| h.clone())).unwrap();
    paths.ensure_dirs().unwrap();
    assert!(home.is_dir());
}

#[test]
fn missing_file_gives_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Settings::load(&tmp.path().join("nope.toml")).unwrap();
    assert_eq!(s, Settings::default());
    assert_eq!(s.launcher_hotkey, Hotkey::parse("Alt+Space").unwrap());
    assert!(s.autostart);
    assert!(s.hotkeys.is_empty());
}

fn write(dir: &tempfile::TempDir, text: &str) -> PathBuf {
    let p = dir.path().join("config.toml");
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn parses_hotkey_table() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        r#"
[launcher]
hotkey = "Alt+Space"
autostart = false

[hotkeys]
"Ctrl+Alt+F" = "apps/firefox.desktop"
"#,
    );
    let s = Settings::load(&p).unwrap();
    assert!(!s.autostart);
    assert_eq!(
        s.hotkeys,
        vec![(
            Hotkey::parse("Ctrl+Alt+F").unwrap(),
            ItemId::parse("apps/firefox.desktop").unwrap()
        )]
    );
}

#[test]
fn bad_hotkey_entries_name_the_key() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(&tmp, "[hotkeys]\n\"Hyper+A\" = \"apps/x.desktop\"\n");
    let err = Settings::load(&p).unwrap_err().to_string();
    assert!(err.contains("Hyper+A"), "{err}");

    let p = write(&tmp, "[hotkeys]\n\"Ctrl+A\" = \"noslash\"\n");
    let err = Settings::load(&p).unwrap_err().to_string();
    assert!(err.contains("Ctrl+A"), "{err}");
}

#[test]
fn load_or_default_survives_invalid_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(&tmp, "[launcher");
    let (s, err) = Settings::load_or_default(&p);
    assert_eq!(s, Settings::default());
    assert!(err.is_some());
}

#[test]
fn plugin_preferences_come_from_plugin_tables() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        r#"
[plugins.kitchen-sink]
greeting = "hi"
loud = true

[plugins.other]
quiet = false
count = 3
nested = { a = "b" }
"#,
    );
    let s = Settings::load(&p).unwrap();
    let prefs = |id: &str| -> Vec<(String, String)> {
        s.plugin_preferences[id]
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    assert_eq!(
        prefs("kitchen-sink"),
        [
            ("greeting".into(), "hi".into()),
            ("loud".into(), "true".into())
        ]
    );
    // Only strings and booleans are preference values; the rest is skipped with a warning.
    assert_eq!(prefs("other"), [("quiet".into(), "false".into())]);
    assert_eq!(s.plugin_preferences.len(), 2);
}

#[test]
fn a_plugins_entry_that_is_not_a_table_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        "[plugins]
loose = \"x\"\n[plugins.ok]\nk = \"v\"\n",
    );
    let s = Settings::load(&p).unwrap();
    assert_eq!(s.plugin_preferences.keys().collect::<Vec<_>>(), ["ok"]);
    assert!(Settings::default().plugin_preferences.is_empty());
}

#[test]
fn widgets_settings_parse_and_skip_bad_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        r#"
[widgets]
order = ["a/b", "bad", "a/b", 3, "/x", "c/d/e"]
hidden = ["c/d", "c/d", "e/"]
"#,
    );
    let s = Settings::load(&p).unwrap();
    assert_eq!(
        s.widgets,
        WidgetSettings {
            order: vec![WidgetKey::new("a", "b")],
            hidden: vec![WidgetKey::new("c", "d")],
        }
    );
    assert_eq!(Settings::default().widgets, WidgetSettings::default());
}

#[test]
fn a_widgets_list_of_the_wrong_type_costs_only_that_list() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        "[launcher]\nautostart = false\n\n[widgets]\norder = \"a/b\"\nhidden = [\"c/d\"]\n",
    );
    let s = Settings::load(&p).unwrap();
    assert!(!s.autostart);
    assert_eq!(
        s.widgets,
        WidgetSettings {
            order: vec![],
            hidden: vec![WidgetKey::new("c", "d")],
        }
    );

    let p = write(&tmp, "widgets = 3\n\n[launcher]\nautostart = false\n");
    let s = Settings::load(&p).unwrap();
    assert!(!s.autostart);
    assert_eq!(s.widgets, WidgetSettings::default());
}
