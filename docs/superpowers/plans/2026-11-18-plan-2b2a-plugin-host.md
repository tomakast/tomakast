# Plan 2b-2a: WASM plugin host Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A third-party WASM component in `<data-dir>/plugins/<id>/` loads without rebuilding the app, after the user approves its permissions. It runs sandboxed with deadlines, memory limits, quotas and an empty WASI, and reloads when its folder changes. A kitchen-sink test plugin proves every host capability and the threat model.

**Architecture:**
- New crate `crates/plugin-host`. Each plugin is a `WasmPlugin` that implements the core's `Module` trait, so it runs on its own module worker thread like a native module.
- wasmtime 50 runs **sync**: one `Store` per plugin, owned by that worker. This departs from the spec's "wasmtime async + background executor", following the tracer ruling (`docs/experiments/2026-11-wasm-tracer.md`, "Sync or async").
- Components are compiled in a short-lived child process (`tomakast compile-plugin`), so Cranelift's memory never stays in the daemon. The daemon only deserializes from its own cache.
- Timers and HTTP finish on host threads and come back through a new core route, "wake the module's worker", as unsolicited effects.
- The WIT 0.1 contract gets its pre-freeze edits first. The freeze itself, the guest SDK and `calc` are plan 2b-2b.
- The API version comes from the manifest's `api` field, not yet from a custom section in the `.wasm` as the spec says (Zed's scheme). The custom section arrives with the guest SDK in plan 2b-2b, which emits it; the host then requires both to agree.
- Timers do not survive an unload: they end with the plugin instance (Task 7), and a plugin re-arms them in `init`. The spec says the plugin's timers survive the unload; the WIT `timers` doc states the plan's rule.

**Tech Stack:**
- Rust 2024, 1.98;
- `wasmtime` and `wasmtime-wasi` 50 (sync, component model, epoch interruption, `StoreLimits`);
- `ureq` 3 (rustls) for host HTTP;
- `sha2` 0.10 (already in `Cargo.lock`);
- `notify` 8 (already used);
- `rusqlite` (WAL);
- `wit-parser` 0.261.

**Spec:** `docs/superpowers/specs/2026-11-18-tomakast-core-design.md`. Sections: Plugin API (WIT), Plugin manifest and installation, Threat model and plugin security, Storage and settings, Performance budgets. Inputs: `docs/superpowers/plans/2026-11-18-plan-2b1-followups.md` (I3–I5, W1–W12, M5, deferred minors), `docs/experiments/2026-11-wasm-tracer.md`, `docs/superpowers/plans/2026-11-18-visual-refine-followups.md` (per-action shortcuts).

## Global Constraints

- Plugins are potentially malicious:
  - a plugin gets nothing beyond the manifest permissions the user approved;
  - WASI has no filesystem, environment, args, stdio or sockets;
  - nothing from a plugin is instantiated, and `init` is not called, before approval.
- Approval is bound to `sha256(plugin.toml ‖ 0x00 ‖ plugin.wasm)`:
  - a grown permission set asks again, and so does a changed `name` or `author` (Ruling AJ);
  - an equal or smaller set is approved silently, and the new hash is recorded.
- The files of a plugin are read once per load; the same bytes are hashed, compiled and instantiated.
- The daemon never deserializes a `.cwasm` from a plugin folder, only from `<data-dir>/cache/plugins/`. Cache entries are written only by the compile child, keyed by the wasm sha256 and `Engine::precompile_compatibility_hash`.
- Deadlines are enforced by epoch interruption: `search` 100 ms, `init` 2000 ms, `run` and `on-event` 1000 ms. A trap or an exceeded limit marks the plugin broken, shows an error toast and unloads it. The core keeps running.
- Memory: at most 64 MiB of linear memory per plugin store (`StoreLimits`).
- The epoch ticker thread is parked while no plugin call is in flight. This is the spec's "~0% CPU in the background".
- Quotas per plugin:
  - storage: 1 MiB in total, keys ≤ 256 B, values ≤ 256 KiB;
  - index: ≤ 1000 items;
  - toasts: ≤ 10 per 10 s;
  - unsolicited view updates: ≤ 30 per s;
  - timers: ≤ 16 active, repeat interval ≥ 250 ms, one-shot delay ≥ 10 ms;
  - HTTP: ≤ 4 concurrent requests, response body ≤ 4 MiB, timeout 30 s, ≤ 5 redirects. Every hop must match the allowlist.
- View caps (host side, before the UI sees a view):
  - list rows ≤ 2000, grid tiles ≤ 500, form fields ≤ 50;
  - markdown ≤ 64 KiB, scratchpad text ≤ 256 KiB;
  - any title, subtitle, label or accessory ≤ 1 KiB;
  - any id ≤ 256 B with no control characters;
  - actions per row ≤ 16.

  An oversized effect is rejected as a whole with an error toast.
