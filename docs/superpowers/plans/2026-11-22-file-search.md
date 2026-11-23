# File Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A built-in `files` module that indexes the home folder, mixes up to 3 file results into the root search (ranked with usage like any item), and offers a "Search Files" screen with a list / grid (thumbnails) toggle and open, reveal, copy path and open-with actions.

**Architecture:** The module walks the covered folders into a compact in-memory tree, keeps it fresh with a file watcher plus periodic re-walks, and answers searches on its own worker using the core's matcher. The core gains three small, Rust-only pieces: path item kinds that the engine ranks as matches (not as answers), an internal effect that records usage from a module's screen, and a grid layout for list views. WIT is unchanged.

**Tech Stack:** Rust, `ignore` (walk with gitignore rules), `globset`, `notify`, `rayon`, `nucleo-matcher` through `tomakast_core::Matcher`, `md-5`, `image` (png/jpeg/webp/gif), `zbus` (reveal via `tomakast-module-apps`), GPUI through `gpui-kit` for the grid layout.

**Spec:** `docs/superpowers/specs/2026-11-22-file-search-design.md` — it supersedes this plan where they differ (its "Changes after approval": 30-minute re-walks when watching is off, gitignore only inside repositories, UI-thread time "within the 5 ms budget" rather than "unchanged", thumbnails decoded in the raster helper).

## Global Constraints

- Module crate `modules/files`, package `tomakast-module-files`, provider id `files`; built-in, not a plugin.
- WIT (`crates/plugin-api/wit`) is not changed. Plugins cannot send the new effect and always get list layout `Rows`.
- Item id of a file or folder: `files/<md5 of the full path's bytes, 32 lowercase hex>`. The command's id: `files/search`.
- Root answers: only for queries with at least **3** characters (`query.trim().chars().count() >= 3`), at most **3** items, none when `in_root = false`.
- Screen: no minimum, at most **100** rows.
- Ranking of path items in the root list: the engine's match score (title and keywords) plus usage, with the same formula as indexed items, minus `PATH_TIE_PENALTY = 8` match points.
- Default roots `["~"]`; skipped by default: hidden entries, `.gitignore`/`.ignore` matches, folders named `node_modules`, `target`, `__pycache__`, `.venv`, `venv`; `~/Library` on macOS, `~/AppData` on Windows. Symlinks not followed; other file systems not crossed.
- `[files]` table keys and defaults: `in_root = true`, `roots = ["~"]`, `exclude = []`, `hidden = false`.
- Re-walk at most every **30 minutes**; file watching capped at `min(max_user_watches / 2, 200_000)` folders (Linux; elsewhere 200_000).
- Thumbnails: freedesktop cache `$XDG_CACHE_HOME/thumbnails/{large,normal}` (fallback `~/.cache/thumbnails`), name `md5(file URI).png`, valid when the PNG's `Thumb::MTime` text chunk equals the file's mtime in whole seconds. Generated only for PNG, JPEG, WebP, GIF, files ≤ **50 MB** and ≤ **100 megapixels**; generated size 256 px (`large`). Never on the UI thread.
- Budgets (measured in the acceptance note): module search ≤ 30 ms at the 95th percentile over this home folder; resident memory of the index ≤ ~150 MB for 500k entries; window open time and the UI-thread time per keystroke unchanged.
- Run and try only through `just` recipes; never a keyboard-grabbing window for your own runs (`TOMAKAST_NO_KEYBOARD=1`, throwaway `TOMAKAST_HOME`); never kill processes you did not start; no screenshots or personal file names committed; no attribution lines in commits; no push, no merge into `main`.
- Other sessions edit core/UI in parallel: keep shared changes to what the task names.

## Review Focus

1. **A path the core cannot hold as an id or show as text** (longer than 256 bytes, a newline or a bidi override in the name, invalid UTF-8): it must still be indexed, found, shown with a replacement character and opened by its real bytes. Test in Task 5 (`id_and_title_of_hostile_names`) and Task 8 (`opens_a_file_with_a_newline_in_its_name`).
2. **The index is being rebuilt or the watcher fires while the user types**: a search during a re-walk answers from the old index, never from a half-built one, and never blocks for the walk. Test in Task 7 (`search_during_rewalk_uses_the_old_index`).
3. **An app and a never-opened file match equally (`fire` → Firefox, `fire.txt`)**: the app leads; after the file is opened for that query a few times, the file leads. Test in Task 1 (`unopened_file_ranks_below_equal_app`, `habit_lifts_file_above_app`).
4. **A file removed or renamed after it was shown**: its action fails with a clear toast ("File not found"), not a panic, and the stale entry leaves the index on the next event. Test in Task 8 (`open_of_a_vanished_file_fails_with_a_toast`).
5. **A query typed on the wrong layout or in another script** (`ашку` → `fire.txt`, `otchet` → `Отчёт.pdf`): the module finds it and the engine keeps it in the root list. Test in Task 6 (`finds_across_layouts_and_scripts`) and Task 1 (`path_item_matched_through_its_own_script`).

---

## Execution Order

Run Tasks 5 and 6 (index, search, bench) first, then 1–4 (shared core/UI), then 7–12. The bench gates the core work: if resident memory or p95 misses the budget with the core's `TextKeys` per name, find the leaner entry point before touching `engine.rs`.

---

## File Structure

