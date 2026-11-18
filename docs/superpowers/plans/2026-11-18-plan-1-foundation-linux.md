# Plan 1: core, launcher window and app search on Linux

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A daemon on KDE Plasma (Wayland): a global hotkey instantly opens the launcher, which searches and launches applications from `.desktop` files and ranks them by usage (all time, today, time of day). Plus a portable window-experiment binary that the user runs on macOS and Windows.

**Architecture:** Cargo workspace. `tomakast-core` holds the index, matching, ranking, storage, the module trait and settings, with no GPUI dependencies. `tomakast-ui` is the only crate that knows about GPUI: the window and the list view. `tomakast-platform` gathers the OS shims: global hotkeys via the XDG portal, and autostart. `tomakast-module-apps` is the built-in applications module. `tomakast-app` is the `tomakast` binary with the CLI and single-instance IPC.

**Tech Stack:** Rust 1.98 (edition 2024), gpui-kit `=0.10.0` (pulls in gpui-pre `=0.3.12`), nucleo-matcher 0.3, rusqlite 0.43 (feature `bundled`), chrono 0.4, uuid 1 (`v4`), async-channel 2.5, interprocess 2.4, ashpd 0.14 (no tokio, on async-io), freedesktop-desktop-entry 0.8, freedesktop-icons 0.4, notify 8, directories 6, toml 1, serde 1, clap 4, bitflags 2, anyhow/thiserror, tracing + tracing-subscriber, tempfile (dev).

**Spec:** `docs/superpowers/specs/2026-11-18-tomakast-core-design.md`; roadmap: `docs/superpowers/plans/2026-11-18-roadmap.md`.

## Global Constraints

- GPUI versions are pinned exactly: `gpui-kit = { version = "=0.10.0" }`. Do not depend on other GPUI crates directly: everything GPUI comes through `gpui_kit::*`.
- `tomakast-core` does not depend on GPUI and contains no platform code. GPUI types do not leave `tomakast-ui`.
- On Linux, GPUI exits the process by default when the last window closes. The daemon must call `cx.set_quit_mode(QuitMode::Explicit)`.
- Budgets from the spec: warm open (hotkey → first frame) ~50 ms; keystroke → results over 10k items ≤ 16 ms, of which matching and ranking ≤ 5 ms; ~0% CPU and no redraws while the window is hidden; RSS with the window hidden ≤ ~200 MB (measure it and do not regress).
- The target system of this plan is Linux, KDE Plasma on Wayland (layer-shell, the `org.freedesktop.portal.GlobalShortcuts` portal). The other crates must build on macOS and Windows: platform code goes under `cfg`, and on other OSes stubs return `Unsupported`.
- Every record in the store has a stable `id` and `modified_at` (groundwork for sync).
- Item identifier: `<provider-id>/<item-id>`; for applications, `apps/<desktop-file-id>`.
- Logging goes through `tracing`; the level is set by `RUST_LOG`, default `info`.
- **Executing agents do not run a keyboard-grabbing window on the user's live session.** UI is verified with headless tests or with the `TOMAKAST_NO_KEYBOARD=1` variable: layer-shell with `KeyboardInteractivity::None`. Interactive checks (typing, hotkeys, focus) are done by the user from a checklist.
- **The `TOMAKAST_HOME=<dir>` variable** moves the config, database and socket into `<dir>` and disables writing autostart. All dev and test runs of the daemon use it, so the user's real data is not touched.
- The IPC socket name is unique per user: on Linux, a path in `$XDG_RUNTIME_DIR`; on Windows, the pipe name includes the user name; on macOS, a path in the user's `$TMPDIR`.

## Review Focus

1. **Broken or odd `.desktop` files** (invalid UTF-8, no `Name`, malformed quotes in `Exec`). Such an entry is skipped and logged; the rest of the scan continues. Test: Task 12 `scan_skips_broken_entries`.
2. **A second daemon launch while the first is running.** The second daemon does not start and reports that one is already running. Test: Task 8 `second_server_reports_already_running`.
3. **A broken or unreadable `config.toml`.** The daemon starts with default settings and logs the error. Test: Task 7 `load_or_default_survives_invalid_toml`.
4. **Cyrillic and case in the query.** The query "фай" finds "Файлы", "FIRE" finds "Firefox". Test: Task 2 `matches_cyrillic_and_case_insensitive`.
5. **An application removed after the scan.** Launching returns an error, usage is not recorded, the window stays open and shows the error. Test: Task 6 `failed_action_does_not_record_usage`.

---

