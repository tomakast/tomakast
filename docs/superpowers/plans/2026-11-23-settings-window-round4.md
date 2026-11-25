# Settings window round 4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The settings window behaves like a native settings app: focus, sections, a Raycast-like
hotkeys table, a steady hotkey field, searchable pickers with real defaults, a draggable
Widgets list, KDE-like window controls.

**Architecture:** All in `crates/ui/src/settings/` except point 3/4 (Task 5), which adds a value
kind and a WIT effect case and touches the calc and translator plugins. Shared pieces first
(hotkey field, layout model), then pages.

**Tech Stack:** Rust 2024, GPUI via gpui-kit =0.10.0, wasmtime plugins (wasm32-wasip2), TOML.

**Spec:** `docs/superpowers/specs/2026-11-23-settings-window-round4-design.md` (points 1–11 are
referred to by number).

## Global Constraints

- User-visible strings in plain English; no internal names; never "tile", never "Not set" for a
  hotkey.
- Run things with `just` from the worktree (`just check` is the gate: build, clippy, tests).
- To look at the window: the built binary with `TOMAKAST_NO_KEYBOARD=1` and a throwaway home
  under `/run/user/1000/`; screenshots with `spectacle -b -n -a -o <file>` into the session
  scratchpad, never into the repo. Remove the throwaway home and stop every daemon you started.
  Never kill a process you did not start.
- No push, no merge, no `git stash`, no attribution lines in commits. Do not edit the justfile.
- Each click, key press or drag the task adds gets a GPUI test that simulates it
  (`crates/ui/tests/settings.rs` and the tests in `fields.rs` show how).
- Docs and code comments in English.

## Review Focus

1. A click on a field must still focus that field (and a click on a dropdown open it) after the
   root takes clicks on empty space — test both.
2. Recording a hotkey and blurring by a click elsewhere must end recording exactly as before
   (keep a shown combination, or cancel when none) — test it.
3. Settings search for an application title must open the closed Applications group and show
   the row — test it.
4. A hand-written value outside a list (`timezone = "Moscow"`) shows as written, not as the
   default — test it.
5. Dragging a widget onto its own place writes nothing; a drop on another row writes one move —
   test both.

---

### Task 1: Focus, sidebar weight, one "tomakast" section, Widgets name, launcher hotkey on General only

Points 1, 2, 6, 7, the rename part of 11.

**Files:** `crates/ui/src/settings/window.rs`, `layout/sidebar.rs`, `layout/mod.rs` (`Section`),
`pages/mod.rs` (`page_keys` order), `pages/home.rs` (title, section), `pages/hotkeys.rs` (drop the
launcher row), `pages/general.rs` (doc comment), `crates/ui/tests/settings.rs`.

- [ ] Tests first (`crates/ui/tests/settings.rs`):
  - `clicking_empty_space_blurs_a_text_field`: focus a text field (type into it), simulate a
    click on the page area outside any control, assert the field is no longer focused and the
    typed value was committed (as on blur).
  - `clicking_a_field_still_focuses_it`: after the change, a click on a text field focuses it.
  - `a_recording_hotkey_field_ends_on_a_click_elsewhere`: start recording, press a combination,
    click empty space, assert the combination was committed (and, with nothing pressed, that
    recording was cancelled with nothing written).
  - `the_selected_sidebar_entry_is_not_bold`: if the test API cannot read font weight, assert
    instead in a unit test of `sidebar.rs` that no `font_weight` is applied (or drop this test
    and say so in the report).
  - `builtin_pages_are_one_section`: the sidebar shows the section label "tomakast" once, no
    "Core", no "Modules", and the entries in order General, Appearance, Location, Hotkeys,
    Widgets, About, then the "Plugins" label.
  - `the_launcher_hotkey_is_only_on_general`: the Hotkeys page has no row titled as the launcher
    hotkey key's title; General has it.
