# Settings Window Look, Layout and Frame Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the settings window look and feel like the launcher: our own sidebar with three
titled sections and icons, Appearance (theme) and About pages, a Config file row on General
that always works, plugin pages that follow the plugins folder rather than the plugin runtime,
and a window frame of our own with controls that fit the platform.

**Architecture:** The window stops using the component library's `Settings` layout. Pages are
built into a small model of our own (page → groups → rows) that one renderer draws with the
launcher's tokens and that one pure function filters for search. Library controls (inputs,
selects, switches, buttons, dialogs) and the library Root stay. The frame uses GPUI's
client-side decorations; the library Root already draws the border, shadow and resize edges;
we draw the title area and the controls, chosen by a pure function.

**Tech Stack:** Rust 2024, GPUI through `gpui-kit =0.10.0` (pinned), toml/toml_edit, zbus
(Linux, already a dependency of `tomakast-platform`).

**Spec:** `docs/superpowers/specs/2026-11-23-settings-window-look-design.md` (builds on
`docs/superpowers/specs/2026-11-22-settings-window-design.md`).

## Global Constraints

- `gpui-kit =0.10.0`, no upgrade, no fork, no vendored copy of its settings layout.
- No new external crates; `kwinrc` is parsed by hand.
- Only bundled assets: Lucide icons from the kit's asset set, Inter, JetBrains Mono.
- Window background opaque: the launcher panel fill `0x1c1c1e` (dark) / `0xf6f6f6` (light) at
  alpha 1.0.
- New sizes are named constants in `crates/ui/src/theme.rs`, not literals in window code.
- Every interactive element keeps an accessible label; sidebar entries are labelled with the
  page title (`"General"`, `"Weather"`, …); the search field keeps the label `"Search..."`.
- Window title stays `"Settings — tomakast"`.
- Do not touch other branches' areas; changes to `crates/app/src/daemon.rs`,
  `crates/plugin-host/src/manager.rs` and `crates/core/src/settings*.rs` stay minimal.
- Run and try the app only through `just` recipes; for own runs `TOMAKAST_NO_KEYBOARD=1` and
  a throwaway short `TOMAKAST_HOME` under `/run/user/1000/`. One `just try` at a time.
- No screenshots committed; no attribution lines in commits; no push, no merge.
- `just check` passes at the end of every task.

## Review Focus

1. **A plugin page shown while the folder can be listed but the runtime is down**: preferences
   and command hotkeys stay editable and write to the file → Task 3 test
   `plugin_pages_are_editable_when_plugins_do_not_run`.
2. **Search with a query that matches only a group title or only a key name** (`wifi_lookup`):
   the page appears with that group → Task 4 unit tests `a_group_title_match_keeps_the_group`,
   `a_key_name_matches`.
3. **The catalog changes while a query is typed**: the query stays and the shown page stays if
   it still matches → Task 4 test `a_catalog_change_keeps_the_query`.
4. **A theme value written by hand that is not one of the three** (`theme = "blue"`): the
   window shows it in the error state, the launcher uses System → Task 1 test
   `an_unknown_theme_warns_and_uses_system`, Task 6 test `an_invalid_theme_is_shown_as_invalid`.
5. **KWin settings with only one side set, or letters outside I/A/X** (`ButtonsOnRight=HIAX`,
   no `ButtonsOnLeft`) → Task 7 unit tests in `kwin_buttons`.

---

### Task 1: Theme key in the config, weather without captions

**Files:**
- Modify: `crates/core/src/settings_schema.rs` (new key, `Page::Appearance`)
- Modify: `crates/core/src/settings.rs` (`Settings.appearance`)
- Modify: `crates/core/src/lib.rs` (re-export `ThemeChoice` if needed)
- Modify: `plugins/weather/plugin.toml` (drop three `description` lines)
- Modify: `crates/plugin-host/tests/weather.rs` (drop `every_preference_has_a_description`)
- Test: `crates/core/tests/settings_schema.rs`, `crates/core/tests/settings.rs`

**Interfaces:**
- Produces: `tomakast_core::settings::ThemeChoice { System, Light, Dark }` (`Clone, Copy,
  Debug, Default = System, PartialEq, Eq`), `ThemeChoice::parse(&str) -> Option<ThemeChoice>`
  (`"system"`, `"light"`, `"dark"`); `Settings.appearance: ThemeChoice`;
  `settings_schema::Page::Appearance`; the key `["appearance", "theme"]`.

