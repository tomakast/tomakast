//! Editing `[hotkeys]` in `config.toml` without losing the user's comments and layout.

use tomakast_core::settings_edit::{
    ensure_config_file, remove_plugin_table, set_item_hotkey, set_key, set_plugin_enabled,
    set_plugin_preference, write_config,
};
use tomakast_core::{Hotkey, ItemId, Settings};

fn hk(s: &str) -> Hotkey {
    Hotkey::parse(s).unwrap()
}

fn kitty() -> ItemId {
    ItemId::parse("apps/kitty").unwrap()
}

#[test]
fn adding_keeps_comments_and_other_tables() {
    let text = "# my launcher settings\n[launcher]\nhotkey = \"Alt+Space\" # the usual\n";
    let out = set_item_hotkey(text, &kitty(), Some(&hk("ctrl+alt+t"))).unwrap();
    for line in text.lines() {
        assert!(out.contains(line), "lost {line:?} in:\n{out}");
    }
    assert!(
        out.trim_end()
            .ends_with("[hotkeys]\n\"Ctrl+Alt+T\" = \"apps/kitty\""),
        "{out}"
    );
}

#[test]
fn changing_removes_every_spelling_of_the_items_entries() {
    let text = "[hotkeys]\n\"alt+ctrl+T\" = \"apps/kitty\"\n# keep me\n\"Ctrl+Alt+X\" = \"apps/other\"\n\"Ctrl+Alt+K\" = \"apps/kitty\"\n";
    let out = set_item_hotkey(text, &kitty(), Some(&hk("ctrl+alt+y"))).unwrap();
    assert!(out.contains("# keep me"), "{out}");
    assert!(out.contains("\"Ctrl+Alt+X\" = \"apps/other\""), "{out}");
    assert_eq!(out.matches("apps/kitty").count(), 1, "{out}");
    assert!(out.contains("\"Ctrl+Alt+Y\" = \"apps/kitty\""), "{out}");
}

#[test]
fn clearing_removes_the_entry() {
    let text = "[hotkeys]\n\"Ctrl+Alt+T\" = \"apps/kitty\"\n";
    let out = set_item_hotkey(text, &kitty(), None).unwrap();
    assert!(!out.contains("apps/kitty"), "{out}");
    assert!(out.contains("[hotkeys]"), "{out}");
}

#[test]
fn a_broken_file_is_refused() {
    assert!(set_item_hotkey("[hotkeys\n", &kitty(), Some(&hk("ctrl+alt+t"))).is_err());
    assert!(
        set_item_hotkey(
            "[hotkeys]\n\"Ctrl+\" = \"a/b\"\n",
            &kitty(),
            Some(&hk("ctrl+alt+t"))
        )
        .is_err()
    );
}

#[test]
fn write_config_creates_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_config(&path, "[hotkeys]\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[hotkeys]\n");
}

#[cfg(unix)]
#[test]
fn write_config_replaces_the_symlink_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real.toml");
    std::fs::write(&target, "old").unwrap();
    let link = dir.path().join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    write_config(&link, "new").unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
}

#[test]
fn the_result_parses_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let out = set_item_hotkey("", &kitty(), Some(&hk("ctrl+alt+y"))).unwrap();
    write_config(&path, &out).unwrap();
    let settings = Settings::load(&path).unwrap();
    assert_eq!(settings.hotkeys, [(hk("ctrl+alt+y"), kitty())]);
}

#[test]
fn a_comment_above_a_changed_entry_stays() {
    let text = "[hotkeys]\n# terminals\n\"Ctrl+Alt+T\" = \"apps/kitty\"\n\"Ctrl+Alt+X\" = \"apps/other\"\n";
    let out = set_item_hotkey(text, &kitty(), Some(&hk("ctrl+alt+y"))).unwrap();
    assert!(out.contains("# terminals"), "{out}");
    let out = set_item_hotkey(text, &kitty(), None).unwrap();
    assert!(out.contains("# terminals"), "{out}");
}

