//! File types from names, and their theme icons: freedesktop shared-mime-info on Linux.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use tomakast_core::Icon;

/// The icon of a file whose type is unknown.
const UNKNOWN_ICON: &str = "text-x-generic";
const FOLDER_ICON: &str = "folder";

/// A MIME type and the weight of the glob that gave it.
#[derive(Clone, Debug)]
struct Rule {
    weight: u32,
    mime: String,
}

/// Whether the desktop's icon theme holds an icon name.
type IconLookup = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Extension and name globs of the system MIME database (`*.ext` and literal names; globs
/// with other wildcards are left out), and the generic icon of each type.
pub struct MimeDb {
    /// `*.ext` globs by lowercase extension (`png`, `tar.gz`).
    suffix: HashMap<String, Rule>,
    /// `*.ext` globs flagged case-sensitive (`cs`), by the extension as written.
    suffix_cs: HashMap<String, Rule>,
    /// Literal names (`makefile`), lowercase.
    literal: HashMap<String, Rule>,
    literal_cs: HashMap<String, Rule>,
    /// `mime/subclasses`: type to the types it is a subclass of.
    parents: HashMap<String, Vec<String>>,
    /// `mime/generic-icons`: type to icon name.
    generic: HashMap<String, String>,
    /// The icon theme, asked once per type (a lookup probes the theme's folders).
    has_icon: IconLookup,
    /// Type to the icon name chosen for it.
    icons: Mutex<HashMap<String, String>>,
}

impl Default for MimeDb {
    /// No globs; icons looked up in the desktop's theme.
    fn default() -> MimeDb {
        MimeDb {
            suffix: HashMap::new(),
            suffix_cs: HashMap::new(),
            literal: HashMap::new(),
            literal_cs: HashMap::new(),
            parents: HashMap::new(),
            generic: HashMap::new(),
            has_icon: Box::new(|name| tomakast_platform::icon_theme::resolve(name).is_some()),
            icons: Mutex::new(HashMap::new()),
        }
    }
}

impl MimeDb {
    /// Linux: `mime/globs2` and `mime/generic-icons` of the XDG data dirs; elsewhere empty.
    pub fn system() -> MimeDb {
        if !cfg!(target_os = "linux") {
            return MimeDb::default();
        }
        let env_dir = |key: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let data_home = env_dir("XDG_DATA_HOME")
            .or_else(|| std::env::home_dir().map(|h| h.join(".local/share")));
        let data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let dirs: Vec<PathBuf> = data_home
            .into_iter()
            .chain(std::env::split_paths(&data_dirs).filter(|d| d.is_absolute()))
            .map(|d| d.join("mime"))
            .collect();
        MimeDb::from_mime_dirs(&dirs)
    }

    /// The database read from these `mime` folders; at an equal weight the first folder wins.
    /// Missing or unreadable files are skipped.
    pub fn from_mime_dirs(dirs: &[PathBuf]) -> MimeDb {
        let mut db = MimeDb::default();
        for dir in dirs {
            if let Ok(bytes) = std::fs::read(dir.join("globs2")) {
                db.add_globs(&String::from_utf8_lossy(&bytes));
            }
            if let Ok(bytes) = std::fs::read(dir.join("subclasses")) {
                for line in String::from_utf8_lossy(&bytes).lines() {
                    let mut types = line.split_whitespace();
                    if let (Some(child), Some(parent), None) =
                        (types.next(), types.next(), types.next())
                        && !child.starts_with('#')
                    {
                        let parents = db.parents.entry(child.to_string()).or_default();
                        if !parents.iter().any(|p| p == parent) {
                            parents.push(parent.to_string());
                        }
                    }
                }
            }
            if let Ok(bytes) = std::fs::read(dir.join("generic-icons")) {
                for line in String::from_utf8_lossy(&bytes).lines() {
                    if let Some((mime, icon)) = line.trim().split_once(':')
                        && !mime.is_empty()
                        && !icon.is_empty()
                    {
                        db.generic
                            .entry(mime.to_string())
                            .or_insert_with(|| icon.to_string());
                    }
                }
            }
        }
        db
    }

    /// Lines `weight:mime:glob[:flags]`; anything else is skipped.
    fn add_globs(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let mut fields = line.split(':');
            let (Some(weight), Some(mime), Some(glob)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse::<u32>() else {
                continue;
            };
            if mime.is_empty() || glob.is_empty() {
                continue;
            }
            let cs = fields
                .next()
                .is_some_and(|f| f.split(',').any(|f| f == "cs"));
            let wild = |s: &str| s.contains(['*', '?', '[']);
            let (map, key) = match glob.strip_prefix("*.") {
                Some(ext) if !ext.is_empty() && !wild(ext) => {
                    if cs {
                        (&mut self.suffix_cs, ext.to_string())
                    } else {
                        (&mut self.suffix, ext.to_lowercase())
                    }
                }
                Some(_) => continue,
                None if wild(glob) => continue,
                None if cs => (&mut self.literal_cs, glob.to_string()),
                None => (&mut self.literal, glob.to_lowercase()),
            };
            let rule = Rule {
                weight,
                mime: mime.to_string(),
            };
            match map.get(&key) {
                Some(old) if old.weight >= weight => {}
                _ => {
                    map.insert(key, rule);
                }
            }
        }
    }

    /// The same database, asking `has_icon` whether the icon theme holds a name.
    pub fn with_icon_lookup(
        self,
        has_icon: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> MimeDb {
        MimeDb {
            has_icon: Box::new(has_icon),
            icons: Mutex::new(HashMap::new()),
            ..self
        }
    }

    /// The MIME type of a file called `name`, by its globs: a literal name first, then the
    /// longest matching extension, the heavier glob on a tie.
    pub fn mime_of(&self, name: &OsStr) -> Option<&str> {
        let name = name.to_string_lossy();
        let lower = name.to_lowercase();
        let literal = heavier(self.literal_cs.get(name.as_ref()), self.literal.get(&lower));
        if let Some(rule) = literal {
            return Some(&rule.mime);
        }
        let cs = longest_suffix(&self.suffix_cs, &name);
        let ci = longest_suffix(&self.suffix, &lower);
        let rule = match (cs, ci) {
            (Some((a_len, a)), Some((b_len, b))) => {
                if a_len != b_len {
                    if a_len > b_len { a } else { b }
                } else if a.weight >= b.weight {
                    a
                } else {
                    b
                }
            }
            (Some((_, a)), None) => a,
            (None, Some((_, b))) => b,
            (None, None) => return None,
        };
        Some(&rule.mime)
    }

    /// The types `mime` is a subclass of, nearest first, up the `subclasses` chains (no type
    /// twice; `application/octet-stream` left out, as every file is one).
    pub fn parents_of(&self, mime: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let mut queue = vec![mime.to_string()];
        while !queue.is_empty() {
            let next: Vec<String> = queue
                .drain(..)
                .flat_map(|m| self.parents.get(&m).cloned().unwrap_or_default())
                .filter(|p| p != "application/octet-stream" && p != mime)
                .collect();
            for parent in next {
                if !found.contains(&parent) {
                    found.push(parent.clone());
                    queue.push(parent);
                }
            }
        }
        found
    }

    /// The theme icon of a file of type `mime`, or of a folder, as the freedesktop icon
    /// spec orders them: the type's own icon (`image-png`) if the theme has it, else its
    /// generic icon (the `generic-icons` entry, or `<media>-x-generic`) if the theme has
    /// that, else `text-x-generic`. Folders are `folder`. The theme is asked once per type.
    pub fn icon_of(&self, mime: Option<&str>, is_dir: bool) -> Icon {
        let name = match mime {
            _ if is_dir => FOLDER_ICON.to_string(),
            None => UNKNOWN_ICON.to_string(),
            Some(mime) => {
                let mut icons = self.icons.lock().unwrap_or_else(PoisonError::into_inner);
                icons
                    .entry(mime.to_string())
                    .or_insert_with(|| self.type_icon(mime))
                    .clone()
            }
        };
        Icon::Named(name)
    }

    fn type_icon(&self, mime: &str) -> String {
        let specific = mime.replace('/', "-");
        if (self.has_icon)(&specific) {
            return specific;
        }
        let generic = match self.generic.get(mime) {
            Some(icon) => icon.clone(),
            None => {
                let media = mime.split('/').next().unwrap_or(mime);
                format!("{media}-x-generic")
            }
        };
        if (self.has_icon)(&generic) {
            return generic;
        }
        UNKNOWN_ICON.to_string()
    }
}

/// The heavier of two rules, `a` on a tie.
fn heavier<'a>(a: Option<&'a Rule>, b: Option<&'a Rule>) -> Option<&'a Rule> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if b.weight > a.weight { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// The rule of the longest extension of `name` in `map`, with the extension's length.
fn longest_suffix<'a>(map: &'a HashMap<String, Rule>, name: &str) -> Option<(usize, &'a Rule)> {
    if map.is_empty() {
        return None;
    }
    // From the first dot on: the longest extension comes first.
    name.match_indices('.')
        .map(|(i, _)| &name[i + 1..])
        .filter(|ext| !ext.is_empty())
        .find_map(|ext| map.get(ext).map(|rule| (ext.len(), rule)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn named(s: &str) -> Icon {
        Icon::Named(s.to_string())
    }

    #[test]
    fn parents_follow_the_subclasses_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mime");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("subclasses"),
            "# a comment\n\
             application/x-sh application/x-shellscript\n\
             application/x-shellscript text/plain\n\
             text/plain application/octet-stream\n\
             application/x-loop application/x-loop2\n\
             application/x-loop2 application/x-loop\n\
             broken-line\n",
        )
        .unwrap();
        let db = MimeDb::from_mime_dirs(&[dir]);
        assert_eq!(
            db.parents_of("application/x-sh"),
            ["application/x-shellscript", "text/plain"]
        );
        assert!(db.parents_of("image/png").is_empty());
        assert_eq!(db.parents_of("application/x-loop"), ["application/x-loop2"]);
    }

    /// A user `mime` folder and a system one.
    fn db() -> MimeDb {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user/mime");
        let system = tmp.path().join("system/mime");
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(&system).unwrap();
        fs::write(
            user.join("globs2"),
            "# user overrides\n50:text/x-mine:*.txt\n",
        )
        .unwrap();
        fs::write(
            system.join("globs2"),
            "# shared-mime-info\n\
             50:image/png:*.png\n\
             50:text/plain:*.txt\n\
             50:application/gzip:*.gz\n\
             50:application/x-compressed-tar:*.tar.gz\n\
             50:text/x-c++src:*.C:cs\n\
             50:text/x-csrc:*.c\n\
             10:text/x-makefile:makefile\n\
             40:text/x-readme:README*\n\
             garbage line\n\
             :::\n",
        )
        .unwrap();
        fs::write(
            system.join("generic-icons"),
            "application/x-compressed-tar:package-x-generic\n",
        )
        .unwrap();
        MimeDb::from_mime_dirs(&[user, system, tmp.path().join("missing/mime")])
    }

    /// `db` with an icon theme that holds exactly `icons`.
    fn db_with_theme(icons: &'static [&'static str]) -> MimeDb {
        db().with_icon_lookup(move |name| icons.contains(&name))
    }

    fn icon(db: &MimeDb, name: &str) -> Icon {
        db.icon_of(db.mime_of(OsStr::new(name)), false)
    }

    #[test]
    fn mime_icons() {
        let db = db_with_theme(&["image-png", "package-x-generic", "text-x-generic"]);
        assert_eq!(icon(&db, "a.png"), named("image-png"));
        assert_eq!(icon(&db, "A.PNG"), named("image-png"));
        assert_eq!(db.icon_of(None, true), named("folder"));
        assert_eq!(icon(&db, "x.unknownext"), named("text-x-generic"));
        assert_eq!(icon(&db, "noext"), named("text-x-generic"));
        // The theme has no specific icon: the generic one the database names.
        assert_eq!(icon(&db, "a.tar.gz"), named("package-x-generic"));
    }

    #[test]
    fn listed_type_keeps_its_specific_icon_when_the_theme_has_it() {
        let db = db_with_theme(&["application-x-compressed-tar", "package-x-generic"]);
        assert_eq!(icon(&db, "a.tar.gz"), named("application-x-compressed-tar"));
    }

    #[test]
    fn falls_back_to_media_generic_then_text_generic() {
        // No specific icon and no generic-icons entry: the media type's generic icon.
        let db = db_with_theme(&["image-x-generic"]);
        assert_eq!(icon(&db, "a.png"), named("image-x-generic"));
        // Neither: the plain file icon.
        let db = db_with_theme(&[]);
        assert_eq!(icon(&db, "a.png"), named("text-x-generic"));
        assert_eq!(icon(&db, "a.tar.gz"), named("text-x-generic"));
        assert_eq!(db.icon_of(None, true), named("folder"));
    }

    #[test]
    fn globs_by_suffix_case_and_order() {
        let db = db();
        let mime = |name: &str| db.mime_of(OsStr::new(name));
        // The longest matching suffix wins.
        assert_eq!(mime("a.tar.gz"), Some("application/x-compressed-tar"));
        assert_eq!(mime("a.gz"), Some("application/gzip"));
        // `cs` globs match only their own case; the others any case.
        assert_eq!(mime("a.C"), Some("text/x-c++src"));
        assert_eq!(mime("a.c"), Some("text/x-csrc"));
        // A literal name, any case.
        assert_eq!(mime("Makefile"), Some("text/x-makefile"));
        // An earlier folder wins over a later one at the same weight.
        assert_eq!(mime("notes.txt"), Some("text/x-mine"));
        // Globs with wildcards in the middle are not supported.
        assert_eq!(mime("README.md"), None);
    }
}