### Task 1: Workspace skeleton and CI

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`, `.gitignore`, `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/ui/Cargo.toml`, `crates/ui/src/lib.rs`, `crates/platform/Cargo.toml`, `crates/platform/src/lib.rs`, `modules/apps/Cargo.toml`, `modules/apps/src/lib.rs`, `crates/app/Cargo.toml`, `crates/app/src/main.rs`, `.github/workflows/ci.yml`

**Interfaces:**
- Produces: crate names `tomakast-core` (lib `tomakast_core`), `tomakast-ui`, `tomakast-platform`, `tomakast-module-apps`, `tomakast-app` (bin `tomakast`). Shared dependency versions are set in `[workspace.dependencies]`.

- [ ] **Step 1:** Create a workspace with five placeholder crates. `rust-toolchain.toml`: `channel = "1.98"`. Put every dependency version from the Tech Stack into `[workspace.dependencies]`, gpui-kit as `"=0.10.0"`. `.gitignore`: `/target`.
- [ ] **Step 2:** Add `gpui-kit` to `tomakast-ui`. Make sure `cargo build --workspace` passes locally.
  Run: `cargo build --workspace` → Expected: successful build.
- [ ] **Step 3:** Write `.github/workflows/ci.yml`: matrix `ubuntu-latest`, `macos-latest`, `windows-latest`; steps `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. On Ubuntu, install the system libraries for GPUI before building: start with `libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig-dev libx11-xcb-dev libxcb1-dev clang`. If the build fails on a missing `-sys` library, add the needed package.
- [ ] **Step 4:** Run: `cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace` → Expected: all green.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "chore: workspace skeleton and CI"`

### Task 2: Index and matching

**Files:**
- Create: `crates/core/src/index.rs`, `crates/core/src/matcher.rs`
- Modify: `crates/core/src/lib.rs` (pub mod + re-export)
- Test: `crates/core/tests/matcher.rs`, `crates/core/tests/index.rs`

**Interfaces:**
- Produces:
  - `pub struct ItemId(String)` (Clone, Eq, Hash, Debug, Ord) with methods `ItemId::new(provider: &str, local: &str) -> ItemId`, `ItemId::parse(s: &str) -> Option<ItemId>` (requires `/`), `provider(&self) -> &str`, `as_str(&self) -> &str`.
  - `pub enum ItemKind { Application, Command }`
  - `pub enum Icon { File(PathBuf), Name(String) }`
  - `pub struct Action { pub id: String, pub title: String }`
  - `pub struct Item { pub id: ItemId, pub title: String, pub subtitle: Option<String>, pub keywords: Vec<String>, pub icon: Option<Icon>, pub kind: ItemKind, pub actions: Vec<Action> }`. The first action is the primary one; `actions` is non-empty.
  - `pub struct Index` with methods `new() -> Index`, `replace_provider(&mut self, provider: &str, items: Vec<Item>)`, `get(&self, id: &ItemId) -> Option<&Arc<Item>>`, `len(&self) -> usize`, `iter(&self) -> impl Iterator<Item = &Arc<Item>>`.
  - `pub struct Pattern`, `Pattern::parse(query: &str) -> Pattern`, `Pattern::is_empty(&self) -> bool`.
  - `pub struct Matcher`, `Matcher::new() -> Matcher`, `score(&mut self, pattern: &Pattern, item: &Item) -> Option<u32>`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/index.rs
#[test] fn replace_provider_swaps_only_that_provider() {
    // index: apps/a, apps/b, calc/x; replace_provider("apps", [apps/c])
    // => len()==2, get(apps/c).is_some(), get(apps/a).is_none(), get(calc/x).is_some()
}
#[test] fn item_id_parse_requires_slash() {
    assert!(ItemId::parse("apps/firefox.desktop").is_some());
    assert!(ItemId::parse("firefox").is_none());
    assert_eq!(ItemId::new("apps", "x.desktop").provider(), "apps");
}
// tests/matcher.rs
#[test] fn title_match_scores_and_non_match_is_none() {
    // "fire" vs Item{title:"Firefox"} => Some(_); "zzz" => None
}
#[test] fn keyword_match_found_but_below_equal_title_match() {
    // A{title:"Browser"}, B{title:"Firefox", keywords:["browser"]}; query "browser"
    // => both Some, score(A) > score(B)
}
#[test] fn matches_cyrillic_and_case_insensitive() {
    // "фай" vs "Файлы" => Some; "FIRE" vs "Firefox" => Some
}
#[test] fn empty_pattern_matches_everything_with_zero() {
    // Pattern::parse("").is_empty(); score == Some(0)
}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core` → Expected: FAIL (the types do not exist yet).
- [ ] **Step 3:** Implement `index.rs` and `matcher.rs`. `Index` stores a `HashMap<ItemId, Arc<Item>>`. `Matcher` wraps `nucleo_matcher::Matcher` with `Config::DEFAULT`; `Pattern` wraps `nucleo_matcher::pattern::Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy)`. Use `new`, not `parse`: `parse` treats `^ $ ! '` as fzf operators. Score = max of the title score and (best keyword score × 3 / 4).
- [ ] **Step 4:** Run: `cargo test -p tomakast-core` → Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -am "feat(core): item index and fuzzy matcher"` (add new files with `git add`).

### Task 3: Usage model and ranking

**Files:**
- Create: `crates/core/src/usage.rs`
- Test: `crates/core/tests/usage.rs`

**Interfaces:**
- Consumes: `ItemId` (Task 2).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub struct LocalNow { pub unix: i64, pub local_day: i32, pub local_minute: u16 }`. Here `local_day` is the number of days since 1970-01-01 in the local time zone, `local_minute` is 0..1440. Constructors `LocalNow::now()` (via `chrono::Local`) and `LocalNow::from_unix_with_offset(unix: i64, offset_secs: i32) -> LocalNow`.
  - `pub struct UsageRecord { pub id: String, pub item: ItemId, pub query: String, pub at: LocalNow }`. `query` is stored lowercase; `id` is a uuid v4 string. Constructor `UsageRecord::new(item: ItemId, query: &str, at: LocalNow) -> UsageRecord`.
  - `pub struct UsageStats { pub frecency: f64, pub today: u32, pub time_of_day: u32, pub query_hits: u32 }`
  - `pub struct RankWeights { pub half_life_days: f64, pub time_window_minutes: u16, pub w_frecency: f64, pub w_today: f64, pub w_time_of_day: f64, pub w_query: f64, pub usage_scale: f64 }`. `Default`: 14.0, 90, 1.0, 2.0, 1.5, 4.0, 30.0.
  - `pub fn usage_value(s: &UsageStats, w: &RankWeights) -> f64` = `w_frecency*frecency + w_today*today + w_time_of_day*time_of_day + w_query*query_hits`.
  - `pub fn combined_score(match_score: u32, usage: f64, w: &RankWeights) -> f64` = `match_score as f64 + usage_scale * ln(1 + usage)`.
  - `pub struct UsageModel` with methods `new(records: Vec<UsageRecord>) -> UsageModel`, `push(&mut self, r: UsageRecord)`, `stats(&self, query: &str, now: LocalNow, w: &RankWeights) -> HashMap<ItemId, UsageStats>` (a single pass over the records).

