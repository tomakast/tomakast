//! The catalog built from the plugins folder alone.

use tomakast_core::PluginState;
use tomakast_plugin_host::list_catalog;

const WEATHER: &str = include_str!("../../../plugins/weather/plugin.toml");

#[test]
fn lists_plugins_as_not_running() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("weather");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), WEATHER).unwrap();
    std::fs::write(dir.join("plugin.wasm"), b"\0asm").unwrap();
    let catalog = list_catalog(root.path(), "no database");
    assert_eq!(catalog.unavailable, None);
    let [plugin] = catalog.plugins.as_slice() else {
        panic!("one plugin expected: {:?}", catalog.plugins);
    };
    assert_eq!(plugin.id, "weather");
    assert_eq!(plugin.state, PluginState::NotRunning("no database".into()));
    let keys: Vec<_> = plugin.preferences.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["temperature", "wind", "pressure"]);
    assert_eq!(plugin.commands.len(), 1);
}

#[test]
fn a_missing_folder_is_an_empty_list() {
    let root = tempfile::tempdir().unwrap();
    let catalog = list_catalog(&root.path().join("nope"), "x");
    assert!(catalog.plugins.is_empty());
    assert_eq!(catalog.unavailable, None);
}

#[cfg(unix)]
#[test]
fn an_unlistable_folder_is_unavailable() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("plugins");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    let catalog = list_catalog(&dir, "x");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if std::fs::read_dir(&dir).is_ok() && catalog.unavailable.is_none() {
        return; // running as root: permissions do not apply
    }
    let reason = catalog.unavailable.expect("unavailable");
    assert!(reason.starts_with("cannot list "), "{reason}");
    assert!(catalog.plugins.is_empty());
}
