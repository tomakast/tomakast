# Calc colours, bases, units and city short names: acceptance

Date: 2026-11-22. Branch `claude/calc-converter-c43642`. Machine: the plan 1 setup (KDE
Plasma 6.8.3, Wayland, 16 threads, kernel 7.3.5-2-cachyos). Spec:
`docs/superpowers/specs/2026-11-22-calc-colors-bases-units-design.md`; plan:
`docs/superpowers/plans/2026-11-22-calc-colors-bases-units.md`.

Three parts: [the numbers](#the-numbers), [rulings](#rulings) and the
[user checklist](#user-checklist) (result cells blank).

## The numbers

| What | Measured | Budget | How |
|---|---|---|---|
| `calc.wasm` | **1,013,682 bytes**: +41,265 over 972,417 at the start (`58ebada`) | gate 1,047,764 (982,228 + 64 KiB, the user's ruling of 2026-11-22); 34,082 bytes spare | `CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 --manifest-path plugins/calc/Cargo.toml`, then `ls -l target/plugins/wasm32-wasip2/release/calc.wasm` |
| Of that, colours | 995,450 after Task 3 (+23,033) | — | the same, at the Task 3 commit |
| Search, engine only | worst of 1000 random queries ≤ 1 KiB: **23 µs** | 10 ms | `cargo test --release --manifest-path plugins/calc/Cargo.toml -- --nocapture search_is_fast` |
| 1000-line sheet, engine only | **2.38 ms** | 50 ms | the same, `sheet_of_1000_lines_is_fast` |
| 10 000-line zone sheet, engine only | **21.5 ms** | 200 ms | the same, `zone_sheet_of_10000_lines_is_fast` |
| Parsing the rate bodies | er-api **57.3 µs**, coingecko **8.1 µs** | 5 ms | the same, `parsing_rates_is_fast` |
| 1 KiB search query, through the host | **1.10 ms** | 10 ms | `cargo test --release -p tomakast-plugin-host --test calc -- --nocapture pad_and_search_stay_within_budget zone_and_currency_queries_stay_within_budget` |
| `now in New York` / `100 usd in rub`, through the host | **1.06 ms** / **1.03 ms** | 10 ms | the same |
| 1000-line sheet recompute, through the host | **5.24 ms** | 50 ms | the same |

The 1000-line sheet is slower than in the conversions note (1.36 → 2.38 ms engine, 3.16 →
5.24 ms through the host). Every line is now lexed for colours first and, only when it holds
a `#` colour, a second time; the benchmark sheet has a `# comment` line in every five. Both
stay far inside their budgets.

Suites at the end, after the final-review fixes: `cargo test --release --manifest-path
plugins/calc/Cargo.toml`: 128 passed. `cargo test -p tomakast-plugin-host --test calc`: 56
passed, 4 ignored (timing). `just check` (fmt, clippy native and wasm, the workspace tests,
the SDK, calc and weather suites) passed. Two runs before that failed on timing tests outside
this branch (`searches_are_coalesced_per_module` in core, the HTTP import's
`a_dropped_plugin_sends_nothing_and_gets_nothing`) while other sessions' builds kept the load
average above 20; each passes alone, and the third run passed whole.

## Rulings

Made on the user's behalf, in the spec ("my call") or while building. Each says what it
costs if wrong.

- **A `#` is a colour only where the whole line then works.** Otherwise the line reads as
  before, with `#` starting a comment. The price: a comment line that is nothing but a hex
  colour (`#bad`, `#add`, `#cafe`, `#2024`, `#100`) now shows that colour, and it becomes
  `prev` for the next line (`10`, `#bad`, `prev + 1` used to give `11` and now says
  `cannot add a color`); in search, `#2024` or `#123` alone now gets a colour tile. And a line
  that starts with a `#` colour and then fails (`#fff + 1`, the typo `#ff8800 in hls`)
  shows nothing, like a comment. Colours written without `#` show their errors. When both
  readings fail, the colour's error is shown if the colour reading parsed
  (`c = #fff + 1`: `cannot add a color`), else the comment reading's (`issue #1234`:
  `unknown name 'issue'`, as before).
- **What a bare colour shows:** a hex colour shows `rgb(…)`, anything else hex. A colour
  name alone answers in sheets but not in search, as `100 usd` alone does not.
- **`tan` stays a function** that cannot be a variable; `tan in hex` and `x = tan` are the
  colour.
- **Output forms:** hex lowercase; `rgb(255, 136, 0)` with commas; hsl and oklch trimmed to 1
  and 3 decimals; OKLCH outside sRGB is clipped per channel.
- **The tile swatch** is 72 px with a 10 px radius and a hairline border in the theme's
  border colour. A translucent colour lies on a white and grey checkerboard of 8 px squares,
  every square inside the rounded corners. The sheet swatch is 12 px, radius 3, right before
  the right-aligned note.
- **Copy actions** `Copy as HEX/RGB/HSL/OKLCH` evaluate the input afresh, on both the search
  tile and the Home tile.
- **Units that make each other:** for data, data rate, energy and power, a result that fits
  no unit exactly takes a unit of the size of one operand's unit, before the base unit
  (`10 MB/s * 1 min` is `600 MB`). Lengths, areas, volumes, times and speeds keep the rule
  they had, and never land in a unit this branch added (`1 kn * 1 h` is still `1852 m`, not
  `1 nmi`; `1 m * 1 cm2` still `0.1 l`, not `1 dl`): found by the final review, which
  compared about 300 lines against `main`. `1 kWh / 100 W` is `36000 s` (`in h` gives
  `10 h`). `1 GB / 1 s`, an error before, is now `1 GB/s`.
- **`L/100km`** is fuel economy only when written with no spaces; `10 l / 100 km` stays an
  area. Shown alone it is now fuel economy (`10 l/100km` was `0.1 mm2`), the one old answer
  that changed on purpose. Multiplied or divided by another unit it is litres per 100 km as
  before, so trip and range sums keep working: `8 l/100km * 250 km` is `20 l`,
  `50 l / (8 l/100km)` is `625 km` (final review). `km/L` and `mpg` do not multiply with
  other units.
- **Zones of several words beat one-word units after `in`:** `5pm in St Louis` stays a
  city, though `st` is now the stone.
- **`Gb` (gigabit)** also matches the IANA link `GB` (Britain) in any case. `now in Gb` now
  means gigabit; `now in gb` and `GB` behave as before.
- **`mpg`** is exactly a mile per US gallon (1.609344 / 3.785411784 km/L); the spec's
  rounded 0.425143707 missed the tenth digit.
- **City short names:** the list in `zones.rs` (`SHORT_NAMES`), 72 of them. `mad`, `ist`,
  `sin`, `mil` and `was` are left out; a test checks the rest against currencies, units,
  months, keywords, abbreviations, colours and zone names.
- **No `dec`** for decimal, since it is December.
- **`base N`** takes a plain whole number 2–36; other bases than 2, 8 and 16 print bare
  digits.
- **The sheet swatch is a contract change.** `line-annotation` gains `swatch`, edited in
  place in WIT 0.1 (`FROZEN.md`, 2026-11-22). The demo module and the tests that build
  annotations by hand were updated.

### Fixed after the report (the six deferred minors)

- `hsl()` takes an angle only for its hue (`hsl(0, 100%, 1 rad)` is an error).
- Alpha that rounds to 255 prints as opaque (`rgba(255, 136, 0, 0.999)` is `#ff8800`).
- A search with a colour and a comment (`#ff8800 #note`) gets no answer, like any query with
  a comment.
- `transparent` is named in its caption (`Color · transparent`).
- `issue #1234` keeps its old error (see the `#` ruling above).
- A hue that rounds to 360 prints as 0.

## User checklist

Run from the worktree root. `just try approve` builds everything, starts the launcher in a
throwaway home (`$XDG_RUNTIME_DIR/tk-try`) with calc approved, and deletes the home on exit.
Only one `just try` may run on the machine at a time. Open the launcher with your hotkey or
`just toggle` from another terminal; `just log` follows the daemon's log.

```bash
just try approve
```

| # | What | Expected | Result |
|---|---|---|---|
| C1 | Colour tile | Type `#ff8800 in hsl`: the tile shows `hsl(32, 100%, 50%)` with the caption `Color` and an orange square at its right edge, with a thin border. | |
| C2 | Bare hex | `#ff8800` answers `rgb(255, 136, 0)`; `#f80` the same. | |
| C3 | Named colour | `orange in rgb` answers `rgb(255, 165, 0)`, caption `Color · orange`; `orange` alone gets no calculator tile. | |
| C4 | Transparency | `#ff880080 in hex`: the square shows the colour over a checkerboard. | |
| C5 | Copy | On the `#ff8800` tile, Ctrl+K lists `Copy as HEX`, `RGB`, `HSL`, `OKLCH`; `Copy as OKLCH` puts `oklch(0.744 0.181 56.5)` on the clipboard. Enter still copies the shown result. | |
| C6 | Home | Add the `#ff8800 in hsl` tile to Home: it shows the square there too, and its Ctrl+K has the four copy actions. | |
| C7 | Sheet swatch | Open "Calculator", type `brand = #ff8800` and `brand in hsl` on two lines: both notes have a small orange square right before the text. | |
| C8 | Comments | In the sheet, `# note`, `#todo`, `#add later` and `5 + 3 #fee` look as before (no note, no note, no note, `8`). `#fee` alone shows a pale pink square and `rgb(255, 238, 238)`. | |
| C9 | Bases | `255 in base 36` → `73`; `255 in hexadecimal` → `0xff`; `5 in binary` → `0b101`. | |
| C10 | New units | `1 atm in hPa` → `1013.25 hPa`; `10 L/100km in mpg` → `23.52145833 mpg`; `2.4 GHz in MHz` → `2400 MHz`; `100 Mbps in MB/s` → `12.5 MB/s`. | |
| C11 | Units together | `2 kW * 3 h` → `6 kWh`; `1 GB / 100 Mbps` → `80 s`; `10 MB/s * 1 min` → `600 MB`. | |
| C12 | City short names | `now in spb` answers Moscow time (`… MSK`); `5pm bcn in msk` answers `…, 19:00 MSK` (in November); `100 usd in mad` is still a dirham conversion. | |
| C13 | St Louis | `5pm in St Louis` answers a time ending in `CST`; `10 st in kg` → `63.5029318 kg`. | |

Stop with Ctrl+C in the `just try` terminal; the throwaway home is deleted on exit.
