# Plan 2b-2b: guest SDK, calculator and the WIT 0.1 freeze Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Plugin authors get a Rust SDK. The `calc` plugin proves the third-party path end to end: a dynamic answer in search, and a Numi-style scratchpad that never loses work and keeps a history of sheets. After that, the WIT 0.1 contract and the manifest format are frozen by a test.

**Architecture:**
- **SDK.** A new guest-side crate `crates/plugin-sdk` (package `tomakast-plugin`) wraps `wit-bindgen` over `crates/plugin-api/wit/since_v0.1.0`. It gives typed builders for items, views and effects, thin wrappers over the imports, and an `export_plugin!` macro. It also records the API version it was built against, which the host checks against the manifest.
- **Calculator.** `plugins/calc` is a guest crate built on the SDK. Its expression engine is a pure Rust module, `plugins/calc/src/engine/`, unit-tested natively.
- **Freeze.** A coverage test drives kitchen-sink and calc through the host and checks that every import, export, event case and effect case was exercised. A freeze test pins the WIT file and the manifest schema by hash.

**Tech Stack:** Rust 2024, `wit-bindgen` 0.63 (guest, `wasm32-wasip2`), the 2b-2a plugin host (`crates/plugin-host`) and its test harness (`crates/plugin-host/tests/support`), `wit-parser` 0.261.

**Spec:** `docs/superpowers/specs/2026-11-18-tomakast-core-design.md`, sections "`calc`: calculator-scratchpad WASM plugin", "Plugin API (WIT)", "Plugin manifest and installation". Other inputs:
- the WIT contract `crates/plugin-api/wit/since_v0.1.0/plugin.wit`, which holds every rule as doc comments;
- the 2b-2a acceptance note `docs/experiments/2026-11-plan2b2a-acceptance.md` (rulings A–AS).

## Global Constraints

- **The WIT contract is binding** (`plugin.wit` doc comments: permissions, the user-action rule, deadlines, caps, the background budget). Calc stays inside it with margin:
  - `search` answers in ≤ 10 ms for any query ≤ 1 KiB;
  - a scratchpad of 1000 lines recomputes in ≤ 50 ms in the guest.
- **Calc's manifest permissions:** `search = true`, `storage = true`, `clipboard = true`. No `http`, `timers`, `open` or `index`.
- **Never lose calculations** (spec):
  - sheet text is saved to `storage` on every `text-changed` and on `closed`;
  - opening the Calculator restores the last sheet;
  - every sheet is kept in history until the user deletes it.
- **Storage:** at most 100 sheets. Saving a 101st drops the oldest that is not current. Sheets stay within the 1 MiB plugin storage quota: a sheet over 64 KiB is still computed but saved truncated, and the user gets one warning toast per sheet.
- The WIT and manifest freeze is the last task. Before it, any WIT change needs a ledger ruling. After it, the WIT file is read-only.
- Specs, plans and docs are in English. Agents never show a window that grabs the keyboard.

## Review Focus

- **Fast typing in the scratchpad** (100 keystrokes in 2 s): no lost text in storage after Esc. The last revision's annotations win, and `closed` saves the final text even when the last `text-changed` was debounced. Test in Task 3.
- **A query that only looks like math** ("2024 report", "10 km run", "1/2 price"): there is no answer, or a sensible one, and never an error row in search. A number alone or a date-like string gives no answer. Test in Task 2.
- **Huge or pathological expressions** (`9^9^9^9`, `((((…` 10 000 deep, a 1 KiB line of digits, division by zero): there is no trap and no deadline overrun. The answer is an error annotation ("too large", "division by zero"), or nothing in search. Test in Task 2.
- **Storage full** (1 MiB quota reached): saving shows one warning toast. The sheet stays usable, and the oldest non-current sheet is dropped first. Test in Task 3.
- **The plugin is reloaded or broken while a sheet is open:** reopening the Calculator restores the last saved text. Test in Task 3.

---

### Task 1: Guest SDK and API version check

**Files:**
- Create: `crates/plugin-sdk/{Cargo.toml,src/lib.rs,src/builders.rs,src/imports.rs,README.md}`
- Modify:
  - `plugins/kitchen-sink/{Cargo.toml,src/lib.rs}`, moved onto the SDK; its behaviours stay identical;
  - `crates/plugin-host/src/{package.rs or engine.rs, plugin.rs}`, for the API version check;
  - the root `Cargo.toml`. The SDK is **not** a workspace member, because it targets `wasm32-wasip2`. It has its own empty `[workspace]`, like the plugins.

