# Settings window: acceptance

Date: 2026-11-22. Branch `claude/settings-gui-3345c2`. Spec:
`docs/superpowers/specs/2026-11-22-settings-window-design.md`; plan:
`docs/superpowers/plans/2026-11-22-settings-window.md`.

## What was built

- **A separate settings window** (kit window with the kit Root and its `Settings` layout:
  sidebar with search, pages, items with a ↺ reset that removes the key from the file).
  Pages: General (launcher hotkey, start at login), Location (city from the built-in list,
  "Automatic" first; look up location by Wi-Fi), Hotkeys, Home, one page per installed plugin
  (by name), then a greyed "<id> (not installed)" page per `[plugins.x]` table with no plugin.
- **One description of the built-in keys** (`crates/core/src/settings_schema.rs`): the parser
  reads `config.toml` only through it and the three collections (`[hotkeys]`,
  `[[home.widgets]]`, `[plugins.<id>]`), and the window is built from it. A test fails when a
  key reaches `Settings` without being in the schema.
- **Every write changes one key** in the file's current text and keeps comments, order and
  blank lines (`settings_edit::set_key`, `set_plugin_preference`, `remove_plugin_table`,
  `home_file` edits including the new `SetParam` and `ClearSize`). A file that does not parse
  is never rewritten; a write that would produce one is refused.
- **Everything the file holds has a place:** unknown keys under "Not recognized" with a Remove
  button; wrong-typed values in an error state ("Not valid here: …; the default is used");
  undeclared plugin keys and tile params read-only with Remove; tiles of uninstalled plugins
  greyed, movable and removable; `[hotkeys]` entries the launcher drops greyed with the reason.
- **Plugin preference kinds:** `choice` (dropdown), `number` (integer or float, min/max/step)
  and an optional `description`, in the manifest (WIT 0.1 edited in place; `manifest.md`,
  `FROZEN.md` and the freeze test updated). Weather moved to three choices: temperature
  (°C/°F), wind (km/h, m/s, mph, knots, passed to Open-Meteo as `wind_speed_unit`) and pressure
  (hPa/mm Hg).
- **Hotkeys:** a Hotkeys tab (launcher first, every item hotkey, "Add hotkey…" over the indexed
  items, states "Applies after a restart" / "Refused: …" / "Off"), plus a hotkey field per
  command on each plugin page. Every hotkey, the launcher's included, is bound first and
  written only once the system accepts it; refusal notices show under the field that asked.
- **Outside edits:** the window follows `config.toml` through the daemon's existing watch. With
  nothing typed, it updates without a question. When a typed, unwritten value's key changed in
  the file, a question asks "Keep mine" / "Use the file's" (Esc: decide later, the typed text
  stays). A broken file shows a banner and disables the controls until it is fixed.
- **Ways in:** the "Settings" item in search, Ctrl+, (Cmd+, on macOS) in the launcher,
  "Plugin Settings" in Ctrl+K on a plugin's row, `tomakast settings [--page <page>]`.
- **Autostart follows the file live** (it used to apply only at daemon start).
- **A plugin catalog** (every plugin folder: running, waiting for review, denied, failed,
  manifest error) published by the plugin manager for the window. The first scan always
  publishes one, even empty; a plugins folder that cannot be listed makes it unavailable.

## Checked in tests

`just check` on the branch head passes: fmt, clippy (workspace and the two cross targets),
the workspace tests. Per area:

- core: schema parity, `read_key`/`value_fits`/`unrecognized`, one-key edits (comments,
  empty-table rule, refusal of broken input and output), Home `SetParam`/`ClearSize`,
  launcher-hotkey checks, catalog types;
- plugin host: preference kinds and every refusal rule, the catalog states;
- app: the launcher hotkey's accept-then-write path (accepted, refused, failed, after restart,
  reset, both sets following the file after a pending change), live autostart (including a
  start on a broken config), the `settings` IPC request and CLI;
