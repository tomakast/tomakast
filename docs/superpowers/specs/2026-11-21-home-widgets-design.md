# Home: the user's own widgets, widget parameters, an edit mode, and the weather widgets

Date: 2026-11-21. Status: agreed with the user in chat, this document awaiting their approval.
It replaces the "always" set of `2026-11-19-widgets-design.md` (a plugin choosing its own
always tiles with `show-widgets`, and `[widgets] order/hidden`); the rest of that design (the
strip's look, frames, the canvas, condition tiles, the keyboard rules) stands.

## Why

The tiles at the empty query, **Home**, should be the user's: only what they chose, in their
order. A plugin deciding which of its tiles show there turns the strip into something the
user cannot control. The user builds Home from the search: any tile a search shows can be
added, and keeps what it is about (Barcelona's weather stays Barcelona's). Arranging Home
works like the widget and app screens of Apple's systems: the tiles jiggle, and can be moved,
removed and resized, by keyboard first, by mouse too.

The weather plugin is the first heavy user: a real "today" (morning, afternoon, evening,
night) and one widget per measure (humidity, wind, UV, pressure, precipitation, air quality,
pollen, daylight), each found by its word in the search.

## Goals

- Home shows exactly the tiles of `[[home.widgets]]` in `config.toml`, in that order, and
  nothing else. Duplicates are allowed.
- Every tile a search shows has the launcher's action "Add to Home".
- A widget declares **parameters** (named string values, e.g. `city`); a search tile carries
  their values, and a Home tile is the widget plus its size plus those values. Nothing else is
  remembered about it.
- A widget may offer several **sizes**; the user picks one in the edit mode.
- An edit mode for Home, fully usable by keyboard, also by mouse.
- Weather: `today` in four parts of the day, in three sizes; eight measure widgets; search
  words for each.

## Non-goals (later, if at all)

- Editing a Home tile's parameters (changing Barcelona to Lisbon): for now, add the new tile
  and remove the old one. To be designed separately.
- A plugin hiding one of its Home tiles (a player with nothing playing): a tile always shows,
  and draws a placeholder (or the last thing played) when it has nothing. Added later only if
  a real need shows up.
- "Save as my city": dropped. A Home tile with the city as its parameter does the same, and
  plugins no longer manage Home. The launcher's own `[location] city` stays as it is.
- A grid of several rows: Home stays one scrolling row.
- Tiles on Home on the first run: Home starts empty, so there is no strip until the user adds
  a tile.

## Anti-goals (a failure even if it works)

- Any way for a plugin to put a tile on Home, or keep one there, without the user.
- An edit mode that needs the mouse for anything.
- A tile lost to a stray key with no way back.
- Five tiles of one city making ten requests.
- The launcher rewriting the user's `config.toml` beyond the Home list: comments, order and
  other settings stay as written.

## Approaches (decided)

1. **What a Home tile remembers.** Chosen: *declared parameters replace the opaque payload*.
   Every widget lists its parameters in `plugin.toml`; a search tile carries their values;
   Home stores them. Rejected: keeping the payload next to parameters (two places for a
   tile's data, and the user has to know which one survives).
2. **The weather plugin.** Chosen: *one plugin with ten widgets*, one forecast request and
   one air-quality request per place, shared by every tile of that place, one approval.
   Rejected: a separate "air" plugin (duplicates the city search, the location and the
   cache, for an approval that the user gives once anyway).
3. **Order of work.** Chosen: *Home first* (contract, core, host, the add action, the config
   writer), in one session; then *the edit mode* and *the weather widgets* in parallel, in
   their own sessions, since they touch different files. Rejected: everything in sequence on
   one branch (slower, for no gain).

## Part 1: Home

### The settings

```toml
[[home.widgets]]
widget = "weather/now"
size = "quarter"
city = "Barcelona"

[[home.widgets]]
widget = "weather/today"
size = "single"
# no city: where the computer is
```

- Each entry: `widget = "<plugin-id>/<widget-id>"`, `size` (optional: the widget's default
  size), and the widget's parameters as further string keys. `widget` and `size` are
  therefore reserved: a manifest may not declare a parameter with either name.
- The entries' order is the tiles' order. The same widget with the same parameters may appear
  any number of times.
- Read at startup and whenever `config.toml` changes, like every setting. As with plugin
  preferences, a mistake costs only itself: an entry that is not a table, has no valid
  `widget`, or has a parameter that is not a string is skipped with a warning.
- An entry the launcher cannot show now (the plugin is not installed, not approved, disabled,
  or no longer declares that widget) is **not shown and not removed**: it comes back when the
  plugin does. A `size` the widget does not offer shows the widget's default size. A
  parameter the widget does not declare is not passed to it (and stays in the file).
- `[widgets] order` and `[widgets] hidden` are gone; if present they are ignored with one
  warning. There are no outside users to migrate.

### The launcher writes `config.toml`

The launcher has only read `config.toml` until now. From here on it also writes the Home list
(add, move, remove, resize):

- With a format-preserving TOML editor (`toml_edit`, already in the dependency tree), it
  replaces only the `home.widgets` array; everything else in the file stays byte for byte.
  The array is written as `[[home.widgets]]` tables. Builder's call: where a new array goes
  when the file has none (the end of the file is fine).
- Written atomically: a temporary file next to it, then a rename.
- If `config.toml` does not parse, nothing is written, and a toast says
  "config.toml has an error: fix it to change Home". The user's broken file is never
  overwritten.
- The file watcher sees the launcher's own write; reading it back gives the same Home, so it
  changes nothing on screen. Builder's call how to keep that from flickering (comparing the
  read settings to the current ones is enough).
- If the file does not exist, the launcher creates it with only the Home list.

### The plugin manifest

```toml
[[widgets]]
id = "now"
title = "Weather now"
sizes = ["quarter", "single"]   # the first is the default

[[widgets.params]]
key = "city"
title = "City"
```

- `sizes`: one or more of `quarter`, `single`, `double`, no repeats; the first is the
  default. The old `size = "…"` stays accepted as a one-size shorthand; a manifest with both is
  rejected.
- `[[widgets.params]]`: at most 8 per widget, `key` a short id (not `widget` or `size`),
  `title` for later use (the parameter editor). Values are strings; all of a tile's values
  together at most 4 KiB (the payload's old cap).

### The contract (WIT 0.1, edited in place while no outside plugin exists)

- `widget-answer { id, params: list<widget-param>, size: option<widget-size>, actions }`, with
  `widget-param { key: string, value: string }`. Only declared keys, each at most once.
  `size` picks one of the widget's sizes for the search; none means the default. The payload
  is gone.
- `widget-context`: `payload` is replaced by `params: list<widget-param>`; `query` stays (set
  for a search tile only). The size is known from `width` and `height`, as today; builder's
  call whether to add the size itself as a field too.
- `widget-target { widget-id, params, event }`: events of a tile come with its parameters.
  Two Home tiles with equal widget and parameters are the same to the plugin.
- `show-widgets` is removed. `redraw-widget(id)` redraws every tile of that widget, whatever
  its parameters.
- A new event, `home(list<home-tile>)` with `home-tile { widget-id, params }`: the plugin's own
  tiles on Home, each distinct widget and parameters once, in Home order. Sent after `init`
  and whenever that list changes (an add, a removal, an edit of `config.toml`, the plugin
  enabled again); a background event, never a user action. A plugin learns nothing of other
  plugins' tiles. This is how a plugin keeps its Home tiles fresh while the launcher is
  hidden (tiles get `hidden` then, and no renders): the weather plugin refreshes the places
  its Home tiles name, and forgets a place no tile names any more. My call (the alternative,
  learning places only from tiles drawn on screen, leaves every Home tile stale on each open
  and never tells a plugin that a tile was removed).
- `FROZEN` re-hashed and a `FROZEN.md` row added, as for every in-place edit. The SDK, the
  docs (`docs/plugins/manifest.md`, `writing-a-plugin.md`) and the WIT parity test follow.

### What the strip shows

- At the empty query: the Home tiles, in order.
- At a typed query: the search's tiles, as today. Home tiles are not shown while typing (the
  old rule that an "always" tile matching the query stays is dropped).
- The core keeps the last frame of each Home tile and draws it at once when the launcher
  opens, as it did for always tiles. Builder's call how a Home tile is keyed in the core (its
  position plus widget and parameters is enough; identical duplicates may share a render).