**Interfaces:**
- Produces:
  - `tomakast_plugin::Plugin` trait with `init`, `search`, `run` and `on_event`. Their signatures mirror the WIT, with default bodies that return `Ok(vec![])`;
  - `tomakast_plugin::export_plugin!(Type)`;
  - builders: `Item::new(id, title)` with `.subtitle(..)`, `.action(id, title)`, `.icon(..)`, `.keywords(..)`; `View::list/scratchpad/detail/form/grid(...)`; `Effect` constructors;
  - import wrappers `storage::{get, set, remove}`, `preferences::get`, `log::{debug, info, warn, error}`, `timers::{start, cancel}`, `http::fetch`, `open::{url, path}`;
  - `pub const API_VERSION: &str = "0.1.0"`, emitted into the component so the host can read it.

**API version check (host):**
- Read the version from the component: preferably from the `@0.1.0` suffix of its imported interface names (`Component::component_type().imports()`), otherwise from a custom section `tomakast-api` that the SDK emits. Verify which of the two survives `wit-component` and use that one; record the choice in the report.
- When the version is present and differs from the manifest `api`, or is not a supported version, the plugin is refused with a clear log line and an error toast "built for API x.y.z".
- A component with no version marker is accepted on the manifest's word, with a debug log.

- [ ] **Step 1:** Write failing tests in `crates/plugin-host/tests/api_version.rs`:
  - `matching_version_loads`;
  - `manifest_and_component_disagree_is_refused`;
  - `unsupported_version_is_refused`.

  They use the kitchen-sink built through the SDK, plus a fixture whose manifest claims `0.2.0`.
- [ ] **Step 2:** Implement the SDK and move kitchen-sink onto it. Every existing plugin-host test stays green, unchanged.
- [ ] **Step 3:** Write `crates/plugin-sdk/README.md`: a hello-world plugin (manifest + `lib.rs` + build command `cargo build --release --target wasm32-wasip2`), then installing it by copying the folder. The README is the author's entry point; plan 2b-2b's Task 4 links it from the docs.
- [ ] **Step 4:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(plugin-sdk): Rust guest SDK; host checks the API version`.

### Task 2: Calculator engine

**Files:**
- Create: `plugins/calc/{Cargo.toml,plugin.toml,src/lib.rs,src/engine/{mod.rs,lexer.rs,parser.rs,eval.rs,units.rs,format.rs}}`. This task adds only the engine; `lib.rs` is a stub that exports nothing yet.
- Test: unit tests inside `src/engine/`, run natively with `cargo test --manifest-path plugins/calc/Cargo.toml`. The engine has no WASM or SDK dependency, and the calc crate builds for the host target too.

**Interfaces:**
- Produces:
  - `engine::Sheet::eval(text: &str) -> Vec<LineResult>`, one per line, with `LineResult::{Empty, Value(Value), Assign(String, Value), Error(String)}`;
  - `engine::eval_query(query: &str) -> Option<Value>`, for search. It returns `None` unless the query is "clearly math": it has an operator, a function, a percentage, a unit conversion or a constant, and parses completely;
  - `engine::format(&Value) -> String`;
  - `Value { number: f64, unit: Option<Unit> }`.

**Language (calc's own choice within the spec):**
- **Numbers:** `.` is the decimal point. `_` may group digits (`1_000`). Scientific notation: `1e3`.
- **Operators:** `+ - * / ^ ( )`, unary minus, and `mod`. `^` is right-associative.
- **Functions:** `sqrt abs round floor ceil ln log sin cos tan`, in radians. **Constants:** `pi`, `e`.
- **Variables:** `x = 5`, then `x * 2`. A name is `[a-zA-Z_][a-zA-Z0-9_]*` and must not be a unit or function name.
- **Previous result:** `prev` (also `ans`) is the last non-empty line's value.
- **Percentages:** `15% of 80` gives 12. `80 + 15%` gives 92, and `80 - 15%` gives 68. A bare `15%` gives 0.15.
- **Units:**
  - length: `mm cm m km in ft yd mi`;
  - mass: `mg g kg t oz lb`;
  - time: `ms s min h day week`.
- **Unit rules:**
  - `5 km in mi` or `5 km to mi` converts;
  - `+` and `-` of the same dimension give the left operand's unit;
  - `*` or `/` by a plain number keeps the unit;
  - mixing dimensions is an error "cannot add km and kg".
- **Comments:** `#` to the end of the line. A line with only a comment is `Empty`.
- **Output:**
  - at most 10 significant digits, trailing zeros trimmed, no grouping;
  - the unit is appended with a space (`3.106855961 mi`);
  - integers up to 1e15 print without a decimal point;
  - `inf` and `NaN` become the errors "too large" and "not a number".