- [ ] **Step 1: Write failing tests**

```rust
// crates/core/tests/settings.rs
#[test]
fn theme_is_read_from_appearance() {
    let s = Settings::parse("[appearance]\ntheme = \"dark\"\n").unwrap();
    assert_eq!(s.appearance, ThemeChoice::Dark);
    assert_eq!(Settings::parse("").unwrap().appearance, ThemeChoice::System);
}

#[test]
fn an_unknown_theme_warns_and_uses_system() {
    let s = Settings::parse("[appearance]\ntheme = \"blue\"\n").unwrap();
    assert_eq!(s.appearance, ThemeChoice::System);
}

// crates/core/tests/settings_schema.rs
#[test]
fn theme_is_a_choice_on_appearance() {
    let key = builtin_keys().iter().find(|k| k.path == ["appearance", "theme"]).unwrap();
    assert_eq!(key.page, Page::Appearance);
    assert_eq!(key.default, "system");
    assert_eq!(key.on_invalid, OnInvalid::Warn);
    let ValueKind::Choice(options) = &key.kind else { panic!() };
    let pairs: Vec<_> = options.iter().map(|o| (o.value.as_str(), o.title.as_str())).collect();
    assert_eq!(pairs, [("system", "System"), ("light", "Light"), ("dark", "Dark")]);
}
```

Adjust the existing schema parity test only if it lists keys explicitly.

- [ ] **Step 2: Run** `cargo test -p tomakast-core theme` — expected FAIL (no `appearance`).
- [ ] **Step 3: Implement.** Add the `SettingKey` (title `"Theme"`, keywords `["theme", "dark",
  "light", "appearance", "colors"]`, description `None`) first among keys of its page, after the
  General/Location keys in the list; `Choice` value checking already exists. Parse it in
  `Settings::parse` through the schema walk like `location` keys (Warn).
- [ ] **Step 4: Remove the weather captions** (the `description` lines of `temperature`,
  `wind`, `pressure`) and the test `every_preference_has_a_description`.
- [ ] **Step 5: Run** `cargo test -p tomakast-core` and `cargo test -p tomakast-plugin-host
  --test weather` — PASS.
- [ ] **Step 6: Commit** `settings: [appearance] theme key; weather choices without captions`.

### Task 2: The theme applies live to the launcher and the settings window

**Files:**
- Modify: `crates/ui/src/theme.rs` (`mode_for`, preference global)
- Modify: `crates/ui/src/lib.rs` (export `set_theme_preference`)
- Modify: `crates/app/src/daemon.rs` (call at start and on each successful reload)

**Interfaces:**
- Consumes: `ThemeChoice` (Task 1).
- Produces: `pub fn tomakast_ui::set_theme_preference(choice: ThemeChoice, cx: &mut App)` —
  stores a global `ThemePreference(ThemeChoice)` and, when it changed, re-runs the theme switch
  for every open window (`cx.refresh_windows()` after `Theme::change` as `sync` does).
  `theme::mode_for(choice: ThemeChoice, system: ThemeMode, dev_light: bool) -> ThemeMode`.

- [ ] **Step 1: Failing unit test** in `theme.rs`, replacing `dev_light_forces_the_light_mode`:

```rust
#[test]
fn mode_follows_the_choice_then_the_system() {
    use tomakast_core::settings::ThemeChoice::*;
    assert!(mode_for(System, ThemeMode::Dark, false).is_dark());
    assert!(!mode_for(System, ThemeMode::Light, false).is_dark());
    assert!(mode_for(Dark, ThemeMode::Light, false).is_dark());
    assert!(!mode_for(Light, ThemeMode::Dark, false).is_dark());
    assert!(!mode_for(Dark, ThemeMode::Dark, true).is_dark()); // dev flag wins
}
```

- [ ] **Step 2: Run** `cargo test -p tomakast-ui mode_follows` — FAIL.
- [ ] **Step 3: Implement** `mode_for`, the global, `set_theme_preference`; `sync` reads the
  global (default `System`). In the daemon call `set_theme_preference(settings.appearance, cx)`
  after the first load and in the reload loop next to the hotkeys load.
- [ ] **Step 4: UI test** in `crates/ui/tests/settings.rs`
  `choosing_dark_switches_the_theme`: open the window with `[appearance] theme = "light"`,
  call `set_theme_preference(Dark)`, assert `cx.theme().mode.is_dark()`.
