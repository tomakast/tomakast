# Launcher visual refinement: acceptance

Date: 2026-11-18. Branch `visual-refine`. Spec: `docs/superpowers/specs/2026-11-18-launcher-visual-refine-design.md`. Plan: `docs/superpowers/plans/2026-11-18-visual-refine.md`. Machine: KDE Plasma 6.8.3 Wayland, KWin 6.8.3, scale 1.5.

## Agent measurements

### Warm-open latency (plan 2b-1 protocol)

Run: release build at `af8bd82`, `TOMAKAST_HOME=<temp dir>`, `TOMAKAST_NO_KEYBOARD=1`, `RUST_LOG=info,tomakast_ui=debug`, no `TOMAKAST_DEMO` and no `TOMAKAST_DEV_*`. `tomakast daemon &`, wait for the socket and then 13 s, then 500 `tomakast toggle` calls (250 shows) with a 50 ms sleep. The metric is `launcher show request -> on_next_frame` from the debug log. RSS is sampled with the window hidden.

| Run | Warm median | p95 | max | n | First open | RSS hidden, after start | RSS hidden, after 250 shows | Toggle failures |
|---|---|---|---|---|---|---|---|---|
| 1 | 18.8 ms | 21.5 ms | 44.6 ms | 248 | 34.3 ms | 189.0 MiB | 221.1 MiB | 0 |
| 2 | 19.7 ms | 22.1 ms | 25.6 ms | 249 | 35.2 ms | 189.2 MiB | 221.0 MiB | 0 |

- Target: median ≤ 22.7 ms, which is the plan 2b-1 baseline of 20.7 ms plus 2 ms. ✅ Both runs are at or below that baseline. Blur, the shadow margin and the new chrome cost nothing measurable.
- RSS after 250 shows is 221 MiB, against 220.6–220.9 MiB on plan-2b1-views. That is within noise.
- Run 1's 44.6 ms maximum is a single outlier: its p95 is 21.5 ms.
- Run 1 has 248 warm samples instead of 249: one show logged no frame time.
- One log line matches `error|panick` in each run: `ERROR freedesktop_desktop_entry::exec: Unknown field code: '%%F'`. This is the known noise from an installed `.desktop` file. There were no panics.

### KWin open and close animation (from Task 1)

- With the layer-shell namespace `tomakast-launcher`, KWin's `scale` effect animated the launcher surface for about 205 ms on show and on hide. D-Bus `activeEffects` polls reported `"blur" "scale"`, and the captured frames showed the panel scaled down and half transparent.
- Cause:
  - KWin maps a layer-shell namespace to a window type, and an unknown namespace becomes a Normal window;
  - `scale` animates `normalWindow || dialog`;
  - `fadingpopups` animates popups, OSDs and similar types.
  - `utility` is the type that neither effect animates.
- Fix: the namespace is now `utility`, unconditionally (Ruling 5). After the change, all 69 show polls and all 69 hide polls report `"blur"` alone. The panel is at full opacity and full size in the first captured frame. Our code has no animation of its own.
- Trade-off: the namespace no longer identifies tomakast. Users of compositors with namespace rules (Hyprland `layerrule`, sway) cannot target the launcher by name, and for KWin it means "utility window". To undo it, change `LAYER_NAMESPACE` in `crates/ui/src/window.rs`. With the old namespace, the KDE workaround is to turn off System Settings → Desktop Effects → Scale.

### Screenshots

Screenshots are **not committed**, because the blurred backdrop is the user's desktop (Ruling 4). They are kept in the git-ignored `.superpowers/sdd/2026-11-18-visual-refine/shots/`.

They were taken with the controller's `shot.sh`, which sets `TOMAKAST_NO_KEYBOARD=1` and `TOMAKAST_DEMO=1`, uses a throwaway home and crops to the window. "Light" means `TOMAKAST_DEV_LIGHT=1` over the dark desktop (see below).

| View | Dark | Light |
|---|---|---|
| Root, empty query | `t6-final-dark-root-empty.png` | `t6-final-light-root-empty.png` |
| Root, typed query (`TOMAKAST_DEV_QUERY=fir`) | `t6-final-dark-root-typed.png` | `t6-final-light-root-typed.png` |
| Action panel (Ctrl+K, `TOMAKAST_DEV_ACTIONS=1`) | `t6-final-dark-actions.png` | `t6-final-light-actions.png` |
| Toast (error) | `t6-final-dark-toast.png` | `t6-final-light-toast.png` |
| Demo list | `t6-final-dark-list.png` | `t6-final-light-list.png` |
| Demo grid | `t6-final-dark-grid.png` | `t6-final-light-grid.png` |
| Demo detail | `t6-final-dark-detail.png` | `t6-final-light-detail.png` |
| Demo form | `t6-final-dark-form.png` | `t6-final-light-form.png` |
| Demo scratchpad | `t6-final-dark-scratchpad.png` | `t6-final-light-scratchpad.png` |

- The `t6-base-light-*` files are the light pass before its fixes. Earlier tasks' shots are the `t1-*` to `t5-*` files and the `anim*/` folders.
- The toast shots used a temporary, uncommitted patch so that a daemon-origin failure (`tomakast run demo/panic`) shows a toast, as in Task 2. Without the patch, a CLI `run` only logs the failure.

### Checklist against the spec (agent, from the screenshots)

