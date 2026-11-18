# Plan 2b-1 acceptance (Task 10)

Date: 2026-11-18. Binary: `target/release/tomakast` built from branch `plan-2b1-views` (`cargo build --release -p tomakast-app`).
Machine: the plan 1 setup (KDE Plasma 6.8.3, Wayland, 3840×2160 at 240 Hz, scale 1.5, NVIDIA RTX 5070 Ti on Vulkan).

Two parts: [agent measurements](#agent-measurements) and the [user checklist](#user-checklist) (result cells blank).

## Agent measurements

Run: `TOMAKAST_HOME=<temp dir> TOMAKAST_NO_KEYBOARD=1 TOMAKAST_DEMO=1 RUST_LOG=info,tomakast_ui=debug`, release build, real app directories scanned read-only.
`tomakast daemon &`, wait for the socket + 13 s, then 500 `tomakast toggle` calls (250 shows) with a 50 ms sleep; RSS sampled with the window hidden. 0 toggle failures. The log confirms `TOMAKAST_DEMO=1: the demo module is registered`.

| Metric | Plan 1 | Plan 2b-1 | Change |
|---|---|---|---|
| Warm open, median (`launcher show request -> on_next_frame`) | 17.8 ms (other metric, see below) | 20.8 ms (p95 23.1, max 26.6, n = 249) | see the like-for-like comparison |
| First open after start | 33.9 ms | 40.5 ms | varies 33–40 ms between runs |
| RSS hidden, after start | 187.7 MiB | 187.7 MiB | 0% |
| RSS hidden, after 200–250 shows | 220.3 MiB (200 shows) | 219.3 MiB (250 shows) | −0.5% ✅ |

### Like-for-like comparison with master

Plan 1's 17.8 ms is not a valid baseline: the same machine and protocol on master (28e2d17, which already measures from request receipt) gives about 21 ms. So master and this branch were run back to back, twice each, same protocol (500 toggles, `TOMAKAST_NO_KEYBOARD=1`, no `TOMAKAST_DEMO`):

| Build | Warm median (run 1 / run 2) | First open | RSS hidden after 250 shows |
|---|---|---|---|
| master 28e2d17 | 21.1 / 20.6 ms | 39.1 / 33.8 ms | 217.8 / 219.8 MiB |
| plan-2b1-views | 20.7 / 20.7 ms | 33.1 / 38.0 ms | 220.9 / 220.6 MiB |

No regression: warm median is the same within noise (±0.5 ms), RSS differs by ≤ 1.5%. The first run above (with `TOMAKAST_DEMO=1`: 20.8 ms, 219.3 MiB) is in the same range, so the demo module costs nothing measurable. The apparent +17% against plan 1 comes from the earlier measurement, not from plan 2b-1.

The one log line matching `error|panick` in every run is `ERROR freedesktop_desktop_entry::exec: Unknown field code: '%%F'`: the known plan 1 noise from an installed `.desktop` file. No panic or other error.

## User checklist

Run from the worktree root. If your shell is fish, run `bash` first. Everything here uses a throwaway home, so no real config, database or autostart entry is touched and **no backup is needed**. The keyboard is grabbed normally (`TOMAKAST_NO_KEYBOARD` is unset).

```bash
cargo build --release -p tomakast-app
export TK="$PWD/target/release/tomakast"
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-accept.XXXX)
unset TOMAKAST_NO_KEYBOARD
TOMAKAST_DEMO=1 RUST_LOG=info,tomakast_ui=debug "$TK" daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!
```

Open the launcher with `"$TK" toggle` (or the hotkey, if KDE accepted it). Type `demo` to list the demo items.

| # | Check | How | Expected | Result |
|---|---|---|---|---|
| D1 | Demo: List | Enter on "Demo: List" | A list with two sections (Fruit, Vegetables) and a detail pane that follows the selection. Typing filters the rows. Ctrl+K on a row shows "Pick" / "Pick again"; choosing one shows a Success toast. | |
| D2 | Demo: Grid | Enter on "Demo: Grid" | 12 tiles, 4 columns. Arrow keys move the selection in both directions; Enter on a tile shows its action. | |
| D3 | Demo: Detail | Enter on "Demo: Detail" | The markdown shows the `<script>` and `<div>` text as plain text, not rendered. The remote image is **not** loaded (no network request). Nothing opens until clicked. Clicking the https link opens example.com in the browser; clicking the `file:///etc/passwd` link does nothing. The metadata block is shown. | |
| D4 | Demo: Form | Enter on "Demo: Form"; fill every field (text, multiline, checkbox, dropdown, date) and submit | A Success toast lists every value. Tab moves between fields. | |
| D5 | Demo: Scratchpad | Enter on "Demo: Scratchpad"; type lines, one containing `err` | Every non-empty line gets its length on the right; the `err` line is marked as an error. Fast typing never loses characters. | |
| D6 | Demo: Slow | Type `slow` | After ~300 ms a "Slow answer for …" row appears. Other rows stay responsive while it is pending. | |
| D7 | Fast typing with Demo: Slow | Type `slow demo slow` quickly, backspace, retype | No flicker of stale rows: only the answer for the latest query is shown; the UI never stalls. | |
| D8 | Demo: Panic | Enter on "Demo: Panic" | An Error toast appears; the launcher keeps working (typing, Esc, another item). The log has `module demo panicked`. | |
| D9 | Apps: show in file manager | Type an app name, Ctrl+K, choose "Show in File Manager" | The file manager opens with the `.desktop` file selected, or an Error toast says why not. | |
| D10 | Apps: copy path | Ctrl+K on an app, choose "Copy Path", paste somewhere | The `.desktop` path is pasted; a "Path copied" toast showed. | |
| D11 | Esc depth | Open Demo: List, then Esc once, again, again | The first Esc closes the action panel (or an open dropdown) if one is open; otherwise it pops one view. Esc never clears the query; Esc at the root closes the launcher. Note any step that skips a level. | |
| D12 | Toast timing | Trigger a Success toast (D4) and an Error toast (D8) | Each appears at once and disappears on its own; note roughly how long each stays (target: a few seconds). | |

Stop the daemon:

```bash
kill "$DAEMON"; rm -rf "$TOMAKAST_HOME"; unset TOMAKAST_HOME
```