- `crates/core/src/index.rs` — `ItemKind::File`, `ItemKind::Folder`, `ItemKind::is_path()`.
- `crates/core/src/engine.rs` — ranking of dynamic path items; `PATH_TIE_PENALTY`.
- `crates/core/src/effect.rs`, `crates/core/src/core.rs` — `Effect::RecordUsage`.
- `crates/core/src/view.rs` — `ListLayout`, `ListView::layout`.
- `crates/core/src/settings.rs` — `FilesSettings`, `Settings::files`.
- `crates/ui/src/root_sections.rs` — labels for the new kinds.
- `crates/ui/src/views/list.rs` (+ `stack.rs`, `launcher_view.rs` key handling) — grid layout of a list.
- `crates/plugin-host/src/{convert.rs,caps.rs}` — `layout: Rows`; refuse `RecordUsage` as a plugin effect.
- `modules/apps/src/{lib.rs,linux.rs}` — `AppEntry::mime_types`, `launch_with_file`.
- `modules/files/src/lib.rs` — `FilesModule` (the `Module` impl, actions, root answers).
- `modules/files/src/index.rs` — `FileIndex` tree, walk, incremental updates.
- `modules/files/src/rules.rs` — what is covered: roots, exclusions, `~` expansion.
- `modules/files/src/search.rs` — matching and ranking over the index.
- `modules/files/src/live.rs` — the index thread: first walk, watcher, re-walks, config reload.
- `modules/files/src/names.rs` — ids, display titles, subtitles, file URIs.
- `modules/files/src/mime.rs` — extension → MIME → icon name (Linux shared-mime-info).
- `modules/files/src/screen.rs` — the "Search Files" list view, layout toggle.
- `modules/files/src/state.rs` — persisted module state: opened files, layout.
- `modules/files/src/thumbs.rs` — thumbnail cache lookup and generation (Linux).
- `modules/files/src/open_with.rs` — the open-with list (Linux).
- `modules/files/examples/bench.rs` + `justfile` recipe `files-bench` — timings for the acceptance note.
- `crates/app/src/daemon.rs`, `crates/app/Cargo.toml`, root `Cargo.toml` — registration.
- `docs/experiments/2026-11-file-search-acceptance.md` — acceptance note.

---

### Task 1: Path kinds ranked as matches in the engine

**Files:**
- Modify: `crates/core/src/index.rs` (`ItemKind`), `crates/core/src/engine.rs` (`search_with`), `crates/core/src/lib.rs` (export `PATH_TIE_PENALTY`)
- Modify: `crates/ui/src/root_sections.rs` (`kind_section`, `kind_label`)
- Test: `crates/core/tests/engine.rs`, `crates/ui/src/root_sections.rs` tests

**Interfaces:**
- Produces: `ItemKind::File`, `ItemKind::Folder`; `impl ItemKind { pub fn is_path(self) -> bool }`; `pub const PATH_TIE_PENALTY: u32 = 8;` in `engine.rs`, re-exported from `tomakast_core`.

Rule for a dynamic item whose `kind.is_path()`: on the empty query it is dropped; otherwise the engine matches it with its own matcher against title and keywords (`ItemKeys::new(item)`, pattern built with `Pattern::across(query, self.index.scripts().union(keys.scripts()), ..)` so a name in a script the index lacks still gets its readings); no match → dropped; a match `m` scores `combined_score(m.saturating_sub(PATH_TIE_PENALTY), usage, weights)` with tie-break `TieBreak::Static(lowercase title)`. Other dynamic items keep `DYNAMIC_MATCH_SCORE`.

