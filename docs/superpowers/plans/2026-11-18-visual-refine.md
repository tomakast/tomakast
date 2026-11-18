# Launcher visual refinement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Raycast-inspired look for the launcher: blurred translucent panel with a shadow, one-line rows with the kind on the right, full-width selection, root sections, a footer with key hints and toasts, a popover action panel, consistent pushed views; no animation.

**Architecture:** All sizes and colors move into design tokens in `crates/ui/src/theme.rs`. Shared pieces (`views::row`, `views::section_label`, `chrome::header`, `chrome::footer`, `chrome::key_chip`) are used by the root and every pushed view. Root sections come from a pure function, unit-tested. The window gets the blur and the shadow margin through gpui's `set_client_inset` and `set_client_corner_radii`.

**Tech Stack:** Rust 2024, gpui-kit 0.10.0 (gpui-pre 0.3.12), KDE Plasma 6.8 Wayland (`ext-background-effect`).

**Spec:** `docs/superpowers/specs/2026-11-18-launcher-visual-refine-design.md` (parent: `2026-11-18-tomakast-core-design.md`).

## Global Constraints

- All values (sizes, opacities, colors, TTLs) are the spec's; they live only in `theme.rs` as named constants.
- No open or close animation anywhere; warm-open median stays within +2 ms of 20.7 ms (plan 2b-1 protocol).
- Agents never show a window that grabs the keyboard: dev runs and screenshots use `TOMAKAST_NO_KEYBOARD=1` and a throwaway `TOMAKAST_HOME`.
- Element ids and accessibility roles that existing tests use (`launcher-input`, `launcher-toast`, `view-back`, `view-header`, `ListItem` rows, …) are kept.
- The shadow margin is used only on Linux/Wayland; it is 0 elsewhere.
- Docs in English.

## Review Focus

- Arrow keys and Enter with section labels present: selection never lands on a label; Enter on the first row after a label works; "fir⏎" still launches the first real row.
- A very long title plus subtitle plus accessory: the subtitle truncates first, the title second, the accessory never wraps or is pushed out.
- Light theme: every token has a light value; text and selection stay readable on a light, blurred backdrop.
- A toast arriving while the action panel is open, and a toast replacing another: the footer shows the newest toast and returns to the context after the TTL.
- Pushed list view with sections and a detail pane: the same row and label styling as the root, the highlight spans the list column, not the detail pane.

---

### Task 1: Tokens and window chrome

**Files:**
- Modify: `crates/ui/src/theme.rs`, `crates/ui/src/window.rs`, `crates/ui/src/launcher_view.rs` (root `render` only)

**Interfaces:**
- Produces:
  - `theme::Palette`, gaining the fields `panel`, `border`, `divider`, `text`, `muted`, `selected`, `chip`, `popover`, `shadow: [BoxShadow; 2]`, `error` and `success`;
  - constants `PANEL_SIZE (750., 474.)`, `PANEL_RADIUS 12.`, `SHADOW_MARGIN` (24 on Linux, 0 elsewhere), `HEADER_HEIGHT 56.`, `FOOTER_HEIGHT 40.`, `ROW_HEIGHT 40.`, `ICON_SIZE 20.`, `SECTION_HEIGHT 28.`, `ROW_GUTTER 8.`, `ROW_RADIUS 8.`.

- [ ] **Step 1:** Unit test `palette_has_light_and_dark_values`: `palette_for(dark: bool)` returns distinct `panel`, `text` and `selected` values for each mode, and `panel.a` equals the spec opacity (0.78 dark, 0.80 light).
- [ ] **Step 2:** Implement the tokens. `palette(cx)` delegates to `palette_for(cx.theme().mode.is_dark())`, and error and success come from the kit theme.
- [ ] **Step 3:** Set up the window:
  - `window_background: Blurred`;
  - request `PANEL_SIZE`, not panel plus margin, because gpui adds the inset itself;
  - in `open`, call `set_client_inset(px(SHADOW_MARGIN))` and `set_client_corner_radii(Corners::all(px(PANEL_RADIUS)))`;
  - the root `render` gets an outer transparent div with padding `SHADOW_MARGIN`, and the panel inside carries the fill, border, radius and `palette.shadow`.