- [ ] `Section`: replace `Core` and `Modules` with one variant `Launcher` labelled `"tomakast"`.
- [ ] Page order in `page_keys`: General, Appearance, Location, Hotkeys, Home (shown "Widgets"),
  About. The `SettingsPage::Home` key and the page id the launcher's "Edit Home…" opens stay as
  they are; only what is shown changes: title "Widgets", a fitting Lucide icon that is bundled
  (`layout-grid` or another already in `assets/icons`; check before using).
- [ ] Root focus: the settings window's root element tracks a focus handle of the window (reuse
  one if the window already has one). Verify in the gpui source (`Interactivity`, mouse down)
  that the innermost focusable element wins and ancestors are skipped; cite the file and line in
  the report.
- [ ] Remove `FontWeight::MEDIUM` from the selected sidebar entry; grep the settings code for any
  other selected-state weight.
- [ ] `just check` passes; commit `settings: a click on empty space takes the focus; one "tomakast" section; Widgets; the launcher hotkey on General only`.

### Task 2: The hotkey field that never moves anything

Point 9 and the empty state of point 5.

**Files:** `crates/ui/src/settings/fields.rs` (`HotkeyField`), `pages/mod.rs` (`hotkey_state`,
`KeyState`, `key_control` for hotkey rows), `pages/general.rs` (launcher hotkey row),
`pages/plugins.rs` (command rows), `pages/hotkeys.rs` (item rows), `crates/ui/src/theme.rs`
(width constant).

**Interfaces produced:** `HotkeyField` keeps its events (`HotkeyCommit`) and adds a clear request
(an event the window handles exactly as the old reset did: `request_hotkey(target, None)`); it
takes its stand state (after restart / off / refused text) through a setter instead of rows
drawing lines under it. Task 3 places this field in table cells.

- [ ] Tests first (in `fields.rs` tests or `crates/ui/tests/settings.rs`):
  - `the_hotkey_field_keeps_its_size`: measure the field's bounds empty, recording with nothing
    pressed, recording with the longest combination the recorder allows
    (Ctrl+Alt+Shift+Super+PageDown or the longest key name in `crate::shortcut`), and refused;
    all four equal.
  - `an_empty_hotkey_field_reads_add_hotkey`: aria value "Add hotkey"; recording with nothing
    pressed: "Press keys…".
  - `the_row_keeps_its_height_while_recording`: the row's bounds do not change between idle and
    recording, nor when a refusal shows.
  - `clear_removes_the_hotkey`: a field with a hotkey shows a clear button (accessible name
    "Clear hotkey of <title>"); clicking it asks for the hotkey to be removed (same request the
    reset button made); it is absent while empty and while recording.
  - `a_hotkey_state_is_a_tooltip_icon`: "Applies after a restart" is the accessible description
    of the field (findable by the test API), and no line with that text sits under the field.
- [ ] Field: fixed `w` (a theme constant sized for the longest combination) and fixed height;
  content swapped in place; chips clipped, never wrapping. Hint ("↵ Keep · Esc Cancel") and the
  refusal drawn in an overlay anchored under the field (`deferred(anchored(...))` as the kit's
  popovers do), not in flow. Clear (×) inside the field's right edge. State icon inside the
  field's left edge with a tooltip.
- [ ] Hotkey rows (launcher hotkey, commands, items) stop using the reset button and the lines
  under the control for hotkeys; `KeyState` stays for other keys.
- [ ] Screenshots (throwaway hook or test-only path that calls `start` and `recorded_press`):
  empty, recording, longest combination, refused — dark theme, into the scratchpad; list the
  paths in the report.
- [ ] `just check`; commit `settings: the hotkey field keeps its size; Add hotkey, clear inside it, hint and refusal over the row`.

### Task 3: Collapsible groups with icons and columns; Hotkeys as a table; plugin Commands as the same table

Points 5 and 8.

**Files:** `crates/ui/src/settings/layout/{mod.rs,page.rs,search.rs}`, `pages/hotkeys.rs`
(rewrite), `pages/plugins.rs` (Commands group), `settings/window.rs` (open/closed state per
group, lazy hotkey fields, remove `added`, `add_hotkey`, `ItemSearch`), `crates/core` only if no
per-provider way to list indexed items exists (look in `index.rs`/`core.rs` first: `iter`,
`ItemId::provider`), `crates/ui/tests/settings.rs`.