How the signals are computed (one record `r`, current `now`, lowercase query `q`):
- frecency += `0.5^((now.unix - r.at.unix) / (half_life_days * 86400))`;
- today += 1 if `r.at.local_day == now.local_day`;
- time_of_day += 1 if the circular minute difference `min(|a-b|, 1440-|a-b|) <= time_window_minutes`;
- query_hits += 1 if `q` is non-empty and `r.query.starts_with(q)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn frecency_halves_after_half_life() { /* record 14 days before now => stats.frecency ≈ 0.5 (eps 1e-9) */ }
#[test] fn today_counts_only_same_local_day() { /* records day=now.day and day-1 => today==1 */ }
#[test] fn time_of_day_window_wraps_midnight() { /* now minute 5, record minute 1430 => time_of_day==1; record minute 200 => 0 */ }
#[test] fn query_hits_use_prefix_of_recorded_query() { /* recorded "fir": q "fi" => 1, q "fo" => 0, q "" => 0 */ }
#[test] fn zero_usage_keeps_match_score() { assert_eq!(combined_score(120, 0.0, &RankWeights::default()), 120.0); }
#[test] fn from_unix_with_offset_computes_local_minute_and_day() {
    // unix 0 with offset +3h => local_minute 180, local_day 0; unix 0 with offset -1h => local_minute 1380, local_day -1
}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test usage` → Expected: FAIL.
- [ ] **Step 3:** Implement `usage.rs` using the formulas above.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test usage` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): usage model and ranking weights`.

### Task 4: Store (SQLite)

**Files:**
- Create: `crates/core/src/store.rs`
- Test: `crates/core/tests/store.rs`

**Interfaces:**
- Consumes: `UsageRecord`, `LocalNow`, `ItemId`.
- Produces: `pub struct Store` with methods `Store::open(path: &Path) -> anyhow::Result<Store>`, `Store::open_in_memory() -> anyhow::Result<Store>`, `record_usage(&self, r: &UsageRecord) -> anyhow::Result<()>`, `load_usage(&self, since_unix: i64) -> anyhow::Result<Vec<UsageRecord>>` (ascending by `at_unix`).

Schema v1 (`PRAGMA user_version = 1`, `journal_mode = WAL`):
```sql
CREATE TABLE usage(
  id TEXT PRIMARY KEY, item_id TEXT NOT NULL, query TEXT NOT NULL,
  at_unix INTEGER NOT NULL, local_day INTEGER NOT NULL, local_minute INTEGER NOT NULL,
  modified_at INTEGER NOT NULL);
CREATE INDEX usage_at ON usage(at_unix);
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn roundtrip_usage_record() { /* in-memory: record, load_usage(0) == [same record] */ }
#[test] fn load_usage_filters_by_since() { /* records at 100 and 200; load_usage(150) -> only 200 */ }
#[test] fn file_store_persists_across_reopen() { /* tempfile dir; open, record, drop, open again, load -> 1 */ }
#[test] fn reopening_does_not_rerun_migration() { /* open twice on same file — no error, user_version == 1 */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test store` → Expected: FAIL.
- [ ] **Step 3:** Implement `store.rs` on rusqlite (`bundled`). Migration: if `user_version == 0`, run the DDL and set it to 1. `modified_at` = `at.unix` at the time of writing.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test store` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): sqlite store for usage history`.

### Task 5: Search engine

**Files:**
- Create: `crates/core/src/engine.rs`
- Test: `crates/core/tests/engine.rs`

**Interfaces:**
- Consumes: Task 2 (`Index`, `Matcher`, `Pattern`, `Item`), Task 3 (`UsageModel`, `RankWeights`, `LocalNow`, `UsageRecord`, `usage_value`, `combined_score`).
- Produces:
  - `#[derive(Clone)] pub struct Hit { pub item: Arc<Item>, pub score: f64 }`
  - `pub struct SearchEngine` with methods `new(usage: UsageModel, weights: RankWeights) -> SearchEngine`, `index(&self) -> &Index`, `index_mut(&mut self) -> &mut Index`, `search(&mut self, query: &str, now: LocalNow) -> Vec<Hit>`, `record_launch(&mut self, item: &ItemId, query: &str, now: LocalNow) -> UsageRecord` (adds a record to the model and returns it for persisting).

Result order:
- **Empty query:** all items; sorted by `usage_value` descending, ties broken by case-insensitive `title` ascending. `score` = `usage_value`.
- **Non-empty query:** only matches; sorted by `combined_score` descending, ties broken by `title`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn habit_beats_slightly_better_match() {
    // items "Firefox", "Fire Alarm"; 5× record_launch(Fire Alarm, "fi", now-1h)
    // search("fi", now)[0].item.title == "Fire Alarm"
}
#[test] fn empty_query_orders_by_usage_then_title() {
    // "Zed"(used 2×), "Alacritty"(0), "Btop"(0) => ["Zed","Alacritty","Btop"]
}
#[test] fn time_of_day_prefers_morning_app_in_morning() {
    // A launched at 09:00 on each of past 10 days; B at 21:00 on each of past 10 days
    // empty query at 09:10 => A first; at 21:05 => B first
}
#[test] fn non_matching_items_are_excluded() { /* query "zzz" => empty */ }
#[test] #[ignore] fn search_10k_under_5ms() {
    // 10_000 synthetic items + 1_000 usage records; for q in ["f","fi","fir","fire"]:
    // median of 20 runs of search(q) < 5 ms. Run: cargo test --release -p tomakast-core -- --ignored
}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test engine` → Expected: FAIL.
- [ ] **Step 3:** Implement `engine.rs`: call `UsageModel::stats` once per search, then walk the index.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test engine && cargo test --release -p tomakast-core --test engine -- --ignored` → Expected: PASS, including the 5 ms budget.
- [ ] **Step 5:** Commit: `feat(core): search engine with usage-aware ranking`.