### "Add to Home"

- The launcher's own action, at the end of the action menu (Ctrl/Cmd+K) of every search
  tile; Enter stays the plugin's first action. A plugin cannot add, remove or rename it.
- It appends the tile (widget, the size the search showed, its parameters) to the end of
  `[[home.widgets]]`, and shows the toast "Added to Home".
- A stale search tile (answered for an older query) takes no action, as now.

### The plugins that change with it

- **calc**: its tile's payload becomes a declared parameter (builder's call of the name, e.g.
  `expression`); added to Home it keeps showing that expression's result.
- **kitchen-sink**: its three always tiles go; the tests and the acceptance measurements that
  used them put them on Home through a test `config.toml`.
- **weather**: in Part 1 only enough to keep working: `now` and `today` take `city` as a
  parameter (no parameter: where the computer is), and stop calling `show-widgets`. Its
  third widget, `city` (the tile of a searched city), goes: a city search answers `now` with
  the `city` parameter, so "Add to Home" writes `weather/now` from the start. A Home tile
  with a city fetches that city while it is drawn, as a searched city does now; refreshing
  the places of the `home` event while the launcher is hidden is Part 3, with the rest.
- `just try` writes a Home list into its temporary `config.toml`, so the try still shows
  tiles.

## Part 2: The edit mode

