//! The application bundles (`Safari.app`) in macOS's application folders: plain Rust, so it
//! is tested on every OS.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tomakast_core::{Action, Item, ItemId, ItemKind};

use crate::AppEntry;

/// How many folders deep a bundle is looked for under an application folder: the folder
/// itself and its plain subfolders (`Utilities`, a suite's own folder).
const MAX_DEPTH: usize = 2;

/// The application folders of macOS, `home`'s own first, in the order a name found twice is
/// taken from.
pub fn application_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    home.map(|h| h.join("Applications"))
        .into_iter()
        .chain(
            ["/Applications", "/System/Applications"]
                .into_iter()
                .map(PathBuf::from),
        )
        .collect()
}

/// Bundles that live outside the application folders and are still apps people open.
pub const EXTRA_BUNDLES: &[&str] = &["/System/Library/CoreServices/Finder.app"];

/// Whether `name` is a bundle's: `*.app`.
fn is_bundle_name(name: &OsString) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
}

/// The `.app` bundles in `dirs` and their plain subfolders (never inside a bundle, never
/// hidden entries), then each of `extra` that exists, sorted by name within a folder. A
/// bundle name found again (in a later folder, or deeper) is left out: the first one stands.
pub fn find_bundles(dirs: &[PathBuf], extra: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, seen: &mut HashSet<OsString>, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<(OsString, PathBuf)> = entries
            .flatten()
            .map(|entry| (entry.file_name(), entry.path()))
            .filter(|(name, _)| !name.to_string_lossy().starts_with('.'))
            .collect();
        entries.sort();
        let mut subfolders = Vec::new();
        for (name, path) in entries {
            // Through links: an app may be linked into /Applications.
            if !path.is_dir() {
                continue;
            }
            if is_bundle_name(&name) {
                if seen.insert(name) {
                    found.push(path);
                }
            } else if depth + 1 < MAX_DEPTH {
                subfolders.push(path);
            }
        }
        for folder in subfolders {
            walk(&folder, depth + 1, seen, found);
        }
    }
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        walk(dir, 0, &mut seen, &mut found);
    }
    for bundle in extra.iter().map(PathBuf::from) {
        if bundle.is_dir()
            && let Some(name) = bundle.file_name()
            && seen.insert(name.to_owned())
        {
            found.push(bundle);
        }
    }
    found
}

/// A bundle's name without `.app`: `Safari.app` is "Safari".
pub fn bundle_stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    without_app(&name).to_string()
}

/// `name` without a trailing `.app`, in any case.
fn without_app(name: &str) -> &str {
    match name.len().checked_sub(4) {
        Some(cut) if name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(".app") => {
            &name[..cut]
        }
        _ => name,
    }
}

/// The item and entry of the bundle at `path`, titled `title` (the name Finder shows, else
/// the bundle's own): its id is `apps/<bundle name>`, its bundle name a keyword when the
/// title differs (an English name under a localized one).
pub fn bundle_item(path: &Path, title: Option<String>) -> (Item, AppEntry) {
    let stem = bundle_stem(path);
    let title = title
        .map(|title| without_app(&title).to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| stem.clone());
    let keywords = if stem != title {
        vec![stem]
    } else {
        Vec::new()
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let action = |id: &str, title: &str| Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    };
    let item = Item {
        id: ItemId::new("apps", &name),
        title,
        subtitle: None,
        keywords,
        icon: None,
        kind: ItemKind::Application,
        source: None,
        actions: vec![
            action("open", "Open"),
            action("reveal", "Show in Finder"),
            action("copy-path", "Copy Path"),
        ],
    };
    let entry = AppEntry {
        desktop_file: path.to_path_buf(),
        exec: Vec::new(),
        terminal: false,
        working_dir: None,
        mime_types: Vec::new(),
    };
    (item, entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mkdir(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    #[test]
    fn bundles_in_the_folders_and_their_plain_subfolders() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("home/Applications");
        let apps = tmp.path().join("Applications");
        let system = tmp.path().join("System/Applications");
        // Bundles hold apps of their own (helpers): never looked into.
        mkdir(&apps.join("Safari.app/Contents/MacOS/Helper.app"));
        mkdir(&apps.join("Utilities/Terminal.app"));
        mkdir(&apps.join("Adobe Suite/Photoshop.app"));
        // Too deep.
        mkdir(&apps.join("Adobe Suite/Plugins/Hidden.app"));
        mkdir(&apps.join(".hidden/Secret.app"));
        mkdir(&apps.join(".Ghost.app"));
        fs::write(apps.join("NotADir.app"), b"").unwrap();
        mkdir(&apps.join("Notes.APP"));
        // A name the user's folder already has: the user's stands.
        mkdir(&user.join("Notes.APP"));
        mkdir(&system.join("Calculator.app"));
        mkdir(&system.join("Utilities/Terminal.app"));
        let extra = tmp.path().join("CoreServices/Finder.app");
        mkdir(&extra);
        let missing = tmp.path().join("CoreServices/Missing.app");

        let found = find_bundles(
            &[user.clone(), apps.clone(), system.clone()],
            &[extra.to_str().unwrap(), missing.to_str().unwrap()],
        );
        assert_eq!(
            found,
            [
                user.join("Notes.APP"),
                apps.join("Safari.app"),
                apps.join("Adobe Suite/Photoshop.app"),
                apps.join("Utilities/Terminal.app"),
                system.join("Calculator.app"),
                extra,
            ]
        );
    }

    #[test]
    fn application_dirs_start_with_the_users_own() {
        assert_eq!(
            application_dirs(Some(Path::new("/Users/me"))),
            [
                PathBuf::from("/Users/me/Applications"),
                "/Applications".into(),
                "/System/Applications".into()
            ]
        );
        assert_eq!(application_dirs(None).len(), 2);
    }

    #[test]
    fn a_bundle_is_an_app_titled_as_finder_shows_it() {
        let path = Path::new("/System/Applications/Calculator.app");
        let (item, entry) = bundle_item(path, Some("Калькулятор".into()));
        assert_eq!(item.id.as_str(), "apps/Calculator.app");
        assert_eq!(item.title, "Калькулятор");
        // The English name still finds it.
        assert_eq!(item.keywords, ["Calculator"]);
        assert_eq!(item.kind, ItemKind::Application);
        let actions: Vec<_> = item.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["open", "reveal", "copy-path"]);
        assert_eq!(entry.desktop_file, path);

        // Finder shows the extension when the user asked it to: never in the title.
        let (item, _) = bundle_item(
            Path::new("/Applications/Safari.app"),
            Some("Safari.app".into()),
        );
        assert_eq!(item.title, "Safari");
        assert!(item.keywords.is_empty());
        // Finder shows a `:` in a file name as `/`.
        let (item, _) = bundle_item(Path::new("/Applications/A:B.app"), Some("A/B".into()));
        assert_eq!(item.title, "A/B");
        let (item, _) = bundle_item(Path::new("/Applications/Zed.app"), None);
        assert_eq!(item.title, "Zed");
    }

    #[test]
    fn stems_lose_only_the_bundle_extension() {
        assert_eq!(bundle_stem(Path::new("/Applications/Safari.app")), "Safari");
        assert_eq!(bundle_stem(Path::new("/A/Notes.APP")), "Notes");
        assert_eq!(
            bundle_stem(Path::new("/A/Visual Studio Code.app")),
            "Visual Studio Code"
        );
        assert_eq!(bundle_stem(Path::new("/A/Я.app")), "Я");
        assert_eq!(bundle_stem(Path::new("/A/plain")), "plain");
    }
}
