# Settings window: every key of `config.toml`, one to one

Status: draft for the user's approval, 2026-11-22.

## Intent

Today every setting of tomakast lives in `config.toml`, edited by hand. The user's words:
"a GUI for settings, both for the launcher as a whole and settings for individual plugins.
Settings in the GUI correspond 1:1 to the settings in the config file." Why: many people find
it easier to click through a GUI than to learn a config format. The file stays: it is still
the one source of truth, still edited by hand by those who like that, and the window is a
second way to edit the same file.

Success: the user opens "Settings" from the launcher, sees every setting the file can hold
(the launcher's own, every plugin's, every hotkey, Home), changes one with the mouse, and it
takes effect at once and is in `config.toml` with their comments and layout intact. A hand
edit of the file shows up in the open window without a question, unless the user was in the
middle of typing a value the file just changed under them.

## Decisions (the user's)

1. **One description of every setting drives both reading the file and the window**
   (approach A below). A new setting is added in one place and appears in both.
2. **Hotkeys have a tab of their own**, and are also edited in context: on a plugin's page
   next to its commands, and in the launcher's Ctrl+K as today.
3. **Plugin preferences get richer kinds**, built from the component library's controls.
4. **A hand edit while the window is open:** when the window has no pending edit, the file's
   new contents are loaded at once, without a question. When both changed, a modal asks which
   version to keep.
5. Earlier (2026-11-20, `docs/experiments/2026-11-gpui-kit-components.md`): settings and the
   plugin manager live in **a separate ordinary window** opened through the kit, not in the
   launcher panel, so the kit's settings layout, dialogs and tooltips work there.

## Author's calls (for the user to check)

1. Every change is written at once (switches, dropdowns at once; text on Enter or leaving the
   field), as in Home edit mode. So "edited in both places" means a typed, unwritten value.
2. The modal appears only when the outside edit changed the same key the user is typing;
   an outside edit of other keys updates them silently and leaves the typed field alone.
3. A Home page: reorder, resize, edit params, remove; adding stays in the launcher.
4. Everything the file holds has a visible place, read-only where the window has no control
   for it; the window never deletes anything the user did not ask it to.
5. The launcher hotkey is written only after the system accepted it, like item hotkeys.
6. Weather moves to the new kinds (the user asked, 2026-11-22); calc does not (the calc
   converter session works on it in parallel). No password kind.

## Goals

- A settings window with pages for: General (launcher hotkey, autostart), Location, Hotkeys,
  Home, and one page per installed plugin.
- Every key `Settings` reads from `config.toml` has a control in the window, and every
  control writes exactly one key (or one entry of a list). A test fails when the parser and
  the description disagree.
- A change applies at once, exactly as a hand edit of the file does today, through the same
  reload paths.
- Writes keep the user's comments, order and blank lines (as Home edit mode and item hotkeys
  already do).
- New plugin preference kinds: `choice` (a dropdown) and `number`; an optional `description`
  for every preference.
- Opening: a "Settings" item in the launcher's search, `tomakast settings` on the command
  line, Ctrl+, (Cmd+, on macOS) in the launcher, and "Plugin Settings" in a plugin row's
  Ctrl+K, which opens the window at that plugin's page.

## Non-goals

- A plugin manager (enable, disable, remove, approve). Approval stays in the launcher's
  review prompt. The window shows a plugin's state as text only.
- Theme, font or appearance settings: the file has none today.
- Adding Home tiles from the window. "Add to Home" in the launcher's search stays the way to
  add one, because there the user sees the tile first.
- Migrating `plugins/calc` to the new preference kinds: the calc converter session works on
  calc in parallel.
- A `secret` / password kind. A value in `config.toml` is plain text whatever the window
  shows; a masked field would suggest a protection that is not there.
- Verifying macOS and Windows by hand in this work (see Testing).

## Anti-goals (failure, even if it "works")

- The window and the file disagree: a key the file holds that the window does not show, a
  control that writes something the parser reads differently, or a value shown that is not
  the one in effect.
