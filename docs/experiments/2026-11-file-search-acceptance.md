# File search: acceptance (2026-11-22)

Spec: `docs/superpowers/specs/2026-11-22-file-search-design.md`.
Plan: `docs/superpowers/plans/2026-11-22-file-search.md`.

## Setup

- Machine: AMD Ryzen 7 7800X3D (8 cores, 16 threads), 30 GB RAM, home folder on btrfs,
  KDE Plasma 6.8.3 on Wayland, rustc 1.98.1. `max_user_watches` is 524288.
- Other sessions were building on the same machine: the load average was 5 to 37 during
  the runs, and it is given next to each one. Timings are release builds.
- Index and module search: `just files-bench` (`modules/files/examples/bench.rs`). It walks
  the home folder read-only with the default coverage (`roots = ["~"]`, hidden entries,
  ignore files and build folders skipped; runs 1 to 3 applied `.gitignore` files anywhere,
  run 4 only inside a git repository, as `fd`), changes only the in-memory index, and prints
  counts and timings, never file names. It searches with empty signals (no opens, no KDE
  recent list), 30 runs per query, at the root list's limit (3) and the screen's (100).
- UI thread: `ui_thread_time_per_keystroke` in `modules/files/src/lib.rs` (ignored by
  default; `cargo test --release -p tomakast-module-files -- --ignored --nocapture
  ui_thread_time`). A `Core` with a fake apps module, once without and once with
  `FilesModule` over a temporary home (2,003 files), the same apps for both. It measures
  the CPU time of the calling thread (`CLOCK_THREAD_CPUTIME_ID`) for `Core::search` and
  for `Core::apply` of the module's answer, 50 runs per query. The apps are synthetic Latin
  and Cyrillic names with one keyword each; 1,000 of them (this machine has 324 desktop
  entries) and 10,000 as a stress figure. Four runs of one release test binary, one after
  the other, at `10e50e7`; all four are in the appendix.
- Agents never ran `just try` (it grabs the keyboard); see "Not verified".

## Index

| Run | Load | First walk | Entries | Resident after walk | Burst of 5,000 (remove / re-add) | Re-walk | Resident with both indexes / after dropping the old |
|---|---|---|---|---|---|---|---|
| 1, right after `just check` | 13 | 6,711 ms | 455,637 | 50.7 MB (3.4 before) | 2 ms / 246 ms | 1,982 ms | 114.5 MB / 76.7 MB |
| 2, warm | 11 | 1,942 ms | 455,637 | 50.7 MB | 2 ms / 241 ms | 1,760 ms | 114.4 MB / 76.7 MB |
| 3, after more parallel builds | 16 | 7,986 ms | 455,637 | 50.7 MB | 2 ms / 237 ms | 1,887 ms | 114.6 MB / 76.8 MB |
| 4, at `10e50e7` (`.gitignore` only inside a repository) | 6.6 | 6,868 ms | 455,731 | 50.7 MB (3.4 before) | 3 ms / 212 ms | 1,934 ms | 114.7 MB / 76.9 MB |

- The page cache cannot be dropped here (no root), so there is no controlled cold walk.
  Runs 1 and 3 followed heavy builds by other sessions that evicted much of it, and land in
  the range Task 6 measured for a cold walk (6.5 to 8.3 s). Warm: 1.8 to 2.0 s. The walk is
  single-threaded on purpose (memory over speed); the spec says "about 2 s warm, up to ~8 s
  on a cold disk cache on this machine".
- The index itself takes about 47 MB for 455,637 entries, so about 52 MB at 500k.
- While a re-walk builds the new index, the old one still answers, so both are resident:
  about 115 MB at the peak. Afterwards resident memory stays near 77 MB, not 51 MB: the
  allocator keeps the freed pages.
- A burst of 5,000 changes is applied in about a quarter of a second, on the index thread.
- Run 4 has 94 more entries than runs 1 to 3: `.gitignore` files outside a git repository
  no longer hide anything, and the home folder changed between the runs; the bench does not
  tell the two apart. Its first walk (6.9 s) again followed builds by other sessions.

## Module search (per keystroke, on the module's worker)

Run 4 (at `10e50e7`), in ms; `shown` is the screen's rows, `matches` all matches:

