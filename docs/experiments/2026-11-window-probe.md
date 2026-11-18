# Window probe (plan 1, Task 9)

Date: 2026-11-18. Probe: `crates/ui/examples/window_probe.rs`.
GPUI: `gpui-kit =0.10.0` → `gpui-pre =0.3.12`, `gpui-component 0.10.0`, `gpui-base 0.10.0`.

Status: Linux steps 1–3 are done. Step 4 (macOS, Windows, and `--interactive` everywhere, KDE included) is a
manual gate for the user; see [How to run on macOS / Windows](#how-to-run-on-macos--windows-step-4) and
[How to run `--interactive` on KDE](#how-to-run---interactive-on-kde).

## Decisions for Task 10 and Task 13

| Question | Answer on Linux / KDE Wayland | Confidence |
|---|---|---|
| Recreate or persistent window? | **Recreate, for now.** GPUI 0.3.12 has no way to hide a window on Linux: `App::hide` is a logged no-op and `Window` has no hide/show. Latency ✅: a warm show reaches `on_next_frame` in a median of ~17 ms. RSS ⚠️: the hidden-window RSS budget (~200 MB) is **exceeded after about 100 shows** (202–203 MB) and keeps growing (211 MB after 400), see [Memory](#memory). | Latency: high. RSS: medium-to-low, because the growth is unprofiled and its long-run trend is unknown. macOS/Windows have a persistent path in the probe that the user has not run yet (Step 4). |
| Does prewarming the renderer help? | **Yes, it is required.** Without it the first show takes ~200 ms. With it the first show takes ~23 ms. Opening a 1×1 window and closing it in the same tick is enough. Waiting for its first frame (`--prewarm-frame`) gives no extra benefit. The prewarm itself blocks the main thread for ~190 ms, so do it at daemon start. | High |
| Cause of the white backdrop | The gpui-kit Root, i.e. `gpui_kit::open_window`. It is not the Wayland backend. There are two parts, see [Backdrop](#white-backdrop-step-3). | High (verified with screenshots) |
| How to remove it | On Linux, open the launcher window with bare `cx.open_window` (the `--root plain` variant), not `gpui_kit::open_window`. Still call `gpui_kit::init`. The `gpui-component` `Input` works in a plain root; this was checked by rendering it. No GPUI patch is needed. | High on Linux. Unverified on macOS/Windows (Step 4). |

Task 10 notes:
- Use `--root plain`'s recipe:
  - `cx.open_window(WindowOptions { window_background: Transparent, kind: LayerShell { layer: Overlay, .. }, .. }, |_, cx| cx.new(..))`.
  - The view's root `div` draws the panel: rounded, semi-transparent fill.
  - Keep `KeyboardInteractivity::None` under `TOMAKAST_NO_KEYBOARD=1`.
- Prewarm with a 1×1 layer-shell window, `KeyboardInteractivity::None`, `focus: false`, opened and removed at startup.
- `cx.set_quit_mode(QuitMode::Explicit)` keeps the process alive after the last window closes. This was confirmed: every probe run prints its table after the window has been removed, and then exits only through `cx.quit()`.
- A plain root has no gpui-component `WindowState`, so dialogs, sheets, notifications and tooltips from gpui-component will not work in that window. The launcher needs only an input and a list, so this is acceptable. If they are ever needed, use the GPUI patch [described below](#gpui-patch-not-applied).

## Setup

- KDE Plasma 6.8.3, Wayland; one 3840×2160 output at 240 Hz, scale 1.5.
- GPU: NVIDIA RTX 5070 Ti (Vulkan; the system HDR Vulkan layer prints `[HDR Layer]` lines, which are not ours).
- `cargo build --release -p tomakast-ui --example window_probe`.
- All Linux runs:
  - layer-shell, layer `Overlay`, `KeyboardInteractivity::None`, `focus: false`; nothing grabbed the keyboard;
  - a 720×460 window, transparent background, 14 px panel radius, semi-transparent fill;
  - the window is shown for `--interval-ms`, then hidden for `--interval-ms`; there is the same gap before the first show with or without prewarm.

Metrics, per show, measured from the show request (`open_window` call, or the native show for `persistent`):
- `render`: time to the first `ProbeView::render`;
- `on_next_frame`: time to the first `window.on_next_frame` callback registered in that render. GPUI runs it at the start of the next frame request, i.e. after the first frame was presented and the compositor asked for the next one. It is the closest available proxy for "first frame on screen". It is not a hotkey → frame measurement; the probe measures that in `--interactive` mode only.
- `first` is show #0; median and p95 (nearest rank) are over warm shows #1…#N-1.

## Results on Linux (Step 2)

`--strategy recreate`, 10 cycles, 700 ms interval. All numbers are in ms.

| root | prewarm | render first | render median | render p95 | frame first | frame median | frame p95 |
|---|---|---|---|---|---|---|---|
| kit | off | 209.1 | 15.7 | 16.4 | 217.7 | 17.5 | 19.8 |
| kit | `--prewarm` | 14.8 | 15.7 | 19.8 | 23.7 | 17.6 | 21.4 |
| kit | `--prewarm-frame` | 15.8 | 16.2 | 20.0 | 25.6 | 17.9 | 22.2 |
| plain | off | 191.3 | 15.7 | 16.3 | 199.4 | 16.7 | 17.3 |
| plain | `--prewarm` | 14.5 | 15.6 | 19.6 | 22.6 | 16.8 | 20.7 |
| plain | `--prewarm-frame` | 15.3 | 15.2 | 16.1 | 23.3 | 16.2 | 17.1 |
| kit-transparent | off | 192.5 | 15.4 | 16.5 | 201.1 | 17.2 | 18.4 |
| kit-transparent | `--prewarm` | 16.5 | 15.3 | 17.6 | 25.4 | 17.1 | 19.2 |
| kit-transparent | `--prewarm-frame` | 15.0 | 15.8 | 16.3 | 23.6 | 17.5 | 18.1 |

`--strategy persistent` on Linux prints `unsupported` and exits. The reason is given in the decisions table.

Prewarm cost: opening and closing the 1×1 window takes 187–202 ms on the main thread.

Repeat runs of the Step 1 command (`--strategy recreate --prewarm`) gave `on_next_frame` first 23.2–24.4 ms and median 17.1–18.6 ms.

Criteria:

| Criterion | Target | Result |
|---|---|---|
| median warm show to `on_next_frame` | ≤ 50 ms | 16–18 ms ✅ |
| `first` with `--prewarm` | ≤ 50 ms | 22–26 ms ✅ |
| idle, window hidden | 0 CPU ticks | 0 ticks ✅ |

Where the warm ~15 ms goes (hypothesis, not profiled): each recreate makes a new layer surface, waits for its configure, and creates a new Vulkan swapchain (the HDR layer logs 3 swapchain creations per window). At 240 Hz this is not a vsync wait.

### Idle

`--cycles 3 --idle 10`: CPU time (utime+stime from `/proc/self/stat`) over 10 s with the window hidden.

| Run | CPU ticks in 10 s | RSS |
|---|---|---|
| kit root, `--prewarm` | 0 | 189 MB |
| plain root | 0 | 188 MB |
| no window ever opened (`--cycles 0`) | 0 | 27 MB |

### Memory

| State | RSS |
|---|---|
| after `gpui_kit::init`, before any window | 27 MB |
| after prewarm, or after the first window | 184–189 MB |
| after 10 recreate cycles, window hidden | 188–189 MB |
| after 30 cycles | 194 MB |
| after 100 cycles (kit / plain) | 203 / 202 MB |
| after 400 cycles (kit / plain) | 211 / 211 MB |

- About 160 MB comes with the first window (renderer, Vulkan driver, fonts). Root kind does not matter.
- Right after startup (prewarm or first window) RSS is within the ~200 MB budget, with little margin.
- **With recreate the budget is exceeded after about 100 shows:** 202–203 MB after 100 shows and 211 MB after 400, with both roots. A daemon shows the launcher thousands of times, so this growth is the real finding, not the startup number.
- The growth is sublinear: about +0.14 MB per show over the first 100 shows, then about +0.03 MB per show up to 400. A hypothesis is allocator growth that slows down rather than a per-window leak. It was not profiled, so the trend beyond 400 shows is unknown.
- **Action items for Task 13:**
  1. Run a long-run RSS measurement: `window_probe --strategy recreate --prewarm --root plain --cycles 3000 --interval-ms 50`, plus the daemon itself after a few thousand toggles. Threshold: RSS with the window hidden stays ≤ 200 MB, or at least plateaus (< +5 MB over the last 1000 shows).
  2. If it fails, the fallback is a GPUI patch that adds hide/show for layer surfaces (unmap by attaching a null buffer, remap on show). That allows the persistent strategy on Linux. Measure the persistent strategy against the same budgets before adopting it.
  3. Until then, recreate stays the strategy because it is the only one possible on Linux without a patch.
- The very first run after the build showed 244 MB once and did not repeat. A plausible cause is first-time creation of a driver or pipeline cache.

## White backdrop (Step 3)

Method:
- Screenshots were taken with `spectacle -b -n -f` while the probe window was shown, over a dark desktop. They were cropped and zoomed at the window corners.
- The screenshots stayed in the session scratchpad and are not committed, because they include the user's screen.

| `--root` | What is visible |
|---|---|
| `kit` (`gpui_kit::open_window`) | An opaque white rectangle with square corners behind the rounded panel. The semi-transparent panel looks opaque grey because it sits on white. A 1 px square frame line. The panel is offset by 20 px right and down from the screen centre. |
| `plain` (bare `cx.open_window`) | Correct: fully transparent outside the rounded 14 px panel, and the desktop shows through the semi-transparent fill. Centred. The same holds with the gpui-component `Input` inside (`--with-input`). |
| `kit-transparent` (kit Root with its instance style set to `bg(transparent)`) | The white fill is gone and the panel is translucent. A square 1 px frame line and the 20 px offset remain. |

Cause. There are two parts, and both come from gpui-component's root plugin `WindowState`, which `gpui_component::init` registers with `gpui_base::Root`:

1. **The white fill.** `WindowState::style` (`gpui-component-0.10.0/src/root.rs`) paints the Root surface with `cx.theme().tokens.background` (white in the light theme). Its corner radii come from `window_corner_radii`: `theme.window_radius` for client-side decorations, otherwise 0. The default theme sets `window_radius: px(0.)` (`src/theme/mod.rs`), so the white fill has square corners even though layer surfaces count as client-decorated (see 2).
   - Removing it: the Root's instance style is refined after plugin styles, so `root.style().background = Some(transparent_black().into())` overrides it. The `kit-transparent` variant proves this.
2. **The frame and offset.** `WindowState::decorate` wraps the surface in `window_border()` (`src/window_border.rs`). With `Decorations::Client`, on Linux, that border:
   - sets `window.set_client_inset(20px)`, which grows the surface by 40 px and moves the content 20 px in;
   - draws a 1 px `theme.window_border` frame and a drop shadow.

   Layer-shell surfaces always report `Decorations::Client`:
   - `gpui-pre-linux-0.3.12/src/linux/wayland/window.rs` initialises `decorations: WindowDecorations::Client`;
   - only an xdg-decoration configure can change that, and layer surfaces have no xdg-decoration object.

   So the kit Root always treats the launcher overlay as a CSD window.

It is not the Wayland backend's transparency:
- with `WindowBackgroundAppearance::Transparent` it does not set an opaque region and switches the renderer to transparent;
- `--root plain` renders correctly.

### Fix

The recommended fix needs no patch: use a plain root (bare `cx.open_window`) for the launcher window, and draw the panel in the view.

### GPUI patch (not applied)

This patch is only needed if the launcher must use the kit Root, for example to get gpui-component dialogs, tooltips or notifications in that window. It was not set up, following the controller ruling.

- In `gpui-pre-linux` `wayland/window.rs`, create layer-shell windows with `decorations: WindowDecorations::Server`. Layer surfaces have no decorations of any kind, so "not client-drawn" is the correct report.
- With that, `window_border()` becomes a pass-through and the corner radii become 0.
- The app then only needs the instance-style `bg(transparent)` override above.
- Wire it in with `[patch.crates-io] gpui-pre-linux = { git = "<fork>", branch = "layer-shell-decorations" }`. This is also an upstream bug-report candidate.

## Notes for Task 11 (ashpd)

- gpui 0.3.12 already depends on `ashpd 0.13` (zbus 5).
  - The probe uses `ashpd = { version = "0.13", default-features = false, features = ["async-io", "global_shortcuts"] }` as a Linux dev-dependency, so `Cargo.lock` keeps a single ashpd.
  - In 0.13, `global_shortcuts` is a separate feature.
- The workspace still declares `ashpd = "0.14"` (zbus 6).
  - zbus 6 is already in the tree via `notify-rust` (from gpui-pre-linux), so 0.14 would not add a third zbus. It would add a second ashpd.
  - Using 0.13 in Task 11 avoids that.
- The portal `Activated` signal carries `timestamp()` as a `Duration`. Its clock base is not documented; `--interactive` prints it next to `CLOCK_MONOTONIC` and realtime, so the user's KDE run will show which one it is.
- For a non-sandboxed process the probe calls `ashpd::register_host_app("dev.tomakast.WindowProbe")` before `GlobalShortcuts`. The KDE run will show whether that name, or something else, appears in the confirmation dialog.

## Cross-platform code status

The Linux `--interactive` path has **not been run** either. That covers the portal code (`register_host_app`, `create_session`, `bind_shortcuts`, `receive_activated`) and the hotkey toggle, Esc and deactivation wiring. Its first run is the user's KDE check.

The macOS and Windows branches of the probe have **not been run**:
- persistent hide/show:
  - macOS: `orderOut:` / `orderFrontRegardless` or `makeKeyAndOrderFront:` on the `NSWindow` from the raw handle;
  - Windows: `ShowWindow(SW_HIDE / SW_SHOWNOACTIVATE)`;
- RSS: `proc_pidinfo` / `GetProcessMemoryInfo`;
- CPU: `getrusage` / `GetProcessTimes`;
- `global-hotkey 0.10` for Alt+Space.

They do type-check: `cargo clippy -p tomakast-ui --example window_probe -- -D warnings` passes for `aarch64-apple-darwin` and `x86_64-pc-windows-gnu`. To get there, the cross-check used a stub `stitched_shaders.metal` and a stub `windres`. Neither is committed; they were needed only for the check, not for a real build on those OSes. CI (`cargo test --workspace` on macos-latest and windows-latest) compiles the example natively.

On macOS and Windows the window is `WindowKind::PopUp` with `titlebar: None`.
- macOS: GPUI makes it a non-activating `NSPanel` at pop-up level that can join all spaces.
- Windows: GPUI gives it `WS_EX_TOOLWINDOW | WS_EX_TOPMOST`, which should mean no taskbar or Alt-Tab entry.
- Non-interactive runs use `focus: false`.

## How to run on macOS / Windows (Step 4)

Build (both OSes, from the repository root):

```sh
cargo build --release -p tomakast-ui --example window_probe
# macOS: ./target/release/examples/window_probe <flags>
# Windows: .\target\release\examples\window_probe.exe <flags>
```

1. **Timing matrix.** Run each line, then copy the two table rows and the RSS line into the results table below.

   ```sh
   window_probe --strategy recreate
   window_probe --strategy recreate --prewarm
   window_probe --strategy persistent
   window_probe --strategy persistent --prewarm
   window_probe --strategy recreate --prewarm --root plain
   window_probe --strategy persistent --prewarm --cycles 3 --idle 10
   window_probe --strategy recreate --prewarm --cycles 3 --idle 10
   ```

2. **Look.** Put a light, busy window (for example a web page) behind the screen centre, then run:

   ```sh
   window_probe --root kit --cycles 1 --interval-ms 5000
   window_probe --root kit-transparent --cycles 1 --interval-ms 5000
   window_probe --root plain --cycles 1 --interval-ms 5000
   ```

   For each one, note:
   - whether there is any opaque or white area outside the rounded panel;
   - whether the corners are rounded;
   - whether the background shows through the panel.

3. **Focus and taskbar.** While a non-interactive run is cycling (`window_probe --cycles 10 --interval-ms 1500`), type in another app.
   - macOS: does typing continue, and does the other app stay active (menu bar title)?
   - Windows: does the probe appear in the taskbar or in Alt-Tab?

4. **Interactive.** Run `window_probe --interactive`, then work through the [interactive checklist](#interactive-checklist).
   - Variants: `window_probe --interactive --strategy persistent` and `window_probe --interactive --root plain`.
   - macOS may ask for Accessibility or Input Monitoring permission for global hotkeys. Grant it to the terminal, then restart the probe.

### Results table (fill in)

| OS | flags | render first / median / p95 | on_next_frame first / median / p95 | RSS before / after first / after last | idle CPU |
|---|---|---|---|---|---|
| macOS | `--strategy recreate` | | | | |
| macOS | `--strategy recreate --prewarm` | | | | |
| macOS | `--strategy persistent` | | | | |
| macOS | `--strategy persistent --prewarm` | | | | |
| macOS | `--strategy recreate --prewarm --root plain` | | | | |
| macOS | `--strategy persistent --prewarm --idle 10` | | | | |
| macOS | `--strategy recreate --prewarm --idle 10` | | | | |
| Windows | `--strategy recreate` | | | | |
| Windows | `--strategy recreate --prewarm` | | | | |
| Windows | `--strategy persistent` | | | | |
| Windows | `--strategy persistent --prewarm` | | | | |
| Windows | `--strategy recreate --prewarm --root plain` | | | | |
| Windows | `--strategy persistent --prewarm --idle 10` | | | | |
| Windows | `--strategy recreate --prewarm --idle 10` | | | | |

| OS | check | kit | kit-transparent | plain |
|---|---|---|---|---|
| macOS | no backdrop outside the rounded panel | | | |
| macOS | rounded corners | | | |
| Windows | no backdrop outside the rounded panel | | | |
| Windows | rounded corners | | | |

| OS | check | result |
|---|---|---|
| macOS | non-interactive window does not steal focus from the current app | |
| Windows | no taskbar icon | |
| Windows | no Alt-Tab entry | |

## How to run `--interactive` on KDE

This mode grabs the keyboard (`KeyboardInteractivity::Exclusive`) while the window is shown.
- Esc or Alt+Space hides it.
- Every show also auto-hides after 30 s as a safety net.
- `TOMAKAST_NO_KEYBOARD=1` turns the grab off (but then you cannot type into it).

```sh
cargo run --release -p tomakast-ui --example window_probe -- --interactive
# variants:
cargo run --release -p tomakast-ui --example window_probe -- --interactive --root plain
```

1. On the first run KDE should show a portal dialog to confirm the `Alt+Space` shortcut. Note the application name it shows.
2. Press Alt+Space and work through the checklist.
3. Quit with Ctrl+C while the window is hidden. Start the probe again and note whether KDE asks for confirmation again.
4. Copy the `[probe] bound …`, `[probe] hotkey: … timestamp=…` and `[probe] show #…` lines into the table.

### Interactive checklist

| Check | KDE | macOS | Windows |
|---|---|---|---|
| Alt+Space shows and hides the window | | | |
| Text can be typed into the field | | | |
| macOS: typing works without activating the app (other app's menu bar stays) | — | | — |
| macOS: focus returns to the previous app after hiding | — | | — |
| A click outside the window closes it (`[probe] window active = false`) | | | |
| KDE: portal asks for confirmation again after a process restart | | — | — |
| KDE: application name shown by the portal dialog | | — | — |
| Portal `Activated` timestamp present; which clock it matches (monotonic / realtime / other) | | — | — |
| `hotkey received -> frame` (ms), a few presses | | | |
| `request -> on_next_frame` (ms), a few presses | | | |