- `open`:
  - URL schemes `http`, `https` and `mailto` only;
  - paths only to existing files and folders that are not executable. Linux: no exec bit and no `.desktop`; macOS: no `.app` or exec bit; Windows: not `.exe .bat .cmd .ps1 .lnk .msi .com .scr .vbs .js`.
- Specs, plans and docs in English. Agents never show a window that grabs the keyboard (`TOMAKAST_NO_KEYBOARD=1` and a throwaway `TOMAKAST_HOME` for dev runs).
- Built-in modules stay trusted and keep platform access. The `Module` trait changes in Tasks 1–2 are the last trait changes of this plan; native modules being built in parallel rebase once on them.

## Review Focus

- **A plugin folder that is still being copied** (the wasm is half-written when the watcher fires): no load is attempted until the folder has been quiet for 500 ms. A failed load is retried on the next change, never in a loop. Test in Task 9.
- **The same plugin updated while one of its views is open:** the old instance's view stays on the stack but its events go nowhere (an error toast "plugin reloaded"), with no panic. A new view from the new instance works. Test in Task 9.
- **A plugin that answers every keystroke slowly** (90 ms, under the deadline): typing stays responsive and stale answers are dropped by generation, as for native modules. Test in Task 6.
- **Clock or epoch edge:** a call that starts while the ticker is parked still gets its full deadline, not a deadline that is already expired. Test in Task 5.
- **An approval decision for a plugin id whose folder was deleted and later re-created with different content:** it re-prompts, because approval is bound to the hash. Test in Task 8.

---

### Task 1: Core plumbing for plugins

**Files:**
- Modify: `crates/core/src/module.rs`, `crates/core/src/host.rs`, `crates/core/src/core.rs`, `crates/core/src/engine.rs`, `crates/core/src/testing.rs`, `modules/apps/src/lib.rs`, `modules/demo/src/lib.rs`, `crates/ui/src/window.rs` (the `core_update` match)
- Test: `crates/core/tests/host.rs` (new file or the existing host tests), `crates/core/src/engine.rs` tests

**Interfaces:**
- Produces:
  - `Module::id(&self) -> &str`, owned by the module (was `&'static str`). `ModuleHost` keys workers by `String`; the worker thread name stays `module-<id>`.
  - `Core::register(m)`: registering after `start_modules` starts the worker at once. Registering an id that is running replaces it: the old worker exits after its current call, and its index items are dropped.
  - `Core::unregister(provider: &str)` stops the worker and removes the provider's index items and dynamic answers.
  - `ModuleContext::wake(&self, token: u64)` queues a wake-up on this module's own worker, which then calls `Module::on_wake(&mut self, token: u64) -> anyhow::Result<Vec<Effect>>` (default `Ok(vec![])`).
  - `ModuleContext::emit(&self, effects: Vec<Effect>)` sends effects from any thread.
  - Both routes end in `CoreEvent::ModuleEffects { provider: String, effects: Vec<Effect> }`, which becomes `CoreUpdate::ModuleEffects { provider, effects, items_changed }`. The core applies `ReplaceItems` itself, as for replies.
  - `Hit` order for dynamic items: equal scores keep the provider's list order, not title order (I5/W3).
  - A dynamic item whose id equals an indexed item's id is dropped from the dynamic answer, with a warning (deferred T2 minor).

- [ ] **Step 1:** Write failing tests:
  - `module_with_runtime_id_runs`: a `FakeModule` built with `String` id `"plug-a"` answers an action;
  - `register_after_start_starts_worker`;
  - `register_same_id_replaces_and_drops_items`: the old items are gone, the new module's `start` ran;
  - `unregister_removes_items_and_dynamic_answers`;
  - `wake_calls_on_wake_on_the_worker_thread`: `on_wake` records `thread::current().name() == Some("module-fake")` and the token;
  - `emit_from_other_thread_arrives_as_module_effects`;
  - `module_effects_apply_replace_items` (`items_changed == true`, no `ReplaceItems` left in `effects`);
  - `dynamic_ties_keep_provider_order`: answers `["b","a","c"]` with equal usage come out b, a, c. Replace the old `hung_module_search_misses_deadline` assertion "falls back to the title order" accordingly;
  - `dynamic_id_equal_to_static_id_is_dropped`.
- [ ] **Step 2:** Run `cargo test -p tomakast-core` and see them fail (compile errors count as RED only for the signature change; the behavioural ones must fail on assertions).
- [ ] **Step 3:** Implement. `FakeModule` gains an owned id and an `on_wake` hook. `apps` and `demo` return their ids as `&str`. The UI's `core_update` gets a `CoreUpdate::ModuleEffects` arm that logs and drops the effects; Task 3 implements the target rule.
- [ ] **Step 4:** Run `cargo test --workspace`, clippy and fmt; all green. Commit `feat(core): runtime module ids, unregister, wake and unsolicited effects`.