- [ ] **Step 4:** Check for animation:
  - check whether KWin animates our layer surface on map and unmap: `qdbus org.kde.KWin /Effects org.kde.kwin.Effects.activeEffects` lists the loaded effects;
  - take 3 screenshots at 0, 40 and 80 ms after `toggle` and compare the panel alpha between them;
  - if KWin animates, find a per-surface opt-out (layer namespace, window type) and apply it. Otherwise record the KWin setting in the experiment note.
- [ ] **Step 5:** Verify: take a screenshot (dark), `cargo test -p tomakast-ui`, then clippy and fmt. Commit `feat(ui): blurred panel with shadow and design tokens`.

### Task 2: Header, footer and key chips

**Files:**
- Create: `crates/ui/src/chrome.rs`
- Modify: `crates/ui/src/launcher_view.rs`, `crates/ui/src/toast.rs`, `crates/ui/src/views/action_panel.rs` (remove the old `footer`)

**Interfaces:**
- Produces:
  - `chrome::key_chip(label: &str, palette: &Palette) -> AnyElement`;
  - `chrome::header(back: bool, content: AnyElement, cx) -> AnyElement`, with the divider below;
  - `chrome::footer(left: FooterLeft, primary: Option<&str>, palette) -> AnyElement`;
  - `enum FooterLeft { Context(String), Toast(Entity<ToastBar>) }`;
  - `ToastBar::is_showing(&self) -> bool`;
  - `fn shortcut_label() -> [&'static str; 2]`, which gives `["Ctrl","K"]`, or `["⌘","K"]` on macOS.

- [ ] **Step 1:** Unit tests:
  - `footer_left_prefers_toast`: a pure `footer_left(context, toast_showing) -> FooterLeft` returns `Toast` while one is showing, `Context` otherwise;
  - `shortcut_label_per_platform` (cfg-gated).
- [ ] **Step 2:** Implement the header. The search input gets no border and no background, 18 px text, and the back chip when the stack is not empty.
- [ ] **Step 3:** Implement the footer:
  - the context reads "tomakast" at the root and the view title in pushed views;
  - toasts render in the footer's left slot with a colored dot, not as a separate bar;
  - on the right: the primary action title + `↵` chip, a thin vertical divider, then "Actions" + chips.
- [ ] **Step 4:** Verify: take screenshots of the root and of a toast (Demo: Form submit cannot be typed without the keyboard, so trigger Demo: Panic through `tomakast run demo/panic`). Run the tests, clippy and fmt. Commit `feat(ui): header and footer with key hints; toasts in the footer`.

### Task 3: Rows and root sections

**Files:**
- Create: `crates/ui/src/root_sections.rs`
- Modify: `crates/ui/src/views/mod.rs` (`row`, new `section_label`), `crates/ui/src/launcher_view.rs` (root list, selection), `crates/ui/src/views/list.rs`

**Interfaces:**
- Consumes: the Task 1 tokens.
- Produces:
  - `enum RootRow { Label(&'static str), Hit(usize) }`;
  - `fn root_rows(hits: &[(ItemKind, &str /*title*/, f64 /*score*/)], query_empty: bool) -> Vec<RootRow>`;
  - `fn kind_label(kind: ItemKind) -> &'static str`;
  - `views::section_label(text: &str, palette) -> AnyElement`.

- [ ] **Step 1:** Unit tests in `root_sections.rs`:
  - `typed_query_is_one_results_section` → `[Label("Results"), Hit(0), Hit(1), …]` in hit order;
  - `empty_query_suggestions_then_kinds`: hits with scores `[3.0, 1.0, 0, 0]` and kinds `[App, Cmd, App, Cmd]` → `Suggestions: 0, 1`, `Applications: 2`, `Commands: 3`;
  - `suggestions_capped_at_8`: 10 hits with usage → 8 under Suggestions, the other 2 go under their kind;
  - `no_item_twice`;
  - `empty_sections_omitted`;
  - `kind_labels` → "Application" and "Command".
