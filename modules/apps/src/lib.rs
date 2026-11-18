//! `apps` module: indexes installed applications and launches them.
//!
//! Linux scans freedesktop `.desktop` files; other platforms are stubs until
//! their own plans land.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tomakast_core::{ActionOutcome, Item, ItemId, Module, ModuleContext};

#[cfg(target_os = "linux")]
mod linux;

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
        #[cfg(target_os = "linux")]
        {
            linux::options_from_env()
        }
        #[cfg(not(target_os = "linux"))]
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
}

pub fn scan(opts: &ScanOptions) -> Vec<(Item, AppEntry)> {
    #[cfg(target_os = "linux")]
    {
        linux::scan(opts)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = opts;
        Vec::new()
    }
}

pub fn launch(entry: &AppEntry) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::launch(entry)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = entry;
        anyhow::bail!("launching applications is not supported on this platform yet")
    }
}

type Entries = Arc<Mutex<HashMap<ItemId, AppEntry>>>;

pub struct AppsModule {
    entries: Entries,
}

impl AppsModule {
    pub fn new() -> AppsModule {
        AppsModule {
            entries: Entries::default(),
        }
    }
}

impl Default for AppsModule {
    fn default() -> Self {
        AppsModule::new()
    }
}

fn publish(ctx: &ModuleContext, entries: &Entries, opts: &ScanOptions) {
    let found = scan(opts);
    let map: HashMap<ItemId, AppEntry> = found
        .iter()
        .map(|(item, entry)| (item.id.clone(), entry.clone()))
        .collect();
    *entries.lock().unwrap_or_else(|e| e.into_inner()) = map;
    ctx.replace_items(found.into_iter().map(|(item, _)| item).collect());
}

impl Module for AppsModule {
    fn id(&self) -> &'static str {
        "apps"
    }

    fn start(&mut self, ctx: ModuleContext) {
        let entries = self.entries.clone();
        let spawned = std::thread::Builder::new()
            .name("apps-scan".into())
            .spawn(move || {
                let opts = ScanOptions::from_env();
                publish(&ctx, &entries, &opts);
                #[cfg(target_os = "linux")]
                linux::watch(&opts.dirs, linux::DEBOUNCE, || {
                    publish(&ctx, &entries, &opts);
                    true
                });
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "failed to start the apps scanner thread");
        }
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<ActionOutcome> {
        if action_id != "open" {
            anyhow::bail!("apps: unknown action {action_id:?}");
        }
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&item.id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("apps: unknown item {}", item.id.as_str()))?;
        launch(&entry)?;
        Ok(ActionOutcome::HideWindow)
    }
}