| Query | Root median | Root p95 | Screen median | Screen p95 | Shown | Matches |
|---|---|---|---|---|---|---|
| `fire` | 0.72 | 1.02 | 0.69 | 1.01 | 100 | 3,095 |
| `doc` | 1.08 | 1.73 | 1.05 | 1.36 | 100 | 8,249 |
| `readme` | 0.75 | 1.86 | 0.67 | 1.05 | 100 | 2,124 |
| `main.rs` | 0.60 | 1.45 | 0.59 | 0.73 | 39 | 39 |
| `proj/read` | 2.83 | 3.70 | 2.87 | 3.78 | 100 | 320 |
| `ашку` | 4.26 | 5.35 | 4.03 | 5.51 | 100 | 345 |
| `otchet` | 0.53 | 1.08 | 0.61 | 0.78 | 100 | 560 |
| `png` | 3.12 | 4.06 | 2.93 | 3.42 | 100 | 119,810 |
| `config` | 0.48 | 0.56 | 0.47 | 0.64 | 100 | 2,262 |
| `cargo` | 0.87 | 1.22 | 0.82 | 1.01 | 100 | 6,503 |
| `invoice` | 0.42 | 0.65 | 0.37 | 0.54 | 100 | 137 |
| `screen` | 0.84 | 1.40 | 0.78 | 1.07 | 100 | 1,252 |
| `notes` | 1.30 | 1.50 | 1.29 | 1.39 | 100 | 7,742 |
| `todo` | 1.17 | 1.92 | 1.31 | 1.49 | 100 | 5,431 |
| `zzzqx` | 0.23 | 0.41 | 0.22 | 0.81 | 1 | 1 |
| `.pdf` | 0.64 | 0.80 | 0.66 | 0.95 | 100 | 1,344 |
| `photo` | 1.68 | 2.24 | 1.59 | 1.87 | 100 | 32,668 |
| `downloads` | 0.43 | 0.54 | 0.39 | 0.50 | 81 | 81 |
| `rust` | 0.76 | 0.92 | 0.83 | 1.59 | 100 | 5,466 |
| `kde` | 0.43 | 0.73 | 0.39 | 0.44 | 100 | 2,434 |
| `a/e` | 17.43 | 22.84 | 17.49 | 19.07 | 100 | 160,570 |
| `doc/re` | 10.09 | 11.11 | 10.48 | 11.29 | 100 | 48,549 |
| `10/2` | 5.56 | 6.22 | 5.37 | 5.69 | 100 | 8,723 |

Worst p95: 22.84 ms (run 4, `a/e` at the root limit). Without the three queries added for
run 4 (`a/e`, `doc/re`, `10/2`) the worst is 5.51 ms (`ашку` at the screen limit); runs 1 to
3, without them, had 4.39 ms (run 3, root and screen), and 3.87 ms and 4.46 ms (runs 1 and
2, screen only). The new queries are the slow ones: a path query whose parts are single
letters (`a/e`) matches 160,570 entries and `doc/re` 48,549, and every match is scored.
`10/2` is timed here as the index search; the root list does not search a query without a
letter (it is arithmetic for calc), only the Search Files screen does. Next slowest: a
query on the wrong layout (`ашку`) and a path query (`proj/read`). These numbers time the
search over the index; the module's answer also builds at most four items (three files and
the "Search Files for …" fallback) and re-reads
KDE's recent list when it changed.

Run 5 (2026-11-23), with the root limit briefly at 8 (since back to 3): worst p95 19.02 ms (`a/e` at the root limit), `doc/re` 10.19 ms,
`10/2` 5.89 ms, `ашку` 4.14 ms; every other query ≤ 2.7 ms at p95. The root and screen
columns stay within run-to-run noise of each other: keeping the best 8 instead of 3 costs
nothing measurable. Walk 6.4 s, 455,859 entries, 50.7 MB after the walk, 114.7 MB during a
re-walk, 76.9 MB after.

## UI thread per keystroke

1,000 apps, ms of UI-thread CPU time, median / p95, run 1 of the four in the appendix
(load 10.5):