### Task 2: WIT 0.1 pre-freeze edits and the core view model

**Files:**
- Modify: `crates/plugin-api/wit/since_v0.1.0/plugin.wit`, `crates/core/src/view.rs`, `crates/core/src/effect.rs`, `crates/core/src/index.rs`, the parity test in `crates/plugin-api` (or wherever `wit_parity` lives), `modules/demo/src/lib.rs`

**Interfaces:**
- Produces (core types, mirrored in WIT with the same names in kebab-case):
  - `Action { id, title, shortcut: Option<String> }`, where `shortcut` uses the `Hotkey::parse` syntax (`ctrl+shift+c`);
  - `FormField { id, label, kind, error: Option<String> }` (W4);
  - `Icon::{Named, File, Emoji(String), Color(String)}`. `Color` is `#rrggbb`. `Emoji` is at most 32 bytes (Ruling O). WIT: `icon { named, plugin-file, emoji, color }` (W9);
  - `ViewEvent::Closed`, sent when a view is popped or the window hides; it carries nothing (W5);
  - `Item::source: Option<String>`, a label the UI shows as the row accessory instead of the kind label. The host sets it to the plugin's manifest name, and plugins cannot set it (spec: "the source of every item is visible").

**WIT changes (each ruling is written into `plugin.wit` as a doc comment):**
- **W1:** `event` gets `view(view-target)` with `record view-target { view-id: string, event: view-event }`, and `on-event: func(event: event)`.
- **W2:** `run: func(item: item, action-id: string)`. The host passes the whole item the user chose, including dynamic ones.
- **W3:** dynamic results keep the provider's order (Task 1); `item` gets no rank.
- **W4, W5, W9:** as in Interfaces above.
- **W6:** the behavioural rules from the follow-ups file move into doc comments, verbatim in meaning:
  - the revision rule;
  - form updates keep the fields the user touched;
  - `action-chosen.row-id` is the tile id in a grid and the view id in a detail view;
  - `pop-view` pops only the replying view;
  - `line` is 0-based and the last annotation per line wins;
  - `columns: 0` means 1;
  - toast lifetimes are 3 s success, 5 s error, and progress until replaced.
- **W7:** document "rows of one section are contiguous; a row with `section: none` continues the previous section". The renderer is unchanged.
- **W8:** remove the `index`, `clipboard` and `toast` import interfaces. Effects are the only path, and the host checks permissions and quotas on effects (Task 7). `init: func() -> result<list<effect>, string>`.
- **W10:** state every view cap and quota from the Global Constraints in the doc comments of the records they bound.
- **W11:** unsolicited effects:
  - `update-view` targets the topmost stack entry with that view id owned by the plugin, and is dropped if there is none;
  - `push-view` is honoured only while the launcher is open and the top view belongs to the plugin;
  - `pop-view` and `close-window` are ignored;
  - `toast` shows only while the window is open;
  - `copy-text` applies only during a call the user started: `run`, `action-chosen` or `form-submitted`, not `query-changed`, `text-changed` or `closed` (Rulings X and AL); unsolicited `replace-items` applies at most once per 5 s (Ruling AD).
- **W12:** `search: func(query: string) -> result<list<item>, string>`. The package doc comment moves above `package` and describes the world.
- **Kind:** document that the host sets the kind to `command` and the source to the plugin name.
- **Lifecycle:** document that the core may unload an idle plugin and reload it, so a plugin keeps no state it cannot restore from `storage`. (Unloading idle plugins is not built in 0.1.)
- **M5:** document that links in a plugin's markdown open only if the plugin holds the `open` permission (Task 8 enforces it).

- [ ] **Step 1:** Update the parity test first so it fails. It is table-driven over record, variant and enum names; add field-type checks for the changed records (`action`, `form-field`, `icon`, `event`, `view-event`, `item`, `effect`) and the export signatures.
- [ ] **Step 2:** Edit `plugin.wit` and the core types until the parity test passes. `apps` icons stay `Named`/`File`. The demo gets:
  - a form field with an `error`;
  - one action with `shortcut: "ctrl+shift+c"`;
  - a grid with `Emoji` and `Color` tiles.

  Demo tests change in the same commit.