| Spec item | Result |
|---|---|
| Blurred backdrop, rounded to the panel; 1 px border; shadow | ✅ (the shadow is faint on a dark desktop, as the spike predicted) |
| One-line rows: title, then the muted subtitle; kind at the right edge | ✅ |
| Selection spans the list width minus the 8 px gutters, radius 8 | ✅ |
| Root sections: Suggestions / Applications / Commands; Results for a typed query | ✅ for "Applications" and "Results" in these shots. A fresh home has no usage, so there is no Suggestions section, and Commands sits below the visible rows. Both rest on Task 3's unit tests and shots. |
| Header: borderless 18 px input; back chip and filter or title in pushed views | ✅ |
| Footer: context or toast on the left; primary `↵`, divider, Actions `Ctrl` `K` on the right | ✅ |
| Action panel: popover above the footer at the bottom right, filter at the bottom | ✅ |
| Grid: radius 8, selection fill, 12 px titles | ✅ |
| Detail, form, scratchpad: 16 px padding; form labels right-aligned in a column | ✅ |
| Same header, rows, labels, footer and chips in root and pushed views | ✅ |
| No open or close animation | ✅ (see the KWin section) |

## Light pass and final polish (Task 6, commit `af8bd82`)

- **Light muted text:** the muted opacity is now a dark/light pair of (0.55, 0.62). With 0.55, light muted text on the panel fill was 3.8:1. It is now 4.8:1, and dark stays at 5.6:1. A unit test keeps both at 4.5:1 or above. The spec gives "around 55%" and leaves the exact value to the builder.
- **Form fields:** they use a translucent tint (white 0.05 / black 0.04) with the panel border, instead of the kit's opaque input fill. In light mode that fill painted stark white boxes.
- **Popover opacity raised from 0.92 to 0.97.** At 0.92 the rows under the popover showed through as ghost text. This is a deviation from the spec, approved by the controller for this pass.
- **Inputs drop the kit's 10 px inner padding**, so text lines up with the elements next to it:
  - the root search text lines up with the section labels, row icons and footer text;
  - the list filter lines up with the pushed views' titles;
  - the popover's filter placeholder lines up with the action titles.
- **Form controls are control-high (32 px):** labels, checkboxes and dropdowns all take that height, so each label sits on the line of its control's text.
- **Scratchpad editor:** it is borderless, like the rest of the panel, and its text lines up with the panel's 16 px edge.

All new values are named constants in `crates/ui/src/theme.rs`.

## Dev-only environment variables

These are for development runs and screenshots, not user settings. They are documented in `crates/ui/src/lib.rs`. A flag counts as set when its value is neither empty nor `0`.

- `TOMAKAST_DEV_QUERY=<text>`: the root search starts with `<text>` on each open.
- `TOMAKAST_DEV_ACTIONS=1`: the action panel opens by itself when a row with actions is selected.
- `TOMAKAST_DEV_LIGHT=1`: forces the light mode whatever the system appearance.

They work alongside the existing `TOMAKAST_HOME`, `TOMAKAST_NO_KEYBOARD` and `TOMAKAST_DEMO`.

## User checklist

These checks need a person. Stop your own tomakast daemon first (for example the one from the `plan-2b1` worktree), or the hotkey may open that older build instead of this one; start it again afterwards. Run the checks from the worktree root; if your shell is fish, run `bash` first. Everything uses a throwaway home, so **no backup is needed**. The keyboard is grabbed normally (`TOMAKAST_NO_KEYBOARD` is unset).

```bash
cargo build --release -p tomakast-app
export TK="$PWD/target/release/tomakast"
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-visual.XXXX)
unset TOMAKAST_NO_KEYBOARD TOMAKAST_DEV_QUERY TOMAKAST_DEV_ACTIONS TOMAKAST_DEV_LIGHT
TOMAKAST_DEMO=1 "$TK" daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!
```

Open the launcher with the hotkey, or with `"$TK" toggle`. When done, run `kill $DAEMON; rm -rf "$TOMAKAST_HOME"`.

| # | Check | How | Expected | Result |
|---|---|---|---|---|
| V1 | Keyboard focus with the `utility` namespace | Press the hotkey and type `fir` at once | The letters land in the search box every time, and the caret is visible at the start of the box before typing | |
| V2 | The open feels instant | Hotkey, then Esc, several times | No scale, fade or slide on open or close | |
| V3 | The look on a real backdrop | Open it over a light window (a browser page), then over a dark one | The blur, the border and the shadow read as a finished panel; the text is crisp | |
| V4 | Light theme on a real light scheme | System Settings → Colors → Breeze Light; restart the daemon; open it | Light panel, readable muted text, visible selection, form fields not stark white | |
| V5 | Ctrl+K | Select a row, press Ctrl+K, type to filter, press Esc | The popover opens at the bottom right above the footer; the popover is opaque, no rows show through it; the ↵ chip is on the selected action | |
| V6 | Back chip | Type `demo`, open "Demo: List", click the `←` chip | Back to the root list | |
| V7 | Toast timing | Type `demo`, run "Demo: Panic" (an error toast) and "Demo: Slow" (a success toast) | The toast shows in the footer; success hides after about 3 s, error after about 5 s | |
| V8 | Scratchpad and form | Open "Demo: Scratchpad" and "Demo: Form" | No heavy editor border; Tab moves through the form fields; labels sit on their control's line | |