| Query | `search`, without the module | `search`, with it | Applying its answer | With it, total |
|---|---|---|---|---|
| (empty) | 0.18 / 0.22 | 0.18 / 0.20 | 0.17 / 0.22 | 0.35 / 0.41 |
| `f` | 0.46 / 0.53 | 0.45 / 0.48 | 0.45 / 0.49 | 0.90 / 0.95 |
| `fi` | 0.61 / 0.66 | 0.61 / 0.66 | 0.60 / 0.63 | 1.21 / 1.28 |
| `fir` | 0.53 / 0.58 | 0.55 / 0.59 | 0.54 / 0.57 | 1.09 / 1.19 |
| `fire` | 0.53 / 0.62 | 0.54 / 0.66 | 0.54 / 0.66 | 1.09 / 1.31 |
| `firefox` | 0.58 / 0.59 | 0.60 / 0.64 | 0.58 / 0.60 | 1.18 / 1.25 |
| `ашку` | 0.47 / 0.49 | 0.48 / 0.53 | 0.48 / 0.53 | 0.97 / 1.03 |
| `otchet` | 0.53 / 0.59 | 0.54 / 0.57 | 0.54 / 0.58 | 1.08 / 1.14 |
| `file4` | 0.54 / 0.56 | 0.55 / 0.59 | 0.56 / 0.58 | 1.11 / 1.16 |

Over all four runs, with 1,000 apps: the total with the module is 0.35 to 1.25 ms at the
median, and its worst p95 is 1.61 ms (`firefox`, run 4); every run passed the test's 5 ms
assertion.

10,000 apps (stress), over the four runs: `search` alone takes 2.15 to 2.19 ms (median) at
the empty query and 5.07 to 6.97 ms for typed queries, with or without the module. The total
with the module is 4.28 to 4.30 ms at the empty query and 10.20 to 13.56 ms for typed ones
(median); its p95 is 4.35 to 4.57 ms and 10.64 to 15.01 ms, except one spike of 20.62 ms
(`firefox`, run 4: the search and the re-rank both reached about 10.5 ms at p95 while their
medians stayed at 6.3 ms). The p95 of `search` without the module also spiked, to 8.10 ms
(`firefox`, run 2). At this size the static match alone is past the 5 ms budget, with or
without the module.

- `Core::search` costs the same with the module as without it (medians within 0.03 ms with
  1,000 apps in every run).
- Every answer of the module triggers one re-rank on the UI thread (`Core::apply` ranks the
  query again with the answer merged), and it costs about as much as the search itself. The
  module answers every keystroke, with an empty answer below three characters, so the UI
  thread does about twice the work per keystroke. This is how the core treats every dynamic
  source: the calc and weather plugins (both with the `search` permission) already cause
  one re-rank each per keystroke, and file search adds one more.
- The search on the keystroke and the re-rank when the answer arrives are separate tasks
  of the UI thread, with frames between them; each stays near 0.6 ms with 1,000 apps.
- Window open time: not measured on a real window. The closest proxy is the empty query,
  which the window shows when it opens: `search` costs the same (0.17 to 0.18 ms), and the
  module's empty answer adds one re-rank (0.17 ms) shortly after.

## Example queries

From the tests (a temporary home, synthetic names; no file of this machine):

- `fire` with an app "Firefox" and a never-opened `~/fire.txt`: Firefox, then `fire.txt`.
  After `fire.txt` is opened from the root list for `fire`, through the core as the
  launcher does it: `fire.txt`, then Firefox (`firefox_leads_until_the_file_is_opened_by_habit`).
- One open is already enough to flip that order (checked by changing the test to one
  open). The spec allows this only where the usage formula does the same for apps, and it
  does: one launch for a query is worth about 48 to 67 points
  (`usage_scale · ln(1 + usage)`), far more than the 8-point tie penalty, so an app 8 match
  points behind another also leads after one launch for that query.
- `ашку` (`fire` typed on the Russian layout) finds `fire.txt`; `otchet` finds `Отчёт.pdf`
  (`finds_across_layouts_and_scripts`, `path_item_matched_through_its_own_script`).

## Budgets