#[test]
fn a_comment_above_the_only_entry_stays_when_it_is_cleared() {
    let text =
        "[hotkeys]\n# mine\n\"Ctrl+Alt+T\" = \"apps/kitty\"\n\n[launcher]\nautostart = true\n";
    let out = set_item_hotkey(text, &kitty(), None).unwrap();
    assert!(out.contains("# mine"), "{out}");
    assert!(out.contains("[launcher]"), "{out}");
}

#[test]
fn a_kept_comment_still_parses() {
    let text =
        "[hotkeys]\n# mine\n\"Ctrl+Alt+T\" = \"apps/kitty\"\n\n[launcher]\nautostart = true\n";
    let out = set_item_hotkey(text, &kitty(), None).unwrap();
    assert_eq!(out, "[hotkeys]\n# mine\n\n[launcher]\nautostart = true\n");
    assert!(set_item_hotkey(&out, &kitty(), Some(&hk("ctrl+alt+t"))).is_ok());
}

// --- set_key and the plugin tables ---

const FILE: &str =
    "# my launcher\n[launcher]\n# fast\nhotkey = \"Alt+Space\"\n\n[location]\ncity = \"Berlin\"\n";

#[test]
fn set_key_changes_one_value_and_keeps_comments() {
    let out = set_key(FILE, &["launcher", "hotkey"], Some("Ctrl+Space".into())).unwrap();
    assert_eq!(out, FILE.replace("\"Alt+Space\"", "\"Ctrl+Space\""));
}

#[test]
fn removing_the_last_key_removes_a_plain_table() {
    let out = set_key(FILE, &["location", "city"], None).unwrap();
    assert!(!out.contains("[location]"), "{out}");
    assert!(out.starts_with("# my launcher\n[launcher]\n# fast\n"));
}

#[test]
fn removing_the_last_key_keeps_a_commented_table() {
    let text = "[location]\n# where I live\ncity = \"Berlin\"\n";
    let out = set_key(text, &["location", "city"], None).unwrap();
    assert!(
        out.contains("[location]") && out.contains("# where I live"),
        "{out}"
    );
}

#[test]
fn set_key_creates_a_plain_table() {
    let out = set_key("", &["location", "wifi_lookup"], Some(true.into())).unwrap();
    assert_eq!(out, "[location]\nwifi_lookup = true\n");
}

#[test]
fn set_key_refuses_a_broken_file_and_a_broken_result() {
    assert!(set_key("[launcher\n", &["launcher", "autostart"], Some(true.into())).is_err());
    assert!(set_key("", &["launcher", "hotkey"], Some("Ctrl+".into())).is_err());
}

#[test]
fn plugin_preference_round_trip() {
    let out = set_plugin_preference("", "weather", "wind", Some("ms".into())).unwrap();
    assert_eq!(out, "[plugins.weather]\nwind = \"ms\"\n");
    let out = set_plugin_preference(&out, "weather", "wind", None).unwrap();
    assert_eq!(out, "");
    let out = remove_plugin_table("[plugins.x]\na = \"1\"\n[plugins.y]\nb = 2\n", "x").unwrap();
    assert_eq!(out, "[plugins.y]\nb = 2\n");
}

#[test]
fn a_plugin_switch_writes_only_off() {
    let text = "# mine\n[plugins.weather]\nwind = \"ms\"\n";
    let off = set_plugin_enabled(text, "weather", false).unwrap();
    assert_eq!(off, format!("{text}\n[enabled_plugins]\nweather = false\n"));
    assert_eq!(
        Settings::parse(&off)
            .unwrap()
            .disabled_plugins
            .into_iter()
            .collect::<Vec<_>>(),
        ["weather"]
    );
    // On again: the entry and the emptied table go, the plugin's preferences stay.
    assert_eq!(set_plugin_enabled(&off, "weather", true).unwrap(), text);
    // Turning on a plugin that is on changes nothing.
    assert_eq!(set_plugin_enabled(text, "weather", true).unwrap(), text);
}

