#![cfg(target_os = "linux")]

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tomakast_core::{Effect, Toast, ToastKind};
use tomakast_module_apps::{
    AppEntry, FileManager, FileManagerError, ScanOptions, copy_path_effects, desktop_file_uri,
    reveal_command, reveal_with, scan,
};

fn fixtures(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn entry(path: &str) -> AppEntry {
    AppEntry {
        desktop_file: PathBuf::from(path),
        exec: vec!["x".into()],
        terminal: false,
        working_dir: None,
    }
}

#[test]
fn items_have_three_actions() {
    let found = scan(&ScanOptions {
        dirs: vec![fixtures("user")],
        locales: vec![],
        current_desktops: vec!["KDE".to_string()],
        path_env: Some(OsString::new()),
        icon_theme: "hicolor".to_string(),
    });
    let (item, _) = found
        .iter()
        .find(|(i, _)| i.id.as_str() == "apps/firefox.desktop")
        .expect("firefox present");
    let actions: Vec<(&str, &str)> = item
        .actions
        .iter()
        .map(|a| (a.id.as_str(), a.title.as_str()))
        .collect();
    assert_eq!(
        actions,
        vec![
            ("open", "Open"),
            ("reveal", "Show in File Manager"),
            ("copy-path", "Copy Path"),
        ]
    );
}

#[test]
fn copy_path_returns_copy_effect_and_toast() {
    let effects = copy_path_effects(&entry("/usr/share/applications/firefox.desktop"));
    assert_eq!(
        effects,
        vec![
            Effect::CopyText("/usr/share/applications/firefox.desktop".to_string()),
            Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "Path copied".to_string(),
                message: None,
            }),
        ]
    );
}

#[test]
fn desktop_file_uri_is_percent_encoded() {
    assert_eq!(
        desktop_file_uri(Path::new("/home/u/My Apps/ä#1.desktop")),
        "file:///home/u/My%20Apps/%C3%A4%231.desktop"
    );
}

#[test]
fn reveal_command_opens_the_parent_directory() {
    let (program, args) = reveal_command(Path::new("/usr/share/applications/firefox.desktop"));
    assert_eq!(program, "xdg-open");
    assert_eq!(args, vec![OsString::from("/usr/share/applications")]);
}

struct NoFileManager;
impl FileManager for NoFileManager {
    fn show_items(&self, _uris: &[String]) -> Result<(), FileManagerError> {
        Err(FileManagerError::Unavailable("no service".into()))
    }
}

struct Recording(RefCell<Vec<String>>);
impl FileManager for Recording {
    fn show_items(&self, uris: &[String]) -> Result<(), FileManagerError> {
        self.0.borrow_mut().extend(uris.iter().cloned());
        Ok(())
    }
}

#[test]
fn reveal_falls_back_when_no_file_manager_service() {
    let ran = RefCell::new(Vec::new());
    let effects = reveal_with(
        &entry("/usr/share/applications/firefox.desktop"),
        &NoFileManager,
        &mut |program, args| {
            ran.borrow_mut().push((program.to_owned(), args.to_vec()));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(effects, vec![Effect::CloseWindow]);
    assert_eq!(
        ran.into_inner(),
        vec![(
            "xdg-open".to_string(),
            vec![OsString::from("/usr/share/applications")]
        )]
    );
}

#[test]
fn reveal_uses_the_file_manager_service_when_present() {
    let fm = Recording(RefCell::new(Vec::new()));
    let ran = Cell::new(false);
    let effects = reveal_with(&entry("/a/b.desktop"), &fm, &mut |_, _| {
        ran.set(true);
        Ok(())
    })
    .unwrap();
    assert_eq!(effects, vec![Effect::CloseWindow]);
    assert_eq!(fm.0.into_inner(), vec!["file:///a/b.desktop".to_string()]);
    assert!(!ran.get(), "fallback must not run when D-Bus succeeds");
}