- ui (headless, kit window): about 100 tests of the window: every page, every field kind and
  its commit rule, failed writes, page identity across catalog changes, Hotkeys and Home
  editing, recording ending on a page change, outside edits with and without the question,
  the question queue, Esc, gone tiles, a broken file.

Each task was reviewed against the spec and the plan, and the whole branch once more at the
end. That last review's findings were fixed: plugin tables read-only until the plugins are
listed, Reset on a written `hotkey = "Alt+Space"` removes the line, refusal notices under
the field they answer, a `location`/`home` that is not a table listed on its page, and
descriptions for the weather units. Everything else was recorded with a ruling.

## Run on KDE Plasma (Wayland)

`just build`, then the built daemon with a throwaway `TOMAKAST_HOME`
(`/run/user/1000/tk-settings-smoke`; the scratchpad path is too long for the IPC socket),
`TOMAKAST_NO_KEYBOARD=1`, calc and weather installed and approved, and a `config.toml` with
comments, `[location]`, two `[hotkeys]` for one item, `[plugins.weather] wind = "ms"`, a
`[plugins.gone]` table, an unknown top-level key and two Home tiles.

- `tomakast settings` opened the window; opening it changed nothing in the file.
- `tomakast settings --page plugin:weather` showed the Weather page: the three choices with
  their values (wind "m/s" with ↺), the pressure description, and "Refresh Weather" with an
  empty hotkey field. The sidebar listed General, Location, Hotkeys, Home, Calculator,
  Weather and "gone (not installed)".
- General showed the launcher hotkey with "Off" (no hotkeys in a `TOMAKAST_NO_KEYBOARD` run),
  "Start at login", and `zz_unknown = 1` under "Not recognized" with Remove.
- The user picked Barcelona on Location by hand: `config.toml` got `city = "Barcelona"` in
  place, and the daemon applied it at once (`location settings city=Barcelona`).
- A hand edit of `[plugins.weather]` while the window was open: the plugin got
  "preferences changed" without a restart, and the window followed the file.
- The daemon log has no errors from the window. The environmental lines are the known missing
  `.SystemUIFont` and "server-side decorations not supported" (KWin gives the window a
  client-side frame).

Not seen by eye in this run (the window had no keyboard focus by design): the conflict
question, typing in fields, the Hotkeys recorder, Home editing. The headless tests cover them;
the checklist below is for a hand run.

## macOS and Windows

Not run. `just check` type-checks and lints only `tomakast-location` for
`aarch64-apple-darwin` and `x86_64-pc-windows-gnu`; the settings window code is platform
independent GPUI/kit code, but it was not built for those targets here.

## Checklist for `just try approve`

1. Ctrl+, in the launcher, and the "Settings" item, open the window; a second open focuses it.
2. Ctrl+K on Weather's row → "Plugin Settings" opens the Weather page.
3. Change wind to m/s: `config.toml` gets `[plugins.weather] wind = "ms"`, the tiles redraw.
4. Type a city param on a Home tile without Enter, then edit the same key in the file and
   save: the question appears; "Keep mine" writes your value, other hand edits stay.
5. Edit a different key in the file while typing: no question, your text stays.
6. Break the file (`[launcher`): banner, controls disabled; fix it: the window recovers.
7. Hotkeys tab: "Add hotkey…" → pick Calculator → press a combination → Enter; it works at
   once. Press an already bound combination while recording: it is recorded, not run.
8. Reset the launcher hotkey when the file says `hotkey = "Alt+Space"`: the line is removed.
9. Open settings right after the daemon starts, before the plugins finish loading: plugin
   tables show "Loading plugins…" and stay read-only until the plugins are listed (no
   "Remove all").
10. Hide the launcher, then open settings from the item: the launcher hides first.

## Known limits

- The conflict question appears only while the settings window is in front: a text field
  commits when the window loses focus, and a recording hotkey field ends on Alt-Tab.
- A recording hotkey field gets no question (asking would end the recording).
- The kit sidebar has no section headings, so plugin pages follow Home without a "Plugins"
  heading; a sidebar title cannot be greyed, hence "(not installed)".
