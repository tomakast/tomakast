# Location for plugins

Date: 2026-11-20. Status: approved by the user in chat. First round: the operating system's
location service on all three OSes, about 10 km. Second round (2026-11-21): four sources in
turn, so that a computer without GeoClue still has a location, and the accuracy in the
contract.

## Goal

A plugin can learn where the computer is, approximately, with the user's approval. The first
user is the weather plugin: its city defaults to `auto`, the computer's location.

Rejected: IP geolocation (the IP goes to a third party for a guess; wrong under a VPN). The
Wi-Fi lookup below turns the service's own IP fallback off.

## Contract (WIT 0.1, edited in place while no outside plugin exists)

- A permission, `location = true` under `[permissions]`. The approval prompt says "Know your
  approximate location (within about 10 km)". An update that adds it is reviewed again, like
  every new permission. Where the launcher takes the location from is the user's setting, not
  a permission: a plugin cannot tell the sources apart, and needs nothing more for any.
- An import, `location.request: func() -> result<_, string>`. It fails at once without the
  permission. The answer arrives later as `event.location(result<coordinates, string>)`:
  - `coordinates { latitude: f64, longitude: f64, accuracy-km: u32 }`; the position is rounded
    by the host to 0.1° (about 11 km of latitude) before the plugin sees it, and
    `accuracy-km`, how far the place may be from it, is never below 11;
  - or why there is none.
- A request while one of the plugin's waits is not sent again: the one event answers both.
- The host finds the location at most once at a time for all plugins, and keeps the answer:
  a fix of 50 km or better for 10 minutes; a coarser one, or a failure, for 1 minute.
- When the user changes the launcher's location settings, the kept answer is forgotten, and
  every running plugin that asked before gets the new answer as an event, unasked.
- The event is a background event (the background budget counts it), never a user action.
  An answer that arrives after the plugin stopped is discarded.

## Sources, in turn

The first that answers wins; when none does, the plugin gets the reasons of all.

1. **The city the user set**, `[location] city` in `config.toml`, by name from the built-in
   list of cities (the weather plugin's, now shared). Accuracy 11 km. A name that is not on
   the list is an error that says so: it does not fall through to a guess.
2. **The operating system's service** (below), 60 s at most. Its own accuracy, at least 11 km.
3. **Wi-Fi, on Linux, only when the user turned it on**, `[location] wifi_lookup = true`
   (off by default): the networks NetworkManager last saw (no new scan, no privileges) go to
   BeaconDB, an open database of where networks are, 20 s at most.
   - Sent: each network's address (BSSID) and signal, at least two of them (fewer gets no
     answer), hidden networks and names ending in `_nomap` left out (the networks' owners'
     opt-out). The service's IP and cell fallbacks are turned off (`considerIp: false`,
     `fallbacks: {ipf: false, lacf: false}`): no answer rather than an IP guess.
   - Checked in BeaconDB's source (`src/geolocate.rs`, 2026-11-21): `considerIp: false` or
     `ipf: false` turns its IP estimate off, and an IP estimate is marked `fallback: ipf`,
     which the launcher refuses anyway; `lacf` is not read, but no cells are sent, so there
     is nothing for a cell estimate. It needs two known networks itself.
   - The request names the client (`tomakast`) and nothing of the user. BeaconDB still sees
     the computer's IP address, and keeps its HTTP logs 28 days.
   - Accuracy from the answer, at least 11 km.
4. **The time zone**: the main place of the computer's zone (`Europe/Moscow` is Moscow), from
   a table built into the launcher (from the tz database's `zone1970.tab`, with its old names).
   Accuracy 500 km, since one zone may cover a country. A zone without a place (`UTC`) is an
   error.

A source that outlives its time is not waited for: the next is tried, and that source is not
asked again until its call returns.

## Settings

```toml
[location]
city = "Москва"      # a city of the built-in list; empty or missing: find it automatically
wifi_lookup = false  # source 3
```

A wrong value (not a string, not a boolean) is ignored with a warning, like plugin
preferences: it does not cost the user the rest of their settings. The settings are read at
startup and whenever `config.toml` changes. Choosing the city inside the launcher (a picker)
is a later piece.

## Operating systems

The backends live in a small crate, `tomakast-location`, that depends on nothing of the
launcher, so it type-checks for every target from Linux. Each asks for a city-level fix.

- **Linux:** the XDG desktop portal's Location interface (`ashpd`), `accuracy = City`. The
  portal asks GeoClue, and the desktop may show its own permission dialog. The portal refuses
  while `org.gnome.system.location enabled` is off (on KDE too, as seen), and without
  GeoClue. The Wi-Fi networks come from NetworkManager over D-Bus (`GetAllAccessPoints`, then
  each access point's `HwAddress`, `Ssid`, `Strength` in percent, sent as about
  `percent / 2 − 100` dBm).
- **macOS:** Core Location (`CLLocationManager`, `kCLLocationAccuracyThreeKilometers`), on a
  thread with its own run loop. The system grants location only to an app bundle with
  `NSLocationUsageDescription`; a launcher started outside one may be refused or never asked.
- **Windows:** `Windows.Devices.Geolocation.Geolocator` (`RequestAccessAsync`, then
  `GetGeopositionAsync`), with the desired accuracy `Default`. Refused when location is off
  in Settings.
- **The time zone** on every OS through `iana-time-zone` (on Windows the OS gives the IANA
  name).

Verified so far: the host's rules are tested with fake sources. The Linux portal has run once
against a real portal (KDE, xdg-desktop-portal 1.22, no GeoClue, the GNOME setting off): it
refused at once, without a dialog, with `NotAllowed: Location services disabled`. In a
headless daemon run on that machine (no Wi-Fi lookup): `auto` gave the time zone's place,
"≈ Madrid, ES"; `[location] city = "Berlin"` written to `config.toml` turned the weather to
Berlin within seconds, unasked; removing it went back. NetworkManager listed the networks
around (52, 39 usable); nothing was sent. A real fix from an OS service, and a real Wi-Fi
lookup, have not been seen. macOS and Windows only
type-check and pass clippy for their targets (`just check`).

## Weather

- `city` defaults to `auto` (an empty value means `auto` too); a city of its own overrides
  the launcher's location for the weather only. The plugin asks for the location at `init`,
  when the preference becomes `auto`, every 3 hours (every 30 minutes when the answer was
  coarser than 50 km, or failed), and when the user refreshes; and it takes the event that
  comes unasked.
- The tile names the biggest city of its built-in list within 20 km, else the nearest within
  50 km (`Berlin, DE`), else the coordinates. An answer coarser than 50 km is marked:
  `≈ Moscow, RU`.
- Without a location the tiles say "No location" and "Set a city in preferences", calmly.
  A failure that answers the plugin's own request keeps a place known before (the service
  was slow); one that comes unasked (the user set a city that is not on the list) forgets
  it, so the tile does not quietly stay at the old place.

## Testing

- The plugin host's tests, and the harness, never reach a real source: each source is
  injected, the harness's default refuses, and the Wi-Fi request and its answer are pure
  functions tested on their own.
- `just locate` tries the sources on this computer and prints each answer, except the Wi-Fi
  lookup. `just locate-wifi` also does that one, which sends the networks around to BeaconDB,
  so it is never run without the user's yes.