#[test]
fn set_key_edits_dotted_and_inline_forms_in_place() {
    let dotted = "launcher.hotkey = \"Alt+Space\" # mine\n";
    let out = set_key(dotted, &["launcher", "hotkey"], Some("Ctrl+Space".into())).unwrap();
    assert_eq!(out, "launcher.hotkey = \"Ctrl+Space\" # mine\n");
    let inline = "location = { city = \"Berlin\" }\n";
    let out = set_key(inline, &["location", "city"], Some("Rome".into())).unwrap();
    assert_eq!(out, "location = { city = \"Rome\" }\n");
}

#[test]
fn removing_a_key_hands_its_comments_to_the_next_key() {
    let text = "[location]\n# the city\ncity = \"Berlin\"\nwifi_lookup = true\n";
    let out = set_key(text, &["location", "city"], None).unwrap();
    assert_eq!(out, "[location]\n# the city\nwifi_lookup = true\n");
}

#[test]
fn removing_a_missing_key_changes_nothing() {
    assert_eq!(
        set_key(FILE, &["launcher", "autostart"], None).unwrap(),
        FILE
    );
    assert_eq!(set_key("", &["location", "city"], None).unwrap(), "");
}

#[test]
fn removing_a_plugin_table_keeps_a_commented_plugins_parent() {
    let text = "# plugin settings\n[plugins]\n[plugins.x]\na = \"1\"\n";
    let out = remove_plugin_table(text, "x").unwrap();
    assert!(
        out.contains("# plugin settings") && out.contains("[plugins]"),
        "{out}"
    );
    assert!(!out.contains("plugins.x"), "{out}");
}

#[test]
fn removing_a_missing_key_keeps_an_empty_table() {
    let text = "[launcher]\n[location]\ncity=\"B\"\n";
    let out = set_key(text, &["launcher", "autostart"], None).unwrap();
    assert_eq!(out, text);
}

#[test]
fn ensure_creates_a_missing_file_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a/b/config.toml");
    assert!(ensure_config_file(&path).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"");
}

#[test]
fn ensure_leaves_an_existing_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# mine\n[launcher]\n").unwrap();
    assert!(!ensure_config_file(&path).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"# mine\n[launcher]\n");
}

/// The calculator's old time zone moves to `[location]` once: kept when the general one is not
/// set, dropped when it is; an emptied `[plugins.calc]` goes; other files are left alone.
#[test]
fn the_calculators_time_zone_moves_to_location() {
    use tomakast_core::settings_edit::migrate;
    let moved = migrate("[plugins.calc]\ntimezone = \"Asia/Tokyo\"\n").unwrap();
    let s = Settings::parse(&moved).unwrap();
    assert_eq!(s.location.timezone, "Asia/Tokyo");
    assert!(!moved.contains("plugins"), "{moved}");
    let kept = migrate(
        "[location]\ntimezone = \"Europe/Berlin\"\n\n[plugins.calc]\ntimezone = \"Asia/Tokyo\"\nx = 1\n",
    )
    .unwrap();
    let s = Settings::parse(&kept).unwrap();
    assert_eq!(s.location.timezone, "Europe/Berlin");
    assert_eq!(s.plugin_preferences["calc"].get("timezone"), None);
    assert_eq!(s.plugin_preferences["calc"]["x"], "1");
    // An empty one is just dropped.
    let s = Settings::parse(&migrate("[plugins.calc]\ntimezone = \"\"\n").unwrap()).unwrap();
    assert_eq!(s.location.timezone, "");
    for untouched in [
        "",
        "[location]\ntimezone = \"UTC\"\n",
        "[plugins.calc]\nx = 1\n",
        "a = [",
    ] {
        assert_eq!(migrate(untouched), None, "{untouched}");
    }
}