- [ ] **Step 3:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(wit): WIT 0.1 pre-freeze edits (W1–W12, shortcuts, source)`.

### Task 3: UI for the new view model and unsolicited effects

**Files:**
- Modify: `crates/ui/src/stack.rs`, `crates/ui/src/launcher_view.rs`, `crates/ui/src/views/{form,grid,action_panel,scratchpad,mod}.rs`, `crates/ui/src/markdown.rs`, `crates/ui/src/theme.rs`
- Test: `crates/ui/tests/stack.rs`, `crates/ui/tests/launcher_view.rs`

**Interfaces:**
- Consumes: Task 1 `CoreUpdate::ModuleEffects`, Task 2 types.
- Produces:
  - `LauncherController::core_update` applies `ModuleEffects` with the W11 rule;
  - `ViewEvent::Closed` is sent for every popped view and for every stack entry when the window hides;
  - `trait LinkPolicy { fn may_open_links(&self, provider: &str) -> bool }`, held by the controller. The default implementation returns `true`; Task 8 installs the plugin-aware one.

- [ ] **Step 1:** Write failing tests:
  - `unsolicited_update_targets_topmost_matching_view`;
  - `unsolicited_update_without_target_is_dropped`;
  - `unsolicited_push_needs_open_window_and_owner_on_top`;
  - `unsolicited_close_and_pop_are_ignored`;
  - `pop_flushes_pending_scratchpad_text_then_sends_closed` (I3: the pending debounced `TextChanged` arrives before `Closed`);
  - `hide_flushes_and_closes_every_stack_entry`;
  - `form_field_error_is_rendered` (a test-support id `field-error-<id>` with the text);
  - `action_shortcut_runs_action_and_shows_chips`: `ctrl+shift+c` on the selected row runs that action, and the panel shows the chips;
  - `emoji_and_color_icons_render`;
  - `item_source_replaces_kind_label`;
  - `links_disabled_when_policy_denies`.
- [ ] **Step 2:** Implement. Rendering details:
  - the shortcut chips reuse `chrome::key_chip`;
  - the emoji renders as text at `ICON_SIZE`;
  - the color is a rounded `ICON_SIZE` square;
  - the form error is 12 px in `palette.error` under the field.

  All new values go into `theme.rs`. A shortcut that collides with a launcher key (Enter, Esc, Up/Down, Ctrl+K) is ignored with a warning when the view is pushed.
- [ ] **Step 3:** Run the tests, clippy and fmt. Take dark screenshots of the demo form, grid and action panel (`TOMAKAST_NO_KEYBOARD=1`, throwaway home; never commit screenshots). Commit `feat(ui): unsolicited effects, view lifecycle, shortcuts, new icons`.

### Task 4: Plugin host crate, manifest and storage v2

**Files:**
- Create: `crates/plugin-host/{Cargo.toml,src/lib.rs,src/manifest.rs,src/package.rs,src/db.rs}`
- Modify: root `Cargo.toml` (member + workspace deps `wasmtime`, `wasmtime-wasi`, `ureq`, `sha2`), `crates/core/src/store.rs` (schema v2)
- Test: unit tests in each new file; `crates/core/src/store.rs` tests

**Interfaces:**
- Produces:
  - `Manifest::parse(text: &str, folder_name: &str) -> Result<Manifest, ManifestError>`;
  - `struct Manifest { id, name, version, api: String, author: Option<String>, commands: Vec<CommandDecl>, preferences: Vec<PreferenceDecl>, permissions: Permissions }`;
  - `CommandDecl { id, title, icon: Option<String> }` (no `mode`: Ruling AP);
  - `PreferenceDecl { key, title, kind: PrefKind::{Text, Bool}, default: Option<String> }`;
  - `Permissions { storage: bool, clipboard: bool, open: bool, timers: bool, index: bool, search: bool, http: Vec<DomainPattern> }` (`search` replaces the top-level `dynamic` key: Ruling AJ), with `Permissions::is_subset_of(&self, other: &Permissions) -> bool` and `Permissions::describe(&self) -> Vec<String>` (plain-language lines for the prompt);
  - `Package::read(dir: &Path) -> Result<Package, PackageError>`, which reads `plugin.toml` and `plugin.wasm` once into memory, with `Package { manifest, wasm: Arc<[u8]>, content_hash: [u8; 32], wasm_hash: [u8; 32] }`;
  - `PluginDb::open(path: &Path) -> anyhow::Result<PluginDb>`: its own WAL connection to the same file;
  - `PluginDb` methods `kv_get`, `kv_set`, `kv_remove`, `kv_usage(plugin)`, `grant(plugin) -> Option<Grant>`, `set_grant(&Grant)`;
  - schema v2 in core `Store::init` (`user_version` 1 → 2), with tables:
    - `plugin_kv(plugin_id, key, value, updated_at, PRIMARY KEY(plugin_id, key))`;
    - `plugin_grants(plugin_id PRIMARY KEY, content_hash BLOB, permissions TEXT, decision TEXT CHECK(decision IN ('allow','deny')), decided_at, updated_at)`.

**Manifest format (`plugin.toml`):**

```toml
id = "kitchen-sink"   # [a-z0-9-]{1,32}, must equal the folder name
name = "Kitchen Sink" # ≤ 64 chars
version = "0.1.0"
api = "0.1.0"         # must be a supported WIT version
author = "tomakast"

