//! `apps` module: indexes installed applications and launches them.
//!
//! Linux scans freedesktop `.desktop` files, macOS the `.app` bundles of its application
//! folders; other platforms are stubs until their own plans land.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tomakast_core::{Effect, Icon, Item, ItemId, Module, ModuleContext, Toast, ToastKind};
use tomakast_platform::file_icons::FileIcons;

pub mod bundles;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod process;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod watch;

/// The backend of this OS.
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;

/// Inputs of a scan. Tests build this explicitly; the daemon uses `from_env`.
#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Application directories in priority order (first wins on duplicates).
    pub dirs: Vec<PathBuf>,
    /// Locale candidates, most specific first: `["ru_RU", "ru"]`.
    pub locales: Vec<String>,
    /// Parsed `$XDG_CURRENT_DESKTOP`.
    pub current_desktops: Vec<String>,
    /// `$PATH` used to resolve `TryExec`.
    pub path_env: Option<OsString>,
    /// Icon theme for `Icon=` names, e.g. `breeze-dark`; lookups fall back to `hicolor`.
    pub icon_theme: String,
}

impl ScanOptions {
    pub fn from_env() -> ScanOptions {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            backend::options_from_env()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            ScanOptions {
                dirs: Vec::new(),
                locales: Vec::new(),
                current_desktops: Vec::new(),
                path_env: None,
                icon_theme: "hicolor".to_string(),
            }
        }
    }
}

/// What is needed to launch an application found by `scan`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub desktop_file: PathBuf,
    pub exec: Vec<String>,
    pub terminal: bool,
    pub working_dir: Option<PathBuf>,
    /// The MIME types the application declares (`MimeType=`).
    pub mime_types: Vec<String>,
}

pub fn scan(opts: &ScanOptions) -> Vec<(Item, AppEntry)> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        backend::scan(opts)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = opts;
        Vec::new()
    }
}

pub fn launch(entry: &AppEntry) -> anyhow::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        backend::launch(entry)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = entry;
        anyhow::bail!("launching applications is not supported on this platform yet")
    }
}

#[cfg(target_os = "linux")]
pub use linux::{
    DbusFileManager, FileManager, FileManagerError, desktop_file_uri, reveal_command, reveal_with,
};

/// Opens `file` with the application, as `gio launch <desktop file> <file>` does. The
/// process is detached; nothing is run through a shell.
pub fn launch_with_file(entry: &AppEntry, file: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        backend::launch_with_file(entry, file)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (entry, file);
        anyhow::bail!("opening a file with an application is not supported on this platform yet")
    }
}

/// Copies the `.desktop` file path and confirms with a toast.
pub fn copy_path_effects(entry: &AppEntry) -> Vec<Effect> {
    vec![
        Effect::CopyText(entry.desktop_file.display().to_string()),
        Effect::Toast(Toast {
            kind: ToastKind::Success,
            title: "Path copied".to_string(),
            message: None,
        }),
    ]
}

fn reveal(entry: &AppEntry) -> anyhow::Result<Vec<Effect>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        backend::reveal(entry)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = entry;
        anyhow::bail!("showing a file in the file manager is not supported on this platform yet")
    }
}

type Entries = Arc<Mutex<HashMap<ItemId, AppEntry>>>;

pub struct AppsModule {
    entries: Entries,
    icon_dir: Option<PathBuf>,
}

impl AppsModule {
    pub fn new() -> AppsModule {
        AppsModule {
            entries: Entries::default(),
            icon_dir: None,
        }
    }

    /// The module with the apps' own icons rendered into `dir` (macOS: an app's icon is in its
    /// bundle, in no icon theme).
    pub fn with_icon_dir(dir: PathBuf) -> AppsModule {
        AppsModule {
            icon_dir: Some(dir),
            ..AppsModule::new()
        }
    }
}

impl Default for AppsModule {
    fn default() -> Self {
        AppsModule::new()
    }
}

fn publish(ctx: &ModuleContext, entries: &Entries, found: Vec<(Item, AppEntry)>) {
    let map: HashMap<ItemId, AppEntry> = found
        .iter()
        .map(|(item, entry)| (item.id.clone(), entry.clone()))
        .collect();
    *entries.lock().unwrap_or_else(|e| e.into_inner()) = map;
    ctx.replace_items(found.into_iter().map(|(item, _)| item).collect());
}

/// `found` with each app's icon as the system draws it, where it has none of its own.
fn with_icons(mut found: Vec<(Item, AppEntry)>, icons: &FileIcons) -> Vec<(Item, AppEntry)> {
    let home = std::env::home_dir().unwrap_or_default();
    for (item, entry) in &mut found {
        if item.icon.is_none() {
            item.icon = icons.icon(&entry.desktop_file, true, &home).map(Icon::File);
        }
    }
    found
}

impl Module for AppsModule {
    fn id(&self) -> &str {
        "apps"
    }

    /// Runs on the module worker; the scan and the directory watch get their
    /// own thread, so the worker stays free to launch apps.
    fn start(&mut self, ctx: ModuleContext) {
        let entries = self.entries.clone();
        // Only macOS draws an app's icon from its bundle.
        let icons = self
            .icon_dir
            .clone()
            .filter(|_| cfg!(target_os = "macos"))
            .map(FileIcons::new);
        let spawned = std::thread::Builder::new()
            .name("apps-scan".into())
            .spawn(move || {
                let opts = ScanOptions::from_env();
                let found = scan(&opts);
                match &icons {
                    // The list at once, its icons once they are rendered.
                    Some(icons) => {
                        publish(&ctx, &entries, found.clone());
                        publish(&ctx, &entries, with_icons(found, icons));
                    }
                    None => publish(&ctx, &entries, found),
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                watch::watch(&opts.dirs, watch::DEBOUNCE, || {
                    let found = scan(&opts);
                    let found = match &icons {
                        Some(icons) => with_icons(found, icons),
                        None => found,
                    };
                    publish(&ctx, &entries, found);
                    true
                });
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "failed to start the apps scanner thread");
        }
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        if !matches!(action_id, "open" | "reveal" | "copy-path") {
            anyhow::bail!("apps: unknown action {action_id:?}");
        }
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&item.id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("apps: unknown item {}", item.id.as_str()))?;
        match action_id {
            "reveal" => reveal(&entry),
            "copy-path" => Ok(copy_path_effects(&entry)),
            _ => {
                launch(&entry)?;
                Ok(vec![Effect::CloseWindow])
            }
        }
    }
}
