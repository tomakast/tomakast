# Plugin on/off switches

The user turns installed plugins on and off from the settings window: a switch on each
plugin's own page, and a Plugins page listing every installed plugin with its switch.

## What "off" means

- An off plugin does not run at all. The manager does not compile, instantiate or register
  it, so none of its code runs and it makes no network calls. Its items and commands leave
  the root list, its command hotkeys do nothing (the item is not indexed), and its Home tiles
  are hidden, as for any plugin that is not registered.
- Nothing else changes: its `[plugins.<id>]` preferences, its `[hotkeys]` entries and its
  `[[home.widgets]]` entries stay in `config.toml`, so turning it back on restores everything.
- The switch takes effect live: the manager already watches `config.toml`; a changed switch
  re-syncs that plugin folder (unload when off, the normal load path when on).
- Off is not Denied. The approval records are untouched: an off plugin keeps its grant (or
  its denial), so turning it back on runs it without a new review, unless its build changed
  (the existing rules ask again then). An off plugin is not gated at all: the manager reads
  its folder for what the settings window shows (no code runs), unloads it, drops a pending
  review (the launcher stops asking about a plugin the user turned off) and marks it Off.
  Turned on, it goes through the gate as before, so a plugin waiting for review or denied
  does not start running because its switch is on.
- When `config.toml` does not load at startup, the launcher runs on the defaults, so the
  plugins it turns off run until the file is fixed (the settings window shows the file's
  error). A broken file read later changes nothing: the switches stay as they were.
- Turning a plugin on retries content that failed before (as a change of its folder does):
  turning something off and on is how users ask for another try.

## Storage

```toml
[enabled_plugins]
weather = false
```

- A top-level table keyed by plugin id, not a key inside `[plugins.<id>]`: that table holds
  the plugin's own preferences, and a plugin may declare a preference named `enabled`.
  `[enabled_plugins]` is the launcher's own, so nothing can clash with it.
- Only `false` is written. Turning a plugin on removes its key (and the table once empty), so
  the file stays clean and a new plugin is on.
- `true` is accepted and means on. A value that is not a boolean is warned about and reads as
  on. `[enabled_plugins]` that is not a table refuses the file, as `[plugins]` and
  `[hotkeys]` do.
- It is a collection in the schema (`COLLECTIONS`), so the window does not list it under "Not
  recognized". `Settings::disabled_plugins` is the set of ids the file turns off.

## Settings window

- Every installed plugin already has a page and a sidebar entry, preferences or not; the
  page now always starts with the switch, also for a plugin in review, denied, failed, not
  running or with a broken manifest.
- The switch reads the file (as every other control), so outside edits move it at once and it
  works while the plugin runtime is down. The state line reads the plugin manager's catalog,
  with the switch taken into account:
  - off: "Off";
  - otherwise the state line as before ("Waiting for your approval in the launcher", "Not
    allowed", …), which is what tells the user that an "on" switch does not mean running.
  - Nothing is written under the switch, in any state: the user asked for no description
    there; the state line above it says how the plugin stands.
- A new **Plugins** page (`plugin:` pages' parent) is the first entry of the plugins part of
  the sidebar. It lists the installed plugins in the sidebar's order (by name), each row with
  the plugin's icon, name and switch; under the name, its version and, only when something is
  wrong (waiting for approval, not allowed, failed, not running, manifest error), a few words
  saying what — not "Running", not "Off" (the switch says it); a click on the row (not on the switch)
  opens the plugin's page. A search field on top filters the rows fuzzily by name or id (the
  launcher's own matcher), best match first; "No plugins match." when nothing does. With no
  plugin installed it shows "No plugins installed." and no field. It is
  there once the plugins are listed; when the plugins folder cannot be listed, it says why.
- The Widgets page greys the tiles of an off plugin and says "Off" in their subtitle.