[[commands]]
id = "open"           # [a-z0-9-]{1,32}; indexed as `<plugin-id>/<command-id>`, run with action `run`
title = "Kitchen Sink"
icon = "face-smile"   # optional theme icon name; follows the id rule

[[preferences]]
key = "greeting"
title = "Greeting"
kind = "text"         # or "bool"
default = "hello"

[permissions]
storage = true
clipboard = true
open = true
timers = true
index = true
search = true         # the host calls `search` with what the user types (Ruling AJ)
http = ["example.com", "*.example.org"]
```

- [ ] **Step 1:** Write failing tests:
  - `manifest_parses_full_example`;
  - `manifest_rejects_id_mismatch_with_folder`;
  - `manifest_rejects_bad_id_chars`;
  - `manifest_rejects_unknown_api_version`;
  - `manifest_rejects_unknown_permission_key`;
  - `domain_pattern_matches` (`*.example.org` matches `a.example.org` but neither `example.org` nor `evil-example.org`; plain `example.com` matches only itself; IDNs are compared lowercase ASCII/punycode);
  - `permissions_subset`;
  - `package_hash_covers_both_files` (changing one byte of either file changes `content_hash`);
  - `package_rejects_missing_wasm`;
  - `package_rejects_wasm_over_32_mib`;
  - `store_migrates_v1_to_v2_keeping_usage`;
  - `kv_quota_accounting`;
  - `grant_roundtrip`.
- [ ] **Step 2:** Implement. The crate depends on `tomakast-core` and not on GPUI. Pick the `wasmtime` features by building (start from the tracer's `compiler` set + `component-model`), and record the chosen set and the stripped size delta in the crate doc comment.
- [ ] **Step 3:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(plugin-host): manifest, package hashing and plugin storage schema`.

### Task 5: Engine, compile child, cache and guest test harness

**Files:**
- Create: `crates/plugin-host/src/{engine.rs,compile.rs,cache.rs,epoch.rs}`, `crates/plugin-host/tests/support/mod.rs`, `plugins/kitchen-sink/{Cargo.toml,plugin.toml,src/lib.rs}`
- Modify: `crates/app/src/cli.rs` (hidden subcommand), `.gitignore` if needed

**Interfaces:**
- Produces:
  - `HostEngine::new() -> anyhow::Result<HostEngine>`: one `wasmtime::Engine` with epoch interruption on, wasm GC off, and the same `Config` the compile child uses (`fn engine_config() -> wasmtime::Config` shared by both);
  - `Cache::new(dir: PathBuf)`, `Cache::path_for(wasm_hash: &[u8;32], engine: &Engine) -> PathBuf`;
  - `compile_in_child(exe: &Path, wasm: &[u8], wasm_hash: [u8;32], out: &Path) -> anyhow::Result<()>`:
    - writes the bytes to a temp file in the cache dir;
    - runs `<exe> compile-plugin --input <tmp> --sha256 <hex> --output <out>`;
    - the child re-hashes the input, refuses a mismatch, precompiles and writes `<out>.tmp`, then renames it;
    - timeout 60 s;
  - `HostEngine::load(&self, wasm_hash, wasm: &[u8], cache: &Cache, exe: &Path) -> anyhow::Result<Component>`: deserializes the cache entry, compiling in the child first on a miss. It never deserializes a file from outside `cache.dir`;
  - `Epoch::global() -> &'static Epoch`, `Epoch::enter(&self, deadline: Duration) -> EpochGuard`:
    - a ticker thread with a 5 ms tick, parked on a condvar while the in-flight count is 0;
    - `enter` increments the count and sets the store deadline to `ceil(deadline / 5 ms) + 1` ticks from the current epoch;
  - `tests/support::guest(name: &str) -> PathBuf`:
    - builds `plugins/<name>` once per test process (`OnceLock`) with `cargo build --release --target wasm32-wasip2 --manifest-path plugins/<name>/Cargo.toml`;
    - `CARGO_TARGET_DIR=<workspace>/target/plugins`;
    - returns the `.wasm` path and copies `plugin.toml` next to it in a temp plugin folder;
  - `plugins/kitchen-sink` is a guest crate (own empty `[workspace]`, `wit-bindgen 0.63`, `crate-type = ["cdylib"]`) implementing the world. Its exports start minimal, and later tasks add the behaviours their tests need, each behaviour selected by command id or query.

- [ ] **Step 1:** Write failing tests:
  - `cache_entry_is_written_by_child_and_deserialized`;
  - `child_refuses_hash_mismatch`;
  - `load_never_reads_outside_cache_dir` (a `.cwasm` dropped into the plugin folder is ignored, and the source wasm is compiled instead);
  - `cache_key_changes_with_engine_config` (a mismatching compat hash means a miss);
  - `epoch_ticker_parks_when_idle` (no tick advance over 200 ms idle);
  - `deadline_counts_from_call_start_after_idle` (Review Focus: after 1 s idle, a 100 ms deadline call that spins traps between 100 and 130 ms);
  - `daemon_rss_does_not_include_cranelift` (in-process RSS after `load` of a cold component grows by < 8 MiB: compile happened in the child).
