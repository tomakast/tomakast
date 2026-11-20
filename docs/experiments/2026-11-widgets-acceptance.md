# Widgets acceptance (Task 15)

Date: 2026-11-20. Branch `fb-calc`, measured at `edc7098` (Task 15 changes docs and WIT doc
comments only). Machine: the plan 1 setup (KDE Plasma 6.8.3, Wayland, 16 threads, kernel
7.3.5-1-cachyos).

Five parts: [the freeze](#the-freeze), [measurements](#measurements), [rulings](#rulings),
[known limitations](#known-limitations) and the [user checklist](#user-checklist) (result
cells blank).

## The freeze

WIT 0.1 was edited in place for widgets (the user's ruling: no parallel session and no outside
plugin depended on it yet).

- `crates/plugin-api/wit/since_v0.1.0/plugin.wit` now has the sha256
  **`5d8208a01debdd8ec4df0628ee17dd9477d5fa734af4ce6e7ef7a63f326adb32`**, recorded in
  `FROZEN` (`sha256sum -c FROZEN` in that folder prints `plugin.wit: OK`).
- `FROZEN.md` has three rows for this work. `1168025ee612` (2026-11-19) is the widgets change
  on its own branch: `[[widgets]]`, `search-answer`, `render-widget`, the three widget
  effects, the `widget` event, the frame and canvas records, and `locale.local-time-zone`.
  `4c6d12a2da2d` is the Lucide `icon.named` doc of `plan-2b2b-sdk-calc`. The 2026-11-20 merge
  row, now `5d8208a01deb`, holds the hash of the merged file. Every later in-place doc edit
  updated that row, with no new row:
  - node nesting ≤ 32 and overlay sizing (Task 9);
  - how the canvas draws, and that a slow canvas is stopped (Task 10);
  - the SVG cost caps (Task 11);
  - Task 15: `icon.named` describes the Linux theme-icon fallback, and the `effect` doc says
    that an answer to a widget event that is not a user action has its `push-view`,
    `pop-view` and `close-window` dropped. Both were already what the launcher and host did.
- `crates/plugin-host/tests/wit_coverage.rs` requires every widget case, with no `PENDING`
  list. Without it (Task 14, Step 1) the test failed on 39 names:
  - color `rgba`;
  - draw `ellipse`, `image`, `path`, `pop-clip`, `pop-transform`, `push-clip`,
    `push-transform`, `rect`, `svg`, `text`;
  - effect `redraw-widget`, `set-widget-image`;
  - image-format `jpeg`, `png`, `svg`;
  - node-kind `button`, `canvas`, `icon`, `image`, `overlay`, `progress`, `spacer`;
  - paint `linear`, `solid`;
  - path-segment `close`, `cubic-to`, `line-to`, `move-to`, `quad-to`;
  - pointer-kind `down`, `enter`, `leave`, `move`, `up`;
  - widget-event `hidden`, `key`, `pointer`, `shown`.

  Kitchen-sink's `anim` canvas and `probe` mode `all` now cover them.
- `crates/core/tests/wit_parity.rs` and `crates/plugin-api/tests/freeze.rs` pass after the
  re-hash.

## Measurements

Release builds (`just build`: `tomakast`, `tomakast-raster` and both plugins). An agent cannot
open the real launcher (it grabs the keyboard), so everything that needs the hotkey path, the
compositor or the GPU is in the [user checklist](#user-checklist) as "to be measured by the
user". What runs headless is below.

| What | Measured | Budget | How |
|---|---|---|---|
| 1. Warm open with three always tiles (kitchen-sink `static`, `buttons`, `anim`), against the same open with none | to be measured by the user (M1) | ~50 ms, and at most the no-tile number + 5 ms | median of 20 opens, from the `launcher show request -> on_next_frame` debug line |
| 2. CPU with the `anim` 60 fps canvas on screen, against the launcher open with no animating tile; render rate | to be measured by the user (M2) | renders ≤ 60 per s | `pidstat -u -p <daemon pid> 1 30`; the `widgets` trace target |
| 3. CPU with the launcher hidden; renders while hidden | to be measured by the user (M3) | ~0 % CPU; **0** renders | `pidstat` over 60 s; the `widgets` render lines between the hide and the next show |
| 4. Calc's tile, rendered through the host (calc in wasm, round trip through the core) | **1.01 ms**, best of 5 | 16 ms (soft deadline) | `cargo test --release -p tomakast-plugin-host --test calc -- --include-ignored --nocapture the_answer_tile_renders_well_inside_16_ms` |
| Open path: no plugin call before the first frame | the cached frame is drawn before any render; pass | none on the open path | `cargo test --release -p tomakast-ui --test strip reopen_draws_the_cached_frame_before_any_render` |
| Hidden: zero renders | pass | 0 | `cargo test --release -p tomakast-core --test widgets hidden_launcher_means_zero_renders` |
| Only visible tiles render (200 tiles) | pass | viewport + the peeking tile | the same suite, `only_visible_tiles_render_among_two_hundred` |
| 60 renders per s per tile | pass (both tests) | ≤ 60 | the same suite, `renders_are_limited_to_60_per_second_per_tile` and `ticks_render_at_most_60_per_second` |
| Canvas paint, UI thread, 10 000 commands of one kind in a 355 × 120 canvas, best of 5 | path of 10 000 segments **1.19 ms**; rects apart **4.77 ms**; quads stacked **24.3 ms**; rects **44.2 ms**; ellipses **43.5 ms**; texts **35.7 ms** | 8 ms asserted for the path and rects apart; the rest printed (the paint guard stops them) | `cargo test --release -p tomakast-ui --test strip canvas_paint_cost -- --nocapture` |
| SVG at the filter budget (120 000 units), CPU time in the raster code | turbulence 346 × 346 **10.98 ms**; blur 199 × 200 **8.69 ms** | 16 ms (the raster deadline) | `cargo test --release -p tomakast-raster --lib a_document_at_the_budget_renders_in_time -- --nocapture` |
| `calc.wasm` | **964,408 bytes**, 17,820 under the gate | gate 982,228 | `just build`, then `ls -l target/plugins/wasm32-wasip2/release/calc.wasm` |
| `kitchen_sink.wasm` | **318,775 bytes** | none | the same |

Notes:

- The calc round trip is an upper bound of about 1 ms granularity: the harness polls for the
  frame with 1 ms sleeps. Task 13 measured 1.06 ms the same way.
- The canvas numbers leave out the GPU's rasterizing and glyph rasters (the test text
  system). They show why the caps are not a speed promise: at 10 000 small primitives of one
  kind, rects, ellipses and texts take 36–44 ms on the UI thread. The paint guard stops such
  a widget after 5 paints over 8 ms in a row, or one over 50 ms. Kitchen-sink's `anim` holds
  about a dozen primitives.

## Rulings

The builder's calls of the spec ("Left to the builder"), and the controller's rulings during
the plan, one line each.

- Tile height **120 px** (three 40 px list rows); the gap between tiles **8 px**; 8 px above
  and below the strip; corner radius 10 px; focus ring 2 px.
- The left-edge peek: once the strip is scrolled, the tile before shows in the left margin,
  as the next one does in the right.
- The flexbox subset: `row`, `column` and `overlay`; padding, gap, `align`, `justify`, grow
  and a fixed size.
- Text styles: title 16, value 28, body 14, caption 12.
- The caps as in the plan's Global Constraints, plus node nesting ≤ 32 (root 0, ancestors
  counted).
