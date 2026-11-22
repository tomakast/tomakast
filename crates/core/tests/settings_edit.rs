//! Editing `[hotkeys]` in `config.toml` without losing the user's comments and layout.

use tomakast_core::settings_edit::{set_item_hotkey, write_config};
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
