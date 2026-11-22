//! The store state of one plugin instance and the host side of the plugin's imports.
//!
//! Every import checks the plugin's permissions itself; WASI is linked with an empty context
//! (no filesystem, environment, arguments, stdio or sockets) and its blocking poll calls are
//! refused, because epoch deadlines cannot interrupt host code.
//!
//! Host threads (the timer thread, the HTTP workers, the location thread) hand their events to the plugin's
//! [`Inbox`], which wakes the plugin's worker; [`crate::WasmPlugin::on_wake`] delivers them.

pub(crate) mod http;
pub(crate) mod locale;
pub(crate) mod location;
mod log;
pub(crate) mod open;
pub(crate) mod preferences;
mod storage;
pub(crate) mod timers;

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use tomakast_core::ModuleContext;

use wasmtime::component::{HasSelf, Linker, Resource, ResourceTable};
use wasmtime::{ResourceLimiter, bail};
use wasmtime_wasi::p2::DynPollable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::bindings::{self, tomakast::plugin as wit};
use crate::db::PluginDb;
use crate::manifest::{Manifest, Permissions};
use wit::types::{Event, HomeTile};

pub(crate) use log::{LogLimiter, sanitize};

/// Linear memory one plugin store may hold, over all its memories.
pub const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// Table elements one plugin store may hold, over all its tables.
pub const MAX_TABLE_ELEMENTS: usize = 10_000;
/// Core module instances one plugin store may create. The brief's "1 instance" means one
/// plugin component per store, which `WasmPlugin` guarantees by instantiating once; wasmtime
/// counts core module instances, and a component built by `wit-component` has several (the
/// Rust `wasm32-wasip2` kitchen-sink needs 3, measured). 8 leaves room for other toolchains;
/// memory and tables are bounded in total regardless.
pub const MAX_CORE_INSTANCES: usize = 8;
/// Memories and tables (each) one plugin store may create.
pub const MAX_MEMORIES_AND_TABLES: usize = 16;

/// The store data of one plugin instance.
pub(crate) struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    pub(crate) limits: Limits,
    /// Why a host function or the limiter stopped the guest, for the broken reason.
    pub(crate) fault: Option<String>,
    plugin: String,
    permissions: Permissions,
    db: PluginDb,
    /// Every declared preference with a value: the user's, else the manifest default.
    prefs: BTreeMap<String, String>,
    log: LogLimiter,
    timers: timers::Timers,
    http: http::Http,
    location: location::Location,
    opener: Arc<dyn open::Opener>,
    /// Where `locale.local-time-zone` reads the computer's time zone, on every call.
    time_zone: locale::ZoneSource,
    /// Where `locale.languages` reads the computer's languages, on every call.
    languages: locale::LanguagesSource,
    /// The call in progress was started by the user (`run`, `action-chosen` or
    /// `form-submitted`), not by the host (`init`, `search`, a timer, an HTTP answer, the other
    /// view events). Set by [`HostState::start_call`] before each call.
    pub(crate) user_action: bool,
    /// `open-url` and `open-path` calls in the call in progress that passed the checks and
    /// reached the opener.
    pub(crate) opens: u32,
    /// Recent `storage.set` and `storage.remove` calls, for their rate.
    storage_writes: crate::caps::Window,
    /// Test-only: where the imports the guest calls are recorded.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) recorder: Option<crate::recorder::Recorder>,
}

impl HostState {
    /// `prefs` are the user's values from `[plugins.<id>]`; only declared keys are kept. Timer
    /// and HTTP events go to `inbox`.
    pub(crate) fn new(
        manifest: &Manifest,
        db: PluginDb,
        prefs: BTreeMap<String, String>,
        inbox: &Arc<Inbox>,
    ) -> HostState {
        HostState {
            // The defaults already deny all of this; stated so that a new default cannot
            // open it. No preopened dirs, env, args; stdin closed, stdout and stderr discarded.
            wasi: WasiCtx::builder()
                .allow_tcp(false)
                .allow_udp(false)
                .allow_ip_name_lookup(false)
                // One `get-random-bytes` call returns at most 64 KiB.
                .max_random_size(64 * 1024)
                .build(),
            table: ResourceTable::new(),
            limits: Limits::default(),
            fault: None,
            plugin: manifest.id.clone(),
            permissions: manifest.permissions.clone(),
            db,
            prefs: preferences::effective(&manifest.preferences, prefs),
            log: LogLimiter::default(),
            timers: timers::TimerService::global().timers(Arc::clone(inbox)),
            http: http::Http::new(manifest.permissions.http.clone(), Arc::clone(inbox)),
            location: location::Location::new(manifest.permissions.location, Arc::clone(inbox)),
            opener: Arc::new(open::PlatformOpener),
            time_zone: locale::system_zone(),
            languages: locale::system_languages(),
            user_action: false,
            opens: 0,
            storage_writes: crate::caps::Window::default(),
            #[cfg(any(test, feature = "testing"))]
            recorder: None,
        }
    }

