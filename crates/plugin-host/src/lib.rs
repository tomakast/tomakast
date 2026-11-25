//! WASM plugin host: manifest, package reading and hashing, plugin storage, the wasmtime
//! engine, the out-of-process compile child, the precompiled cache, the epoch ticker, and the
//! plugin imports (storage, preferences, log, open, HTTP, timers, locale), hash-bound approval,
//! the checks of what widgets send, and the built-in `plugins` review module.
//!
//! This crate does not depend on GPUI.
//!
//! # wasmtime features
//!
//! `wasmtime` 50 with `default-features = false` and `runtime, std, anyhow, component-model,
//! cranelift, parallel-compilation`; `wasmtime-wasi` 50 with only `p2`. Not enabled: `gc*`,
//! `cache`, `wat`, `profiling`, `coredump`, `debug`, `pooling-allocator`, `threads`,
//! `component-model-async`. One exception is forced: `wasmtime-wasi/p2` enables
//! `wasmtime/async` (and pulls in `tokio`); the host never uses async calls. Wasm GC is also
//! switched off in [`engine_config`].
//!
//! Size, measured at Task 5 (2026-11-19, Linux x86_64, release, `strip`): the `tomakast` binary
//! grew from 48.26 MiB to 60.71 MiB, **+12.45 MiB**. At this point the binary only reaches the
//! compile child, so the linker drops the WASI and instantiation code: this is a lower bound.
//! Re-measured at Task 9 (2026-11-19, same setup), with the daemon loading plugins: 67.57 MiB,
//! **+19.31 MiB** over the binary without wasmtime.

pub mod approval;
mod bindings;
pub mod budget;
pub mod cache;
pub mod caps;
pub mod catalog;
pub mod compile;
mod convert;
pub mod db;
pub mod engine;
pub mod epoch;
mod imports;
pub mod location_sources;
pub mod manager;
pub mod manifest;
pub mod package;
pub mod plugin;
pub mod plugins_module;
#[cfg(any(test, feature = "testing"))]
mod recorder;
pub mod watch;
pub mod widget_caps;

pub use approval::{Decision, Gate, PluginLinkPolicy, ReviewReason, ReviewState, check, decide};
pub use cache::Cache;
pub use catalog::{catalog_entry, list_catalog};
pub use compile::compile_in_child;
pub use db::{Grant, GrantDecision, KvError, PluginDb, WidgetGrant};
pub use engine::{HostEngine, api_version, check_api_version, engine_config};
pub use epoch::{Epoch, EpochGuard};
#[cfg(any(test, feature = "testing"))]
pub use imports::http::Responder;
pub use imports::location::{Fix, LocationSource, Locator, Source};
pub use imports::open::{Opener, PlatformOpener};
pub use manager::{ManagerAction, ManagerStats, ManagerThread, PluginManager};
pub use manifest::{
    CommandDecl, DomainPattern, MAX_WIDGET_PARAMS, Manifest, ManifestError, ParamEntry,
    Permissions, PrefKind, PreferenceDecl, RESERVED_IDS, RESERVED_PARAM_KEYS,
    SUPPORTED_API_VERSIONS, WidgetEntry,
};
pub use package::{MAX_WASM_BYTES, Package, PackageError};
pub use plugin::{PluginEvents, PreferencesHandle, WasmPlugin};
pub use plugins_module::{PluginsHandle, PluginsModule};
#[cfg(any(test, feature = "testing"))]
pub use recorder::Recorder;
