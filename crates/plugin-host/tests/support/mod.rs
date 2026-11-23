//! Shared helpers for the plugin-host integration tests: building guest plugins and running
//! their exports before the real bindings exist.
#![allow(dead_code)]

pub mod harness;
pub mod server;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Instance, Linker, ResourceTable, Val};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// The test copy of the compile child (`tomakast compile-plugin` without the app).
pub fn compile_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tomakast-compile-plugin"))
}

/// Builds `plugins/<name>` (once per test process) and returns `plugin.wasm` in a fresh plugin
/// folder `<tmp>/<name>/` that also holds the plugin's `plugin.toml` and its `icons/`.
pub fn guest(name: &str) -> PathBuf {
    let built = built_wasm(name);
    let folder = fixtures_root()
        .join(format!("{}", next_fixture()))
        .join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let source = workspace_root().join("plugins").join(name);
    std::fs::copy(source.join("plugin.toml"), folder.join("plugin.toml")).unwrap();
    // Image files the plugin names as `plugin-file`: weather's icons, translator's flags.
    for dir in ["icons", "flags"] {
        if let Ok(files) = std::fs::read_dir(source.join(dir)) {
            std::fs::create_dir_all(folder.join(dir)).unwrap();
            for file in files {
                let file = file.unwrap();
                std::fs::copy(file.path(), folder.join(dir).join(file.file_name())).unwrap();
            }
        }
    }
    let wasm = folder.join("plugin.wasm");
    std::fs::copy(built, &wasm).unwrap();
    wasm
}

fn built_wasm(name: &str) -> PathBuf {
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let mut built = BUILT.get_or_init(Default::default).lock().unwrap();
    if let Some(path) = built.get(name) {
        return path.clone();
    }
    let root = workspace_root();
    let target_dir = root.join("target/plugins");
    let manifest = root.join("plugins").join(name).join("Cargo.toml");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    // The guest is built with the profile of these tests: unoptimized under `cargo test`,
    // optimized under `cargo test --release` (where the timing tests run).
    let (profile_args, profile_dir): (&[&str], _) = if cfg!(debug_assertions) {
        (&[], "debug")
    } else {
        (&["--release"], "release")
    };
    let output = Command::new(cargo)
        // The workspace root, so that rustup applies `rust-toolchain.toml` (wasm32-wasip2).
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .arg("build")
        .args(profile_args)
        .args(["--target", "wasm32-wasip2", "--manifest-path"])
        .arg(&manifest)
        .output()
        .expect("run cargo for the guest build");
    assert!(
        output.status.success(),
        "building guest {name} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wasm = target_dir
        .join("wasm32-wasip2")
        .join(profile_dir)
        .join(format!("{}.wasm", name.replace('-', "_")));
    assert!(wasm.is_file(), "guest build produced no {}", wasm.display());
    built.insert(name.to_owned(), wasm.clone());
    wasm
}

/// A per-process folder for plugin fixtures. It is not removed at exit (statics are never
/// dropped); it lives in the system temp dir.
fn fixtures_root() -> &'static Path {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    ROOT.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

fn next_fixture() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Store state with an empty WASI context: no args, env, files, stdio or sockets.
pub struct State {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// A store for `engine` with an epoch deadline of 400 ticks (2 s while a ticker runs), enough
/// to instantiate. It is finite on purpose: a call that is not armed through
/// `Epoch::enter_store` still traps, after about 2 s, instead of spinning forever.
pub fn store(engine: &Engine) -> Store<State> {
    let mut store = Store::new(
        engine,
        State {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
        },
    );
    store.set_epoch_deadline(400);
    store
}

/// Instantiates with WASI p2 (sync) and the plugin's own imports as traps, until the real
/// bindings exist.
pub fn instantiate(engine: &Engine, store: &mut Store<State>, component: &Component) -> Instance {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).unwrap();
    linker.define_unknown_imports_as_traps(component).unwrap();
    linker.instantiate(store, component).unwrap()
}

/// Calls the `search` export through the dynamic API, with no readings.
pub fn search(store: &mut Store<State>, instance: &Instance, query: &str) -> wasmtime::Result<()> {
    let func = instance
        .get_func(&mut *store, "search")
        .expect("search export");
    let mut results = [Val::Bool(false)];
    func.call(
        &mut *store,
        &[Val::String(query.into()), Val::List(Vec::new())],
        &mut results,
    )
}
