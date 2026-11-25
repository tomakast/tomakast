//! Linux backend: freedesktop `.desktop` scanning, launching and watching.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use freedesktop_desktop_entry::DesktopEntry;
use tomakast_core::{Action, Effect, Icon, Item, ItemId, ItemKind};
use tomakast_platform::icon_theme;

use crate::process::{reap, spawn_detached};
use crate::{AppEntry, ScanOptions};

pub(crate) fn options_from_env() -> ScanOptions {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = non_empty_env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let data_dirs = non_empty_env("XDG_DATA_DIRS")
        .unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));

    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |dir: PathBuf, must_exist: bool| {
        if (!must_exist || dir.is_dir()) && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if let Some(data_home) = data_home {
        push(data_home.join("applications"), false);
    }
    for dir in std::env::split_paths(&data_dirs).filter(|d| !d.as_os_str().is_empty()) {
        push(dir.join("applications"), false);
    }
    if let Some(home) = &home {
        push(
            home.join(".local/share/flatpak/exports/share/applications"),
            true,
        );
    }
    push("/var/lib/flatpak/exports/share/applications".into(), true);
    push("/var/lib/snapd/desktop/applications".into(), true);

    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| non_empty_env(k))
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default();
    let current_desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let icon_theme = icon_theme::theme_from_env();

    ScanOptions {
        dirs,
        locales: locales_from_lang(&lang),
        current_desktops,
        path_env: std::env::var_os("PATH"),
        icon_theme,
    }
}

fn non_empty_env(key: &str) -> Option<OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}

/// `ru_RU.UTF-8` → `["ru_RU", "ru"]`; `C`/`POSIX`/empty → no locales.
fn locales_from_lang(lang: &str) -> Vec<String> {
    let base = lang.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }
    let mut out = vec![base.to_string()];
    if let Some((language, _)) = base.split_once('_') {
        out.push(language.to_string());
    }
    out
}

pub(crate) fn scan(opts: &ScanOptions) -> Vec<(Item, AppEntry)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for dir in &opts.dirs {
        let mut files = Vec::new();
        collect_desktop_files(dir, &mut files);
        files.sort();
        for path in files {
            let Ok(rel) = path.strip_prefix(dir) else {
                continue;
            };
            let desktop_id = rel.to_string_lossy().replace('/', "-");
            // The first directory wins, even when its entry is filtered out below:
            // a user-level NoDisplay override must hide the system entry.
            if !seen.insert(desktop_id.clone()) {
                continue;
            }
            if let Some(found) = load_entry(&path, &desktop_id, opts) {
                out.push(found);
            }
        }
    }
    out
}

fn collect_desktop_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        // `metadata` follows symlinks, which flatpak/snap exports rely on.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect_desktop_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "desktop") {
            out.push(path);
        }
    }
}

