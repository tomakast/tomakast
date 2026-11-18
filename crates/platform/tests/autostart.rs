use std::path::Path;
use tomakast_platform::set_autostart_in;

#[cfg(target_os = "linux")]
#[test]
fn autostart_writes_and_removes_desktop_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("autostart");
    let file = dir.join("tomakast.desktop");

    set_autostart_in(&dir, true, Path::new("/x/tomakast")).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("[Desktop Entry]"));
    assert!(text.contains("Type=Application"));
    assert!(text.contains("Name=tomakast"));
    assert!(text.contains("Exec=\"/x/tomakast\" daemon"));
    assert!(text.contains("X-GNOME-Autostart-enabled=true"));

    set_autostart_in(&dir, false, Path::new("/x/tomakast")).unwrap();
    assert!(!file.exists());
    // Disabling twice is fine.
    set_autostart_in(&dir, false, Path::new("/x/tomakast")).unwrap();
}