- **Limits:**
  - nesting depth ≤ 64, else "too deeply nested";
  - a line ≤ 1 KiB is evaluated, a longer one is "line too long";
  - `^` with a result over 1e308 is "too large";
  - division by zero is the error "division by zero".

- [ ] **Step 1:** Write the failing table-driven tests:
  - `arithmetic_and_precedence`;
  - `power_is_right_associative`;
  - `variables_and_prev`;
  - `percentages`;
  - `unit_conversion_and_arithmetic` (for example `1 mi in km` gives `1.609344 km`, and `3 ft + 20 cm` gives `3.656167979 ft`);
  - `dimension_mismatch_is_an_error`;
  - `comments_and_empty_lines`;
  - `formatting`;
  - `pathological_inputs_do_not_panic`: `9^9^9^9`, 10 000 `(`, 1 KiB of digits, `1/0`, `0/0`, and a line of only operators;
  - `query_heuristic`: `2+2*3` gives 8; `15% of 80` gives 12; `5 km in mi` answers; `sqrt(2)` answers; and `42`, `2024 report`, `10 km run`, `2026-11-19` and `hello` give `None`;
  - `search_is_fast`: 1000 random queries of ≤ 1 KiB evaluate in < 10 ms each, release build;
  - `sheet_of_1000_lines_is_fast` (< 50 ms, release).
- [ ] **Step 2:** Implement a hand-written lexer and a Pratt parser. No parser-generator dependency, to keep the wasm small.
- [ ] **Step 3:** Run `cargo test --release --manifest-path plugins/calc/Cargo.toml`, clippy and fmt. Commit `feat(calc): expression engine with variables, percentages and units`.

### Task 3: The calc plugin

**Files:**
- Modify: `plugins/calc/{src/lib.rs,plugin.toml}`
- Create: `plugins/calc/src/{sheets.rs,views.rs}`, `crates/plugin-host/tests/calc.rs`

**Interfaces:**
- Consumes: the Task 1 SDK, the Task 2 engine, and the 2b-2a host test harness (`tests/support`).
- Manifest:
  - `id = "calc"`, `name = "Calculator"`;
  - permissions `search`, `storage`, `clipboard`;
  - commands `calculator` ("Calculator") and `history` ("Calculator History").
- **Search:**
  - `eval_query` gives one item `calc/answer` with title = the formatted result and subtitle = the query, plus the action `copy` ("Copy Result");
  - `run(item, "copy")` returns `copy-text(item.title)`, using W2: the whole item comes back;
  - any engine error gives no item.
- **Calculator:**
  - `run(calc/calculator, "run")` pushes `scratchpad-view` with id `sheet:<id>` and the current sheet's text and annotations, one `line-annotation` per non-empty line, `is-error` for errors;
  - on `text-changed`, it re-evaluates, answers `update-view` with the same revision, and saves;
  - on `closed`, it saves (storage needs no user action);
  - scratchpad actions (Ruling AQ): `copy-last` ("Copy Last Result"), `new` ("New Sheet"), `history` ("History").
- **History:** a `list-view` `history` with one row per sheet:
  - the row title is the first non-empty line (or "Empty sheet"), and the subtitle is "Edited <relative time>";
  - row actions are `open` (Enter) and `delete` ("Delete Sheet");
  - `open` pushes that sheet and makes it current.
