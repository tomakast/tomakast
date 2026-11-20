# Launcher Query History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The launcher keeps every query the user ran or looked at. ↑ on the first row at an empty query recalls them the way a shell does. A built-in "Search History" command lists them, newest first, with Delete Entry and Clear Search History.

**Architecture:**
- **Storage.** A new core table, `query_history`, arrives in schema v4. The migration creates it empty: history starts at the upgrade.
- **In memory.** The core's `QueryHistory` is a cheap-to-clone handle (`Arc<Mutex<…>>`) over the newest-first list and its own `Store` connection. The core records into it when an action succeeds. The UI reads it for ↑/↓ and records into it when the launcher hides. The built-in `history` module gets a clone, so all three share one list, on a file store and on an in-memory one alike.
- **Root query effect.** "Enter puts the query in the root search field" is a new internal effect, `Effect::SetRootQuery(String)`. It has no WIT case, so only built-in modules can send it.
- **Selection is unchanged.** The first root row starts selected at every query, as today, so Enter right after opening still runs the top suggestion. Recall starts from ↑ on that first row at an empty query.

**Tech Stack:** Rust 2024, rusqlite 0.43 (bundled SQLite), gpui-kit 0.10 (`InputState::set_value` / `select_all` / `selected_range`), uuid v4.

**Spec:** `docs/superpowers/specs/2026-11-19-query-history-design.md` (approved). Other inputs:
- the keyboard section of `docs/superpowers/specs/2026-11-19-widgets-design.md` (not approved). Only its key reservations apply here;
- "Storage" and the budgets in `docs/superpowers/specs/2026-11-18-tomakast-core-design.md`.

## Global Constraints

- **The plugin WIT does not change.** `crates/plugin-api/wit/**`, `FROZEN` and the `wit_parity` tables stay as they are. `convert.rs` maps WIT to core only, so a plugin cannot produce `Effect::SetRootQuery`. `caps.rs` still denies it explicitly, as defence in depth.
- **History is core data.** It is never offered to plugins: no import, no event, no effect carries it. The module is built in and registered through `register_native`, so no plugin can take the id `history`.
- **What is recorded.**
  - The query of every *successful* action, from the root list, its action panel and its shortcuts.
  - The root query when the launcher hides through `LauncherController::hide`, whether or not anything ran.
  - Not recorded: a query that, trimmed, has fewer than **2** chars (`chars().count()`, so `é` is one char and blanks count as empty). This also skips the empty query of hotkey and `tomakast run` runs, and a one-character query that ran.
  - Queries are stored **trimmed, case kept**, and deduplicated by exact text. The newest use wins and keeps the row's `id`.
  - Cap: **1000** entries, the newest kept.
  - Not recorded either: the query that ran the Search History command, both on that run and on the hide that follows. Otherwise the list would open with its own query on top. The core is told by provider (`Core::exempt_from_history("history")`), so it never needs to know module ids by itself.
- **Sync groundwork.** The table is `query_history(id TEXT PRIMARY KEY, query TEXT NOT NULL UNIQUE, at_unix INTEGER NOT NULL, modified_at INTEGER NOT NULL)`. `at_unix` and `modified_at` are Unix seconds, and `modified_at = at_unix` on every write. Deletes are hard deletes, as for `usage`: there are no tombstones.
- **Order.** Newest first is `at_unix DESC, rowid DESC`. A reuse rewrites the row (`INSERT OR REPLACE`), so it gets a new rowid. That is the tiebreak within one second.
- **Never log a query's text.** Queries are as sensitive as what the user types: log ids and counts only.
- **Budgets.**
  - Nothing touches SQLite on the open path. The history is loaded once, in `Core::new` and when the daemon wires the history.
  - ↑/↓ cost one mutex lock and a string clone, then the usual `search`, inside the 16 ms keystroke budget.
  - The hide-time write runs after the window is removed.
