//! File search: the walked index of the covered folders, its root answers and the actions on
//! a file or folder.
//!
//! The module answers root searches of at least three characters with up to three files and
//! folders (the engine ranks them with usage, like every item), and publishes the "Search
//! Files" command. The index lives on its own thread (`live`); a search reads it under a
//! short lock and never waits for a walk.

pub mod index;
pub mod live;
pub mod mime;
pub mod names;
#[cfg(target_os = "linux")]
pub mod open_with;
pub mod recent;
pub mod rules;
pub mod screen;
pub mod search;
pub mod state;
pub mod thumbs;

use std::collections::{HashMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tomakast_core::{
    Action, Effect, Icon, Item, ItemId, ItemKind, ListRow, Module, ModuleContext, QueryReading,
    SearchAnswer, Toast, ToastKind, View, ViewEvent, ViewId,
};

use crate::live::{Live, Options};
use crate::mime::MimeDb;
pub use crate::names::PROVIDER;
use crate::names::{display_name, display_parent, display_path, item_id};
use crate::recent::Recent;
use crate::screen::{Shown, modified_label};
use crate::search::{Signals, search};
use crate::state::{Layout, State};

/// Root answers only for queries of at least this many characters.
const ROOT_MIN_CHARS: usize = 3;
/// At most this many files and folders in the root list.
const ROOT_LIMIT: usize = 3;
/// The last this many items handed out keep their path, for their actions.
const REMEMBERED: usize = 2000;
/// The local part of the "Search Files" command's id.
const COMMAND: &str = "search";
/// The local part of the root list's "Search Files for …" fallback: shown above the files
/// when nothing else matches, it opens the screen with the query typed.
const MORE: &str = "more";
/// The saved index, in the data folder: read back at start, so files are found at once.
const INDEX_FILE: &str = "index.bin";
/// The wake-up token of a screen refresh (the first walk ended, thumbnails were made).
const REFRESH: u64 = 1;

/// Asks the module's worker for a screen refresh: one wake-up until it is served, however
/// many ask meanwhile.
struct Refresh {
    wake: Box<dyn Fn() + Send + Sync>,
    asked: AtomicBool,
}

impl Refresh {
    fn new(wake: Box<dyn Fn() + Send + Sync>) -> Refresh {
        Refresh {
            wake,
            asked: AtomicBool::new(false),
        }
    }

    /// From any thread.
    fn ask(&self) {
        if !self.asked.swap(true, Ordering::SeqCst) {
            (self.wake)();
        }
    }

    /// On the worker, before the refresh is built: what happens from now on asks again.
    fn served(&self) {
        self.asked.store(false, Ordering::SeqCst);
    }
}

/// Where the module reads and keeps its files.
pub struct FilesConfig {
    pub config_file: PathBuf,
    /// `state.json` lives here.
    pub data_dir: PathBuf,
    /// `~` of roots and subtitles.
    pub home: PathBuf,
    /// `$XDG_CACHE_HOME/thumbnails`.
    pub thumbnail_root: PathBuf,
    /// KDE's `recently-used.xbel` (Linux).
    pub recent_file: Option<PathBuf>,
}

impl FilesConfig {
    /// The user's home folder, `$XDG_CACHE_HOME` (`~/.cache`) and, on Linux, `$XDG_DATA_HOME`
    /// (`~/.local/share`); relative XDG paths are ignored, as the XDG spec asks. With no
    /// absolute home folder `home` is empty, and the module starts nothing (see
    /// [`FilesModule`]).
    pub fn from_env(config_file: PathBuf, data_dir: PathBuf) -> FilesConfig {
        let home = std::env::home_dir()
            .filter(|h| h.is_absolute())
            .unwrap_or_default();
        let env_dir = |key: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let thumbnail_root = env_dir("XDG_CACHE_HOME")
            .unwrap_or_else(|| home.join(".cache"))
            .join("thumbnails");
        let recent_file = cfg!(target_os = "linux").then(|| {
            env_dir("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local/share"))
                .join("recently-used.xbel")
        });
        FilesConfig {
            config_file,
            data_dir,
            home,
            thumbnail_root,
            recent_file,
        }
    }
}

/// Opens and reveals paths; tests record the calls instead.
pub(crate) trait Opener: Send {
    /// Opens `path` with its default application (a folder in the file manager).
    fn open(&self, path: &Path) -> anyhow::Result<()>;
    /// Shows `path` selected in the file manager.
    fn reveal(&self, path: &Path) -> anyhow::Result<()>;
}

/// The desktop's own opener and file manager.
struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, path: &Path) -> anyhow::Result<()> {
        tomakast_platform::open::open_path(path)
    }

    fn reveal(&self, path: &Path) -> anyhow::Result<()> {
        reveal::reveal(path)
    }
}

/// Showing a file selected in the file manager, per OS.
mod reveal {
    use std::path::Path;

    /// The `org.freedesktop.FileManager1` service (Dolphin, Nautilus…), else the parent
    /// folder opened.
    #[cfg(target_os = "linux")]
    pub(crate) fn reveal(path: &Path) -> anyhow::Result<()> {
        reveal_with(path, &tomakast_module_apps::DbusFileManager, &mut |dir| {
            tomakast_platform::open::open_path(dir)
        })
    }

    /// `reveal` with the service and the opener of the fallback given.
    #[cfg(target_os = "linux")]
    pub(crate) fn reveal_with(
        path: &Path,
        file_manager: &dyn tomakast_module_apps::FileManager,
        open: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        match file_manager.show_items(&[crate::names::file_uri(path)]) {
            Ok(()) => Ok(()),
            Err(err) => {
                tracing::debug!(%err, "files: FileManager1.ShowItems failed, opening the folder");
                open(path.parent().unwrap_or(Path::new("/")))
            }
        }
    }

    /// `open -R <path>`.
    #[cfg(target_os = "macos")]
    pub(crate) fn reveal(path: &Path) -> anyhow::Result<()> {
        let mut cmd = std::process::Command::new("open");
        cmd.arg("-R").arg(path);
        spawn(cmd, "open")
    }

