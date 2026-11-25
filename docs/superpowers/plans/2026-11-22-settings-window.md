# Settings Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A separate settings window that shows and edits every key of `config.toml` one to
one: launcher, location, hotkeys, Home and every plugin's preferences.

**Architecture:** `crates/core` gets one schema of the built-in keys that the parser walks
and the window renders, plus a plugin catalog type that the plugin manager publishes. Edits
are one-key text edits that keep comments; the window writes at once and re-reads the file
on every change the daemon's existing config watch reports. The window is a kit window
(`gpui_kit::open_window`, kit Root) using the kit's `Settings` layout with custom fields
where the kit's own do not commit the way the spec needs.

**Tech Stack:** Rust 2024, GPUI through gpui-kit `=0.10.0`, `toml` / `toml_edit`,
wasm32-wasip2 for the weather plugin.

**Spec:** `docs/superpowers/specs/2026-11-22-settings-window-design.md` (read it first; this
plan argues from it and does not repeat its "what the user sees").

## Global Constraints

- `crates/core` has no GPUI dependency.
- gpui-kit stays `=0.10.0`; no new external dependencies except `tomakast-places` for
  `crates/ui` (workspace-internal).
- Every write: read the file's current text, apply one edit, `write_config` (atomic, symlink
  kept). Never write a text that `Settings::from_toml` refuses. Never rewrite a broken file.
- `Settings`' public fields and today's tests in `crates/core/tests/settings.rs`,
  `settings_edit.rs`, `home_file.rs` stay as they are and pass.
- Nothing of the settings window is created at daemon start; the launcher's open path does
  not change.
- `TOMAKAST_NO_KEYBOARD=1` opens the settings window without focus.
- Plugin manifest changes update `manifest.rs`, `crates/plugin-api/tests/freeze.rs`,
  `docs/plugins/manifest.md` and `crates/plugin-api/wit/since_v0.1.0/FROZEN.md` together.
- Do not touch `plugins/calc` (the calc converter session works on it).
- English for all copy, code comments and docs. No attribution lines in commits. Never commit
  screenshots. Run the app only through `just` recipes or the built binary with a throwaway
  `TOMAKAST_HOME` and `TOMAKAST_NO_KEYBOARD=1`; never kill processes you did not start.
- After every task: `just check` passes.

## Review Focus

1. **The window's own write echoes back through the watch.** Expect: no modal, no flicker of a
   typed field; covered in Task 13 (`own_write_raises_nothing`).
2. **A file broken by hand while a field is typed.** Expect: banner, controls disabled, typed
   text kept, nothing written; Task 13 (`broken_file_while_typing_keeps_text_and_writes_nothing`).
3. **A value of the wrong type for a built-in or plugin key (`city = 5`, `precision = "x"`).**
   Expect: the control shows the default in effect with "Not valid here", the file is not
   rewritten until the user picks; Tasks 1 and 10 (`wrong_type_is_invalid_not_missing`,
   `invalid_plugin_value_shows_error_state`).
4. **The launcher hotkey refused by the system.** Expect: file unchanged, old hotkey still
   bound, notice under the field; Task 6 (`refused_launcher_hotkey_is_not_written`).
5. **A plugin folder whose manifest does not parse, with a `[plugins.x]` table.** Expect: its
   page shows "Manifest error", values read-only, no "Remove all"; Tasks 5 and 10
   (`broken_manifest_is_in_the_catalog`, `manifest_error_page_has_no_remove_all`).

---

### Task 1: The schema of built-in keys; the parser walks it

**Files:**
- Create: `crates/core/src/settings_schema.rs`
- Modify: `crates/core/src/settings.rs` (`from_toml`, `plugin_preferences`, drop `RawSettings`/`RawLauncher`), `crates/core/src/lib.rs`
- Test: `crates/core/tests/settings_schema.rs`; existing `crates/core/tests/settings.rs` unchanged

**Interfaces:**
- Produces (all `pub`, re-exported from `tomakast_core::settings_schema`):
  - `enum ValueKind { Bool, Text, Number(NumberRange), Choice(Vec<ChoiceOption>), Hotkey, City }`
  - `struct NumberRange { min: Option<f64>, max: Option<f64>, step: Option<f64>, integer: bool }`
  - `struct ChoiceOption { value: String, title: String }`
  - `enum Page { General, Location }`
  - `enum OnInvalid { Error, Warn }` — what the parser does with a wrong-typed value.
  - `struct SettingKey { path: &'static [&'static str], kind: ValueKind, default: &'static str, title: &'static str, description: Option<&'static str>, page: Page, keywords: &'static [&'static str], on_invalid: OnInvalid }`
  - `fn builtin_keys() -> &'static [SettingKey]` (a `LazyLock`), in this order:
    `launcher.hotkey` (Hotkey, `"Alt+Space"`, Error), `launcher.autostart` (Bool, `"true"`,
    Error), `location.city` (City, `""`, Warn), `location.wifi_lookup` (Bool, `"false"`, Warn).
    Titles: "Launcher hotkey", "Start at login", "City", "Look up location by Wi-Fi".
  - `const COLLECTIONS: &[&[&str]] = &[&["hotkeys"], &["home", "widgets"], &["plugins"]]`
  - `enum KeyValue { Missing, Valid(toml::Value), Invalid(toml::Value) }`
  - `fn read_key(file: &toml::Table, key: &SettingKey) -> KeyValue`
  - `fn value_fits(kind: &ValueKind, value: &toml::Value) -> bool` (Bool: boolean; Text,
    City: string; Hotkey: string that `Hotkey::parse`s; Number: integer, or float unless
    `integer`, within min/max; Choice: string equal to an option's value)
  - `struct Unrecognized { path: Vec<String>, value: String /* TOML text */, reason: String }`
  - `fn unrecognized(file: &toml::Table) -> Vec<Unrecognized>` — top-level keys outside the
    schema's first segments and collections (except legacy `widgets`, which keeps its own
    warning and is listed with reason "no longer a setting"), and unknown keys inside
    `launcher`, `location`, `home` (other than `widgets`).

