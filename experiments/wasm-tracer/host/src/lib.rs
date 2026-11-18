//! Host side of the WASM tracer bullet (plan 2b-1, Task 1). Not product code.
//!
//! One `Store` per plugin, one instance per store, all calls synchronous.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasmtime::Result;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::error::Context;
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    world: "tracer",
    path: "../wit",
});

/// A search result as the guest returns it.
pub type SearchItem = Item;

/// Per-store state: an empty WASI context (no args, env, files or sockets) and the limiter.
pub struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Path of the guest component built by
/// `cargo build --release --target wasm32-wasip2 --manifest-path experiments/wasm-tracer/guest/Cargo.toml`.
pub fn guest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../guest/target/wasm32-wasip2/release/tracer_guest.wasm")
}

/// Reads the guest component's bytes.
pub fn guest_bytes() -> Result<Vec<u8>> {
    let path = guest_path();
    std::fs::read(&path).with_context(|| format!("reading guest component {}", path.display()))
}

/// The engine configuration used everywhere: defaults, plus epoch interruption when asked.
pub fn make_config(epoch: bool) -> Config {
    let mut config = Config::new();
    config.epoch_interruption(epoch);
    config
}

/// A fresh engine; `epoch` turns on epoch interruption (a compile-time setting).
pub fn make_engine(epoch: bool) -> Engine {
    Engine::new(&make_config(epoch)).expect("engine config is valid")
}

/// Compiles the guest from its bytes (`Component::new`).
pub fn load_component(engine: &Engine) -> Result<Component> {
    Component::new(engine, guest_bytes()?)
}

/// A linker with the synchronous WASI p2 imports the `wasm32-wasip2` std guest needs.
pub fn make_linker(engine: &Engine) -> Linker<State> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).expect("WASI p2 links");
    linker
}

/// Per-plugin store settings.
#[derive(Clone, Copy, Debug, Default)]
pub struct PluginOptions {
    /// `StoreLimits::memory_size` in bytes; `None` means no limiter.
    pub memory_limit: Option<usize>,
    /// `StoreLimitsBuilder::trap_on_grow_failure`: trap in `memory.grow` instead of returning -1.
    pub trap_on_grow_failure: bool,
    /// Epoch ticks each call may run for; required when the engine has epoch interruption on.
    pub epoch_deadline_ticks: Option<u64>,
}

/// One plugin: its own `Store` and one reused instance.
pub struct Plugin {
    store: Store<State>,
    bindings: Tracer,
    options: PluginOptions,
}

impl Plugin {
    pub fn new(
        engine: &Engine,
        component: &Component,
        linker: &Linker<State>,
        options: PluginOptions,
    ) -> Result<Self> {
        let mut limits = StoreLimitsBuilder::new();
        if let Some(bytes) = options.memory_limit {
            limits = limits
                .memory_size(bytes)
                .trap_on_grow_failure(options.trap_on_grow_failure);
        }
        let state = State {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            limits: limits.build(),
        };
        let mut store = Store::new(engine, state);
        if options.memory_limit.is_some() {
            store.limiter(|state| &mut state.limits);
        }
        // With epoch interruption on, the initial deadline is 0, so instantiation needs one too.
        if let Some(ticks) = options.epoch_deadline_ticks {
            store.set_epoch_deadline(ticks);
        }
        let bindings = Tracer::instantiate(&mut store, component, linker)?;
        Ok(Self {
            store,
            bindings,
            options,
        })
    }

    /// Resets the epoch deadline so each call gets the full budget.
    fn before_call(&mut self) {
        if let Some(ticks) = self.options.epoch_deadline_ticks {
            self.store.set_epoch_deadline(ticks);
        }
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<SearchItem>> {
        self.before_call();
        self.bindings.call_search(&mut self.store, query)
    }

    /// Calls the infinite loop; returns the call's result and how long it ran.
    pub fn spin(&mut self) -> (Result<()>, Duration) {
        self.before_call();
        let start = Instant::now();
        let result = self.bindings.call_spin(&mut self.store);
        (result, start.elapsed())
    }

    pub fn hog(&mut self) -> Result<()> {
        self.before_call();
        self.bindings.call_hog(&mut self.store)
    }
}

/// A thread that calls `Engine::increment_epoch` every `period` until dropped.
pub struct Ticker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Ticker {
    pub fn start(engine: &Engine, period: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let engine = engine.clone();
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(period);
                engine.increment_epoch();
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// A WIT package whose record refers to itself. Kept out of `wit/` so `bindgen!` still works.
pub const RECURSIVE_WIT: &str = r#"package tomakast:recursive@0.1.0;

interface tree {
  record node {
    label: string,
    children: list<node>,
  }
}

world recursive {
  export tree;
}
"#;

/// Parses one WIT package from `source`; returns the parser's error text if WIT rejects it.
pub fn parse_wit(source: &str) -> std::result::Result<(), String> {
    let mut resolve = wit_parser::Resolve::default();
    resolve
        .push_str("probe.wit", source)
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}

/// Resident set size of this process in KiB, from `/proc/self/status`.
pub fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

/// Median and p95 (nearest rank) of a sample.
pub fn median_p95(samples: &mut [Duration]) -> (Duration, Duration) {
    samples.sort_unstable();
    let rank = |p: f64| {
        let index = ((p * samples.len() as f64).ceil() as usize).max(1) - 1;
        samples[index.min(samples.len() - 1)]
    };
    (rank(0.5), rank(0.95))
}