- 5 late or failed frames in a row stop a widget, for that plugin instance, with an error
  toast; it keeps its last good frame.
- Frames are not kept across daemon restarts (YAGNI): the first open after a start shows
  placeholders.
- SVG text uses the system fonts, loaded off the UI thread (in the raster helper).
- Built-in tiles would use the same component list through the `Module` trait; no built-in
  tile in this plan.
- Calc keeps no list fallback.
- A click on a tile outside its controls runs its primary action; so does a click on a
  canvas without an id (it is not interactive).
- A pointer user action is a completed click (`down`, then `up` on the same node); a press
  alone is not (the coordinator's ruling, as the spec says).
- A stale condition tile may stay visible until its provider's next answer (a failed search
  sends one empty), but is never actionable. Enter and Ctrl+K wait up to 116 ms for the fresh
  answer, then fall back to the list's first row (the coordinator's ruling).
- In a focused canvas, keys go to the plugin, over "typing goes to the field".
- `locale.local-time-zone` was added in the same re-hash (the coordinator's ruling). Calc's
  `timezone` preference overrides it: empty means the computer's zone, and an unknown value
  means UTC with a warning.
- `payload` is in the render context and in the event (`widget-target`), so a condition
  tile's events and frames carry what `search` computed.
- Frame-level `actions` serve always tiles; a condition tile's come from its answer.
- Schema v5 for the grants' `widgets` column, behind query history's v4.
- Widget effects (`show-widgets`, `redraw-widget`, `set-widget-image`) are allowed in answer
  to `init`, `run` and `on-event`.
- A widget the user approved, removed by an update and declared again with the same id and
  title runs without a new prompt, like permissions; adding or renaming one asks again.
- `push-view`, `pop-view`, `close-window`, `copy-text` and `open` from a widget event need a
  user action (an `action`, a `key` or a completed click); otherwise they are dropped.
- Overlays take their size from their first child; later children are stacked at its
  origin.
- Icons, emoji and monograms in tiles are drawn at most as big as the tile is high (120 px).
- An unresolved `plugin-file` icon refuses the frame. The host's checks of a frame count
  inside the plugin's 16 ms soft deadline.
- Only the tiles in the viewport and the peeking one build their elements; the others are
  sized empty boxes.
- The canvas paint guard: 5 paints over 8 ms in a row, or one over 50 ms, stop the widget.
  This is skipped for a tile's first paint per showing, and is ×4 in debug builds. Only this
  stop clears the tile's frame.
- Failed tessellation recovers: a stroke is split in halves at a segment boundary (up to 8
  times), and a fill is tried once more at 4× the tolerance. A path still failing is logged
  once per widget.
- Canvas drawing: line box 1.25 × size; non-zero fills; centred strokes scaled by the mean
  scale; gradients span the shape's axis-aligned box; text ≤ 200 after scaling; emoji ignore
  the text's opacity.
- Images and SVG are decoded and rasterized in a separate helper process, `tomakast-raster`,
  one job at a time. It runs with a 512 MiB address-space limit and a CPU limit on Unix, and
  is killed past twice the 16 ms deadline (×4 in debug builds). A crash, a kill or a hit limit
  refuses that document for the module instance. The helper ships next to the `tomakast`
  binary, and `just build` builds it.
- The SVG cost model, refused before rendering:
  - ≤ 4096 elements (the UI counts usvg nodes, `<use>` copies included);
  - ≤ 32 filter primitives, and ≤ 32 applied in all;
  - `numOctaves` ≤ 10, morphology radius ≤ 32 device px, convolve order ≤ 9;
  - ≤ 16 `<image>` and ≤ 4 Mpx decoded;
  - a filter region ≤ 2× the raster;
  - a weighted filter budget of 120 000 units (the worst accepted document rasterizes in
    11.0 ms in release);
  - no filters inside a pattern, mask, clip path or `feImage`;
  - no external references, and no font fallback.
- An image re-sent with the same bytes keeps its version and costs nothing; kitchen-sink
  sends `anim`'s images once, from `init`.
- Theme icon names: the Lucide glyph, else (Linux) the freedesktop theme icon, else the
  monogram (the user's ruling; the WIT and the plugin docs now say so).
- The 60-per-second gate allows 1.5 ms of early jitter. Pointer `move`, `enter` and `leave`
  carry no request.

## Known limitations

- `locale` answers only the time zone. Language and the decimal separator (the rest of G6)
  stay open.
- Emoji glyphs in canvas text ignore the command's opacity.
- Text and images do not rotate on the native layer (use SVG).
- Frames are not kept across daemon restarts.
- The canvas caps are not affordable at their limits. 10 000 rects, ellipses or texts paint
  for 36–44 ms on the UI thread, and the paint guard stops such a widget. Before that, the
  first slow paints can still stall the launcher for up to ~50 ms.
- The raster helper's memory and CPU limits apply on Unix only. On Windows it is bounded by
  the kill past its time alone. A launcher shipped without `tomakast-raster` next to it draws
  no plugin images (the daemon logs a warning at start).
- Repeated denial toasts on hover or press over navigating nodes rely on the existing
  throttle (one per plugin per 10 s).
- A gradient's angle under a non-uniform scale is approximate.

## User checklist

Run from the worktree root, in `bash` (if your shell is fish, start `bash` first). Nothing
touches your real home (L8 alone switches the desktop's colour scheme, and restores it): `just try` uses a throwaway home under `$XDG_RUNTIME_DIR/tk-try`,
deletes it when it stops, and grabs the keyboard normally. Open and hide the launcher from a
second terminal with `just toggle`, and follow the log with `just log`. `pidstat` comes from
the `sysstat` package (`sudo pacman -S sysstat` on CachyOS).

**Measurements (M1–M3).** `just try` logs at `RUST_LOG`, else at `info,tomakast_ui=debug`,
which has the open line but not the `widgets` render trace. So start the measuring session
through `just try approve` (both plugins approved) in the background of this terminal, with
the trace on and plain log lines (`NO_COLOR=1`, so `grep` sees `ms=` and `widgets: render`).
It builds first; the `until` loop waits for the daemon, and the `sleep` gives it time to load
both plugins before the first `just toggle`:

```bash
NO_COLOR=1 RUST_LOG=info,tomakast_ui=debug,widgets=trace just try approve &
until DAEMON=$(pgrep -nf 'release/tomakast daemon'); do sleep 1; done; sleep 5
home="${XDG_RUNTIME_DIR:-/tmp}/tk-try"
log="$home/daemon.log"
median() { grep 'launcher show request' "$log" | grep -o 'ms=[0-9.]*' | cut -d= -f2 \
  | tail -20 | sort -n | awk '{a[NR]=$1} END {print "median of", NR, ":", (a[10]+a[11])/2, "ms"}'; }
opens() { for i in $(seq 21); do just toggle; sleep 1; just toggle; sleep 1; done; median; }
renders() { grep -c 'widgets: render' "$log"; }
cpu() { pidstat -u -p "$DAEMON" 1 "$1" | tail -1; }
```

Leave the keyboard alone while a loop runs: the launcher opens and hides by itself. Each
loop's first open is a warm-up; `median` takes the last 20. The no-tile baseline (M1b, M2b)
removes kitchen-sink's folder from the running daemon, so its tiles leave the strip and calc,
which shows no tile at an empty query, stays. (`[widgets] hidden` in the session's
`config.toml`, written before the daemon starts, would do the same with kitchen-sink still
loaded; the daemon reads it only at start.)

| # | What | Step | Expected | Result |
|---|---|---|---|---|
| M1a | Warm open, three always tiles | `opens` | The median is about 50 ms or less. Write it down. | |
| M1b | Warm open, no tiles | `rm -rf "$home/plugins/kitchen-sink"`; wait 2 s (the log says the folder was removed); `opens` | The median is about 50 ms. M1a is at most this number + 5 ms. | |
| M2a | CPU, `anim` on screen | First start the session again with kitchen-sink (`kill "$DAEMON"; wait`, which also deletes the home, then the block above again). Then `n0=$(renders); just toggle; cpu 30; n1=$(renders); echo "renders/s: $(( (n1 - n0) / 30 ))"`, and press Esc when it prints. | The `Average:` %CPU is low and steady (write it down). Renders per second are about 60, and not more (nearly all of them `anim`'s; the other two render once per showing). 0 means the trace is not on: check the session's `RUST_LOG`. | |
| M2b | CPU, open with no animating tile | `rm -rf "$home/plugins/kitchen-sink"`; wait 2 s; `just toggle; cpu 30`, then Esc. | %CPU near 0, well below M2a. | |
| M3 | Hidden | Start the session again with kitchen-sink, as for M2a. `just toggle`, wait 3 s, Esc (the tiles were shown and are now hidden). Then `n0=$(renders); cpu 60; n1=$(renders); echo "renders while hidden: $((n1 - n0))"`. | %CPU ~0.0; renders while hidden **0**. This counts only if M2a saw renders (the trace works). | |

Teardown: `kill "$DAEMON"; wait` (`just try` deletes the home when the daemon stops).

**The look and the keys (L1–L9).** Stop the measuring session first (teardown above). Then
`just try`, which leaves both plugins for review (L1), and go on in the same session; or
`just try approve`, which approves them up front, for L2–L9. Ctrl+C stops it and deletes the
home.

| # | What | Step | Expected | Result |
|---|---|---|---|---|
| L1 | The review lists the tiles | `just try`; `just toggle`; open "Review plugin: Kitchen Sink", then "Review plugin: Calculator". | Kitchen Sink's review says "It shows tiles in the launcher: Static, Buttons, Animation, Echo, Probe"; Calculator's names "Calculator answer". Allow both. | |
| L2 | Calc's tile | `just toggle`; type `2+2`. | Calc's double tile comes first, like Raycast's: `2+2` on the left in the monospace font over "Expression", `4` large on the right over "Number". The always tiles stay after it (Static peeks in at the right). No calculator list row. The tile is selected; Enter copies `4` (paste it somewhere). Ctrl+K shows "Copy Result" and "Copy Input + Result"; the second copies `2+2 = 4`. `100 km in mi` shows "Conversion" and "Length". | |
| L3 | The peek | At an empty query. | Three tiles: Static and Buttons fill the width, and Animation's left edge peeks in at the right margin, with no scrollbar. Scroll the strip sideways (wheel or trackpad over it): Animation comes in, and Static peeks in at the left. | |
| L4 | The scrollbar | Scroll the strip, then stop. | A thin scrollbar shows only while it scrolls, and fades out at rest. | |
| L5 | ↑, history and Tab | Type `2+2` and press Esc (the launcher hides and keeps the query in its history). `just toggle`; at the empty query press ↑ three times, then ↓ until the field is empty, then ↓ twice more. Type `say` and press ↑ three times; then ↓ twice; ↑ once more, then Esc, and Esc again (the launcher hides); `just toggle`; at the empty query press Tab. Last, type `search history` and run "Search History". | At the empty query the first ↑ moves the focus ring onto the first tile (Static), the second leaves the strip for the field (no ring, no row highlighted), and the third recalls `2+2` (with its tile selected; ↑ and ↓ keep walking the history). Past the newest entry the field is empty with nothing highlighted; ↓ then rings Static, and ↓ again highlights the first row. With the typed `say`, ↑ rings the first tile, ↑ again goes to the field, and the third ↑ changes nothing; ↓ rings the first tile, ↓ again returns to the list's first row; from there ↑ rings the first tile again, and Esc in the strip returns to the list. Tab at the empty query also puts the ring on the first tile. "Search History" lists `2+2`. | |
| L6 | Keys in the strip and Ctrl+K on a tile | Tab into the strip; → to Buttons; Tab to "+1", Enter; then type `tile:hi`, Ctrl+K. | A success toast `inc`. For `tile:hi`, the double Echo tile shows `hi`, selected; Ctrl+K lists "Copy" and "Copy twice", and Enter copies `hi`. | |
| L7 | `anim` is smooth | At an empty query, scroll to Animation and watch it for 10 s; then Tab until its canvas has the focus ring, press `a`, then Esc. | The small square glides across the tile every 2 s with no stutter, over the still shapes, text and images. With the canvas focused, `a` goes to the plugin and does not reach the search field; Esc moves the focus from the canvas back to its tile. | |
| L8 | Light and dark | This changes your real desktop colour scheme for a moment. The launcher hides when it loses focus, so switch by a delayed command. Note your scheme (`kreadconfig6 --group General --key ColorScheme`; `BreezeDark` on this machine), run `(sleep 8; plasma-apply-colorscheme BreezeLight) &` and `just toggle` at once, and watch the tiles; then the same with `BreezeDark`. Restore yours with `plasma-apply-colorscheme <the noted name>`. | The tiles follow each switch while open: text, borders and the colour of `anim`'s square change with the launcher's own colours; nothing is stretched or stale. Both themes look right. | |
| L9 | Theme icon fallback | Type `say hello`. | "Say hello" (icon name `face-slightly-smiling`, not a Lucide name) shows the icon of that name from your icon theme if Breeze has one, else a "K" monogram. `calculator` and `file-clock` (type `calculator`) draw the Lucide glyphs. | |