- [ ] **Step 2:** Implement `root_rows`. Inside "Applications" and "Commands", order by lowercase title.
- [ ] **Step 3:** Rewrite `row`:
  - one line: icon, title, then the subtitle in the muted color 8 px after it, then accessories pushed to the right edge (`ml_auto`, `flex_none`);
  - the title is `flex_shrink` with a lower shrink than the subtitle, so the subtitle truncates first;
  - the row is `w_full`, and its highlight is inset by `ROW_GUTTER` on each side.
- [ ] **Step 4:** Build the root `uniform_list` from `root_rows`. A label renders with `section_label`, and a hit renders as a row whose accessory is `Accessory::Text(kind_label)`. `selected` keeps indexing into `hits`, while the list index maps through `root_rows`. Arrow keys move between hits only; the existing selection tests keep passing.
- [ ] **Step 5:** In pushed list views, `list.rs` renders sections with `section_label` and rows with the same `row`.
- [ ] **Step 6:** Verify: take screenshots of the empty root, the root with "demo" typed (via `tomakast run`? There is no query IPC, so add the dev-only env `TOMAKAST_DEV_QUERY` that pre-fills the root query on show, read in `LauncherView::new`, documented as dev-only), and Demo: List. Run the tests, clippy and fmt. Commit `feat(ui): one-line rows with kind accessory, root sections, full-width selection`.

### Task 4: Action panel popover

**Files:**
- Modify: `crates/ui/src/views/action_panel.rs`

- [ ] **Step 1:** Restyle the panel:
  - absolute, anchored at bottom-right 8 px above the footer, 320 px wide;
  - `palette.popover` fill, a border, the shadow, radius 10;
  - 32 px rows that use the selection fill;
  - the filter input at the bottom of the popover with a divider above it;
  - when an action has a shortcut, it renders on the right as key chips.
- [ ] **Step 2:** Make sure the existing action-panel tests still pass (filtering, Enter, Esc).
- [ ] **Step 3:** Verify: open the panel for the screenshot through `TOMAKAST_DEV_ACTIONS=1`, a dev-only env that opens the panel on show, documented next to `TOMAKAST_DEV_QUERY`. Commit `feat(ui): action panel as a popover`.

### Task 5: Pushed views consistency

**Files:**
- Modify: `crates/ui/src/views/grid.rs`, `detail.rs`, `form.rs`, `scratchpad.rs`, `crates/ui/src/launcher_view.rs` (`render_header` → `chrome::header`)

- [ ] **Step 1:** Header in pushed views. List and grid views get the back chip plus their filter input in the header slot; detail, form and scratchpad views get the back chip plus the title.
- [ ] **Step 2:** Restyle the views to the spec:
  - grid: radius 8, a selection fill instead of a border, 12 px titles;
  - detail, form and scratchpad: 16 px padding and body text 14 / muted 13;
  - form: labels right-aligned in a left column.
- [ ] **Step 3:** Verify: take a screenshot of each demo view, opened through `TOMAKAST_DEV_RUN=<item id>`, a dev-only env that runs an item on show. Then the tests, clippy and fmt. Commit `feat(ui): consistent pushed views`.

### Task 6: Light theme, acceptance note, latency

**Files:**
- Create: `docs/experiments/2026-11-visual-refine.md`

- [ ] **Step 1:** Light pass: take screenshots under a light Plasma color scheme. This needs the user, so the agent fakes it by forcing `palette_for(false)` through the dev-only `TOMAKAST_DEV_LIGHT=1`. Fix the contrast issues it shows.
- [ ] **Step 2:** Re-measure the warm open with the plan 2b-1 protocol (500 toggles, `TOMAKAST_NO_KEYBOARD=1`). The median must stay ≤ 22.7 ms. Record the result together with RSS.
- [ ] **Step 3:** Write the experiment note:
  - the screenshots, with the paths of the images saved under `docs/experiments/img/visual-refine/`;
  - the KWin animation finding;
  - a user checklist covering the look on the real backdrop, the hotkey open feeling instant, and the light theme on a real light scheme.

  Commit `docs(experiments): visual refinement acceptance`.