    /// Notes that the guest called the import `name` (`<interface>.<function>`, as in the
    /// WIT), when a test recorder is attached; nothing otherwise.
    #[cfg_attr(not(any(test, feature = "testing")), allow(unused_variables))]
    pub(crate) fn record_import(&self, name: &'static str) {
        #[cfg(any(test, feature = "testing"))]
        if let Some(recorder) = &self.recorder {
            recorder.note(format!("import {name}"));
        }
    }

    /// Resets the per-call state before a guest call; `user_action` says whether the user
    /// started it.
    pub(crate) fn start_call(&mut self, user_action: bool) {
        self.user_action = user_action;
        self.opens = 0;
    }

    pub(crate) fn set_opener(&mut self, opener: Arc<dyn open::Opener>) {
        self.opener = opener;
    }

    pub(crate) fn set_time_zone(&mut self, source: locale::ZoneSource) {
        self.time_zone = source;
    }

    pub(crate) fn set_languages(&mut self, source: locale::LanguagesSource) {
        self.languages = source;
    }

    pub(crate) fn set_locator(&mut self, locator: Arc<location::Locator>) {
        self.location.set_locator(locator);
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set_http_responder(&mut self, responder: http::Responder) {
        self.http.set_responder(responder);
    }

    /// Suspends the plugin's timers until `until` (Ruling AK).
    pub(crate) fn suspend_timers(&mut self, until: std::time::Instant) {
        self.timers.suspend(until);
    }

    /// Replaces the effective preferences, before `preferences-changed` is delivered.
    pub(crate) fn set_prefs(&mut self, prefs: BTreeMap<String, String>) {
        self.prefs = prefs;
    }

    /// Book-keeping for `event`, taken from the inbox: it no longer counts as waiting (a
    /// one-shot timer ends, an HTTP request frees its slot). Returns whether to deliver it:
    /// not the `timer-fired` of a timer the plugin cancelled meanwhile.
    pub(crate) fn delivering(&mut self, event: &Event) -> bool {
        match event {
            Event::TimerFired(id) => self.timers.delivered(*id),
            Event::HttpDone(_) => {
                self.http.delivered();
                true
            }
            Event::Location(_) => {
                self.location.delivered();
                true
            }
            Event::View(_) | Event::PreferencesChanged | Event::Widget(_) | Event::Home(_) => true,
        }
    }
}

/// Events from host threads waiting for the plugin's worker, and the coalescing wake-up: a
/// wake-up is sent only while none is pending, so a busy worker has at most one in its queue
/// however many events arrive.
#[derive(Default)]
pub(crate) struct Inbox {
    events: Mutex<VecDeque<Event>>,
    /// A wake-up is queued on the worker and not served yet.
    armed: AtomicBool,
    ctx: OnceLock<ModuleContext>,
    /// New effective preferences, waiting for their `preferences-changed` delivery.
    prefs: Mutex<Option<BTreeMap<String, String>>>,
    /// The plugin's latest Home tiles, waiting for their `home` delivery.
    home: Mutex<Option<Vec<HomeTile>>>,
}

impl Inbox {
    fn lock(&self) -> MutexGuard<'_, VecDeque<Event>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where wake-ups go; set once, in `start`, before the plugin can start a timer.
    pub(crate) fn connect(&self, ctx: ModuleContext) {
        if self.ctx.set(ctx).is_err() {
            tracing::error!(target: "plugin", "plugin inbox connected twice");
            return;
        }
        // Events queued before the worker started (new preferences) wake it now.
        if !self.lock().is_empty()
            && !self.armed.swap(true, Ordering::AcqRel)
            && let Some(ctx) = self.ctx.get()
        {
            ctx.wake(0);
        }
    }