- [ ] **Step 2:** Implement. The `compile-plugin` subcommand is hidden (`#[command(hide = true)]`) and does nothing but compile.
- [ ] **Step 3:** Run `cargo test --workspace` (the guest build is part of the plugin-host tests), clippy and fmt. Commit `feat(plugin-host): engine, out-of-process compile, cache, epoch ticker`.

### Task 6: `WasmPlugin`: bindings, calls, limits, storage, preferences, log

**Files:**
- Create: `crates/plugin-host/src/{bindings.rs,convert.rs,plugin.rs,imports/mod.rs,imports/storage.rs,imports/preferences.rs,imports/log.rs,caps.rs}`
- Modify: `plugins/kitchen-sink/src/lib.rs`
- Test: `crates/plugin-host/tests/plugin.rs`

**Interfaces:**
- Consumes: Task 1 `Module`/`ModuleContext`, Task 2 types, Task 4 `Package`/`PluginDb`, Task 5 `HostEngine`/`Epoch`.
- Produces:
  - `wasmtime::component::bindgen!` over `crates/plugin-api/wit/since_v0.1.0`, sync;
  - `convert.rs`: `From`/`TryFrom` between the bindings and the core types. This is the compile-time parity check (deferred T3 minor); the parity test keeps checking the world shape;
  - `caps.rs`: `check_effects(provider: &str, perms: &Permissions, effects: Vec<Effect>) -> Result<Vec<Effect>, CapError>` applies the view caps and id rules. Task 7 adds the permission checks;
  - `WasmPlugin::new(package: Package, component: Component, engine: &HostEngine, db: PluginDb, prefs: BTreeMap<String,String>) -> WasmPlugin`, which implements `Module`:
    - `start` instantiates in a fresh `Store` with an empty `WasiCtx` and `StoreLimits` (64 MiB memory, 10 000 table elements, 1 instance);
    - `start` indexes the manifest commands as items `<id>/<command>` with kind `Command`, `source = Some(manifest.name)` and action `run`, then calls `init` under a 2000 ms deadline;
    - `search` runs under 100 ms, `run_action` and `on_view_event` under 1000 ms;
    - every plugin item gets the `<plugin-id>/` prefix enforced (foreign ids dropped) and `source` set;
  - broken state: on a trap, a limit or a panic, the plugin answers every later call with `Err("plugin <id> stopped: <reason>")`, emits one error toast through `ModuleContext::emit`, and asks to be unloaded through a `PluginEvents` sender (Task 9 consumes it);
  - imports: `storage` (quota, needs the `storage` permission, otherwise returns an error), `preferences` (values from `[plugins.<id>]` in `config.toml`, defaults from the manifest), `log` (into `tracing` with target `plugin`, messages truncated to 4 KiB, ≤ 100 lines per s).

- [ ] **Step 1:** Write failing tests:
  - `init_commands_are_indexed_with_source`;
  - `search_roundtrip`;
  - `run_receives_whole_item` (W2);
  - `on_event_view_target_roundtrip` (W1);
  - `infinite_loop_in_search_traps_and_marks_broken`;
  - `memory_hog_traps_at_64_mib`;
  - `guest_panic_is_an_error_not_a_host_panic`;
  - `storage_quota_enforced`;
  - `storage_without_permission_errors`;
  - `preferences_default_and_override`;
  - `oversized_list_view_is_rejected_with_toast`;
  - `foreign_item_id_dropped`;
  - `slow_search_does_not_block_typing` (Review Focus: a 90 ms search for every keystroke of a 10-key burst; the core gets ≤ 1 answer per generation still current, and stale ones are dropped).
