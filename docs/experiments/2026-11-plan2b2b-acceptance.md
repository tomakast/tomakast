# Plan 2b-2b acceptance (Task 4)

Date: 2026-11-19. Branch `plan-2b2b-sdk-calc`, base `b57b83f`. Machine: the plan 1 setup (KDE
Plasma 6.8.3, Wayland, 16 threads, kernel 7.3.5-1-cachyos).

Five parts: [the freeze](#the-freeze), [calc's numbers](#calcs-numbers),
[rulings](#rulings), [known limitations](#known-limitations) and the
[user checklist](#user-checklist) (result cells blank).

## The freeze

WIT 0.1 and the manifest format are frozen.

- Frozen on **2026-11-19**. `crates/plugin-api/wit/since_v0.1.0/plugin.wit` has the sha256
  **`e9e0077e6a6a446726c120064dc64401e208daffbe46288047e0951cc80f7095`**, recorded in
  `crates/plugin-api/wit/since_v0.1.0/FROZEN` in `sha256sum` format (run `sha256sum -c FROZEN`
  in that folder to check it). The folder holds only `plugin.wit`, `FROZEN` and `FROZEN.md`
  (the freeze policy and the record of re-hashes, Ruling 15), and the freeze test checks that
  too. `.gitattributes` keeps the WIT files LF on every OS, so the hash is the
  same on the Windows CI leg.
- `crates/plugin-api/tests/freeze.rs`:
  - `wit_0_1_is_frozen` compares the hash with `FROZEN`. On a change it fails with "WIT 0.1 is
    frozen; read crates/plugin-api/wit/since_v0.1.0/FROZEN.md. Relaxing a limit is a
    deliberate re-hash of FROZEN, recorded in FROZEN.md with its reason; tightening a limit or
    any change of shape needs a new `since_v0.2.0` folder".
  - `manifest_schema_is_frozen` reads the keys `Manifest::parse` accepts from the parser
    itself (every table refuses unknown keys and lists the expected ones). It checks that they
    equal the key column of `docs/plugins/manifest.md`. Removing the `search` row from the
    doc, or renaming `author`, makes it fail, naming the key.
- `crates/plugin-host/tests/wit_coverage.rs` drives kitchen-sink and calc through the host,
  with a test-only recorder in `WasmPlugin` (feature `testing`). Every import function, export,
  `event` / `view-event` case, `effect` case and `view` case (the kind of view a `push-view` or
  `update-view` carries) of the parsed WIT must be in the recording, and nothing outside it. Before kitchen-sink was extended, it failed naming the gaps:
  `["effect close-window", "import log.log"]`, and, once `view` cases were required too,
  `["view form-view", "view grid-view"]`. Kitchen-sink now has `log:<text>`, `close-window`
  and `view:<kind>` actions.

The last task and the final fix wave changed the WIT three times, all doc comments only:

- Ruling 2 (a): the background budget counts "time spent in the call (including host
  imports)". That is what the host has measured since plan 2b-2a.
- Ruling 10: the package doc says that `preferences` and `log` need no permission, and that
  `storage`, `open`, `http` and `timers` need their own. It used to say "the imports need their
  own permission". `FROZEN` was re-hashed after this edit.
- Ruling 14 (final review G1): the package doc says that WASI gives the wall clock, the
  monotonic clock and random numbers, and nothing else. `FROZEN` was re-hashed again, and the
  change is the first row after the freeze in `FROZEN.md`.

Rulings 7, 9 and 9b changed the WIT earlier in the plan.

## Calc's numbers

Release builds, measured on 2026-11-19. The timings are from before Task 4's fix round; the wasm sizes are after it.

| What | Measured | Budget | How |
|---|---|---|---|
| Search, engine only | worst of 1000 random queries ≤ 1 KiB: **8 µs** | 10 ms | `cargo test --release --manifest-path plugins/calc/Cargo.toml -- --nocapture search_is_fast` (native, not wasm) |
| 1000-line sheet, engine only | **0.66 ms** | 50 ms | the same, `sheet_of_1000_lines_is_fast` |
| 1 KiB search query, through the host | **0.04 ms**, best of 5 | 10 ms | `cargo test --release -p tomakast-plugin-host --test calc pad_and_search_stay_within_budget -- --nocapture`: the round trip through the core, calc in wasm |
| 1000-line sheet recompute, through the host | **3.2 ms**, best of 5 | 50 ms | the same test: `text-changed` to `update-view` with 1000 annotations |
| `calc.wasm` | **315,784 bytes** (308 KiB) | — | `target/plugins/wasm32-wasip2/release/calc.wasm`, `opt-level = "s"`, LTO, stripped |
| `kitchen_sink.wasm` | **285,253 bytes** (279 KiB) | — | the same profile |

Task 3 measured the same round trips at about 1.1 ms (query) and 4.3 ms (sheet). The query's
difference is most likely the harness: its `search` polls for the answer with 1 ms sleeps, so a
round trip reads as either about 0 or about 1 ms. Both runs are far inside the budgets. The
C3 and C4 expectations below were checked against the engine (`Sheet::eval`, `eval_query`).


## Rulings

Every 2b-2b ruling from the ledger (20), superseded ones included.

| Ruling | Decision | Cost if wrong |
|---|---|---|
| 1 | `plugins/calc` has crate-type `["cdylib", "rlib"]` and no wasm-only code in `src/engine/`, so the engine's tests run natively. | none |
| 2 | Before the freeze: (a) the background-budget wording becomes "time spent in the call (including host imports)"; (b) the spec's Plugin API section is updated where it no longer matched: timers end with the instance and are re-armed in `init`; `search` is a permission; the user-action definition; a push of a view id already on the stack returns to it (Rulings 9/9b). Done in Task 4. | none |
| 3 | Typestate builder: `Item::new(id, title)` gives an `ItemDraft` (subtitle, keywords, icon); `ItemDraft::action(..)` / `with_action(..)` gives an `Item`. An item without actions cannot be built through the SDK. | an extra type |
| 4 | CI runs fmt, clippy and test for `crates/plugin-sdk` and `plugins/calc`, and clippy for `plugins/kitchen-sink`, through `--manifest-path` steps. The two drift checks (format characters against core, `API_VERSION` against the WIT) also live in a workspace test, so `cargo test --workspace` enforces them. | CI minutes |
| 5 | Tree depth, not only parentheses, is at most 64 everywhere: `%` chains, unary chains and infix chains count. Deeper is "too deeply nested". A test runs the worst cases on a 256 KiB stack. | none |
| 6 | *Superseded by 6b, then 6d.* Phone- and date-like digit groups get no search answer: a query of only digits and `-` or `/`, with no spaces, where a group has ≥ 3 digits. | one heuristic |
| 6b | *Superseded by 6d.* No answer only for the phone shape `^\d{3,}-\d{3,}(-\d+)*$` and the date shapes `^\d{4}-\d{1,2}(-\d{1,2})?$`, `^\d{1,2}/\d{1,2}/\d{2,4}$`, `^\d{1,2}/\d{4}$`. `1000/5`, `10-100`, `3/4`, `12/25` answer. | one heuristic |
| 6c | *Superseded by 6d.* Add `^\d{4}/\d{1,2}(/\d{1,2})?$` to the no-answer list. | none |
| 6d | In force. No search answer only for `^\d{3}-\d{4}$`, `^\d{3}-\d{3}-\d{4}$`, `^\d{4}-\d{2}$`, `^\d{4}[-/]\d{1,2}[-/]\d{1,2}$` and `^\d{1,2}/\d{1,2}/\d{4}$`. `1000-5`, `1500-300`, `100-200`, `5/1000`, `3/4` and `12/25` answer. | one heuristic |
| 7 | The host's storage quota errors start with `quota: `, documented in the WIT `storage` doc; calc matches the prefix, not a substring. | none (before the freeze) |
| 8 | Calc saves on every `text-changed` while its write budget, less a reserve of 3, allows (raised to 4 in Task 4's fix round: one whole save can take 4 writes). `closed` and the actions may use the reserve. **Above about 12 edits per second, the newest text waits for the next event or for `closed`.** The first save of a sheet at the 100-sheet cap takes 2 more writes (Ruling 13) and may wait the same way. | at extreme typing rates, a crash in the gap loses the last fraction of a second |
| 9 | Contract and UI, before the freeze: a `push-view` of a view id the same plugin already has on the stack returns to that entry. The views above it are closed (with `closed` events), and its content is replaced, instead of stacking a copy. Documented in the WIT `push-view` doc. | a plugin cannot stack two views with one id on purpose (it can use distinct ids) |
| 9b | An unsolicited `push-view` returns to an existing entry only if every entry above it belongs to the same plugin; otherwise the push is dropped (unsolicited effects pop no one else's views). | none |
| 10 | The WIT package doc is corrected: `preferences` and `log` need no permission, and the other imports need their own. Then `FROZEN` is re-hashed, in Task 4's fix round, before anything is merged. | none |
| 11 | The spec's Plugin API, manifest and threat-model sections are rewritten to match the frozen WIT and manifest: `init` without a context, `run(item, action-id)`, the API version read from the import names, `index`/`clipboard`/`toast` as effects, the permission list with `search`, user actions, budgets. | none |
| 12 | Cosmetic line-wrap leftovers (`wit_coverage.rs:4`, lines of this note, the kitchen-sink `view:list` doc and id) are parked for the final fix wave. | none |
| 13 | The decision-free part of the final review's I1: a sheet is listed only after its first save of text that is not blank; empty sheets are dropped first; every drop (cap or quota) shows a toast naming the dropped sheet. The cap of 100 and the drop-on-quota policy stay as planned and go to the user. | none (strictly less silent loss) |
| 14 | G1: the WIT package doc states that WASI gives the wall clock, the monotonic clock and random numbers, and nothing else; `FROZEN` is re-hashed. | none (before the merge) |
| 15 | G2 policy, provisional until the user confirms it: the numbers in `plugin.wit` are contract. Relaxing a limit (which breaks no plugin) is a deliberate re-hash recorded in `FROZEN.md` with a reason; tightening a limit or any change of shape needs `since_v0.2.0`. The freeze test's failure message says so. | the policy text changes |
| 16 | The decision-free minors of the final review go in the same wave: calc saves waiting text at the top of `search` (M3); a drift check of the SDK's plugin-id check against the manifest's; the SDK README documents a `git` dependency and "keep logic pure" (M7, M10); `export_plugin!` takes a path (M9); the CI guard for calc goes (M12); a pushed pad is cut to the 256 KiB cap (M4); the drill-down note in the author guide; L5, L6 and M2 below; the "WIT 0.2 candidates" list in the roadmap. | none |

## Known limitations

- **Ruling 8's leftover.** Above about 12 edits per second, calc's newest text waits in
  memory for the next event or `closed`. If the plugin breaks or is reloaded in that gap,
  without a `closed`, the last fraction of a second of typing is lost. Calc holds `timers`
  since calc conversions, but its 10-minute tick does not save waiting edits.
- **A re-pushed pad can show older text (L5, M2).** When the user runs "Calculator" by
  hotkey or CLI while typing in that same pad, the launcher returns to the pad and replaces it
  with the text calc knew when it answered. The pad's unsent edit still reaches calc as
  `text-changed`, but the editor then shows text without the keystrokes of the last ~30 ms
  plus the round trip, and the next keystroke saves that older text over the newer one. It
  follows from the frozen `push-view` wording (WIT 0.2 candidate G5).
- **An edit after `closed` can wait (L6).** When the write budget is spent, `closed` leaves
  the newest edit in memory. Calc saves it on its next call: any `search` (every keystroke in
  the launcher), `run` or view event. A `search` does not save it when that needs a sheet
  dropped (at the cap or with storage full), so that edit waits for a `run` or view event.
  If the daemon exits or the plugin is reloaded before then, that edit is lost.
- **A pad over 256 KiB is re-pushed cut.** Opening a sheet again (from History or by the
  Calculator command) while its pad holds more than 256 KiB pushes the first 256 KiB, the most
  a pushed pad may hold. Only the first 64 KiB of a sheet are saved anyway.
- **Drop toasts need the launcher open.** Every sheet dropped for the cap or the quota is
  named in a toast. A drop made while the launcher is hidden (by `closed` on hide) is told
  in a toast the launcher does not show.
- **Reinstalling deletes calc's sheets.** Removing a plugin folder while the daemon runs
  deletes its approval and its storage (2b-2a Rulings AG and AR). Step C8 below loses every
  sheet on purpose.

## User checklist

Run from the worktree root (in `bash`, if your shell is fish). Everything uses a throwaway
home, so no real config, database, plugins or autostart entry is touched and no backup is
needed. The keyboard is grabbed normally (`TOMAKAST_NO_KEYBOARD` is unset): open the launcher
with the hotkey, or with `target/release/tomakast toggle` from a shell with the same
`TOMAKAST_HOME`.

```bash
cargo build --release -p tomakast-app
CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 \
  --manifest-path plugins/calc/Cargo.toml
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-calc.XXXX)
STAGE=$(mktemp -d)/calc && mkdir -p "$STAGE"
cp plugins/calc/plugin.toml "$STAGE/"
cp target/plugins/wasm32-wasip2/release/calc.wasm "$STAGE/plugin.wasm"
unset TOMAKAST_NO_KEYBOARD
target/release/tomakast daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!
mkdir -p "$TOMAKAST_HOME/plugins" && cp -r "$STAGE" "$TOMAKAST_HOME/plugins/"
```

| # | What | Step | Expected | Result |
|---|---|---|---|---|
| C1 | Approve calc | Open the launcher; the first suggestion is "Review plugin: Calculator". Open it. | It lists "Store data on this computer (up to 1 MiB)", "Copy text to the clipboard", "Run timers in the background", "See what you type in the launcher search" and "Make web requests to open.er-api.com, api.coingecko.com", and nothing else (since calc conversions; the 2b-2b run had the first, second and fourth). Allow it; "Calculator" and "Calculator History" are now found by search. | |
| C2 | Search answer | Type `2+2*3`. | One row "8", subtitle `2+2*3`, labelled "Calculator". Enter copies `8`: paste it somewhere to check. | |
| C3 | Not math | Type `2024 report`, `10 km run`, `42`, `2026-11-19`, `555-1234`, then `1/2 price`. | No calculator row, and never an error row. | |
| C4 | A sheet | Run "Calculator". Type `x = 5`, `x * 2`, `5 km in mi`, `15% of 80`, `80 + 15%`, `prev * 2`, `1/0`, one per line. | Annotations: `5`, `10`, `3.106855961 mi`, `12`, `92`, `184`, and "division by zero" marked as an error. | |
| C5 | Restored | Press Esc, then run "Calculator" again. | The same text and annotations. | |
| C6 | History | In the pad, open the action panel (Ctrl+K) and choose New Sheet; type `1+1`; then choose History. | Two rows, newest first: "1+1" and "x = 5", each "Edited …". Enter on "x = 5" opens that sheet; Esc, then "Calculator" opens it again (it is current now). | |
| C7 | Delete a sheet | In History, choose Delete Sheet on "1+1" (Ctrl+K). | The row disappears; History opened again does not show it. | |
| C8 | Reinstall re-prompts | `rm -rf "$TOMAKAST_HOME/plugins/calc"`; wait a second; `cp -r "$STAGE" "$TOMAKAST_HOME/plugins/"`; open the launcher. | The log has `plugin folder removed; its approval and data are deleted`. "Review plugin: Calculator" is back as a new plugin. After Allow, "Calculator" opens an empty sheet (the old sheets were deleted with the folder). | |

Teardown: `kill "$DAEMON"; rm -rf "$TOMAKAST_HOME" "$(dirname "$STAGE")"; unset TOMAKAST_HOME`.

## Open after the final re-review (fix before or right after the merge)

- **Hidden drop toasts.** Fixed (commit e193c7d, "drop toasts are never hidden"). Queued drop toasts could replace each other, so a dropped sheet's name could go unseen; an action's own toast (such as "No result to copy") also replaced the drop toast its flush had made. `flush` now leaves the names in `dropped`, and every calc call that returns effects appends one toast naming them all after its own effects. Tests: `a_drop_toast_comes_after_the_calls_own_toast` and (after the next fix) `search_makes_no_room_when_storage_is_full`.
- **Drop scans in `search`.** Fixed (commit "search makes no drops", right after the one above). `flush_quietly` now makes only saves that need no drop. A save that needs one (a first save at the cap, or a write refused for the quota) waits for the next call that is not a search, and searches do not retry it until then (a refused write counts against the budget). Tests: `a_save_that_needs_a_drop_waits_for_a_call_that_is_not_search`, `search_makes_no_room_when_storage_is_full`, and the release-only timing test `search_stays_within_its_deadline_at_the_cap_with_storage_full`. That test uses 100 sheets of 10 KiB filling the quota and a 50 KiB first text that needs the cap's drop and three for the quota. On the development machine the old search took 3.2 ms in release and 9.5 ms in a debug host, so the miss was not reproduced there; it needs a slow disk or a contended database. The fixed search takes 1.0 ms. The call after it that makes the room takes about 12 ms, against a 1000 ms deadline.
- **HTTP pool starvation** (plan 2b-2a ruling AS): two plugins can hold all 8 workers. Fix: a per-plugin worker cap.