    /// Hands new effective preferences to the plugin: one `preferences-changed` is queued
    /// unless one is already waiting, which then carries these values.
    pub(crate) fn set_preferences(&self, prefs: BTreeMap<String, String>) {
        let first = self
            .prefs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(prefs)
            .is_none();
        if first {
            self.push(Event::PreferencesChanged);
        }
    }

    /// The preferences waiting for delivery, if any.
    pub(crate) fn take_preferences(&self) -> Option<BTreeMap<String, String>> {
        self.prefs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Hands the plugin its Home tiles: one `home` event is queued unless one is already
    /// waiting, which then carries these tiles. The queued event's own list is a placeholder:
    /// [`take_home`](Self::take_home) gives the tiles at delivery.
    pub(crate) fn set_home(&self, tiles: Vec<HomeTile>) {
        let first = self
            .home
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(tiles)
            .is_none();
        if first {
            self.push(Event::Home(Vec::new()));
        }
    }

    /// The Home tiles waiting for delivery, if any.
    pub(crate) fn take_home(&self) -> Option<Vec<HomeTile>> {
        self.home
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Queues `event` and wakes the worker unless a wake-up is already pending.
    pub(crate) fn push(&self, event: Event) {
        self.lock().push_back(event);
        if !self.armed.swap(true, Ordering::AcqRel) {
            match self.ctx.get() {
                Some(ctx) => ctx.wake(0),
                None => self.armed.store(false, Ordering::Release),
            }
        }
    }

    /// Takes the queued events, clearing the wake-up first: an event pushed after this call
    /// sends a new wake-up.
    pub(crate) fn take(&self) -> VecDeque<Event> {
        self.armed.store(false, Ordering::Release);
        std::mem::take(&mut *self.lock())
    }

    /// Drops the queued `timer-fired` of a cancelled timer.
    pub(crate) fn remove_timer(&self, id: u64) {
        self.lock()
            .retain(|e| !matches!(e, Event::TimerFired(t) if *t == id));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl wit::types::Host for HostState {}

/// A linker with WASI (sync, empty context), the plugin imports, and the blocking poll calls
/// replaced by functions that stop the guest.
pub(crate) fn linker(engine: &wasmtime::Engine) -> wasmtime::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    bindings::Plugin::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    refuse_blocking(&mut linker)?;
    Ok(linker)
}

/// `wasi:io/poll`'s `poll` and `pollable.block` wait in host code, where the epoch deadline
/// cannot reach (`std::thread::sleep` in a guest uses them). They stop the guest instead.
fn refuse_blocking(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    const REASON: &str = "called a blocking function (wasi:io/poll); plugins may not block";
    linker.allow_shadowing(true);
    let mut poll = linker.instance("wasi:io/poll@0.2.12")?;
    poll.func_wrap(
        "poll",
        |mut store, _: (Vec<Resource<DynPollable>>,)| -> wasmtime::Result<(Vec<u32>,)> {
            store.data_mut().fault = Some(REASON.into());
            bail!(REASON)
        },
    )?;
    poll.func_wrap(
        "[method]pollable.block",
        |mut store, _: (Resource<DynPollable>,)| -> wasmtime::Result<()> {
            store.data_mut().fault = Some(REASON.into());
            bail!(REASON)
        },
    )?;
    linker.allow_shadowing(false);
    Ok(())
}

/// The store's resource limits, counted over all memories and tables of the store (wasmtime's
/// `StoreLimits` counts each memory on its own). Growing past a limit stops the guest.
#[derive(Default)]
pub(crate) struct Limits {
    memory_bytes: usize,
    table_elements: usize,
    /// Set when a limit stopped the guest.
    pub(crate) hit: Option<&'static str>,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = (self.memory_bytes - current.min(self.memory_bytes)).saturating_add(desired);
        if total > MAX_MEMORY_BYTES {
            self.hit = Some("exceeded the 64 MiB memory limit");
            bail!("plugin memory limit of {MAX_MEMORY_BYTES} bytes exceeded");
        }
        self.memory_bytes = total;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total =
            (self.table_elements - current.min(self.table_elements)).saturating_add(desired);
        if total > MAX_TABLE_ELEMENTS {
            self.hit = Some("exceeded the limit of 10 000 table elements");
            bail!("plugin table limit of {MAX_TABLE_ELEMENTS} elements exceeded");
        }
        self.table_elements = total;
        Ok(true)
    }

    fn instances(&self) -> usize {
        MAX_CORE_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_MEMORIES_AND_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES_AND_TABLES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tomakast_core::testing::FakeModule;
    use tomakast_core::{Core, LocalNow, RankWeights};

    /// Waits up to 2 s until `wakes` holds `n` entries, then 100 ms more for any extra one.
    fn wakes_settle(wakes: &Mutex<Vec<(u64, String)>>, n: usize) -> usize {
        let deadline = Instant::now() + Duration::from_secs(2);
        while wakes.lock().unwrap().len() < n && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(100));
        wakes.lock().unwrap().len()
    }

    #[test]
    fn inbox_sends_one_wake_up_until_taken() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tomakast_core::store::Store::open(&tmp.path().join("db")).unwrap();
        let now = LocalNow::from_unix_with_offset(1_800_000_000, 0);
        let mut core = Core::new(store, RankWeights::default(), now).unwrap();
        let fake = FakeModule::new(vec![], || Ok(vec![]));
        let (wakes, ctx) = (Arc::clone(&fake.wakes), Arc::clone(&fake.ctx));
        core.register(Box::new(fake));
        core.start_modules();
        let deadline = Instant::now() + Duration::from_secs(2);
        while ctx.lock().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the fake module never started");
            std::thread::sleep(Duration::from_millis(5));
        }

        let inbox = Inbox::default();
        // Before `connect` an event waits without a wake-up, and does not block later ones.
        inbox.push(Event::TimerFired(0));
        inbox.connect(ctx.lock().unwrap().clone().unwrap());
        for id in 1..=100 {
            inbox.push(Event::TimerFired(id));
        }
        assert_eq!(wakes_settle(&wakes, 1), 1, "one wake-up for 100 events");
        assert_eq!(inbox.take().len(), 101);
        // Taken: the next event wakes again.
        inbox.push(Event::TimerFired(101));
        assert_eq!(wakes_settle(&wakes, 2), 2);
        inbox.push(Event::TimerFired(102));
        assert_eq!(wakes_settle(&wakes, 2), 2, "not taken yet");
        inbox.remove_timer(101);
        let left: Vec<Event> = inbox.take().into();
        assert!(matches!(left[..], [Event::TimerFired(102)]), "{left:?}");
    }

