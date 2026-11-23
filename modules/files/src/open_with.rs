//! "Open With…" (Linux): the applications that declare a file's type, as a list to pick from.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tomakast_core::{Action, ListLayout, ListRow, ListView, SearchMode, ViewId};
use tomakast_core::{Item, ItemId};
use tomakast_module_apps::{AppEntry, ScanOptions};

/// The Open With list's view id.
pub const VIEW_ID: &str = "files/open-with";
/// How long `xdg-mime` may take to name the default application.
const XDG_MIME_PATIENCE: Duration = Duration::from_secs(2);

/// Apps declaring `mime` (or its `generic` parent like `text/plain` for `text/*`), the default one first
/// (`xdg-mime query default <mime>`), then by title.
pub fn candidates(
    apps: &[(Item, AppEntry)],
    mime: &str,
    default: Option<&str>,
) -> Vec<(Item, AppEntry)> {
    candidates_among(apps, &[mime.to_string()], default)
}

/// [`candidates`] for a type and the `parents` the MIME database gives it. Every `text/*`
/// type also accepts `text/plain`, as shared-mime-info makes it a subclass of it.
pub fn candidates_among(
    apps: &[(Item, AppEntry)],
    accepted: &[String],
    default: Option<&str>,
) -> Vec<(Item, AppEntry)> {
    let mut accepted: Vec<&str> = accepted.iter().map(String::as_str).collect();
    if accepted.iter().any(|m| m.starts_with("text/")) && !accepted.contains(&"text/plain") {
        accepted.push("text/plain");
    }
    let mut found: Vec<(Item, AppEntry)> = apps
        .iter()
        .filter(|(_, entry)| {
            entry
                .mime_types
                .iter()
                .any(|declared| accepted.contains(&declared.as_str()))
        })
        .cloned()
        .collect();
    let is_default = |item: &Item| default.is_some_and(|d| desktop_id(&item.id) == d);
    // `false` sorts first: the default, then by title.
    found.sort_by_cached_key(|(item, _)| (!is_default(item), item.title.to_lowercase()));
    found
}

/// The desktop id in an apps item id: `apps/org.kde.okular.desktop` is `org.kde.okular.desktop`.
pub(crate) fn desktop_id(id: &ItemId) -> &str {
    id.as_str().split_once('/').map_or("", |(_, local)| local)
}

/// One row per candidate; its id is the desktop id.
pub fn rows(found: &[(Item, AppEntry)]) -> Vec<ListRow> {
    found
        .iter()
        .map(|(item, _)| ListRow {
            id: desktop_id(&item.id).to_string(),
            section: None,
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            icon: item.icon.clone(),
            accessories: Vec::new(),
            detail_markdown: None,
            actions: vec![Action {
                id: "open".to_string(),
                title: "Open".to_string(),
                shortcut: None,
            }],
        })
        .collect()
}

/// The list for `file`; the core filters it by what is typed.
pub fn view(file: &Path, rows: Vec<ListRow>) -> ListView {
    let name = crate::names::display_name(file.file_name().unwrap_or(file.as_os_str()));
    ListView {
        id: ViewId(VIEW_ID.to_string()),
        title: format!("Open {name} With"),
        placeholder: "Search applications…".to_string(),
        search: SearchMode::CoreFilter,
        query: String::new(),
        revision: 0,
        rows,
        show_detail: false,
        layout: ListLayout::Rows,
        footer_action: None,
        busy: false,
    }
}

/// Where Open With was started: a root row (its action already counted the use) or the
/// Search Files screen (which counts it itself, for the screen's text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Root,
    Screen { query: String },
}

/// The open Open With list: its file, where it came from and the applications it offers.
pub(crate) struct Pending {
    pub file: PathBuf,
    pub origin: Origin,
    pub apps: Vec<(Item, AppEntry)>,
}

/// The system side of Open With; tests give their own.
pub(crate) trait Backend: Send {
    /// The installed applications.
    fn apps(&self) -> Vec<(Item, AppEntry)>;
    /// The desktop id of the default application for `mime`.
    fn default_for(&self, mime: &str) -> Option<String>;
    /// Opens `file` with `entry`.
    fn launch(&self, entry: &AppEntry, file: &Path) -> anyhow::Result<()>;
}

pub(crate) struct SystemBackend;

