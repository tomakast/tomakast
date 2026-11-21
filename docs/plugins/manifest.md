# `plugin.toml`: the plugin manifest (API 0.1)

Every plugin folder holds two files: `plugin.toml`, described here, and `plugin.wasm`, the
WebAssembly component. The manifest says what the plugin is, which commands it adds to the
launcher, which settings it has, and what it may do. The user approves the manifest's
permissions before the plugin runs.

This format is frozen with WIT 0.1. A test (`crates/plugin-api/tests/freeze.rs`) checks that
the keys `Manifest::parse` accepts are exactly the keys in the tables below. A new key needs a
new API version (the `[[widgets]]` keys were added in place on 2026-11-19; see `FROZEN.md`).

Unknown keys are errors, in every table. A manifest that does not parse is not loaded, and the
log says why.

## Example

```toml
id = "my-calc"
name = "My Calculator"
version = "0.1.0"
api = "0.1.0"
author = "Ada Lovelace"

[[commands]]
id = "calculator"
title = "Calculator"
icon = "calculator"

[[preferences]]
key = "precision"
title = "Significant digits"
kind = "text"
default = "10"

[[preferences]]
key = "thousands"
title = "Group thousands"
kind = "bool"
default = "false"

[permissions]
search = true
storage = true
clipboard = true
http = ["api.example.com", "*.cdn.example.org"]
```

## The folder

- The folder is `<data-dir>/plugins/<id>/`, and its name must equal `id`.
- `plugin.toml` is UTF-8 and at most 64 KiB.
- `plugin.wasm` is at most 32 MiB.
- Both must be regular files. A symbolic link is followed only when it resolves to a file
  inside the plugin folder.
- The approval is bound to a hash of both files together. An update that asks for no new
  permissions, keeps its `name` and `author`, and declares only widgets the user approved
  (same id and title), runs without a new prompt. An update that asks for more, renames the
  plugin, adds a widget or renames one is reviewed again. Removing the folder deletes the
  approval and the plugin's storage. A folder copied in again is reviewed as a new plugin.

## Top-level keys

| Key | Type | Required | Limits and rules |
|---|---|---|---|
| `id` | string | yes | 1–32 characters from `a-z`, `0-9` and `-`. It must equal the folder name. `apps`, `demo` and `plugins` are reserved for built-in modules. Item ids of the plugin start with `<id>/`. |
| `name` | string | yes | 1–64 characters, not only spaces. No control characters, no invisible formatting characters (Unicode category Cf), no line or paragraph separators (U+2028, U+2029). The launcher shows it on every row, view and toast of the plugin. |
| `version` | string | yes | 1–32 characters, not only spaces, no control characters. Free-form; it is shown and logged, not compared. |
| `api` | string | yes | The plugin API version: `"0.1.0"`. A component built for another version, according to the versioned names of its imports, is refused with "built for API x.y.z". |
| `author` | string | no | The same rules as `name`. Shown in the approval prompt. |
| `commands` | array of tables | no | At most 64. See `[[commands]]`. |
| `preferences` | array of tables | no | At most 32. See `[[preferences]]`. |
| `widgets` | array of tables | no | At most 16. See `[[widgets]]`. |
| `permissions` | table | no | See `[permissions]`. Missing means no permissions. |

## `[[commands]]`

A command is an item the launcher indexes for the plugin, before it calls `init`. Its item id
is `<plugin-id>/<command-id>`. Running it calls the plugin's `run` with that item and the action
id `run`. The commands belong to the host: the plugin's `replace-items` effect does not remove
them, and an item that reuses a command's id is dropped.

| Key | Type | Required | Limits and rules |
|---|---|---|---|
| `id` | string | yes | 1–32 characters from `a-z`, `0-9` and `-`; unique within the manifest. |
| `title` | string | yes | 1–128 characters, not only spaces, no control characters. What the user searches for. |
| `icon` | string | no | The name of a Lucide icon (lucide.dev), such as `calculator`; the launcher bundles the set, so it looks the same on every OS: 1–256 bytes, no control characters, no Unicode format characters. A name outside the set is looked up in the desktop's icon theme on Linux (the freedesktop theme, then `hicolor`). A name found in neither, any such name on macOS and Windows, and no icon show a monogram: the first letter of the plugin's name on a tinted square. |

```toml
[[commands]]
id = "history"
title = "Calculator History"
icon = "file-clock"
```

## `[[preferences]]`

Settings the user writes into `config.toml`, under `[plugins.<id>]`. The plugin reads them with
`preferences.get` and hears about a change through `event.preferences-changed`.

| Key | Type | Required | Limits and rules |
|---|---|---|---|
| `key` | string | yes | 1–32 characters from `a-z`, `0-9` and `-`; unique within the manifest. |
| `title` | string | yes | 1–128 characters, not only spaces, no control characters. |
| `kind` | string | yes | `"text"` or `"bool"`. A `bool` preference reaches the plugin as the text `"true"` or `"false"`. |
| `default` | string | no | The value while the user has set none. Always a TOML string, also for a `bool` preference (`default = "false"`). Without a default and a user value, `preferences.get` answers `none`. |