### Task 6: Modules and `Core`

**Files:**
- Create: `crates/core/src/module.rs`, `crates/core/src/core.rs`, `crates/core/src/testing.rs` (feature `testing`, also used in Task 10)
- Test: `crates/core/tests/core.rs`

**Interfaces:**
- Consumes: Tasks 2–5.
- Produces:
  - `pub enum ActionOutcome { HideWindow, KeepOpen }`
  - `pub enum CoreEvent { ReplaceItems { provider: String, items: Vec<Item> } }`
  - `#[derive(Clone)] pub struct ModuleContext`, method `replace_items(&self, items: Vec<Item>)`. It sends `CoreEvent::ReplaceItems` with the module's provider and drops items with a foreign `id.provider()` with a `warn!`.
  - `pub trait Module: Send + 'static { fn id(&self) -> &'static str; fn start(&mut self, ctx: ModuleContext); fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<ActionOutcome>; }`
  - `pub struct Core` with methods:
    - `new(store: Store, weights: RankWeights, now: LocalNow) -> anyhow::Result<Core>` — loads usage for the last 180 days;
    - `register(&mut self, m: Box<dyn Module>)`, `start_modules(&mut self)`;
    - `events(&self) -> async_channel::Receiver<CoreEvent>`, `apply(&mut self, ev: CoreEvent)`;
    - `search(&mut self, query: &str, now: LocalNow) -> Vec<Hit>`;
    - `execute(&mut self, item: &ItemId, action_id: &str, query: &str, now: LocalNow) -> anyhow::Result<ActionOutcome>`. First the owning module's `run_action`; only on success, `record_launch` in the engine and `record_usage` in the store. An unknown item or module is an `Err`.

- [ ] **Step 1: Write the failing tests** (with `tomakast_core::testing::FakeModule`: `FakeModule::new(items: Vec<Item>, result: fn() -> anyhow::Result<ActionOutcome>)`. In `start` it publishes `items`, and in `run_action` it records `(item id, action id)` into `calls: Arc<Mutex<Vec<(ItemId, String)>>>` and returns `result()`):

```rust
#[test] fn module_items_become_searchable_after_apply() { /* start_modules; recv event; apply; search("") has 2 */ }
#[test] fn execute_records_usage_in_store_and_engine() { /* execute ok => store.load_usage(0).len()==1; empty-query order changes */ }
#[test] fn failed_action_does_not_record_usage() { /* run_action -> Err => execute is Err; load_usage(0) is empty */ }
#[test] fn foreign_provider_items_are_dropped() { /* module "fake" publishes id "other/x" => not in index */ }
#[test] fn execute_unknown_item_is_error() {}
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test core` → Expected: FAIL.
- [ ] **Step 3:** Implement. Modules are stored in a `HashMap<String, Box<dyn Module>>`; the channel is `async_channel::unbounded`.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): module trait and core orchestrator`.

### Task 7: Settings, hotkeys and paths

**Files:**
- Create: `crates/core/src/settings.rs`, `crates/core/src/paths.rs`
- Test: `crates/core/tests/settings.rs`

**Interfaces:**
- Produces:
  - `bitflags! pub struct Modifiers: u8 { CTRL, ALT, SHIFT, SUPER }`
  - `pub struct Hotkey { pub mods: Modifiers, pub key: String }`. `key` is the lowercase key name (`"space"`, `"t"`, `"f1"`). Methods: `Hotkey::parse(s: &str) -> Result<Hotkey, HotkeyError>`, `to_xdg_trigger(&self) -> String`, `Display` (`"Ctrl+Alt+T"`).
  - `pub struct Settings { pub launcher_hotkey: Hotkey, pub autostart: bool, pub hotkeys: Vec<(Hotkey, ItemId)> }`. `Default`: `Alt+Space`, `true`, empty.
  - `Settings::load(path: &Path) -> anyhow::Result<Settings>` — returns `Default` if the file does not exist.
  - `Settings::load_or_default(path: &Path) -> (Settings, Option<anyhow::Error>)`.
  - `pub struct Paths { pub config_file: PathBuf, pub data_dir: PathBuf, pub db_file: PathBuf }` with methods `Paths::detect() -> anyhow::Result<Paths>` (`directories::ProjectDirs::from("", "", "tomakast")`, `config_dir/config.toml`, `data_dir/tomakast.db`; if `TOMAKAST_HOME` is set, everything lives in it) and `ensure_dirs(&self) -> anyhow::Result<()>`.

`config.toml` format:
```toml
[launcher]
hotkey = "Alt+Space"
autostart = true

