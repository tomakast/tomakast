#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::path::PathBuf;

use tomakast_module_apps::{AppEntry, ScanOptions, launch, scan};

fn fixtures(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn opts(dirs: &[&str]) -> ScanOptions {
    ScanOptions {
        dirs: dirs.iter().map(|d| fixtures(d)).collect(),
        locales: vec![],
        current_desktops: vec!["KDE".to_string()],
        path_env: Some(OsString::new()),
        icon_theme: "hicolor".to_string(),
    }
}

fn find<'a>(
    found: &'a [(tomakast_core::Item, AppEntry)],
    id: &str,
) -> Option<&'a (tomakast_core::Item, AppEntry)> {
    found.iter().find(|(item, _)| item.id.as_str() == id)
}

#[test]
fn scan_basic_entry() {
    let found = scan(&opts(&["user"]));
    let (item, entry) = find(&found, "apps/firefox.desktop").expect("firefox present");
    assert_eq!(item.title, "Firefox");
    assert_eq!(item.subtitle.as_deref(), Some("Web Browser"));
    assert_eq!(entry.exec, vec!["firefox"]);
    assert!(!entry.terminal);
    assert_eq!(item.actions.len(), 3);
    assert_eq!(item.actions[0].id, "open");
    assert_eq!(item.actions[0].title, "Open");
    for kw in ["internet", "www", "Web Browser", "firefox"] {
        assert!(
            item.keywords.iter().any(|k| k == kw),
            "missing keyword {kw}"
        );
    }
    assert_eq!(
        item.icon,
        Some(tomakast_core::Icon::File("/opt/firefox/icon.png".into()))
    );
}

#[test]
fn scan_uses_locale_name() {
    let mut o = opts(&["user"]);
    o.locales = vec!["ru_RU".into(), "ru".into()];
    let found = scan(&o);
    let (item, _) = find(&found, "apps/files.desktop").unwrap();
    assert_eq!(item.title, "Файлы");

    let found = scan(&opts(&["user"]));
    let (item, _) = find(&found, "apps/files.desktop").unwrap();
    assert_eq!(item.title, "Files");
}

#[test]
fn scan_filters_hidden_nodisplay_onlyshowin_tryexec() {
    let found = scan(&opts(&["system"]));
    for id in [
        "hidden",
        "nodisplay",
        "onlyshowin",
        "tryexec",
        "notshowin",
        "link",
    ] {
        assert!(
            find(&found, &format!("apps/{id}.desktop")).is_none(),
            "{id} must be filtered"
        );
    }
}

#[test]
fn user_dir_overrides_system_duplicate() {
    let found = scan(&opts(&["user", "system"]));
    let dups: Vec<_> = found
        .iter()
        .filter(|(i, _)| i.id.as_str() == "apps/dup.desktop")
        .collect();
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].0.title, "Dup From User");
}

#[test]
fn subdir_desktop_id_uses_dashes() {
    let found = scan(&opts(&["system"]));
    assert!(find(&found, "apps/kde-konsole.desktop").is_some());
}

#[test]
fn scan_skips_broken_entries() {
    let found = scan(&opts(&["broken"]));
    let ids: Vec<_> = found.iter().map(|(i, _)| i.id.as_str()).collect();
    assert_eq!(ids, vec!["apps/good.desktop"]);
}

#[test]
fn launch_missing_binary_errors() {
    let entry = AppEntry {
        desktop_file: PathBuf::from("/nonexistent/x.desktop"),
        exec: vec!["/nonexistent/bin".into()],
        terminal: false,
        working_dir: None,
        mime_types: vec![],
    };
    assert!(launch(&entry).is_err());
}

#[test]
fn launch_harmless_command_succeeds() {
    let entry = AppEntry {
        desktop_file: PathBuf::from("/nonexistent/x.desktop"),
        exec: vec!["/bin/true".into()],
        terminal: false,
        working_dir: Some(std::env::temp_dir()),
        mime_types: vec![],
    };
    launch(&entry).unwrap();
}

#[test]
fn mime_types_are_read() {
    let found = scan(&opts(&["mime"]));
    let (_, entry) = find(&found, "apps/viewer.desktop").expect("viewer present");
    assert_eq!(entry.mime_types, vec!["image/png", "image/jpeg"]);
    let found = scan(&opts(&["user"]));
    let (_, entry) = find(&found, "apps/firefox.desktop").expect("firefox present");
    assert!(entry.mime_types.is_empty());
}
