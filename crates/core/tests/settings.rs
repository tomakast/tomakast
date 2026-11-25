use std::ffi::OsString;
use std::path::PathBuf;

use tomakast_core::leader::LeaderMode;
use tomakast_core::settings::Modifiers;
use tomakast_core::{
    FilesSettings, HomeEntry, Hotkey, ItemId, LocationSettings, Paths, Settings, ThemeChoice,
    WidgetKey, WidgetSize, params,
};

#[test]
fn parse_and_format_hotkey() {
    let h = Hotkey::parse("ctrl+alt+t").unwrap();
    assert_eq!(h.to_string(), "Ctrl+Alt+T");
    assert_eq!(h.to_xdg_trigger().as_deref(), Some("CTRL+ALT+t"));
    assert_eq!(
        Hotkey::parse("Super+Space")
            .unwrap()
            .to_xdg_trigger()
            .as_deref(),
        Some("LOGO+space")
    );
}

#[test]
fn xdg_trigger_uses_keysyms() {
    let trigger = |s: &str| Hotkey::parse(s).unwrap().to_xdg_trigger();
    assert_eq!(
        trigger("Ctrl+Alt+Enter").as_deref(),
        Some("CTRL+ALT+Return")
    );
    assert_eq!(trigger("Super+F1").as_deref(), Some("LOGO+F1"));
    assert_eq!(trigger("Ctrl+Escape"), None);
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
    // Strings, booleans and numbers are preference values; the rest is skipped with a warning.
    assert_eq!(
        prefs("other"),
        [
            ("count".into(), "3".into()),
            ("quiet".into(), "false".into())
        ]
    );
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
fn plugin_switches_turn_off_only_what_is_false() {
    let s = Settings::parse(
        "[enabled_plugins]\nweather = false\ncalc = true\nodd = \"no\"\n\n[plugins.weather]\nenabled = true\n",
    )
    .unwrap();
    // `true` is on, a value that is not a boolean is skipped (on), and a preference named
    // `enabled` is the plugin's own: it does not turn anything on or off.
    assert_eq!(s.disabled_plugins.iter().collect::<Vec<_>>(), ["weather"]);
    assert_eq!(s.plugin_preferences["weather"]["enabled"], "true");
    assert!(Settings::default().disabled_plugins.is_empty());
    // A switch table that is not a table refuses the file, as `[plugins]` does.
    assert!(Settings::parse("enabled_plugins = false\n").is_err());
}

#[test]
fn home_widgets_parse_in_order_with_params_and_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        r#"
[[home.widgets]]
widget = "weather/now"
size = "quarter"
city = "Barcelona"

[[home.widgets]]
widget = "weather/now"

[[home.widgets]]
widget = "weather/now"
size = "quarter"
city = "Barcelona"
"#,
    );
    let s = Settings::load(&p).unwrap();
    assert_eq!(s.home.len(), 3);
    assert_eq!(s.home[0], s.home[2]);
    assert_eq!(s.home[0].widget, WidgetKey::new("weather", "now"));
    assert_eq!(s.home[0].size, Some(WidgetSize::Quarter));
    assert_eq!(
        s.home[0].params,
        params([("city".into(), "Barcelona".into())])
    );
    assert_eq!(s.home[1].size, None);
    assert!(s.home[1].params.is_empty());
}

#[test]
fn a_bad_home_entry_costs_only_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        r#"
[[home.widgets]]
size = "single"

[[home.widgets]]
widget = "nowidget"

[[home.widgets]]
widget = "a/b"
size = "huge"

[[home.widgets]]
widget = "a/c"
city = 5

[[home.widgets]]
widget = "a/d"
size = "double"
"#,
    );
    let s = Settings::load(&p).unwrap();
    assert_eq!(
        s.home,
        vec![
            HomeEntry {
                widget: WidgetKey::new("a", "b"),
                size: None,
                params: params([]),
            },
            HomeEntry {
                widget: WidgetKey::new("a", "d"),
                size: Some(WidgetSize::Double),
                params: params([]),
            },
        ]
    );
}

#[test]
fn old_widgets_table_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        "[launcher]\nautostart = false\n\n[widgets]\norder = [\"a/b\"]\n",
    );
    let s = Settings::load(&p).unwrap();
    assert!(!s.autostart);
    assert!(s.home.is_empty());
}

