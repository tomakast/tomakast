# Calc conversions acceptance (Task 7)

Date: 2026-11-19. Branch `fb-calc`, measured at `eef0ba2`. Machine: the plan 1 setup (KDE
Plasma 6.8.3, Wayland, 16 threads, kernel 7.3.5-1-cachyos).

Three parts: [the numbers](#the-numbers), [rulings](#this-plans-rulings) and the
[user checklist](#user-checklist) (result cells blank).

## The numbers

| What | Measured | Budget | How |
|---|---|---|---|
| Search, engine only | worst of 1000 random queries ≤ 1 KiB: **26 µs** | 10 ms | `cargo test --release --manifest-path plugins/calc/Cargo.toml -- --nocapture search_is_fast` (native, not wasm) |
| 1000-line sheet, engine only | **1.36 ms** | 50 ms | the same, `sheet_of_1000_lines_is_fast` |
| 10 000-line zone sheet, engine only | **17.4 ms** | 200 ms | the same, `zone_sheet_of_10000_lines_is_fast` |
| Parsing the rate bodies | er-api fixture **56.9 µs**, coingecko fixture **7.7 µs** | 5 ms | the same, `parsing_rates_is_fast` |
| 1 KiB search query, through the host | **1.08 ms** | 10 ms | `cargo test --release -p tomakast-plugin-host --test calc -- --nocapture pad_and_search_stay_within_budget zone_and_currency_queries_stay_within_budget`: the round trip through the core, calc in wasm |
| `now in New York`, through the host | **1.05 ms** | 10 ms | the same, `zone_and_currency_queries_stay_within_budget` |
| `100 usd in rub`, through the host | **1.05 ms** | 10 ms | the same |
| 1000-line sheet recompute, through the host | **3.16 ms** | 50 ms | the same, `pad_and_search_stay_within_budget` |
| `calc.wasm` | **924,711 bytes** (903 KiB), +597,843 over 326,868 at `7299987` | gate 982,228 (326,868 + 640 KiB); 57,517 bytes spare | `CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 --manifest-path plugins/calc/Cargo.toml`, then `ls -l target/plugins/wasm32-wasip2/release/calc.wasm` |

Full suites at the same commit: `cargo test --release --manifest-path plugins/calc/Cargo.toml`:
85 passed. `cargo test -p tomakast-plugin-host --test calc`: 44 passed, 4 ignored (the timing
tests, which need `--release`).

After the final-review fixes: `calc.wasm` is **941,592 bytes**, 40,636 under the gate (the
same build command); `cargo test --release --manifest-path plugins/calc/Cargo.toml`: 88 passed;
`cargo test -p tomakast-plugin-host --test calc`: 46 passed, 4 ignored.

The planning measurements (at `7299987`): jiff 1.0.1 with the bundled tz database 832,520
bytes (+505,652); chrono-tz 0.10.4 1,280,064 (+953,196); serde + serde_json 370,985 (+44,117).
The final build is 92,191 bytes above the jiff-only build: that is all the new engine code and
tables (units, rates, cities, the JSON reader).

## This plan's rulings

- **jiff over chrono-tz.** +505,652 bytes against +953,196 for the same tz data, and its API
  fits: a case-insensitive `tz::db().get`, `available()` and calendar `Span` arithmetic with
  month-end clamping. Moments are formatted by a hand-written formatter, not `strftime`; the
  output is the same.
- **A hand-written JSON reader for the rate bodies** (`src/json.rs`), instead of `serde_json`
  (+44 KiB). The `Cargo.toml` note "no parser crates" stays true.
- **The zone is shown.** A moment with a time shows its tz abbreviation (`MSK`, or numeric
  `+04`). An answer that used the home zone without showing it (a date-only moment, a duration
  counted from `today` or a date) names the IANA zone in its subtitle
  (`days until 31 dec · Europe/Moscow`), and so does a clock time read in the home zone and
  shown in another (`5pm in Tokyo · UTC`: 5pm UTC). `now` reads no zone (its instant is the
  same everywhere), so `now in Tokyo` and `now to unix` name none. A date-only moment converted to a zone stays
  date-only. `today`, `tomorrow` and `yesterday` in a zone are computed in that zone, and the
  subtitle names the zone used.
- **Soft names.** Every new unit name and alias, currency code, ticker, month, city and the
  words `now today tomorrow yesterday time` are soft: a variable of that name wins, and the
  unit or currency meaning applies only after a number or after `in`/`to`. The 20 unit names
  that existed before (`mm cm m km in ft yd mi mg g kg t oz lb ms s min h day week`) stay
  reserved, and so does `until`.
- **`cup` and `ton` shadow CUP and TON in lower case.** The volume unit and the metric tonne
  win (`1 ton in kg` is `1000 kg`); `CUP` is the Cuban peso and `TON` the coin.
- **Bare words and values.** Bare `now`, `time`, `today`, `tomorrow` and `yesterday` answer
  (an exception like `pi`, in any case). Bare dates, clock times and money (`31 dec`, `5pm`,
  `100 usd`) do not. `0xff` does, because writing a number in another base is a conversion.
  `now in` and `today in history` get no answer.
- **City names.** Derived city names skip backward-link zones (`Etc/`, `US/`, `SystemV/`,
  `Canada/`, `Brazil/`, `Mexico/`, `Chile/`, top-level names, and the short legacy
  `Australia/` names). `pacific`, `eastern`, `central` and `mountain` map to the US zones.
  `now in east` is "unknown unit".
- **Word time units are plural** unless the number is exactly 1 (`42 days`, `1 day`).
- **The age of rates is calc's fetch time**, not the API's update time. Rates older than 24
  hours still convert and the subtitle says so.
- **One 10-minute tick.** Fiat is due when missing or ≥ 1 h old; crypto when missing, or when
  ≥ 10 min old and either at `init` or used in the last 30 min (60 s of slack on every age).
  A missing feed is always due. Opening a sheet counts as using the feeds its lines need. A
  feed about to be fetched is not also retried as an unsaved write in the same refresh: one
  write per feed per refresh. A failed fetch of a feed calc holds no table of starts a
  one-shot retry after 60 s (one waiting at a time; while offline that is one try a minute).
  A table dated more than 60 s after now (the clock was ahead) is due, and its answers say
  `rates of unknown age`.
- **Open sheets follow.** New rates, and a change of the `timezone` preference, update every
  open pad unasked (at the revision the launcher last sent). Such an update saves nothing and
  does not count as using a feed.
- **Rate writes never use the reserve and never drop sheets.** With no room or a full quota
  the rates stay in memory, with no toast. A rate answer over 8 KiB is refused unread.
- **A test-only HTTP responder** in the host, so no calc test touches the network. The harness
  defaults to offline.

## User checklist

Run from the worktree root (in `bash`, if your shell is fish). Same throwaway home, build and
staging as the [2b-2b checklist](2026-11-plan2b2b-acceptance.md#user-checklist). Do not run
this against your real home. The network is on for K1–K5 and K7–K8: approve the prompt (K1)
before K2, and K2 and K3 fetch live rates. K6 starts again in a fresh home with the network
off (see below the table), so do it last. The expected values were checked against the
engine (a native probe over the same code); currency amounts and times differ with the day.

```bash
cargo build --release -p tomakast-app
CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 \
  --manifest-path plugins/calc/Cargo.toml
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-calc.XXXX)
STAGE=$(mktemp -d)/calc && mkdir -p "$STAGE"
cp plugins/calc/plugin.toml "$STAGE/"
cp target/plugins/wasm32-wasip2/release/calc.wasm "$STAGE/plugin.wasm"
unset TOMAKAST_NO_KEYBOARD
target/release/tomakast daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!
mkdir -p "$TOMAKAST_HOME/plugins" && cp -r "$STAGE" "$TOMAKAST_HOME/plugins/"
```

| # | What | Expected | Result |
|---|---|---|---|
| K1 | Re-approval | The prompt lists the five permission lines, in the C1 order. | |
| K2 | Currency | `100 usd in rub` answers within a few seconds of approval. | |
| K3 | Crypto | `0.5 btc in usd` and `1 eth in btc` answer. | |
| K4 | Zones | `now in Tokyo` answers with a time ending in `JST` and the subtitle `now in Tokyo`; `5pm London in Moscow` answers `…, 20:00 MSK` with the subtitle `5pm London in Moscow` (in November). | |
| K4a | Home zone named | With `timezone` not set (UTC, before K5): `5pm in Tokyo` answers tomorrow's date `…, 02:00 JST`, and its subtitle is `5pm in Tokyo · UTC`. | |
| K5 | Preference | The setting goes in `$TOMAKAST_HOME/config.toml` (the throwaway home's; the command below creates it if missing). No restart: the daemon watches the file and gives calc the new value within about a second. The launcher hides when it loses focus (and every view closes), so the edit is made by a delayed command: run `(sleep 15; printf '\n[plugins.calc]\ntimezone = "Europe/Moscow"\n' >> "$TOMAKAST_HOME/config.toml") &` (it appends; the throwaway home has no `[plugins.calc]` table yet), then open "Calculator" at once and type `now + 0 min` (it ends in `UTC`). Wait without typing: about 15 s later the sheet's line changes to `MSK` unasked. Then the search `now + 0 min` ends in `MSK`. | |
| K6 | Offline | In a fresh home started with the network off (below): after approving, `2+2` answers `4`, `100 usd in rub` gets no calculator row, the sheet line `100 usd in rub` says `no exchange rates yet`, and there is no toast. Then, with the sheet open and no typing, the delayed `nmcli networking on` turns the network on: within a minute or two (the next 60 s retry after the network is up) the line becomes a RUB amount unasked. | |
| K7 | Units and bases | `100 F to C` → `37.77777778 °C`; `255 in hex` → `0xff`; `1 GiB in MiB` → `1024 MiB`; `10 km/h in mph` → `6.213711922 mph`; `1 ton in kg` → `1000 kg`. | |
| K8 | Not math | `31 dec`, `5pm`, `100 usd`, `today in history`, `5pm meeting` get no calculator row; bare `now` answers with the time in the zone's abbreviation (`… MSK` after K5) and the subtitle `now`, and `today` answers the date with the subtitle `today · Europe/Moscow`. | |

K6 setup: a fresh home, so that no rates are stored, with the network off before the daemon
starts (`nmcli networking off` turns NetworkManager's networking off; unplugging the cable and
turning Wi-Fi off does the same):

```bash
kill "$DAEMON"; rm -rf "$TOMAKAST_HOME"
nmcli networking off
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-calc.XXXX)
target/release/tomakast daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!
mkdir -p "$TOMAKAST_HOME/plugins" && cp -r "$STAGE" "$TOMAKAST_HOME/plugins/"
```

Approve the prompt again and check the search part of K6. Then run
`(sleep 20; nmcli networking on) &`, open "Calculator" at once, type `100 usd in rub`, and wait
without typing (the launcher must keep focus: leaving it hides it and closes the sheet).

Teardown: `kill "$DAEMON"; rm -rf "$TOMAKAST_HOME" "$(dirname "$STAGE")"; unset TOMAKAST_HOME`,
and `nmcli networking on` if K6 left the network off.