- [ ] **Step 1: Write the failing tests** in `crates/core/tests/settings_schema.rs`:

```rust
#[test]
fn every_schema_key_reaches_settings() {
    let text = r#"
[launcher]
hotkey = "Ctrl+Alt+K"
autostart = false
[location]
city = "Berlin"
wifi_lookup = true
"#;
    let s = Settings::parse(text).unwrap();
    assert_eq!(s.launcher_hotkey, Hotkey::parse("Ctrl+Alt+K").unwrap());
    assert!(!s.autostart);
    assert_eq!(s.location.city, "Berlin");
    assert!(s.location.wifi_lookup);
    // One assertion per schema key: adding a key without a line here fails.
    assert_eq!(builtin_keys().len(), 4);
}

#[test]
fn wrong_type_is_invalid_not_missing() {
    let file: toml::Table = "[location]\ncity = 5\n".parse().unwrap();
    let city = builtin_keys().iter().find(|k| k.path == ["location", "city"]).unwrap();
    assert!(matches!(read_key(&file, city), KeyValue::Invalid(toml::Value::Integer(5))));
    let s = Settings::parse("[location]\ncity = 5\n").unwrap();
    assert_eq!(s.location.city, "");
}

#[test]
fn wrong_typed_autostart_is_still_an_error() {
    assert!(Settings::parse("[launcher]\nautostart = \"yes\"\n").is_err());
}

#[test]
fn unrecognized_lists_unknown_keys_at_every_level() {
    let file: toml::Table =
        "zz = 1\nwidgets = []\n[launcher]\nfoo = true\n[location]\nbar = \"x\"\n[home]\nbaz = 2\n"
            .parse().unwrap();
    let paths: Vec<String> = unrecognized(&file).iter().map(|u| u.path.join(".")).collect();
    assert_eq!(paths, ["zz", "widgets", "launcher.foo", "location.bar", "home.baz"]);
}

#[test]
fn numbers_in_plugin_tables_reach_plugins_as_text() {
    let s = Settings::parse("[plugins.p]\nn = 10\nf = 2.5\n").unwrap();
    assert_eq!(s.plugin_preferences["p"]["n"], "10");
    assert_eq!(s.plugin_preferences["p"]["f"], "2.5");
}
```

  `from_toml` is `pub(crate)` today and the existing tests go through `Settings::load` on a
  temp file. Add `pub fn parse(text: &str) -> anyhow::Result<Settings>` (what `from_toml` is,
  made public; `load` calls it). The window (Task 9) parses the text it read with it too.

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test settings_schema` — expect compile
  failure (module missing).

- [ ] **Step 3: Implement** `settings_schema.rs` and rewrite `Settings::from_toml` to parse
  into `toml::Table`, read each built-in key with `read_key` (Invalid + `OnInvalid::Error` →
  the same error text as today: `[launcher] hotkey \`…\`` with context; `Warn` → warn and
  default), keep `[hotkeys]`, `plugins`, `home` and legacy `widgets` handling as today, log one
  warning per `unrecognized` entry, and accept integers/floats in `plugin_preferences` as
  `value.to_string()` (TOML's own rendering). A non-table `launcher` or `location` keeps
  today's behaviour (error for `launcher`, warning for `location`).

- [ ] **Step 4: Run** `cargo test -p tomakast-core` — all pass, including the untouched
  `tests/settings.rs`.

- [ ] **Step 5: Commit** — `git commit -m "core: one schema of built-in settings; the parser walks it"`

---

### Task 2: One-key edits of the file

**Files:**
- Modify: `crates/core/src/settings_edit.rs`, `crates/core/src/home_file.rs`, `crates/core/src/settings.rs` (a `pub fn home_rows`)
- Test: `crates/core/tests/settings_edit.rs`, `crates/core/tests/home_file.rs`

**Interfaces:**
- Consumes: `Settings::from_toml` (Task 1).
- Produces:
  - `pub fn set_key(text: &str, path: &[&str], value: Option<toml_edit::Value>) -> anyhow::Result<String>` — any path of 1+ segments (built-in key, unrecognized key); `None` removes it. A table left with no keys and no comments (header decor or body) is removed; a table it creates is plain `[a]` (not inline, not dotted). Refuses when the input or the output does not pass `Settings::from_toml`.
  - `pub fn set_plugin_preference(text: &str, plugin: &str, key: &str, value: Option<toml_edit::Value>) -> anyhow::Result<String>` — writes under `[plugins.<plugin>]` (a `[plugins.x]` header, not `[plugins]` with a sub-table).
  - `pub fn remove_plugin_table(text: &str, plugin: &str) -> anyhow::Result<String>`
  - `HomeEdit::SetParam { i: usize, key: String, value: Option<String> }` — changes only that value of entry `i`; `None` removes the key.
  - `pub fn home_rows(text: &str) -> anyhow::Result<Vec<Option<HomeEntry>>>` in `settings.rs` (wraps `home_elements`; unreadable entries are `None`).

- [ ] **Step 1: Write the failing tests** (in `tests/settings_edit.rs`):

```rust
const FILE: &str = "# my launcher\n[launcher]\n# fast\nhotkey = \"Alt+Space\"\n\n[location]\ncity = \"Berlin\"\n";

#[test]
fn set_key_changes_one_value_and_keeps_comments() {
    let out = set_key(FILE, &["launcher", "hotkey"], Some("Ctrl+Space".into())).unwrap();
    assert_eq!(out, FILE.replace("\"Alt+Space\"", "\"Ctrl+Space\""));
}

#[test]
fn removing_the_last_key_removes_a_plain_table() {
    let out = set_key(FILE, &["location", "city"], None).unwrap();
    assert!(!out.contains("[location]"), "{out}");
    assert!(out.starts_with("# my launcher\n[launcher]\n# fast\n"));
}

#[test]
fn removing_the_last_key_keeps_a_commented_table() {
    let text = "[location]\n# where I live\ncity = \"Berlin\"\n";
    let out = set_key(text, &["location", "city"], None).unwrap();
    assert!(out.contains("[location]") && out.contains("# where I live"), "{out}");
}

#[test]
fn set_key_creates_a_plain_table() {
    let out = set_key("", &["location", "wifi_lookup"], Some(true.into())).unwrap();
    assert_eq!(out, "[location]\nwifi_lookup = true\n");
}

#[test]
fn set_key_refuses_a_broken_file_and_a_broken_result() {
    assert!(set_key("[launcher\n", &["launcher", "autostart"], Some(true.into())).is_err());
    assert!(set_key("", &["launcher", "hotkey"], Some("Ctrl+".into())).is_err());
}

#[test]
fn plugin_preference_round_trip() {
    let out = set_plugin_preference("", "weather", "wind", Some("ms".into())).unwrap();
    assert_eq!(out, "[plugins.weather]\nwind = \"ms\"\n");
    let out = set_plugin_preference(&out, "weather", "wind", None).unwrap();
    assert_eq!(out, "");
    let out = remove_plugin_table("[plugins.x]\na = \"1\"\n[plugins.y]\nb = 2\n", "x").unwrap();
    assert_eq!(out, "[plugins.y]\nb = 2\n");
}
```

  And in `tests/home_file.rs`: `set_param_changes_only_that_value` (an entry with `city =
  "Oslo"` and a comment above it; `SetParam { i: 0, key: "city", value: Some("Rome") }` changes
  only that string), `set_param_none_removes_the_key`.

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test settings_edit --test home_file` — fail.
- [ ] **Step 3: Implement** the functions with `toml_edit`, reusing the comment-carrying
  approach of `set_item_hotkey` where a removed key had comment lines above it (they move to
  the next key or stay under the header).
- [ ] **Step 4: Run** `cargo test -p tomakast-core` — pass.
- [ ] **Step 5: Commit** — `git commit -m "core: one-key edits of config.toml that keep comments"`

---

### Task 3: Plugin preference kinds `choice` and `number`, and `description`

**Files:**
- Modify: `crates/plugin-host/src/manifest.rs`, `crates/plugin-api/tests/freeze.rs` (if its key list is not read from the doc), `docs/plugins/manifest.md`, `crates/plugin-api/wit/since_v0.1.0/FROZEN.md`
- Test: `crates/plugin-host/src/manifest.rs` (its `#[cfg(test)]` module)

**Interfaces:**
- Consumes: `ValueKind`, `ChoiceOption`, `NumberRange` (Task 1).
- Produces:
  - `PrefKind::{Text, Bool, Choice, Number}`
  - `PreferenceDecl { key, title, kind, default, description: Option<String>, options: Vec<ChoiceOptionDecl>, min: Option<f64>, max: Option<f64>, step: Option<f64> }`, `ChoiceOptionDecl { value: String, title: String }` (serde, `deny_unknown_fields`)
  - `impl PreferenceDecl { pub fn value_kind(&self) -> ValueKind }` (`integer` = all of min/max/step present and whole, or default parses as an integer and none is fractional)

Rules (from the spec's table): `description` 1–256 chars, no control chars; `options` only for
`choice` and required there, 2–32, values unique, value chars `a-z0-9-_.`, titles by the
`name` rule; `min`/`max`/`step` only for `number`, `min <= max`, `step > 0`; `default` must be
an option value (`choice`) or a number within bounds (`number`).

- [ ] **Step 1: Write the failing tests:** `choice_preference_parses`,
  `number_preference_parses`, and one refusal test per rule:
  `choice_without_options_is_refused`, `choice_with_one_option_is_refused`,
  `duplicate_option_values_are_refused`, `options_on_text_are_refused`,
  `min_above_max_is_refused`, `zero_step_is_refused`, `choice_default_not_an_option_is_refused`,
  `number_default_out_of_bounds_is_refused`, `long_description_is_refused`. Each asserts
  `Manifest::parse(..)` is `Err` and the message names the preference key.
- [ ] **Step 2: Run** `cargo test -p tomakast-plugin-host manifest` — fail.
- [ ] **Step 3: Implement** the fields and checks in `Manifest::parse`'s preference loop;
  document them in `manifest.md`'s `[[preferences]]` table (one row per new key, plus a
  "`choice`" example) and note the in-place edit with today's date in `FROZEN.md`.
- [ ] **Step 4: Run** `cargo test -p tomakast-plugin-host` and `cargo test -p tomakast-plugin-api` (the freeze test) — pass.
- [ ] **Step 5: Commit** — `git commit -m "plugin manifest: choice and number preferences, descriptions"`

---

### Task 4: Weather: temperature, wind and pressure as choices

**Files:**
- Modify: `plugins/weather/plugin.toml`, `plugins/weather/src/{lib.rs,api.rs,tile.rs,places.rs,measures.rs}`
- Test: the weather crate's own tests

**Interfaces:**
- Consumes: Task 3's manifest kinds.
- Produces in `plugin.toml`:

```toml
[[preferences]]
key = "temperature"
title = "Temperature"
kind = "choice"
default = "celsius"
options = [{ value = "celsius", title = "°C" }, { value = "fahrenheit", title = "°F" }]

[[preferences]]
key = "wind"
title = "Wind speed"
kind = "choice"
default = "kmh"
options = [{ value = "kmh", title = "km/h" }, { value = "ms", title = "m/s" }, { value = "mph", title = "mph" }, { value = "kn", title = "knots" }]

[[preferences]]
key = "pressure"
title = "Pressure"
kind = "choice"
default = "hpa"
options = [{ value = "hpa", title = "hPa" }, { value = "mmhg", title = "mm Hg" }]
```

- In code: `Units { temperature: TempUnit, wind: WindUnit }` replaces the `Metric/Imperial`
  enum; `TempUnit::{Celsius, Fahrenheit}`, `WindUnit::{Kmh, Ms, Mph, Kn}`. The forecast URL
  adds `&temperature_unit=fahrenheit` for °F and `&wind_speed_unit=<ms|mph|kn>` unless km/h.
  Wind labels: `km/h`, `m/s`, `mph`, `kn`. Cache keys use both units (e.g. `"c-kmh"`). The
  old `fahrenheit` / `mmhg` keys are not read. Unknown values read as the default.

- [ ] **Step 1: Update the failing tests:** existing `Units::Imperial` tests become
  `Units { temperature: Fahrenheit, wind: Mph }`; add `forecast_url_metres_per_second`
  (URL ends with `&wind_speed_unit=ms`, no `temperature_unit`), `forecast_url_knots_with_fahrenheit`,
  `wind_label_per_unit`, `unknown_wind_value_reads_as_kmh`.
- [ ] **Step 2: Run** `cargo test --manifest-path plugins/weather/Cargo.toml` — fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the weather tests and `just build` (the wasm build) — pass; the manifest
  parses (`cargo test -p tomakast-plugin-host` has a test loading `plugins/*/plugin.toml`; if
  not, add `weather_manifest_parses` there).
- [ ] **Step 5: Commit** — `git commit -m "weather: temperature, wind and pressure units as choices"`

---

### Task 5: The plugin catalog

**Files:**
- Create: `crates/core/src/plugin_catalog.rs`
- Modify: `crates/core/src/lib.rs`, `crates/plugin-host/src/manager.rs`, `crates/app/src/daemon.rs` (forward the action; nothing else yet)
- Test: `crates/plugin-host/src/manager.rs` tests

**Interfaces:**
- Consumes: `ValueKind` (Task 1), `PreferenceDecl::value_kind` (Task 3), `WidgetDecl` (existing).
- Produces (in `tomakast_core::plugin_catalog`, re-exported):
  - `struct PluginCatalog { plugins: Vec<CatalogPlugin>, unavailable: Option<String> }` (`Default` = empty, available)
  - `struct CatalogPlugin { id: String, name: String, version: String, author: Option<String>, state: PluginState, preferences: Vec<PluginPreference>, commands: Vec<CatalogCommand>, widgets: Vec<WidgetDecl> }`
  - `enum PluginState { Running, Review, Denied, Failed(String), ManifestError(String) }`
  - `struct PluginPreference { key: String, title: String, description: Option<String>, kind: ValueKind, default: Option<String> }`
  - `struct CatalogCommand { id: String, title: String, icon: Option<String> }`
  - `ManagerAction::Catalog(PluginCatalog)`, sent after every `scan`, `folder_changed` and
    `on_event` whose catalog differs from the last one sent. A folder whose `Package::read`
    fails is a `CatalogPlugin` with `id` = folder name, `name` = folder name, empty
    `version`, and `ManifestError(message)`. Sorted by `name`.

- [ ] **Step 1: Write the failing tests:** `catalog_lists_running_review_and_denied`
  (three plugin folders: one approved, one pending, one denied — states as named),
  `broken_manifest_is_in_the_catalog` (a folder with `plugin.toml` = `id = 1`: state
  `ManifestError`, id = folder name), `catalog_is_sent_only_when_it_changes` (a second
  `folder_changed` with nothing changed sends no `Catalog`). Reuse the manager tests' existing
  fixture helpers.
- [ ] **Step 2: Run** `cargo test -p tomakast-plugin-host manager` — fail.
- [ ] **Step 3: Implement:** the manager keeps `catalog: BTreeMap<String, CatalogPlugin>`
  updated in `sync` (each branch sets the entry's state) and `removed`; a helper builds the
  action when it differs. The daemon's action loop stores it for Task 9 (a `match` arm that,
  for now, logs at debug).
- [ ] **Step 4: Run** `cargo test -p tomakast-plugin-host` and `just check` — pass.
- [ ] **Step 5: Commit** — `git commit -m "plugin host: publish a catalog of installed plugins"`

---

### Task 6: The launcher hotkey through accept-then-write; notices with an origin

**Files:**
- Modify: `crates/core/src/hotkeys.rs`, `crates/app/src/hotkeys.rs`, `crates/app/src/daemon.rs` (`RecorderRequests`, `apply_outcomes`), `crates/ui/src/hotkeys.rs` (trait), the one launcher call site of `assign` (`crates/ui/src/launcher_view.rs` / `views/hotkey_recorder.rs`)
- Test: `crates/core/tests` (hotkeys), `crates/app/src/hotkeys.rs` tests

**Interfaces:**
- Produces:
  - `tomakast_core::hotkeys::HotkeyTarget { Launcher, Item(ItemId) }`
  - `tomakast_core::hotkeys::HotkeyOrigin { Launcher, Settings }`
  - `pub fn check_launcher(assigned: &[(ItemId, Hotkey)], hotkey: &Hotkey) -> Result<(), Refusal>` (usable, not `TakenBy`)
  - `HotkeyTable::check_target(&self, target: &HotkeyTarget, hotkey: &Hotkey) -> Result<(), Refusal>`
  - `trait HotkeyRequests { fn assign(&self, target: HotkeyTarget, hotkey: Option<Hotkey>, origin: HotkeyOrigin); }` (the launcher passes `HotkeyOrigin::Launcher`; `None` for `Launcher` target means "reset to the default `Alt+Space`")
  - `Hotkeys::request(&mut self, target: &HotkeyTarget, hotkey: Option<Hotkey>, origin: HotkeyOrigin, title: …)`; `Pending` stores `target` and `origin`; for `Launcher` it sends `SetKind::Launcher` and `save` writes `set_key(&text, &["launcher", "hotkey"], …)` (`None` → removes the key).
  - `Outcome::Notice { text: String, origin: HotkeyOrigin }` (file-driven notices use `Launcher`).

- [ ] **Step 1: Write the failing tests** in `crates/app/src/hotkeys.rs` (its fake registrar):
  `launcher_hotkey_is_written_after_the_system_accepts`, `refused_launcher_hotkey_is_not_written`
  (registrar refuses → file byte-identical, `applied[Launcher]` unchanged, one `Notice` with
  `origin == Settings`), `launcher_hotkey_taken_by_an_item_is_refused` (`Refusal::TakenBy`),
  `item_notice_carries_its_origin`. In core: `check_launcher_refuses_an_items_hotkey`.
- [ ] **Step 2: Run** `cargo test -p tomakast-app hotkeys` and `cargo test -p tomakast-core hotkeys` — fail.
- [ ] **Step 3: Implement.** `apply_outcomes` routes `Notice { origin: Settings }` to
  `SettingsController::hotkey_notice` once Task 9 exists; until then, to the launcher toast
  as today (leave a `// Task 9` free arm: the match is exhaustive over origin).
- [ ] **Step 4: Run** `just check` — pass; the launcher's recorder tests in `crates/ui/tests/hotkeys.rs` still pass.
- [ ] **Step 5: Commit** — `git commit -m "hotkeys: the launcher hotkey is written only once the system took it"`

---

### Task 7: Daemon: autostart follows the file; `tomakast settings`

**Files:**
- Modify: `crates/app/src/daemon.rs`, `crates/app/src/ipc.rs`, `crates/app/src/cli.rs`
- Test: those files' tests

**Interfaces:**
- Produces:
  - In the daemon's config reload loop: `autostart_follow(previous: bool, settings: &Settings, home_override: bool) -> Option<bool>` (pure; `Some(enabled)` when it changed and `TOMAKAST_HOME` is not set) and the call to `tomakast_platform::set_autostart`.
  - A hook in the same loop, before parsing: `on_config_changed: Rc<dyn Fn(&mut App)>` (set by Task 9 to `SettingsController::config_changed`). The broken-file `continue` stays after the hook.
  - `Request::Settings(Option<String>)`, encoded `settings` / `settings <page>`; `Command::Settings { #[arg(long)] page: Option<String> }` sends it. The daemon's handler calls the hook set by Task 9 (until then logs "settings window not built").
- [ ] **Step 1: Write the failing tests:** `autostart_follows_a_reload` (true→false gives
  `Some(false)`; unchanged gives `None`; `home_override` gives `None`),
  `settings_request_round_trips` (`Request::decode(&Request::Settings(Some("plugin:weather".into())).encode())`),
  `cli_parses_settings_page`.
- [ ] **Step 2: Run** `cargo test -p tomakast-app` — fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p tomakast-app` — pass.
- [ ] **Step 5: Commit** — `git commit -m "daemon: autostart follows config.toml; settings request"`

---

### Task 8: Ways in: the Settings item, Ctrl+, and "Plugin Settings"

**Files:**
- Create: `modules/settings/{Cargo.toml,src/lib.rs}` (mirrors `modules/home`)
- Modify: root `Cargo.toml` (member + workspace dep), `crates/core/src/effect.rs`, `crates/plugin-host/src/caps.rs` (refuse it from plugins, as `EditHome`), `crates/ui/src/stack.rs`, `crates/ui/src/launcher_view.rs`, `crates/ui/src/views/action_panel.rs`, `crates/app/src/daemon.rs` (register the module)
- Test: `modules/settings` unit test, `crates/plugin-host/src/caps.rs` test, `crates/ui/tests/action_panel.rs`

**Interfaces:**
- Produces:
  - `Effect::OpenSettings(Option<String>)` — internal; the page id as in Task 7. Honoured from
    a window reply and from a daemon run; the launcher hides first.
  - Module `settings`, item `settings/open`, title "Settings", subtitle "Launcher, plugins,
    hotkeys and Home", keywords `["settings", "preferences", "config", "options"]`, icon
    `settings`, one action `open` → `[Effect::OpenSettings(None)]`.
  - Ctrl+, (Cmd+, on macOS) in the launcher → the same.
  - `PanelEntry::PluginSettings(String)` titled "Plugin Settings", on a root row whose
    provider is a plugin in the catalog with preferences or commands → `OpenSettings(Some("plugin:<id>"))`.
    The launcher reads the catalog from the `SettingsController` global (Task 9); until then the
    entry's condition reads an empty catalog and the panel test is written against Task 9's
    controller (do this step after Task 9 if executing strictly in order).
  - Until Task 9, `stack.rs` routes `OpenSettings` to a `tracing::info!`.
- [ ] **Step 1: Write the failing tests:** `open_runs_open_settings` (module), `open_settings_is_not_a_plugin_effect` (caps), `plugin_row_offers_plugin_settings` (panel; after Task 9).
- [ ] **Step 2–4:** run, implement, run (`just check`).
- [ ] **Step 5: Commit** — `git commit -m "settings item, Ctrl+, and Plugin Settings open the settings window"`

---

### Task 9: The window shell, the controller, General and Location

**Files:**
- Create: `crates/ui/src/settings/{mod.rs,window.rs,fields.rs,pages/mod.rs,pages/general.rs}`
- Modify: `crates/ui/src/lib.rs` (export), `crates/ui/Cargo.toml` (`tomakast-places`), `crates/places/src/cities.rs` (`pub fn names(&self) -> impl Iterator<Item = &str>`), `crates/app/src/daemon.rs` (create the global, wire Tasks 5–8 hooks), `crates/ui/src/stack.rs` (`OpenSettings`)
- Test: `crates/ui/tests/settings.rs`

**Interfaces:**
- Consumes: Tasks 1, 2, 5, 6, 7, 8.
- Produces:
  - `pub enum SettingsPage { General, Location, Hotkeys, Home, Plugin(String) }` with `pub fn parse(id: &str) -> Option<SettingsPage>` (`general`, `location`, `hotkeys`, `home`, `plugin:<id>`).
  - `pub struct SettingsController` (a GPUI `Global`) with:
    `pub fn new(config_file: PathBuf, core: Rc<RefCell<Core>>) -> Self`,
    `pub fn open(&mut self, page: Option<SettingsPage>, cx: &mut App)` (one instance: focuses and switches page when open),
    `pub fn set_catalog(&mut self, catalog: PluginCatalog, cx: &mut App)`,
    `pub fn catalog(&self) -> &PluginCatalog`,
    `pub fn set_hotkey_table(&mut self, table: HotkeyTable, cx: &mut App)`,
    `pub fn set_hotkey_requests(&mut self, requests: Rc<dyn HotkeyRequests>)`,
    `pub fn hotkey_notice(&mut self, text: String, cx: &mut App) -> bool` (false when no window: the caller toasts),
    `pub fn config_changed(&mut self, cx: &mut App)`,
    `pub fn is_recording(&self, cx: &App) -> bool`, `pub fn recorded_press(&mut self, hotkey: Hotkey, cx: &mut App)`,
    `pub fn window(&self) -> Option<AnyWindowHandle>`.
  - `SettingsWindow` (view): owns `FileState { text: String, settings: Result<Settings, String>, file: Option<toml::Table> }` read from disk, `last_written: Option<String>`, the field states, and builds `gpui_kit::component::setting::Settings` pages each render.
  - `fields.rs`: `TextField`, `NumberField`, `CityField`, `HotkeyField` — each an entity with its own input state, rendered through `SettingField::render`; text/number commit on Enter or blur, Esc restores; each exposes `fn pending(&self, cx) -> Option<String>` and `fn set_from_file(&mut self, value, window, cx)` (ignored while pending). Switch and choice use the kit's `SettingField::switch` / `dropdown`.
  - Window options: `gpui_kit::open_window`, decorated, resizable, about 900×640, title "Settings — tomakast", `app_id` `dev.tomakast.Tomakast.Settings`, `focus: false` under `TOMAKAST_NO_KEYBOARD=1`.
  - Every write: `SettingsWindow::write(&mut self, edit: impl FnOnce(&str) -> anyhow::Result<String>, cx)` reads the file, applies, `write_config`, sets `last_written`, re-reads. An error shows under the field.
- **First step of the task is a mount check:** the kit `Settings` layout has only been read,
  never run. If it cannot render in a `TestAppContext` window opened through
  `gpui_kit::open_window`, or custom fields cannot be placed in it, stop and report: the
  fallback (our own sidebar and rows in the same window) changes Tasks 9–12's look and needs
  the main session's word.
- [ ] **Step 1: Write the failing tests** in `crates/ui/tests/settings.rs` (headless, temp
  `config.toml`, kit Root through `gpui_kit::open_window`):
  `window_opens_once_and_switches_page` (two `open` calls → one window; second with
  `Location` shows the City item), `switch_writes_at_once` (toggle "Start at login" → file has
  `autostart = false`, comments kept), `reset_removes_the_key`,
  `city_automatic_removes_the_key`, `text_field_writes_only_on_enter_or_blur` (type into a
  number/text field: file unchanged; Enter → written), `esc_restores_the_file_value`,
  `broken_file_disables_controls_and_shows_the_banner` (banner text starts with
  "config.toml has an error:"), `unrecognized_keys_are_listed_on_general` (`zz = 1` → a
  "Not recognized" row with a remove button that removes it),
  `invalid_builtin_value_shows_error_state` (`city = 5` → "Not valid here: 5; the default is
  used").
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test settings` — fail.
- [ ] **Step 3: Implement**, including the daemon wiring: create the `SettingsController`
  global after `LauncherController`; Task 7's hook calls `config_changed`; `Request::Settings`
  calls `open`; `ManagerAction::Catalog` calls `set_catalog`; when plugins cannot load at all,
  `set_catalog(PluginCatalog { plugins: vec![], unavailable: Some(reason) })`;
  `Outcome::Table` also calls `set_hotkey_table`; `Notice { origin: Settings }` goes to
  `hotkey_notice` and falls back to the toast when it returns false.
- [ ] **Step 4: Run** `just check` — pass.
- [ ] **Step 5: Commit** — `git commit -m "settings window: shell, General and Location"`

---

### Task 10: Plugin pages

**Files:**
- Create: `crates/ui/src/settings/pages/plugins.rs`
- Test: `crates/ui/tests/settings.rs`

**Interfaces:**
- Consumes: catalog (Task 5), `set_plugin_preference` / `remove_plugin_table` (Task 2), fields (Task 9), `HotkeyField` with `HotkeyTarget::Item` (Task 6).
- Produces: one page per catalog plugin (by name), then a greyed page per `[plugins.x]` with
  no catalog entry. Page content and copy as in the spec's "A plugin's page", "Settings of
  plugins that are not installed" and "Plugins the window cannot fully see". Choice →
  dropdown with option titles; Number → `NumberField` honouring min/max/step; a file value
  that does not fit the kind → error state "Not valid here: <value>".
- [ ] **Step 1: Write the failing tests:** `plugin_page_shows_each_kind` (a catalog plugin
  with text/bool/choice/number prefs → four controls with their titles and descriptions),
  `choice_writes_the_option_value`, `number_writes_a_toml_number` (`precision = 12`, not a
  string), `invalid_plugin_value_shows_error_state`, `undeclared_keys_are_read_only_with_remove`,
  `not_installed_plugin_has_remove_all`, `manifest_error_page_has_no_remove_all`,
  `plugins_not_running_makes_tables_read_only` (`unavailable: Some(..)` → notice
  "Plugins are not running: …", no editable control), `command_hotkey_field_sends_a_request`
  (a fake `HotkeyRequests` records `(Item(<plugin>/<cmd>), Some(hotkey), Settings)`).
- [ ] **Steps 2–4:** run (fail), implement, `just check`.
- [ ] **Step 5: Commit** — `git commit -m "settings window: plugin pages"`

---

### Task 11: The Hotkeys page and one recorder for both windows

**Files:**
- Create: `crates/ui/src/settings/pages/hotkeys.rs`
- Modify: `crates/ui/src/views/hotkey_recorder.rs` (move the refusal wording into a plain `pub(crate) fn refusal_text(table: &HotkeyTable, target: &HotkeyTarget, hotkey: &Hotkey, title_of: &dyn Fn(&ItemId) -> String) -> Option<String>`), `crates/ui/src/window.rs` (`LauncherController::is_recording` stays launcher-only), `crates/app/src/daemon.rs` (`apply_outcomes`: a press goes to the settings recorder when `SettingsController::is_recording`, else the launcher's, else runs)
- Test: `crates/ui/tests/settings.rs`, `crates/ui/tests/hotkeys.rs` (unchanged, must pass)

**Interfaces:**
- Produces: the page as in the spec's "Hotkeys": launcher row first, then every `[hotkeys]`
  entry from `item_hotkeys(&settings)` plus greyed rows for its `dropped` list ("Not used:
  <item> already has <hotkey>", remove = `set_key(&["hotkeys", "<key>"], None)`), item
  title/icon from `Core::item_title` (id greyed when not indexed), state from the table's
  `HotkeyState` ("Applies after a restart", "Refused: <reason>", "Off"), "Add hotkey…"
  opening a search over `Core::search` results whose picked item gets a focused empty
  `HotkeyField`. Notices from `hotkey_notice` show under the field that made the request.
- [ ] **Step 1: Write the failing tests:** `hotkeys_page_lists_launcher_then_items`,
  `dropped_entry_is_greyed_with_remove`, `refused_state_is_shown`,
  `taken_hotkey_is_refused_in_the_field` (same wording as the launcher's recorder:
  `Already set on "…". Clear it there first.`), `global_press_reaches_the_settings_recorder`
  (`recorded_press` while recording fills the field), `add_hotkey_picks_an_item`,
  `notice_shows_under_the_field`.
- [ ] **Steps 2–4:** run (fail), implement, `just check` (the launcher's `tests/hotkeys.rs` passes unchanged).
- [ ] **Step 5: Commit** — `git commit -m "settings window: Hotkeys page; one recorder logic for both windows"`

---

### Task 12: The Home page

**Files:**
- Create: `crates/ui/src/settings/pages/home.rs`
- Test: `crates/ui/tests/settings.rs`

**Interfaces:**
- Consumes: `home_rows` and `edit_home` with `Move`, `Resize`, `Remove`, `SetParam` (Task 2); widget decls from the catalog (Task 5).
- Produces: the page as in the spec's "Home" and "Everything else": rows in file order;
  unreadable entries greyed and inert; a tile whose plugin is not in the catalog greyed with
  id/size/params as text, movable and removable; declared params as `TextField`s (commit
  writes `SetParam`); undeclared params read-only with remove; size dropdown of the widget's
  sizes plus "Default" (removes `size`); up/down; remove. The note "Add tiles from the
  launcher's search: Add to Home." `edit_home`'s `Changed` error re-reads the file and shows
  "config.toml changed: try again" on the row.
- When the catalog's `unavailable` is set, the Home page and the Location page (Task 9's) show
  "Changes apply after a restart" at the top (their live path is the plugin manager's watch).
- [ ] **Step 1: Write the failing tests:** `plugins_not_running_notes_restart_on_location_and_home`, `home_rows_in_file_order_with_unreadable_greyed`,
  `move_down_writes_move`, `size_default_removes_the_size_key`, `param_commit_writes_set_param`,
  `undeclared_param_is_read_only`, `uninstalled_tile_is_movable_not_editable`.
- [ ] **Steps 2–4:** run (fail), implement, `just check`.
- [ ] **Step 5: Commit** — `git commit -m "settings window: Home page"`

---

### Task 13: Outside edits and the conflict modal

**Files:**
- Create: `crates/ui/src/settings/conflict.rs`
- Modify: `crates/ui/src/settings/window.rs`
- Test: `crates/ui/tests/settings.rs`

**Interfaces:**
- Consumes: `config_changed` (Task 9), every field's `pending` / `set_from_file`.
- Produces: `SettingsWindow::reload_from_disk(&mut self, window, cx)`:
  1. read the text; if it equals `last_written`, do nothing;
  2. parse; a broken file → banner and disabled controls, typed text kept, nothing written;
  3. for each field: no pending → `set_from_file`; pending and the file's value of that key
     changed since the window last read it → open the kit dialog (`window.open_dialog`) with
     "config.toml changed while you were editing <title>." / "Keep mine" (writes the typed
     value onto the new file) / "Use the file's" (drops it); pending and that key unchanged →
     leave it. One dialog at a time; a second conflicting key queues behind it.
- [ ] **Step 1: Write the failing tests:** `outside_edit_updates_controls_without_a_question`,
  `own_write_raises_nothing` (write through a switch, then deliver `config_changed` → no
  dialog, no field reset), `outside_edit_of_the_typed_key_asks`,
  `keep_mine_writes_on_top_of_the_new_file` (the outside edit of another key is kept too),
  `use_the_files_drops_the_typed_text`, `outside_edit_of_another_key_leaves_the_typed_field`,
  `broken_file_while_typing_keeps_text_and_writes_nothing`.
- [ ] **Steps 2–4:** run (fail), implement, `just check`.
- [ ] **Step 5: Commit** — `git commit -m "settings window: outside edits and the conflict question"`

---

### Task 14: Acceptance

**Files:**
- Create: `docs/experiments/2026-11-settings-window-acceptance.md`
- Modify: `docs/plugins/manifest.md` only if Task 3 missed something; the weather plugin's docs comment in `plugins/weather/src/lib.rs` (units)

- [ ] **Step 1:** `just check` — passes.
- [ ] **Step 2: Headless smoke run:** `just build`, then the built daemon with a throwaway
  `TOMAKAST_HOME` (calc and weather installed and approved as `just try approve` does, a
  `config.toml` with comments, `[location]`, two `[hotkeys]`, three Home entries, an unknown
  key, a `[plugins.gone]` table) and `TOMAKAST_NO_KEYBOARD=1`. Run `tomakast settings`,
  `tomakast settings --page plugin:weather`, `tomakast run settings/open`; read the log for
  errors; hand-edit the file and check the log shows the reload. Stop only the daemon you
  started.
- [ ] **Step 3: Write the acceptance note:** what ran, what passed, what could not be seen
  headless, macOS/Windows built and type-checked only (`cargo check --target
  x86_64-pc-windows-gnu` / `aarch64-apple-darwin` if the targets are installed; say so if
  not), and a checklist for the user's `just try` covering each spec "What the user sees"
  bullet and the two modal cases.
- [ ] **Step 4: Commit** — `git commit -m "Settings window acceptance note"`