- **Storage keys (calc's own):**
  - `sheet:<id>` holds the text;
  - `sheet-edited:<id>` holds unix seconds;
  - `sheets` holds the ids, most recent first, comma-separated;
  - `current` holds the current sheet id.

  Ids are base-36 counters from `next-id`. The time comes from WASI wall clock.

- [ ] **Step 1:** Write failing integration tests in `crates/plugin-host/tests/calc.rs`, building calc with the harness:
  - `search_answers_math_and_copies`;
  - `search_ignores_non_math`;
  - `calculator_restores_last_sheet`;
  - `text_changed_annotates_and_saves`;
  - `closed_saves_the_final_text` (Review Focus: the last debounced edit arrives only as `closed`'s preceding flush);
  - `history_lists_sheets_newest_first`;
  - `open_from_history_makes_current`;
  - `delete_sheet`;
  - `hundred_sheets_cap_drops_oldest_non_current`;
  - `storage_full_warns_once_and_keeps_working`;
  - `reload_restores_text` (unregister and register calc again; reopening shows the saved text);
  - `copy_last_needs_user_action` (`action-chosen` copies; nothing copies on `text-changed`).
- [ ] **Step 2:** Implement.
- [ ] **Step 3:** Manual run, `TOMAKAST_HOME=<tmp>` and `TOMAKAST_NO_KEYBOARD=1`:
  - install calc by copying `plugins/calc` (wasm + `plugin.toml`) into `<tmp>/plugins/calc`;
  - approve it with the `approve_plugin` example from 2b-2a;
  - check `tomakast run calc/calculator` in the log.

  Take screenshots only if the session is unlocked (`loginctl show-session $XDG_SESSION_ID -p LockedHint` says no), crop them to the window, and never commit them.
- [ ] **Step 4:** Run `cargo test --workspace`, clippy and fmt. Commit `feat(calc): calculator plugin with search answers, scratchpad and history`.

### Task 4: WIT coverage, freeze and docs

**Files:**
- Create:
  - `crates/plugin-host/tests/wit_coverage.rs`;
  - `crates/plugin-api/tests/freeze.rs`;
  - `crates/plugin-api/wit/since_v0.1.0/FROZEN` (sha256 of `plugin.wit`, plus the date);
  - `docs/plugins/writing-a-plugin.md`, `docs/plugins/manifest.md`;
  - `docs/experiments/2026-11-plan2b2b-acceptance.md`.
- Modify: `crates/plugin-host/src/plugin.rs` (a test-only recorder of imports, events and effects, behind `cfg(any(test, feature = "testing"))`).

- [ ] **Step 1:** Write the coverage test, which runs a scripted scenario.
  - **Recording:** the harness records which imports the guest called, which `event` and `view-event` cases it received, and which `effect` cases it returned.
  - **Scenario:**
    - kitchen-sink: every behaviour it has, including timers and HTTP against the local test server;
    - calc: search, calculator, history, copy.
  - **Assertion:** `wit_parser` parses `plugin.wit`, and every import function, every export, every `event`/`view-event` case and every `effect` case must appear in the recording.
  - If something is missing, the test names it. Extend kitchen-sink for any gap.
- [ ] **Step 2:** Write the freeze tests:
  - `wit_0_1_is_frozen`: the sha256 of `plugin.wit` equals the one in `FROZEN`. The failure message says "WIT 0.1 is frozen: a change needs a new `since_v0.2.0` folder".
  - `manifest_schema_is_frozen`: the set of manifest keys and permission keys accepted by `Manifest::parse` equals the list in `docs/plugins/manifest.md`.
- [ ] **Step 3:** Write the docs:
  - `docs/plugins/writing-a-plugin.md`: an overview of the execution model, permissions and user actions, deadlines and budgets, views and effects, a link to the SDK README, and calc as the worked example;
  - `docs/plugins/manifest.md`: every key, with its type, limits and an example.
- [ ] **Step 4:** Write the acceptance note:
  - calc's numbers: search latency, sheet recompute time, and the wasm size of calc and kitchen-sink;
  - a user checklist: type `2+2*3` in the launcher and press Enter to copy; open Calculator, type a sheet with variables, units and percentages, press Esc, reopen and see it restored; History opens an older sheet; delete a sheet; reinstall the plugin and see it re-prompt;
  - the frozen hash.
- [ ] **Step 5:** Update the roadmap: 2b-2b is done, WIT 0.1 is frozen, and the follow-up features can run in parallel. Run `cargo test --workspace`, clippy and fmt. Commit `test(plugin-api): WIT 0.1 coverage and freeze; docs: plugin author guide`.