impl Backend for SystemBackend {
    fn apps(&self) -> Vec<(Item, AppEntry)> {
        tomakast_module_apps::scan(&ScanOptions::from_env())
    }

    fn default_for(&self, mime: &str) -> Option<String> {
        xdg_mime_default(mime)
    }

    fn launch(&self, entry: &AppEntry, file: &Path) -> anyhow::Result<()> {
        tomakast_module_apps::launch_with_file(entry, file)
    }
}

/// `xdg-mime query default <mime>`: a desktop id, or `None` when there is none, the tool is
/// missing or it is too slow.
fn xdg_mime_default(mime: &str) -> Option<String> {
    let mut child = Command::new("xdg-mime")
        .args(["query", "default", mime])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + XDG_MIME_PATIENCE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let mut out = String::new();
    child
        .stdout
        .take()?
        .take(4096)
        .read_to_string(&mut out)
        .ok()?;
    let id = out.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_core::{ItemId, ItemKind};

    fn app(desktop_id: &str, title: &str, mimes: &[&str]) -> (Item, AppEntry) {
        (
            Item {
                id: ItemId::new("apps", desktop_id),
                title: title.to_string(),
                subtitle: None,
                keywords: Vec::new(),
                icon: None,
                kind: ItemKind::Application,
                source: None,
                actions: Vec::new(),
            },
            AppEntry {
                desktop_file: format!("/apps/{desktop_id}").into(),
                exec: vec![title.to_lowercase()],
                terminal: false,
                working_dir: None,
                mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            },
        )
    }

    fn ids(found: &[(Item, AppEntry)]) -> Vec<&str> {
        found.iter().map(|(i, _)| i.id.as_str()).collect()
    }

    fn apps() -> Vec<(Item, AppEntry)> {
        vec![
            app("zathura.desktop", "Zathura", &["application/pdf"]),
            app(
                "okular.desktop",
                "Okular",
                &["application/pdf", "image/png"],
            ),
            app("gwenview.desktop", "Gwenview", &["image/png", "image/jpeg"]),
            app("kate.desktop", "Kate", &["text/plain"]),
            app("browser.desktop", "Browser", &["application/pdf"]),
            app("bare.desktop", "Bare", &[]),
        ]
    }

    #[test]
    fn candidates_default_first() {
        let found = candidates(&apps(), "application/pdf", Some("zathura.desktop"));
        assert_eq!(
            ids(&found),
            [
                "apps/zathura.desktop",
                "apps/browser.desktop",
                "apps/okular.desktop"
            ]
        );
        // Without a default (or one that does not declare the type), by title.
        let found = candidates(&apps(), "application/pdf", Some("gwenview.desktop"));
        assert_eq!(
            ids(&found),
            [
                "apps/browser.desktop",
                "apps/okular.desktop",
                "apps/zathura.desktop"
            ]
        );
        assert_eq!(candidates(&apps(), "application/pdf", None), found);
    }

    #[test]
    fn candidates_empty_for_unknown_type() {
        assert!(candidates(&apps(), "application/x-nothing", None).is_empty());
    }

    #[test]
    fn text_types_accept_apps_declaring_text_plain() {
        let found = candidates(&apps(), "text/markdown", None);
        assert_eq!(ids(&found), ["apps/kate.desktop"]);
        // The type's own declaration is not lost.
        let mut more = apps();
        more.push(app("md.desktop", "Aaa Markdown", &["text/markdown"]));
        assert_eq!(
            ids(&candidates(&more, "text/markdown", None)),
            ["apps/md.desktop", "apps/kate.desktop"]
        );
    }

    #[test]
    fn open_with_view_rows() {
        let found = candidates(&apps(), "image/png", None);
        let rows = rows(&found);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["gwenview.desktop", "okular.desktop"]);
        assert_eq!(rows[0].title, "Gwenview");
        assert_eq!(rows[0].actions.len(), 1);
        assert_eq!(rows[0].actions[0].id, "open");

        let v = view(Path::new("/home/u/a b.png"), rows);
        assert_eq!(v.id.0, VIEW_ID);
        assert_eq!(v.title, "Open a b.png With");
        assert_eq!(v.search, tomakast_core::SearchMode::CoreFilter);
        assert_eq!(v.rows.len(), 2);
    }
}
