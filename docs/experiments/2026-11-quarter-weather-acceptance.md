# Quarter tiles and the weather plugin: acceptance

Date: 2026-11-20. Branch `claude/widget-quarter-weather-750845`.

## What changed

- **A third tile size, `quarter`.** `size = "quarter"` under `[[widgets]]`. Its width is
  `(W - 3 * gap) / 4`, 173.5 px in the 750 px panel (W = 718, gap = 8). Four quarters fill the
  strip, and two quarters with the gap between them are exactly one single (355 px). The
  quarter keeps the strip's one height, 120 px: the strip stays one row, so packing, sideways
  scrolling, the peeking tile and ←/→ focus work unchanged for every mix of sizes. The name
  `quarter` follows the width, like `single` (half) and `double` (all).
- **The quarter size needed no WIT change** (the location work later did, see below).
  `plugin.wit` names no tile size: `size` lives in `plugin.toml`
  only, and the render context already carries the tile's `width` and `height`. So `FROZEN`
  keeps its hash, and `FROZEN.md` gets no row. The manifest freeze test covers keys, not
  values, and passes.
- **kitchen-sink** declares `quarter-a` and `quarter-b`; the query `quarters` answers both as
  condition tiles, (its single tiles `static`, `buttons` and `anim` are on Home only when the user puts them there).
- **`plugins/weather`**, Open-Meteo, no key: the widgets `now` (quarter or single) and `today`
  (single), shown on Home, and, for a city typed in the search, the tile `now` with that city. See "A
  smaller example: weather" in `docs/plugins/writing-a-plugin.md`.
- **`just try` no longer installs kitchen-sink** (the user, 2026-11-20). Its host tests and
  CI checks stay; the `quarters` query below needs it installed by hand.

## Decisions taken without the user

- The quarter's height is the strip's (120 px), not half of it: two rows of quarters would
  need a second strip row, a new packing rule and a new focus order (↑/↓ inside the strip),
  for little gain.
- The weather plugin puts nothing on Home itself; `just try` writes a `[[home.widgets]]` list with both tiles, and a user who wants only the quarter keeps one entry.
- The default city was Berlin; since the location work (below) it is `auto`.
- A Cyrillic city name is geocoded in Russian, then Ukrainian: Open-Meteo matches names only
  in the language it is asked in (`Москва` with `language=en` finds nothing).
- Failures back off 1, 2, 4, 8, then 15 minutes; an unknown city is not asked again until the
  city changes or the user refreshes.
- A bare city name in the search is matched against a built-in list (the user's choice over
  asking Open-Meteo about every word typed): GeoNames cities of at least 100,000 people,
  under their name, ASCII name and Cyrillic names, and other Latin names for cities of a
  million or more; 551 KB of data, the wasm grows from 247 KB to 848 KB. Words that are also
  cities do show a tile: `Nice`, `Reading`, `Bath`, `Split`, `Mobile`.
- `weather in` / `погода в` with a city not on the list asks Open-Meteo's geocoder, as the
  preference does.

## Location (added later the same day)

The user chose the operating system's location service over the time zone's city and IP
geolocation, for all three OSes at once, rounded to about 10 km. See
`docs/superpowers/specs/2026-11-20-location-design.md`. WIT 0.1 gained the `location`
interface and event in place (`FROZEN` `aebb34f22cd1`, a `FROZEN.md` row). The weather
plugin's `city` defaults to `auto`.

On the dev box (KDE, xdg-desktop-portal 1.22) GeoClue is not installed, and the GNOME
setting the portal itself reads, `org.gnome.system.location enabled`, is `false`. Seen with
`just locate` (row L1): the portal refuses at once, without a dialog, with `NotAllowed:
Location services disabled`.

