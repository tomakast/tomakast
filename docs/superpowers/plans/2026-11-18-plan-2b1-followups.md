# Plan 2b-1: follow-ups

Carried over from the plan 2b-1 final review (2026-11-18). The "Fix before merge" items (I1, I2) were fixed. Open plan 2b-2 with a "WIT 0.1 pre-freeze" task that rules on W1–W12 and writes the W6 rules into `plugin.wit` as doc comments.

## Open user gates

- Plan 2b-1 acceptance checklist: `docs/experiments/2026-11-plan2b1-acceptance.md` (D1–D12).
- Plan 1 KDE checklist Part A is still blank: `docs/experiments/2026-11-plan1-acceptance.md`.

## For plan 2b-2 — behaviour gaps the spec will hit

These do not block this merge. Each is something `calc` or the WASM host needs, and each touches the WIT.

**I3. The last scratchpad edits are dropped when the view closes** — `crates/ui/src/views/scratchpad.rs:37-39`, `launcher_view.rs:457-471`

- The pending debounce task is dropped with the view ("a popped view sends nothing"). Type, then press Esc or lose focus within 30 ms, and the module never receives that text. Hiding the window drops it the same way.
- The spec says calc "never loses calculations: the content is saved to the plugin's storage on change". The module can only save what it is sent.
- Fix: flush the pending `TextChanged` on pop and on hide. This pairs with W5 below (no view-closed event).

**I4. There is no route for effects a module did not just return** — `module.rs:47-67`, `stack.rs:370-407`

- `ModuleContext` can only `replace_items`. A native module cannot update a view from a background thread, and `apply_effects` drops `UpdateView`/`PopView` when the reply has no window origin.
- The spec's execution model depends on this path: timer or HTTP completes → `on-event` → `update-view`. Progress toasts and async loading need it too.
- 2b-2 has to add it (a `CoreEvent` carrying effects with no request id) and define which view instance an unsolicited `update-view` targets. Decide the rule before the freeze; see W11.

**I5. A dynamic source cannot order its own results, and they all tie** — `engine.rs:62-71`, `:93`

- Every dynamic item scores `DYNAMIC_MATCH_SCORE` plus usage, and ties sort by title. `hung_module_search_misses_deadline` asserts this ("Equal dynamic scores fall back to the title order").
- For one calculator answer that is fine. For any source returning several results (converter variants, later file search, which the spec routes through dynamic sources) the source's ranking is lost and replaced by alphabetical order, and all of them normally sit above the static matches.
- This is a contract question, not only a ranking one: WIT `item` has no rank field, and adding one after the freeze is a breaking change. See W3.

## For plan 2b-2 — WIT 0.1 contract, settle before the freeze

Changing a record, a variant or a signature after the freeze means a new version folder. All of these are cheap now.

- **W1. `view-id` is outside the event it belongs to** (`plugin.wit:316`). `on-event(view-id: option<string>, event)` allows a view event with no view id and a timer with one. Put the id inside the `view` case (a record `{ view-id, event }`).
- **W2. `run(item-id, action-id)` gives the plugin only an id** (`plugin.wit:314`), while the native trait passes the whole `Item` (`module.rs:95`). For a dynamic answer the plugin must either remember its last search (the spec lets the core unload an idle plugin) or encode the result in an id of at most 256 bytes without control characters. Calc's "Enter copies the result" depends on this. Pass the `item`, or document that ids must be self-describing.
- **W3. `item` has no rank** (see I5). Decide whether dynamic results keep the source's order (simplest: the host keeps list order within one provider) or carry a score.
- **W4. A form cannot show a module-side field error.** Plan Task 7 says an update carries "labels, options and errors", but `form-field` is `{ id, label, kind }` (`plugin.wit:124-128`). "This name is taken" can only be a toast today. Add `error: option<string>` now if it is wanted at all.
- **W5. No view lifecycle event.** A plugin is never told that its view was popped or the window hidden. It cannot stop a timer that refreshes that view, or do a final save (see I3). Adding a case to `event` later is breaking.
- **W6. The behavioural rules are not in the WIT.** A plugin author reading `plugin.wit` cannot learn any of the following, and all of it is contract:
  - `update-view` ignores `query` / `text`, and is dropped unless `revision` equals the last revision the host sent (`stack.rs:112-124`);
  - a form update is dropped as a whole unless field ids and kinds are unchanged, and it replaces only values the user has not touched (`stack.rs:125-134`, `form.rs:257-295`);
  - `action-chosen.row-id` is the tile id in a grid and the view id in a detail view (`launcher_view.rs:1043-1045`, user decision);
  - `pop-view` only pops the replying view; `line` is 0-based and the last annotation per line wins; `columns: 0` means 1;
  - toast lifetimes, and that `progress` stays until replaced (there is no way to dismiss it).
