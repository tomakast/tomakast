# Settings window round 5 (polish) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the settings window look finished: one hotkey field look, aligned columns, a
single Raycast-like Hotkeys table, smooth scrolling, smaller Breeze controls, correct hover
colours, whole-line widget drag, theme-correct icons.

**Architecture:** Changes stay inside crates/ui/src/settings (fields, layout, pages, frame),
crates/ui/src/theme.rs and modules/files. Cheap, certain fixes land first; the Hotkeys
single-table restructure is last so it cannot hold the rest hostage.

**Tech Stack:** Rust 2024, GPUI via gpui-kit =0.10.0 (gpui-component 0.10.0, gpui-fast 0.1.17).

**Spec:** docs/superpowers/specs/2026-11-24-settings-window-round5-design.md

## Global Constraints

- Plain user-facing English; no internal names in the UI.
- Every click, key press or drag gets a GPUI test; every layout claim (alignment, width) gets a
  bounds test in a test window.
- GPUI redraws hover only for elements with `.id()`.
- Run `just check` before each task's last commit. No push, no merge, no `git stash`, no edits
  to the justfile, no attribution lines, screenshots never in the repo.

## Review Focus

- A theme switch (dark ↔ light) while the window is open: hover colours and icon tints follow.
- A hotkey field recording, then the page re-sorting or re-rendering: the field must not move.
- A drag started on a line and dropped outside any line: nothing is written, the line is restored.
- Search on the Hotkeys page revealing a closed group in the single table.
- Scrolling the Hotkeys page while a group opens or the index changes: no stall.

---

### Task 1: Small, certain fixes

**Files:** crates/ui/src/theme.rs, crates/ui/src/settings/frame.rs,
crates/ui/src/settings/layout/page.rs (`control_button`), every settings dropdown/control
button builder, modules/files/src/lib.rs, crates/ui/src/settings/window.rs or pages/hotkeys.rs
(open state).

- [ ] Breeze controls: `BREEZE_BUTTON_SIZE = 22.`, `BREEZE_BUTTON_GAP = 6.`, keep
  `BREEZE_GLYPH_SHARE = 0.9`; the right margin equals the gap between the title area's top and
  the buttons' top (buttons centred vertically). Update `breeze_glyphs_match_kde` / frame tests
  to the new values.
- [ ] Hover colour. Cause found: the kit's outline button hover replaces our `palette.field`
  fill with `theme.input` mixed 50% with transparent (gpui-component button.rs
  `outline_background`), which is darker on dark. Build settings control buttons and dropdown
  buttons with the kit's custom variant (`ButtonCustomVariant` with `.hover()` / `.active()`)
  or an equivalent that paints: rest = `palette.field` over the group fill; hover = rest plus
  `palette.hover`; active = rest plus twice `palette.hover` — each as one opaque colour
  composited over `GROUP_FILL` (so translucency cannot read darker). A palette-level unit test:
  in dark, hover and active are lighter than rest (higher lightness); in light, darker. Cover
  every dropdown in the window (size, choice, long choice/time zone/language pickers) and the
  other control buttons (Remove, Reset to default only if it is a filled button).
- [ ] Search Files icon. Cause found: `modules/files/src/lib.rs` gives the "Search Files"
  command and the "Search Files for …" fallback `Icon::Named("system-search")`, which is not a
  bundled (Lucide) name, so it is taken from the desktop's icon theme — light glyphs under a
  dark desktop theme, invisible on the light app theme. Use the bundled Lucide `search` icon
  (tinted with the theme's text colour). Update the module's tests. Grep the other modules for
  `Icon::Named` names that are desktop symbolic icons (monochrome) and switch those to bundled
  names too; leave coloured mime/app icons alone.
- [ ] Applications is open the first time the Hotkeys page is shown, whatever it holds; other
  groups keep their rule. Test: a fresh window with no hotkeys shows Applications' rows.
- [ ] `just check`, commit.

### Task 2: The hotkey field and column alignment

**Files:** crates/ui/src/settings/fields.rs (`HotkeyField`), crates/ui/src/settings/layout/page.rs
(`column_titles`), crates/ui/src/theme.rs (`HOTKEY_FIELD_WIDTH`, related), tests.

- [ ] The field is always drawn as the window's text-field box: same fill, border colour,
  radius, height (`CONTROL_HEIGHT`) and inner left padding as the settings text fields. This
  reverses round 4's "bare" empty state (no fill, no border): the user asked for the outline.
  Empty: "Add hotkey", muted, the text fields' placeholder size. Recording: "Press keys…" in
  the same style, plus today's ring. With a hotkey: chips. All content left-aligned at the
  inner padding.
- [ ] The status icon is no longer a reserved left slot: when present it sits on the right,
  before the ×; when absent it takes no room. The × shows as today.
- [ ] Width: fixed, the smallest that fits the longest accepted combination (four modifiers and
  a key, e.g. "Ctrl Alt Shift Super Space" as chips) plus the status icon and ×. Measure it in a
  test window and set the constant from the measurement (record the numbers in the report). It
  replaces 300.
- [ ] Column titles: "Hotkey" starts exactly at the hotkey fields' text start (field left +
  inner padding); "Name" starts at the rows' icon left. Bounds tests on the Hotkeys page and a
  plugin Commands table.