**Interfaces:** `Group` gains `icon: Option<PageIcon>`, `columns: Option<(String, String)>`
(left and right header), and `collapsible: Option<GroupKey>` where the window keeps which keys
are open; search keeps working on rows of closed groups (their row descriptors exist; only
their drawing and their fields are lazy).

- [ ] Tests first:
  - `hotkeys_lists_items_by_source`: with a fake core holding two app items and a plugin
    command, the Hotkeys page shows group headers "Applications" and the plugin's name, a
    column header "Name"/"Hotkey", and no "Add hotkey…" control.
  - `a_group_with_a_hotkey_starts_open`: the group of an item with a hotkey is open (its row
    visible); a group without one is closed (its rows not drawn) and opens on a click on its
    header, closes on a second click.
  - `search_opens_a_closed_group`: typing an application title in the window's search shows
    that application's row.
  - `an_unindexed_item_with_a_hotkey_is_greyed_and_removable`, and the existing dropped-entry
    tests keep passing (move them to the "Not used" group).
  - `plugin_commands_are_a_hotkey_table`: a plugin page shows "Commands" with the column header
    and each command's field reading "Add hotkey".
- [ ] Group header: chevron, source icon (apps: a bundled icon such as `app-window`; a module:
  its icon; a plugin: as its sidebar entry), name, count of items in muted text; the whole
  header is a button with an accessible name "<name>, N items" and expanded state.
- [ ] Rows: item icon (16 px), title, hotkey field in the right column, aligned under the
  "Hotkey" header.
- [ ] Hotkey fields for a group's items are created when the group opens (and kept while the
  window lives); none for closed groups.
- [ ] Delete the "Add hotkey…" select, `view.added`, `ItemSearch`, and their tests.
- [ ] `just check`; commit `settings: Hotkeys as a table of items by source, in groups that open and close; plugin commands as the same table`.

### Task 4: KDE-like window controls

Point 10.

