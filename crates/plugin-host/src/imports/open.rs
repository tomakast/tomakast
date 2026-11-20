//! `open`: URLs and files handed to the user's default application, under the `open`
//! permission, only during a call the user started (`run`, `action-chosen` or
//! `form-submitted`; Rulings X and AL), and at most once per call.
//! The host checks first and gives the opener only the parsed form:
//! - a URL passes [`tomakast_core::checked_url`], the check markdown links use too (no
//!   whitespace or control characters, `http`, `https` or `mailto`, no user name or password,
//!   and a `mailto` keeps only `to`, `cc`, `bcc`, `subject` and `body`; Ruling AC);
//! - a path is absolute and canonicalizes to an existing regular file or folder (symbolic
//!   links are followed, and both the given name and the target's name are checked) that is
//!   not executable: no exec bit on a file (Unix), and none of [`REFUSED_EXTENSIONS`], on
//!   every OS (Ruling AA). A refused path gets one generic error, so that a plugin cannot
//!   probe the filesystem; the reason is logged.
//!
//! The check and the open are not atomic: a file swapped between them is opened as it is
//! then. Launching programs stays a privilege of the built-in modules.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use tomakast_core::checked_url;

use super::HostState;
use crate::bindings::tomakast::plugin::open::Host;

const DENIED: &str = "the open permission was not granted";
const NO_USER_ACTION: &str = "opening needs a user action (run, a chosen action, a submitted \
                              form, or a widget's action, key or completed click)";
const ONE_PER_CALL: &str = "a call may open at most one URL or path";
/// `open-url` and `open-path` calls one guest call may make (Ruling AL): each spawns a
/// process.
pub const MAX_OPENS_PER_CALL: u32 = 1;
/// The one refusal a plugin sees for any path that may not be opened.
pub const PATH_REFUSED: &str = "this path may not be opened";

/// Extensions (lowercase, without the dot) of files that run code, install software or open
/// something else when opened, on any OS. Refused everywhere: a file copied between systems
/// keeps its meaning.
pub const REFUSED_EXTENSIONS: &[&str] = &[
    // Windows executables, scripts, shortcuts and installers.
    "exe",
    "bat",
    "cmd",
    "ps1",
    "lnk",
    "msi",
    "com",
    "scr",
    "vbs",
    "js",
    "pif",
    "cpl",
    "hta",
    "jse",
    "vbe",
    "wsf",
    "wsh",
    "msc",
    "msp",
    "reg",
    "url",
    // macOS bundles, scripts, shortcuts and installers.
    "app",
    "command",
    "tool",
    "terminal",
    "workflow",
    "action",
    "prefpane",
    "pkg",
    "mpkg",
    "scpt",
    "dmg",
    "webloc",
    "inetloc",
    "fileloc",
    // Linux launchers, packages and scripts.
    "desktop",
    "appimage",
    "flatpakref",
    "deb",
    "rpm",
    "sh",
    "py",
    "pl",
    "rb",
    // Runs on the Java runtime.
    "jar",
];

/// Opens what the host checked. The daemon uses [`PlatformOpener`]; tests record instead.
pub trait Opener: Send + Sync {
    fn open_url(&self, url: &str) -> anyhow::Result<()>;
    fn open_path(&self, path: &Path) -> anyhow::Result<()>;
}

/// The user's default applications, through `tomakast_platform::open`.
pub struct PlatformOpener;

impl Opener for PlatformOpener {
    fn open_url(&self, url: &str) -> anyhow::Result<()> {
        tomakast_platform::open::open_url(url)
    }

    fn open_path(&self, path: &Path) -> anyhow::Result<()> {
        tomakast_platform::open::open_path(path)
    }
}

/// Why `names` (the given name and the target's), a folder or file with or without an exec
/// bit, may not be opened.
fn refuse(names: [&Path; 2], is_dir: bool, exec: bool) -> Option<String> {
    for name in names {
        let extension = name
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);
        if let Some(ext) = extension
            && REFUSED_EXTENSIONS.contains(&ext.as_str())
        {
            return Some(format!(".{ext} files may not be opened"));
        }
    }
    // Folders carry the exec bit (search permission); only files are programs by it.
    if exec && !is_dir {
        return Some("executable files may not be opened".into());
    }
    None
}