- A write loses the user's comments or reorders their file, or rewrites a file that does not
  parse.
- The window asks "which version?" after its own write, or when the user had nothing pending.
- Opening the launcher gets slower. Nothing of the settings window is built or loaded until
  it is opened.
- A plugin's settings page lets the plugin draw anything: its page is built only from its
  manifest's declarations, like every other plugin surface.

## Constraints

- `crates/core` has no GPUI dependency; the description of settings lives there, the window
  in `crates/ui`.
- gpui-kit `=0.10.0`. Its `Settings` layout (sidebar with search, pages, groups, items with
  title, description and a control, "reset to default") works only in a window with the kit
  Root, which a window opened with `gpui_kit::open_window` has. It was read, not run, in the
  2026-11-20 experiment.
- The kit's text setting field calls `set_value` on every keystroke and overwrites the input
  from `value` on every render (`setting/fields/string.rs`). That fits neither "write on
  commit" nor "keep the typed value through a reload". Text, number and city fields are
  therefore custom fields (`SettingField::render` with an input state the window owns); the
  kit's switch and dropdown fields are used as they are.
- The hotkey recorder (`crates/ui/src/views/hotkey_recorder.rs`) is a `LauncherView` part,
  and a press of an already bound global hotkey reaches it only through
  `LauncherController::is_recording` / `recorded_press` in the daemon's `apply_outcomes`.
- Live apply today: the launcher hotkey and `[hotkeys]` through the daemon's config watch;
  plugin preferences, `[location]` and Home through the plugin manager's watch. **Autostart is
  applied only at daemon start** (`daemon.rs`, `autostart_plan`).
- `[location] city` names one of 6272 built-in cities (`crates/places`).
- The plugin manifest format is frozen with WIT 0.1, but may be edited in place while no
  outside plugins exist (`crates/plugin-api/wit/since_v0.1.0/FROZEN.md`, the freeze test, and
  `docs/plugins/manifest.md` change together).
- Other sessions work in parallel (translator plugin, file search, calc converter). Shared
  code is touched narrowly: `settings.rs`, `settings_edit.rs`, `manifest.rs`, the daemon's
  reload loop, `LauncherController`.

## Approaches considered

**A. One description of every setting; the parser and the window both read it. Chosen by
the user.** `crates/core` gets a schema: a list of every key with its place in the file, its
kind, default, title and description, and the page and group it shows on. `Settings` is read
by walking the schema; the window is built from the same schema. Plugin preferences already
have their schema, the manifest. A test fails when the parser reads a key that the schema
does not list. Cost: today's serde-based reading is rewritten, carefully keeping its
behaviour.

**B. Hand-written pages plus a parity test.** Reading stays as it is; a test compares the
keys the parser knows with the keys the pages show. Quicker to start, but every new setting
is written twice, and the test catches a miss instead of preventing it. Not chosen.

**C. Generate the schema from the `Settings` struct with a derive macro.** Rejected: the file
has irregular parts (`[hotkeys]` maps a hotkey to an item, `[[home.widgets]]` is a list of
tables, `[plugins.<id>]` is keyed by plugin), so the macro would cost more than it saves.

## Design

### What the user sees

```
┌ Settings ───────────────────────────────────────────────────────────┐
│ [Search settings…]  │  General                                       │
│                     │  ───────────────────────────────────────────   │
│ General             │  Launcher hotkey            [ Alt+Space ] ↺    │
│ Location            │  The combination that shows the launcher.      │
│ Hotkeys             │                                                │
│ Home                │  Start at login                        [■]  ↺  │
│ Plugins             │                                                │
│   Calculator        │                                                │
│   Weather           │                                                │
│   Translator  ⏳    │                                                │
└─────────────────────────────────────────────────────────────────────┘
```