fn load_entry(path: &Path, desktop_id: &str, opts: &ScanOptions) -> Option<(Item, AppEntry)> {
    let de = match DesktopEntry::from_path(path, Some(&opts.locales)) {
        Ok(de) => de,
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "skipping unreadable desktop file");
            return None;
        }
    };
    if de.type_() != Some("Application") || de.no_display() || de.hidden() {
        return None;
    }
    if let Some(only) = de.only_show_in() {
        let only: Vec<&str> = only.into_iter().filter(|s| !s.is_empty()).collect();
        if !only
            .iter()
            .any(|d| opts.current_desktops.iter().any(|c| c == d))
        {
            return None;
        }
    }
    if let Some(not) = de.not_show_in()
        && not
            .iter()
            .any(|d| opts.current_desktops.iter().any(|c| c == d))
    {
        return None;
    }
    if let Some(try_exec) = de.try_exec()
        && !try_exec.is_empty()
        && !executable_exists(try_exec, opts.path_env.as_deref())
    {
        return None;
    }
    let Some(title) = de.name(&opts.locales).map(|n| n.into_owned()) else {
        tracing::warn!(path = %path.display(), "skipping desktop file without Name");
        return None;
    };
    // A missing `Exec` is an error here too (`ExecFieldNotFound`).
    let exec = match de.parse_exec() {
        Ok(exec) => exec,
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "skipping desktop file with bad Exec");
            return None;
        }
    };

    let generic = de
        .generic_name(&opts.locales)
        .map(|g| g.into_owned())
        .filter(|g| !g.is_empty());
    let comment = de
        .comment(&opts.locales)
        .map(|c| c.into_owned())
        .filter(|c| !c.is_empty());
    let subtitle = generic.clone().or(comment);

    let mut keywords: Vec<String> = de
        .keywords(&opts.locales)
        .map(|k| {
            k.into_iter()
                .map(|k| k.into_owned())
                .filter(|k| !k.is_empty())
                .collect()
        })
        .unwrap_or_default();
    keywords.extend(generic);
    if let Some(base) = exec
        .first()
        .and_then(|e| Path::new(e).file_name())
        .map(|b| b.to_string_lossy().into_owned())
    {
        keywords.push(base);
    }

    let item = Item {
        id: ItemId::new("apps", desktop_id),
        title,
        subtitle,
        keywords,
        icon: de
            .icon()
            .and_then(|name| resolve_icon(name, &opts.icon_theme)),
        kind: ItemKind::Application,
        source: None,
        actions: vec![
            Action {
                id: "open".into(),
                title: "Open".into(),
                shortcut: None,
            },
            Action {
                id: "reveal".into(),
                title: "Show in File Manager".into(),
                shortcut: None,
            },
            Action {
                id: "copy-path".into(),
                title: "Copy Path".into(),
                shortcut: None,
            },
        ],
    };
    let entry = AppEntry {
        desktop_file: path.to_path_buf(),
        exec,
        terminal: de.terminal(),
        working_dir: de.path().filter(|p| !p.is_empty()).map(PathBuf::from),
        mime_types: de
            .mime_type()
            .map(|types| {
                types
                    .into_iter()
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    };
    Some((item, entry))
}

/// An entry's `Icon=`: an absolute path as it is, else a name looked up in `theme`, its
/// parents and `hicolor` ([`icon_theme::lookup`], cached for the process, so a rescan does
/// not walk the icon directories again).
fn resolve_icon(name: &str, theme: &str) -> Option<Icon> {
    if name.is_empty() {
        return None;
    }
    if Path::new(name).is_absolute() {
        return Some(Icon::File(name.into()));
    }
    icon_theme::lookup(name, theme).map(Icon::File)
}

fn executable_exists(name: &str, path_env: Option<&std::ffi::OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if name.contains('/') {
        return is_exec(Path::new(name));
    }
    path_env.is_some_and(|paths| std::env::split_paths(paths).any(|dir| is_exec(&dir.join(name))))
}

pub(crate) fn launch(entry: &AppEntry) -> anyhow::Result<()> {
    if entry.terminal {
        let mut cmd = Command::new("gio");
        cmd.arg("launch").arg(&entry.desktop_file);
        let child = spawn_detached(cmd).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("terminal apps need gio")
            } else {
                anyhow::Error::new(err).context("failed to run gio launch")
            }
        })?;
        reap(child);
        return Ok(());
    }
    let (program, args) = entry
        .exec
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("empty Exec for {}", entry.desktop_file.display()))?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = &entry.working_dir {
        cmd.current_dir(dir);
    }
    let child = spawn_detached(cmd)
        .map_err(|err| anyhow::Error::new(err).context(format!("failed to launch {program}")))?;
    reap(child);
    Ok(())
}

/// The command that opens `file` with the application: `gio launch <desktop file> <file>`.
pub(crate) fn launch_with_file_command(entry: &AppEntry, file: &Path) -> Command {
    let mut cmd = Command::new("gio");
    cmd.arg("launch").arg(&entry.desktop_file).arg(file);
    cmd
}

pub(crate) fn launch_with_file(entry: &AppEntry, file: &Path) -> anyhow::Result<()> {
    let child = spawn_detached(launch_with_file_command(entry, file)).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("opening a file with an application needs gio")
        } else {
            anyhow::Error::new(err).context("failed to run gio launch")
        }
    })?;
    reap(child);
    Ok(())
}

