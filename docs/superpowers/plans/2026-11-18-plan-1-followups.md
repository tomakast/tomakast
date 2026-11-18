# Plan 1: follow-ups

Carried over from the plan 1 per-task and final reviews (2026-11-18). Pick these up early in the next plan. The "Fix before merge" items were fixed in the final fix wave.

## Open user gates

- KDE acceptance checklist: `docs/experiments/2026-11-plan1-acceptance.md` (Parts A and B).
- Window probe on macOS and Windows: `docs/experiments/2026-11-window-probe.md` (entry gate of plan 2a).
- CI on all three OSes has never run (no remote yet).
- Hidden RSS 220–223 MiB vs the ~200 MB target: needs the user's sign-off; check whether the ~157 MiB file-backed part is GPU driver mappings (`smaps`) before any GPUI patch.
- `license = "MIT"` in Cargo.toml was added without being requested: confirm or change.

## From the final fix-wave re-review

- A positive icon-cache entry can go stale for the life of the daemon (freedesktop-icons `with_cache`); the icon theme is detected once per scanner thread.
- The acceptance doc's restore block deletes the backup even if a restore `cp` fails.
- The acceptance doc still describes the latency metric as "show request → first frame"; since I4 it is measured from hotkey/IPC receipt, and the 17.8/33.9 ms figures predate that.

## Fix early in the next plan (tracked, not blocking)

| Ledger item | Note |
|---|---|
| T12: nonexistent scan dir is never watched | On a fresh account `~/.local/share/applications` or the flatpak export dir may not exist, so the first user-level install is missed until restart. Spec: "index updates when directories change". Watch the nearest existing parent. |
| T12: watcher `Err` (overflow) only logged, no rescan | Treat an error as a change. Two lines. |
| T7 (carried): key names not validated against XKB | See M2. The duplicate/launcher-collision/no-modifier parts were already fixed in `hotkey_bindings`. |
| T13: `tomakast run` exits 0 on daemon-side failure | Needs a reply channel from the UI thread; do it when the IPC protocol next changes. |
| T11/T13: `APP_ID` duplicated in ui + platform; `TOMAKAST_HOME` rule duplicated in daemon + `Paths` | Move both to `tomakast-core` when either is touched. |
| T8: stalled-client threads leak until disconnect; Ping spawn failure maps to `Dead` | Add a read timeout on the server side; map spawn failure to `TimedOut`. |
| T3: future-dated records give frecency > 1; `half_life ≤ 0` → NaN | Clamp age to ≥ 0. One line; matters after a clock change. |
| T4: `user_version > 1` accepted silently | Refuse to open a newer schema before the second migration exists. |
| T10: KeepOpen untested; prewarm test passes against a no-op | Cover when the first KeepOpen action appears. |
| T9: workspace `ashpd` / inline dep versions (also T11 inline versions) | Already 0.13 in the workspace; move the remaining inline versions (`async-io`, `futures-util`, ui dev-deps) to `[workspace.dependencies]`. |

## Can stay deferred

| Ledger item | Reason |
|---|---|
| T1: unused workspace dep versions unverified | All are in use now and resolve. Closed. |
| T9: click-outside wording; 60 Hz note; `os::clocks` mislabelled; `ON_FRAME` thread-local; backdrop commands | Probe and doc polish for a throwaway experiment. |
| T2: `replace_provider` does not assert provider; `ItemId::new` accepts `/` in provider; non-empty `actions` not enforced | Guarded upstream by `ModuleContext` and by `confirm`/`primary_action` using `first()`. Revisit when the WASM host adds untrusted providers. |
| T2: multi-word query must match one haystack | Design choice; revisit with real usage. |
| T2: stray blank line in `crates/core/Cargo.toml` (also in `modules/apps/Cargo.toml`) | Cosmetic. |
| T3: `stats` clones `ItemId` per record; `LocalNow::now` reads the clock twice | Measured inside the 5 ms budget. |
| T3 / T5: missing tests (non-zero `combined_score`, `push`, aggregation, window boundary, habit baseline, debug-build guard, empty-query perf); `Hit` lacks `Debug` | Covered indirectly by engine tests; add opportunistically. |
| T4: `modified_at` and WAL not asserted in tests | Low risk. |
| T6: "no module for provider" branch untested (the `ghost/x` case in `execute_unknown_item_is_error` fails earlier, at "unknown item"); `register` replaces same-id module; start order nondeterministic; TDD RED not captured; `pub mod core` shadows `core` | Unreachable through `ModuleContext` today; one module. |
| T7: unknown TOML keys ignored; `parse_rejects_bad_hotkeys` does not assert variants; misplaced doc comment | Cosmetic. |
| T8: `ServerHandle` drop only unlinks; `Err(_)` → daemon discards the send error; stale-cleanup TOCTOU; `sun_path` length; socket mode under a shared `TOMAKAST_HOME`; `TOMAKAST_HOME` ignored on mac/win; Windows `AddrInUse` vs `PermissionDenied` | Same-user, dev-only or belongs to plans 3/4. |
| T10: `core_changed` does not scroll to the kept selection; `hide_launcher` silent if global absent; `hide()` logs errors at debug; deferred hide not tied to a window handle | No user-visible effect found. |
| T11: subscribe before bind; `Exec` quoting / "(deleted)" suffix; `hotkey_probe` example kept | Rare paths; keep the example until plans 3/4. |
| T12: symlink fan-out (bounded by the kernel's symlink limit); launch error names the program, not the app; unparsable user file masks the system file; coverage gaps; Access events reset the debounce | Edge cases. Icon-theme coverage is M1. |
| T13: ANSI colours in redirected logs; `Unsupported` logged twice | Cosmetic. |