- **Sidebar** (the kit's): a search box that filters items by title, description and
  keywords; pages; under "Plugins", one page per installed plugin, in name order.
- **An item**: title, optional description under it, the control on the right, and "reset
  to default" (↺) when the file holds the key. Reset removes the key from the file.
- **General**: launcher hotkey (a hotkey field: click, press a combination, Enter or click
  away to keep it, Esc to cancel), "Start at login" (switch).
- **Location**: city (a searchable dropdown of the built-in cities; the first entry is
  "Automatic", which removes the key), "Look up location by Wi-Fi" (switch) with its
  privacy description (the networks around are sent to BeaconDB).
- **Hotkeys**: the launcher hotkey first, then every `[hotkeys]` entry as a row: the item's
  icon and title (or its id, greyed, when the item is not indexed now), its hotkey field, and
  a remove button. "Add hotkey…" opens a search over the launcher's items (the core's own
  search); picking one adds a row with an empty hotkey field focused. The recorder's refusals
  are the same as in the launcher ("Already set on …", "This is the launcher's hotkey").
- **Home**: the tiles in order, each a row: the widget's title and plugin, a size dropdown
  (only the sizes the widget declares; "Default" removes the key), one text field per
  declared param (by its title), up / down buttons, and remove. A note at the top: "Add tiles
  from the launcher's search: Add to Home." An entry the launcher cannot read is a greyed row
  "An entry the launcher cannot read" that is never moved or changed, as in edit mode.
- **A plugin's page**: its name, version and author; its state when it is not running
  ("Waiting for your approval in the launcher", "Not allowed", "Failed to load: …"); its
  preferences, by declaration order, each with the control of its kind; then "Commands", each
  with its icon, title and a hotkey field (the contextual hotkey editing). Values for keys
  the manifest does not declare are listed at the end under "Not used by this plugin" as
  read-only `key = value` lines with a remove button each.
- **Settings of plugins that are not installed** (a `[plugins.x]` table with no plugin `x`)
  appear as greyed pages under "Plugins" with the same read-only lines and a "Remove all"
  button. Nothing is removed unless the user asks.
- **Keys the launcher does not know** (a top-level table or key outside the schema, and an
  unknown key inside `[launcher]`, `[location]` or `[home]`) are listed read-only on the page
  of their table (top-level ones on General) under "Not recognized", with the parser's
  warning, and a remove button each. The window never removes them by itself.
- **Everything else the file holds has a visible place too** (author's call, from the
  anti-goal "a key the file holds that the window does not show"):
  - a built-in key with a value of the wrong type (`city = 5`): its control shows the
    default in effect, in the error state "Not valid here: 5; the default is used";
  - a Home tile param its widget does not declare: a read-only line in that tile's row,
    with a remove button;
  - a Home tile whose plugin is not installed: a greyed row with its widget id, size and
    params as text; it can be moved and removed, not edited;
  - a `[hotkeys]` entry the launcher drops (a second hotkey for the same item, an item id
    that does not parse is already a file error): a greyed row "Not used: <item> already has
    <hotkey>", with a remove button.