### Getting in and out

- **Keyboard:** select a Home tile (↑ from the list, as now), Ctrl/Cmd+K, "Edit Home". Also a
  launcher command "Edit Home" in the search; with an empty Home it shows the toast "Home is
  empty: add widgets from the search".
- **Mouse:** right-click a Home tile and "Edit Home", or a long press on a tile (about
  500 ms).
- **Out:** Esc or Enter, a click outside the tiles, or the launcher hiding.

### Inside

```
 ┌──────┐ ┌──────┐ ┌─────────────┐
 │ ⊖    │ │ ⊖    │ │ ⊖           │   ← the tiles jiggle
 │ 12°  │ │ 21°  │ │ Lisbon ...  │
 └──────┘ └══════┘ └─────────────┘
           selected
```

- The tiles jiggle (a small rotation, each a little out of phase with its neighbours); the
  selected one has the focus ring. Each tile has a remove badge (⊖) in its corner.
- The search field is dimmed, and typing does nothing (a stray letter neither leaves the mode
  nor starts a search).
- The tiles keep drawing, but take no events: keys and the pointer belong to the edit mode.
- **Keys:**
  - ←/→ (and Tab/Shift+Tab) select a tile; the strip scrolls to it.
  - Shift+←/→ move the selected tile one place.
  - Delete or Backspace removes it.
  - Ctrl/Cmd+K opens the tile's menu: its sizes (the current one marked; only when it has
    more than one) and "Remove".
  - Ctrl/Cmd+Z restores the last removed tile, at its old place; again for the one before,
    as long as the mode lasts.
  - Esc or Enter: done.
- **Mouse:** drag a tile to move it (the strip scrolls at its edges); click ⊖ to remove;
  right-click for the same menu as Ctrl/Cmd+K.
- **After each removal** a toast: "Press [Ctrl] [Z] to restore widget" (on macOS "[⌘] [Z]"),
  with the keys drawn as key chips, like the footer's shortcut chips, not as text.
- Every change is written to `config.toml` at once: a launcher closed mid-edit loses nothing.
- Removing the last tile leaves an empty Home; leaving the mode then removes the strip.
- If the operating system asks for reduced motion, the tiles do not jiggle; a dashed outline
  marks the mode instead. Builder's call how to read the setting on each OS; if it cannot be
  read, the tiles jiggle.

## Part 3: The weather widgets

### The widgets

Every widget takes the parameter `city` (absent: where the computer is, from the launcher's
location). The plugin preference `city` is removed: each tile now carries its own city, and
the launcher's `[location] city` covers "my city". `fahrenheit` stays (°F and mph); a new
bool preference `mmhg` shows pressure in mm Hg instead of hPa.