- [ ] **Step 1: Write the failing tests** in `crates/core/tests/engine.rs` (use the file's existing item builders and `at(..)`):

```rust
#[test]
fn unopened_file_ranks_below_equal_app() {
    // index: app "Firefox"; dynamic: File "fire.txt" with keyword "~/fire.txt"
    let hits = e.search_with("fire", at(0, 0), &[file]);
    assert_eq!(titles(&hits), ["Firefox", "fire.txt"]);
}

#[test]
fn habit_lifts_file_above_app() {
    for _ in 0..3 { e.record_launch(&file.id, "fire", at(0, 0)); }
    let hits = e.search_with("fire", at(0, 0), &[file]);
    assert_eq!(titles(&hits)[0], "fire.txt");
}

#[test]
fn path_items_never_show_on_the_empty_query() {
    assert!(e.search_with("", at(0, 0), &[file]).iter().all(|h| h.item.kind != ItemKind::File));
}

#[test]
fn path_item_that_does_not_match_is_dropped() {
    assert!(e.search_with("zzz", at(0, 0), &[file]).is_empty());
}

#[test]
fn path_item_matched_through_its_own_script() {
    // index holds only Latin apps; dynamic File "Отчёт.pdf"
    let hits = e.search_with("otchet", at(0, 0), &[report]);
    assert_eq!(titles(&hits), ["Отчёт.pdf"]);
}

#[test]
fn answers_still_lead() {
    // a non-path dynamic item (Command) still scores DYNAMIC_MATCH_SCORE and sorts first
}
```

And in `root_sections.rs` tests: `assert_eq!(kind_label(ItemKind::File), "File"); assert_eq!(kind_label(ItemKind::Folder), "Folder");`.

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test engine` — expected: compile errors (`ItemKind::File` unknown).
- [ ] **Step 3: Implement** the kinds, `is_path`, the engine rule above; `kind_section` puts both in `(2, "Files")`; fix any other exhaustive `ItemKind` matches the compiler names.
- [ ] **Step 4: Run** `cargo test -p tomakast-core --test engine && cargo test -p tomakast-ui root_sections` — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Core: file and folder items from a dynamic source rank as matches with usage"`.

---

### Task 2: Usage recorded from a module's screen

**Files:**
- Modify: `crates/core/src/effect.rs`, `crates/core/src/core.rs` (`take_core_effects`), `crates/ui/src/stack.rs` (exhaustive matches), `crates/plugin-host/src/caps.rs` (exhaustive matches)
- Test: `crates/core/tests/core.rs`

**Interfaces:**
- Produces: `Effect::RecordUsage { item: ItemId, query: String }` — internal, built-in modules only (no WIT case). The core applies it in `take_core_effects` and the UI never sees it: when `item.provider() == provider` it calls `record_usage(&item, &query, now)`; otherwise it is dropped with a `warn!`. `take_core_effects` gains a `now: LocalNow` parameter.

- [ ] **Step 1: Write the failing tests** in `crates/core/tests/core.rs`:

```rust
#[test]
fn record_usage_from_a_view_event_ranks_the_item() {
    // a fake module answers a view event with RecordUsage{ item: "m/x", query: "rep" }
    // afterwards a dynamic item m/x (kind File, title "report.txt") outranks an equal unopened one
}

#[test]
fn record_usage_for_another_provider_is_dropped() {
    // module "m" sends RecordUsage{ item: "apps/firefox", .. }: no usage recorded for apps/firefox
}

#[test]
fn record_usage_never_reaches_the_ui() {
    // the CoreUpdate::ViewEffects effects list does not contain RecordUsage
}
```

Also in `caps.rs` tests: a plugin answer containing `Effect::RecordUsage` is refused with `"record-usage is not a plugin effect"`.

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test core record_usage` — expected: FAIL (no variant).
- [ ] **Step 3: Implement** the variant, its handling, the `stack.rs` arms (ignored like `EditHome` where the UI lists internal effects; name `"RecordUsage"`), and the plugin-host refusal.
- [ ] **Step 4: Run** `cargo test -p tomakast-core -p tomakast-plugin-host -p tomakast-ui` — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Core: built-in modules record usage from their own screens"`.

---

### Task 3: Grid layout of a list view

**Files:**
- Modify: `crates/core/src/view.rs`, `crates/core/src/lib.rs` (export `ListLayout`)
- Modify: every `ListView { .. }` literal the compiler names (`crates/plugin-host/src/{convert.rs,caps.rs}`, `crates/plugin-host/tests/calc.rs`, `crates/ui/tests/*.rs`, `modules/{demo,history}/src/lib.rs`): add `layout: ListLayout::Rows`
- Modify: `crates/ui/src/views/list.rs`, `crates/ui/src/stack.rs` (an update may change `layout`), `crates/ui/src/launcher_view.rs` (left/right keys in a grid list)
- Test: `crates/ui/tests/launcher_view.rs` (or a new `crates/ui/tests/list_grid.rs` reusing `tests/support`)

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListLayout {
    #[default]
    Rows,
    /// Tiles of `columns` per row (0 means 1); each tile shows the row's icon large
    /// and the row's title as its caption.
    Grid { columns: u8 },
}
// ListView gains: pub layout: ListLayout,
```

UI behaviour in `Grid`: sections and the detail pane are not drawn; Up/Down move by `columns`, Left/Right move the selection by one (in a grid list they no longer move the caret; Home/End still do). An `UpdateView` that changes only `layout` (same revision) keeps the field's text and selects the same row id.

- [ ] **Step 1: Write the failing UI tests:**

```rust
#[gpui::test]
fn grid_list_moves_in_two_dimensions(cx: &mut TestAppContext) {
    // push a Module-search ListView with layout Grid{columns: 3} and 7 rows
    // Right, Right, Down -> selected row index 5; Left -> 4; Down -> 6 (short last row)
}

#[gpui::test]
fn switching_layout_keeps_query_and_selection(cx: &mut TestAppContext) {
    // type "ab" in a Rows list, select row "r2", the module answers ActionChosen with an
    // UpdateView of the same id/revision and layout Grid{columns:4}
    // -> the field still reads "ab", the selected row id is still "r2"
}

#[gpui::test]
fn plugin_lists_always_use_rows(...) // in plugin-host convert tests: converted ListView.layout == Rows
```

- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test launcher_view grid` — expected: compile failure (`layout` unknown).
- [ ] **Step 3: Implement** the type, the literal updates, and the grid rendering in `views/list.rs` (reuse the tile look and sizes of `views/grid.rs`; image from `row.icon`, caption `row.title`; keep the existing virtualisation for rows, a scrolled column of tile rows for the grid).
- [ ] **Step 4: Run** `cargo test -p tomakast-ui -p tomakast-plugin-host -p tomakast-core` — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Lists can be drawn as a grid of tiles; an update may switch the layout"`.

---

### Task 4: The `[files]` settings table

**Files:**
- Modify: `crates/core/src/settings.rs`, `crates/core/src/lib.rs` (export `FilesSettings`)
- Test: `crates/core/src/settings.rs` tests

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesSettings {
    pub in_root: bool,        // default true
    pub roots: Vec<String>,   // default vec!["~".into()]; raw, `~` not expanded here
    pub exclude: Vec<String>, // default empty
    pub hidden: bool,         // default false
}
impl Default for FilesSettings { .. }
// Settings gains: pub files: FilesSettings,
```

A malformed `[files]` table is reported like a malformed `[location]` (same error style); an unknown key is ignored with a `warn!`.

- [ ] **Step 1: Write the failing tests:** `files_defaults_without_a_table` (`Settings::default().files == FilesSettings::default()`, and parsing `""` gives the same); `files_table_parses` (`in_root = false`, `roots = ["~", "/data"]`, `exclude = ["~/Games", "*.iso"]`, `hidden = true` round-trip into the struct); `files_bad_type_is_an_error` (`in_root = "yes"` → `Settings::load` errs mentioning `files.in_root`).
- [ ] **Step 2: Run** `cargo test -p tomakast-core settings::tests::files` — expected: FAIL.
- [ ] **Step 3: Implement**, following how `location` is read in `RawSettings`.
- [ ] **Step 4: Run** the same — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Settings: the [files] table"`.

---

### Task 5: Module crate, names and the walked index

**Files:**
- Create: `modules/files/Cargo.toml`, `modules/files/src/{lib.rs,index.rs,rules.rs,names.rs}`
- Modify: root `Cargo.toml` (workspace member + `tomakast-module-files`, `ignore = "0.4"`, `globset = "0.4"`, `rayon = "1"`, `md-5 = "0.10"`, `serde_json = "1"` workspace deps)
- Test: unit tests in each file (`tempfile` dev-dep)

**Interfaces:**
- Produces (`names.rs`):

```rust
pub fn item_id(path: &Path) -> ItemId;            // files/<md5 hex of path bytes>
pub fn display_name(name: &OsStr) -> String;      // lossy UTF-8, hidden chars -> U+FFFD
pub fn display_parent(path: &Path, home: &Path) -> String; // "~/x/y", "/data/z"
pub fn file_uri(path: &Path) -> String;           // file:// + percent-encoding as apps' desktop_file_uri
```

- Produces (`rules.rs`):

```rust
pub struct Rules { /* expanded roots, absolute excluded prefixes, name GlobSet, hidden */ }
impl Rules {
    pub fn from_settings(s: &FilesSettings, home: &Path) -> Rules;
    pub fn roots(&self) -> &[PathBuf];
    /// Name-level and prefix-level exclusion (built-in names, `exclude` globs and paths,
    /// hidden when !hidden). Gitignore is applied by the walker, not here.
    pub fn excludes(&self, path: &Path, is_dir: bool) -> bool;
}
```

`exclude` entries starting with `~` or `/` (or a drive letter on Windows) are path prefixes; others are globs on the entry name.

- Produces (`index.rs`):

```rust
pub type EntryId = u32;
pub struct FileIndex { /* entries: Vec<Entry{name: Box<OsStr>, parent: EntryId, is_dir, mtime: i64, alive: bool}>,
                          dirs: HashMap<PathBuf, EntryId>, root ids, and the per-entry match data
                          search.rs needs (kept in step with every upsert/remove, so the index is
                          mutated in place and never rebuilt for a small change) */ }
impl FileIndex {
    pub fn walk(rules: &Rules) -> FileIndex;                 // parallel walk (ignore::WalkBuilder::build_parallel)
    pub fn len(&self) -> usize;                              // alive entries
    pub fn path(&self, id: EntryId) -> PathBuf;
    pub fn entry(&self, id: EntryId) -> EntryView<'_>;      // name, is_dir, mtime, depth
    pub fn ids(&self) -> impl Iterator<Item = EntryId> + '_; // alive
    pub fn upsert(&mut self, path: &Path, rules: &Rules);   // add or refresh one path (and walk it if a dir),
                                                             // through the same walker settings as `walk`
                                                             // (ancestors' ignore files apply)
    pub fn remove(&mut self, path: &Path);                  // marks the entry and its subtree dead
}
```

`same_file_system(true)` skips every mount under a root (none under this home today); a user adds a mounted disk as its own root. Walker settings: `hidden(!rules.hidden)`, `git_ignore(true)`, `ignore(true)`, `git_exclude(true)`, `parents(true)`, `follow_links(false)`, `same_file_system(true)`, `filter_entry(|e| !rules.excludes(..))`. Roots themselves are entries (so `~` folders show).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn item_id_is_stable_and_valid()       // same path -> same id; is_valid(); 38 bytes
#[test] fn id_and_title_of_hostile_names()      // 300-byte name, "\n", "\u{202e}", non-UTF-8 bytes (unix):
                                                // id valid; display_name has no hidden chars
#[test] fn walk_skips_hidden_ignored_and_builtin_names()
    // tmp/{a.txt, .secret/x, node_modules/m.js, proj/.gitignore("out/"), proj/out/o.bin, proj/src/main.rs}
    // -> paths == {tmp, a.txt, proj, proj/.gitignore? NO (hidden), proj/src, proj/src/main.rs}
#[test] fn hidden_true_includes_dot_entries()
#[test] fn exclude_by_path_and_by_glob()        // exclude = ["<tmp>/games", "*.iso"]
#[test] fn symlinks_listed_not_followed()       // link -> dir: link is an entry, its target's children are not under it
#[test] fn upsert_and_remove_subtree()          // remove(dir) -> its children gone from ids(); upsert(new file) -> present
#[test] fn upsert_respects_gitignore()          // proj/.gitignore "out/": upsert(proj/out/new.bin) -> not indexed
```

- [ ] **Step 2: Run** `cargo test -p tomakast-module-files` — expected: FAIL (crate stubs).
- [ ] **Step 3: Implement** `names.rs`, `rules.rs`, `index.rs`. Parallel walk: collect `(PathBuf, is_dir, mtime)` per thread, then insert sorted by path so parents precede children.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "files: covered folders, names and ids, the walked index"`.

---

### Task 6: Search over the index, and the bench

**Files:**
- Create: `modules/files/src/search.rs`, `modules/files/examples/bench.rs`
- Modify: `justfile` (recipe `files-bench`)
- Test: unit tests in `search.rs`

**Interfaces:**
- Consumes: `FileIndex`, `EntryId` (Task 5); `tomakast_core::{Matcher, Pattern, TextKeys, Scripts}`.
- Produces:

```rust
pub struct Signals<'a> {
    pub opened: &'a HashMap<PathBuf, Opened>, // opens through the launcher; Task 9 persists them
    pub recent: &'a HashSet<PathBuf>,         // KDE recently-used.xbel (Linux), else empty
    pub now: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Opened { pub count: u32, pub last: i64 } // defined here, persisted by state.rs (Task 9)
pub struct Found { pub id: EntryId, pub score: u32 }
pub fn search(index: &FileIndex, query: &str, limit: usize, signals: &Signals) -> Vec<Found>;
pub fn recent(index: &FileIndex, limit: usize, signals: &Signals) -> Vec<Found>; // empty-query screen
```

Matching: the query is matched against the entry name with the core matcher (`Pattern::across(query, scripts_of_all_names, ..)`, `score_text`). If the query contains a path separator, the part after the last separator matches the name and the part before must match `display_parent` (fuzzy). Ranking adds to the match score, in this order of weight: opened by the launcher (count, recency), in KDE's recent list, modified recently, shallower. Weights are the implementer's; the tests pin the orderings. Speed: any pre-filter must be exact (never drop a match), proven by `prefilter_never_drops_a_match`; scan with `rayon`, one `Matcher` per thread (`score_text` takes `&mut self`). Per-entry match data: the spec's "only where they differ from the plain name" — do not keep a full `TextKeys` per name if the bench shows it breaks the memory budget.

`recent()` order: opened by the launcher (most recent first), then KDE's recent list, then the most recently modified files.

- [ ] **Step 1: Write the failing tests** (build a `FileIndex` from a temp tree):

```rust
#[test] fn finds_by_fuzzy_name()                 // "mnrs" -> main.rs
#[test] fn finds_across_layouts_and_scripts()    // "ашку" -> fire.txt ; "otchet" -> Отчёт.pdf
#[test] fn separator_matches_the_parent()        // "proj/read" -> proj/x/README.md, not other/README.md
#[test] fn opened_beats_unopened_equal_match()
#[test] fn recent_list_beats_plain_equal_match()
#[test] fn shallower_beats_deeper_equal_match()
#[test] fn limit_is_respected()
#[test] fn prefilter_never_drops_a_match()       // 2000 generated names in Latin/Cyrillic/mixed, 50 queries incl.
                                                 // layout and translit ones: search(..usize::MAX) ids ==
                                                 // brute force over every entry with Matcher::score_text
#[test] fn recent_orders_opened_then_kde_then_modified()
```

- [ ] **Step 2: Run** `cargo test -p tomakast-module-files search` — expected: FAIL.
- [ ] **Step 3: Implement** `search.rs`.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Bench.** `examples/bench.rs` walks `$HOME` with default rules and prints: cold-or-warm walk time (as run), entry count, resident memory after building the `FileIndex` (from `/proc/self/status` `VmRSS` on Linux), the time to apply a burst of 5,000 `upsert`s and 5,000 `remove`s, and for each of 20 fixed queries (`fire`, `doc`, `readme`, `main.rs`, `proj/read`, `ашку`, `otchet`, `png`, `config`, `cargo`, `invoice`, `screen`, `notes`, `todo`, `zzzqx`, `.pdf`, `photo`, `downloads`, `rust`, `kde`) the search time over 30 runs (median, p95), and only the count of results — never file names. Recipe:

```make
# Time the file index and file search over your home folder (prints counts and timings, no file names).
files-bench:
    cargo run -q --release -p tomakast-module-files --example bench
```

Run `just files-bench`; expected: p95 ≤ 30 ms for every query and the index's resident memory ≤ 150 MB. If not, improve the pre-filter, parallelism or per-entry data before moving on.
- [ ] **Step 6: Commit** `git commit -m "files: search over the index; just files-bench"`.

---

### Task 7: Keeping the index fresh

**Files:**
- Create: `modules/files/src/live.rs`
- Test: unit tests in `live.rs`

**Interfaces:**
- Consumes: `Rules`, `FileIndex` (Task 5), `search::search` (Task 6), `tomakast_core::Settings` (Task 4).
- Produces:

```rust
#[derive(Clone)]
pub struct Live { /* Arc<RwLock<FileIndex>>, the current Rules, a stop flag */ }
impl Live {
    /// Spawns the `files-index` thread: first walk, then watch + re-walks; watches
    /// `config_file` and re-walks when `[files]` roots/exclude/hidden change.
    pub fn start(config_file: PathBuf, home: PathBuf, on_settings: impl Fn(&FilesSettings) + Send + 'static) -> Live;
    /// Runs `f` under the read lock. Never waits for a walk: a full re-walk builds a new
    /// FileIndex off the lock and swaps it in; watcher batches mutate in place under short
    /// write locks. Empty before the first walk.
    pub fn read<R>(&self, f: impl FnOnce(&FileIndex) -> R) -> R;
    pub fn stop(&self);
}
```

Behaviour: watcher events debounced 200 ms, applied with `upsert`/`remove` in place under a write lock per batch. Watches are **non-recursive, one per indexed folder** (never `RecursiveMode::Recursive` on a root: that would watch hidden and ignored trees too); a folder that appears gets its watch, a removed one loses it. Folder count over the watch cap → no watcher, re-walk every 5 minutes instead. Full re-walk every 30 minutes regardless. `on_settings` hands `in_root` to the module on every config change.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn created_file_is_found_after_the_debounce()   // write tmp/new.txt; within 2 s `read(|i| search(i, "new", ..))` finds it
#[test] fn removed_folder_leaves_the_index()
#[test] fn search_during_rewalk_uses_the_old_index()    // a slow-walk test hook: read() during the walk
                                                        // sees the previous index and returns at once
#[test] fn ignored_folder_is_not_watched()              // writes under proj/target/ cause no index change
#[test] fn config_change_of_roots_rewalks()             // rewrite config [files] roots -> new root's file found
#[test] fn in_root_change_reaches_the_module()          // on_settings called with in_root=false
```

- [ ] **Step 2: Run** `cargo test -p tomakast-module-files live` — expected: FAIL.
- [ ] **Step 3: Implement** `live.rs` (watcher via `notify::recommended_watcher`, `RecursiveMode::NonRecursive` per folder; the cap from `/proc/sys/fs/inotify/max_user_watches` on Linux).
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "files: watch, re-walk and follow config changes"`.

---

### Task 8: The module: root answers, actions, registration

**Files:**
- Modify: `modules/files/src/lib.rs`; Create: `modules/files/src/mime.rs`
- Modify: `crates/app/src/daemon.rs`, `crates/app/Cargo.toml`
- Test: unit tests in `lib.rs`, `mime.rs`

**Interfaces:**
- Consumes: `Live` (Task 7), `names` (Task 5), `search::search` (Task 6), `Effect::RecordUsage` (Task 2).
- Produces:

```rust
pub const PROVIDER: &str = "files";
pub struct FilesModule { .. }
pub struct FilesConfig {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,        // state.json lives here
    pub home: PathBuf,            // `~` of roots and subtitles
    pub thumbnail_root: PathBuf,  // $XDG_CACHE_HOME/thumbnails
    pub recent_file: Option<PathBuf>, // KDE recently-used.xbel (Linux)
}
impl FilesConfig { pub fn from_env(config_file: PathBuf, data_dir: PathBuf) -> FilesConfig; }
impl FilesModule { pub fn new(config: FilesConfig) -> FilesModule; }
// Tests always pass temp folders: no test walks the real home or writes the real thumbnail cache.
// mime.rs
pub struct MimeDb { .. }
impl MimeDb {
    pub fn system() -> MimeDb;                    // Linux: XDG data dirs' mime/globs2; else empty
    pub fn mime_of(&self, name: &OsStr) -> Option<&str>;
    pub fn icon_of(&self, mime: Option<&str>, is_dir: bool) -> Icon; // "folder", "image-png" (generic-icons fallback), "text-x-generic"
}
```

Module: `id() == "files"`, `is_dynamic_source() == true`; `start` publishes one static item `files/search` ("Search Files", kind Command, subtitle "Search files by name", icon `Named("system-search")`, action `open`) and starts `Live`. `search_with(query, _)`: empty unless `in_root` and ≥ 3 chars; top 3 from the searcher as `Item { kind: File|Folder, title: display_name, subtitle: Some(display_parent), keywords: vec![display path of the full path], icon, actions }`. The module remembers id → path for every item and row it handed out (bounded, last 2000).

Actions (ids and shortcuts):

| id | title | shortcut |
|---|---|---|
| `open` | Open | — (Enter) |
| `reveal` | Show in File Manager | `ctrl+shift+f` |
| `copy-path` | Copy Path | `ctrl+shift+c` |
| `open-with` | Open With… | `ctrl+o` (Linux only) |

Pin the shortcuts with a test in `crates/ui/src/shortcut.rs` tests: `file_search_shortcuts_are_usable` asserts `usable(..)` is `Some` for `ctrl+shift+f`, `ctrl+shift+c`, `ctrl+o`, `ctrl+g` (the last is Task 9's toggle).

`open`: `tomakast_platform::open_path`, records the path in the module's opened state (Task 9's `State::opened`; until Task 9, an in-memory map), returns `[CloseWindow]`. `reveal`: Linux via `tomakast_module_apps::DbusFileManager` `show_items([file_uri])`, falling back to opening the parent folder; macOS `open -R <path>`; Windows `explorer /select,<path>`; returns `[CloseWindow]`. `copy-path`: `[CopyText(path), Toast "Path copied"]`. A path that no longer exists: `Err("File not found: <display path>")` (the launcher shows the error toast) and `index.remove(path)` is queued. Registration in `daemon.rs` next to `AppsModule`, with `FilesConfig::from_env(paths.config_file, paths.data_dir.join("files"))`. Check that the daemon's own tests that call `start_modules()` do not register it (they would walk the real home).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn root_answers_need_three_characters()        // "fi" -> no items; "fir" -> items
#[test] fn root_answers_are_capped_at_three()
#[test] fn in_root_false_answers_nothing()
#[test] fn file_item_shape()                            // kind File, id = names::item_id, subtitle "~/..", actions ids in table order
#[test] fn folder_item_has_kind_folder()
#[test] fn copy_path_copies_and_toasts()
#[test] fn open_of_a_vanished_file_fails_with_a_toast() // run_action Err contains "File not found"
#[test] fn opens_a_file_with_a_newline_in_its_name()    // unix: the path handed to the opener (test seam) equals the real bytes
#[test] fn mime_icons()                                 // "a.png" -> Named("image-png"), dir -> Named("folder"), "x.unknownext" -> Named("text-x-generic")
```

Use a test seam for opening/revealing (a `trait Opener` with a recording fake), like `apps`' `reveal_with`.
- [ ] **Step 2: Run** `cargo test -p tomakast-module-files` — expected: FAIL.
- [ ] **Step 3: Implement** the module, `mime.rs`, and the daemon registration.
- [ ] **Step 4: Run** `cargo test -p tomakast-module-files -p tomakast-app` — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "files: root answers, open, reveal, copy path; registered in the daemon"`.

---

### Task 9: The "Search Files" screen and the module's state

**Files:**
- Create: `modules/files/src/screen.rs`, `modules/files/src/state.rs`
- Modify: `modules/files/src/lib.rs` (`run_action` for `files/search`, `on_view_event`)
- Test: unit tests in `screen.rs`, `state.rs`

**Interfaces:**
- Consumes: `ListView`, `ListLayout` (Task 3), `Effect::RecordUsage` (Task 2), `search::{search, recent}` (Task 6).
- Produces:

```rust
// state.rs — `<data_dir>/state.json`, written atomically (temp + rename)
#[derive(Serialize, Deserialize, Default)]
pub struct State { pub layout: Layout, pub opened: HashMap<PathBuf, Opened> } // opened capped at 2000, oldest dropped
#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
pub enum Layout { #[default] List, Grid }
// `Opened` comes from search.rs (Task 6).
impl State { pub fn load(dir: &Path) -> State; pub fn save(&self, dir: &Path) -> io::Result<()>; pub fn record_open(&mut self, path: &Path, now: i64); }
// screen.rs
pub const VIEW_ID: &str = "files/search";
pub fn view(query: &str, revision: u64, rows: Vec<ListRow>, layout: Layout) -> ListView;
```

View: title "Search Files", placeholder "Search files…", `SearchMode::Module`, at most 100 rows; row id = the item id's local part; row title = display name, subtitle = display parent, accessory `Text` = modified date (`"Today 14:05"`, `"Yesterday"`, `"12 Nov"`, `"12 Nov 2024"`), icon = MIME icon (grid: the thumbnail when Task 10 found one). Layout `Grid` → `ListLayout::Grid { columns: 5 }`. Each row's actions = the table of Task 8 plus `toggle-layout` titled "Show as Grid" / "Show as List" with shortcut `ctrl+g`.
Events: `QueryChanged { revision, text }` → `[UpdateView(view(text, revision, rows, layout))]`; `ActionChosen { action_id: "toggle-layout" }` → flip, save state, `[UpdateView(same query/revision, flipped layout)]`; `ActionChosen { action_id: "open", row_id }` → open, `record_open`, `[RecordUsage { item: files/<row_id>, query: current screen query }, CloseWindow]`; other actions as in Task 8; `Closed` → forget the screen's query.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn command_pushes_the_screen_in_the_saved_layout()
#[test] fn empty_query_lists_recent()
#[test] fn query_changed_answers_with_the_same_revision()
#[test] fn toggle_flips_and_persists_layout()          // state.json reloaded -> Grid
#[test] fn open_from_screen_records_usage_with_the_screen_query()
#[test] fn screen_has_no_minimum_and_caps_at_100()
#[test] fn state_survives_a_corrupt_file()             // garbage state.json -> State::default(), no panic
#[test] fn opened_is_capped_at_2000()
#[test] fn modified_date_labels()                      // fixed now/mtime pairs -> the four label forms
```

- [ ] **Step 2: Run** `cargo test -p tomakast-module-files screen state` — expected: FAIL.
- [ ] **Step 3: Implement**; make Task 8's opened map use `State`.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "files: the Search Files screen with a list and grid layout"`.

---

### Task 10: Thumbnails (Linux)

**Files:**
- Create: `modules/files/src/thumbs.rs`
- Modify: `modules/files/src/screen.rs`, `modules/files/src/lib.rs`, `modules/files/Cargo.toml` (`image = { version = "=0.25.10", default-features = false, features = ["png", "jpeg", "webp", "gif"] }`, `png` for text chunks if `image` does not expose them)
- Test: unit tests in `thumbs.rs`

**Interfaces:**
- Produces:

```rust
pub struct Thumbs { /* cache dir, background queue (one thread), ModuleContext for updates */ }
impl Thumbs {
    pub fn new(cache_root: PathBuf) -> Thumbs;                 // $XDG_CACHE_HOME/thumbnails
    /// A valid cached thumbnail (large first, then normal).
    pub fn cached(&self, path: &Path, mtime: i64) -> Option<PathBuf>;
    /// Queues generation for a supported image without one; `done` runs on the queue thread.
    pub fn request(&self, path: &Path, mtime: i64, done: impl FnOnce() + Send + 'static);
}
pub fn generate(path: &Path, mtime: i64, out: &Path) -> anyhow::Result<()>; // decode with limits, fit 256 px, write PNG with Thumb::URI and Thumb::MTime, 0600, temp + rename
```

In grid layout the screen uses `Icon::File(cached)` when there is one; otherwise it requests generation for supported images and, when one finishes while the screen is open, emits an unsolicited `UpdateView` of the current query/revision through `ModuleContext::emit` (at most one update per 250 ms, batching finished thumbnails).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn cached_thumbnail_found_by_uri_md5()
#[test] fn stale_thumbnail_is_ignored()                 // Thumb::MTime differs -> None
#[test] fn generates_png_with_spec_chunks()             // 1000x500 PNG -> 256x128, chunks URI and MTime, mode 0600
#[test] fn refuses_too_large_images()                   // header says 20000x20000 -> Err, nothing written
#[test] fn skips_unsupported_types()                    // .pdf without a cached thumbnail -> no request
```

- [ ] **Step 2: Run** `cargo test -p tomakast-module-files thumbs` — expected: FAIL.
- [ ] **Step 3: Implement** (`#[cfg(target_os = "linux")]` for generation; elsewhere `cached` returns `None` and `request` does nothing).
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "files: thumbnails in the grid from the shared cache, made for images when missing"`.

---

### Task 11: Open with… (Linux)

**Files:**
- Modify: `modules/apps/src/lib.rs`, `modules/apps/src/linux.rs` (`AppEntry::mime_types` from `MimeType=`; `pub fn launch_with_file(entry: &AppEntry, file: &Path) -> anyhow::Result<()>` via `gio launch <desktop-file> <file>`)
- Create: `modules/files/src/open_with.rs`; Modify: `modules/files/src/lib.rs`, `modules/files/Cargo.toml` (Linux dep on `tomakast-module-apps`)
- Test: unit tests in `open_with.rs`, `modules/apps/tests`

**Interfaces:**
- Consumes: `tomakast_module_apps::{scan, ScanOptions, AppEntry, launch_with_file}`, `MimeDb::mime_of` (Task 8).
- Produces:

```rust
pub const VIEW_ID: &str = "files/open-with";
/// Apps declaring `mime` (or its `generic` parent like `text/plain` for `text/*`), the default one first
/// (`xdg-mime query default <mime>`), then by title.
pub fn candidates(apps: &[(Item, AppEntry)], mime: &str, default: Option<&str>) -> Vec<(Item, AppEntry)>;
pub fn view(file: &Path, rows: Vec<ListRow>) -> ListView; // title "Open <name> With", CoreFilter search
```

`open-with` on a file row pushes the view; choosing a row (`open`) launches it with the file, records the open in `State::record_open`, and returns `[CloseWindow]`. It sends `RecordUsage` for the file only when Open With was started from the Search Files screen: from a root row, the root action already recorded usage. No app for the type → toast "No application opens this type". Not offered on macOS/Windows.

- [ ] **Step 1: Write the failing tests:** `mime_types_are_read` (apps: a desktop file with `MimeType=image/png;image/jpeg;` → `entry.mime_types == ["image/png","image/jpeg"]`); `candidates_default_first`; `candidates_empty_for_unknown_type`; `open_with_view_rows` (one row per candidate, ids are desktop ids).
- [ ] **Step 2: Run** `cargo test -p tomakast-module-apps -p tomakast-module-files open_with` — expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "files: open with, from the apps that declare the file's type"`.

---

### Task 12: Check, try and the acceptance note

**Files:**
- Create: `docs/experiments/2026-11-file-search-acceptance.md`
- Modify: `docs/superpowers/backlog.md` (File search row: point to the spec, plan and note)

- [ ] **Step 1:** Run `just check`; expected: all green. Fix what fails.
- [ ] **Step 2:** Run `just files-bench` (twice: the second run is warm); record entry count, walk times, resident memory, per-query median/p95.
- [ ] **Step 3:** Root-list checks without touching the user's session: a test in `crates/ui/tests` (or `modules/files` with `Core`) that registers `FilesModule` over a temp home with `fire.txt` and a fake `Firefox` app, types `fire`, and asserts Firefox first, then after three opens of `fire.txt` for `fire`, `fire.txt` first. Record the UI-thread time of `Core::search` and of applying the module's `SearchResults` (each answer triggers a re-rank on the UI thread), with and without the module on the same corpus, to show it stays within the 5 ms budget.
- [ ] **Step 4:** Write the note: setup, numbers (tables), the budgets each met or missed, what was not verified (macOS/Windows paths, a physical try on KDE; for the user to look at: file rows dropping out and coming back while typing, since each keystroke clears dynamic answers, and whether the selection jumps), and how to try: `just try` then type a file name; Ctrl+K → "Search Files"; Ctrl+G toggles the grid. No personal file names.
- [ ] **Step 5: Commit** `git commit -m "File search acceptance note"`.