Second round (2026-11-21, the user's ruling): the launcher tries four sources in turn: the
city in `[location] city`, the OS service, Wi-Fi via BeaconDB (Linux, only with
`[location] wifi_lookup = true`), the time zone. `coordinates` gained `accuracy-km` (`FROZEN`
`780c5954b687`). Seen with `just locate` on the dev box: the portal refused as before, and
the time zone, `Europe/Madrid`, gave `40.4, -3.7 within 500 km`. So `auto` here shows
"≈ Madrid, ES" unless a city is set. Seen in a headless daemon run (the weather's kept
place): "≈ Madrid, ES"; `[location] city = "Berlin"` → "Berlin, DE" within 5 s; the table
removed → "≈ Madrid, ES". `city = "Xyzzy"` logs that it is not on the list; the tile's
"No location" is covered by a host test, not seen. The Wi-Fi lookup has not been run (it sends the
networks around to BeaconDB; row L3 needs the user's yes). A city set in `config.toml` does
not survive `just try`, which starts from a fresh home each time.

Seen in a headless daemon run with `city = "Berlin"` in `config.toml`: the approval stores
`location = true` with the other permissions, the plugin loads without a new review, and
fetches Berlin's weather.

## Checked by the builder

- `just check` passes (fmt, clippy native and wasm, every test, the weather plugin included).
- A headless daemon (own `TOMAKAST_HOME`, `TOMAKAST_NO_KEYBOARD=1`) against the real
  Open-Meteo: the plugin loaded; `city = "Москва"` written into `config.toml` while it ran
  was picked up within seconds (`preferences changed` in the log), geocoded in Russian and
  fetched (stored: `name=Москва`, `temp=4.5`, `code=3`); then `Xyzzyqqplonk` was picked up
  too, with no warning or error of the plugin in the log. The fetch for the default city
  before the change was not looked at, nor toasts (the run had no window open). Not checked
  by eye: how the tiles look; that is the checklist below.

## User checklist

Run from this worktree: `just try approve` (only one `just try` at a time across
worktrees), then `just toggle` from another terminal.

| # | Step | Expected | Result |
|---|---|---|---|
| Q1 | Open the launcher, empty query. | The weather quarter and, after it, the weather single (plus any demo tiles). The quarter is half as wide as the single. | |
| Q2 | Type `Москва`. | Three tiles: the Москва quarter first, then the weather quarter, then the single. The two quarters with the gap between them are as wide as the single. (With kitchen-sink installed by hand, `quarters` shows its two quarter tiles, each saying `173.5 x 120`.) | |
| Q3 | Tab into the strip, then → through every tile and ← back. | Each tile gets the focus ring in turn, the focused tile is always whole on screen, and the strip scrolls to it. | |
| W1 | Look at the weather quarter (city `auto`). | Without GeoClue (the portal refuses at once here, see L1): the time zone's place, marked, e.g. "≈ Madrid, ES", no toast. With GeoClue installed (and the desktop's permission given): the nearest big city, unmarked, e.g. "Berlin, DE". | |
| S1 | Type `Москва`, then `paris`, then `weather in Tokyo`. | A weather quarter for that city comes first in the strip (`Москва, RU`, `Paris, FR`; Tokyo by the geocoder, `Tokyo`), "Loading" for a moment, then the temperature. Enter copies a line like `Paris, FR: 9°, Overcast, H 11° L 6°`. | |
| S2 | Type `fire`, `settings`, `Berl`. | No weather tile. | |
| W2 | In `$XDG_RUNTIME_DIR/tk-try/config.toml` add `[plugins.weather]` and `city = "Москва"`, save. | Within a few seconds both tiles show Москва (no restart). | |
| W3 | Set `city = "Xyzzyqqplonk"`. | "Unknown city" with the name, no toast. | |
| W4 | Set `city = "Москва"` and `fahrenheit = "true"`. | Temperatures in °F, wind in mph. | |
| W5 | Turn the network off, focus the weather tile, press Enter (Refresh). | The last reading stays; the quarter adds "· N min ago", the single says "Offline · …". No toast. Network back: within a few minutes (or Enter) it says "Updated just now". | |
| L1 | `just locate` (from this worktree). | The OS service's answer, or its reason, then the time zone's place; for each, what plugins get. The desktop may ask for permission first. | 2026-11-20, KDE, no GeoClue, location off: `no location: … NotAllowed: Location services disabled`, at once, no dialog. 2026-11-21: the same, then `the time zone: 40.4, -3.6833 within 500 km` (Europe/Madrid). |
| L2 | Run `just try`, then in `$XDG_RUNTIME_DIR/tk-try/config.toml` add `[location]` and `city = "Berlin"`, save. | Within a few seconds the weather tiles say "Berlin, DE", unmarked, without a refresh. `city = "Xyzzy"`: "No location". Remove the table: back to the time zone's place. | |
| L3 | Only with the user's yes (sends the Wi-Fi networks around and the IP to BeaconDB): `just locate-wifi`. | A `Wi-Fi:` line with a place within a few km, or why not (fewer than two networks, not in BeaconDB). | |

## Home, Part 1: a headless look (2026-11-21)

`just try` now writes a `[[home.widgets]]` list into its home's `config.toml`: `weather/now`
(`quarter`) and `weather/today` (`single`). `just try` itself was not run (it grabs the
keyboard). Instead the recipe was reproduced by hand in a throwaway home: calc and weather
copied from the release build and approved with `approve_plugin`, that `config.toml`, and
`tomakast daemon` from the release build with `TOMAKAST_NO_KEYBOARD=1`.

- The daemon started and loaded calc and weather; the log has no error from either, and none
  about the Home list.
- **Not confirmed: that the two Home tiles rendered.** Tiles render only while the launcher is
  on screen, and the launcher was not shown. The core logs nothing when it tells a plugin its
  `home` tiles (only a failure), and a render is a `trace` line of target `widgets`, so a
  hidden daemon's log shows neither. With `RUST_LOG` at `trace` for `widgets`, `plugin` and
  `tomakast_core` the log stayed empty of both. Rendering of Home tiles rests on the core,
  host and UI tests, and on the user's look at `just try`.
- In the first run, the daemon opened three HTTPS connections within 15 s of start, one of
  them to `api.open-meteo.com`'s address, so weather fetches in the background with Home
  set. The second run, on the same home, opened none within 20 s (a fresh cached answer is
  the likely reason; not checked).
- Editing `config.toml` while the daemon ran: a Home list cut to one entry produced no log
  line, as there is none for a valid change. A deliberately broken file (`garbage = [`) was
  picked up at once and logged as a warning that plugin preferences, location settings and
  Home stay as they were. The effect of the cut list on the strip was not seen, for the same
  reason as above.
- The daemon was killed (only mine) and the temporary home removed.