**Files:** `crates/ui/src/settings/frame.rs`, `crates/ui/src/theme.rs`, `assets/icons` (a glyph
only if Breeze's shape has no bundled icon; draw it as an SVG of the same stroke style).

- [ ] Take a screenshot of a real Breeze title bar on this machine (`spectacle -b -n -a -o` of
  an existing window with server decorations, e.g. the active window) into the scratchpad;
  measure button diameter, glyph size and stroke, spacing, right margin, and hover look.
- [ ] Breeze style: button size and gap from the measurement scaled to the 48 px title area
  (state the numbers in the theme constants' docs); minimize a down chevron, maximize an up
  chevron, restore a diamond, close an ×; hover fill as Breeze (close red with a white glyph).
  The existing frame tests keep passing; add `breeze_glyphs_match_kde` asserting the glyph per
  button and state.
- [ ] Screenshot our title bar next to the native one; list the paths in the report.
- [ ] `just check`; commit `settings: window controls drawn as KDE's Breeze draws them`.

### Task 5: Searchable pickers with real defaults: time zone, translator languages

Points 3 and 4.

**Files:** `crates/core/src/settings_schema.rs` (`ValueKind::TimeZone`, manifest `kind =
"timezone"`), the manifest parser for preferences (find where `kind = "choice"` is parsed),
`crates/plugin-api/wit/since_v0.1.0/plugin.wit` (effect case
`preference-default(tuple<string, string>)` or a record `{key, value}`) and `FROZEN.md`,
`crates/core/tests/wit_parity.rs`, the plugin host's effect conversion and `caps.rs` (allowed
for plugins, only for the plugin's own declared keys; a key it does not declare is dropped with
a warning), the catalog (`PluginCatalog`/`CatalogPlugin`: the reported defaults), the plugin SDK
if it wraps effects, `crates/ui/src/settings/pages/plugins.rs` and `fields.rs` (a searchable
pick field generalising `CityField`), `plugins/calc/{plugin.toml,src/lib.rs}`,
`plugins/translator/{plugin.toml,src/*.rs,tests}`, `crates/ui/Cargo.toml` (jiff with its bundled
tz database, if `Cargo.lock` has nothing that lists zones).

- [ ] Tests first:
  - core: `timezone_kind_parses` (`kind = "timezone"` → `ValueKind::TimeZone`), and
    `value_fits` accepts any string for it.
  - host: `preference_default_is_kept_for_declared_keys` and
    `preference_default_for_an_undeclared_key_is_dropped`.
  - translator: `the_reported_defaults_follow_automatic` — for system languages `["en","ru"]`,
    `["de"]`, `["en"]`, `[]` the effects sent from `init` equal `pair::automatic` (first always;
    second only when known); `manifest_languages_match_langs` — the `options` of `first` and
    `second` in `plugin.toml` are exactly `langs.rs` in order, value = code, title = English
    name.
  - ui: `a_long_choice_is_searchable` (more than 10 options → typing filters the list);
    `an_unset_timezone_shows_the_computers_zone` (with an injected zone source);
    `a_hand_written_zone_shows_as_written` (`timezone = "Moscow"`);
    `an_unset_language_shows_the_plugins_default` (catalog default `ru` → "Russian");
    `an_unknown_default_shows_automatic`.
- [ ] Calc: `timezone` becomes `kind = "timezone"`, title "Time zone", no description; its
  behaviour on an empty value is unchanged.
- [ ] Translator: `languages` replaced by `first` ("Primary language") and `second` ("Secondary
  language"), `kind = "choice"`, options from `langs.rs`; the plugin reads `first`/`second`
  (either missing → its automatic value for that one; equal → as today when the pair is not
  usable); `init` sends `preference-default` for `first` and, when known, `second`. No
  migration of `languages`.
- [ ] Pick field: one searchable select for city, time zone and long choices (no hint text, no
  description about "empty"); a pick writes at once; picking the shown default while the file
  has none writes nothing.
- [ ] `just check`, and the plugins' own tests (`just` recipe that builds/tests plugins; find it
  in the justfile); commit `settings: searchable time zone and language pickers that show what an unset value stands for; translator: primary and secondary language`.

### Task 6: Widgets as a draggable list

Point 11.

**Files:** `crates/ui/src/settings/pages/home.rs` (rewrite of the rows), `settings/window.rs`
(drop the param fields `home_params` if nothing else uses them), every user-visible string or
accessible name with "tile" (`grep -rni tile crates/ui/src modules crates/core/src` and keep only
code identifiers), `crates/ui/tests/settings.rs`.

- [ ] Tests first:
  - `widgets_are_one_line_each`: a row shows the widget's title, the plugin's name and the
    params summary (`Calculator · 2+2` style: plugin name, then param values joined with ` · `),
    a size dropdown and a remove button; no up/down buttons; no param fields.
  - `dragging_a_widget_moves_it`: simulate a drag of row 0 onto row 2: one write, the file's
    order is 1, 2, 0 (or as `HomeEdit::Move { from: 0, to: 2 }` defines it).
  - `dropping_a_widget_on_itself_writes_nothing`.
  - `an_unreadable_entry_is_not_draggable` and the unreadable index mapping holds.
  - `no_tile_in_accessible_names`: no accessible name on the page contains "tile".
- [ ] Row: drag handle (`grip-vertical` if bundled), plugin icon as the sidebar derives it
  (`PageIcon::Item` logic: first command icon, else monogram), title over subtitle, size
  dropdown, remove (icon button with accessible name "Remove <title> (widget N)"); a greyed row
  for a widget no installed plugin offers (draggable, removable). Drop target shows an insertion
  line.
- [ ] Note above the list, plain: "Add widgets from the launcher: choose Add to Home on a
  result." (match the launcher's real action name).
- [ ] `just check`; commit `settings: Widgets, a list to reorder by dragging, one line per widget; no more tiles`.
