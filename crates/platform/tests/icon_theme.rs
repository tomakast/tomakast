//! Theme-named icons resolved against a fake icon-theme tree. `freedesktop-icons` reads its
//! base directories (from `HOME` and the XDG variables) once per process, so this file is
//! its own test binary, with one test that sets them before the first lookup.

#![cfg(target_os = "linux")]

use std::path::Path;

use tomakast_platform::icon_theme::{FILE_TYPE_ICON_SIZE, lookup, lookup_sized, resolve};

/// Names no real system holds (the library also looks in `/usr/share/pixmaps`).
const IN_THEME: &str = "tomakast-test-icon-in-theme";
const IN_HICOLOR: &str = "tomakast-test-icon-in-hicolor";
const NOWHERE: &str = "tomakast-test-icon-nowhere";
/// Only in the working directory, which the library tries last, as a relative path.
const IN_CWD: &str = "tomakast-test-icon-in-cwd";
/// A file-type icon in a theme laid out as Breeze is.
const FILE_TYPE: &str = "tomakast-test-x-type";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn index_theme(name: &str, inherits: Option<&str>) -> String {
    let inherits = inherits
        .map(|i| format!("Inherits={i}\n"))
        .unwrap_or_default();
    format!(
        "[Icon Theme]\nName={name}\n{inherits}Directories=48x48/apps\n\n\
         [48x48/apps]\nSize=48\nType=Fixed\n"
    )
}

#[test]
fn theme_names_resolve_in_the_desktop_theme_then_hicolor() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let config = root.path().join("config");
    let icons = data.join("icons");
    write(
        &icons.join("TestTheme/index.theme"),
        &index_theme("TestTheme", Some("hicolor")),
    );
    write(
        &icons.join("hicolor/index.theme"),
        &index_theme("Hicolor", None),
    );
    let in_theme = icons.join(format!("TestTheme/48x48/apps/{IN_THEME}.png"));
    let in_hicolor = icons.join(format!("hicolor/48x48/apps/{IN_HICOLOR}.png"));
    write(&in_theme, "");
    write(&in_hicolor, "");
    write(
        &config.join("gtk-3.0/settings.ini"),
        "[Settings]\ngtk-icon-theme-name=TestTheme\n",
    );
    // Two themes up the chain: Grandchild inherits Child, which inherits TestTheme.
    write(
        &icons.join("Grandchild/index.theme"),
        &index_theme("Grandchild", Some("Child")),
    );
    write(
        &icons.join("Child/index.theme"),
        &index_theme("Child", Some("TestTheme")),
    );
    // Breeze's file-type folders: 16 px glyphs (also as their 3x copy) and the scalable 64 px
    // icons, none at 48.
    write(
        &icons.join("Sizes/index.theme"),
        "[Icon Theme]\nName=Sizes\nInherits=hicolor\nDirectories=mimetypes/16,mimetypes/16@3x,mimetypes/64\n\n\
         [mimetypes/16]\nSize=16\nType=Fixed\n\n\
         [mimetypes/16@3x]\nSize=16\nScale=3\nType=Fixed\n\n\
         [mimetypes/64]\nSize=64\nType=Scalable\nMinSize=64\nMaxSize=256\n",
    );
    for dir in ["16", "16@3x", "64"] {
        write(
            &icons.join(format!("Sizes/mimetypes/{dir}/{FILE_TYPE}.svg")),
            "",
        );
    }
    let cwd = root.path().join("cwd");
    write(&cwd.join(format!("{IN_CWD}.png")), "");
    std::env::set_current_dir(&cwd).unwrap();
    // SAFETY: the only test of this binary, before anything else runs or reads the
    // environment.
    unsafe {
        std::env::set_var("HOME", root.path());
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_DATA_DIRS", &data);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    }

    assert_eq!(lookup(IN_THEME, "TestTheme"), Some(in_theme.clone()));
    // Not in the theme: its parent, `hicolor`.
    assert_eq!(lookup(IN_HICOLOR, "TestTheme"), Some(in_hicolor.clone()));
    assert_eq!(lookup(NOWHERE, "TestTheme"), None);
    assert_eq!(lookup(IN_THEME, "Grandchild"), Some(in_theme.clone()));
    assert_eq!(lookup(IN_HICOLOR, "Grandchild"), Some(in_hicolor.clone()));
    // File types at their own size: the 64 px icon, not the 16 px glyph's 3x copy.
    assert_eq!(
        lookup_sized(FILE_TYPE, "Sizes", FILE_TYPE_ICON_SIZE),
        Some(icons.join(format!("Sizes/mimetypes/64/{FILE_TYPE}.svg")))
    );
    // The desktop's theme, read from the GTK settings.
    assert_eq!(resolve(IN_THEME), Some(in_theme));
    assert_eq!(resolve(IN_HICOLOR), Some(in_hicolor));
    assert_eq!(resolve(NOWHERE), None);
    // Never a file of the daemon's working directory.
    assert_eq!(lookup(IN_CWD, "TestTheme"), None);
    assert_eq!(resolve(IN_CWD), None);
    // A name is a name, never a path: a plugin's name cannot reach a file outside the
    // icon directories, nor one by its path.
    let outside = root.path().join("secret.png");
    write(&outside, "");
    for name in [
        "../../../secret",
        "/etc/passwd",
        outside.to_str().unwrap(),
        "48x48/apps/tomakast-test-icon-in-theme",
        "",
        "a\0b",
    ] {
        assert_eq!(resolve(name), None, "{name:?}");
        assert_eq!(lookup(name, "TestTheme"), None, "{name:?}");
    }
}