- The number stepper no longer stops at the bounds (the kit would clamp silently on blur); an
  out-of-range value is refused with a hint on Enter and on blur.
- Flaky tests outside this branch's code were seen during `just check`: a `plugin-sdk` lib
  test abort and a `plugin-host` http timing test; both passed on re-run.

## Second round: look, layout and frame

Spec `docs/superpowers/specs/2026-11-23-settings-window-look-design.md`, plan
`docs/superpowers/plans/2026-11-23-settings-window-look.md`.

### Run on KDE Plasma (Wayland)

`just build`, then the built daemon with `TOMAKAST_HOME=/run/user/1000/tk-look`,
`TOMAKAST_NO_KEYBOARD=1`, calc and weather installed and approved, and a `config.toml` with
comments, `[appearance] theme`, `[location] city`, `[plugins.weather] wind = "ms"`, a
`[plugins.gone]` table and one Home tile. Each page was opened with
`tomakast settings --page <id>` in dark and in light (`[appearance] theme`), and the window
was captured.

- The sidebar shows Core (General, Appearance, Location, Hotkeys, About), Modules (Home) and
  Plugins (Calculator, Weather, `gone (not installed)` with a greyed monogram), each entry
  with its icon. There are no Preferences or Commands sub-entries. Group titles are headings
  in the page.
- The window is opaque, in the launcher's colours, with rounded corners and the kit's shadow.
  The title band has the search field over the sidebar, the page title over the page, and
  Breeze-like minimize, maximize and close on the right (KWin's default order).
- Weather shows its three choices with no captions; wind is m/s with ↺. Location shows
  Barcelona with ↺ and the Wi-Fi switch. Appearance shows Theme with the file's value and ↺.
  About shows the name and version. General shows the Config file row with the path, Open and
  Show in folder.
- The first run showed controls ending at two different right edges, because the reset
  button kept a slot right of the control. Fixed: ↺ sits left of the control, and every
  control ends at one edge.

The screen was locked during the run. KWin then stops sending frames to an open window, so
a running window never redraws. Each capture was therefore taken from a fresh daemon,
showing the first frame of the page asked for. For the same reason these were not seen by
eye:

- dragging, resizing, double-click to maximize, and the window menu;
- a theme switch applied while the window is open;
- Open and Show in folder;
- search typing.

The headless tests cover the window's side of these: a theme preference set while the window
is open switches it, search, the opener calls (with a fake opener) and the choice of
controls. The daemon's side of a theme change (a file edit → reload → the new preference)
has no test and was not seen; it is item 3 of the hand checklist below, as is the frame's
look and behaviour.

### macOS and Windows

Not built for those targets here. The frame code for them is selected at run time with
`cfg!` and type-checks on Linux. The `#[cfg(target_os)]` bodies were never compiled:
`reveal_path` for macOS (`open -R`) and Windows (`explorer /select,`), and the non-Linux
`kwin_button_layout`. macOS uses native traffic lights, and the app owns the title
band's drag and double-click. Windows marks the band as caption and the buttons as caption
buttons.

### Checklist for `just try approve`

1. The frame:
   - drag the title band to move the window;
   - double-click it to maximize, and again to restore;
   - right-click it for the window menu;
   - resize from an edge or corner;
   - the buttons minimize, maximize/restore and close.
2. Set KWin's buttons on the left (System Settings → Window Decorations → Titlebar Buttons),
   then reopen the window: the controls follow the side and order.
3. Appearance → Dark/Light/System: the launcher and the window switch at once.
4. Search `wifi`: only Location stays. Search `zzz`: "No settings match". Esc clears the search.
5. Delete `config.toml`, then click Open: an empty file is created and opens in the editor.
   Show in folder opens the file manager with the file selected.
6. Stop the plugins (break `tomakast.db`'s permissions): the plugin pages stay, show
   "Not running: …", and remain editable.
