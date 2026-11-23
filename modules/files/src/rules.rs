//! What the index covers: the roots and what is skipped by name or by path. Gitignore files
//! are the walker's business (see `index`), not this module's.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};

/// Folders skipped wherever they are.
const BUILTIN_DIR_NAMES: &[&str] = &["node_modules", "target", "__pycache__", ".venv", "venv"];

/// The `[files]` settings the rules are built from. A module-local mirror of the settings
/// table; the settings layer maps onto it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub roots: Vec<String>,
    pub exclude: Vec<String>,
    pub hidden: bool,
    /// Inside repositories, honour `.gitignore`, `.git/info/exclude` and (with
    /// `global_ignore`) the global gitignore.
    pub gitignore: bool,
    /// Honour the user's global gitignore (on in production; tests turn it off so they do
    /// not depend on the machine's git config).
    pub global_ignore: bool,
}

impl Default for Coverage {
    fn default() -> Self {
        Coverage {
            roots: vec!["~".to_string()],
            exclude: Vec::new(),
            hidden: false,
            gitignore: true,
            global_ignore: true,
        }
    }
}

/// Expanded roots, absolute excluded prefixes, the name globs and the hidden switch.
#[derive(Clone, Debug)]
pub struct Rules {
    roots: Vec<PathBuf>,
    prefixes: Vec<PathBuf>,
    names: GlobSet,
    hidden: bool,
    gitignore: bool,
    global_ignore: bool,
}

impl Rules {
    pub fn new(c: &Coverage, home: &Path) -> Rules {
        let roots = c.roots.iter().map(|r| expand(r, home)).collect();
        let mut prefixes = Vec::new();
        if cfg!(target_os = "macos") {
            prefixes.push(home.join("Library"));
        }
        if cfg!(windows) {
            prefixes.push(home.join("AppData"));
        }
        let mut globs = GlobSetBuilder::new();
        for entry in &c.exclude {
            if is_path_entry(entry) {
                prefixes.push(expand(entry, home));
            } else {
                match Glob::new(entry) {
                    Ok(g) => {
                        globs.add(g);
                    }
                    Err(e) => tracing::warn!("files: bad exclude pattern {entry:?}: {e}"),
                }
            }
        }
        let names = globs.build().unwrap_or_else(|_| GlobSet::empty());
        Rules {
            roots,
            prefixes,
            names,
            hidden: c.hidden,
            gitignore: c.gitignore,
            global_ignore: c.global_ignore,
        }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether dot entries are indexed.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// Whether `.gitignore` files and `.git/info/exclude` apply inside repositories.
    pub fn gitignore(&self) -> bool {
        self.gitignore
    }

    /// Whether the user's global gitignore applies (only with [`gitignore`](Self::gitignore)).
    pub fn global_ignore(&self) -> bool {
        self.gitignore && self.global_ignore
    }

    /// Name-level and prefix-level exclusion (built-in names, `exclude` globs and paths,
    /// hidden when !hidden). Gitignore is applied by the walker, not here.
    pub fn excludes(&self, path: &Path, is_dir: bool) -> bool {
        if self.prefixes.iter().any(|p| path.starts_with(p)) {
            return true;
        }
        let Some(name) = path.file_name() else {
            return false;
        };
        if !self.hidden && is_dot(name) {
            return true;
        }
        if is_dir && BUILTIN_DIR_NAMES.iter().any(|b| name == OsStr::new(b)) {
            return true;
        }
        self.names.is_match(Path::new(name))
    }
}

fn is_dot(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// `~`, `/...` or (Windows) `C:...` entries are paths; anything else is a name glob.
fn is_path_entry(entry: &str) -> bool {
    if entry.starts_with('~') || entry.starts_with('/') {
        return true;
    }
    let b = entry.as_bytes();
    cfg!(windows) && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn expand(entry: &str, home: &Path) -> PathBuf {
    if entry == "~" {
        return home.to_path_buf();
    }
    match entry.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(exclude: &[&str], hidden: bool) -> Rules {
        let c = Coverage {
            roots: vec!["~".into(), "~/docs".into(), "/data".into()],
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            hidden,
            gitignore: true,
            global_ignore: false,
        };
        Rules::new(&c, Path::new("/home/u"))
    }

    #[test]
    fn roots_are_expanded() {
        let r = rules(&[], false);
        let roots: Vec<_> = r.roots().iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(roots, ["/home/u", "/home/u/docs", "/data"]);
    }

    #[test]
    fn builtin_names_hidden_globs_and_prefixes() {
        let r = rules(&["*.iso", "~/games", "/mnt/x"], false);
        assert!(r.excludes(Path::new("/home/u/node_modules"), true));
        assert!(!r.excludes(Path::new("/home/u/node_modules"), false));
        assert!(r.excludes(Path::new("/home/u/p/target"), true));
        assert!(r.excludes(Path::new("/home/u/.cache"), true));
        assert!(r.excludes(Path::new("/home/u/disk.iso"), false));
        assert!(r.excludes(Path::new("/home/u/games"), true));
        assert!(r.excludes(Path::new("/home/u/games/doom/x.wad"), false));
        assert!(r.excludes(Path::new("/mnt/x/a"), false));
        assert!(!r.excludes(Path::new("/home/u/gamesx"), true));
        assert!(!r.excludes(Path::new("/home/u/a.txt"), false));
        assert!(!rules(&[], true).excludes(Path::new("/home/u/.cache"), true));
    }
}