[hotkeys]
"Ctrl+Alt+F" = "apps/firefox.desktop"
```
XDG trigger: modifiers `CTRL`, `ALT`, `SHIFT`, `LOGO` (that is Super) in this order, then the lowercase XKB key name, separated by `+`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn parse_and_format_hotkey() {
    let h = Hotkey::parse("ctrl+alt+t").unwrap();
    assert_eq!(h.to_string(), "Ctrl+Alt+T");
    assert_eq!(h.to_xdg_trigger(), "CTRL+ALT+t");
    assert_eq!(Hotkey::parse("Super+Space").unwrap().to_xdg_trigger(), "LOGO+space");
}
#[test] fn parse_rejects_bad_hotkeys() {
    for s in ["", "Ctrl+", "Ctrl+A+B", "Hyper+A"] { assert!(Hotkey::parse(s).is_err(), "{s}"); }
}
#[test] fn tomakast_home_overrides_paths() { /* TOMAKAST_HOME=/tmp/x => config_file == /tmp/x/config.toml, db_file == /tmp/x/tomakast.db */ }
#[test] fn missing_file_gives_defaults() { /* load(nonexistent) == Settings::default() */ }
#[test] fn parses_hotkey_table() { /* sample above => hotkeys == [(Ctrl+Alt+F, apps/firefox.desktop)] */ }
#[test] fn load_or_default_survives_invalid_toml() { /* "[launcher" => (default, Some(err)) */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-core --test settings` → Expected: FAIL.
- [ ] **Step 3:** Implement (serde + toml). An invalid hotkey or id in `[hotkeys]` is a load error that names the key in the message.
- [ ] **Step 4:** Run: `cargo test -p tomakast-core --test settings` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): settings, hotkey parsing, paths`.

### Task 8: Single-instance IPC and CLI

**Files:**
- Create: `crates/app/src/ipc.rs`, `crates/app/src/cli.rs`
- Modify: `crates/app/src/main.rs`
- Test: `crates/app/tests/ipc.rs`

**Interfaces:**
- Consumes: `ItemId`.
- Produces:
  - `pub enum Request { Toggle, Run(ItemId), Ping }` with methods `encode(&self) -> String` (`"toggle"`, `"run <id>"`, `"ping"`) and `Request::decode(line: &str) -> anyhow::Result<Request>`.
  - `pub fn serve_on(name: &str, handler: impl Fn(Request) + Send + 'static) -> Result<ServerHandle, ServeError>`, where `ServeError::AlreadyRunning` is returned if the name is held by a live server.
  - `pub fn send_to(name: &str, req: &Request) -> anyhow::Result<()>` — writes the line, waits for `"ok\n"`.
  - `pub const SOCKET_NAME: &str = "tomakast.sock"`; `serve`/`send` are wrappers using this name.
  - CLI (clap): no subcommand — `send(Toggle)`, and if that fails, start the daemon; `daemon` — run the daemon, exiting with code 1 if a daemon is already running; `toggle`; `run <ID>`.

The socket name is unique per user:
- Linux: `GenericFilePath` `$XDG_RUNTIME_DIR/tomakast.sock`, or `<TOMAKAST_HOME>/tomakast.sock` if the variable is set;
- Windows: `GenericNamespaced` `tomakast-<USERNAME>.sock`;
- macOS: `GenericFilePath` in `std::env::temp_dir()`, which is the user's `$TMPDIR`.

On `AddrInUse` for a file socket, first try `Ping`. If there is no reply, delete the file and bind again. If a reply arrives, `AlreadyRunning`.

- [ ] **Step 1: Write the failing tests** (a unique name per test: `format!("tomakast-test-{}", uuid)`):

```rust
#[test] fn request_roundtrip() { /* Toggle, Ping, Run(apps/x.desktop) encode→decode */ }
#[test] fn decode_rejects_garbage() { /* "", "run", "jump x" => Err */ }
#[test] fn send_reaches_handler() { /* serve_on(name, push to channel); send_to(name, Toggle); recv == Toggle */ }
#[test] fn second_server_reports_already_running() { /* serve_on twice => Err(AlreadyRunning) */ }
#[test] fn send_without_server_errors() {}
#[test] fn stale_socket_file_is_replaced() { /* create empty file at socket path (Linux/macOS) => serve_on succeeds */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-app --test ipc` → Expected: FAIL.
- [ ] **Step 3:** Implement on `interprocess::local_socket` (synchronous API, an acceptor thread).
- [ ] **Step 4:** Run: `cargo test -p tomakast-app --test ipc` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(app): single-instance IPC and CLI`.

### Task 9: Window experiment (time-boxed, with a manual gate)

**Order:** do this task right after Task 1. It needs only the skeleton, and the user's run on the Mac and Windows is the slowest step, so let it proceed in parallel with Tasks 2–8. The number is kept for the sake of references.

The goal is to close the spec's open questions about the window before all other UI tasks. The time box is one working day. If it is not done within that time, the task stops: the measurements are collected and handed to the user.

**Files:**
- Create: `crates/ui/examples/window_probe.rs`, `docs/experiments/2026-11-window-probe.md`

**Interfaces:**
- Produces: findings recorded in `docs/experiments/2026-11-window-probe.md`. Task 10 and Task 13 depend on them:
  - recreate the window or keep it persistent;
  - whether prewarming the renderer helps;
  - the cause of the white backdrop and how to remove it.

What `window_probe` does:
- Flags:
  - `--strategy recreate|persistent`. `persistent` = the window is not destroyed and is hidden by OS means. If that is unavailable on the current platform, print `unsupported` and exit.
  - `--prewarm` — at startup, open and immediately close a 1×1 window.
  - `--root kit|plain` — open the window via `gpui_kit::open_window` or via bare `cx.open_window` without the kit root. Needed to find the cause of the white backdrop.
  - `--cycles N` (default 10), `--interval-ms` (default 700).
  - `--interactive` — a user-only mode. A window with an input field and keyboard (Exclusive on Wayland, a regular key window on macOS and Windows) is shown and hidden by the global hotkey `Alt+Space`. On Linux the hotkey goes through the portal (temporary code modeled on Task 11), on macOS and Windows through the `global-hotkey` crate. What is checked:
    - whether text can be typed;
    - on macOS: whether you can type without activating the app, and whether focus returns to the previous app after hiding;
    - whether a click outside the window closes it;
    - on KDE: whether the portal asks for confirmation again after a process restart, and under which application name;
    - whether the portal activation event carries a timestamp from which "hotkey → frame" can be measured.
- A 720×460 window: transparent background, 14 px corner radius, semi-transparent content fill.
  - Linux/Wayland: layer-shell, layer `Overlay`, `KeyboardInteractivity::None`, so the experiment does not grab the user's keyboard.
  - macOS and Windows: `WindowKind::PopUp`, no title bar.
- It shows and hides the window `N` times by itself. On each show it measures the time from the request to the first `render` and to `window.on_next_frame`. At the end it prints a table: `first`, median, p95 for both metrics, RSS before and after.
- `--idle 10` mode: 10 seconds with the window hidden. CPU ticks come from `/proc/self/stat` on Linux, `getrusage` on macOS, `GetProcessTimes` on Windows.
- Always calls `cx.set_quit_mode(QuitMode::Explicit)`.

- [ ] **Step 1:** Implement `window_probe`. Run: `cargo run --release -p tomakast-ui --example window_probe -- --strategy recreate --prewarm` on KDE → Expected: the table is printed; the process does not exit when the last window closes.
- [ ] **Step 2:** On Linux, run the `strategy × prewarm × root` matrix and `--idle 10`. Record the results in `docs/experiments/2026-11-window-probe.md`. Criteria:
  - median warm show up to `on_next_frame` ≤ 50 ms;
  - `first` with `--prewarm` ≤ 50 ms;
  - idle: 0 CPU ticks.
- [ ] **Step 3:** Find the cause of the white backdrop: compare `--root kit` and `--root plain`, then narrow it down inside. Record the cause and the fix. If the fix requires a GPUI patch, wire it in via `[patch.crates-io]` to a fork and record the link in the document.
- [ ] **Step 4 (manual gate, done by the user):** build and run `window_probe` on macOS and Windows. Record in the document:
  - the measurements table;
  - whether the window steals focus from the current app (macOS);
  - whether there is no taskbar or Alt-Tab icon (Windows);
  - transparency and rounded corners without a backdrop;
  - `--interactive` results (list above) on each OS, including KDE.

  If the criteria are not met, follow the fallback plan from the spec: fork the platform backend, then discuss with the user. Task 10 can start after steps 2–3 without waiting for this gate.
- [ ] **Step 5:** Commit: `chore(ui): window probe experiment and findings`.

### Task 10: Launcher window and view

**Files:**
- Create: `crates/ui/src/launcher_view.rs`, `crates/ui/src/window.rs`, `crates/ui/src/theme.rs`
- Modify: `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/launcher_view.rs` (headless tests from `gpui_kit::test`; how to use them is described in the gpui-kit 0.10.0 crate's `TESTING.md` at `~/.cargo/registry/src/*/gpui-kit-0.10.0/TESTING.md`)

**Interfaces:**
- Consumes: `Core`, `Hit`, `LocalNow`, `ActionOutcome`, `CoreEvent` (Task 6); Task 9 findings.
- Produces:
  - `pub struct LauncherController` with methods:
    - `new(core: Rc<RefCell<Core>>) -> LauncherController`;
    - `prewarm(&mut self, cx: &mut App)`;
    - `show(&mut self, cx: &mut App)`, `hide(&mut self, cx: &mut App)`, `toggle(&mut self, cx: &mut App)`;
    - `is_visible(&self) -> bool`;
    - `core_changed(&mut self, cx: &mut App)` — recompute the results of the open window.

    Stored in a `gpui::Global`.
  - `pub fn init(cx: &mut App)` — key bindings in the `"Launcher"` context: `up` → `SelectPrev`, `down` → `SelectNext`, `enter` → `Confirm`, `escape` → `Dismiss`; theme follows the system one (`window.appearance()`) and tracks its changes.
  - `pub struct LauncherView` — an input field (placeholder "Search apps and commands…") and a virtualized list of `Hit`. In a row: a 24 px icon (`Icon::File` → `img(path)`, no icon → empty space), `title`, `subtitle` in a muted color. The selected row has the `list_active` background. At the bottom, a line for errors.

Behavior:
- Every input → `core.search(query, LocalNow::now())`. The selection resets to 0 and the list scrolls to the top.
- `Confirm` → `core.execute(id, actions[0].id, query, now)`. On `Ok(HideWindow)`, `hide`. On `Err(e)` the window stays open and `e.to_string()` appears in the bottom line for 4 s.
- `Dismiss` and loss of window activation (where the platform reports it) → `hide`.
- Every `show` starts with an empty query; the input field gets focus immediately.
- Window and transparency follow the Task 9 findings. Each show logs a "show request → `on_next_frame`" measurement at `debug` level.

- [ ] **Step 1: Write the failing tests** (headless, `Core` with `tomakast_core::testing::FakeModule` from Task 6):

```rust
// typing_filters_results: type "fir" => 1 row, title "Firefox"
// enter_executes_selected_and_hides: down, enter => FakeModule got run_action for the 2nd item; controller.is_visible()==false
// escape_hides: escape => is_visible()==false
// error_keeps_window_and_shows_message: run_action -> Err("gone") => is_visible()==true, output contains "gone"
// show_resets_query: type "x", hide, show => field is empty
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-ui` → Expected: FAIL.
- [ ] **Step 3:** Implement `window.rs`, `launcher_view.rs`, `theme.rs`.
- [ ] **Step 4:** Run: `cargo test -p tomakast-ui` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(ui): launcher window and list view`.

### Task 11: Global hotkeys (Linux, portal) and autostart

**Files:**
- Create: `crates/platform/src/hotkeys.rs`, `crates/platform/src/hotkeys/linux.rs`, `crates/platform/src/autostart.rs`
- Test: `crates/platform/tests/hotkeys.rs`, `crates/platform/tests/autostart.rs`

**Interfaces:**
- Consumes: `Hotkey`, `Hotkey::to_xdg_trigger` (Task 7).
- Produces:
  - `pub struct HotkeyBinding { pub id: String, pub description: String, pub hotkey: Hotkey }`. `id` is `"launcher"` or `ItemId::as_str()`.
  - `pub enum HotkeyBackend { Portal, Unsupported { reason: String } }`
  - `pub fn register_global_hotkeys(bindings: Vec<HotkeyBinding>, on_activate: impl Fn(String) + Send + 'static) -> HotkeyBackend`. Starts a thread with `async_io::block_on`. Via ashpd `GlobalShortcuts`: `create_session` → `bind_shortcuts(NewShortcut::new(id, description).preferred_trigger(trigger))` → the `receive_activated()` stream → `on_activate(shortcut_id)`. If there is no portal or binding fails, returns `Unsupported` with the reason. On non-Linux, an `Unsupported` stub.
  - `pub fn portal_shortcuts(bindings: &[HotkeyBinding]) -> Vec<(String, String, String)>` — a pure function returning (id, description, trigger).
  - `pub fn set_autostart_in(dir: &Path, enabled: bool, exe: &Path) -> anyhow::Result<()>` and `pub fn set_autostart(enabled: bool) -> anyhow::Result<()>`. On Linux, writes or removes `$XDG_CONFIG_HOME/autostart/tomakast.desktop` with the contents `[Desktop Entry]`, `Type=Application`, `Name=tomakast`, `Exec="<exe>" daemon`, `X-GNOME-Autostart-enabled=true`. On other OSes, `Ok(())` with a `warn!`; the real implementation comes in plans 3 and 4.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn portal_shortcuts_use_xdg_triggers() { /* launcher Alt+Space => ("launcher", "Open tomakast", "ALT+space") */ }