- [ ] **Step 2:** Implement, extending `kitchen-sink` with the behaviours the tests select (`spin`, `hog`, `panic`, `big-view`, `foreign-id`, `echo-item`, …).
- [ ] **Step 3:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(plugin-host): WasmPlugin with deadlines, limits, storage, preferences, log`.

### Task 7: Timers, HTTP, open, clipboard and effect permissions

**Files:**
- Create: `crates/plugin-host/src/imports/{timers.rs,http.rs,open.rs}`
- Modify: `crates/plugin-host/src/caps.rs`, `crates/plugin-host/src/plugin.rs`, `plugins/kitchen-sink/src/lib.rs`
- Test: `crates/plugin-host/tests/{timers.rs,http.rs,open.rs,effects.rs}`

**Interfaces:**
- Produces:
  - `timers.start/cancel`:
    - a host timer thread (one for all plugins, parked when no timer is pending) calls `ModuleContext::wake(token)`;
    - `WasmPlugin::on_wake` maps the token to `event.timer-fired(id)` and calls `on-event`; its effects go out through `CoreEvent::ModuleEffects`;
    - the quota and minimum intervals come from the Global Constraints;
    - timers die with the plugin instance (the reload story is in Task 9);
  - `http.fetch`:
    - requests run on a small worker pool (`ureq`, no automatic redirects); the host follows up to 5 redirects itself and checks each hop against the allowlist;
    - only `http` and `https` are allowed, and the body is capped at 4 MiB;
    - completion wakes the plugin, which receives `event.http-done`;
  - `open.open-url` / `open.open-path`: the scheme and path rules from the Global Constraints, through `tomakast_platform`'s open function, or `open::that`-like code there if none exists;
  - `caps::check_effects` adds the permission checks:
    - `copy-text` needs `clipboard`;
    - `replace-items` needs `index` and the 1000-item quota;
    - `toast` follows the 10-per-10 s rate;
    - unsolicited `update-view` follows the 30-per-s rate;
    - a denied effect is dropped with one error toast per plugin per 10 s (W8: same checks as the import path had).

- [ ] **Step 1:** Write failing tests:
  - `one_shot_timer_fires_on_event_and_effects_arrive`;
  - `repeating_timer_minimum_interval`;
  - `timer_quota`;
  - `cancel_stops_timer`;
  - `timer_thread_parks_when_no_timers`;
  - `http_get_allowed_domain_roundtrip` (a local test server bound to `127.0.0.1` with a `localhost` allowlist entry, test-only);
  - `http_denied_domain_errors_without_request`;
  - `http_redirect_to_unlisted_domain_is_blocked`;
  - `http_body_cap`;
  - `http_concurrency_cap`;
  - `open_url_rejects_file_and_javascript_schemes`;
  - `open_path_rejects_executables_and_desktop_files` (Linux cases run; macOS and Windows cases are `cfg`-gated unit tests of the classifier);
  - `copy_text_effect_without_clipboard_permission_is_dropped`;
  - `replace_items_needs_index_permission_and_quota`;
  - `toast_rate_limit`.
- [ ] **Step 2:** Implement, extending `kitchen-sink`.
- [ ] **Step 3:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(plugin-host): timers, http, open and effect permission checks`.

### Task 8: Permissions and approval

**Files:**
- Create: `crates/plugin-host/src/{approval.rs,plugins_module.rs}`
- Modify: `crates/ui/src/window.rs` (install the `LinkPolicy`), `crates/plugin-host/src/lib.rs`
- Test: `crates/plugin-host/tests/approval.rs`

**Interfaces:**
- Produces:
  - `fn decide(grant: Option<&Grant>, package: &Package) -> Decision`, where `Decision::{Run, Ask, Denied}`:
    - no grant means `Ask`;
    - an `allow` grant with the same hash means `Run`;
    - an `allow` grant with a different hash runs silently if the new permissions are a subset of the stored ones (the hash and `updated_at` are recorded), and asks if the set grew;
    - a `deny` grant with the same hash means `Denied`; with a different hash it means `Ask`;
  - built-in module `PluginsModule` (provider id `plugins`, trusted, native):
    - for every plugin in `Ask` it indexes an item `plugins/review-<id>` titled `Review plugin: <name>`, with the subtitle "New plugin needs your approval" or "Plugin update asks for more permissions";
    - Enter pushes a detail view. Its markdown is written by the host, not by the plugin: the plugin's name, id, version and author as plain text, and the folder path; then one line per permission from `Permissions::describe`, with clipboard and http spelled out ("Can make web requests to example.com, *.example.org"); then the hash prefix;
    - the view has the actions "Allow" and "Deny". The decision is written to `plugin_grants`, and `PluginEvents::Approved(id)` / `Denied(id)` is sent so Task 9 loads or skips the plugin;
  - `PluginLinkPolicy`, which returns `true` for native providers and for plugins with `open`.

  The prompt is never shown unprompted. While the daemon is hidden it only sits in search, and the next time the user opens the launcher with an empty query, "Review plugin: …" leads the Suggestions section. This is the spec's "before the first run it shows the requested permissions and asks for confirmation": the plugin never runs before that.

- [ ] **Step 1:** Write failing tests:
  - `decide_table` (every row of the rules above);
  - `review_item_appears_for_new_plugin`;
  - `review_item_leads_empty_query_suggestions`;
  - `allow_writes_grant_and_emits_approved`;
  - `deny_writes_grant_and_plugin_never_instantiates`;
  - `update_with_same_permissions_runs_silently_and_records_hash`;
  - `update_with_more_permissions_asks_again`;
  - `recreated_folder_with_new_content_asks_again` (Review Focus);
  - `prompt_text_comes_from_manifest_as_plain_text` (a plugin name containing markdown or `<b>` shows literally);
  - `link_policy_denies_plugin_without_open`.