/// `text` as the path to open (canonical), or why it may not be opened (for the log).
pub(crate) fn check_path(text: &str) -> Result<PathBuf, String> {
    let given = Path::new(text);
    if text.is_empty() || !given.is_absolute() {
        return Err("the path is not absolute".into());
    }
    let canonical = given
        .canonicalize()
        .map_err(|e| format!("the path cannot be resolved: {e}"))?;
    let meta = std::fs::metadata(&canonical).map_err(|e| format!("no metadata: {e}"))?;
    if !meta.is_file() && !meta.is_dir() {
        return Err("not a file or folder".into());
    }
    #[cfg(unix)]
    let exec = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let exec = false;
    let names = [Path::new(given.file_name().unwrap_or_default()), &canonical];
    match refuse(names, meta.is_dir(), exec) {
        Some(reason) => Err(reason),
        None => Ok(canonical),
    }
}

impl HostState {
    fn may_open(&self) -> Result<(), String> {
        if !self.permissions.open {
            return Err(DENIED.into());
        }
        if !self.user_action {
            return Err(NO_USER_ACTION.into());
        }
        if self.opens >= MAX_OPENS_PER_CALL {
            return Err(ONE_PER_CALL.into());
        }
        Ok(())
    }
}

impl Host for HostState {
    fn open_url(&mut self, url: String) -> Result<(), String> {
        self.record_import("open.open-url");
        self.may_open()?;
        let url = checked_url(&url)?;
        self.opens += 1;
        self.opener.open_url(&url).map_err(|e| {
            tracing::warn!(target: "plugin", plugin = %self.plugin, "open-url failed: {e:#}");
            "the URL could not be opened".to_string()
        })
    }

    fn open_path(&mut self, path: String) -> Result<(), String> {
        self.record_import("open.open-path");
        self.may_open()?;
        let path = check_path(&path).map_err(|reason| {
            tracing::info!(target: "plugin", plugin = %self.plugin, "open-path refused: {reason}");
            PATH_REFUSED.to_string()
        })?;
        self.opens += 1;
        self.opener.open_path(&path).map_err(|e| {
            tracing::warn!(target: "plugin", plugin = %self.plugin, "open-path failed: {e:#}");
            PATH_REFUSED.to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(name: &str, is_dir: bool, exec: bool) -> bool {
        let name = Path::new(name);
        refuse([name, name], is_dir, exec).is_some()
    }

    /// Ruling AA: one list on every OS, case-insensitive; the exec bit on files.
    #[test]
    fn executables_and_launchers_are_refused_on_every_os() {
        assert!(!refused("/home/u/notes.txt", false, false));
        assert!(!refused("/home/u/photo.jpg", false, false));
        assert!(
            !refused("/home/u/folder", true, true),
            "folders have the exec bit"
        );
        assert!(refused("/home/u/run", false, true), "an exec bit on a file");
        for ext in REFUSED_EXTENSIONS {
            assert!(refused(&format!("/x/file.{ext}"), false, false), "{ext}");
            let upper = format!("/x/FILE.{}", ext.to_uppercase());
            assert!(refused(&upper, false, false), "{upper}");
        }
        for ext in [
            "exe",
            "bat",
            "cmd",
            "ps1",
            "lnk",
            "msi",
            "com",
            "scr",
            "vbs",
            "js",
            "desktop",
            "app",
            "jar",
            "appimage",
            "webloc",
            "inetloc",
            "fileloc",
            "url",
            "flatpakref",
            "deb",
            "rpm",
            "sh",
            "py",
            "pl",
            "rb",
            "command",
            "tool",
            "scpt",
            "pkg",
            "dmg",
        ] {
            assert!(REFUSED_EXTENSIONS.contains(&ext), "{ext} is in the list");
        }
        assert!(
            refused("/Applications/Calculator.app", true, true),
            "a bundle folder"
        );
        // The link's own name counts too.
        let names = [Path::new("evil.desktop"), Path::new("/home/u/notes.txt")];
        assert!(refuse(names, false, false).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn paths_must_be_absolute_and_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            check_path(file.to_str().unwrap()).unwrap(),
            file.canonicalize().unwrap()
        );
        assert!(check_path("a.txt").is_err());
        assert!(check_path("").is_err());
        assert!(check_path(tmp.path().join("missing").to_str().unwrap()).is_err());
        assert!(check_path("/dev/null").is_err(), "a device");
    }
}