| Widget | Sizes | Search words (English / Russian) |
|---|---|---|
| `now` | quarter, single (today's wide tile with feels-like, wind, humidity) | `weather`, a bare city / `погода` |
| `today` | quarter, single, double | `weather … today` / `погода … сегодня` |
| `humidity` | quarter, single | `humidity` / `влажность` |
| `wind` | quarter, single | `wind` / `ветер` |
| `uv` | quarter, single | `uv`, `uv index` / `уф`, `ультрафиолет` |
| `pressure` | quarter, single | `pressure` / `давление` |
| `precipitation` | quarter, single | `rain`, `precipitation` / `дождь`, `осадки` |
| `air-quality` | quarter, single | `air quality`, `aqi` / `качество воздуха` |
| `pollen` | quarter, single | `pollen` / `пыльца` |
| `daylight` | quarter, single | `daylight`, `sunrise`, `sunset` / `световой день`, `восход`, `закат` |

The query forms, for any word above: `<word> in <city>`, `<city> <word>`, and `<word>` alone
(where the computer is); in Russian `<word> в <city>`. `today` / `сегодня` at the end turns a
`weather` / `погода` query into the `today` tile. Builder's call: Russian inflection of the
city (`в барселоне`) through the existing city search, which already matches several forms;
whatever it does not match falls back to the geocoder, as now.

### The plugin's day

The tile's day runs **from 06:00 to 06:00** in the place's own time (`timezone=auto`): at
02:00 it still shows the day before, with the night as "now". The parts: morning 6–12,
afternoon 12–18, evening 18–24, night 0–6 of the next date. For each part:

- the temperature (and feels-like, wind) at its middle hour: 9, 15, 21, 3;
- the sky: the most severe weather code in the part (builder's call of the severity order;
  thunder > snow > rain > drizzle > fog > overcast > partly cloudy > clear is the intent);
- the chance of rain: the highest in the part; the amount: the sum.

Parts already over are faded; the current part's icon is the bright one.

### `today`

As agreed on the mockups (`.superpowers/brainstorm/…/today-sizes-v2.html`, not committed):

- **quarter:** four narrow columns; over each, a muted icon for the part (sunrise, sun,
  sunset, moon) instead of a word; then the sky icon and the temperature; the place under.
- **single:** four columns: the part's name, the sky icon, the temperature, the chance of
  rain (blue when high).
- **double:** a left column with the place, today's high and low, sunrise and sunset, and how
  old the reading is; then four columns that add feels-like, wind, the chance and amount of
  rain, with icons in place of words (feels-like, wind) where an icon reads clearly.

### The measure widgets

Each shows the value now, the place, and a chart of the day (06:00 to 06:00); the past part
of the chart faded, a dashed line at now. **quarter**: the measure's icon and name, the value,
a small chart, the place. **single**: adds a line of detail and a larger chart with hour
marks (00, 06, 12, 18). As agreed on `measures-v2.html`:

| Widget | Value | Detail (single) | Chart |
|---|---|---|---|
| `humidity` | relative humidity, % | dew point | line |
| `wind` | speed (km/h or mph) | direction (arrow and compass point), gusts | line |
| `uv` | UV index | the level (Low < 3, Moderate 3–5, High 6–7, Very high 8–10, Extreme 11+) and until when it is 3 or more | bars coloured by level |
| `pressure` | hPa or mm Hg | rising, falling or steady (the change over 3 hours; builder's call of the threshold, about 1 hPa) | line |
| `precipitation` | mm in this hour (cm of snow when it snows) | the next part of the day likely to be wet and its chance ("70% after 18:00") | bars, mm per hour |
| `air-quality` | European AQI | the level (Good, Fair, Moderate, Poor, Very poor, Extremely poor) and the main pollutant | bars coloured by level |
| `pollen` | the strongest pollen now (alder, birch, grass, mugwort, olive, ragweed) and its level | the next two | bars of the strongest |
| `daylight` | the length of the day (9 h 14 min) | sunrise, sunset, and the change per day (+2 min) | the day's length over the next 7 days |

- Pollen data exists only for Europe, in season: elsewhere, or out of season, the tile says
  so calmly ("No pollen data here"). Builder's call of the level bands per pollen (from the
  European scale the data comes with).
- Icons: the launcher's own icon set (the weather plugin already names icons such as
  `cloud-sun`); icons wherever they read better than words, as on the mockups. Builder's call
  where the set lacks one.

### Data

- Per place, **one forecast request** carries the current values, the hourly values for the
  06–06 day (two dates) and the daily values for seven days (sunrise, sunset, daylight) for
  every widget; **one air-quality request** (`air-quality-api.open-meteo.com`: European AQI,
  its sub-indices, the six pollens) is made only while a tile that needs it is on screen or
  on Home (as the `home` event tells). Every tile of a place shares them; refreshed as today
  (every 15 minutes, with the same backoff), for every place a Home tile names, also while
  the launcher is hidden, and for a searched place while its tile is on screen.
- The new host adds to `http` in `plugin.toml`: the plugin's permissions change, so the user
  approves it once more.
- The plugin keeps one state per place (the current one keeps one), so several cities on
  Home each have their own reading, location state and backoff.

## Testing

- **Core:** reading `[[home.widgets]]` (bad entries skipped, unknown ones kept, duplicates);
  writing it keeps the rest of the file exactly; a broken file is not written; the order of
  tiles at the empty and at a typed query.
- **Host:** parameters reach `render-widget` and events; `show-widgets` no longer exists;
  "Add to Home" appends the right entry; the manifest's `sizes` and `params` checks.
- **UI:** the edit mode's keys as a pure function of the state and the key, like the strip's
  keys today; undo of removals; the toast's key chips.
- **Weather:** pure functions for the parts of the day, the sky's severity, the pressure
  trend, the UV, AQI and pollen levels, the search words; recorded API answers as fixtures,
  as now.
- Seen in a headless run, by the usual rules (own `TOMAKAST_HOME`, no keyboard grab).

## Left to the builder

Marked "builder's call" above; besides those: the jiggle's exact motion (the pinned GPUI
cannot rotate a tile's text and images, so a small wobble or shift is expected instead of a
rotation), the toast's timing,
the long-press time, chart styling within the launcher's theme, how the core shares renders
of identical tiles, and the names of the new icons.