#[test] fn autostart_writes_and_removes_desktop_file() { /* tempdir; enabled=true => file contains `Exec="/x/tomakast" daemon`; enabled=false => file gone */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-platform` → Expected: FAIL.
- [ ] **Step 3:** Implement. Add ashpd with `default-features = false, features = ["async-io"]` plus the `global_shortcuts` feature if it is separate in 0.14 (check the crate's `Cargo.toml`).
- [ ] **Step 4:** Run: `cargo test -p tomakast-platform` → Expected: PASS. Manual check on KDE: a temporary `examples/hotkey_probe.rs` registers `Alt+Space`. KDE shows a confirmation dialog, after which pressing the hotkey prints `activated launcher`.
- [ ] **Step 5:** Commit: `feat(platform): portal global shortcuts and XDG autostart`.

### Task 12: `apps` module (Linux)

**Files:**
- Create: `modules/apps/src/lib.rs`, `modules/apps/src/linux.rs`, `modules/apps/tests/fixtures/{user,system}/…*.desktop`
- Test: `modules/apps/tests/scan.rs`

**Interfaces:**
- Consumes: `Module`, `ModuleContext`, `Item`, `ItemId`, `Icon`, `Action`, `ActionOutcome` (Tasks 2, 6).
- Produces:
  - `pub struct AppsModule`, `AppsModule::new() -> AppsModule`, `impl Module` with `id() == "apps"`.
  - `pub struct ScanOptions { pub dirs: Vec<PathBuf>, pub locales: Vec<String>, pub current_desktops: Vec<String>, pub path_env: Option<OsString> }`, `ScanOptions::from_env() -> ScanOptions`.
  - `pub struct AppEntry { pub desktop_file: PathBuf, pub exec: Vec<String>, pub terminal: bool, pub working_dir: Option<PathBuf> }`
  - `pub fn scan(opts: &ScanOptions) -> Vec<(Item, AppEntry)>`
  - `pub fn launch(entry: &AppEntry) -> anyhow::Result<()>`

Scan rules:
- Directories in priority order:
  1. `$XDG_DATA_HOME/applications` (default `~/.local/share/applications`);
  2. `<each of $XDG_DATA_DIRS>/applications` (default `/usr/local/share:/usr/share`);
  3. then, if they exist and are not already included: `~/.local/share/flatpak/exports/share/applications`, `/var/lib/flatpak/exports/share/applications`, `/var/lib/snapd/desktop/applications`.

  Recursively.
- Desktop file id is the path relative to the directory, with `/` replaced by `-`. On duplicates, the first directory wins.
- Skip entries:
  - with `Type` ≠ `Application`, `NoDisplay=true`, `Hidden=true`;
  - with `OnlyShowIn` that contains none of `current_desktops` (from `$XDG_CURRENT_DESKTOP`, separated by `:`);
  - with `NotShowIn` that contains any of them;
  - with a `TryExec` that is not found in `path_env`;
  - without `Name`, without `Exec`, or where `parse_exec()` returned an error (logged).
- Item:
  - `id = apps/<desktop-file-id>`, `title` = localized `Name`;
  - `subtitle` = `GenericName`, otherwise `Comment`;
  - `keywords` = `Keywords` + `GenericName` + basename of the first `Exec` argument;
  - `kind = Application`, `actions = [Action { id: "open", title: "Open" }]`;
  - `icon`: absolute path → `Icon::File`; name → lookup via `freedesktop-icons` (`lookup(name).with_size(48).find()`), found → `Icon::File(path)`, otherwise `None`.
- Locales come from `LC_ALL`, otherwise `LC_MESSAGES`, otherwise `LANG`: `ru_RU.UTF-8` → `["ru_RU", "ru"]`.
- `exec` comes from `DesktopEntry::parse_exec()` (the crate strips field codes like `%u`/`%f`).

Launching:
- `terminal == true` → `gio launch <desktop_file>`; if `gio` is missing, return the error "terminal apps need gio".
- Otherwise `Command::new(exec[0]).args(&exec[1..])`: `current_dir(working_dir)`, stdio = null, `process_group(0)`. A spawn error → `Err` with the application name.
- To avoid zombie processes: after spawn, call `child.wait()` in a separate lightweight thread. Same for `gio launch`.
- `run_action("open")` → `launch` → `Ok(HideWindow)`. An unknown action → `Err`.

Refresh: a `notify` watcher on all existing directories from the list, debounce 500 ms → `scan` → `ctx.replace_items`. The first scan and the watcher run in a background thread created in `start`.

- [ ] **Step 1: Write the failing tests** on fixtures (two directories: `user` and `system`):

```rust
#[test] fn scan_basic_entry() { /* firefox.desktop: Exec="firefox %u" => title "Firefox", exec == ["firefox"], id "apps/firefox.desktop" */ }
#[test] fn scan_uses_locale_name() { /* Name[ru]=Файлы, locales ["ru_RU","ru"] => title "Файлы" */ }
#[test] fn scan_filters_hidden_nodisplay_onlyshowin_tryexec() { /* 4 fixtures with current_desktops ["KDE"], path_env empty dir => none present */ }
#[test] fn user_dir_overrides_system_duplicate() { /* same id in both => title from user */ }
#[test] fn subdir_desktop_id_uses_dashes() { /* system/kde/konsole.desktop => id "apps/kde-konsole.desktop" */ }
#[test] fn scan_skips_broken_entries() { /* invalid UTF-8 file, file without Name, Exec with unbalanced quote => skipped, others still returned */ }
#[test] fn launch_missing_binary_errors() { /* exec ["/nonexistent/bin"] => Err */ }
```

- [ ] **Step 2:** Run: `cargo test -p tomakast-module-apps` → Expected: FAIL.
- [ ] **Step 3:** Implement `linux.rs` (under `cfg(target_os = "linux")`). In `lib.rs`, on other OSes `scan` returns an empty list; this is temporary, until plans 3 and 4.
- [ ] **Step 4:** Run: `cargo test -p tomakast-module-apps` → Expected: PASS.
- [ ] **Step 5:** Commit: `feat(apps): desktop entry scanning and launching on Linux`.

### Task 13: Daemon wiring and acceptance

**Files:**
- Create: `crates/app/src/daemon.rs`, `docs/experiments/2026-11-plan1-acceptance.md`
- Modify: `crates/app/src/main.rs`

**Interfaces:**
- Consumes: everything above.

`daemon::run(paths: Paths) -> anyhow::Result<()>`, startup order:
1. Initialize `tracing`.
2. `Paths::ensure_dirs`. `Paths::detect` honors `TOMAKAST_HOME`.
3. `Settings::load_or_default`; if there is an error, `error!` and keep going.
4. `Store::open(db_file)` → `Core::new`.
5. `register(AppsModule)` → `start_modules`.
6. `serve(handler)` — on `AlreadyRunning`, exit with code 1 and a message. The handler forwards each `Request` into an `async_channel`.
7. `gpui_kit::application().run`. Inside:
   - `gpui_kit::init`, `cx.set_quit_mode(QuitMode::Explicit)`, `tomakast_ui::init`;
   - `LauncherController::new` + `prewarm`;
   - two `cx.spawn` tasks. The first reads `core.events()` → `core.apply` → `controller.core_changed`. The second reads IPC requests: `Toggle` → `toggle`; `Run(id)` → `core.execute(id, primary, "", now)`, errors are logged;
   - `register_global_hotkeys`: `launcher` + all `settings.hotkeys`. Registration happens right at startup. An app hotkey's description is its `ItemId`, not its `title`: the index is still empty at this point. Activations are forwarded into the same channel: `"launcher"` → `Toggle`, everything else → `Run`. The launcher binding's description is `"Open tomakast"`. If `Unsupported` is returned, log a `warn!` with a hint: "bind the `tomakast toggle` command in your system settings".
8. `set_autostart(settings.autostart)` if `TOMAKAST_HOME` is not set; errors are logged.

- [ ] **Step 1:** Implement `daemon.rs` and wire it into the CLI from Task 8.
  Run: `cargo build --release -p tomakast-app` → Expected: successful build.
- [ ] **Step 2:** Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings` → Expected: PASS.
- [ ] **Step 3 (manual acceptance on KDE, performed by the user; the agent prepares the build and the checklist):** run `target/release/tomakast daemon`. Record the results in `docs/experiments/2026-11-plan1-acceptance.md`.
  - After confirmation in the KDE dialog, Alt+Space opens the launcher; pressing it again or Esc closes it. Alt+Space is free on the user's machine: checked against `~/.config/kglobalshortcutsrc`, KRunner is not bound to it.
  - "fir" → Firefox first; Enter launches it, the launcher window closes, and the launched app gets focus. A minute after the app is closed, `ps` shows no zombie processes of the daemon.
  - After several launches of the same app, it rises in the empty query.
  - A hotkey from `[hotkeys]` launches the app without opening the launcher.
  - `tomakast toggle` works. A second `tomakast daemon` exits with code 1.
  - A new `.desktop` in `~/.local/share/applications` shows up in search without a restart.
  - Measurements: median warm open from the `debug` log (≤ ~50 ms); first open after startup; CPU over 10 s with the window hidden (0 ticks); RSS with the window hidden (≤ ~200 MB).
  - A broken `config.toml` → the daemon starts, the error is in the log.
  - After re-login, the daemon starts on its own.
- [ ] **Step 4:** Commit: `feat(app): daemon wiring and plan-1 acceptance notes`.