#[test]
fn location_settings_parse_and_a_wrong_value_costs_only_itself() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(Settings::default().location, LocationSettings::default());
    assert_eq!(
        LocationSettings::default(),
        LocationSettings {
            city: String::new(),
            wifi_lookup: false,
            timezone: String::new(),
        }
    );
    let p = write(
        &tmp,
        "[location]\ncity = \"  Москва \"\nwifi_lookup = true\ntimezone = \" Asia/Tokyo \"\n",
    );
    assert_eq!(
        Settings::load(&p).unwrap().location,
        LocationSettings {
            city: "Москва".into(),
            wifi_lookup: true,
            timezone: "Asia/Tokyo".into(),
        }
    );
    let p = write(
        &tmp,
        "[launcher]\nautostart = false\n\n[location]\ncity = 3\nwifi_lookup = \"yes\"\n",
    );
    let s = Settings::load(&p).unwrap();
    assert!(!s.autostart);
    assert_eq!(s.location, LocationSettings::default());
    let p = write(&tmp, "location = \"Berlin\"\n");
    assert_eq!(
        Settings::load(&p).unwrap().location,
        LocationSettings::default()
    );
}

#[test]
fn home_rows_lists_every_element_and_none_for_unreadable_ones() {
    let text = "[[home.widgets]]\nwidget = \"a/one\"\n[[home.widgets]]\nwidget = \"nowidget\"\n";
    let rows = tomakast_core::settings::home_rows(text).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].is_some() && rows[1].is_none());
    assert!(tomakast_core::settings::home_rows("home = 1\n").is_err());
    assert!(tomakast_core::settings::home_rows("").unwrap().is_empty());
}

#[test]
fn leader_hotkeys_parse_and_display() {
    let h = Hotkey::parse("leader+shift+F").unwrap();
    assert_eq!(h.mods, Modifiers::LEADER | Modifiers::SHIFT);
    assert_eq!(h.key, "f");
    assert_eq!(h.to_string(), "Leader+Shift+F");
}

#[test]
fn a_leader_may_be_modifiers_alone() {
    assert_eq!(Hotkey::parse_leader("Ctrl").unwrap().to_string(), "Ctrl");
    assert_eq!(
        Hotkey::parse_leader("Ctrl+Alt").unwrap().to_string(),
        "Ctrl+Alt"
    );
    assert!(
        Hotkey::parse_leader("Ctrl+Alt")
            .unwrap()
            .is_modifiers_only()
    );
    assert_eq!(Hotkey::parse_leader("F12").unwrap().key, "f12");
    assert!(Hotkey::parse_leader("Leader+F").is_err());
    assert!(Hotkey::parse("Ctrl").is_err());
}

#[test]
fn leader_settings_are_read() {
    let s = Settings::parse("[launcher]\nleader = \"F12\"\nleader_mode = \"tap\"\n").unwrap();
    assert_eq!(s.leader, Some(Hotkey::parse_leader("F12").unwrap()));
    assert_eq!(s.leader_mode, LeaderMode::Tap);
    let d = Settings::parse("").unwrap();
    assert_eq!((d.leader, d.leader_mode), (None, LeaderMode::Hold));
    assert_eq!(
        Settings::parse("[launcher]\nleader = \"\"\n")
            .unwrap()
            .leader,
        None
    );
    assert!(Settings::parse("[launcher]\nleader = \"Ctrl+\"\n").is_err());
    // A bad mode warns and keeps the default.
    assert_eq!(
        Settings::parse("[launcher]\nleader_mode = \"x\"\n")
            .unwrap()
            .leader_mode,
        LeaderMode::Hold
    );
}

#[test]
fn a_launcher_hotkey_with_the_leader_is_an_error() {
    let e = Settings::parse("[launcher]\nhotkey = \"Leader+Space\"\n").unwrap_err();
    assert!(format!("{e:#}").contains("Leader+Space"), "{e:#}");
}

#[test]
fn theme_is_read_from_appearance() {
    let s = Settings::parse("[appearance]\ntheme = \"dark\"\n").unwrap();
    assert_eq!(s.appearance, ThemeChoice::Dark);
    assert_eq!(Settings::parse("").unwrap().appearance, ThemeChoice::System);
}

#[test]
fn an_unknown_theme_warns_and_uses_system() {
    let s = Settings::parse("[appearance]\ntheme = \"blue\"\n").unwrap();
    assert_eq!(s.appearance, ThemeChoice::System);
}

#[test]
fn files_defaults_without_a_table() {
    assert_eq!(Settings::default().files, FilesSettings::default());
    assert_eq!(
        FilesSettings::default(),
        FilesSettings {
            in_root: true,
            roots: vec!["~".into()],
            exclude: Vec::new(),
            hidden: false,
            gitignore: true,
        }
    );
    let tmp = tempfile::tempdir().unwrap();
    let p = write(&tmp, "");
    assert_eq!(Settings::load(&p).unwrap().files, FilesSettings::default());
}

#[test]
fn files_table_parses() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        "[files]\nin_root = false\nroots = [\"~\", \"/data\"]\nexclude = [\"~/Games\", \"*.iso\"]\nhidden = true\ngitignore = false\nsurprise = 1\n",
    );
    assert_eq!(
        Settings::load(&p).unwrap().files,
        FilesSettings {
            in_root: false,
            roots: vec!["~".into(), "/data".into()],
            exclude: vec!["~/Games".into(), "*.iso".into()],
            hidden: true,
            gitignore: false,
        }
    );
}