- **Plugins the window cannot fully see:**
  - a plugin folder whose `plugin.toml` does not parse is in the catalog with the state
    "Manifest error: <message>"; its `[plugins.x]` values show as read-only lines (no
    "Remove all", so the user never wipes the settings of a plugin they still have);
  - when plugins cannot load at all this run (no plugin manager), the Plugins section shows
    one notice, "Plugins are not running: <reason>", every `[plugins.x]` table is read-only,
    and Location and Home pages say "Changes apply after a restart" (their live path is the
    plugin manager's watch). Nothing is hidden.
- **A broken file** (not valid TOML, or a value the parser refuses, such as a malformed
  launcher hotkey): a banner over every page, "config.toml has an error: <message>. Fix the
  file; nothing is written until then." Controls are disabled. The window recovers by itself
  when the file is fixed.

### The schema (`crates/core/src/settings_schema.rs`)

- `SettingKey { path: &'static [&'static str], kind, default, title, description, page,
  group, keywords }` for each scalar key:
  - `launcher.hotkey`: `Hotkey`, default `Alt+Space`.
  - `launcher.autostart`: `Bool`, default `true`.
  - `location.city`: `City`, default empty (automatic).
  - `location.wifi_lookup`: `Bool`, default `false`.
- Collections, each with a page of its own and an editor in `settings_edit`:
  - `hotkeys`: a table of hotkey → item id.
  - `home.widgets`: an array of tables.
  - `plugins.<id>`: a table per plugin, described by that plugin's manifest.
- Kinds: `Bool`, `Text`, `Number { min, max, step, integer }`, `Choice(options)`, `Hotkey`,
  `City`. Plugin preference kinds map onto the same set (`text` → `Text`, `bool` → `Bool`,
  `choice` → `Choice`, `number` → `Number`), so one set of controls serves both.
- `Settings::from_toml` walks the schema: for each scalar key it reads the value, checks its
  kind and falls back to the default exactly as today (a malformed launcher hotkey or
  `[hotkeys]` entry is still an error; a wrong-typed `[location]` value is still skipped with
  a warning). `Settings`' public fields and behaviour do not change; today's tests in
  `settings.rs` pass unchanged.
- Leniency stays: unknown top-level keys and tables are accepted with a warning (today they
  are silently ignored; the warning is new), and the legacy `[widgets]` keeps its warning.
  "The parser and the schema agree" means: **every key the parser gives a meaning to is in
  the schema**, checked by a test that feeds the parser a file with every schema key set to a
  non-default value and asserts each one shows up in `Settings`, and by the parser having no
  key-reading code outside the schema walk and the three collections. It does not mean the
  file may hold only schema keys.
- `Settings::unrecognized(text) -> Vec<(path, warning)>` gives the "Not recognized" list.

### Plugin preference kinds (manifest, WIT 0.1 edited in place)

`[[preferences]]` gains:

| Key | Type | Rules |
|---|---|---|
| `kind` | string | adds `"choice"` and `"number"` to `"text"` and `"bool"`. |
| `description` | string | optional, 1–256 characters, no control characters. Shown under the title. |
| `options` | array of tables | `choice` only, required there: 2–32 `{ value, title }`, values unique, each value with the `key` rule's charset plus `_` and `.`, titles with the `name` rule. |
| `min`, `max`, `step` | numbers | `number` only, optional; `min <= max`; `step > 0`. A `number` with all three integers is an integer field. |

- `default` stays a TOML string for every kind (`default = "metric"`, `default = "10"`); it
  must be one of the options for `choice` and a number within bounds for `number`, or the
  manifest is refused.
- In `config.toml` a `choice` value is a string, a `number` value is a TOML integer or float
  (`precision = 10`). `plugin_preferences()` today skips anything that is not a string or a
  boolean; it now also accepts integers and floats and hands them to the plugin as their TOML
  text (`"10"`, `"2.5"`). The plugin still reads every value as a string through
  `preferences.get`; WIT does not change.
- The window writes only values valid for the kind. A hand-written value that is not (a
  string for a `number`, a value outside `options`) is shown in the control's error state
  with "Not valid here: <value>", and the plugin receives it as before; nothing rewrites it
  until the user picks a value.
- `manifest.rs`, `crates/plugin-api/tests/freeze.rs`, `docs/plugins/manifest.md` and
  `FROZEN.md` change together (the freeze test reads the doc's tables).

### Weather adopts the new kinds

`plugins/weather` replaces its two bools with three choices, each with a description. The
temperature and the wind are separate settings (today one bool switches both), because
metres per second go with Celsius as often as km/h do:

- `temperature`: `celsius` ("°C", default) or `fahrenheit` ("°F"); replaces `fahrenheit`.
- `wind`: `kmh` ("km/h", default), `ms` ("m/s"), `mph` ("mph") or `kn` ("knots"). Open-Meteo
  takes each as `wind_speed_unit`, so the conversion stays on its side.
- `pressure`: `hpa` ("hPa", default) or `mmhg` ("mm Hg"); replaces `mmhg`.

The plugin's single metric/imperial switch becomes these three; its caches, keyed by the
units today, are keyed by the temperature and wind units together.

The old keys are not read any more: no `config.toml` known to hold them (the user's has
none), and a leftover one shows on the plugin's page under "Not used by this plugin" with a
remove button.

### Writing the file (`crates/core/src/settings_edit.rs`)

- New edits over the file's text, each changing one thing and keeping everything else:
  - `set_key(text, path, Option<Value>)`: sets or removes one scalar key (`None` removes
    it). A rule on the text alone, so it holds across sessions: a table a removal leaves with
    no keys and no comments (in its header's decor or inside it) is removed; one with a
    comment is kept. A table the window creates is written plain, so it qualifies.
  - `set_plugin_preference(text, plugin, key, Option<Value>)` and
    `remove_plugin_table(text, plugin)`.
  - `set_item_hotkey` stays as it is (the Hotkeys page goes through the same path as the
    launcher's recorder, below).
  - Home edits go through `home_file::edit_home`, which already moves, resizes and removes
    one entry; it gains "set the params of entry n".
  - Like the existing edits, each refuses a text that does not parse as `Settings`.
- Each write reads the file's current text, applies the one edit, and writes atomically with
  `write_config` (temp file and rename, symlink kept). The window never writes its whole
  model back.
- After a write the window remembers the text it wrote. A change event whose file content is
  byte-for-byte that text is its own write and is ignored. Anything else is an outside edit.
- What changed applies through the existing reload paths, exactly as for a hand edit: the
  daemon's config watch (launcher hotkey, `[hotkeys]`) and the plugin manager's watch
  (preferences, location, Home). **Autostart gets a live path:** the daemon's reload loop also
  calls `set_autostart` when `launcher.autostart` changed and the file loaded cleanly (the
  same `autostart_plan` rules, so `TOMAKAST_HOME` still leaves it alone).

### Commit, outside edits and the modal

- **When a value is written:** a switch, dropdown, size, "reset" or remove at once; a text or
  number field on Enter or when it loses focus (Esc restores the file's value); a hotkey
  field when the recorded combination is confirmed. This matches Home edit mode, where every
  change is written at once.
- So the window's only "pending edit" is a text, number or city field holding typed text
  that is not yet written, or a hotkey field mid-recording.
- **An outside edit** (see "Writing the file") is read and the window updates every control,
  except a field with a pending edit:
  - no pending edit: the new values are shown at once, no question (the user's rule);
  - a pending edit, and the file changed that same key: a modal (the kit's dialog), "config.toml
    changed while you were editing <title>. Keep your value <typed> or use the file's
    <value>?" with "Keep mine" (writes it, on top of the new file) and "Use the file's"
    (drops the typed text);
  - a pending edit, and the file changed other keys only: the other controls update; the
    pending field stays as typed; no question, since nothing conflicts.
- The window has one watch of its own on `config.toml`, using the existing `watch_config`
  helper (`crates/app/src/config_watch.rs`, moved to where the UI can use it, or handed in by
  the daemon as a callback). The daemon and plugin manager watches are unchanged.

### The window

- `crates/ui/src/settings/`: the window, its pages, the custom fields (text, number, city,
  hotkey), and the conflict modal. Opened with `gpui_kit::open_window` as a normal decorated,
  resizable window (about 900×640), app id `dev.tomakast.Tomakast.Settings`. One instance: a
  second open request focuses it and switches to the requested page.
- Nothing of it is created at daemon start; the launcher's prewarm and open path do not
  change. The first open costs about 200 ms (the kit probe); that is acceptable for this
  window.
- `TOMAKAST_NO_KEYBOARD=1` opens it without focus (`focus: false`), as the launcher does, so
  development runs do not take the user's keyboard.
- Data it needs, given by the daemon when it opens and kept current while open:
  - the config file's path and current text;
  - the plugin catalog: for every plugin folder with a manifest that parses, its id, name,
    version, author, state, preference declarations and commands. The plugin manager
    publishes it (new `ManagerAction::Catalog`) whenever it loads, reloads or reviews a
    plugin. Its types live in `tomakast-core` so the UI does not depend on the plugin host;
  - item titles and icons, and a search over items, from the core (for Hotkeys and "Add
    hotkey…");
  - the hotkey table, for the recorder's refusals.
- **The hotkey recorder** is split: the part that checks a combination against the table
  and words the refusal becomes a plain struct in `crates/ui` shared by both windows; each
  window draws it its own way. `LauncherController::is_recording` and `recorded_press`
  become "is any recorder open" and "give the press to the open recorder", so a press of an
  already bound global hotkey reaches the settings window's recorder too.
- **Every hotkey, the launcher's included, goes through the accept-then-write path** of item
  hotkeys (`crates/app/src/hotkeys.rs`: bind first, write `config.toml` only once the system
  accepted it, roll back otherwise). `Hotkeys::request` and `HotkeyTable` learn the launcher
  hotkey as one more entry (its id is already `LAUNCHER_ID` in the daemon), written with
  `set_key("launcher.hotkey")` instead of `set_item_hotkey`. So a combination KDE refuses is
  never written, and the user cannot lock themselves out of the launcher. Where a change
  needs a restart (the portal binds once per run), the file is written and the field says
  "Applies after a restart".
- **Hotkey notices** (refused, after restart, off) go to the settings window when it is open
  and the change came from it, shown under the field; otherwise to the launcher's toast as
  today. Each hotkey row also shows its current state from the table (`HotkeyState`): a
  refused or off row is marked with the reason. This touches `apply_outcomes` and
  `LauncherController` narrowly: one more receiver for notices and table updates.

### Opening it

- A built-in item "Settings" (`settings/open`, icon `settings`), searchable as "Settings",
  "Preferences", "Config". Running it hides the launcher and opens the window.
- Ctrl+, in the launcher (Cmd+, on macOS) does the same.
- In the Ctrl+K panel of a plugin's row: "Plugin Settings", opening the window at that
  plugin's page. Only for plugins with preferences or commands.
- `tomakast settings [--page <page>]`: an IPC request `settings [page]` to the running
  daemon, like `toggle`. Page ids: `general`, `location`, `hotkeys`, `home`,
  `plugin:<id>`.

## Testing

- Core, unit tests:
  - the parity test above (every schema key set to a non-default value reaches `Settings`);
  - today's `settings.rs` and `settings_edit.rs` tests unchanged;
  - each new edit keeps comments, order and blank lines, refuses a broken file, and removes
    a key the window created without leaving an empty table;
  - preference kinds: manifests with each kind, every refusal rule, the freeze test against
    the updated doc;
  - numbers in `[plugins.x]` reach the plugin as text.
- UI, headless (`TestAppContext` with the kit Root, as the launcher's view tests do): the
  window builds every page from a sample file and catalog; a switch writes the file; a text
  field writes only on Enter or blur; an outside edit updates controls; an outside edit of a
  key being typed raises the modal and each button does what it says; an own write raises
  nothing; a broken file disables controls.
- Daemon: autostart follows a reload (the `autostart_plan` test extended).
- By hand on KDE Plasma Wayland with `just try` (throwaway `TOMAKAST_HOME`,
  `TOMAKAST_NO_KEYBOARD=1`): open from the item, from `tomakast settings`, from Ctrl+K on a
  plugin; change each kind of setting and see it apply; hand-edit the file with and without
  a pending field. Results in `docs/experiments/2026-11-settings-window-acceptance.md`.
- macOS and Windows: built and type-checked only; the window is a plain kit window there, but
  it is not run by hand in this work. Said so in the acceptance note.

## Left to the builder

- The exact kit APIs for custom fields, the dialog and window options, within the design
  above.
- Wording of descriptions and empty states, in the project's plain style.
- Whether `watch_config` moves to `crates/core` (no GPUI) or the daemon hands the window a
  change callback.
- The order of the work, staged so `just check` passes after each step: schema and parity
  tests; generic key edits; the window with General and Location; preference kinds and plugin
  pages; Hotkeys page and the shared recorder; Home page; outside edits and the modal; entry
  points; autostart live path.