- [ ] **Step 2:** Implement.
- [ ] **Step 3:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(plugin-host): hash-bound approval and the plugins review module`.

### Task 9: Discovery, hot reload and daemon wiring

**Files:**
- Create: `crates/plugin-host/src/{manager.rs,watch.rs}`
- Modify: `crates/app/src/daemon.rs`, `crates/core/src/paths.rs` (`plugins_dir`, `plugin_cache_dir`)
- Test: `crates/plugin-host/tests/manager.rs`

**Interfaces:**
- Produces:
  - `Paths { plugins_dir: data_dir/plugins, plugin_cache_dir: data_dir/cache/plugins }`;
  - `PluginManager::new(paths, engine, db, settings_prefs) -> PluginManager`;
  - `PluginManager::scan(&mut self) -> Vec<ManagerAction>`, where `ManagerAction::{Register(Box<dyn Module>), Unregister(String), Review(ReviewState)}`. The daemon applies each action to `Core` on the UI thread;
  - a watcher on `plugins_dir` debounced to 500 ms of quiet per plugin folder (Review Focus). A changed folder is re-read and re-hashed, decided again, and replaced through `Core::register` (same id replaces);
  - a removed folder is unregistered;
  - a failed read or compile is logged once and retried on the next change only;
  - a reload unregisters the old instance first. Views from the old instance stay on the stack, and their events answer `Err("plugin reloaded")`, shown as an error toast (Review Focus);
  - a change to `config.toml` delivers `preferences-changed` to plugins whose `[plugins.<id>]` table changed;
  - daemon wiring: the `PluginsModule` is always registered; plugins load after `start_modules`; the UI installs `PluginLinkPolicy`.

- [ ] **Step 1:** Write failing tests:
  - `scan_registers_approved_plugin`;
  - `scan_reviews_new_plugin`;
  - `half_copied_folder_waits_for_quiet` (write the manifest, wait 200 ms, write the wasm in two halves 300 ms apart; exactly one load happens, after the second half plus 500 ms);
  - `broken_wasm_logged_once_and_retried_on_change`;
  - `removed_folder_unregisters`;
  - `reload_replaces_instance_and_old_view_events_error`;
  - `preferences_change_delivers_event`.
- [ ] **Step 2:** Implement and wire the daemon. Run with `TOMAKAST_HOME=<tmp>`, copy `kitchen-sink` into `<tmp>/plugins/`, check the review item, approve it (the agent uses `tomakast run plugins/review-kitchen-sink` + action, or headless tests), and see its command in search.
- [ ] **Step 3:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(plugin-host): discovery, hot reload and daemon wiring`.

### Task 10: Threat-model suite and acceptance

**Files:**
- Create: `crates/plugin-host/tests/threat_model.rs`, `docs/experiments/2026-11-plan2b2a-acceptance.md`
- Modify: `plugins/kitchen-sink/src/lib.rs`

- [ ] **Step 1:** Write the threat-model tests (each asserts the denial and that the host keeps running):
  - `wasi_has_no_filesystem` (reading `/etc/passwd` and `.` fails);
  - `wasi_has_no_env_or_args`;
  - `wasi_has_no_sockets` (a TCP connect to `127.0.0.1` fails);
  - `wasi_stdout_is_not_inherited`;
  - `plugin_file_icon_cannot_escape_folder` (`../x.png`, an absolute path and a symlink pointing outside are rejected after canonicalization);
  - `markdown_from_plugin_has_no_html_and_no_remote_images`;
  - `plugin_cannot_set_item_source_or_kind`;
  - `effects_with_control_chars_in_ids_rejected`;
  - `a_hung_plugin_does_not_delay_other_modules` (a native `FakeModule` answers within 20 ms while a plugin spins until its deadline).
- [ ] **Step 2:** Acceptance measurements, using the plan 2b-1 protocol (`docs/experiments/2026-11-plan2b1-acceptance.md`, release, `TOMAKAST_NO_KEYBOARD=1`, kitchen-sink installed and approved):
  - warm-open median over 500 toggles, at most 22.7 ms;
  - CPU over 10 s hidden with the plugin loaded and no timers, 0 ticks;
  - RSS hidden after load and after 250 shows, against the master baseline (~221 MiB). The plugin adds at most +10 MiB; record the result;
  - cold load time of the plugin (cache miss: child compile) and warm (cache hit).
- [ ] **Step 3:** Write the acceptance note. It holds the numbers, the chosen wasmtime features and the binary size delta, the rulings made during the plan, and a user checklist:
  - install kitchen-sink by copying the folder, review and approve it;
  - update it with a new permission and see that it asks again;
  - deny it;
  - delete it.
- [ ] **Step 4:** Run `cargo test --workspace`, clippy and fmt. Commit `test(plugin-host): threat-model suite; docs(experiments): plan 2b-2a acceptance`.
