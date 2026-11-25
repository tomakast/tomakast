//! Theme-named icons (`accessories-calculator`, `document-open-recent`): the image file a
//! name stands for in the desktop's icon theme.
//!
//! Only Linux has icon themes (the freedesktop icon theme spec). On macOS and Windows a
//! theme name means nothing, and [`resolve`] finds no file: the launcher draws such an icon
//! as the bundled Lucide glyph of that name if there is one, else as the item's monogram. A
//! fallback there (built-in images for common names, or the OS's own symbols) belongs in
//! [`resolve`]'s non-Linux branch, so every caller gets it.
//!
//! A lookup reads the theme's `index.theme` files and probes icon directories: callers do
//! it once per name and off the UI thread.

use std::path::PathBuf;

/// The image file for icon `name` in the desktop's icon theme (found once per process, see
/// [`theme_from_env`]), its parents or `hicolor`; `None` when there is none, when `name`
/// is not a plain icon name, and on every OS but Linux.
pub fn resolve(name: &str) -> Option<PathBuf> {
    resolve_sized(name, ICON_SIZE)
}

/// The size icons are looked up at, in pixels: the launcher draws them at most this big.
pub const ICON_SIZE: u16 = 48;

/// The size the icon of a file type (`image-png`) or a folder is looked up at. Breeze keeps
/// its file-type icons in a scalable 64 px folder and none at 48, where the lookup would take
/// its small 16 px glyphs (their 3x folder); every common theme has 64 px file-type icons.
pub const FILE_TYPE_ICON_SIZE: u16 = 64;

/// [`resolve`] at `size` pixels.
pub fn resolve_sized(name: &str, size: u16) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::sync::OnceLock;
        static THEME: OnceLock<String> = OnceLock::new();
        lookup_sized(name, THEME.get_or_init(theme_from_env), size)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (name, size);
        None
    }
}