#[test]
fn files_bad_type_warns_and_keeps_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        &tmp,
        "[launcher]\nhotkey = \"ctrl+space\"\n\n[files]\nin_root = \"yes\"\nhidden = true\nroots = [1]\n",
    );
    let s = Settings::load(&p).unwrap();
    assert_eq!(s.launcher_hotkey, Hotkey::parse("ctrl+space").unwrap());
    assert_eq!(
        s.files,
        FilesSettings {
            hidden: true,
            ..FilesSettings::default()
        }
    );
}

#[test]
fn files_not_a_table_uses_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(&tmp, "files = 3\n");
    assert_eq!(Settings::load(&p).unwrap().files, FilesSettings::default());
}

/// A file that is not TOML is told in one line: where, and what is wrong there.
#[test]
fn invalid_toml_is_one_line_with_its_place() {
    let text = "[launcher]\nautostart = true\nthis is not = = toml\n";
    let error = format!("{:#}", Settings::parse(text).unwrap_err());
    assert!(!error.contains('\n'), "{error}");
    assert!(
        error.starts_with("invalid TOML at line 3, column "),
        "{error}"
    );
    assert!(error.ends_with("expected `=`"), "{error}");
    let error = format!(
        "{:#}",
        tomakast_core::settings::home_rows("a = [\n").unwrap_err()
    );
    assert!(!error.contains('\n'), "{error}");
    assert!(error.starts_with("invalid TOML at line "), "{error}");
}

/// An error after keys in another script than Latin: its column counts characters, not the
/// bytes they take, and nothing panics on the multi-byte text.
#[test]
fn invalid_toml_in_another_script_says_where() {
    let text = "[launcher]\nавтозапуск = true\nключ = = toml\n";
    let error = format!("{:#}", Settings::parse(text).unwrap_err());
    assert!(
        error.starts_with("invalid TOML at line 3, column 8: "),
        "{error}"
    );
}

/// The `[appearance]` text keys and the `[launcher]` keys of showing and hiding: their
/// defaults, their values, and a wrong value costing only itself.
#[test]
fn text_and_launcher_settings_parse() {
    use std::time::Duration;
    use tomakast_core::{
        FontChoice, LauncherSettings, PopToRoot, ScreenChoice, TextSettings, TextSize,
    };
    let s = Settings::default();
    assert_eq!(
        s.text,
        TextSettings {
            font: FontChoice::System,
            mono_font: FontChoice::Bundled,
            size: TextSize::Default,
            mono_size: 14,
        }
    );
    assert_eq!(
        s.launcher,
        LauncherSettings {
            pop_to_root: PopToRoot::After(Duration::from_secs(90)),
            screen: ScreenChoice::Mouse,
            history: true,
        }
    );
    let s = Settings::parse(
        "[appearance]\nfont = \"system\"\nmono_font = \"Fira Code\"\ntext_size = \"large\"\n\
         mono_size = 18\n\n[launcher]\npop_to_root = \"never\"\nscreen = \"active-window\"\n\
         history = false\n",
    )
    .unwrap();
    assert_eq!(
        s.text,
        TextSettings {
            font: FontChoice::System,
            mono_font: FontChoice::Family("Fira Code".into()),
            size: TextSize::Large,
            mono_size: 18,
        }
    );
    assert_eq!(
        s.launcher,
        LauncherSettings {
            pop_to_root: PopToRoot::Never,
            screen: ScreenChoice::ActiveWindow,
            history: false,
        }
    );
    // The bundled family by name is the bundled family.
    let s = Settings::parse("[appearance]\nfont = \"Inter\"\nmono_font = \"JetBrains Mono\"\n")
        .unwrap();
    assert_eq!(s.text.font, FontChoice::Bundled);
    assert_eq!(s.text.mono_font, FontChoice::Bundled);
    let s = Settings::parse(
        "[appearance]\ntext_size = \"huge\"\nmono_size = 40\n\n[launcher]\npop_to_root = 5\n\
         screen = \"left\"\n",
    )
    .unwrap();
    assert_eq!(s.text, TextSettings::default());
    assert_eq!(s.launcher, LauncherSettings::default());
    for (value, pop) in [
        ("immediately", PopToRoot::Immediately),
        ("30s", PopToRoot::After(Duration::from_secs(30))),
        ("3m", PopToRoot::After(Duration::from_secs(180))),
        ("10m", PopToRoot::After(Duration::from_secs(600))),
    ] {
        let s = Settings::parse(&format!("[launcher]\npop_to_root = \"{value}\"\n")).unwrap();
        assert_eq!(s.launcher.pop_to_root, pop, "{value}");
    }
    assert_eq!(TextSize::Small.scale() * 14., 13.);
    assert_eq!(TextSize::Large.scale() * 14., 16.);
}