    fn tile(widget: &str) -> HomeTile {
        HomeTile {
            widget_id: widget.into(),
            params: vec![],
        }
    }

    #[test]
    fn home_tiles_wait_in_one_slot_with_the_latest_list() {
        let inbox = Inbox::default();
        inbox.set_home(vec![tile("a")]);
        inbox.push(Event::TimerFired(1));
        inbox.set_home(vec![tile("b"), tile("c")]);
        let queued: Vec<Event> = inbox.take().into();
        assert!(
            matches!(queued[..], [Event::Home(_), Event::TimerFired(1)]),
            "one home event for two lists: {queued:?}"
        );
        let tiles = inbox.take_home().unwrap();
        let widgets: Vec<&str> = tiles.iter().map(|t| t.widget_id.as_str()).collect();
        assert_eq!(widgets, ["b", "c"]);
        assert!(inbox.take_home().is_none());
        // Delivered: the next list queues a new event.
        inbox.set_home(vec![]);
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox.take_home().map(|t| t.len()), Some(0));
    }

    const MIB: usize = 1024 * 1024;

    #[test]
    fn memory_limit_counts_every_memory() {
        let mut limits = Limits::default();
        // Two memories of 32 MiB reach the limit together; a third byte does not fit.
        assert!(limits.memory_growing(0, 32 * MIB, None).unwrap());
        assert!(limits.memory_growing(0, 32 * MIB, None).unwrap());
        assert!(limits.memory_growing(0, 1, None).is_err());
        assert!(limits.hit.is_some());
    }

    #[test]
    fn memory_growth_counts_the_delta() {
        let mut limits = Limits::default();
        assert!(limits.memory_growing(0, MIB, None).unwrap());
        assert!(limits.memory_growing(MIB, 64 * MIB, None).unwrap());
        assert!(limits.hit.is_none());
        assert!(
            limits
                .memory_growing(64 * MIB, 64 * MIB + 65536, None)
                .is_err()
        );
    }

    #[test]
    fn table_limit_counts_every_table() {
        let mut limits = Limits::default();
        assert!(limits.table_growing(0, 6_000, None).unwrap());
        assert!(limits.table_growing(0, 4_000, None).unwrap());
        assert!(limits.table_growing(0, 1, None).is_err());
    }
}
