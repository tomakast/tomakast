use std::ffi::OsString;
use std::path::PathBuf;

use tomakast_core::{Hotkey, ItemId, Paths, Settings};

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