- **The up-arrow rule** (the user's call, revised; spec section "↑ in an empty search"):
  - with an empty query the first list row stays selected, as today;
  - ↑ on that first row recalls history;
  - while a recalled query is shown and not edited, ↑ goes back and ↓ goes forward; past the newest entry the field is empty again, with the first row selected.
- **Keys left free for the widget strip:** Tab at the root, and ↑ on the first row with a typed query (or an edited recalled one). That ↑ stays a no-op, as today.
- **Clear and Delete** (the user's call): Clear Search History and Delete Entry erase only `query_history`. `usage`, its queries included, is left alone.
- **CI commands (verbatim from `.github/workflows/ci.yml`):**
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo clippy -p tomakast-plugin-host --lib -- -D warnings`
  - `cargo test --workspace`
- **Commits.** Commit only by explicit path (`git add <paths>`): never `git add -A`, no stash, no push. Another session shares the worktree.

## Review Focus

- **Two queries recorded in the same second, then the older one reused** (the headless tests do this, and so does a fast user): ↑ must recall them newest first, and in the same order after a daemon restart. Test in Task 2 (`history_order_survives_a_reload_within_one_second`).
- **An action whose reply keeps the window open, then ↑ again**: the run moved its query to the front while a recall was showing. ↑ must still step to an older entry and must never crash on an index past the end. Test in Task 4 (`recall_survives_the_history_changing_underneath`).
- **Delete Entry or Clear Search History when the database write fails**: the entry must not vanish from the list while it stays on disk, or it would come back after a restart. Disk first, memory after. Test in Task 2 (`delete_keeps_memory_when_the_store_fails`).
- **↑ at an empty query from a row below the first**: it must move the selection up first, as today, and recall only once the first row is selected. Test in Task 4 (`up_from_a_lower_row_moves_before_recalling`).
- **A history query typed with surrounding blanks or in mixed case** (`  Firefox `): it is stored as `Firefox`, recalled as `Firefox`, and merges with a later `Firefox`, but not with `firefox`. Test in Task 2 (`record_trims_and_newest_use_wins_keeping_the_id`).

---

## File Structure

- `crates/core/src/history.rs` (new): `HistoryEntry`, the recording rule, `QueryHistory`.
- `crates/core/src/store.rs`: schema v4, the `query_history` statements, the busy timeout.
- `crates/core/src/core.rs`: owns a `QueryHistory` and records successful runs.
- `crates/core/src/effect.rs`: `Effect::SetRootQuery`.
- `crates/plugin-host/src/caps.rs`: denies `SetRootQuery`, and an icon-walk arm.
- `crates/ui/src/stack.rs`: applies `SetRootQuery`.
- `crates/ui/src/launcher_view.rs`: recall, `set_root_query`, `root_query`, the test accessors.
- `crates/ui/src/window.rs`: records on hide.
- `modules/history/` (new crate `tomakast-module-history`): the "Search History" command and its list view.
- `crates/app/src/daemon.rs`: opens the history and registers the module.

---

### Task 1: Store schema v4: an empty `query_history`

**Files:**
- Create: `crates/core/src/history.rs` (this task: constants, `HistoryEntry`, `recordable` only)
- Modify: `crates/core/src/store.rs`, `crates/core/src/lib.rs`
- Test: `crates/core/src/store.rs` (unit tests), `crates/core/tests/store.rs`

**Interfaces:**
- Produces, in `crates/core/src/history.rs`:
  - `pub const HISTORY_CAP: usize = 1000;`
  - `pub const MIN_QUERY_CHARS: usize = 2;`
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct HistoryEntry { pub id: String, pub query: String, pub at_unix: i64 }`
  - `pub(crate) fn recordable(query: &str) -> Option<&str>`: `query.trim()` if it has at least `MIN_QUERY_CHARS` chars, else `None`.
- Produces, in `crates/core/src/store.rs`:
  - `pub const SCHEMA_VERSION: i64 = 4;`
  - `pub fn upsert_query(&self, query: &str, at_unix: i64) -> anyhow::Result<String>`: returns the row's id, which stays the same on a reuse.
  - `pub fn load_query_history(&self, limit: usize) -> anyhow::Result<Vec<HistoryEntry>>`: newest first.
  - `pub fn delete_query(&self, id: &str) -> anyhow::Result<bool>`
  - `pub fn clear_query_history(&self) -> anyhow::Result<usize>`: the number of rows deleted.
  - `pub fn prune_query_history(&self, keep: usize) -> anyhow::Result<usize>`: deletes all but the newest `keep` rows and returns the number deleted.
- `lib.rs`: `pub mod history;` and `pub use history::{HISTORY_CAP, HistoryEntry, MIN_QUERY_CHARS};`. `QueryHistory` is added in Task 2.
- The store methods do not apply the length rule: the caller (`QueryHistory`) does.

- [ ] **Step 1: Write the failing tests**

In the `store.rs` unit tests:
- Rename `fresh_store_is_v3` to `fresh_store_is_v4`. It asserts `version == 4`, and that `tables(&store.conn)` contains `"query_history"`.
- Rename `store_migrates_v1_to_v3_keeping_usage` to `store_migrates_v1_to_v4_keeping_usage`. It asserts `version == 4`. The usage row is kept, and the history is empty: `store.load_query_history(1000)` is `[]`. Nothing is seeded from `usage`.
- `store_migrates_v3_to_v4_with_an_empty_history`:
  - build a v3 file (`SCHEMA_V1`, `SCHEMA_V2`, `SCHEMA_V3`, `user_version = 3`) with usage rows `("apps/firefox", "Fi", 100)` and `("apps/files", "files", 200)`;
  - open the store: `version == 4`, `load_usage(0).len() == 2` and `load_query_history(1000)` is empty.

In `crates/core/tests/store.rs`:

```rust
#[test]
fn upsert_query_keeps_the_id_and_moves_the_query_first() {
    let store = Store::open_in_memory().unwrap();
    let aa = store.upsert_query("aa", 100).unwrap();
    store.upsert_query("bb", 100).unwrap();
    assert_eq!(store.upsert_query("aa", 100).unwrap(), aa);
    let rows = store.load_query_history(1000).unwrap();
    let queries: Vec<_> = rows.iter().map(|e| e.query.as_str()).collect();
    assert_eq!(queries, ["aa", "bb"]);
    assert_eq!(rows[0].id, aa);
}

#[test]
fn delete_and_clear_query_history() {
    // upsert "aa"@1, "bb"@2:
    //   delete_query(aa) == true, then false;
    //   clear_query_history() == 1;
    //   load_query_history(1000) is empty.
}

#[test]
fn prune_keeps_the_newest() {
    // upsert "q0".."q4" at 0..4: prune_query_history(2) == 3; load gives ["q4", "q3"].
}

#[test]
fn file_store_keeps_query_history_across_reopen() {
    // upsert "aa"@5 on a file store, drop it, reopen: load == [HistoryEntry { id, query: "aa", at_unix: 5 }].
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tomakast-core store`
Expected: FAIL. The new methods are not defined (compile errors).

- [ ] **Step 3: Implement**

`SCHEMA_V4`:

```sql
CREATE TABLE query_history(
  id TEXT PRIMARY KEY, query TEXT NOT NULL UNIQUE,
  at_unix INTEGER NOT NULL, modified_at INTEGER NOT NULL);
CREATE INDEX query_history_at ON query_history(at_unix);
```

- In `Store::init`, when `version < 4`:
  - run `SCHEMA_V4`, and nothing else: the table starts empty, with no seed from `usage`.
- `upsert_query` is one statement. The REPLACE is deliberate: it gives the row a new rowid, which is the same-second tiebreak.

```sql
INSERT OR REPLACE INTO query_history(id, query, at_unix, modified_at)
VALUES (COALESCE((SELECT id FROM query_history WHERE query = ?2), ?1), ?2, ?3, ?3)
RETURNING id
```

  `?1` is a fresh uuid v4.
- `load_query_history`: `ORDER BY at_unix DESC, rowid DESC LIMIT ?1`.
- `prune_query_history`: `DELETE FROM query_history WHERE id NOT IN (SELECT id FROM query_history ORDER BY at_unix DESC, rowid DESC LIMIT ?1)`.
- `Store::open` also sets `conn.busy_timeout(Duration::from_secs(1))`. From Task 5 on, the history holds a second connection to the same file.
- Update the module doc to "SQLite store for usage and query history."

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tomakast-core store` and `cargo test -p tomakast-plugin-host db`. `PluginDb` checks the shared `SCHEMA_VERSION`.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/history.rs crates/core/src/store.rs crates/core/src/lib.rs crates/core/tests/store.rs
git commit -m "feat(core): schema v4 query_history table"
```

---

### Task 2: `QueryHistory`, and the core records successful runs

**Files:**
- Modify: `crates/core/src/history.rs`, `crates/core/src/core.rs`, `crates/core/src/lib.rs`
- Test: `crates/core/tests/history.rs` (new), `crates/core/tests/core.rs`

**Interfaces:**
- Consumes: Task 1's `Store` methods, `recordable`, `HISTORY_CAP` and `HistoryEntry`.
- Produces, in `crates/core/src/history.rs`:

```rust
/// The launcher's query history: newest first, at most HISTORY_CAP, shared by clones.
#[derive(Clone)]
pub struct QueryHistory { inner: Arc<Mutex<Inner>> }   // Inner { store: Store, entries: Vec<HistoryEntry> }

impl QueryHistory {
    /// Prunes `store` to HISTORY_CAP, then loads it newest first.
    pub fn load(store: Store) -> anyhow::Result<QueryHistory>;
    /// Records `query` used at `at_unix` if `recordable`; returns whether it did.
    /// A store error is logged (no query text) and the change stays in memory.
    pub fn record(&self, query: &str, at_unix: i64) -> bool;
    pub fn entries(&self) -> Vec<HistoryEntry>;
    /// The query `back` steps from the newest (0 = newest).
    pub fn nth_query(&self, back: usize) -> Option<String>;
    /// Disk first; memory only after the store succeeded.
    pub fn delete(&self, id: &str) -> anyhow::Result<bool>;
    pub fn clear(&self) -> anyhow::Result<()>;
}
```

- A poisoned lock is recovered with `unwrap_or_else(PoisonError::into_inner)`, as `plugins_module.rs` does.
- Produces, in `Core` (`crates/core/src/core.rs`):
  - `pub fn history(&self) -> &QueryHistory`
  - `pub fn set_history(&mut self, history: QueryHistory)`
  - `pub fn exempt_from_history(&mut self, provider: &str)`: a successful run of an item of `provider` records nothing, and remembers its query as `skip_on_hide: Option<String>`.
  - `pub fn record_hidden_query(&mut self, query: &str, now: LocalNow) -> bool`: the hide path. It takes `skip_on_hide`, so it applies to one hide only. If the taken value equals `query`'s trimmed text, it records nothing and returns `false`. Otherwise it returns `self.history.record(query, now.unix)`.
  - `Core::new` keeps its signature. Its default is `QueryHistory::load(Store::open_in_memory()?)?`, so the 13 existing callers and every test core get a working in-memory history.
- `lib.rs`: also export `QueryHistory`.

- [ ] **Step 1: Write the failing tests**

`crates/core/tests/history.rs`. The helper `fn mem() -> QueryHistory` wraps `QueryHistory::load(Store::open_in_memory().unwrap()).unwrap()`, and `fn queries(h) -> Vec<String>` gives the queries newest first.

- `record_skips_blank_and_one_character_queries`:
  - `record` returns `false` for `""`, `"a"`, `" a "`, `"é"` and `"   "`;
  - it returns `true` for `"ab"`;
  - afterwards `queries == ["ab"]`.
- `record_trims_and_newest_use_wins_keeping_the_id`:
  - record `"  Firefox "`@10, `"firefox"`@11, `"Firefox"`@12;
  - `queries == ["Firefox", "firefox"]`;
  - the `"Firefox"` entry's id equals the id after the first record.
- `history_order_survives_a_reload_within_one_second`:
  - on a file store, record `"aa"`@100, `"bb"`@100, `"aa"`@100, `"cc"`@100;
  - `queries == ["cc", "aa", "bb"]`;
  - reloaded with `QueryHistory::load(Store::open(&path))` it is the same.
- `record_keeps_the_newest_1000`:
  - on a file store, record `"q{i}"`@`i` for `i in 0..1001`;
  - `entries().len() == 1000`, `nth_query(0) == Some("q1000")` and `nth_query(999) == Some("q1")`;
  - after a reload, the same, and the store holds 1000 rows (`load_query_history(2000).len() == 1000`).
- `load_prunes_a_store_beyond_the_cap`: `upsert_query("q{i}", i)` for `i in 0..1005` straight into a file store. `QueryHistory::load` holds 1000, and a second `Store::open` of the file also loads 1000.
- `nth_query_counts_back_from_the_newest`: record `"aa"`@1 and `"bb"`@2. `nth_query(0) == Some("bb")`, `nth_query(1) == Some("aa")` and `nth_query(2) == None`.
- `delete_removes_the_entry_from_memory_and_disk`:
  - `delete(id_of_aa) == Ok(true)`, then `Ok(false)`;
  - `queries == ["bb"]`, and after a reload too.
- `clear_empties_memory_and_disk`: after `clear()`, `entries()` is empty, and after a reload too.
- `clear_and_delete_leave_usage_alone`: on a file store holding one usage record (`store.record_usage`), record `"aa"` and `"bb"`, `delete(id_of_aa)`, then `clear()`. A second `Store::open` still has `load_usage(0).len() == 1`.
- `delete_keeps_memory_when_the_store_fails`:
  - on a file store, record `"aa"` and `"bb"`;
  - break the table from a second `rusqlite::Connection` on the same file: `DROP TABLE query_history`;
  - `delete(id_of_aa)` is `Err`, then `clear()` is `Err`;
  - `queries == ["bb", "aa"]` after both.
- `clones_share_one_history`: `let b = a.clone(); a.record("aa", 1);` then `b.nth_query(0) == Some("aa")`.

`crates/core/tests/core.rs`. It reuses `core_with`, `drain` and `execute`.
- `successful_action_records_the_query_as_typed`: `execute(&mut core, &beta, "Be")`, then `core.history().nth_query(0) == Some("Be".into())`. Usage lowercases it; the history does not.
- `failed_action_does_not_record_the_query`: with `fail`, `execute(.., "al")`, then `core.history().entries()` is empty.
- `run_with_an_empty_query_records_nothing`: `execute(.., "")`, then `entries()` is empty.
- `exempt_provider_runs_are_not_recorded`: `core.exempt_from_history("fake")`, then `execute(.., "be")`. `entries()` is empty, and usage is still recorded (`store.load_usage(0).len() == 1`).
- `hide_after_an_exempt_run_skips_its_query_once`:
  - exempt `"fake"` and `execute(.., "hist")`;
  - `record_hidden_query("hist", now())` is `false`, and `entries()` is empty;
  - a second `record_hidden_query("hist", now())` is `true`. It is a later hide, so the query is recorded.
- `hide_after_an_exempt_run_records_an_edited_query`: exempt `"fake"`, `execute(.., "hist")`, then `record_hidden_query("firefox", now())` is `true` and `nth_query(0) == Some("firefox")`.
- `record_hidden_query_applies_the_length_rule`: `record_hidden_query("a", now())` is `false` and `record_hidden_query("ab", now())` is `true`.
- `set_history_replaces_the_default_history`: `let h = QueryHistory::load(Store::open_in_memory().unwrap()).unwrap(); core.set_history(h.clone());` then `execute(.., "be")`, and `h.nth_query(0) == Some("be".into())`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tomakast-core --test history --test core`
Expected: FAIL (`QueryHistory` is not defined).

- [ ] **Step 3: Implement**

- `record`, in order:
  1. `recordable`;
  2. move the existing entry with the same query to the front, or insert a new one at index 0;
  3. truncate to `HISTORY_CAP`;
  4. `store.upsert_query`, taking the returned id as the entry's id;
  5. if an entry was dropped by the cap, `store.delete_query(dropped.id)`.

  Errors are `tracing::error!` with no query text.
- In `Core::apply_from`, in `ActionFinished` `Ok`, after `record_usage`: if `item.provider()` is exempt, set `skip_on_hide = recordable(&query)` (owned). Otherwise call `self.history.record(&query, now.unix)` and clear `skip_on_hide`, because a later run replaces the exempt one as "the run before this hide". The exempt providers are a `HashSet<String>` on `Core`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tomakast-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/history.rs crates/core/src/core.rs crates/core/src/lib.rs crates/core/tests/history.rs crates/core/tests/core.rs
git commit -m "feat(core): QueryHistory; successful runs record their query"
```

---

### Task 3: Internal effect `Effect::SetRootQuery`

**Files:**
- Modify: `crates/core/src/effect.rs`, `crates/core/tests/wit_parity.rs` (a comment only), `crates/plugin-host/src/caps.rs`, `crates/ui/src/stack.rs`, `crates/ui/src/launcher_view.rs`
- Test: `crates/plugin-host/src/caps.rs` (unit tests), `crates/ui/tests/stack.rs`

**Interfaces:**
- Produces: `Effect::SetRootQuery(String)`. Doc comment:

  > Internal, for built-in modules: there is no WIT case, so plugins cannot send it. It pops every pushed view (each gets `Closed`, as for Esc), puts the text in the root search field with the caret at its end, and runs the search. It is honoured in a reply to a request of the open window, and ignored unsolicited or from a daemon run.

  Add the matching line to the unsolicited-effects paragraph of the `Effect` doc.
- Produces: `pub(crate) fn set_root_query(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>)` on `LauncherView`. It:
  1. closes the panel;
  2. calls `pop_view` until the stack is empty;
  3. calls `input.set_value(text, window, cx)`, which emits no `InputEvent::Change` and leaves the caret at the end;
  4. clears any recall state (Task 4 adds that state; until then there is nothing to clear);
  5. calls `self.search(cx)` itself, because `set_value` emitted no `Change`.
- Produces, for tests:
  - `#[doc(hidden)] pub fn query_selection(&self, cx: &App) -> std::ops::Range<usize>`, which is `self.input.read(cx).selected_range()`;
  - `#[doc(hidden)] pub fn root_selection(&self) -> Option<String>`, the title of the selected root hit. The test helper `fn root_selection(cx)` downcasts the window root to `LauncherView`, as `pushed_ids` does in `stack.rs`.
- `wit_parity.rs`: next to the `"effect"` row of `TYPES`, add the comment `// core Effect::SetRootQuery is internal (built-in modules) and has no WIT case.`

- [ ] **Step 1: Write the failing tests**

In the `caps.rs` tests:

```rust
#[test]
fn set_root_query_is_never_a_plugin_effect() {
    let checked = gate(&all(), &mut Rates::default(), Origin::Answer, Instant::now(),
        vec![Effect::SetRootQuery("x".into()), toast()]);
    assert_eq!(checked.effects, [toast()]);
    assert_eq!(checked.denied, ["set-root-query is not a plugin effect"]);
}
```

In `crates/ui/tests/stack.rs`:
- `set_root_query_pops_views_and_searches`:
  - a module `FakeModule::new(items(), push_sub)` whose view handler answers `ActionChosen` with `Ok(vec![Effect::SetRootQuery("fir".into())])`;
  - `pushed(cx, module)`, then `press(cx, "enter")` and `deliver`;
  - `pushed_ids(cx)` is empty, `query_value(cx) == "fir"` and `row_titles(cx) == ["Firefox"]`;
  - the module's `view_events` end with `(ViewId("fake/sub"), ViewEvent::Closed)`;
  - the caret is at the end: `query_selection == 3..3`, and `root_selection == Some("Firefox")`;
  - `press(cx, "enter")` then runs Firefox: `calls` ends with `(fake/firefox, "run")`.
- `set_root_query_over_two_views_closes_both`: a push of `fake/sub`, then a second view (reuse `push_sub_then_x` / `history_and_sheet`), then `SetRootQuery`. Both views get `Closed`, top first, and `pushed_ids` is empty.
- `unsolicited_set_root_query_is_ignored`:
  - after `pushed`, emit through the module's stored `ctx`: `ctx.emit(vec![Effect::SetRootQuery("x".into())])`, then `deliver`;
  - `pushed_ids == ["fake/sub"]`;
  - after `press(cx, "escape")`, the root `query_value == ""`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tomakast-plugin-host --lib caps` and `cargo test -p tomakast-ui --test stack set_root_query`
Expected: FAIL (no such variant).

- [ ] **Step 3: Implement**

- `caps.rs`, in the `denied` match: `Effect::SetRootQuery(_) => Some("set-root-query is not a plugin effect")`. In `for_each_icon`, add `| Effect::SetRootQuery(_)` to the no-icon arm.
- `stack.rs`:
  - `apply_effects` gets `Effect::SetRootQuery(text) if window.is_some() => on(window, cx, |host, window, cx| host.set_root_query(text, window, cx))`;
  - `apply_unsolicited` adds it to the `PopView | CloseWindow` "unsolicited effect ignored" arm;
  - `name()` returns `"SetRootQuery"`;
  - a daemon-origin reply falls through to the existing "effect without a window ignored" arm.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tomakast-plugin-host --lib`, `cargo test -p tomakast-ui --test stack` and `cargo clippy --workspace --all-targets -- -D warnings`. Clippy catches any exhaustive `match` on `Effect` missed above, tests included.
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/effect.rs crates/core/tests/wit_parity.rs crates/plugin-host/src/caps.rs crates/ui/src/stack.rs crates/ui/src/launcher_view.rs crates/ui/tests/stack.rs
git commit -m "feat(ui): internal SetRootQuery effect puts a query in the root field"
```

---

### Task 4: ↑/↓ recall in the root field, and recording on hide

**Files:**
- Modify: `crates/ui/src/launcher_view.rs`, `crates/ui/src/window.rs`
- Test: `crates/ui/tests/launcher_view.rs`

**Interfaces:**
- Consumes: `Core::history()`, `Core::record_hidden_query` and `QueryHistory::{nth_query, record}` (Task 2); `root_selection` and `query_selection` (Task 3).
- New state on `LauncherView`: `recall: Option<Recall>`, where `struct Recall { back: usize, text: String }`. `text` is what was put in the field.
- Produces: `pub(crate) fn root_query(&self, cx: &App) -> String`, the root field's value.
- New private `fn on_first_row(&self) -> bool`: the selection is the first listed hit (`rows.iter().find_map(RootRow::hit)`), or nothing is listed.
- The state machine. Root list only: no pushed view, no panel. Recall takes precedence over the list. The selection stays a `usize`, and the initial selection is unchanged:

| State | ↑ | ↓ |
|---|---|---|
| `recall = Some(r)` | `nth_query(r.back + 1)`: `Some(q)` shows it, `None` is a no-op | `r.back == 0`, or `nth_query(r.back - 1)` is `None`: clear the field (`set_value("")`), `recall = None`, `search`, which selects the first row as for any new query. Otherwise show `r.back - 1` |
| `recall = None`, empty query, `on_first_row()` | `nth_query(0)`: `Some(q)` shows it with `back = 0`, `None` is a no-op | `step_root(true)` (as today) |
| anything else | `step_root(false)`, a no-op on the first row (left for the strip) | `step_root(true)` |

- To show `(back, q)`: `set_value(q)`, `select_all` (so typing replaces it), `recall = Some(Recall { back, text: q })`, then `search(cx)` called directly. The query is not empty, so the first row is selected and Enter runs it.
- `search()` starts with this: if `recall` is `Some` and the field's value differs from `recall.text`, set `recall = None`. A user edit is the only thing that emits `InputEvent::Change` with a different value, so editing a recalled query makes it a new query.
- Recording on hide is in `LauncherController::hide` only (one hook):
  1. read `view.root_query(cx)` inside the same `handle.update` that calls `close_all` and `remove_window`;
  2. after it returns `Ok`, call `self.core.try_borrow_mut()`. On `Ok`, run `core.record_hidden_query(&query, LocalNow::now())`, which skips the query that just ran Search History (Task 2). On `Err`, `tracing::warn!` "core busy; query not recorded".

  A compositor close skips recording, and so does a run's `CloseWindow` arriving after the run already recorded it (dedup makes the second record harmless).

- [ ] **Step 1: Write the failing tests**

In `launcher_view.rs`, each test fills the history through `launcher.core.borrow().history().record(..)`.
- `up_in_the_empty_field_recalls_the_newest_query`:
  - record `"files"`@100 and `"firefox"`@200, then ↑;
  - `query_value == "firefox"` and `query_selection == 0..7`;
  - `row_titles == ["Firefox"]` and `root_selection == Some("Firefox")`.
- `up_again_steps_back_and_stops_at_the_oldest`: same history; ↑, ↑ gives `"files"`; another ↑ still gives `"files"`.
- `down_steps_forward_then_back_to_the_empty_field`:
  - same history; ↑, ↑, ↓ gives `"firefox"`;
  - another ↓ gives `query_value == ""`, `root_selection == Some("Files")` and `row_titles == ["Files", "Firefox", "Terminal"]`;
  - another ↓ moves in the list: `root_selection == Some("Firefox")`;
  - ↑ back to Files, then ↑ recalls again: `query_value == "firefox"`.
- `typing_over_a_recalled_query_makes_a_new_query`:
  - record `"firefox"`; ↑, then `type_text("te")`;
  - `query_value == "te"`;
  - ↑ then moves in the list (no-op on the first row), so `query_value == "te"`;
  - Enter runs `fake/terminal`.
- `enter_on_a_recalled_query_runs_its_first_row`: record `"firefox"`; ↑, Enter, `deliver`. `calls == [(fake/firefox, "run")]`.
- `empty_query_still_selects_the_first_row`: with a history recorded, `root_selection == Some("Files")` right after `show`, and Enter runs `fake/files`. The top suggestion still runs on Enter.
- `up_with_no_history_does_nothing`: ↑ gives `query_value == ""` and `root_selection == Some("Files")`.
- `up_from_a_lower_row_moves_before_recalling`: record `"firefox"`; ↓ (Firefox), then ↑ gives `query_value == ""` and `root_selection == Some("Files")`; another ↑ gives `query_value == "firefox"`.
- `editing_then_clearing_a_recalled_query_recalls_again`: record `"firefox"`; ↑, `type_text("x")`, `press(cx, "backspace")` gives `query_value == ""`; ↑ gives `"firefox"`.
- `up_on_a_typed_query_moves_in_the_list`: record `"files"`; type `"f"`, ↓, ↑ gives `query_value == "f"`, and Enter runs `fake/files`.
- `recall_survives_the_history_changing_underneath`:
  - module result `|| Ok(vec![])`, so the window stays open;
  - record `"files"`@1, `"firefox"`@2, `"terminal"`@3;
  - ↑ (terminal), ↑ (firefox, `back = 1`), Enter, `deliver`. The history is now `[firefox, terminal, files]`, and the recall is unchanged because the field was not edited;
  - ↑ gives `query_value == "files"` (live index 2), and another ↑ stays on `"files"` (index 3 is past the end). No panic.
- `hide_after_an_exempt_run_skips_its_query`:
  - module result `|| Ok(vec![])`, and `launcher.core.borrow_mut().exempt_from_history("fake")`;
  - type `"fi"`, Enter, `deliver`, then `hide_launcher(cx)`;
  - `history().entries()` is empty.
- `hide_records_the_typed_query`: type `"fi"`, then `hide_launcher(cx)`. `history().nth_query(0) == Some("fi")`.
- `escape_records_the_query`: type `"term"`, then Escape. `nth_query(0) == Some("term")`.
- `hide_skips_a_one_character_query`: type `"f"`, then hide. `history().entries()` is empty.
- `run_then_close_records_the_query_once`: type `"fir"`, Enter (the `hide` result), `deliver`. `history().entries().len() == 1` and `nth_query(0) == Some("fir")`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tomakast-ui --test launcher_view`
Expected: FAIL. ↑ does not recall, and hide records nothing.

- [ ] **Step 3: Implement**

The table above goes in `select_prev` / `select_next` (the `None =>` root arm). `search`, `reselect` and the rest of the selection code are unchanged. Helpers:
- `fn recall_older(&mut self, window, cx)`
- `fn recall_newer(&mut self, window, cx)`
- `fn show_recalled(&mut self, back: usize, text: String, window, cx)`

`set_root_query` (Task 3) sets `recall = None`. Each ↑/↓ reads the history with a short `self.core.borrow()`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tomakast-ui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/launcher_view.rs crates/ui/src/window.rs crates/ui/tests/launcher_view.rs
git commit -m "feat(ui): up on the first row recalls query history at an empty query; hide records the query"
```

---

### Task 5: The `history` built-in module and the daemon wiring

**Files:**
- Create: `modules/history/Cargo.toml`, `modules/history/src/lib.rs`
- Modify: `Cargo.toml` (add a workspace member and `tomakast-module-history = { path = "modules/history" }`), `crates/app/Cargo.toml`, `crates/app/src/daemon.rs`
- Test: `modules/history/src/lib.rs` (unit tests)

**Interfaces:**
- Consumes: `QueryHistory` (Task 2) and `Effect::SetRootQuery` (Task 3).
- Produces, in `modules/history/src/lib.rs`:

```rust
pub const PROVIDER: &str = "history";
/// Unix seconds now; injectable for tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
pub struct HistoryModule { history: QueryHistory, clock: Clock }
impl HistoryModule {
    pub fn new(history: QueryHistory) -> HistoryModule;        // system clock
    pub fn with_clock(history: QueryHistory, clock: Clock) -> HistoryModule;
}
impl Module for HistoryModule { /* id, start (replace_items), run_action, on_view_event */ }
/// "just now" (< 60 s, or in the future), "{m} min ago" (< 1 h), "{h} h ago" (< 24 h), "{d} d ago".
pub fn ago(now_unix: i64, at_unix: i64) -> String;
```

- The pinned values:
  - **The item:** `history/search-history`, title `"Search History"`, subtitle `"Queries you typed in the launcher"`, keywords `["history", "recent", "queries"]`, `ItemKind::Command`. Its one action is `("open", "Open")`, and `source` is `None`.
  - **The view:** a `View::List` with id `ViewId("history")`, title `"Search History"`, placeholder `"Filter search history…"`, `SearchMode::CoreFilter`, query `""`, revision `0` and `show_detail: false`.
  - **The rows:** `history.entries()` newest first. Each has id = entry id, title = the query, subtitle = `ago(clock(), at_unix)`, no section, icon, accessories or detail.
  - **The row actions**, in order:
    1. `("search", "Search", None)`;
    2. `("delete", "Delete Entry", Some("ctrl+shift+x"))`;
    3. `("clear", "Clear Search History", None)`. It has no shortcut on purpose: it is in the panel only.
  - **`on_view_event(ActionChosen { row_id, action_id })`:**
    - `search` gives `[SetRootQuery(query)]`;
    - `delete` gives `[UpdateView(the list rebuilt)]`;
    - `clear` gives `[UpdateView(the list with no rows), Toast(Success, "Search history cleared", None)]`;
    - a row id that is no longer in the history, for `search` or `delete`, gives `[UpdateView(the list rebuilt), Toast(Error, "This entry is no longer in the history", None)]`;
    - a store error gives `Toast(Error, "Cannot change the search history", Some(format!("{e:#}")))`;
    - an unknown action id is `bail!`.
  - Other view events give `Ok(vec![])`.
- Daemon (`crates/app/src/daemon.rs`), right after `Core::new`:
  1. open `Store::open(&paths.db_file)` for the history and `QueryHistory::load` it. On success, `core.set_history(history)`. On error, `error!("search history is not kept: {e:#}")` and the core's in-memory default stays;
  2. then `let history = core.history().clone();` and `register_native(&mut core, Box::new(HistoryModule::new(history)))`. Bind it first: the closure takes `&mut core`;
  3. `core.exempt_from_history(tomakast_module_history::PROVIDER)`, so the query that opened Search History is not recorded.

  The provider is not pinned (`pin_provider`): the command ranks like any other item.

- [ ] **Step 1: Write the failing tests**

In `modules/history/src/lib.rs` `#[cfg(test)]`. The helpers:
- `fn history() -> QueryHistory` (in-memory);
- `fn module(h, now: i64) -> HistoryModule`, which uses `with_clock(h, Arc::new(move || now))`;
- `fn open(m) -> ListView`, which runs `run_action(search_item, "open")` and unwraps `[PushView(View::List(v))]`.

The tests:
- `ago_formats_minutes_hours_and_days`:
  - `ago(1000, 1000) == "just now"`, `ago(1059, 1000) == "just now"` and `ago(900, 1000) == "just now"`;
  - `ago(1060, 1000) == "1 min ago"` and `ago(1000 + 3599, 1000) == "59 min ago"`;
  - `ago(1000 + 3600, 1000) == "1 h ago"` and `ago(1000 + 86_399, 1000) == "23 h ago"`;
  - `ago(1000 + 86_400, 1000) == "1 d ago"` and `ago(1000 + 3 * 86_400, 1000) == "3 d ago"`.
- `search_history_is_one_command`: the free `fn search_item() -> Item`, which `start` publishes as the module's only item, has `id == ItemId::new("history", "search-history")`, `title == "Search History"` and `kind == ItemKind::Command`; `actions[0].id == "open"` and `id.is_valid()`.
- `open_lists_newest_first_with_ago`:
  - record `"aa"`@`NOW - 3 * 86_400` and `"bb"`@`NOW - 7_200`, with `NOW = 1_700_000_000`;
  - the titles are `["bb", "aa"]` and the subtitles `[Some("2 h ago"), Some("3 d ago")]`;
  - `v.search == SearchMode::CoreFilter`, `v.id == ViewId("history".into())` and `v.title == "Search History"`.
- `row_actions_are_search_delete_clear`:
  - the `(id, title, shortcut)` triples equal the pinned list;
  - `actions[1].hotkey().unwrap().is_ok()`.
- `search_puts_the_query_in_the_root_field`: `ActionChosen { row_id: <bb id>, action_id: "search" }` gives `[Effect::SetRootQuery("bb".into())]`.
- `delete_entry_removes_it_and_updates_the_list`: deleting `bb` gives `[UpdateView(List)]` with row titles `["aa"]`, and `history.entries()` holds only `aa`.
- `clear_empties_history_and_list`: the effects are `UpdateView` with no rows, then `Toast { kind: Success, title: "Search history cleared", message: None }`, and `history.entries()` is empty.
- `a_deleted_row_answers_with_an_error_toast`: delete `bb`, then `search` on `bb` gives `UpdateView` (rows `["aa"]`) plus `Toast(Error, "This entry is no longer in the history")`.
- `unknown_action_is_an_error`: `action_id: "nope"` gives `Err`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tomakast-module-history`
Expected: FAIL (the crate is empty, or the functions are missing).

- [ ] **Step 3: Implement the module, then the daemon wiring**

- `modules/history/Cargo.toml` follows `modules/demo/Cargo.toml`, with deps `tomakast-core`, `anyhow` and `tracing`.
- In `start`, `ctx.replace_items(vec![search_item()])`.
- The rows are rebuilt from `history.entries()` on every answer: no cache in the module.
- In `crates/app/Cargo.toml`, add `tomakast-module-history.workspace = true`.

- [ ] **Step 4: Run the full CI set**

Run each, from the worktree:
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p tomakast-plugin-host --lib -- -D warnings`
- `cargo test --workspace`

Expected: all pass with no warnings. The guest SDK and plugin steps of CI are untouched by this plan: no file under `crates/plugin-sdk` or `plugins/` changes.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock modules/history/Cargo.toml modules/history/src/lib.rs crates/app/Cargo.toml crates/app/src/daemon.rs
git commit -m "feat(history): Search History built-in module, wired into the daemon"
```

---

## Self-Review

**1. Spec coverage**
- *What is recorded:*
  - runs: Task 2;
  - hide without a run, ≥ 2 chars: Tasks 2 (the rule) and 4 (the hook);
  - the Search History query is not recorded, on its run or on the next hide (coordinator's call): Task 2 (`exempt_from_history`, `record_hidden_query`), Task 4 (hide) and Task 5 (wiring);
  - the new table with id and timestamps: Task 1;
  - dedup with the newest winning, and the 1000 cap: Tasks 1 and 2.
  - The spec says runs "need no new data" because `usage` stores the query. This plan records runs into `query_history` directly anyway, because `usage` lowercases queries and is loaded for 180 days only. The table starts empty at the upgrade, with no seed from `usage` (coordinator's call), so duplicates match exactly as typed.
- *Privacy:* never offered to plugins (Global Constraints; Task 3's caps deny); built in (Task 5); Clear Search History (Task 5), which erases only `query_history` (Task 2, `clear_and_delete_leave_usage_alone`).
- *↑ in an empty search:*
  - recall, selected so typing replaces it, ↓ forward, and an empty field past the newest: Task 4;
  - an edit makes a new query: Task 4;
  - the revised up-arrow rule: the first row stays selected at an empty query, and ↑ on it recalls (Task 4). ↑ on the first row with a typed query stays a no-op, left for the strip;
  - Tab is untouched.
- *The command:* a core-filtered list, newest first, "2 h ago", Enter → root field + search, Delete Entry, Clear: Tasks 3 and 5.
- *Left to the builder:*
  - the crate placement is `modules/history`;
  - the recall wiring is in `LauncherView`;
  - the empty-query suggestions show no recent queries (off by default; not built).

**2. Step scan.** Every code step names its file, signature and pinned values. The bodies shown are the three SQL statements, where the REPLACE tiebreak is a decision, and the recall table, which is a state machine the signatures do not determine.

**3. Type consistency.** These names are the same in every task:
- `QueryHistory::{load, record, entries, nth_query, delete, clear}`;
- `Store::{upsert_query, load_query_history, delete_query, clear_query_history, prune_query_history}`;
- `Core::{history, set_history, exempt_from_history, record_hidden_query}`;
- `Effect::SetRootQuery(String)`;
- `LauncherView::{set_root_query, root_query, root_selection, query_selection}`;
- `HistoryModule::{new, with_clock}`, `ago`, `PROVIDER = "history"`.

**4. Review Focus.** All five lines have a named test in their owning task (2, 4, 2, 4, 2).

**5. Proportion.** There are no function bodies beyond the SQL and the recall table. Most of the length is test names with exact values.