#[cfg(target_os = "linux")]
pub use linux::{
    FALLBACK_ICON_THEME, detect_icon_theme, lookup, lookup_sized, theme_chain, theme_from_env,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{KDEGLOBALS, config_home, current_desktops, ini_value};

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    /// The theme every icon lookup falls back to (freedesktop icon theme spec).
    pub const FALLBACK_ICON_THEME: &str = "hicolor";
    /// Plasma's icon theme when none is configured.
    const KDE_DEFAULT_ICON_THEME: &str = "breeze";
    /// The most themes of an inheritance chain looked in (a theme's own `Inherits` can loop).
    const MAX_CHAIN: usize = 16;

    /// Looks icon `name` up at [`super::ICON_SIZE`] in `theme`, every theme it inherits from
    /// and `hicolor`. Results are cached for the process. `name` must be a plain icon name:
    /// one with a path separator or a control character finds nothing (an absolute `Icon=`
    /// path of a desktop entry is the caller's to handle). Only absolute paths are answered:
    /// the library's last resort takes the name as a path, which for a plain name is a file
    /// of the working directory.
    pub fn lookup(name: &str, theme: &str) -> Option<PathBuf> {
        lookup_sized(name, theme, super::ICON_SIZE)
    }

    /// [`lookup`] at `size` pixels. The library looks in a theme and the themes it names in
    /// `Inherits`, not in theirs: each theme of [`theme_chain`] is asked in turn, so an icon
    /// two themes up the chain is found too.
    pub fn lookup_sized(name: &str, theme: &str, size: u16) -> Option<PathBuf> {
        if !is_icon_name(name) {
            return None;
        }
        chain_of(theme).iter().find_map(|theme| {
            freedesktop_icons::lookup(name)
                .with_size(size)
                .with_theme(theme)
                .with_cache()
                .find()
                .filter(|path| path.is_absolute())
        })
    }

    /// [`theme_chain`] of `theme` in the folders of [`icon_dirs`], read once per theme.
    fn chain_of(theme: &str) -> Vec<String> {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock, PoisonError};
        static CHAINS: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
        let mut chains = CHAINS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        chains
            .entry(theme.to_string())
            .or_insert_with(|| theme_chain(theme, &icon_dirs()))
            .clone()
    }

    /// `theme` and the themes it inherits from, depth first in `Inherits` order as the icon
    /// theme spec walks them (each once, at most [`MAX_CHAIN`]), with `hicolor` last. A
    /// theme's `index.theme` is the first one found in `icon_dirs`.
    pub fn theme_chain(theme: &str, icon_dirs: &[PathBuf]) -> Vec<String> {
        fn walk(theme: &str, icon_dirs: &[PathBuf], chain: &mut Vec<String>) {
            if chain.len() >= MAX_CHAIN
                || theme == FALLBACK_ICON_THEME
                || chain.iter().any(|t| t == theme)
            {
                return;
            }
            chain.push(theme.to_string());
            let index = icon_dirs
                .iter()
                .find_map(|dir| std::fs::read_to_string(dir.join(theme).join("index.theme")).ok());
            let parents = index
                .and_then(|text| ini_value(&text, "Icon Theme", "Inherits"))
                .unwrap_or_default();
            for parent in parents.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                walk(parent, icon_dirs, chain);
            }
        }
        let mut chain = Vec::new();
        walk(theme, icon_dirs, &mut chain);
        chain.push(FALLBACK_ICON_THEME.to_string());
        chain
    }

    /// The folders icon themes are in, as the library reads them: `~/.icons`, then the
    /// `icons` folder of `XDG_DATA_HOME` (else `~/.local/share`) and of each `XDG_DATA_DIRS`.
    fn icon_dirs() -> Vec<PathBuf> {
        use std::sync::OnceLock;
        static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
        DIRS.get_or_init(|| {
            let home = non_empty_env("HOME").map(PathBuf::from);
            let data_home = non_empty_env("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
            let data_dirs = non_empty_env("XDG_DATA_DIRS")
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            home.map(|h| h.join(".icons"))
                .into_iter()
                .chain(data_home.map(|d| d.join("icons")))
                .chain(std::env::split_paths(&data_dirs).map(|d| d.join("icons")))
                .filter(|d| d.is_absolute())
                .collect()
        })
        .clone()
    }

    /// Whether `name` can only name an icon, never a path.
    fn is_icon_name(name: &str) -> bool {
        !name.is_empty()
            && name != "."
            && name != ".."
            && !name
                .chars()
                .any(|c| c == '/' || c == '\\' || c.is_control())
    }

    /// The desktop's icon theme, from the environment: [`detect_icon_theme`] under
    /// `XDG_CONFIG_HOME` (else `~/.config`) for the desktops of `XDG_CURRENT_DESKTOP`.
    pub fn theme_from_env() -> String {
        match config_home() {
            Some(config_home) => detect_icon_theme(&config_home, &current_desktops()),
            None => FALLBACK_ICON_THEME.to_string(),
        }
    }

    /// The user's config directory: `XDG_CONFIG_HOME`, else `~/.config`.
    pub(crate) fn config_home() -> Option<PathBuf> {
        non_empty_env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| non_empty_env("HOME").map(|h| PathBuf::from(h).join(".config")))
    }

    /// The desktops of `XDG_CURRENT_DESKTOP`, such as `KDE`.
    pub(crate) fn current_desktops() -> Vec<String> {
        std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The files under the config directory where KDE keeps its global settings, the user's
    /// own first, then Plasma's look-and-feel defaults.
    pub(crate) const KDEGLOBALS: [&str; 2] = ["kdeglobals", "kdedefaults/kdeglobals"];

    /// The active icon theme, read from the desktop's config under `config_home`.
    ///
    /// KDE: `[Icons] Theme=` in `kdeglobals`, then in `kdedefaults/kdeglobals`
    /// (where Plasma keeps the look-and-feel defaults), else `breeze`.
    /// Elsewhere: `gtk-icon-theme-name` in the GTK 4 or GTK 3 `settings.ini`
    /// (gsettings is not queried, to avoid spawning a process), else `hicolor`.
    pub fn detect_icon_theme(config_home: &Path, current_desktops: &[String]) -> String {
        let read = |rel: &str| std::fs::read_to_string(config_home.join(rel)).ok();
        if current_desktops.iter().any(|d| d == "KDE") {
            return KDEGLOBALS
                .iter()
                .find_map(|rel| ini_value(&read(rel)?, "Icons", "Theme"))
                .unwrap_or_else(|| KDE_DEFAULT_ICON_THEME.to_string());
        }
        ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
            .iter()
            .find_map(|rel| ini_value(&read(rel)?, "Settings", "gtk-icon-theme-name"))
            .unwrap_or_else(|| FALLBACK_ICON_THEME.to_string())
    }

    /// The non-empty value of `key` in `[group]` of an INI-style file; quotes are stripped.
    pub(crate) fn ini_value(text: &str, group: &str, key: &str) -> Option<String> {
        let mut in_group = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_group = line == format!("[{group}]");
            } else if in_group
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                let v = v.trim().trim_matches('"');
                return (!v.is_empty()).then(|| v.to_string());
            }
        }
        None
    }

    fn non_empty_env(key: &str) -> Option<OsString> {
        std::env::var_os(key).filter(|v| !v.is_empty())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn config_fixture(name: &str) -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/icon-theme-config")
                .join(name)
        }

        fn desktops(names: &[&str]) -> Vec<String> {
            names.iter().map(|s| s.to_string()).collect()
        }

        #[test]
        fn kde_icon_theme_comes_from_the_icons_group_of_kdeglobals() {
            // `[Icons] Theme=` wins over `kdedefaults/` and over `Theme=` in other groups.
            assert_eq!(
                detect_icon_theme(&config_fixture("kde"), &desktops(&["KDE"])),
                "Papirus-Dark"
            );
            // Plasma writes the default only to `kdedefaults/kdeglobals`.
            assert_eq!(
                detect_icon_theme(&config_fixture("kde-defaults-only"), &desktops(&["KDE"])),
                "breeze-dark"
            );
            // No theme configured at all: Plasma's built-in default.
            assert_eq!(
                detect_icon_theme(&config_fixture("missing"), &desktops(&["KDE"])),
                "breeze"
            );
        }

        #[test]
        fn other_desktops_use_the_gtk_icon_theme_then_hicolor() {
            assert_eq!(
                detect_icon_theme(&config_fixture("gtk"), &desktops(&["GNOME"])),
                "Papirus"
            );
            // kdeglobals is ignored outside KDE.
            assert_eq!(
                detect_icon_theme(&config_fixture("kde"), &desktops(&["GNOME"])),
                "hicolor"
            );
            assert_eq!(
                detect_icon_theme(&config_fixture("missing"), &[]),
                "hicolor"
            );
        }

        #[test]
        fn the_chain_follows_inherits_all_the_way_up() {
            let root = tempfile::tempdir().unwrap();
            let (user, system) = (root.path().join("user"), root.path().join("system"));
            let theme = |dir: &Path, name: &str, inherits: &str| {
                std::fs::create_dir_all(dir.join(name)).unwrap();
                std::fs::write(
                    dir.join(name).join("index.theme"),
                    format!("[Icon Theme]\nName={name}\nInherits={inherits}\n"),
                )
                .unwrap();
            };
            theme(&user, "Custom-Dark", "Custom, Other");
            theme(&system, "Custom", "breeze-dark");
            theme(&system, "breeze-dark", "breeze,hicolor");
            theme(&system, "breeze", "hicolor");
            // A loop, and a parent that is not installed.
            theme(&system, "Other", "Custom-Dark,Missing");
            // The user's copy of a theme stands for the system's.
            theme(&user, "breeze", "hicolor");
            let dirs = [user, system];
            assert_eq!(
                theme_chain("Custom-Dark", &dirs),
                [
                    "Custom-Dark",
                    "Custom",
                    "breeze-dark",
                    "breeze",
                    "Other",
                    "Missing",
                    "hicolor"
                ]
            );
            assert_eq!(theme_chain("hicolor", &dirs), ["hicolor"]);
            assert_eq!(theme_chain("Nowhere", &dirs), ["Nowhere", "hicolor"]);
        }

        #[test]
        fn ini_value_reads_only_the_named_group() {
            let text = "[A]\nk=a\n\n[B]\n k = \"b\" \n[C]\nk=c\n";
            assert_eq!(ini_value(text, "B", "k").as_deref(), Some("b"));
            assert_eq!(ini_value(text, "D", "k"), None);
            assert_eq!(ini_value("[B]\nk=\n", "B", "k"), None);
        }

        #[test]
        fn icon_names_are_never_paths() {
            for name in [
                "folder",
                "org.gnome.Nautilus",
                "accessories-calculator",
                "a b",
            ] {
                assert!(is_icon_name(name), "{name:?}");
            }
            for name in ["", ".", "..", "a/b", "/abs", "a\\b", "a\nb", "a\0b"] {
                assert!(!is_icon_name(name), "{name:?}");
            }
        }
    }
}