```toml
[[preferences]]
key = "greeting"
title = "Greeting"
kind = "text"
default = "hello"
```

And the user's value, in `config.toml`:

```toml
[plugins.my-plugin]
greeting = "good morning"
```

## `[[widgets]]`

A widget is a tile the plugin draws in the strip above the launcher's list. The plugin draws
it in its `render-widget` export. It shows the tile in answer to a query, from `search`, and
on the user's Home (`[[home.widgets]]` in `config.toml`). Every tile a plugin shows names one of these
entries. The approval prompt lists the widgets by title: a new widget, or a new title, is
reviewed again. Like a permission, an approved widget stays approved: an update that drops
it and a later one that declares it again, with the same id and title, run without a
prompt. New sizes or params ask nothing.

| Key | Type | Required | Limits and rules |
|---|---|---|---|
| `id` | string | yes | 1–32 characters from `a-z`, `0-9` and `-`; unique within the manifest. |
| `title` | string | yes | 1–64 characters, with the rules of `name`. The approval prompt shows it. |
| `sizes` | array of strings | yes, or `size` | One or more of `"quarter"`, a quarter of the strip's width; `"single"`, half of it; and `"double"`, its whole width; no repeats. The first is the default. Two quarters with the gap between them are one single. Every tile is as high as the strip. |
| `size` | string | no | A one-size shorthand: `size = "single"` is `sizes = ["single"]`. A widget with both `size` and `sizes` is refused. A Home entry or a search tile may name a `size` from the list; the launcher's "Add to Home" keeps the tile's. |
| `params` | array of tables | no | At most 8 `[[widgets.params]]`, each with a `key` (1–32 characters from `a-z`, `0-9` and `-`, unique within the widget, not `widget` or `size`) and a `title` (1–64 characters, with the rules of `name`). A tile's values are strings, at most 4 KiB together. A search tile gets them from its answer, and a Home tile from its entry in `config.toml`, where each is a string key beside `widget` and `size`; keys the widget does not declare are dropped. |

```toml
[[widgets]]
id = "clock"
title = "World clock"
sizes = ["single", "quarter"]

[[widgets.params]]
key = "zone"
title = "Time zone"
```

The user puts it on Home in `config.toml`, or from the search ("Add to Home") and edits Home in the launcher's own edit mode; see "Which tiles show" in [writing-a-plugin.md](writing-a-plugin.md):

```toml
[[home.widgets]]
widget = "my-plugin/clock"
size = "quarter"
zone = "Europe/Oslo"
```

## `[permissions]`

All permissions are off unless set. The approval prompt lists each permission the plugin asks
for, with the text in the last column. The host checks them on every import call and every
effect; see `plugin.wit` for the exact rules.

| Key | Type | Default | What it allows (and the prompt's text) |
|---|---|---|---|
| `storage` | bool | `false` | The `storage` import: a key-value store of 1 MiB. "Store data on this computer (up to 1 MiB)". |
| `clipboard` | bool | `false` | The `copy-text` effect, in answer to a user action. "Copy text to the clipboard". |
| `open` | bool | `false` | The `open` import (`open-url`, `open-path`), in a user action, and opening links in the plugin's markdown. "Open links and files". |
| `timers` | bool | `false` | The `timers` import. "Run timers in the background". |
| `index` | bool | `false` | The `replace-items` effect: items of the plugin in the launcher search. "Add items to search". |
| `search` | bool | `false` | The host calls the `search` export with every query typed in the launcher search. "See what you type in the launcher search". |
| `location` | bool | `false` | The `location` import: the computer's approximate location, rounded to 0.1° (about 11 km), with its accuracy. The launcher takes it from the user's `[location]` city, the operating system's location service, the Wi-Fi networks around (only when the user turned that on) or the time zone; a plugin cannot tell which. "Know your approximate location (within about 10 km)". |
| `http` | array of strings | `[]` | The `http` import, to these hosts only. "Make web requests to <hosts>". |

An `http` entry is a host name, matched exactly (`api.example.com`), or a wildcard over the
subdomains of a name of at least two labels (`*.example.org` allows `a.example.org` and
`a.b.example.org`, never `example.org` itself). Entries are case-insensitive and ASCII only:
write an international name as punycode (`xn--...`). Each label is 1–63 letters, digits and `-`,
and does not start or end with `-`; the whole name is at most 253 bytes. An IPv4 address is
allowed only by an entry written as that address, and IPv6 addresses never are. A name may not
resolve to a local address, except `localhost` and entries written as an IP address.

```toml
[permissions]
storage = true
http = ["api.example.com", "*.example.org"]
```