    /// `explorer /select,"<path>"`: explorer reads the quotes itself, so the argument is
    /// passed as written (Windows paths cannot hold a quote).
    #[cfg(windows)]
    pub(crate) fn reveal(path: &Path) -> anyhow::Result<()> {
        use std::os::windows::process::CommandExt;
        let mut arg = std::ffi::OsString::from("/select,\"");
        arg.push(path.as_os_str());
        arg.push("\"");
        let mut cmd = std::process::Command::new("explorer");
        cmd.raw_arg(arg);
        spawn(cmd, "explorer")
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    pub(crate) fn reveal(path: &Path) -> anyhow::Result<()> {
        let _ = path;
        anyhow::bail!("showing a file in the file manager is not supported on this system")
    }

    /// Runs `cmd` with no stdio, not through a shell, and reaps it on a throwaway thread.
    #[cfg(any(target_os = "macos", windows))]
    fn spawn(mut cmd: std::process::Command, program: &str) -> anyhow::Result<()> {
        use anyhow::Context;
        use std::process::Stdio;
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to run {program}"))?;
        let reaper = std::thread::Builder::new()
            .name("files-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
        if let Err(err) = reaper {
            tracing::warn!(%err, "files: could not start the reaper thread for {program}");
        }
        Ok(())
    }
}

/// The paths of the last `REMEMBERED` items handed out, by id: an action names only the
/// item, and an id cannot be turned back into its path.
#[derive(Default)]
struct Handed {
    /// Id to its path and the stamp of its latest hand-out.
    paths: HashMap<ItemId, (PathBuf, u64)>,
    /// Hand-outs, oldest first; an entry whose stamp is not the id's latest is stale.
    order: VecDeque<(ItemId, u64)>,
    stamp: u64,
}

impl Handed {
    fn remember(&mut self, id: ItemId, path: PathBuf) {
        self.stamp += 1;
        self.paths.insert(id.clone(), (path, self.stamp));
        self.order.push_back((id, self.stamp));
        while self.paths.len() > REMEMBERED {
            let Some((old, stamp)) = self.order.pop_front() else {
                break;
            };
            if self.paths.get(&old).is_some_and(|(_, s)| *s == stamp) {
                self.paths.remove(&old);
            }
        }
        // Ids handed out again and again leave stale entries behind.
        if self.order.len() > 2 * REMEMBERED {
            let paths = &self.paths;
            self.order
                .retain(|(id, stamp)| paths.get(id).is_some_and(|(_, s)| s == stamp));
        }
    }

    fn get(&self, id: &ItemId) -> Option<&Path> {
        self.paths.get(id).map(|(path, _)| path.as_path())
    }
}

/// The `files` module. Without an absolute home folder it indexes nothing, reads and writes
/// no file, answers no root search, and its command says why.
pub struct FilesModule {
    config: FilesConfig,
    opener: Box<dyn Opener>,
    /// Read on the worker when the module starts (`None` until then, unless a test gave one).
    mime: Option<MimeDb>,
    live: Option<Live>,
    /// `[files] in_root`, as the index thread last read it.
    in_root: Arc<AtomicBool>,
    handed: Handed,
    /// The screen's layout and the paths opened through the launcher, kept in
    /// `<data_dir>/state.json`.
    state: State,
    recent: Recent,
    /// The "Search Files" screen's field while it is open.
    screen: Option<Screen>,
    /// The query of the latest root answer: the fallback opens the screen with it.
    root_query: String,
    /// A root answer said busy (no index yet): once the index is in, the root list is
    /// asked again.
    answered_busy: bool,
    /// The thumbnail cache, once started.
    thumbs: Option<thumbs::Thumbs>,
    refresh: Arc<Refresh>,
    /// Unix seconds now (a test fixes it).
    clock: Box<dyn Fn() -> i64 + Send>,
    /// The applications, default and launcher behind Open With….
    #[cfg(target_os = "linux")]
    open_with_backend: Box<dyn open_with::Backend>,
    /// The open "Open With" list: its file, where it came from and its applications.
    #[cfg(target_os = "linux")]
    open_with: Option<open_with::Pending>,
}

/// What the open "Search Files" screen holds: the typed text and its revision.
#[derive(Default)]
struct Screen {
    query: String,
    revision: u64,
}

impl FilesModule {
    pub fn new(config: FilesConfig) -> FilesModule {
        FilesModule::with_parts(config, Box::new(SystemOpener), None)
    }

    /// `new` with the opener and the MIME database given.
    pub(crate) fn with_parts(
        config: FilesConfig,
        opener: Box<dyn Opener>,
        mime: Option<MimeDb>,
    ) -> FilesModule {
        let recent = Recent::new(config.recent_file.clone());
        FilesModule {
            config,
            opener,
            mime,
            live: None,
            in_root: Arc::new(AtomicBool::new(true)),
            handed: Handed::default(),
            state: State::default(),
            recent,
            screen: None,
            root_query: String::new(),
            answered_busy: false,
            thumbs: None,
            refresh: Arc::new(Refresh::new(Box::new(|| {}))),
            clock: Box::new(unix_now),
            #[cfg(target_os = "linux")]
            open_with_backend: Box::new(open_with::SystemBackend),
            #[cfg(target_os = "linux")]
            open_with: None,
        }
    }

    /// `Module::start` with the index thread's options given: publishes the command, wires
    /// the refresh to the worker and begins.
    pub(crate) fn start_with(&mut self, ctx: ModuleContext, opts: Options) {
        ctx.replace_items(vec![command_item()]);
        self.refresh = Arc::new(Refresh::new(Box::new(move || ctx.wake(REFRESH))));
        self.begin(opts);
    }

    /// Reads the MIME database and the recent list, and starts the index thread.
    pub(crate) fn begin(&mut self, opts: Options) {
        if !self.has_home() {
            tracing::warn!(
                home = %self.config.home.display(),
                "files: no absolute home folder; file search is off"
            );
            return;
        }
        if self.mime.is_none() {
            self.mime = Some(MimeDb::system());
        }
        self.state = State::load(&self.config.data_dir);
        self.recent.refresh();
        self.thumbs = Some(thumbs::Thumbs::new(self.config.thumbnail_root.clone()));
        let in_root = self.in_root.clone();
        let refresh = self.refresh.clone();
        // A screen opened before the first walk ended is empty until then.
        let opts = Options {
            on_first_walk: Some(Box::new(move || refresh.ask())),
            snapshot: Some(self.config.data_dir.join(INDEX_FILE)),
            ..opts
        };
        self.live = Some(Live::start_with(
            self.config.config_file.clone(),
            self.config.home.clone(),
            opts,
            move |files| in_root.store(files.in_root, Ordering::Relaxed),
        ));
    }

    /// `~` is known: every folder the module uses is absolute.
    fn has_home(&self) -> bool {
        self.config.home.is_absolute()
    }

    /// The root list answers `typed` with files: three characters or more, a letter among
    /// them, and files in the root list on.
    fn answers_in_root(&self, typed: &str) -> bool {
        self.in_root.load(Ordering::Relaxed)
            && typed.chars().count() >= ROOT_MIN_CHARS
            && typed.chars().any(char::is_alphabetic)
    }

    /// The root list's files and folders for `query`: none below three characters, and none
    /// without a letter (`10/2`, `2+2` are for calc; the Search Files screen still finds
    /// such names).
    fn root_items(&mut self, query: &str) -> Vec<Item> {
        let typed = query.trim();
        if !self.answers_in_root(typed) {
            return Vec::new();
        }
        let Some(live) = &self.live else {
            return Vec::new();
        };
        self.recent.refresh();
        let signals = Signals {
            opened: &self.state.opened,
            recent: self.recent.paths(),
            home: &self.config.home,
            now: (self.clock)(),
        };
        // Only the paths under the lock; the items are built after it.
        let hits: Vec<(PathBuf, OsString, bool)> = live.read(|index| {
            search(index, query, ROOT_LIMIT, &signals)
                .into_iter()
                .map(|found| {
                    let entry = index.entry(found.id);
                    (
                        index.path(found.id),
                        entry.name.to_os_string(),
                        entry.is_dir,
                    )
                })
                .collect()
        });
        if hits.is_empty() {
            return Vec::new();
        }
        self.root_query = typed.to_string();
        let mut items = vec![more_item(typed)];
        items.extend(
            hits.into_iter()
                .map(|(path, name, is_dir)| self.path_item(path, &name, is_dir)),
        );
        items
    }

    /// The item of a file or folder; its path is remembered for its actions.
    fn path_item(&mut self, path: PathBuf, name: &OsStr, is_dir: bool) -> Item {
        let icon = self.icon_of(name, is_dir);
        let home = &self.config.home;
        let item = Item {
            id: item_id(&path),
            title: display_name(name),
            subtitle: Some(display_parent(&path, home)),
            // The engine matches the full path too (`proj/read`).
            keywords: vec![display_path(&path, home)],
            icon: Some(icon),
            kind: if is_dir {
                ItemKind::Folder
            } else {
                ItemKind::File
            },
            actions: path_actions(),
            source: None,
        };
        self.handed.remember(item.id.clone(), path);
        item
    }

    /// The path of an item handed out, or of a file opened before.
    fn path_of(&self, id: &ItemId) -> Option<PathBuf> {
        if let Some(path) = self.handed.get(id) {
            return Some(path.to_path_buf());
        }
        self.state
            .opened
            .keys()
            .find(|p| item_id(p) == *id)
            .cloned()
    }

    /// The type icon of a file or folder named `name`.
    fn icon_of(&mut self, name: &OsStr, is_dir: bool) -> Icon {
        let mime = self.mime.get_or_insert_with(MimeDb::system);
        mime.icon_of((!is_dir).then(|| mime.mime_of(name)).flatten(), is_dir)
    }

    /// Counts an open of `path` and keeps it in the state file.
    fn record_open(&mut self, path: &Path) {
        self.state.record_open(path, (self.clock)());
        self.save_state();
    }

    /// Writes the state file; a failure is logged, never shown: the open or the toggle it
    /// follows has happened.
    fn save_state(&self) {
        if !self.has_home() {
            return;
        }
        if let Err(err) = self.state.save(&self.config.data_dir) {
            tracing::warn!(
                %err,
                dir = %self.config.data_dir.display(),
                "files: cannot write the state file"
            );
        }
    }

    /// The screen for its current field, in the saved layout.
    fn screen_view(&mut self) -> View {
        let (query, revision) = match &self.screen {
            Some(screen) => (screen.query.clone(), screen.revision),
            None => (String::new(), 0),
        };
        let rows = self.screen_rows(&query);
        let busy = self.live.as_ref().is_some_and(|live| !live.is_ready());
        View::List(screen::view(
            &query,
            revision,
            rows,
            self.state.layout,
            busy,
        ))
    }

    /// The screen's rows for `query`: up to [`screen::LIMIT`], the recent files for a blank
    /// one. Their paths are remembered for their actions. In the grid, a file shows its
    /// cached thumbnail, and one is asked for when an image has none.
    fn screen_rows(&mut self, query: &str) -> Vec<ListRow> {
        // The rows asked for before are gone (or asked for again below).
        if let Some(thumbs) = &self.thumbs {
            thumbs.cancel_pending();
        }
        let Some(live) = &self.live else {
            return Vec::new();
        };
        self.recent.refresh();
        let now = (self.clock)();
        let signals = Signals {
            opened: &self.state.opened,
            recent: self.recent.paths(),
            home: &self.config.home,
            now,
        };
        // Only the paths under the lock; the rows are built after it.
        let hits: Vec<(PathBuf, OsString, bool, i64)> = live.read(|index| {
            search(index, query, screen::LIMIT, &signals)
                .into_iter()
                .map(|found| {
                    let entry = index.entry(found.id);
                    (
                        index.path(found.id),
                        entry.name.to_os_string(),
                        entry.is_dir,
                        entry.mtime,
                    )
                })
                .collect()
        });
        let layout = self.state.layout;
        hits.into_iter()
            .map(|(path, name, is_dir, mtime)| {
                let id = item_id(&path);
                let thumb = (layout == Layout::Grid && !is_dir)
                    .then(|| self.thumbnail(&path, mtime))
                    .flatten();
                let shown = Shown {
                    id: local_part(&id).to_string(),
                    title: display_name(&name),
                    subtitle: display_parent(&path, &self.config.home),
                    icon: match thumb {
                        Some(thumb) => Icon::File(thumb),
                        None => self.icon_of(&name, is_dir),
                    },
                    modified: modified_label(mtime, now, &chrono::Local),
                };
                self.handed.remember(id, path);
                screen::row(shown, layout)
            })
            .collect()
    }

    /// The cached thumbnail of the file at `path`; without one, it is asked for (an image),
    /// and the screen is refreshed once it is made.
    fn thumbnail(&self, path: &Path, mtime: i64) -> Option<PathBuf> {
        let thumbs = self.thumbs.as_ref()?;
        let cached = thumbs.cached(path, mtime);
        if cached.is_none() {
            let refresh = self.refresh.clone();
            thumbs.request(path, mtime, move || refresh.ask());
        }
        cached
    }

    /// Runs `action_id` on the file or folder `id`, from the root list or the screen.
    /// `screen_query` is the Search Files screen's field when the action comes from it.
    fn path_action(
        &mut self,
        id: &ItemId,
        action_id: &str,
        screen_query: Option<&str>,
    ) -> anyhow::Result<Vec<Effect>> {
        let _ = screen_query;
        match action_id {
            "open" | "reveal" | "copy-path" => {}
            #[cfg(target_os = "linux")]
            "open-with" => {}
            _ => anyhow::bail!("files: unknown action {action_id:?}"),
        }
        let path = self
            .path_of(id)
            .ok_or_else(|| anyhow::anyhow!("files: unknown item {}", id.as_str()))?;
        // Not `exists`: that follows symbolic links, and the index holds the links themselves.
        if std::fs::symlink_metadata(&path).is_err() {
            if let Some(live) = &self.live {
                live.recheck(&path);
            }
            anyhow::bail!("File not found: {}", display_path(&path, &self.config.home));
        }
        match action_id {
            "open" => {
                self.opener.open(&path)?;
                self.record_open(&path);
                Ok(vec![Effect::CloseWindow])
            }
            "reveal" => {
                self.opener.reveal(&path)?;
                Ok(vec![Effect::CloseWindow])
            }
            #[cfg(target_os = "linux")]
            "open-with" => Ok(self.open_with_list(path, screen_query)),
            _ => Ok(vec![
                Effect::CopyText(path.display().to_string()),
                Effect::Toast(Toast {
                    kind: ToastKind::Success,
                    title: "Path copied".to_string(),
                    message: None,
                }),
            ]),
        }
    }
}

#[cfg(target_os = "linux")]
impl FilesModule {
    /// The Open With list for `file`: the applications declaring its type (and the types
    /// it is a subclass of), the default one first; a toast when there is none.
    fn open_with_list(&mut self, file: PathBuf, screen_query: Option<&str>) -> Vec<Effect> {
        let no_app = || {
            vec![Effect::Toast(Toast {
                kind: ToastKind::Error,
                title: "No application opens this type".to_string(),
                message: None,
            })]
        };
        let mime = self.mime.get_or_insert_with(MimeDb::system);
        let kind = if std::fs::metadata(&file).is_ok_and(|m| m.is_dir()) {
            Some("inode/directory")
        } else {
            file.file_name().and_then(|name| mime.mime_of(name))
        };
        let Some(kind) = kind.map(str::to_string) else {
            return no_app();
        };
        let mut accepted = vec![kind.clone()];
        accepted.extend(mime.parents_of(&kind));
        let apps = self.open_with_backend.apps();
        let mut found = open_with::candidates_among(&apps, &accepted, None);
        if found.is_empty() {
            return no_app();
        }
        // The default is asked for only when there is a choice to order.
        if found.len() > 1 {
            let default = self.open_with_backend.default_for(&kind);
            found = open_with::candidates_among(&apps, &accepted, default.as_deref());
        }
        let view = open_with::view(&file, open_with::rows(&found));
        self.open_with = Some(open_with::Pending {
            file,
            origin: match screen_query {
                Some(query) => open_with::Origin::Screen {
                    query: query.to_string(),
                },
                None => open_with::Origin::Root,
            },
            apps: found,
        });
        vec![Effect::PushView(View::List(view))]
    }

    /// Opens the Open With list's file with the application of row `row_id`, and counts the
    /// open. A root row's action already counted the use for the launcher's ranking; one from
    /// the screen has not.
    fn open_with_choose(&mut self, row_id: &str) -> anyhow::Result<Vec<Effect>> {
        let Some(pending) = &self.open_with else {
            return Ok(Vec::new());
        };
        let entry = pending
            .apps
            .iter()
            .find(|(item, _)| open_with::desktop_id(&item.id) == row_id)
            .map(|(_, entry)| entry.clone())
            .ok_or_else(|| anyhow::anyhow!("files: unknown application {row_id:?}"))?;
        let file = pending.file.clone();
        let usage = match &pending.origin {
            open_with::Origin::Root => None,
            open_with::Origin::Screen { query } => Some(Effect::RecordUsage {
                item: item_id(&file),
                query: query.clone(),
            }),
        };
        if std::fs::symlink_metadata(&file).is_err() {
            if let Some(live) = &self.live {
                live.recheck(&file);
            }
            anyhow::bail!("File not found: {}", display_path(&file, &self.config.home));
        }
        self.open_with_backend.launch(&entry, &file)?;
        self.record_open(&file);
        let mut effects: Vec<Effect> = usage.into_iter().collect();
        effects.push(Effect::CloseWindow);
        Ok(effects)
    }
}

/// The part of an item id after `files/`: a screen row's id.
fn local_part(id: &ItemId) -> &str {
    id.as_str().split_once('/').map_or("", |(_, local)| local)
}

/// The actions of every file and folder row, in this order.
pub(crate) fn path_actions() -> Vec<Action> {
    let action = |id: &str, title: &str, shortcut: Option<&str>| Action {
        id: id.to_string(),
        title: title.to_string(),
        shortcut: shortcut.map(str::to_string),
    };
    let mut actions = vec![
        action("open", "Open", None),
        action("reveal", "Show in File Manager", Some("ctrl+shift+f")),
        action("copy-path", "Copy Path", Some("ctrl+shift+c")),
    ];
    if cfg!(target_os = "linux") {
        actions.push(action("open-with", "Open With…", Some("ctrl+o")));
    }
    actions
}

/// The root list's fallback for `query`: the core keeps it only when every other hit is a
/// file or folder, and then it leads.
fn more_item(query: &str) -> Item {
    Item {
        id: ItemId::new(PROVIDER, MORE),
        title: format!("Search Files for \u{201c}{query}\u{201d}"),
        subtitle: None,
        keywords: Vec::new(),
        icon: Some(Icon::Named("system-search".to_string())),
        kind: ItemKind::Fallback,
        actions: vec![Action {
            id: "open".to_string(),
            title: "Open".to_string(),
            shortcut: None,
        }],
        source: None,
    }
}

/// The "Search Files" command.
fn command_item() -> Item {
    Item {
        id: ItemId::new(PROVIDER, COMMAND),
        title: "Search Files".to_string(),
        subtitle: Some("Search files by name".to_string()),
        keywords: Vec::new(),
        icon: Some(Icon::Named("system-search".to_string())),
        kind: ItemKind::Command,
        actions: vec![Action {
            id: "open".to_string(),
            title: "Open".to_string(),
            shortcut: None,
        }],
        source: None,
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Module for FilesModule {
    fn id(&self) -> &str {
        PROVIDER
    }

    /// Runs on the module worker: the MIME database is read here, the index on its own
    /// thread.
    fn start(&mut self, ctx: ModuleContext) {
        self.start_with(ctx, Options::default());
    }

    fn is_dynamic_source(&self) -> bool {
        true
    }

    /// The module matches other layouts and scripts itself, so the readings are not used.
    fn search_with(
        &mut self,
        query: &str,
        _readings: &[QueryReading],
    ) -> anyhow::Result<SearchAnswer> {
        let items = self.root_items(query);
        // No index yet: the root list shows the busy line, and is asked again once the
        // index is in (see `on_wake`).
        let busy = self.answers_in_root(query.trim())
            && self.live.as_ref().is_some_and(|live| !live.is_ready());
        self.answered_busy |= busy;
        Ok(SearchAnswer {
            busy,
            ..SearchAnswer::from(items)
        })
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        if item.id == ItemId::new(PROVIDER, COMMAND) {
            if !self.has_home() {
                return Ok(vec![Effect::Toast(Toast {
                    kind: ToastKind::Error,
                    title: "File search needs a home folder".to_string(),
                    message: None,
                })]);
            }
            self.screen = Some(Screen::default());
            return Ok(vec![Effect::PushView(self.screen_view())]);
        }
        if item.id == ItemId::new(PROVIDER, MORE) {
            self.screen = Some(Screen {
                query: self.root_query.clone(),
                revision: 0,
            });
            return Ok(vec![Effect::PushView(self.screen_view())]);
        }
        self.path_action(&item.id, action_id, None)
    }

    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        #[cfg(target_os = "linux")]
        if view.0 == open_with::VIEW_ID {
            return match event {
                ViewEvent::ActionChosen { row_id, action_id } if action_id == "open" => {
                    self.open_with_choose(&row_id)
                }
                ViewEvent::ActionChosen { action_id, .. } => {
                    anyhow::bail!("files: unknown action {action_id:?}")
                }
                ViewEvent::Closed => {
                    self.open_with = None;
                    Ok(Vec::new())
                }
                _ => Ok(Vec::new()),
            };
        }
        if view.0 != screen::VIEW_ID {
            return Ok(Vec::new());
        }
        match event {
            ViewEvent::QueryChanged { revision, text, .. } => {
                self.screen = Some(Screen {
                    query: text,
                    revision,
                });
                Ok(vec![Effect::UpdateView(self.screen_view())])
            }
            ViewEvent::ActionChosen { action_id, .. } if action_id == screen::TOGGLE => {
                self.state.layout = self.state.layout.flipped();
                self.save_state();
                Ok(vec![Effect::UpdateView(self.screen_view())])
            }
            ViewEvent::ActionChosen { row_id, action_id } => {
                let id = ItemId::new(PROVIDER, &row_id);
                let query = self
                    .screen
                    .as_ref()
                    .map(|s| s.query.clone())
                    .unwrap_or_default();
                let mut effects = self.path_action(&id, &action_id, Some(&query))?;
                if action_id == "open" {
                    // Opened here, the file ranks by habit for what was typed here.
                    effects.insert(0, Effect::RecordUsage { item: id, query });
                }
                Ok(effects)
            }
            ViewEvent::Closed => {
                self.screen = None;
                if let Some(thumbs) = &self.thumbs {
                    thumbs.cancel_pending();
                }
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The index is in (the saved one was read or the first walk ended), or thumbnails were
    /// made. A root answer that said busy is answered again; the open screen is refreshed
    /// for its current field and layout (the core applies it to the screen's stack entry
    /// and drops it once the screen is gone, or the field has moved on to a newer revision).
    fn on_wake(&mut self, token: u64) -> anyhow::Result<Vec<Effect>> {
        if token != REFRESH {
            return Ok(Vec::new());
        }
        self.refresh.served();
        let mut effects = Vec::new();
        if self.answered_busy && self.live.as_ref().is_some_and(Live::is_ready) {
            self.answered_busy = false;
            effects.push(Effect::SearchAgain);
        }
        if self.screen.is_some() {
            effects.push(Effect::UpdateView(self.screen_view()));
        }
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;
    use tomakast_core::{Icon, ItemKind, Toast, ToastKind};

    use crate::names::item_id;

    /// Generous: the machine running the tests may be busy.
    const PATIENCE: Duration = Duration::from_secs(10);

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// What the module asked to open and reveal.
    #[derive(Clone, Default)]
    struct Recorder {
        opened: Arc<Mutex<Vec<PathBuf>>>,
        revealed: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl Opener for Recorder {
        fn open(&self, path: &Path) -> anyhow::Result<()> {
            self.opened.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }

        fn reveal(&self, path: &Path) -> anyhow::Result<()> {
            self.revealed.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    /// A home folder, a config file and the module's folders, all in a temp folder.
    struct Env {
        tmp: TempDir,
        home: PathBuf,
        recorder: Recorder,
        /// Wake-ups the module asked for.
        wakes: Arc<AtomicUsize>,
    }

    impl Env {
        fn new() -> Env {
            let tmp = TempDir::new().unwrap();
            let home = tmp.path().join("home");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(tmp.path().join("cfg")).unwrap();
            Env {
                tmp,
                home,
                recorder: Recorder::default(),
                wakes: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn config_file(&self) -> PathBuf {
            self.tmp.path().join("cfg/config.toml")
        }

        fn touch(&self, rel: &str) -> PathBuf {
            let p = self.home.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"x").unwrap();
            p
        }

        fn mime(&self) -> MimeDb {
            let dir = self.tmp.path().join("mime");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("globs2"),
                "50:image/png:*.png\n50:text/plain:*.txt\n",
            )
            .unwrap();
            // Every icon name is in the theme: no dependence on the machine's themes.
            MimeDb::from_mime_dirs(&[dir]).with_icon_lookup(|_| true)
        }

        /// The module, started on this home with the machine's git config left out; returns
        /// once `ready` is indexed.
        fn start(&self, ready: &Path) -> FilesModule {
            self.start_with(ready, Options::default())
        }

        fn start_with(&self, ready: &Path, opts: Options) -> FilesModule {
            let mut module = self.module();
            module.begin(Options {
                global_ignore: false,
                ..opts
            });
            wait_for("the index", || module.indexed(ready));
            module
        }

        /// The module on this home, not started; its wake-ups are counted in `wakes`.
        fn module(&self) -> FilesModule {
            let config = FilesConfig {
                config_file: self.config_file(),
                data_dir: self.tmp.path().join("data"),
                home: self.home.clone(),
                thumbnail_root: self.thumbnail_root(),
                recent_file: None,
            };
            let mut module =
                FilesModule::with_parts(config, Box::new(self.recorder.clone()), Some(self.mime()));
            let wakes = self.wakes.clone();
            module.refresh = Arc::new(Refresh::new(Box::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            })));
            module
        }

        fn thumbnail_root(&self) -> PathBuf {
            self.tmp.path().join("thumbnails")
        }
    }

    impl FilesModule {
        fn indexed(&self, path: &Path) -> bool {
            self.live
                .as_ref()
                .is_some_and(|live| live.read(|i| i.id_of(path).is_some()))
        }

        /// The files and folders of the root answer for `query`, without its fallback.
        fn root(&mut self, query: &str) -> Vec<Item> {
            let items = self.root_answer(query);
            items.into_iter().filter(|i| i.kind.is_path()).collect()
        }

        /// The whole root answer for `query`, the fallback first when there are files.
        fn root_answer(&mut self, query: &str) -> Vec<Item> {
            self.search_with(query, &[]).unwrap().items
        }
    }

    fn ids(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    fn action_ids(item: &Item) -> Vec<&str> {
        item.actions.iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn root_answers_need_three_characters() {
        let env = Env::new();
        let fire = env.touch("fire.txt");
        let mut module = env.start(&fire);
        assert_eq!(ids(&module.root("fir")), [item_id(&fire).as_str()]);
        assert!(module.root("fi").is_empty());
        assert!(module.root("  fi  ").is_empty());
        assert!(module.root("").is_empty());
    }

    #[test]
    fn root_answers_are_capped_at_three() {
        let env = Env::new();
        for n in 0..5 {
            env.touch(&format!("fire{n}.txt"));
        }
        let last = env.home.join("fire4.txt");
        let mut module = env.start(&last);
        wait_for("all five", || {
            (0..5).all(|n| module.indexed(&env.home.join(format!("fire{n}.txt"))))
        });
        assert_eq!(module.root("fire").len(), 3);
    }

    #[test]
    fn root_answer_leads_with_the_fallback_that_opens_the_screen_with_the_query() {
        let env = Env::new();
        let fire = env.touch("fire.txt");
        let mut module = env.start(&fire);
        let answer = module.root_answer(" fire ");
        assert_eq!(
            ids(&answer),
            ["files/more", item_id(&fire).as_str()],
            "the fallback, then the file"
        );
        let more = &answer[0];
        assert_eq!(more.kind, ItemKind::Fallback);
        assert_eq!(more.title, "Search Files for \u{201c}fire\u{201d}");
        let effects = module.run_action(more, "open").unwrap();
        let [Effect::PushView(View::List(view))] = &effects[..] else {
            panic!("{effects:?}");
        };
        assert_eq!(view.query, "fire");
        assert_eq!(
            view.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            [local_part(&item_id(&fire))]
        );
        // No files, no fallback.
        assert!(module.root_answer("zzzq").is_empty());
    }

    #[test]
    fn in_root_false_answers_nothing() {
        let env = Env::new();
        fs::write(env.config_file(), "[files]\nin_root = false\n").unwrap();
        let fire = env.touch("fire.txt");
        let mut module = env.start(&fire);
        assert!(module.root("fire").is_empty());
    }

    #[test]
    fn file_item_shape() {
        let env = Env::new();
        let fire = env.touch("docs/fire.txt");
        let mut module = env.start(&fire);
        let items = module.root("fire");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, item_id(&fire));
        assert_eq!(item.kind, ItemKind::File);
        assert_eq!(item.title, "fire.txt");
        assert_eq!(item.subtitle.as_deref(), Some("~/docs"));
        assert_eq!(item.keywords, ["~/docs/fire.txt"]);
        assert_eq!(item.icon, Some(Icon::Named("text-plain".into())));
        let mut expected = vec!["open", "reveal", "copy-path"];
        if cfg!(target_os = "linux") {
            expected.push("open-with");
        }
        assert_eq!(action_ids(item), expected);
        let shortcuts: Vec<Option<&str>> =
            item.actions.iter().map(|a| a.shortcut.as_deref()).collect();
        let mut keys = vec![None, Some("ctrl+shift+f"), Some("ctrl+shift+c")];
        if cfg!(target_os = "linux") {
            keys.push(Some("ctrl+o"));
        }
        assert_eq!(shortcuts, keys);
        let titles: Vec<&str> = item.actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles[..3], ["Open", "Show in File Manager", "Copy Path"]);
    }

    #[test]
    fn folder_item_has_kind_folder() {
        let env = Env::new();
        let inner = env.touch("projects/firebird/readme.txt");
        let mut module = env.start(&inner);
        let items = module.root("firebird");
        let folder = items
            .iter()
            .find(|i| i.id == item_id(&env.home.join("projects/firebird")))
            .expect("the folder is found");
        assert_eq!(folder.kind, ItemKind::Folder);
        assert_eq!(folder.icon, Some(Icon::Named("folder".into())));
        assert_eq!(folder.subtitle.as_deref(), Some("~/projects"));
    }

    #[test]
    fn copy_path_copies_and_toasts() {
        let env = Env::new();
        let fire = env.touch("fire.txt");
        let mut module = env.start(&fire);
        let item = module.root("fire").remove(0);
        let effects = module.run_action(&item, "copy-path").unwrap();
        assert_eq!(
            effects,
            [
                Effect::CopyText(fire.display().to_string()),
                Effect::Toast(Toast {
                    kind: ToastKind::Success,
                    title: "Path copied".into(),
                    message: None,
                }),
            ]
        );
    }

    #[test]
    fn open_and_reveal_hand_the_path_over_and_close() {
        let env = Env::new();
        let fire = env.touch("fire.txt");
        let mut module = env.start(&fire);
        let item = module.root("fire").remove(0);
        assert_eq!(
            module.run_action(&item, "open").unwrap(),
            [Effect::CloseWindow]
        );
        assert_eq!(
            *env.recorder.opened.lock().unwrap(),
            std::slice::from_ref(&fire)
        );
        assert_eq!(module.state.opened[&fire].count, 1);
        assert_eq!(
            module.run_action(&item, "reveal").unwrap(),
            [Effect::CloseWindow]
        );
        assert_eq!(*env.recorder.revealed.lock().unwrap(), [fire]);
    }

    #[test]
    fn open_of_a_vanished_file_fails_with_a_toast() {
        let env = Env::new();
        let fire = env.touch("docs/fire.txt");
        // No folder is watched: only the module's own report takes the entry out.
        let no_watcher = Options {
            watch_cap: 0,
            ..Options::default()
        };
        let mut module = env.start_with(&fire, no_watcher);
        let item = module.root("fire").remove(0);
        fs::remove_file(&fire).unwrap();
        for action in ["open", "reveal", "copy-path"] {
            let err = module.run_action(&item, action).unwrap_err().to_string();
            assert_eq!(err, "File not found: ~/docs/fire.txt", "{action}");
        }
        assert!(env.recorder.opened.lock().unwrap().is_empty());
        assert!(env.recorder.revealed.lock().unwrap().is_empty());
        wait_for("the entry to leave the index", || !module.indexed(&fire));
    }

    #[cfg(unix)]
    #[test]
    fn opens_a_file_with_a_newline_in_its_name() {
        use std::os::unix::ffi::OsStrExt;
        let env = Env::new();
        let odd = env.touch("odd\nfire.txt");
        let mut module = env.start(&odd);
        let item = module.root("fire").remove(0);
        assert_eq!(item.id, item_id(&odd));
        assert_eq!(item.title, "odd\u{fffd}fire.txt");
        module.run_action(&item, "open").unwrap();
        let opened = env.recorder.opened.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].as_os_str().as_bytes(), odd.as_os_str().as_bytes());
    }

    #[test]
    fn remembers_the_last_items_handed_out() {
        let mut handed = Handed::default();
        let path = |n: usize| PathBuf::from(format!("/h/{n}"));
        let id = |n: usize| item_id(&path(n));
        for n in 0..REMEMBERED {
            handed.remember(id(n), path(n));
        }
        // Handed out again: now the newest, so not the next to go.
        for _ in 0..3 * REMEMBERED {
            handed.remember(id(0), path(0));
        }
        handed.remember(id(REMEMBERED), path(REMEMBERED));
        assert_eq!(handed.get(&id(0)), Some(path(0).as_path()));
        assert_eq!(handed.get(&id(1)), None);
        assert_eq!(handed.get(&id(2)), Some(path(2).as_path()));
        assert_eq!(handed.paths.len(), REMEMBERED);
        assert!(handed.order.len() <= 2 * REMEMBERED + 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reveal_asks_the_file_manager_then_opens_the_folder() {
        use tomakast_module_apps::{FileManager, FileManagerError};

        struct Fm(bool, Mutex<Vec<String>>);
        impl FileManager for Fm {
            fn show_items(&self, uris: &[String]) -> Result<(), FileManagerError> {
                self.1.lock().unwrap().extend_from_slice(uris);
                if self.0 {
                    Ok(())
                } else {
                    Err(FileManagerError::Unavailable("no service".into()))
                }
            }
        }

        let file = Path::new("/home/u/My Docs/a b.txt");
        for works in [true, false] {
            let fm = Fm(works, Mutex::new(Vec::new()));
            let mut opened = Vec::new();
            reveal::reveal_with(file, &fm, &mut |dir| {
                opened.push(dir.to_path_buf());
                Ok(())
            })
            .unwrap();
            assert_eq!(
                *fm.1.lock().unwrap(),
                ["file:///home/u/My%20Docs/a%20b.txt"]
            );
            let expected: &[PathBuf] = if works {
                &[]
            } else {
                &[PathBuf::from("/home/u/My Docs")]
            };
            assert_eq!(opened, expected, "service works: {works}");
        }
    }

    #[test]
    fn without_a_home_folder_nothing_starts() {
        let env = Env::new();
        env.touch("fire.txt");
        for home in ["", "relative/home"] {
            let config = FilesConfig {
                config_file: env.config_file(),
                data_dir: env.tmp.path().join("data"),
                home: PathBuf::from(home),
                thumbnail_root: PathBuf::from(".cache/thumbnails"),
                recent_file: Some(PathBuf::from(".local/share/recently-used.xbel")),
            };
            let mut module =
                FilesModule::with_parts(config, Box::new(env.recorder.clone()), Some(env.mime()));
            module.begin(Options {
                global_ignore: false,
                ..Options::default()
            });
            assert!(module.live.is_none(), "{home:?}");
            assert!(module.root("fire").is_empty());
            assert_eq!(
                module.run_action(&command_item(), "open").unwrap(),
                [Effect::Toast(Toast {
                    kind: ToastKind::Error,
                    title: "File search needs a home folder".into(),
                    message: None,
                })]
            );
        }
    }

    #[test]
    fn search_command_item() {
        let item = command_item();
        assert_eq!(item.id.as_str(), "files/search");
        assert_eq!(item.title, "Search Files");
        assert_eq!(item.kind, ItemKind::Command);
        assert_eq!(item.subtitle.as_deref(), Some("Search files by name"));
        assert_eq!(item.icon, Some(Icon::Named("system-search".into())));
        assert_eq!(action_ids(&item), ["open"]);
    }

    /// The "Search Files" screen, driven as the UI drives it.
    mod screen_events {
        use super::*;
        use crate::screen::{TOGGLE, VIEW_ID};
        use crate::state::{Layout, State};
        use tomakast_core::{Accessory, ListLayout, ListView, View, ViewEvent, ViewId};

        fn view_id() -> ViewId {
            ViewId(VIEW_ID.to_string())
        }

        fn data_dir(env: &Env) -> PathBuf {
            env.tmp.path().join("data")
        }

        fn local(path: &Path) -> String {
            item_id(path).as_str()["files/".len()..].to_string()
        }

        fn list(effect: &Effect) -> &ListView {
            match effect {
                Effect::PushView(View::List(v)) | Effect::UpdateView(View::List(v)) => v,
                other => panic!("not a list view: {other:?}"),
            }
        }

        fn row_ids(view: &ListView) -> Vec<&str> {
            view.rows.iter().map(|r| r.id.as_str()).collect()
        }

        impl FilesModule {
            fn push_screen(&mut self) -> ListView {
                let effects = self.run_action(&command_item(), "open").unwrap();
                assert_eq!(effects.len(), 1, "{effects:?}");
                assert!(matches!(effects[0], Effect::PushView(_)), "{effects:?}");
                list(&effects[0]).clone()
            }

            fn send(&mut self, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
                self.on_view_event(&view_id(), event)
            }

            fn type_in(&mut self, revision: u64, text: &str) -> ListView {
                let effects = self
                    .send(ViewEvent::QueryChanged {
                        revision,
                        text: text.to_string(),
                        readings: Vec::new(),
                    })
                    .unwrap();
                assert_eq!(effects.len(), 1, "{effects:?}");
                assert!(matches!(effects[0], Effect::UpdateView(_)), "{effects:?}");
                list(&effects[0]).clone()
            }

            fn choose(&mut self, row_id: &str, action_id: &str) -> anyhow::Result<Vec<Effect>> {
                self.send(ViewEvent::ActionChosen {
                    row_id: row_id.to_string(),
                    action_id: action_id.to_string(),
                })
            }
        }

        fn toggle_title(view: &ListView) -> &str {
            let toggle = view.rows[0].actions.last().unwrap();
            assert_eq!(toggle.id, TOGGLE);
            &toggle.title
        }

        #[test]
        fn command_pushes_the_screen_in_the_saved_layout() {
            let env = Env::new();
            let fire = env.touch("fire.txt");

            let mut module = env.start(&fire);
            let pushed = module.push_screen();
            assert_eq!(pushed.id, view_id());
            assert_eq!(pushed.title, "Search Files");
            assert_eq!(pushed.query, "");
            assert_eq!(pushed.revision, 0);
            assert_eq!(pushed.layout, ListLayout::Rows);
            assert_eq!(toggle_title(&pushed), "Show as Grid");

            fs::create_dir_all(data_dir(&env)).unwrap();
            fs::write(data_dir(&env).join("state.json"), r#"{"layout":"Grid"}"#).unwrap();
            let mut module = env.start(&fire);
            let pushed = module.push_screen();
            assert_eq!(pushed.layout, ListLayout::Grid { columns: 5 });
            assert_eq!(toggle_title(&pushed), "Show as List");
        }

        #[test]
        fn empty_query_lists_recent() {
            let env = Env::new();
            let old = env.touch("docs/old.txt");
            let new = env.touch("new.txt");
            // "Today" whatever the wall clock: the module's now is the file's modification
            // time (a day long ago, in any time zone), not a moment that may cross midnight.
            let then = 1_700_000_000;
            fs::File::options()
                .write(true)
                .open(&old)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_secs(then as u64))
                .unwrap();
            let mut module = env.start(&old);
            module.clock = Box::new(move || then);
            wait_for("both files", || module.indexed(&new));
            // Opened through the launcher: listed first.
            let item = module.root("old.txt").remove(0);
            module.run_action(&item, "open").unwrap();

            let pushed = module.push_screen();
            let ids = row_ids(&pushed);
            assert_eq!(ids[0], local(&old), "{ids:?}");
            assert!(ids.contains(&local(&new).as_str()), "{ids:?}");
            let row = &pushed.rows[0];
            assert_eq!(row.title, "old.txt");
            assert_eq!(row.subtitle.as_deref(), Some("~/docs"));
            assert_eq!(row.icon, Some(Icon::Named("text-plain".into())));
            assert!(
                matches!(&row.accessories[..], [Accessory::Text(t)] if t.starts_with("Today ")),
                "{:?}",
                row.accessories
            );
            // Typing and clearing the field lists them again.
            module.type_in(1, "new");
            assert_eq!(row_ids(&module.type_in(2, "")), ids);
        }

        /// `10/2` or `2+2` in the launcher is arithmetic for calc, not a file name: the root
        /// list gets no files for a query without a letter, the screen still searches it.
        #[test]
        fn queries_without_a_letter_get_no_root_files() {
            let env = Env::new();
            let half = env.touch("10/2.txt");
            let sum = env.touch("2+2.txt");
            let mut module = env.start(&half);
            wait_for("both files", || module.indexed(&sum));
            for query in ["10/2", "2+2", " 1/3 ", "2026"] {
                assert!(module.root(query).is_empty(), "{query:?}");
            }
            assert_eq!(ids(&module.root("10/2.t")), [item_id(&half).as_str()]);
            module.push_screen();
            assert_eq!(row_ids(&module.type_in(1, "10/2")), [local(&half)]);
            assert_eq!(row_ids(&module.type_in(2, "2+2")), [local(&sum)]);
        }

        #[test]
        fn query_changed_answers_with_the_same_revision() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            env.touch("water.txt");
            let mut module = env.start(&fire);
            module.push_screen();
            let answer = module.type_in(7, "fire");
            assert_eq!(answer.revision, 7);
            assert_eq!(answer.query, "fire");
            assert_eq!(row_ids(&answer), [local(&fire)]);
            assert_eq!(answer.layout, ListLayout::Rows);
            // The layout is part of every answer, so typing never flips the grid back.
            module.choose(&local(&fire), TOGGLE).unwrap();
            let answer = module.type_in(8, "fir");
            assert_eq!(answer.revision, 8);
            assert_eq!(answer.layout, ListLayout::Grid { columns: 5 });
        }

        #[test]
        fn toggle_flips_and_persists_layout() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            let mut module = env.start(&fire);
            module.push_screen();
            module.type_in(3, "fire");

            let effects = module.choose(&local(&fire), TOGGLE).unwrap();
            assert_eq!(effects.len(), 1, "{effects:?}");
            assert!(matches!(effects[0], Effect::UpdateView(_)), "{effects:?}");
            let grid = list(&effects[0]);
            assert_eq!(grid.revision, 3);
            assert_eq!(grid.query, "fire");
            assert_eq!(grid.layout, ListLayout::Grid { columns: 5 });
            assert_eq!(row_ids(grid), [local(&fire)]);
            assert_eq!(toggle_title(grid), "Show as List");
            assert_eq!(State::load(&data_dir(&env)).layout, Layout::Grid);
            assert!(env.recorder.opened.lock().unwrap().is_empty());

            // A new run starts in the grid.
            let mut again = env.start(&fire);
            assert_eq!(again.push_screen().layout, ListLayout::Grid { columns: 5 });

            let effects = module.choose(&local(&fire), TOGGLE).unwrap();
            assert_eq!(list(&effects[0]).layout, ListLayout::Rows);
            assert_eq!(State::load(&data_dir(&env)).layout, Layout::List);
        }

        #[test]
        fn open_from_screen_records_usage_with_the_screen_query() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            let mut module = env.start(&fire);
            module.push_screen();
            module.type_in(1, "fi");
            module.type_in(2, "Fir");
            let effects = module.choose(&local(&fire), "open").unwrap();
            assert_eq!(
                effects,
                [
                    Effect::RecordUsage {
                        item: item_id(&fire),
                        query: "Fir".into(),
                    },
                    Effect::CloseWindow,
                ]
            );
            assert_eq!(
                *env.recorder.opened.lock().unwrap(),
                std::slice::from_ref(&fire)
            );
            assert_eq!(module.state.opened[&fire].count, 1);
            // Kept for the next run.
            assert_eq!(State::load(&data_dir(&env)).opened[&fire].count, 1);
        }

        #[test]
        fn screen_has_no_minimum_and_caps_at_100() {
            let env = Env::new();
            for n in 0..105 {
                env.touch(&format!("f{n}.txt"));
            }
            let mut module = env.start(&env.home.join("f104.txt"));
            wait_for("all files", || {
                (0..105).all(|n| module.indexed(&env.home.join(format!("f{n}.txt"))))
            });
            module.push_screen();
            assert_eq!(module.type_in(1, "f").rows.len(), 100);
            // One character is enough here (the root list wants three).
            assert!(module.root("f").is_empty());
            assert_eq!(module.type_in(3, "").rows.len(), 100);
        }

        #[test]
        fn screen_actions_on_a_file() {
            let env = Env::new();
            let fire = env.touch("docs/fire.txt");
            let no_watcher = Options {
                watch_cap: 0,
                ..Options::default()
            };
            let mut module = env.start_with(&fire, no_watcher);
            module.push_screen();
            let row = local(&fire);
            assert_eq!(row_ids(&module.type_in(1, "fire")), [row.as_str()]);
            assert_eq!(
                module.choose(&row, "reveal").unwrap(),
                [Effect::CloseWindow]
            );
            assert_eq!(
                *env.recorder.revealed.lock().unwrap(),
                std::slice::from_ref(&fire)
            );
            assert!(matches!(
                &module.choose(&row, "copy-path").unwrap()[0],
                Effect::CopyText(text) if *text == fire.display().to_string()
            ));
            assert!(module.choose("0123", "open").is_err());
            fs::remove_file(&fire).unwrap();
            let err = module.choose(&row, "open").unwrap_err().to_string();
            assert_eq!(err, "File not found: ~/docs/fire.txt");
            assert!(env.recorder.opened.lock().unwrap().is_empty());
        }

        #[test]
        fn unknown_modified_time_shows_no_date() {
            let env = Env::new();
            let old = env.touch("old.txt");
            fs::File::options()
                .write(true)
                .open(&old)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH)
                .unwrap();
            let mut module = env.start(&old);
            module.push_screen();
            let view = module.type_in(1, "old");
            assert_eq!(row_ids(&view), [local(&old)]);
            assert!(
                view.rows[0].accessories.is_empty(),
                "{:?}",
                view.rows[0].accessories
            );
        }

        #[test]
        fn screen_fills_in_when_the_first_walk_ends() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            // The first walk waits for the test.
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let gate = Mutex::new(gate);
            let mut module = env.module();
            module.begin(Options {
                global_ignore: false,
                walk: Box::new(move |rules| {
                    let _ = gate.lock().unwrap().recv_timeout(PATIENCE);
                    crate::index::FileIndex::walk(rules)
                }),
                ..Options::default()
            });
            let pushed = module.push_screen();
            assert!(pushed.rows.is_empty());
            assert!(pushed.busy, "no index yet: the busy line shows");
            let typed = module.type_in(3, "fir");
            assert!(typed.rows.is_empty());
            assert!(typed.busy);
            assert_eq!(env.wakes.load(Ordering::SeqCst), 0);

            release.send(()).unwrap();
            wait_for("the wake-up", || env.wakes.load(Ordering::SeqCst) > 0);
            let effects = module.on_wake(REFRESH).unwrap();
            assert_eq!(effects.len(), 1, "{effects:?}");
            assert!(matches!(effects[0], Effect::UpdateView(_)), "{effects:?}");
            let view = list(&effects[0]);
            assert_eq!((view.query.as_str(), view.revision), ("fir", 3));
            assert!(!view.busy, "the index is in: no busy line");
            assert_eq!(view.layout, ListLayout::Rows);
            assert_eq!(row_ids(view), [local(&fire)]);

            // Closed: a wake-up shows nothing.
            module.send(ViewEvent::Closed).unwrap();
            assert!(module.on_wake(REFRESH).unwrap().is_empty());
        }

        #[test]
        fn root_list_is_busy_until_the_first_walk_ends_then_asked_again() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let gate = Mutex::new(gate);
            let mut module = env.module();
            module.begin(Options {
                global_ignore: false,
                walk: Box::new(move |rules| {
                    let _ = gate.lock().unwrap().recv_timeout(PATIENCE);
                    crate::index::FileIndex::walk(rules)
                }),
                ..Options::default()
            });
            // A query files are not asked about is not busy.
            assert!(!module.search_with("fi", &[]).unwrap().busy);
            assert!(!module.search_with("10/2", &[]).unwrap().busy);
            let waiting = module.search_with("fir", &[]).unwrap();
            assert!(waiting.busy, "no index yet: the busy line shows");
            assert!(waiting.items.is_empty());

            release.send(()).unwrap();
            wait_for("the wake-up", || env.wakes.load(Ordering::SeqCst) > 0);
            // No screen open: only the root list is asked again.
            let effects = module.on_wake(REFRESH).unwrap();
            assert_eq!(effects, [Effect::SearchAgain]);
            let answer = module.search_with("fir", &[]).unwrap();
            assert!(!answer.busy);
            assert!(answer.items.iter().any(|item| item.id == item_id(&fire)));
            // Asked again once only.
            assert!(module.on_wake(REFRESH).unwrap().is_empty());
        }

        #[test]
        fn root_list_is_not_busy_with_an_index_in() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            let mut module = env.start(&fire);
            wait_for("the index", || {
                module.live.as_ref().is_some_and(Live::is_ready)
            });
            assert!(!module.search_with("fir", &[]).unwrap().busy);
            assert!(module.on_wake(REFRESH).unwrap().is_empty());
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn grid_shows_thumbnails_and_makes_missing_ones() {
            let env = Env::new();
            let pic = env.home.join("pic.png");
            image::RgbImage::from_pixel(600, 300, image::Rgb([9, 9, 9]))
                .save(&pic)
                .unwrap();
            let doc = env.touch("pic notes.txt");
            let mut module = env.start(&pic);
            wait_for("both files", || module.indexed(&doc));
            // The first walk's wake-up, served while no screen is open: nothing to show.
            wait_for("the first walk's wake-up", || {
                env.wakes.load(Ordering::SeqCst) == 1
            });
            assert!(module.on_wake(REFRESH).unwrap().is_empty());
            module.push_screen();

            // The list asks for no thumbnail.
            let listed = module.type_in(1, "pic");
            assert_eq!(listed.rows.len(), 2);
            assert_eq!(module.thumbs.as_ref().unwrap().requested(), 0);
            assert!(
                listed
                    .rows
                    .iter()
                    .all(|r| matches!(r.icon, Some(Icon::Named(_))))
            );

            // The grid asks for the picture's, and shows its type icon meanwhile.
            let effects = module.choose(&local(&pic), TOGGLE).unwrap();
            let grid = list(&effects[0]);
            assert_eq!(grid.layout, ListLayout::Grid { columns: 5 });
            let icon_of = |view: &ListView, path: &Path| {
                view.rows
                    .iter()
                    .find(|r| r.id == local(path))
                    .unwrap()
                    .icon
                    .clone()
            };
            assert_eq!(icon_of(grid, &pic), Some(Icon::Named("image-png".into())));
            assert_eq!(module.thumbs.as_ref().unwrap().requested(), 1);

            wait_for("the wake-up", || env.wakes.load(Ordering::SeqCst) == 2);
            let effects = module.on_wake(REFRESH).unwrap();
            assert_eq!(effects.len(), 1, "{effects:?}");
            let updated = list(&effects[0]);
            assert_eq!((updated.query.as_str(), updated.revision), ("pic", 1));
            assert_eq!(updated.layout, ListLayout::Grid { columns: 5 });
            let thumb = env
                .thumbnail_root()
                .join("large")
                .join(crate::thumbs::thumbnail_name(&pic));
            assert_eq!(icon_of(updated, &pic), Some(Icon::File(thumb.clone())));
            assert_eq!(
                icon_of(updated, &doc),
                Some(Icon::Named("text-plain".into()))
            );
            assert_eq!(image::image_dimensions(&thumb).unwrap(), (256, 128));
            // One thumbnail, one wake-up; the refresh asks for nothing new.
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(env.wakes.load(Ordering::SeqCst), 2);
            assert_eq!(module.thumbs.as_ref().unwrap().queued(), 0);
        }

        #[test]
        fn closed_forgets_the_query() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            env.touch("water.txt");
            let mut module = env.start(&fire);
            module.push_screen();
            module.type_in(4, "fire");
            assert!(module.send(ViewEvent::Closed).unwrap().is_empty());
            // A later toggle answers for an empty field at the pushed revision.
            let effects = module.choose(&local(&fire), TOGGLE).unwrap();
            let view = list(&effects[0]);
            assert_eq!((view.query.as_str(), view.revision), ("", 0));
            assert_eq!(view.rows.len(), 2);
            // Events of another view are not the screen's.
            let other = ViewId("files/other".into());
            assert!(
                module
                    .on_view_event(&other, ViewEvent::Closed)
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[cfg(target_os = "linux")]
    mod open_with_events {
        use super::*;
        use crate::open_with::{Backend, VIEW_ID};
        use tomakast_core::{View, ViewEvent, ViewId};
        use tomakast_module_apps::AppEntry;

        /// Installed applications given by the test; what is launched is recorded.
        #[derive(Clone, Default)]
        struct Fake {
            apps: Vec<(Item, AppEntry)>,
            default: Option<String>,
            launched: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
        }

        impl Backend for Fake {
            fn apps(&self) -> Vec<(Item, AppEntry)> {
                self.apps.clone()
            }

            fn default_for(&self, _mime: &str) -> Option<String> {
                self.default.clone()
            }

            fn launch(&self, entry: &AppEntry, file: &Path) -> anyhow::Result<()> {
                self.launched
                    .lock()
                    .unwrap()
                    .push((entry.desktop_file.clone(), file.to_path_buf()));
                Ok(())
            }
        }

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

        fn fake() -> Fake {
            Fake {
                apps: vec![
                    app("gwenview.desktop", "Gwenview", &["image/png"]),
                    app("kate.desktop", "Kate", &["text/plain"]),
                    app("okular.desktop", "Okular", &["image/png"]),
                ],
                ..Fake::default()
            }
        }

        fn view_id() -> ViewId {
            ViewId(VIEW_ID.to_string())
        }

        fn pushed(effects: &[Effect]) -> &tomakast_core::ListView {
            match effects {
                [Effect::PushView(View::List(v))] => v,
                other => panic!("not one pushed list: {other:?}"),
            }
        }

        fn choose(module: &mut FilesModule, row: &str) -> anyhow::Result<Vec<Effect>> {
            module.on_view_event(
                &view_id(),
                ViewEvent::ActionChosen {
                    row_id: row.to_string(),
                    action_id: "open".to_string(),
                },
            )
        }

        #[test]
        fn root_row_lists_the_apps_and_launches_without_recording_usage() {
            let env = Env::new();
            let pic = env.touch("pic.png");
            let backend = fake();
            let mut module = env.start(&pic);
            module.open_with_backend = Box::new(backend.clone());
            let item = module.root("pic").remove(0);
            let effects = module.run_action(&item, "open-with").unwrap();
            let view = pushed(&effects);
            assert_eq!(view.id.0, VIEW_ID);
            assert_eq!(view.title, "Open pic.png With");
            let rows: Vec<&str> = view.rows.iter().map(|r| r.id.as_str()).collect();
            assert_eq!(rows, ["gwenview.desktop", "okular.desktop"]);
            // The root action already counted the use: only the module's own count follows.
            assert_eq!(
                choose(&mut module, "okular.desktop").unwrap(),
                [Effect::CloseWindow]
            );
            assert_eq!(
                *backend.launched.lock().unwrap(),
                [(PathBuf::from("/apps/okular.desktop"), pic.clone())]
            );
            assert!(env.recorder.opened.lock().unwrap().is_empty());
            assert_eq!(module.state.opened[&pic].count, 1);
            assert_eq!(
                State::load(&env.tmp.path().join("data")).opened[&pic].count,
                1
            );
        }

        #[test]
        fn screen_row_records_usage_for_the_screen_query() {
            let env = Env::new();
            let pic = env.touch("pic.png");
            let mut module = env.start(&pic);
            module.open_with_backend = Box::new(fake());
            module.run_action(&command_item(), "open").unwrap();
            module
                .on_view_event(
                    &ViewId(screen::VIEW_ID.to_string()),
                    ViewEvent::QueryChanged {
                        revision: 1,
                        text: "pi".into(),
                        readings: Vec::new(),
                    },
                )
                .unwrap();
            let row = item_id(&pic).as_str()["files/".len()..].to_string();
            let effects = module
                .on_view_event(
                    &ViewId(screen::VIEW_ID.to_string()),
                    ViewEvent::ActionChosen {
                        row_id: row,
                        action_id: "open-with".into(),
                    },
                )
                .unwrap();
            pushed(&effects);
            assert_eq!(
                choose(&mut module, "gwenview.desktop").unwrap(),
                [
                    Effect::RecordUsage {
                        item: item_id(&pic),
                        query: "pi".into(),
                    },
                    Effect::CloseWindow,
                ]
            );
        }

        #[test]
        fn the_default_application_leads() {
            let env = Env::new();
            let pic = env.touch("pic.png");
            let mut module = env.start(&pic);
            module.open_with_backend = Box::new(Fake {
                default: Some("okular.desktop".into()),
                ..fake()
            });
            let item = module.root("pic").remove(0);
            let effects = module.run_action(&item, "open-with").unwrap();
            let rows: Vec<&str> = pushed(&effects)
                .rows
                .iter()
                .map(|r| r.id.as_str())
                .collect();
            assert_eq!(rows, ["okular.desktop", "gwenview.desktop"]);
        }

        #[test]
        fn a_type_without_applications_toasts() {
            let env = Env::new();
            let doc = env.touch("doc.xyz");
            let mut module = env.start(&doc);
            module.open_with_backend = Box::new(fake());
            let item = module.root("doc").remove(0);
            assert_eq!(
                module.run_action(&item, "open-with").unwrap(),
                [Effect::Toast(Toast {
                    kind: ToastKind::Error,
                    title: "No application opens this type".into(),
                    message: None,
                })]
            );
            assert!(module.open_with.is_none());
        }

        #[test]
        fn a_vanished_file_fails_and_closing_forgets_the_file() {
            let env = Env::new();
            let pic = env.touch("pic.png");
            let backend = fake();
            let mut module = env.start_with(
                &pic,
                Options {
                    watch_cap: 0,
                    ..Options::default()
                },
            );
            module.open_with_backend = Box::new(backend.clone());
            let item = module.root("pic").remove(0);
            module.run_action(&item, "open-with").unwrap();
            fs::remove_file(&pic).unwrap();
            let err = choose(&mut module, "gwenview.desktop").unwrap_err();
            assert_eq!(err.to_string(), "File not found: ~/pic.png");
            assert!(backend.launched.lock().unwrap().is_empty());
            assert!(module.open_with.is_some());
            assert!(
                module
                    .on_view_event(&view_id(), ViewEvent::Closed)
                    .unwrap()
                    .is_empty()
            );
            assert!(module.open_with.is_none());
            // Nothing to launch for after the list is gone.
            assert!(choose(&mut module, "gwenview.desktop").unwrap().is_empty());
            let err = module.run_action(&item, "open-with").unwrap_err();
            assert_eq!(err.to_string(), "File not found: ~/pic.png");
        }

        #[test]
        fn an_unknown_row_is_an_error() {
            let env = Env::new();
            let pic = env.touch("pic.png");
            let mut module = env.start(&pic);
            module.open_with_backend = Box::new(fake());
            let item = module.root("pic").remove(0);
            module.run_action(&item, "open-with").unwrap();
            assert!(choose(&mut module, "kate.desktop").is_err());
        }
    }

    /// The module in the core, as the launcher runs it: root answers ranked by the engine
    /// against apps, opens recorded by the core.
    mod root_list {
        use super::*;
        use tomakast_core::Core;
        use tomakast_core::store::Store;
        use tomakast_core::testing::FakeModule;
        use tomakast_core::{CoreEvent, CoreUpdate, Hit, LocalNow, RankWeights};

        /// `FilesModule` as `Module::start` starts it, with the machine's git config left out.
        struct Isolated(FilesModule);

        impl Module for Isolated {
            fn id(&self) -> &str {
                self.0.id()
            }

            fn start(&mut self, ctx: ModuleContext) {
                self.0.start_with(
                    ctx,
                    Options {
                        global_ignore: false,
                        ..Options::default()
                    },
                );
            }

            fn is_dynamic_source(&self) -> bool {
                true
            }

            fn search_with(
                &mut self,
                query: &str,
                readings: &[QueryReading],
            ) -> anyhow::Result<SearchAnswer> {
                self.0.search_with(query, readings)
            }

            fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
                self.0.run_action(item, action_id)
            }

            fn on_wake(&mut self, token: u64) -> anyhow::Result<Vec<Effect>> {
                self.0.on_wake(token)
            }
        }

        fn now() -> LocalNow {
            LocalNow::from_unix_with_offset(unix_now(), 0)
        }

        fn app(local: &str, title: &str) -> Item {
            Item {
                id: ItemId::new("apps", local),
                title: title.to_string(),
                subtitle: None,
                keywords: Vec::new(),
                icon: None,
                kind: ItemKind::Application,
                source: None,
                actions: vec![Action {
                    id: "launch".to_string(),
                    title: "Open".to_string(),
                    shortcut: None,
                }],
            }
        }

        /// A core with `apps` from a fake `apps` module and, with `files`, the files module
        /// on `env`'s home; returns once every module published its items.
        fn core(env: &Env, apps: Vec<Item>, files: bool) -> Core {
            let mut core = Core::new(
                Store::open_in_memory().unwrap(),
                RankWeights::default(),
                now(),
            )
            .unwrap();
            core.register(Box::new(
                FakeModule::new(apps, || Ok(vec![Effect::CloseWindow])).with_id("apps"),
            ));
            if files {
                core.register(Box::new(Isolated(env.module())));
            }
            core.start_modules();
            let mut published = 0;
            let want = if files { 2 } else { 1 };
            while published < want {
                if let CoreUpdate::ItemsChanged = core.apply(next_event(&core), now()) {
                    published += 1;
                }
            }
            core
        }

        fn next_event(core: &Core) -> CoreEvent {
            let rx = core.events();
            let deadline = Instant::now() + PATIENCE;
            loop {
                if let Ok(ev) = rx.try_recv() {
                    return ev;
                }
                assert!(
                    Instant::now() < deadline,
                    "no core event within {PATIENCE:?}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        /// A search result of generation `generation`, as the worker sends it.
        fn is_answer(ev: &CoreEvent, generation: u64) -> bool {
            match ev {
                CoreEvent::Instance { event, .. } => is_answer(event, generation),
                CoreEvent::SearchResults { generation: g, .. } => *g == generation,
                _ => false,
            }
        }

        /// Searches `query` and applies events until the files module's answer is ranked in.
        fn settle(core: &mut Core, query: &str) -> Vec<Hit> {
            let generation = core.search(query, now()).generation;
            loop {
                let ev = next_event(core);
                let answer = is_answer(&ev, generation);
                let update = core.apply(ev, now());
                if answer {
                    let CoreUpdate::Results(snapshot) = update else {
                        panic!("the answer was not ranked: {update:?}");
                    };
                    return snapshot.hits;
                }
            }
        }

        fn titles(hits: &[Hit]) -> Vec<&str> {
            hits.iter().map(|h| h.item.title.as_str()).collect()
        }

        /// Runs `action_id` on `item` for `query` and applies the reply.
        fn act(core: &mut Core, item: &ItemId, action_id: &str, query: &str) {
            let request = core.request_action(item, action_id, query).unwrap();
            loop {
                match core.apply(next_event(core), now()) {
                    CoreUpdate::ActionDone {
                        request: r,
                        effects,
                        ..
                    } if r == request => {
                        assert_eq!(effects, [Effect::CloseWindow]);
                        return;
                    }
                    CoreUpdate::ActionFailed {
                        request: r,
                        message,
                    } if r == request => {
                        panic!("{action_id} failed: {message}");
                    }
                    _ => {}
                }
            }
        }

        #[test]
        fn firefox_leads_until_the_file_is_opened_by_habit() {
            let env = Env::new();
            let fire = env.touch("fire.txt");
            let mut core = core(&env, vec![app("firefox", "Firefox")], true);
            // The first walk is in when the file is answered.
            let deadline = Instant::now() + PATIENCE;
            let hits = loop {
                let hits = settle(&mut core, "fire");
                if hits.iter().any(|h| h.item.kind == ItemKind::File) {
                    break hits;
                }
                assert!(Instant::now() < deadline, "fire.txt was never answered");
                std::thread::sleep(Duration::from_millis(20));
            };
            assert_eq!(titles(&hits), ["Firefox", "fire.txt"]);

            let file = hits[1].item.id.clone();
            assert_eq!(file, item_id(&fire));
            for _ in 0..3 {
                act(&mut core, &file, "open", "fire");
                // The root list is searched again before each open, as the user types again.
                settle(&mut core, "fire");
            }
            assert_eq!(
                env.recorder.opened.lock().unwrap().as_slice(),
                [fire.clone(), fire.clone(), fire.clone()]
            );
            assert_eq!(titles(&settle(&mut core, "fire")), ["fire.txt", "Firefox"]);
        }

        /// The UI thread's time per keystroke, measured with the thread clock (Linux).
        #[cfg(target_os = "linux")]
        mod ui_thread {
            use super::*;

            /// Names of the synthetic apps, Latin and Cyrillic (as the core's
            /// `search_across_10k_under_5ms`).
            const APP_NAMES: &[&str] = &[
                "Firefox Web Browser",
                "Fire Application",
                "Files",
                "Calculator",
                "Flatpak Updater",
                "Office Suite",
                "Файлы",
                "Калькулятор",
                "Параметры системы",
                "Почта",
                "Яндекс Музыка",
            ];

            /// `n` synthetic apps, each with a title and a keyword.
            fn corpus(n: usize) -> Vec<Item> {
                (0..n)
                    .map(|i| Item {
                        keywords: vec![format!("{} {i}", APP_NAMES[(i * 7) % APP_NAMES.len()])],
                        ..app(
                            &format!("app-{i}"),
                            &format!("{} {i}", APP_NAMES[i % APP_NAMES.len()]),
                        )
                    })
                    .collect()
            }

            /// CPU time the calling thread used so far.
            fn thread_cpu() -> Duration {
                let mut now = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                // SAFETY: `clock_gettime` writes one `timespec` through a valid pointer to ours.
                let read = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
                assert_eq!(read, 0, "no thread clock");
                Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
            }

            const RUNS: usize = 50;

            /// Median and 95th percentile, in ms.
            fn spread(mut times: Vec<Duration>) -> (f64, f64) {
                times.sort_unstable();
                let ms = |d: Duration| d.as_secs_f64() * 1000.0;
                (
                    ms(times[times.len() / 2]),
                    ms(times[(times.len() * 95).div_ceil(100) - 1]),
                )
            }

            /// The UI thread's CPU time of `RUNS` searches for `query` and, with `answered`, of
            /// applying the files module's answer to each (the wait for it is not counted).
            fn keystrokes(
                core: &mut Core,
                query: &str,
                answered: bool,
            ) -> (Vec<Duration>, Vec<Duration>) {
                let mut searches = Vec::with_capacity(RUNS);
                let mut applies = Vec::with_capacity(RUNS);
                for _ in 0..RUNS {
                    let started = thread_cpu();
                    let generation = core.search(query, now()).generation;
                    searches.push(thread_cpu() - started);
                    if !answered {
                        continue;
                    }
                    loop {
                        let ev = next_event(core);
                        if !is_answer(&ev, generation) {
                            core.apply(ev, now());
                            continue;
                        }
                        let started = thread_cpu();
                        let update = core.apply(ev, now());
                        applies.push(thread_cpu() - started);
                        assert!(matches!(update, CoreUpdate::Results(_)));
                        break;
                    }
                }
                (searches, applies)
            }

            /// The UI thread's time per keystroke with and without the module, over the same apps:
            /// `Core::search`, and the re-rank each of the module's answers triggers (it answers
            /// every query, an empty answer below three characters). A measurement, not a gate of
            /// `just check` (a debug build on a busy machine). In a release build it asserts the
            /// 5 ms budget at the 95th percentile with 1,000 apps (three times this machine's), for
            /// the search and the re-rank together; 10,000 apps are a stress figure only, past the
            /// budget without the module too. Run: `cargo test --release -p tomakast-module-files
            /// -- --ignored --nocapture ui_thread_time`.
            #[test]
            #[ignore]
            fn ui_thread_time_per_keystroke() {
                let env = Env::new();
                for i in 0..2_000 {
                    env.touch(&format!("dir{}/file{i}.txt", i % 50));
                }
                env.touch("fire.txt");
                env.touch("firefox-notes.md");
                env.touch("Отчёт.pdf");
                for size in [1_000, 10_000] {
                    let apps = corpus(size);
                    let mut without = core(&env, apps.clone(), false);
                    let mut with = core(&env, apps, true);
                    let deadline = Instant::now() + PATIENCE;
                    while !settle(&mut with, "fire")
                        .iter()
                        .any(|h| h.item.kind == ItemKind::File)
                    {
                        assert!(Instant::now() < deadline, "the walk never ended");
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    println!(
                        "{size} apps; ms of UI-thread CPU, median / p95\n{:<10} {:>15} {:>15} {:>15} \
                         {:>15}",
                        "query", "search without", "search with", "apply answer", "with, total"
                    );
                    for query in [
                        "", "f", "fi", "fir", "fire", "firefox", "ашку", "otchet", "file4",
                    ] {
                        let (plain, _) = keystrokes(&mut without, query, false);
                        let (searches, applies) = keystrokes(&mut with, query, true);
                        let total: Vec<Duration> = searches
                            .iter()
                            .zip(&applies)
                            .map(|(s, a)| *s + *a)
                            .collect();
                        let [plain, searches, applies, total] =
                            [plain, searches, applies, total].map(spread);
                        let cell = |(median, p95): (f64, f64)| format!("{median:.2} / {p95:.2}");
                        println!(
                            "{:<10} {:>15} {:>15} {:>15} {:>15}",
                            format!("{query:?}"),
                            cell(plain),
                            cell(searches),
                            cell(applies),
                            cell(total)
                        );
                        if size <= 1_000 && !cfg!(debug_assertions) {
                            assert!(total.1 < 5.0, "{query:?}: p95 {:.2} ms", total.1);
                        }
                    }
                }
            }
        }
    }
}