| Budget | Measured | Result |
|---|---|---|
| Module search ≤ 30 ms at the 95th percentile over this home folder | worst p95 22.84 ms (`a/e` at the root limit, run 4); 19.02 ms in run 5 at the root limit of 8; 5.51 ms without the three queries added for run 4; 4.46 ms in runs 1 to 3 | met |
| Resident memory of the index ≤ ~150 MB for 500k entries | 47 MB for 455,637 entries (~52 MB at 500k), the same for 455,731 in run 4; 115 MB at the peak of a re-walk, 77 MB after | met |
| UI-thread time per keystroke ≤ 5 ms (core spec) | 1,000 apps: total with the module's re-rank ≤ 1.61 ms at p95 over four runs | met |
| UI-thread time per keystroke stays within the 5 ms budget (each module answer adds one re-rank, as calc and weather already do) | 1,000 apps: `Core::search` unchanged (medians within 0.03 ms), one re-rank per answer at about the cost of the search, total 0.35 to 1.25 ms at the median and ≤ 1.61 ms at p95 over four runs | met |
| Window open time unchanged | not measured on a real window; the empty-query proxy above | not verified |
| First walk: about 2 s warm, up to ~8 s cold (spec, not a budget) | 1.8 to 2.0 s warm, 6.7 to 8.0 s with the cache evicted | as the spec says |

A possible follow-up (a core change, not made here): skip the re-rank when an empty answer
replaces an empty one, so the module's answers below three characters cost nothing.

## Review Focus: the tests that pin it

All pass in `just check`.

1. A path the core cannot hold as an id or show as text (over 256 bytes, a newline or a
   bidi override, invalid UTF-8) is indexed, found, shown with a replacement character and
   opened by its real bytes: `id_and_title_of_hostile_names` (`modules/files/src/names.rs`),
   `opens_a_file_with_a_newline_in_its_name` (`modules/files/src/lib.rs`).
2. A search during a re-walk answers from the old index and does not wait for the walk:
   `search_during_rewalk_uses_the_old_index` (`modules/files/src/live.rs`).
3. An app and a never-opened file matching equally: the app leads, a file opened by habit
   leads: `unopened_file_ranks_below_equal_app`, `habit_lifts_file_above_app`
   (`crates/core/tests/engine.rs`), and end to end through `Core` and `FilesModule`:
   `firefox_leads_until_the_file_is_opened_by_habit` (`modules/files/src/lib.rs`, new).
4. A file removed after it was shown: its action fails with "File not found", and the entry
   is checked again: `open_of_a_vanished_file_fails_with_a_toast` (`modules/files/src/lib.rs`).
5. A query on the wrong layout or in another script: `finds_across_layouts_and_scripts`
   (`modules/files/src/search.rs`), `path_item_matched_through_its_own_script`
   (`crates/core/tests/engine.rs`).

## `just check`