/// Why the file manager service could not show an item.
#[derive(Debug)]
pub enum FileManagerError {
    /// No `org.freedesktop.FileManager1` service, or no session bus.
    Unavailable(String),
}

impl std::fmt::Display for FileManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileManagerError::Unavailable(why) => write!(f, "file manager unavailable: {why}"),
        }
    }
}

impl std::error::Error for FileManagerError {}

/// The `org.freedesktop.FileManager1` D-Bus service, behind a trait so tests
/// never touch the real session bus.
pub trait FileManager {
    fn show_items(&self, uris: &[String]) -> Result<(), FileManagerError>;
}

/// Calls `ShowItems` on the session bus (blocking; runs on the module worker).
pub struct DbusFileManager;

impl FileManager for DbusFileManager {
    fn show_items(&self, uris: &[String]) -> Result<(), FileManagerError> {
        let unavailable = |err: zbus::Error| FileManagerError::Unavailable(err.to_string());
        let conn = zbus::blocking::Connection::session().map_err(unavailable)?;
        conn.call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(uris, ""),
        )
        .map(|_| ())
        .map_err(unavailable)
    }
}

/// `file://` + the absolute path with everything but unreserved characters
/// and `/` percent-encoded.
pub fn desktop_file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The fallback when the file manager service is missing: open the parent
/// directory with `xdg-open`. Built here, run by the caller.
pub fn reveal_command(desktop_file: &Path) -> (String, Vec<OsString>) {
    let dir = desktop_file.parent().unwrap_or_else(|| Path::new("/"));
    ("xdg-open".to_string(), vec![dir.as_os_str().to_owned()])
}

/// Shows the `.desktop` file in the file manager: D-Bus first, then
/// `run(program, args)` with the fallback command.
pub fn reveal_with(
    entry: &AppEntry,
    file_manager: &dyn FileManager,
    run: &mut dyn FnMut(&str, &[OsString]) -> std::io::Result<()>,
) -> anyhow::Result<Vec<Effect>> {
    match file_manager.show_items(&[desktop_file_uri(&entry.desktop_file)]) {
        Ok(()) => {}
        Err(err) => {
            tracing::debug!(%err, "FileManager1.ShowItems failed, falling back to xdg-open");
            let (program, args) = reveal_command(&entry.desktop_file);
            run(&program, &args).map_err(|err| {
                anyhow::Error::new(err).context(format!("failed to run {program}"))
            })?;
        }
    }
    Ok(vec![Effect::CloseWindow])
}

/// Reveals for real: the session bus, then a detached `xdg-open`.
pub(crate) fn reveal(entry: &AppEntry) -> anyhow::Result<Vec<Effect>> {
    reveal_with(entry, &DbusFileManager, &mut |program, args| {
        let mut cmd = Command::new(program);
        cmd.args(args);
        spawn_detached(cmd).map(reap)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_with_file_is_gio_launch_without_a_shell() {
        let entry = AppEntry {
            desktop_file: "/usr/share/applications/viewer.desktop".into(),
            exec: vec!["viewer".into()],
            terminal: false,
            working_dir: None,
            mime_types: vec![],
        };
        let cmd = launch_with_file_command(&entry, Path::new("/tmp/a b; rm -rf x.png"));
        assert_eq!(cmd.get_program(), "gio");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                "launch",
                "/usr/share/applications/viewer.desktop",
                "/tmp/a b; rm -rf x.png"
            ]
        );
    }

    #[test]
    fn locales_parse() {
        assert_eq!(locales_from_lang("ru_RU.UTF-8"), ["ru_RU", "ru"]);
        assert_eq!(locales_from_lang("de_DE@euro"), ["de_DE", "de"]);
        assert_eq!(locales_from_lang("fr"), ["fr"]);
        assert!(locales_from_lang("C.UTF-8").is_empty());
        assert!(locales_from_lang("").is_empty());
    }
}