- **W7. `section: none` after a sectioned row** renders under the previous header (`list.rs:101`; Task 4 minor 4). Either fix the renderer or document "rows of one section must be contiguous; `none` continues the previous one".
- **W8. Three effects duplicate imports**: `copy-text`, `toast`, `replace-items` (and `init` returns items as well). The host must apply the same permission and quota checks on the effect path as on the import path, or a plugin without the clipboard permission copies text by returning an effect. Consider dropping one side of each pair.
- **W9. `icon` cannot express the grid's stated use.** The spec lists "emoji/colors/images" for grids; `icon` is `named | plugin-file` (`plugin.wit:25-28`). A colour swatch needs a file per colour. Adding a variant case later is breaking.
- **W10. No size limits are stated**, and the UI assumes small input: the grid renders every tile each frame (`grid.rs:133`), and markdown is parsed on every render (see M1). The host must cap rows, tiles, markdown bytes and id lengths (view and row ids have no rule at all), and the WIT should say so.
- **W11. Unsolicited effects have no target rule** (see I4): which instance does an `update-view` returned from a timer event update when the same view id is on the stack twice?
- **W12. Small inconsistencies:** `search` is infallible while every other export returns `result` (`plugin.wit:312`); the package doc comment (`plugin.wit:3-14`) sits after the `package` line and attaches to `interface types` (Task 3 deferred minor).

## Minor (Nice to Have)

- **M1. Markdown is parsed twice per frame** — `detail.rs:45,50`, `list.rs:232,240`. `plain_text` and `render` each parse the module's text on the UI thread on every render. Harmless at demo sizes; with W10 unset it is a way for a plugin to slow the UI. Cache the parsed blocks in `DetailState` / per selected row. Next plan.
- **M2. Every dynamic reply re-matches the whole static index on the UI thread** — `core.rs:136` → `refresh` → `search_with`. With N dynamic plugins that is N extra full searches per keystroke, against a 16 ms budget. Keep the static hits of the current generation and merge. Next plan.
- **M3. A late answer does not take the selection** — `launcher_view.rs:184-193`. When the answer lands above a static match the user never moved off, the selection stays on the static row, so Enter runs that row and not the top one ("10 km" matching KMail). The plan asked for selection by id, so this is a plan issue: keep the selection on row 0 unless the user moved it since the query changed. Next plan, with the converter.
- **M4. A hung `run_action` gives no feedback** — the Enter gate expires silently after 5 s (`stack.rs:144`). Review Focus 2 asks that a failing call surface as an error toast; a panic does, a hang does not. Epoch traps cover WASM in 2b-2; for native modules a "still working…" progress toast at gate expiry would do. Next plan.
- **M5. A link's target is never shown** — `markdown.rs:401-411`. Text "https://bank.example" can point anywhere, and the click opens it without the plugin holding the `open` permission. The plan mandates click-to-open, so this is for the 2b-2 threat model: show the target (footer or tooltip), or gate on the permission.
- **M6. Returning to a form loses the focused field** — popping back to a form focuses its *first* field (`form.rs:120`), not the one that had the focus before. Can stay.


## Deferred minors marked "Next plan"

-  T1 | Epoch deadline formula (10 ms × 10 trapped at 100.57 ms) | Open. The findings doc still recommends `ceil(budget / tick)` in "Notes for 2b-2" | **Next plan**: 2b-2 sets the real deadline; correct the note there |
-  T2 | Dynamic id equal to a static id → two rows | Open (`engine.rs:62-91`) | **Next plan**: plugin ids are untrusted; dedupe in `search_with` |
-  T2 | `DYNAMIC_MATCH_SCORE` margin heuristic | Open | **Next plan**, with I5 / W3 |
-  T3 | Misplaced package doc comment | Open (`plugin.wit:3-14`) | **Next plan**, with W12 |
-  T3 | Parity table not compile-checked against `view.rs` | Open. Note it also checks names only, not field types | **Next plan**: the bindgen ↔ core conversions in 2b-2 are the compile check; keep the test for the world shape |
-  T4-2 | `dynamic_rows_stay_until_new_results_arrive` cannot fail on its last step | Open (`stack.rs:740-759`: same id and title for both answers) | **Next plan**: use a query-dependent id; the held-row logic is subtle and this is its only guard |
-  T4-4 | Section header after a `None` row | Open (`list.rs:101`) | **Next plan, before the freeze** (W7) |
-  T4-6 | Ledger entries for unanswered requests never removed | Open (`stack.rs:223`) | **Next plan**: with plugins that can be killed mid-call it stops being bounded; prune on window close and on gate expiry |
-  T5-2 | Action panel does not scroll | Open (`action_panel.rs:128-134`) | **Next plan**, when a module has more than ~6 actions |
-  T9 | Any D-Bus error triggers the fallback; an unresponsive `FileManager1` blocks the apps worker | Open (`linux.rs:367-381`). While it blocks, app launches queue behind it | **Next plan**: give the call a short timeout or its own thread; low odds, but it blocks the launcher's main job |

## Recommendations

- Open plan 2b-2 with a "WIT 0.1 pre-freeze" task that rules on W1–W12 and writes the rules of W6 into `plugin.wit` as doc comments. Several of them (W2, W3, W5) change what `calc` can do.
- Treat the demo module as the regression fixture for the view model: when the WIT changes, the demo and the parity table change in the same commit.
- Add one "degenerate view" test per view kind (no rows, no tiles, no fields, empty markdown, annotations past the end) asserting that Esc still pops. I2 is the case that was missing; the others pass by reading, but nothing guards them.
- When the host lands, cap view sizes there (W10) rather than hardening each renderer.

