# Visual refinement: follow-ups

Items left after the visual refinement merge (30f4020). They come from the final whole-branch review and the task reviews. The user decided to come back to them later (2026-11-18). None of them blocks other work.

## UX

- **Action panel: keyboard selection is not scrolled into view.** With more than about 8 actions, Down moves the highlight out of sight (`crates/ui/src/views/action_panel.rs`; the list only scrolls by mouse).
- **Form dropdown trigger has no field tint.** "Green ▾" reads as plain text between tinted fields (`crates/ui/src/views/form.rs`).
- **A long primary action title is not truncated.** The right side of the footer is `flex_none`, so a long title squeezes the context and the toast to nothing (`crates/ui/src/chrome.rs`).
- **No "No results" line** for a typed query with no matches; the body is empty.
- **No footer hint in form, scratchpad and bare views**, for example "Submit Ctrl+↵".
- **Root and list rows have no hover or click behaviour.**
- **Clicks in the 24 px transparent shadow margin are taken by the launcher.** They do not reach the window below and do not dismiss the launcher.
- **Per-action shortcuts:** `Action` in the core has no shortcut field, so the action panel only shows `↵` on the selected row. Add the field (it touches WIT; decide before the WIT 0.1 freeze in plan 2b-2) and render the shortcuts as key chips.

## Code

- **Values still outside `theme.rs`:** check for any literal sizes left after the final fix wave and move them into tokens.
- **Some values copy kit metrics.** `EDITOR_TEXT_INSET` and `CONTROL_HEIGHT` duplicate gpui-component internals and will drift silently when the kit changes. Note this next to the `=0.10.0` pin.
- **`shadow_margin()` reads `WAYLAND_DISPLAY` on every render.** Cache it in a `OnceLock`.
- **Repeated allocations per frame:** `theme::palette(cx)` is built several times per frame and `palette.shadow.to_vec()` allocates. Small; revisit if render shows up in profiles.
- **Dev envs and tests:** the dev envs (`TOMAKAST_DEV_QUERY`, `TOMAKAST_DEV_ACTIONS`, `TOMAKAST_DEV_LIGHT`) are honoured in release builds, and the UI tests read them from the process environment, so a shell with one exported would fail the tests. Isolate them in the tests.

## Tests (Review Focus gaps)

No automated test exists for:

- a toast arriving while the action panel is open;
- subtitle-first truncation;
- the up-reveal scroll in the root list;
- the highlight staying inside the list column next to a detail pane;
- a long toast message.

## Platform

- **A Wayland compositor without `ext-background-effect`:** the panel fill (0.78/0.80) has nothing blurred behind it; check readability.
- **The `utility` layer namespace:** Hyprland and sway layerrules cannot target `tomakast-launcher`. Revisit with a KDE-only gate if someone needs a rule.
- **macOS and Windows** (plans 2a and 3): `Blurred` covers the whole window there and the platform draws the shadow, so the margin is 0. Verify on the real machines.

## User checklist

V1–V8 in `docs/experiments/2026-11-visual-refine.md` are not filled in yet.