- [ ] **Step 5: Run** `cargo test -p tomakast-ui` — PASS.
- [ ] **Step 6: Commit** `theme: [appearance] theme applies live to every window`.

### Task 3: Plugin pages follow the plugins folder, not the plugin runtime

**Files:**
- Modify: `crates/core/src/plugin_catalog.rs` (`PluginState::NotRunning`, `unavailable` doc)
- Modify: `crates/plugin-host/src/manager.rs` (extract the catalog entry builder)
- Create: `crates/plugin-host/src/catalog.rs` (folder-only listing), export from `lib.rs`
- Modify: `crates/app/src/daemon.rs` (`start_settings` builds the catalog from the folder)
- Modify: `crates/ui/src/settings/pages/mod.rs`, `pages/plugins.rs`, `pages/general.rs`,
  `pages/home.rs`, `settings/window.rs`
- Test: `crates/plugin-host/tests/catalog.rs` (new), `crates/ui/tests/settings.rs`

**Interfaces:**
- Produces: `PluginState::NotRunning(String)` (the reason the runtime is down).
  `pub fn tomakast_plugin_host::catalog_entry(manifest: &Manifest, state: PluginState) ->
  CatalogPlugin` (moved out of `Manager::set_package_state`, which now calls it).
  `pub fn tomakast_plugin_host::list_catalog(plugins_dir: &Path, reason: &str) ->
  PluginCatalog`: every folder (same folder-name rule as the manager) read with
  `Package::read`; readable → `catalog_entry(.., NotRunning(reason))`, unreadable → the
  manager's ManifestError entry; a missing folder → empty list; a folder that cannot be listed
  → `unavailable: Some("cannot list <dir>: <error>")`. Sorted like `push_catalog`.
- `PluginCatalog.unavailable` now means only "the plugins folder cannot be listed".

- [ ] **Step 1: Failing tests** in `crates/plugin-host/tests/catalog.rs`:
  `lists_plugins_as_not_running` (a temp folder with the weather fixture → one plugin, state
  `NotRunning("no database")`, its three preferences and command), `a_missing_folder_is_an_empty_list`,
  `an_unlistable_folder_is_unavailable` (unix, mode 0o000, restore after).
- [ ] **Step 2: Run** `cargo test -p tomakast-plugin-host --test catalog` — FAIL.
- [ ] **Step 3: Implement** `catalog_entry` and `list_catalog`; `start_settings` takes the
  plugins dir and, with `plugins_unavailable = Some(reason)`, sets
  `list_catalog(&paths.plugins_dir, &reason)` instead of the empty unavailable catalog.
- [ ] **Step 4: Failing UI tests** in `crates/ui/tests/settings.rs`:
  - `plugin_pages_are_editable_when_plugins_do_not_run`: catalog with weather in
    `NotRunning("no database")`; open `plugin:weather`; `has_label("Not running: no database")`;
    pick `m/s` for Wind; the file gets `[plugins.weather] wind = "ms"`.
  - `no_plugin_pages_until_the_plugins_are_listed`: file with `[plugins.weather]` and
    `[plugins.gone]`, no catalog set → no `"Weather"`, no `"gone (not installed)"`, no
    `"Plugins"` label; after `set_catalog` with weather → both entries.
  - `no_plugin_pages_when_the_folder_cannot_be_listed`: catalog `unavailable: Some(..)` → as
    above, and no `"Plugins are not running"` text anywhere.
  - Remove: `tables_are_read_only_until_the_plugins_are_listed`, the `"Changes apply after a
    restart."` assertions (Location/Home), the `plugins-unavailable` assertions.
- [ ] **Step 5: Implement in the window**: `page_keys` adds plugin and table pages only when
  `catalog_known && catalog.unavailable.is_none()`; delete `PageKey::PluginsNotice`, the
  `restart` note parameter of `location`/`home::page`, and the "Loading plugins…" branch of
  `table_page`; `plugins.rs` state line for `NotRunning(reason)` → `"Not running: {reason}"`.
- [ ] **Step 6: Run** `cargo test -p tomakast-ui --test settings` and
  `cargo test -p tomakast-plugin-host` — PASS.
- [ ] **Step 7: Commit** `settings window: plugin pages follow the plugins folder, not the runtime`.

### Task 4: Our own layout: page model, sidebar with sections and icons, search