Passes: fmt, clippy with `-D warnings` (workspace, plugin host, cross-target location
crate), every workspace test, the guest SDK and the plugins: 1,723 tests passed, 10
ignored, across 94 suites, on the final tree of this branch (the file module's unit tests:
123, the UI's: 117). The ledger had noted one run with "114 passed; 2 failed"; it was
narrowed to the `tomakast-ui` lib suite (116 tests at `2809b5e`), a flake that predates
this branch and is not caused by it. It did not come back in any full `just check` run
here.

## Not verified

- A physical try on KDE: the real window, its open time, the rows, the grid and thumbnails,
  the keys. The user's `just try` covers it.
- For the user to look at while typing: file rows dropping out and coming back (the UI
  holds the previous dynamic rows until the new query's answer arrives or 100 ms pass), and
  whether the selection jumps when they come back.
- macOS and Windows paths are not compiled here (Show in File Manager, skipping
  `~/Library` and `~/AppData`, `\` separators).
- A controlled cold walk (the page cache cannot be dropped without root).
- Module search with real signals (opens, KDE's recent list): the bench uses empty ones.
- The UI-thread measurement with usage history: its apps have no launch records (the
  core's own stress test, `search_10k_under_5ms`, covers ranking with 1,000 of them).
- That one launch lifts an app over another 8 match points ahead, as it lifts a file over
  an app: derived from the usage formula, not tested.

## Known limits

- The index is saved after every walk (`<data>/files/index.bin`, 15.5 MB for 455,861
  entries, readable by the user alone) and read back at start in about 0.3 s, so files are
  found almost at once; the first walk then runs in the background and replaces it. Only the
  very first start, or one after `[files]` changed, waits for a walk (2 to 8 seconds); the
  "Search Files" screen shows the busy line under its field meanwhile and fills when the
  walk ends; so does the root list, for a query of three characters or more, and its files
  join it when the walk ends, without another key. While that first background walk runs, the daemon holds both indexes (about
  115 MB, as during any re-walk).
- On the "Search Files" screen with no rows (a query that finds nothing), the grid toggle
  cannot be reached: it is an action of a row.
- Open With… takes about a second the first time when it is used before the application
  scan finished.
- An edit of a `.gitignore` applies at the next re-walk, at most 30 minutes later.
- `.gitignore` files, `.git/info/exclude` and the global gitignore apply only inside a git
  repository (as `fd`), and only with `gitignore = true` (the default); `.ignore` files and
  the built-in names (`node_modules`, `target`, …) apply everywhere, always.
- When watching is off (more folders than the watch cap, the system out of file watches, or
  after the watcher lost events), new files can take up to 30 minutes to appear.
- A file that appears later is checked against the ignore files by the module itself, which
  follows the walker except in two corners: inside a repository whose `.git` is a file (a
  worktree, a submodule) it does not read `info/exclude`, and across folders it lets the
  deepest file with an opinion decide, where the walker lets any `.ignore` file win over a
  `.gitignore`. The next re-walk puts such an entry right.
- After a re-walk the daemon keeps about 77 MB for the index instead of 51 MB (freed pages
  stay with the allocator).
- Thumbnails are decoded in the `tomakast-raster` helper, whose protocol version is now 3:
  build the helper together with the launcher (`just build` does).
- Smaller follow-ups are in the build ledger, handed to the main session as
  `file-search-ledger.md` in the side-session notes folder (the working copy was removed).

## How to try

1. `just try` (or `just try approve`), then `just toggle` from another terminal. The try
   home is its own (kept between runs, not your real one), but the file index walks your
   real home folder, read-only, and
   thumbnails made for the grid are written into your real `~/.cache/thumbnails` (the
   shared freedesktop cache). Give it a few seconds after start.
2. Type at least three characters of a file name: up to three files and folders join the
   root list (when nothing else matches, "Search Files for “…”" leads them and opens the
   Search Files screen with what you typed), below an equally matching app until you open the file from the launcher for
   that query (once is enough). Enter opens it; Ctrl+K shows Show in File Manager
   (Ctrl+Shift+F), Copy Path (Ctrl+Shift+C) and Open With… (Ctrl+O).
3. Type "Search Files" and press Enter: a screen that searches files only, up to 100
   results, the recent files when the field is empty. Ctrl+K on a row shows its actions,
   among them "Show as Grid" / "Show as List"; Ctrl+G does the same directly. The choice
   is remembered.
4. Settings, in `config.toml` (the try home's, or your own):

   ```toml
   [files]
   in_root = true      # files in the root list
   roots = ["~"]       # folders to index
   exclude = []        # paths ("~/big") or name globs ("*.iso") to skip
   hidden = false      # include hidden files and folders
   gitignore = true    # inside git repositories, skip what .gitignore lists
   ```

   A change applies without a restart (the index is walked again).

## Saved index (2026-11-23)

From `just files-bench` over this home folder (455,861 entries): the saved index is 15.5 MB,
built in 6 ms and read back (match keys included) in 293 ms, besides reading the file. The
first walk of the same run took 6.6 s; a warm one takes about 2 s.

## Appendix: UI-thread runs

Four runs of `ui_thread_time_per_keystroke` at `10e50e7`, release build, one after the other
(`target/release/deps/tomakast_module_files-* --ignored --nocapture ui_thread_time`), with
the load average (`/proc/loadavg`) read before each; output as printed:

```
=== run 1, loadavg: 10.47 6.75 12.74 7/3962 1380369
1000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             0.18 / 0.22     0.18 / 0.20     0.17 / 0.22     0.35 / 0.41
"f"            0.46 / 0.53     0.45 / 0.48     0.45 / 0.49     0.90 / 0.95
"fi"           0.61 / 0.66     0.61 / 0.66     0.60 / 0.63     1.21 / 1.28
"fir"          0.53 / 0.58     0.55 / 0.59     0.54 / 0.57     1.09 / 1.19
"fire"         0.53 / 0.62     0.54 / 0.66     0.54 / 0.66     1.09 / 1.31
"firefox"      0.58 / 0.59     0.60 / 0.64     0.58 / 0.60     1.18 / 1.25
"ашку"         0.47 / 0.49     0.48 / 0.53     0.48 / 0.53     0.97 / 1.03
"otchet"       0.53 / 0.59     0.54 / 0.57     0.54 / 0.58     1.08 / 1.14
"file4"        0.54 / 0.56     0.55 / 0.59     0.56 / 0.58     1.11 / 1.16
10000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             2.16 / 2.20     2.16 / 2.19     2.12 / 2.15     4.28 / 4.35
"f"            5.23 / 5.97     5.36 / 6.18     5.31 / 6.08   10.66 / 12.12
"fi"           6.82 / 7.27     6.78 / 7.74     6.75 / 7.18   13.56 / 15.01
"fir"          5.80 / 6.53     5.93 / 6.86     5.87 / 6.47   11.85 / 13.03
"fire"         5.93 / 6.74     5.81 / 6.13     5.81 / 6.32   11.63 / 12.34
"firefox"      6.43 / 7.37     6.45 / 7.79     6.43 / 7.32   12.88 / 14.75
"ашку"         5.46 / 6.74     5.22 / 5.96     5.21 / 5.65   10.44 / 11.63
"otchet"       5.84 / 7.23     5.70 / 6.16     5.73 / 6.07   11.44 / 12.14
"file4"        5.97 / 6.32     6.03 / 6.45     6.04 / 6.64   12.06 / 12.92
test tests::root_list::ui_thread::ui_thread_time_per_keystroke ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 8.50s
=== run 2, loadavg: 9.64 6.69 12.66 6/3927 1380789
1000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             0.17 / 0.20     0.18 / 0.18     0.17 / 0.18     0.35 / 0.36
"f"            0.46 / 0.48     0.45 / 0.47     0.45 / 0.46     0.91 / 0.92
"fi"           0.61 / 0.66     0.62 / 0.72     0.61 / 0.72     1.25 / 1.40
"fir"          0.53 / 0.56     0.55 / 0.70     0.55 / 0.67     1.10 / 1.31
"fire"         0.54 / 0.66     0.57 / 0.74     0.55 / 0.65     1.13 / 1.32
"firefox"      0.59 / 0.70     0.60 / 0.65     0.59 / 0.63     1.19 / 1.25
"ашку"         0.48 / 0.64     0.51 / 0.60     0.49 / 0.65     1.02 / 1.17
"otchet"       0.53 / 0.75     0.54 / 0.59     0.53 / 0.56     1.08 / 1.14
"file4"        0.55 / 0.67     0.58 / 0.72     0.57 / 0.64     1.16 / 1.41
10000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             2.19 / 2.42     2.16 / 2.36     2.13 / 2.34     4.29 / 4.57
"f"            5.19 / 5.84     5.22 / 5.79     5.19 / 5.68   10.41 / 11.45
"fi"           6.97 / 7.91     6.72 / 7.06     6.69 / 7.00   13.42 / 13.87
"fir"          5.82 / 7.75     5.87 / 6.62     5.85 / 6.36   11.77 / 12.76
"fire"         5.77 / 6.06     5.81 / 6.87     5.82 / 6.47   11.69 / 13.09
"firefox"      6.38 / 8.10     6.34 / 6.59     6.33 / 6.62   12.69 / 13.13
"ашку"         5.15 / 6.67     5.12 / 5.55     5.10 / 5.36   10.22 / 10.81
"otchet"       5.74 / 6.57     5.70 / 6.16     5.70 / 6.41   11.39 / 12.41
"file4"        5.99 / 7.56     5.97 / 6.22     5.98 / 6.38   11.96 / 12.67
test tests::root_list::ui_thread::ui_thread_time_per_keystroke ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 8.45s
=== run 3, loadavg: 8.91 6.64 12.58 3/4007 1381236
1000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             0.17 / 0.22     0.18 / 0.19     0.17 / 0.18     0.35 / 0.36
"f"            0.46 / 0.50     0.46 / 0.48     0.45 / 0.46     0.91 / 0.94
"fi"           0.61 / 0.65     0.62 / 0.88     0.61 / 0.66     1.23 / 1.53
"fir"          0.56 / 0.76     0.54 / 0.56     0.54 / 0.55     1.07 / 1.12
"fire"         0.53 / 0.57     0.55 / 0.56     0.54 / 0.57     1.10 / 1.13
"firefox"      0.58 / 0.72     0.60 / 0.63     0.59 / 0.61     1.19 / 1.24
"ашку"         0.47 / 0.49     0.49 / 0.50     0.48 / 0.48     0.97 / 0.99
"otchet"       0.53 / 0.56     0.55 / 0.61     0.54 / 0.57     1.08 / 1.17
"file4"        0.55 / 0.61     0.55 / 0.59     0.57 / 0.64     1.13 / 1.23
10000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             2.15 / 2.41     2.16 / 2.26     2.13 / 2.28     4.29 / 4.55
"f"            5.10 / 6.81     5.14 / 5.60     5.13 / 5.35   10.28 / 10.92
"fi"           6.75 / 7.41     6.67 / 6.84     6.65 / 6.87   13.32 / 14.09
"fir"          5.83 / 7.36     5.79 / 6.01     5.77 / 6.13   11.57 / 12.13
"fire"         5.93 / 7.20     5.81 / 6.41     5.79 / 6.54   11.66 / 12.96
"firefox"      6.42 / 7.37     6.39 / 6.82     6.33 / 7.15   12.75 / 13.79
"ашку"         5.07 / 5.49     5.13 / 6.13     5.11 / 5.70   10.27 / 12.00
"otchet"       5.70 / 6.49     5.68 / 5.97     5.69 / 6.15   11.36 / 11.88
"file4"        5.94 / 6.13     5.99 / 6.47     6.01 / 6.31   12.01 / 12.70
test tests::root_list::ui_thread::ui_thread_time_per_keystroke ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 8.35s
=== run 4, loadavg: 8.28 6.54 12.51 2/4016 1381638
1000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             0.18 / 0.30     0.18 / 0.21     0.17 / 0.21     0.36 / 0.41
"f"            0.46 / 0.50     0.46 / 0.50     0.45 / 0.50     0.92 / 1.01
"fi"           0.61 / 0.68     0.62 / 0.72     0.60 / 0.71     1.23 / 1.41
"fir"          0.54 / 0.72     0.54 / 0.57     0.54 / 0.56     1.08 / 1.15
"fire"         0.53 / 0.56     0.56 / 0.64     0.55 / 0.65     1.10 / 1.30
"firefox"      0.59 / 0.63     0.60 / 0.92     0.59 / 0.66     1.19 / 1.61
"ашку"         0.49 / 0.63     0.49 / 0.54     0.48 / 0.52     0.97 / 1.05
"otchet"       0.53 / 0.55     0.54 / 0.68     0.54 / 0.69     1.08 / 1.38
"file4"        0.55 / 0.59     0.55 / 0.58     0.56 / 0.60     1.11 / 1.16
10000 apps; ms of UI-thread CPU, median / p95
query       search without     search with    apply answer     with, total
""             2.16 / 2.27     2.17 / 2.37     2.13 / 2.17     4.30 / 4.53
"f"            5.11 / 5.31     5.11 / 5.55     5.08 / 5.64   10.20 / 10.97
"fi"           6.70 / 7.17     6.68 / 6.91     6.65 / 7.20   13.33 / 14.31
"fir"          5.87 / 7.36     5.81 / 6.22     5.79 / 6.03   11.61 / 12.23
"fire"         5.75 / 7.49     5.82 / 6.93     5.80 / 6.70   11.62 / 13.45
"firefox"      6.33 / 7.51    6.34 / 10.39    6.33 / 10.68   12.67 / 20.62
"ашку"         5.10 / 6.88     5.13 / 5.37     5.09 / 5.36   10.24 / 10.64
"otchet"       5.77 / 7.08     5.71 / 5.77     5.70 / 5.80   11.40 / 11.51
"file4"        5.99 / 8.05     5.97 / 6.04     5.98 / 6.04   11.95 / 12.07
test tests::root_list::ui_thread::ui_thread_time_per_keystroke ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 8.39s
```