- [ ] Tests: field box equal size empty / with hotkey / recording; text start equal in all three;
  General's launcher hotkey field has no left gap when it has no status icon.
- [ ] `just check`, commit.

### Task 3: A widget is dragged whole

**Files:** crates/ui/src/settings/pages/home.rs, tests.

- [ ] The whole line starts the drag (mouse down anywhere outside the size dropdown and the
  remove button, then move). The drag image is the full line: same width as the line, same
  content, a shadow, opacity 0.9. The source line stays in place at `DISABLED_OPACITY` until
  the drop or cancel. The grip stays. Drop handling and one write per drop unchanged.
- [ ] Tests: a drag started on the title moves the widget (write checked); a mouse down on the
  size dropdown or remove button does not start a drag; the drag preview's width equals the
  line's width (bounds); a drag dropped outside any line writes nothing.
- [ ] `just check`, commit.

### Task 4: Smooth scrolling of the Hotkeys page

**Files:** crates/ui/src/settings/window.rs, pages/hotkeys.rs, pages/mod.rs, layout/*.

- [ ] Measure first: time one render of the window with the Hotkeys page shown, Applications
  open, 130 apps indexed (a test with a fake index, or the dev build with timing logs), in the
  dev profile (it is optimized). Record where the time goes (model build, sorting, search text,
  `pages::build` for all pages, element construction, icon loading).
- [ ] Remove the per-frame work the measurement shows: build the hotkeys model and pages only
  when their inputs change (index generation, file, hotkey table, query, open groups, theme),
  cache sorted item lists, decode app icons once. Do not rebuild every page when one is shown.
  If element construction of 100+ rows still dominates, render only the rows in view.
- [ ] A test or bench that fails if a re-render with unchanged inputs rebuilds the model
  (e.g. a counter of model builds). Report before/after frame times.
- [ ] `just check`, commit.

### Task 5: The Hotkeys page is one table

**Files:** crates/ui/src/settings/layout/page.rs (table group rendering), pages/hotkeys.rs,
pages/plugins.rs (Commands), tests.

- [ ] One box for the whole page: column titles once at the top; per source a group row
  (chevron, source icon, name semibold, count muted; no fill of its own, hover tint;
  a little taller than item rows); its items indented so an item's icon lines up with the
  group's title text; item titles regular weight in the text colour; hairline dividers as in
  other boxes. No per-group boxes, no repeated column titles. "Not used" is the last group of
  the same table. A plugin page's Commands table: the same table, no group rows.
- [ ] Unchanged behaviour: open/closed rule, Applications open first time, count, search
  revealing closed groups, Enter/Space on a group row, lazy fields, bound items first, frozen
  order while a field is in use, greyed rows. Existing tests keep passing (update only their
  structure-specific expectations).
- [ ] Tests: one column-title row on the page; an item icon's left equals its group title's
  left (bounds); group rows have no fill at rest.
- [ ] `just check`, commit.

### Task 6: Polish pass

- [ ] Capture every page in dark and light (fresh window per page; drop any capture that is not
  the settings window's size, compare hashes so no two pages are byte-identical by accident).
- [ ] Fix every misalignment, uneven spacing or inconsistent control seen, in one commit with
  tests where a layout claim is made.