**Files:**
- Create: `crates/ui/src/settings/layout/mod.rs` (model), `layout/search.rs` (pure filter),
  `layout/sidebar.rs`, `layout/page.rs` (page area renderer)
- Modify: `crates/ui/src/settings/pages/*.rs` (build the model instead of the library's
  `SettingPage`/`SettingGroup`/`SettingItem`/`SettingField`)
- Modify: `crates/ui/src/settings/window.rs` (render the new layout; drop `generation`,
  `SelectIndex`, `tracked`'s title-suffix trick)
- Modify: `crates/ui/src/theme.rs` (sidebar width, title area height, sidebar row height and
  icon size tokens)
- Test: `crates/ui/src/settings/layout/search.rs` (unit), `crates/ui/tests/settings.rs`

**Interfaces:**
- Produces (in `layout/mod.rs`):
  - `pub(crate) enum Section { Core, Modules, Plugins }` with labels `"Core"`, `"Modules"`,
    `"Plugins"`.
  - `pub(crate) enum PageIcon { Named(&'static str), Item { icon: Option<tomakast_core::Icon>,
    monogram: String }, Greyed(String) }` — `Named` is a Lucide name; `Item` is drawn with
    `views::icon_element`; `Greyed` is a muted monogram.
  - `pub(crate) struct Page { key: PageKey, section: Section, title: String, icon: PageIcon,
    groups: Vec<Group> }`.
  - `pub(crate) struct Group { title: Option<String>, description: Option<String>,
    rows: Vec<Row> }`.
  - `pub(crate) struct Row { title: Option<String>, description: Option<String>,
    keywords: Vec<String>, disabled: bool, render: Rc<dyn Fn(RowOptions, &mut Window, &mut App)
    -> AnyElement> }` with `Row::new(title, render)` (title left, control right) and
    `Row::custom(render)` (full width); builder methods `description`, `keywords`,
    `disabled`. `pub(crate) struct RowOptions { pub disabled: bool }` replaces the library's
    `options.is_disabled()`.
  - `search::filter(pages: &[PageText], query: &str) -> Vec<Vec<usize>>`: for each page the
    indices of its matching groups, where `PageText { title, groups: Vec<GroupText { title,
    rows: Vec<RowText { title, keywords } > }> }` is the text-only view of the model.
- Page order and sections: Core = General, Appearance, Location, Hotkeys, About (Appearance
  and About pages are created in Task 6; until then Core has three entries); Modules = Home;
  Plugins = installed plugins by name, then not-installed tables by id. Plugin icon: first
  command icon, else monogram of the name; not-installed: `Greyed(id)`.
  Built-in icons: General `settings`, Appearance `palette`, Location `map-pin`, Hotkeys
  `keyboard`, About `info`, Home `house`.

- [ ] **Step 1: Failing unit tests** in `layout/search.rs`:
  `an_empty_query_keeps_everything`, `a_row_title_match_keeps_its_group`,
  `a_group_title_match_keeps_the_group`, `a_key_name_matches` (a row whose keywords include
  `wifi_lookup` matches `wifi_l`), `a_page_title_match_keeps_the_whole_page`,
  `matching_ignores_case`, `no_match_leaves_every_page_empty`.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui search::` — FAIL; implement `filter`; PASS.
- [ ] **Step 3: Failing UI tests** in `crates/ui/tests/settings.rs`:
  - `sidebar_has_sections_in_order`: with calc and weather installed and a `[plugins.gone]`
    table: the labels `"Core"`, `"Modules"`, `"Plugins"` exist, and the vertical order (by
    element bounds) is Core, General, Location, Hotkeys, Modules, Home, Plugins, Calculator,
    Weather, `gone (not installed)`.
  - `plugin_entries_have_no_sub_entries`: on a plugin with preferences and commands, the
    sidebar has no entries labelled `"Preferences"` or `"Commands"` (the page has the headings:
    exactly one element each, inside the page area).
  - `every_sidebar_entry_has_an_icon`: each entry element has an icon child (test selector
    `sidebar-icon` with the page title).
  - `no_modules_or_plugins_section_when_empty`: no catalog and no tables → no `"Plugins"` label.
  - `search_filters_and_shows_the_empty_state`: type `wifi` → only Location in the sidebar,
    its Wi-Fi row present; type `zzz` → `"No settings match"`; Esc → query cleared, all back.
  - `a_catalog_change_keeps_the_query`: type `wind`, set a new catalog → query still `wind`,
    Weather shown.
  - Port every existing test whose selectors referred to the library layout; behaviour
    assertions unchanged.
- [ ] **Step 4: Implement** the model, the port of the pages, the sidebar and the page area.
  Look: background `palette.panel` at alpha 1.0; sidebar rows like launcher rows (selection
  `palette.selected`, `ROW_RADIUS`, `ROW_TEXT_SIZE`); section labels with
  `views::section_label`; group titles as headings in the page (`SMALL_TEXT_SIZE`, semibold,
  muted, like section labels); row title `ROW_TEXT_SIZE` `palette.text`, description
  `MUTED_TEXT_SIZE` `palette.muted`; divider `palette.divider` between sidebar and page.
  Library controls in rows get the launcher form's styling: `Input` `.bg(palette.field)`
  `.border_color(palette.border)`, as `views/form.rs` does; selects and buttons the same where
  the kit allows; heights `CONTROL_HEIGHT`, radius `CONTROL_RADIUS`.
  Page switching keeps `shown`/`last_page`/`pending_plugin` semantics; the shown page is kept
  by `PageKey`, so no index bookkeeping is needed.
- [ ] **Step 5: Run** `cargo test -p tomakast-ui` — PASS; `cargo clippy -p tomakast-ui
  --all-targets -- -D warnings` clean.
- [ ] **Step 6: Commit** `settings window: own layout with sections, icons and search, in the launcher's look`.

### Task 5: Config file always exists; open and show it

**Files:**
- Create: `crates/platform/src/reveal.rs` (show in folder), export from `lib.rs`
- Modify: `crates/app/src/daemon.rs` (create the file at start)
- Create or modify: `crates/core/src/settings_edit.rs` (`ensure_config_file`)
- Test: `crates/core/tests/settings_edit.rs`, `crates/platform` unit test for the Linux call
  argument

**Interfaces:**
- Produces: `pub fn tomakast_core::settings_edit::ensure_config_file(path: &Path) ->
  std::io::Result<bool>` — creates the parent folders and an empty file when it is missing;
  `Ok(true)` when it created it, `Ok(false)` when it was there; never touches an existing file.
  `pub fn tomakast_platform::reveal_path(path: &Path) -> anyhow::Result<()>` — Linux:
  `org.freedesktop.FileManager1.ShowItems([file:// URI], "")` on the session bus (zbus blocking
  connection), on any D-Bus error `open_path(parent)`; macOS `open -R <path>`; Windows
  `explorer /select,<path>`. `pub fn file_uri(path: &Path) -> String` (Linux, percent-encodes
  bytes outside unreserved and `/`).

- [ ] **Step 1: Failing tests**: `ensure_creates_a_missing_file_and_folders`,
  `ensure_leaves_an_existing_file_alone` (content byte-equal after), `file_uri_escapes_spaces`
  (`/a b/c.toml` → `file:///a%20b/c.toml`).
- [ ] **Step 2: Run** the three — FAIL; implement; PASS.
- [ ] **Step 3: Daemon**: call `ensure_config_file(&paths.config_file)` before the first
  `Settings::load_or_default`; on error log `warn!` and go on (as today with a missing file).
- [ ] **Step 4: Run** `cargo test -p tomakast-core -p tomakast-platform -p tomakast-app` — PASS.
- [ ] **Step 5: Commit** `config: the daemon creates config.toml; reveal_path shows a file in its folder`.

### Task 6: Appearance, About, and the Config file row on General

**Files:**
- Create: `crates/ui/src/settings/pages/appearance.rs`, `pages/about.rs`
- Modify: `crates/ui/src/settings/pages/general.rs` (Config file row), `pages/mod.rs`
  (page keys, `SettingsPage::Appearance`/`About`), `crates/ui/src/settings/mod.rs`
  (`SettingsPage::parse` gains `"appearance"`, `"about"`)
- Test: `crates/ui/tests/settings.rs`, `crates/ui/src/settings/mod.rs` unit test

**Interfaces:**
- Consumes: Task 1 key, Task 2 `set_theme_preference` (not called by the window: the daemon's
  watch applies the written value), Task 4 model, Task 5 `ensure_config_file`, `reveal_path`,
  `tomakast_platform::open_path`.
- Produces: `SettingsPage::Appearance`, `SettingsPage::About`; Core order becomes General,
  Appearance, Location, Hotkeys, About (extend `sidebar_has_sections_in_order`).
- Appearance page: one row "Theme", a select with System/Light/Dark, the reset ↺ when the file
  holds the key, the error state for an invalid value; writes through `set_key(["appearance",
  "theme"], ..)` at once (like the city select: never pending). Uses the existing choice
  control of plugin pages.
- About page: rows "tomakast" (title) and "Version" with `env!("CARGO_PKG_VERSION")` of the
  workspace (passed from the app or read in `tomakast-ui`, same workspace version).
- General gains a row "Config file": the path text (muted, truncated from the start), buttons
  **Open** and **Show in folder** (labels `"Open config file"`, `"Show config file in
  folder"`). Each first calls `ensure_config_file` (a created file is recorded as the window's
  own write, `last_written = Some(String::new())`), then `open_path` / `reveal_path`; an error
  puts `"Could not open: {error}"` under the row (the `errors` map, key `"config-file"`).

- [ ] **Step 1: Failing tests**: `page_ids_parse` gains `appearance`, `about`;
  UI: `theme_select_writes_the_file` (pick Dark → `[appearance]\ntheme = "dark"` in the file,
  comments kept), `an_invalid_theme_is_shown_as_invalid` (`theme = "blue"` → the error line
  `"Not valid here: …"`), `theme_reset_removes_the_key`, `about_shows_the_version`,
  `config_row_shows_the_path`, `open_recreates_a_deleted_config_file` (delete the file, click
  `"Open config file"` with a test opener hook → file exists, empty, no question raised).
  Opening in tests goes through a `ConfigOpener` trait object the controller holds
  (`open(&Path) -> Result`, `reveal(&Path) -> Result`), the daemon installs the platform one;
  tests install a recording fake, including one that fails → `"Could not open: nope"`.
- [ ] **Step 2: Run** — FAIL; implement; run `cargo test -p tomakast-ui` — PASS.
- [ ] **Step 3: Commit** `settings window: Appearance and About pages; Config file row on General`.

### Task 7: The window frame

**Files:**
- Create: `crates/platform/src/kwin_buttons.rs` (parse KWin's button setting), export
- Create: `crates/ui/src/settings/frame.rs` (choice of controls, title area, controls)
- Modify: `crates/ui/src/settings/mod.rs` (`options()`), `settings/window.rs` (title area in
  the layout), `crates/ui/src/theme.rs` (window radius, control button sizes)
- Test: unit tests in both new files; `crates/ui/tests/settings.rs`

**Interfaces:**
- Produces in `tomakast-platform`: `pub enum FrameButton { Minimize, Maximize, Close }`;
  `pub fn kwin_buttons(text: &str) -> (Vec<FrameButton>, Vec<FrameButton>)` — parses the
  `[org.kde.kdecoration2]` group of a `kwinrc` text: `ButtonsOnLeft`/`ButtonsOnRight`, letters
  `I` → Minimize, `A` → Maximize, `X` → Close, others dropped, duplicates dropped; a missing key
  (or group) takes KWin's default (`MS` left, `HIAX` right);
  `pub fn kwin_button_layout() -> Option<(Vec<FrameButton>, Vec<FrameButton>)>` — `None`
  unless `XDG_CURRENT_DESKTOP` contains `KDE`; otherwise `kwin_buttons` of
  `$XDG_CONFIG_HOME/kwinrc` (default `~/.config/kwinrc`), of `""` when it cannot be read.
- Produces in `frame.rs`:
  - `pub(crate) enum ControlStyle { Breeze, Adwaita, Windows, None }`;
    `fn control_style(platform: Platform, current_desktop: &str) -> ControlStyle` — macOS →
    `None` (native traffic lights), Windows → `Windows`, Linux: `KDE` → `Breeze`, `GNOME`,
    `Unity`, `Pantheon`, `Budgie`, `X-Cinnamon`, `MATE`, `XFCE` → `Adwaita`, else `Breeze`.
  - `pub(crate) struct FrameControls { left: Vec<FrameButton>, right: Vec<FrameButton>, style:
    ControlStyle }`;
    `fn frame_controls(client_decorated: bool, layout: (Vec<FrameButton>, Vec<FrameButton>),
    supported: WindowControls, style: ControlStyle) -> FrameControls` — empty when not client
    decorated (Linux server-side) or style `None`; drops Minimize/Maximize the window manager
    does not support; Close always kept.
  - Layout source on Linux: `kwin_button_layout()` on KDE, else GPUI's `cx.button_layout()`
    converted, else Minimize, Maximize, Close on the right.

- [ ] **Step 1: Failing unit tests** (`kwin_buttons`): `defaults_when_keys_are_missing`
  (`[org.kde.kdecoration2]\n` → `([], [Min, Max, Close])`), `only_right_set`
  (`ButtonsOnRight=HIAX` → `([], [Min, Max, Close])`), `left_side_close`
  (`ButtonsOnLeft=XIA\nButtonsOnRight=` → `([Close, Min, Max], [])`), `unknown_letters_dropped`
  (`ButtonsOnRight=MSBFHLIAX` → `[Min, Max, Close]`), `other_groups_ignored`.
  (`frame_controls`): `no_controls_under_server_decorations`, `unsupported_minimize_is_dropped`,
  `close_is_always_there`, `mac_has_no_drawn_controls`; (`control_style`): `kde_is_breeze`,
  `gnome_is_adwaita`, `unknown_desktop_is_breeze`, `a_desktop_list_is_split_on_colons`
  (`"ubuntu:GNOME"` → Adwaita).
- [ ] **Step 2: Run** — FAIL; implement; PASS.
- [ ] **Step 3: Window options**: `window_decorations: Some(WindowDecorations::Client)`;
  `titlebar: TitlebarOptions { title: "Settings — tomakast", appears_transparent: true,
  traffic_light_position: Some(point(px(12.), px(14.))) }`; set the kit theme's
  `window_radius` to `theme::SETTINGS_WINDOW_RADIUS` (10 px) when the window opens.
- [ ] **Step 4: Title area**: a band of `theme::SETTINGS_TITLE_HEIGHT` over both columns: search
  field over the sidebar (left inset on macOS for the traffic lights), the shown page's title
  over the page, the controls at the outer edge(s). Empty parts: `WindowControlArea::Drag`;
  on Linux mouse-down starts `start_window_move`, double-click `zoom_window`, right-click
  `show_window_menu`. Buttons: Linux call `minimize_window` / `zoom_window` /
  `remove_window` on click; Windows mark them `WindowControlArea::Min/Max/Close`. Breeze shape:
  round 18 px buttons with Lucide `minus` / `chevron-up` (`chevrons-down-up` when maximized) /
  `x`, hover tint `palette.selected`, close hover `palette.error` fill with white glyph;
  Adwaita shape: 24 px round buttons always filled with `palette.chip`, glyph `palette.text`;
  Windows: 46×title-height rectangles, close hover `#c42b1c` fill. Labels: `"Minimize"`,
  `"Maximize"`/`"Restore"`, `"Close"`.
- [ ] **Step 5: UI test** `title_area_shows_the_page_title`: open on Location → the title area
  holds `"Location"` and the window title is still `"Settings — tomakast"`.
- [ ] **Step 6: Run** `cargo test -p tomakast-ui -p tomakast-platform` — PASS; `just check`.
- [ ] **Step 7: Commit** `settings window: its own title area and window controls`.

### Task 8: Acceptance on KDE and notes

**Files:**
- Modify: `docs/experiments/2026-11-settings-window-acceptance.md` (a new section)
- Modify (outside the repo): the shared notes file `settings-window.md` (Files page goes under
  Modules; `PluginState::NotRunning`; `list_catalog`; `ensure_config_file`)

- [ ] **Step 1:** `just build`; run the built daemon with `TOMAKAST_NO_KEYBOARD=1` and
  `TOMAKAST_HOME=/run/user/1000/tk-look` (calc and weather installed and approved, a
  `config.toml` with comments, `[plugins.gone]`); open `tomakast settings`, then
  `--page plugin:weather`, `--page appearance`, `--page about`. Screenshot each (not committed)
  in dark and light (`[appearance] theme`), and send them to the user.
- [ ] **Step 2:** Check by eye: sections and icons, no sub-entries, launcher colours, our frame
  with Breeze-like controls on the right, resize from an edge, double-click maximizes, theme
  switch applies to the launcher and the window at once; delete `config.toml` → Open recreates
  it; stop the daemon, remove the home.
- [ ] **Step 3:** Write the acceptance section (what was checked, what was not: macOS and
  Windows frames not run), update the merge note, `just check`, commit
  `settings window: acceptance of the look round`.
