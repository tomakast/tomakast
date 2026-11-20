# Calc conversions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Calc converts about as much as Raycast does: more units (temperature, area, volume, speed, data, angle, calendar months and years), dates and times with time zones, fiat and crypto currencies on fetched rates, and number bases. All of it works in the search answer and in sheets.

**Architecture:**
- **Engine.** Calc's own pure engine (`plugins/calc/src/engine/`) grows: a `Value` enum (number with unit, based number, moment), a richer unit table, a zone module over a bundled tz database (`jiff`), and a currency table.
- **Context.** The engine still does no I/O. Every evaluation takes a `Context`: the current time, the user's zone and the last stored rates.
- **Rates.** The plugin (`src/rates.rs`) fetches rates over `http` at `init` and on one 10-minute `timers` tick. It keeps them in memory and in `storage`, one key per feed. Search only reads the in-memory copy.
- **Host.** One test-only seam: a fake HTTP responder in `crates/plugin-host` (feature `testing`). With it, calc's fixed `https` URLs are tested end to end without the network.

**Tech Stack:** Rust 2024, the guest SDK `crates/plugin-sdk` (`tomakast-plugin`), `jiff` 1.0.1 with its bundled tzdb (the only new dependency), and the plugin-host test harness (`crates/plugin-host/tests/support`). Rate bodies are read by a small hand-written JSON reader: `serde_json` would cost 44 KiB of the size gate.

**Spec:** `docs/superpowers/specs/2026-11-19-calc-conversions-design.md`. Binding inputs:
- the WIT contract `crates/plugin-api/wit/since_v0.1.0/plugin.wit` (http, timers, preferences, the background budget);
- `docs/experiments/2026-11-plan2b2b-acceptance.md`: Ruling 5 (depth 64), Ruling 6d (date and phone shapes), Ruling 8 (write budget), the 100 ms search deadline and the 1 KiB line rule.

## Global Constraints

**Contract and engine**
- **WIT.** `plugin.wit` is not edited (its `FROZEN` hash stays). Calc uses `preferences`, `http` and `timers` as they are.
- **Engine purity.** `src/engine/` reads no clock, no storage and no SDK. Time, zone and rates arrive in `Context`, so every exact date value is tested natively with an injected `now`.
- **English only.** Month names, units, keywords and city names are English.

**Search**
- Search never waits for the network, never starts a fetch and writes nothing for rates. Its existing quiet flush of waiting sheet edits (`flush_quietly`) stays.
- Search keeps the 2b-2b margins: ≤ 10 ms for any query ≤ 1 KiB (`search_is_fast`), the 1 KiB line rule, and depth ≤ 64 for every node kind (Ruling 5).
- The Ruling 6d shapes and `looks_like_date` stay byte-identical. ISO dates (`2026-12-31`) are not a new form.
- **The bare-value rule.** A bare value gets no answer; a conversion or an operation does.
  - No answer: `10 km`, `100 usd`, `$100`, `10 C`, `31 dec`, `dec 31`, `5pm`, `15:00 MSK`.
  - Answers: `0xff` and the other non-decimal literals, because writing a number in another base is itself a conversion (`0xff` → `255`).
  - Answers, as an explicit exception like `pi` (the coordinator's call: the user asked for "a `now` variable", and Raycast answers it): the bare words `now`, `time`, `today`, `tomorrow` and `yesterday`. They answer the current date or time in the user's zone, with the zone shown (`now` → `Thu, 19 Nov 2026, 17:42 MSK`; `today` → `Thu, 19 Nov 2026` with the subtitle `today · Europe/Moscow`). These words match in any case (`Now`, `NOW`): they are not unit names, so the case-sensitive unit rule does not apply. `now in` and `today in history` still error and get none.

**Formats**

| Value | Example |
|---|---|
| moment | `Thu, 19 Nov 2026, 17:42 MSK`, with the tz database abbreviation, which can be numeric (`+04`) |
| date-only moment | `Thu, 19 Nov 2026` |
| duration | `42 days`: `day`, `week`, `month` and `year` are plural unless the number is exactly 1 |
| fiat | 2 decimals, the integer part grouped by 3 with U+0020, then the code: `9 312.50 RUB` |
| fiat below 0.01 (not 0) | as crypto: `0.001 USD` |
| crypto | 8 significant digits, trailing zeros trimmed, grouped the same way: `0.052238806 BTC` |
| money of 1e15 or more | the existing `number()` rule and the code |
| based number | `0xff`, `0b11111111`, `0o377`, `-0xff` |

**Names**
- **Reserved names.** The 20 unit names that existed before this plan (`mm cm m km in ft yd mi mg g kg t oz lb ms s min h day week`) stay reserved as variables.
- **Soft names.** Every new unit name and alias, currency code, ticker, month name, city and the words `now today tomorrow yesterday time` are soft:
  - a variable of that name wins wherever a name may stand;
  - the unit or currency meaning applies only right after a number or after `in`/`to`.
  - `until` is reserved.
- **Case.**
  - Physical units are case-sensitive (exact canonical name or alias).
  - Currency codes and tickers, zone names, cities and abbreviations match in any case.
  - Keywords are lower-case (`in to of mod until hex bin oct decimal date unix am pm`).
- **`in`/`to` target precedence:**
  1. `hex` / `bin` / `oct` / `decimal`;
  2. `date` / `unix`;
  3. physical unit;
  4. currency;
  5. zone.
  - Anything else is `unknown unit '<word>'`, the existing message.
  - The one intended shadowing: the volume unit `cup` wins over the Cuban peso in lower case (`CUP` is the peso).

**Rates and storage**
- **URLs.**
  - Fiat: `https://open.er-api.com/v6/latest/USD`.
  - Crypto: `https://api.coingecko.com/api/v3/simple/price?ids=<ids, comma-joined, table order>&vs_currencies=usd`.
- **Permissions.** Exactly `search`, `storage`, `clipboard`, `timers` and `http = ["open.er-api.com", "api.coingecko.com"]`. The approval prompt asks again once (expected).
- **Refresh.** One repeating timer of 10 min (`TICK_MS = 600_000`). With a slack of 60 s:
  - fiat is due when missing or ≥ 1 h old;
  - crypto is due when missing, or when ≥ 10 min old and either at `init` or used within the last 30 min.
  - The age of rates is the time calc received them (not the API's update time). An answer is stale after 24 h.
- **Write budget (Ruling 8).** Calc's `WRITE_BUDGET` stays 16 per second with `RESERVE` 4.
  - A rate write is one `set` per feed per successful answer, and it never uses the reserve.
  - A rate write that finds no room, or fails on the quota, keeps the rates in memory. It never drops a sheet and never shows a toast.
- **Background budget.** 1 s per 10 s. A tick starts at most two fetches. An answer parses ≤ 8 KiB and makes one write.
- **Time zone preference.** `timezone`, text, default `UTC`. It accepts IANA names, cities and abbreviations. An unknown value falls back to UTC, and the answer shows `UTC`.
- **The answer shows the zone it used.** A moment with a time shows its abbreviation. A search answer whose value used the home zone but shows none appends ` · <IANA name>` to its subtitle: a date-only moment, or a duration counted from `today`, `now` or a date (`days until 31 dec · Europe/Moscow`). This is also how a wrong default (`UTC`) becomes visible.

**Size (measured while planning, at `7299987`, release profile, wasm32-wasip2)**

| Build | `calc.wasm` | Delta |
|---|---|---|
| today | 326,868 bytes | |
| with jiff 1.0.1 (`default-features = false, features = ["std", "tzdb-bundle-always"]`) | 832,520 | +505,652 |
| with chrono-tz 0.10.4 (all zones, `case-insensitive` on) | 1,280,064 | +953,196 |
| with serde + serde_json (two derived structs) | 370,985 | +44,117: not taken |

- **jiff is chosen:** it is half the size, and its API fits: a case-insensitive `tz::db().get`, `available()`, calendar `Span` arithmetic with month-end clamping, and `strftime` `%Z`.
- **Gate.** If `calc.wasm` exceeds 982,228 bytes (326,868 + 640 KiB), stop and ask the user before going on. jiff leaves about 147 KiB of that for all new engine code and tables, so Tasks 2, 4 and 6 each check the size.
- **No parser crates.** The `Cargo.toml` comment ("The engine is hand-written: no parser crates, so the wasm stays small") stays true: jiff is a date library, and the rate bodies are read by `src/json.rs`.

**Tests and commits**
- **No network in tests.** Every calc host test runs with the offline responder unless it installs a fake one.
- **Commands** (calc is its own cargo project; CI sets `CARGO_TARGET_DIR=target/plugins` for it):
  - `cargo fmt --check --manifest-path plugins/calc/Cargo.toml`;
  - `cargo clippy --manifest-path plugins/calc/Cargo.toml --all-targets -- -D warnings`, and the same with `--target wasm32-wasip2`;
  - `cargo test --release --manifest-path plugins/calc/Cargo.toml` (the timing tests are ignored in debug);
  - the wasm build: `CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 --manifest-path plugins/calc/Cargo.toml`;
  - the host: `cargo test -p tomakast-plugin-host --test calc --test http --test wit_coverage`.
- **Commits.** Another session shares this worktree and its index. Commit by path: `git add <paths>` and then `git commit -m <msg> -- <paths>`, so nothing another session staged is swept in. Never `git add -A`, never stash, no push, no attribution lines.

## Review Focus

1. **Rates missing, stale or malformed.** On the first start offline, after days without network, or when the API answers HTML, 429 or `{"result":"error"}`:
   - a currency query gets no search answer, and a sheet line says "no exchange rates yet";
   - stale rates convert and the subtitle says "rates from N days ago";
   - nothing bad is stored, and no toast appears.
   - Tests: Task 4 `rates_older_than_a_day_say_so`, `without_rates_a_currency_query_has_no_answer`, `malformed_stored_rates_are_ignored`; Task 6 `bad_rate_answers_store_nothing`.
2. **Rates arriving with storage full or the write budget spent.** Calc keeps the rates in memory and still converts. No sheet is dropped and no toast is shown. Tests: Task 6 `rates_never_drop_a_sheet_when_storage_is_full` and `background_writes_leave_the_reserve`.
3. **A `timezone` preference the user writes by hand** (`Moscow`, `europe/moscow`, `MSK`, `Mars/Base`, empty). The first three resolve to Moscow. Anything unknown is UTC, and the answer visibly says UTC: bare `now` ends in ` UTC`, and date-only answers name it in the subtitle (`today · UTC`, `days until 1 jan · UTC`). Tests: Task 3 `now_uses_the_time_zone_preference` and `date_answers_name_their_zone`.
4. **Calendar edges.**
   - Month-end clamping: `31 jan 2026 + 1 month` is 28 Feb.
   - A civil day across a DST change keeps the wall time; 24 h does not.
   - Two dates across a DST change are whole days apart, not "23 h".
   - Test: Task 2 `calendar_months_and_dst`.
5. **Existing sheets that use a word that is now a unit, currency, month or time word** (`hours = 40`, `year = 2026`, `c = 3`, `time = 5`, `try = 2`) keep working. Tests: Task 1 `new_unit_names_stay_free_for_variables`, Task 2 `time_words_stay_free_for_variables` and Task 4 `currency_codes_stay_free_for_variables`.

---

## File structure

| File | Responsibility | Tasks |
|---|---|---|
| `plugins/calc/src/engine/mod.rs` | `Value`, `Base`, `Context`, `Sheet::eval`, `eval_query`, `is_mathy`, the engine test tables | 1, 2, 4 |
| `plugins/calc/src/engine/units.rs` | the physical unit table, aliases, affine conversion, plural display | 1 |
| `plugins/calc/src/engine/lexer.rs` | based literals, Unicode words, `°`, clock times, currency symbols | 1, 2, 4 |
| `plugins/calc/src/engine/parser.rs` | `Target`, `Radix`, `Clock`, `Date`, `Until`, money prefix, the `in` disambiguation | 1, 2, 4 |
| `plugins/calc/src/engine/eval.rs` | the arithmetic of every value kind | 1, 2, 4 |
| `plugins/calc/src/engine/format.rs` | numbers, money, based numbers (moments delegate to `time`) | 1, 4 |
| `plugins/calc/src/engine/zones.rs` (new) | `ZoneId`, the name, city and abbreviation lookup over jiff's bundled tzdb | 2 |
| `plugins/calc/src/engine/time.rs` (new) | `Moment` and its arithmetic and formatting | 2 |
| `plugins/calc/src/engine/currency.rs` (new) | fiat codes, tickers → CoinGecko ids, symbols, `Rates`, `Table`, `Used` | 4 |
| `plugins/calc/src/rates.rs` (new) | stored rate encoding, the in-memory feeds, refresh policy, fetching and answers | 4, 6 |
| `plugins/calc/src/json.rs` (new) | a small JSON reader for the two rate bodies | 6 |
| `plugins/calc/src/views.rs` | the views take a `Context`; the stale-rates subtitle | 2, 4 |
| `plugins/calc/src/lib.rs` | building the `Context`, the zone preference, the `init`, timer and HTTP events | 2, 3, 6 |
| `plugins/calc/src/sheets.rs` | `WriteWindow` extracted; `set_background` for rate writes; the key list and the Ruling 8 text | 6 |
| `plugins/calc/plugin.toml`, `Cargo.toml` | the preference, the permissions, `jiff` | 2, 3, 6 |
| `plugins/calc/tests/fixtures/` (new) | captured `er-api-latest-usd.json` and `coingecko-simple-price.json` | 6 |
| `crates/plugin-host/src/imports/http.rs`, `src/plugin.rs`, `src/imports/mod.rs`, `src/lib.rs` | the test-only `Responder` | 5 |
| `crates/plugin-host/tests/support/harness.rs` | `PluginSetup` (seed, prefs, responder) kept for `reload` | 3, 5 |
| `crates/plugin-host/tests/calc.rs`, `tests/http.rs` | the host tests | 3–6 |
| `docs/plugins/writing-a-plugin.md`, `docs/experiments/2026-11-plan2b2b-acceptance.md`, `docs/experiments/2026-11-calc-conversions-acceptance.md` (new) | the docs and the acceptance | 7 |

---

### Task 1: More units, number bases and the `Value` enum

**Files:**
- Modify: `plugins/calc/src/engine/mod.rs`, `units.rs`, `lexer.rs`, `parser.rs`, `eval.rs`, `format.rs`
- Modify: `plugins/calc/src/views.rs` (only where the `Value` struct was destructured)
- Test: the `#[cfg(test)]` modules of the same files

**Interfaces:**
- Produces, in `engine/mod.rs`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq)]
  pub enum Value {
      /// A number with an optional unit.
      Num { number: f64, unit: Option<Unit> },
      /// A whole number shown in base 2, 8 or 16 (`255 in hex`); a plain number in arithmetic.
      Based { number: f64, base: Base },
  }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Base { Bin, Oct, Hex }
  impl Value {
      pub fn plain(number: f64) -> Value;
      pub fn with_unit(number: f64, unit: Option<Unit>) -> Value;
      /// The number and unit an arithmetic operand stands for (`Based` is a plain number).
      pub fn quantity(&self) -> Option<(f64, Option<Unit>)>;
  }
  ```
- Produces, in `units.rs`:
  ```rust
  pub enum Dim { Length, Mass, Time, Temperature, Area, Volume, Speed, Data, Angle }
  pub struct Unit { pub name: &'static str, pub dim: Dim, pub factor: f64, pub offset: f64 }
  /// Canonical name or alias, case-sensitive.
  pub fn lookup(name: &str) -> Option<Unit>;
  /// The 20 names that were units before this plan: they cannot be variables.
  pub fn reserved(name: &str) -> bool;
  /// Affine: base = n * factor + offset.
  pub fn convert(number: f64, from: Unit, to: Unit) -> Option<f64>;
  /// Factors only, for `+`, `-` and `/`.
  pub fn convert_interval(number: f64, from: Unit, to: Unit) -> Option<f64>;
  /// The name to print: "days" for `day` unless `number == 1.0`.
  pub fn display(unit: Unit, number: f64) -> &'static str;
  ```
- Produces, in `parser.rs`:
  - `Expr::Radix(f64)`: a literal written as `0x…`, `0b…` or `0o…`;
  - `Expr::Convert(Box<Expr>, Target)` with `pub enum Target { Unit(Unit), Base(Option<Base>) }`, where `None` is `decimal`.

The unit table. The base unit of each family has factor 1. Aliases are case-sensitive. The `c` and `f` aliases are soft like every new name.

| Dim (base) | Canonical: factor (aliases) |
|---|---|
| Length (m) | the existing 8, plus aliases: `millimeter(s) millimetre(s) centimeter(s) centimetre(s) meter(s) metre(s) kilometer(s) kilometre(s) inch inches foot feet yard(s) mile(s)` |
| Mass (g) | the existing 6, plus `milligram(s) gram(s) kilogram(s) kilo(s) tonne(s) ounce(s) pound(s) lbs` |
| Time (s) | the existing 6, plus `month` 2 629 746, `year` 31 556 952; aliases `millisecond(s) second(s) sec secs minute(s) mins hour(s) hr hrs days weeks months years yr yrs` |
| Temperature (K) | `°C` 1, offset 273.15 (`C c celsius degC`); `°F` 5/9, offset 459.67 × 5/9 (`F f fahrenheit degF`); `K` 1, 0 (`kelvin kelvins`) |
| Area (m²) | `mm2` 1e-6 (`mm²`), `cm2` 1e-4 (`cm²`), `m2` 1 (`m²`), `km2` 1e6 (`km²`), `ha` 1e4 (`hectare(s)`), `acre` 4046.8564224 (`acres`), `ft2` 0.09290304 (`ft² sqft`), `in2` 0.00064516 (`in²`), `mi2` 2 589 988.110336 (`mi²`) |
| Volume (l) | `ml` 0.001 (`mL milliliter(s) millilitre(s)`), `l` 1 (`L liter(s) litre(s)`), `m3` 1000 (`m³`), `tsp` 0.00492892159375 (`teaspoon(s)`), `tbsp` 0.01478676478125 (`tablespoon(s)`), `floz` 0.0295735295625, `cup` 0.2365882365 (`cups`), `pt` 0.473176473 (`pint(s)`), `qt` 0.946352946 (`quart(s)`), `gal` 3.785411784 (`gallon(s)`) |
| Speed (m/s) | `m/s` 1 (`mps`), `km/h` 1/3.6 (`kph kmh`), `mph` 0.44704, `kn` 1852/3600 (`knot(s) kt`) |
| Data (bit) | `bit` 1 (`bits`), `B` 8 (`byte(s)`), `KB` 8e3 (`kB kilobyte(s)`), `MB` 8e6 (`megabyte(s)`), `GB` 8e9 (`gigabyte(s)`), `TB` 8e12 (`terabyte(s)`), `KiB` 8·2¹⁰ (`kibibyte(s)`), `MiB` 8·2²⁰ (`mebibyte(s)`), `GiB` 8·2³⁰ (`gibibyte(s)`), `TiB` 8·2⁴⁰ (`tebibyte(s)`) |
| Angle (rad) | `deg` π/180 (`° degree(s)`), `rad` 1 (`radian(s)`) |

**Lexer changes:**
- A word starts with a Unicode alphabetic character, `_` or `°`, and continues with alphanumerics (`²` and `³` count as numeric) or `_`.
- `0x`, `0b` and `0o` followed by at least one valid digit form one `Tok::Num(value, text)`. A value above 2^53 is `Err("too large")`.
- A plain `0x` stays `0` followed by the word `x`.

**Parser changes:**
- **Compound units.** Before `split_inch_conversion`, `Ident(a) Slash Ident(b)` becomes `Ident("a/b")` when `units::lookup("a/b")` is a unit. So `100 km/h` and `10 km / h` are speeds, while `10 km / 500 m` stays a ratio.
- **`in` after a number.** A number followed by `in` takes it as the inch unit unless the word after `in` names a base, or (from Task 2) `date`, `unix` or a place, or (from Task 4) a currency. So `255 in hex` is a base conversion and `12 in` stays 12 inches.
- **`split_inch_conversion`** keeps using `units::lookup` only, for physical units.
- **Soft unit names.** A soft unit name in name position is an `Expr::Name`. At evaluation it reads the variable, or fails with `'<name>' is a unit; put a number before it`.

**Evaluation changes:**
- Add, subtract and divide go through `convert_interval`, not inline factor math.
- `sin`, `cos` and `tan` accept a plain number (radians) or an angle.
- `Convert` to a `Base` needs a plain whole number with |n| ≤ 2^53. The errors are `<kw> needs a plain number`, `<kw> needs a whole number` and `too large for <kw>`, where `<kw>` is `hex`, `bin` or `oct`.

- [ ] **Step 1: Write the failing tests in `engine/mod.rs`**

```rust
#[test]
fn more_units() {
    table(&[
        ("1 ha in m2", "10000 m2"), ("1 acre in m2", "4046.856422 m2"),
        ("1 mi2 in km2", "2.58998811 km2"), ("1 m² in cm2", "10000 cm2"),
        ("1 gal in l", "3.785411784 l"), ("1 cup in ml", "236.5882365 ml"),
        ("3 tsp in tbsp", "1 tbsp"), ("2 pt in qt", "1 qt"), ("1 m3 in l", "1000 l"),
        ("8 floz in cup", "1 cup"),
        ("100 km/h in mph", "62.13711922 mph"), ("10 m/s in km/h", "36 km/h"),
        ("1 kn in km/h", "1.852 km/h"), ("10 km / h", "10 km/h"), ("10 km / 500 m", "20"),
        ("1 GB in MB", "1000 MB"), ("1 GiB in MiB", "1024 MiB"),
        ("1 GB in GiB", "0.9313225746 GiB"), ("1 B in bit", "8 bit"), ("1 TiB in GiB", "1024 GiB"),
        ("180 deg in rad", "3.141592654 rad"), ("1 rad in deg", "57.29577951 deg"),
        ("sin(90 deg)", "1"), ("cos(180°)", "-1"), ("sin(5 km)", "error: sin needs a plain number"),
        ("1 year in day", "365.2425 days"), ("1 month in day", "30.436875 days"),
        ("1 week in day", "7 days"), ("1 day in h", "24 h"), ("1 week in days", "7 days"),
        ("5 km in GB", "error: cannot convert km to GB"),
    ]);
}

#[test]
fn temperature_is_affine() {
    table(&[
        ("0 C in F", "32 °F"), ("100 °C to °F", "212 °F"), ("100 F to C", "37.77777778 °C"),
        ("-40 C in F", "-40 °F"), ("300 K in C", "26.85 °C"), ("0 K in F", "-459.67 °F"),
        ("20 C + 5 C", "25 °C"), ("20 C + 9 F", "25 °C"), ("20 C - 5 K", "15 °C"),
        ("100 f to c", "37.77777778 °C"), ("20 celsius in fahrenheit", "68 °F"),
        ("5 C in km", "error: cannot convert °C to km"),
    ]);
}

#[test]
fn aliases_and_plurals() {
    table(&[
        ("5 kilometers in miles", "3.106855961 mi"), ("2 hours in minutes", "120 min"),
        ("3 feet in inches", "36 in"), ("1 kilogram in pounds", "2.204622622 lb"),
        ("90 seconds in min", "1.5 min"), ("2 litres in ml", "2000 ml"),
        ("1 gigabyte in megabytes", "1000 MB"), ("1 kph in m/s", "0.2777777778 m/s"),
        ("1 day", "1 day"), ("2 day", "2 days"), ("0.5 week", "0.5 weeks"),
    ]);
}

#[test]
fn number_bases() {
    table(&[
        ("0xff", "255"), ("0b1010 + 1", "11"), ("0o17", "15"),
        ("255 in hex", "0xff"), ("255 in bin", "0b11111111"), ("255 in oct", "0o377"),
        ("0xff in decimal", "255"), ("-255 in hex", "-0xff"), ("12 in hex", "0xc"),
        ("0xff in hex + 1", "256"), ("12 in", "12 in"), ("12 in cm", "30.48 cm"),
        ("2.5 in hex", "error: hex needs a whole number"),
        ("5 km in hex", "error: hex needs a plain number"),
        ("2 ^ 60 in hex", "error: too large for hex"), ("0x", "error: unexpected 'x'"),
    ]);
}

#[test]
fn new_unit_names_stay_free_for_variables() {
    assert_eq!(run("hours = 40\nrate = 50\nhours * rate"), ["hours = 40", "rate = 50", "2000"]);
    assert_eq!(run("year = 2026\nyear + 1"), ["year = 2026", "2027"]);
    assert_eq!(run("c = 3\nc * 2\n10 c in f"), ["c = 3", "6", "50 °F"]);
    table(&[
        ("hours", "error: 'hours' is a unit; put a number before it"),
        ("day = 5", "error: 'day' is a unit name and cannot be a variable"),
    ]);
}
```

Also make these deliberate changes to the existing tests:
- In `unit_conversion_and_arithmetic`: `("1 week in day", "7 days")` (was `"7 day"`).
- In `query_heuristic`, add to the answers: `("100 F to C", "37.77777778 °C")`, `("100 km/h in mph", "62.13711922 mph")`, `("1 GiB in MiB", "1024 MiB")`, `("255 in hex", "0xff")`, `("0xff", "255")` and `("0b1010", "10")`.
- In `query_heuristic`, add to the no-answer list: `"10 C"`, `"1 GB"`, `"5 kilometers"`, `"C"`, `"hex"`, `"0x"` and `"in hex"`.
- In `search_is_fast`'s `PIECES`, add `"C"`, `"°F"`, `"km/h"`, `"GiB"`, `"0xff"`, `"0b1"`, `"hex"`, `"deg"`, `"hours"` and `"m²"`.
- In `pathological_inputs`, add `format!("0x{}", "f".repeat(1000))` and `"°".repeat(500)`, and assert `one(&format!("0x{}", "f".repeat(1000))) == "error: too large"`.

In `units.rs`:
- `lookup_is_exact` also asserts `lookup("kilometers").map(|u| u.name) == Some("km")` and `lookup("Kilometers").is_none()`.
- `names_are_unique` covers every canonical name and alias.
- New `reserved_is_the_old_set`: `reserved("km") && reserved("week") && !reserved("hours") && !reserved("C") && !reserved("cup")`.

In `lexer.rs`, `based_literals_and_words` asserts:
- `lex("0xff") == Ok(vec![Tok::Num(255.0, "0xff".into())])`, and the same shape for `"0b1010"` (10.0) and `"0o17"` (15.0);
- `lex("0x") == Ok(vec![Tok::Num(0.0, "0".into()), Tok::Ident("x".into())])`;
- `lex("0x20000000000001") == Err("too large".into())`;
- `lex("°C")`, `lex("m²")` and `lex("São")` are each a single `Ident`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path plugins/calc/Cargo.toml`
Expected: FAIL. The new tables fail (`unexpected 'ha'`, `7 day`), and the lexer test fails on `0xff`.

- [ ] **Step 3: Implement the interfaces above**

`Value` becomes the enum. `format(&Value)` handles `Based`, and uses `units::display` for the unit name. `views.rs` needs no logic change beyond compiling. `is_mathy` returns `true` for `Expr::Radix`. `Expr::Radix` and `Target` count toward depth like `Lit` and `Convert`.

- [ ] **Step 4: Run every check**

Run: the calc fmt, clippy (both targets) and `cargo test --release` commands of the Global Constraints.
Expected: all pass. `search_is_fast` and `sheet_of_1000_lines_is_fast` stay within 10 ms and 50 ms.

- [ ] **Step 5: Commit**

```bash
git add plugins/calc/src/engine plugins/calc/src/views.rs
git commit -m "feat(calc): temperature, area, volume, speed, data and angle units; number bases" -- plugins/calc/src/engine plugins/calc/src/views.rs
```

---

### Task 2: Dates, times and time zones in the engine

**Files:**
- Create: `plugins/calc/src/engine/zones.rs`, `plugins/calc/src/engine/time.rs`
- Modify: `plugins/calc/Cargo.toml`, `engine/mod.rs`, `lexer.rs`, `parser.rs`, `eval.rs`, `format.rs`, `plugins/calc/src/views.rs`, `plugins/calc/src/lib.rs`, `plugins/calc/Cargo.lock`
- Test: the same modules, plus the views tests

**Interfaces:**
- Consumes: `Value`, `Target`, `units::lookup` and `units::display` from Task 1.
- Produces, in `zones.rs`:
  ```rust
  /// An index into jiff's bundled zone names (`tz::db().available()`, 598 of them), kept in a `OnceLock<Vec<String>>`.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct ZoneId(u16);
  pub fn utc() -> ZoneId;
  /// An IANA name (any case, `_` or space), a city, or an abbreviation; trimmed. `None` for anything else.
  pub fn find(name: &str) -> Option<ZoneId>;
  /// The IANA name.
  pub fn name(id: ZoneId) -> &'static str;
  /// Cached per id in a `thread_local`.
  pub(crate) fn tz(id: ZoneId) -> jiff::tz::TimeZone;
  /// The longest zone name over at most 5 tokens at `toks[0..]` (words joined by a space,
  /// or words joined by `/`), and how many tokens it took.
  pub(crate) fn at(toks: &[Tok]) -> Option<(ZoneId, usize)>;
  ```
- Produces, in `time.rs`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq)]
  pub struct Moment { pub unix: i64, pub zone: ZoneId, pub date_only: bool }
  pub fn now(ctx: &Context) -> Moment;
  /// `today` is 0, `tomorrow` 1, `yesterday` -1: midnight in `ctx.zone`, date-only.
  pub fn day(ctx: &Context, offset_days: i64) -> Result<Moment, String>;
  /// Today's date in `zone`, at that wall time.
  pub fn at_clock(ctx: &Context, hour: u8, minute: u8, zone: ZoneId) -> Result<Moment, String>;
  /// Midnight in `ctx.zone`, date-only; a missing year is the current one.
  pub fn on_date(ctx: &Context, day: u8, month: u8, year: Option<i32>) -> Result<Moment, String>;
  /// See the rules below.
  pub fn add(m: Moment, amount: f64, unit: Unit) -> Result<Moment, String>;
  /// `a - b` as a duration value.
  pub fn between(a: Moment, b: Moment) -> Value;
  pub fn format(m: &Moment) -> String;
  ```
- Produces, in `engine/mod.rs`:
  ```rust
  pub enum Value { Num { .. }, Based { .. }, Moment(Moment) }
  pub struct Context<'a> { pub now: i64, pub zone: ZoneId, used: Cell<Used>, /* rates: Task 4 */ _p: PhantomData<&'a ()> }
  impl Context<'static> { pub fn new(now: i64, zone: ZoneId) -> Context<'static>; }
  impl Context<'_> { pub fn used(&self) -> Used; }
  /// What an evaluation relied on; Task 4 adds `fiat` and `crypto`.
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub struct Used { pub zone: bool }
  impl Sheet { pub fn eval(text: &str, ctx: &Context) -> Vec<LineResult>; }
  pub fn eval_query(query: &str, ctx: &Context) -> Option<Value>;
  ```
- Produces, in `parser.rs`:
  - `Tok::Clock(u8, u8, String)`, which the lexer makes from `\d{1,2}:\d{2}`;
  - `Expr::Clock { hour: u8, minute: u8, zone: Option<ZoneId> }`, `Expr::Date { day: u8, month: u8, year: Option<i32> }` and `Expr::Until(Unit, Box<Expr>)`;
  - `Target::{Zone(ZoneId), Date, Unix}` added.
- Produces, in `views.rs`: every view function takes the context: `answer(query, ctx)`, `pad(id, text, ctx)`, `pad_update(id, text, revision, ctx)`, `annotations(text, ctx)` and `last_result(text, ctx)`. `answer` appends ` · <IANA name>` to the subtitle when `ctx.used().zone` and the value is not a moment with a time (Global Constraints).
- Produces, in `lib.rs`: `fn with_context<R>(f: impl FnOnce(&Context) -> R) -> (R, Used)`, which builds `Context::new(sheets::unix_now(), zones::utc())` and returns what the evaluation used. Task 3 changes the zone; Task 4 lends the rates.
- `used.zone` is set whenever evaluation reads `ctx.zone`: `now`, `today`, a date, a clock without a zone, `until`, `to date`.

**Grammar:**
- `now`, `time`, `today`, `tomorrow` and `yesterday` are names. A variable wins (variables stay case-sensitive); otherwise they are moments, matched in any case (`Now`, `TODAY`).
- `<number> am|pm` and `<clock> [am|pm]` are a clock time. Hours are 0–23, or 1–12 with am/pm, and minutes 0–59. Anything else is `not a time`. An optional zone may follow: `zones::at` on the next tokens.
- `<day> <month> [<year>]` and `<month> <day> [<year>]` are a date.
  - Month names: `jan … dec`, `sept` and the full names, in any case.
  - The year is a number written with exactly 4 digits.
  - `31 feb` is `not a date`.
- `<time unit word> until <expr>`: the operand is parsed one nesting level deeper with min binding power 2.
  - `day` and `week` count civil days from today. A date without a year that is already past rolls to next year.
  - The other units count exact seconds from now.
- Targets: `in|to <zone>` keeps the instant in a new zone. `to date` makes a plain number into unix seconds in `ctx.zone`. `to unix` makes a moment into a plain number.

**Arithmetic (`time::add` and `eval`):**
- `Moment ± time quantity` is a moment. `time quantity + Moment` is the same.
  - A whole number of days or weeks adds civil days (DST-safe).
  - A whole number of months or years adds calendar months (clamped to the month end).
  - Anything else adds exact seconds (month and year at their average length) and clears `date_only`.
  - Out of jiff's range is `date out of range`.
- `Moment - Moment`:
  - both date-only: whole civil days, in `day`;
  - otherwise exact, shown in `day` when |d| ≥ 1 day, else `h` when ≥ 1 h, else `min`.
- **Errors** (exact text):
  - `cannot add two dates`;
  - `cannot multiply a date` (also for `/`, `^` and `mod`);
  - `cannot add a date and a number` (also for `-`);
  - `cannot subtract a date from a number`;
  - `cannot convert a date to <unit>`;
  - `cannot convert <unit> to a date`;
  - `cannot convert <unit|a plain number> to <IANA name>`.
- **Search.** `is_mathy` is `true` for `Until` and for a bare `Expr::Name` of `now`, `time`, `today`, `tomorrow` or `yesterday` in any case (next to `pi`). It is `false` for `Clock` and `Date`. Search has no variables, so a bare time word there is always the moment.

**Zone tables in `zones.rs`:**
- `CITIES: &[(&str, &str)]` maps lower-case names to IANA names. It holds at least 150 capitals and large cities whose names are not the last part of an IANA name: `washington`, `san francisco`, `beijing`, `delhi`, `new delhi`, `mumbai`, `saint petersburg`, `rio de janeiro`, `munich`, `barcelona`, `milan`, `osaka`, `kiev`, `sao paulo`, `são paulo`, and so on.
- `ABBREVIATIONS: &[(&str, &str)]`, upper-case keys: `UTC GMT BST WET CET CEST EET EEST MSK IST PKT ICT WIB SGT HKT JST KST AEST AEDT ACST AWST NZST NZDT EST EDT CST CDT MST MDT PST PDT AKST HST BRT ART`.
  - Each abbreviation maps to one zone (`PST` → America/Los_Angeles, `CST` → America/Chicago). The answer shows the zone's real abbreviation at that instant.
- `find` also derives a city from every IANA name's last part (`_` read as a space). It prefers names outside `Etc/`, `US/` and `SystemV/`. `CITIES` and `ABBREVIATIONS` override a derived name.

- [ ] **Step 1: Add jiff and check the bundled database natively**

In `plugins/calc/Cargo.toml`, add `jiff = { version = "1.0.1", default-features = false, features = ["std", "tzdb-bundle-always"] }`. Without `tz-system`, the native tests and the wasm build use the same bundled data on every OS, Windows included.

Then write this test in `zones.rs`:

```rust
#[test]
fn the_database_is_bundled() {
    assert!(jiff::tz::db().get("Asia/Tokyo").is_ok());
    assert!(jiff::tz::db().available().count() >= 590);
}
```

Run: `cargo test --manifest-path plugins/calc/Cargo.toml the_database_is_bundled`
Expected: PASS.

- [ ] **Step 2: Write the failing tests**

The engine test helper gets a context: `fn ctx() -> Context<'static> { Context::new(1_795_099_320, zones::find("Europe/Moscow").unwrap()) }`. That is 2026-11-19T14:42:00Z. `run`, `one` and `eval_query` in the tests use it. `fn run_in(zone: &str, text: &str)` uses the same `now` in another zone.

```rust
#[test]
fn dates_and_durations() {
    table(&[
        ("now", "Thu, 19 Nov 2026, 17:42 MSK"), ("time", "Thu, 19 Nov 2026, 17:42 MSK"),
        ("today", "Thu, 19 Nov 2026"), ("tomorrow", "Fri, 20 Nov 2026"), ("yesterday", "Wed, 18 Nov 2026"),
        ("Now", "Thu, 19 Nov 2026, 17:42 MSK"), ("TODAY", "Thu, 19 Nov 2026"), ("Now + 1 h", "Thu, 19 Nov 2026, 18:42 MSK"),
        ("now + 2 weeks", "Thu, 3 Dec 2026, 17:42 MSK"), ("2 weeks + now", "Thu, 3 Dec 2026, 17:42 MSK"),
        ("today - 3 days", "Mon, 16 Nov 2026"), ("today + 1 month", "Sat, 19 Dec 2026"),
        ("today + 1 year", "Fri, 19 Nov 2027"), ("today + 1.5 days", "Fri, 20 Nov 2026, 12:00 MSK"),
        ("now + 90 min", "Thu, 19 Nov 2026, 19:12 MSK"),
        ("days until 31 dec", "42 days"), ("dec 31 - today", "42 days"), ("31 december - today", "42 days"),
        ("days until 5 jan", "47 days"), ("weeks until 31 dec", "6 weeks"),
        ("(dec 31 - today) in weeks", "6 weeks"), ("now - today", "17.7 h"),
        ("31 dec", "Thu, 31 Dec 2026"), ("dec 31 2027", "Fri, 31 Dec 2027"),
        ("now + now", "error: cannot add two dates"), ("now * 2", "error: cannot multiply a date"),
        ("today + 3", "error: cannot add a date and a number"),
        ("5 - today", "error: cannot subtract a date from a number"),
        ("31 feb", "error: not a date"), ("25:00", "error: not a time"), ("13pm", "error: not a time"),
        ("now + 1e300 years", "error: date out of range"),
        ("today in km", "error: cannot convert a date to km"),
    ]);
}

#[test]
fn time_zones() {
    table(&[
        ("now in Tokyo", "Thu, 19 Nov 2026, 23:42 JST"), ("time in London", "Thu, 19 Nov 2026, 14:42 GMT"),
        ("now in New York", "Thu, 19 Nov 2026, 09:42 EST"),
        ("now in america/los_angeles", "Thu, 19 Nov 2026, 06:42 PST"),
        ("now to UTC", "Thu, 19 Nov 2026, 14:42 UTC"),
        ("5pm London in Moscow", "Thu, 19 Nov 2026, 20:00 MSK"),
        ("15:00 MSK in PST", "Thu, 19 Nov 2026, 04:00 PST"),
        ("5:30 pm in Tokyo", "Thu, 19 Nov 2026, 23:30 JST"), ("12am", "Thu, 19 Nov 2026, 00:00 MSK"),
        ("now in Dubai", "Thu, 19 Nov 2026, 18:42 +04"), ("now in São Paulo", "Thu, 19 Nov 2026, 11:42 -03"),
        ("now in Narnia", "error: unknown unit 'Narnia'"),
        ("5 km in Tokyo", "error: cannot convert km to Asia/Tokyo"),
    ]);
}

#[test]
fn unix_time() {
    table(&[
        ("1700000000 to date", "Wed, 15 Nov 2023, 01:13 MSK"), ("now to unix", "1795099320"),
        ("today to unix", "1795035600"), ("1700000000 to date in UTC", "Tue, 14 Nov 2023, 22:13 UTC"),
        ("5 km to date", "error: cannot convert km to a date"),
        ("now to hex", "error: hex needs a plain number"),
    ]);
}

#[test]
fn calendar_months_and_dst() {
    table(&[("31 jan 2026 + 1 month", "Sat, 28 Feb 2026")]);
    assert_eq!(run_in("Europe/London", "1774745940 to date"), ["Sun, 29 Mar 2026, 00:59 GMT"]);
    assert_eq!(run_in("Europe/London", "1774746000 to date"), ["Sun, 29 Mar 2026, 02:00 BST"]);
    assert_eq!(run_in("Europe/London", "1774699200 to date + 1 day"), ["Sun, 29 Mar 2026, 12:00 BST"]);
    assert_eq!(run_in("Europe/London", "1774699200 to date + 24 h"), ["Sun, 29 Mar 2026, 13:00 BST"]);
    assert_eq!(run_in("Europe/London", "29 mar 2026 - 28 mar 2026"), ["1 day"]);
}

#[test]
fn time_words_stay_free_for_variables() {
    assert_eq!(run("time = 5\ntime * 2"), ["time = 5", "10"]);
    assert_eq!(run("dec = 3\ndec + 1"), ["dec = 3", "4"]);
    table(&[("until = 1", "error: 'until' is reserved and cannot be a variable")]);
}
```

In `zones.rs`:
- `find_accepts_names_cities_and_abbreviations`: `name(find(x)?)` is `"Europe/Moscow"` for each of `"Europe/Moscow"`, `"europe/moscow"`, `" Moscow "`, `"MSK"` and `"msk"`; `"America/New_York"` for `"new york"` and `"New_York"`. `find("Mars/Base")`, `find("")` and `find("in")` are `None`.
- `every_city_and_abbreviation_resolves`: every value of `CITIES` and `ABBREVIATIONS` is a name jiff knows.
- `tables_do_not_collide`: no `ABBREVIATIONS` key, lower-cased, is a `CITIES` key, a unit name or alias, a month name, or a keyword.

Then update the existing test tables and timing tests deliberately:

**`query_heuristic` answers:**
- `("now in Tokyo", "Thu, 19 Nov 2026, 23:42 JST")`;
- `("time in London", "Thu, 19 Nov 2026, 14:42 GMT")`;
- `("days until 31 dec", "42 days")`;
- `("dec 31 - today", "42 days")`;
- `("now + 2 weeks", "Thu, 3 Dec 2026, 17:42 MSK")`;
- `("5pm London in Moscow", "Thu, 19 Nov 2026, 20:00 MSK")`;
- `("1700000000 to date", "Wed, 15 Nov 2023, 01:13 MSK")`;
- `("now to unix", "1795099320")`;
- `("now", "Thu, 19 Nov 2026, 17:42 MSK")`, `("time", "Thu, 19 Nov 2026, 17:42 MSK")`, `("today", "Thu, 19 Nov 2026")`, `("tomorrow", "Fri, 20 Nov 2026")`, `("yesterday", "Wed, 18 Nov 2026")`, `("Now", "Thu, 19 Nov 2026, 17:42 MSK")`, `("NOW", "Thu, 19 Nov 2026, 17:42 MSK")`, `("Today", "Thu, 19 Nov 2026")`.

**`query_heuristic` no answer:** `"15:00"`, `"5pm"`, `"5pm London"`, `"15:00 MSK"`, `"31 dec"`, `"dec 31"`, `"today in history"`, `"now in Narnia"`, `"5pm meeting"`, `"jan 2026"`, `"may 5 - 3"`, `"time to go"`.

**`PIECES`:** add `"now"`, `"today"`, `"Tokyo"`, `"New York"`, `"15:00"`, `"5pm"`, `"dec"`, `"31"`, `"until"`, `"days"`, `"to date"`, `"to unix"`, `"1700000000"` and `" + 1 month"`.

**`pathological_inputs`:**
- Add `format!("now{}", " in Tokyo".repeat(200))`, `format!("{}31 dec", "days until ".repeat(100))`, `"New York ".repeat(100)`, `format!("now{}", " + 1 day".repeat(500))`, `"15:00 ".repeat(200)` and `"31 dec ".repeat(100)`.
- Assert that the first two give `"error: too deeply nested"`.

**`sheet_of_1000_lines_is_fast`:** gets a sixth line kind, `"now in Tokyo\n"`.

**New timing test.** `zone_sheet_of_10000_lines_is_fast` (`#[cfg_attr(debug_assertions, ignore)]`) evaluates 10 000 lines of `5pm London in New York` in < 200 ms. Wasm is slower, and the `on-event` deadline is 1000 ms.

**Views tests:** they get a `ctx()` helper, and their expected values do not change. A new views test, `answers_name_the_zone_they_used`, checks the subtitle:
- `"today + 1 day"` → `"today + 1 day · Europe/Moscow"`;
- `"days until 31 dec"` → `"days until 31 dec · Europe/Moscow"`;
- `"now in Tokyo"` → `"now in Tokyo"` (its abbreviation shows);
- `"now"` → `"now"`, titled `"Thu, 19 Nov 2026, 17:42 MSK"`;
- `"today"` → `"today · Europe/Moscow"`, titled `"Thu, 19 Nov 2026"`;
- `"2+2"` → `"2+2"`.

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test --manifest-path plugins/calc/Cargo.toml`
Expected: FAIL to compile (`Context`, `zones`), then fail on the tables.

- [ ] **Step 4: Implement the interfaces above**

Use jiff 1.0 names:
- `Timestamp::from_second`, `Timestamp::to_zoned_datetime(tz)`;
- `civil::date(y, m, d).at(h, mi, 0, 0).to_zoned_datetime(tz)`;
- `ZonedDateTime::checked_add(Span::new().days(n) / .months(n))`;
- `strftime("%a, %-d %b %Y, %H:%M %Z")`, and `"%a, %-d %b %Y"` for date-only moments.

`lib.rs` passes `with_context` into every `views` call. Behaviour in the plugin is UTC until Task 3.

- [ ] **Step 5: Run every check**

Run: the calc fmt, clippy (both targets) and `cargo test --release`; the wasm build; `cargo test -p tomakast-plugin-host --test calc --test wit_coverage`.
Expected: all pass. The host suites are unchanged.

Record the size: `ls -l target/plugins/wasm32-wasip2/release/calc.wasm`. Expect about 835,000 bytes. If it is over 982,228 bytes, stop and report.

- [ ] **Step 6: Commit**

```bash
git add plugins/calc/Cargo.toml plugins/calc/Cargo.lock plugins/calc/src
git commit -m "feat(calc): dates, times, durations and time zones over jiff's bundled tzdb" -- plugins/calc/Cargo.toml plugins/calc/Cargo.lock plugins/calc/src
```

---

### Task 3: The `timezone` preference

**Files:**
- Modify: `plugins/calc/plugin.toml`, `plugins/calc/src/lib.rs`
- Modify: `crates/plugin-host/tests/support/harness.rs`
- Test: `crates/plugin-host/tests/calc.rs`

**Interfaces:**
- Consumes: `zones::find` and `zones::utc` from Task 2.
- Produces, in `lib.rs`: a `thread_local` `ZONE: Cell<ZoneId>`, and `fn load_zone()`.
  - `load_zone` reads `preferences::get("timezone")` and resolves it with `zones::find`. Otherwise it uses UTC and logs `timezone '<value>' is not a known time zone; using UTC` with `log::warn`.
  - It runs in `init` and on `Event::PreferencesChanged`, which returns no effects.
  - `with_context` uses `ZONE`.
- Produces, in the harness:
  ```rust
  #[derive(Default)]
  pub struct PluginSetup { pub seed: Vec<(String, Vec<u8>)>, pub prefs: BTreeMap<String, String> }
  pub fn plugin_harness_with(id: &'static str, setup: PluginSetup,
                             build: impl FnOnce(WasmPlugin) -> WasmPlugin) -> Harness;
  ```
  - `plugin_harness` and `plugin_harness_built` call it.
  - `Harness` keeps `prefs`, and `reload` passes them to `WasmPlugin::new`.

The manifest gains, before `[permissions]`:

```toml
[[preferences]]
key = "timezone"
title = "Time zone (IANA name such as Europe/Moscow, or a city)"
kind = "text"
default = "UTC"
```

- [ ] **Step 1: Write the failing host tests in `calc.rs`**

```rust
/// Calc with `prefs` as the user's values.
fn with_prefs(prefs: &[(&str, &str)]) -> Harness;

#[test]
fn now_uses_the_time_zone_preference() {
    for (zone, suffix) in [(None, " UTC"), (Some("Asia/Tokyo"), " JST"), (Some("Moscow"), " MSK"),
                           (Some("europe/moscow"), " MSK"), (Some("Mars/Base"), " UTC"), (Some(""), " UTC")] {
        let mut calc = match zone { None => start(), Some(z) => with_prefs(&[("timezone", z)]) };
        let title = calc.answers("now + 0 min")[0].title.clone();
        assert!(title.ends_with(suffix), "{zone:?}: {title}");
        assert_eq!(title.split(", ").count(), 3, "{title}");
    }
}

#[test]
fn a_preference_change_moves_now() {
    // `build` keeps `plugin.preferences()`, as wit_coverage does.
    // handle.set({"timezone": "Europe/Moscow"}); calc.settle(50 ms);
    // answers("now + 0 min")[0].title ends with " MSK"; calc.unsolicited is empty.
}

#[test]
fn date_answers_name_their_zone() {
    // start(): answers("days until 1 jan")[0].subtitle == Some("days until 1 jan · UTC");
    // with_prefs(&[("timezone", "Asia/Tokyo")]): Some("days until 1 jan · Asia/Tokyo").
}

#[test]
fn search_answers_time_zones() {
    let mut calc = start();
    assert!(calc.answers("now in Tokyo")[0].title.ends_with(" JST"));
    assert!(calc.answers("now")[0].title.ends_with(" UTC"));
    assert_eq!(calc.answers("today")[0].subtitle.as_deref(), Some("today · UTC"));
    assert!(calc.answers("31 dec").is_empty() && calc.answers("5pm").is_empty());
    assert_eq!(calc.answers("days until 1 jan").len(), 1);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p tomakast-plugin-host --test calc now_uses_the_time_zone_preference`
Expected: FAIL. The suffix is " UTC" for Tokyo, and the harness has no `plugin_harness_with` yet.

- [ ] **Step 3: Implement `PluginSetup`, the manifest preference and `load_zone`**

- [ ] **Step 4: Run every check**

Run: the calc checks; the wasm build; `cargo test -p tomakast-plugin-host --test calc --test wit_coverage`.
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add plugins/calc/plugin.toml plugins/calc/src/lib.rs crates/plugin-host/tests/support/harness.rs crates/plugin-host/tests/calc.rs
git commit -m "feat(calc): a timezone preference for now and today" -- plugins/calc/plugin.toml plugins/calc/src/lib.rs crates/plugin-host/tests/support/harness.rs crates/plugin-host/tests/calc.rs
```

---

### Task 4: Currencies on stored rates

**Files:**
- Create: `plugins/calc/src/engine/currency.rs`, `plugins/calc/src/rates.rs`
- Modify: `engine/mod.rs`, `units.rs` (`Dim::Currency`), `lexer.rs`, `parser.rs`, `eval.rs`, `format.rs`, `plugins/calc/src/views.rs`, `plugins/calc/src/lib.rs`
- Test: the same modules and `crates/plugin-host/tests/calc.rs`

**Interfaces:**
- Consumes: `Context` (Task 2), `Value::Num` and `Target::Unit` (Task 1).
- Produces, in `currency.rs`:
  ```rust
  /// ISO 4217 codes that open.er-api.com serves (Task 6 checks them against a captured answer).
  pub const FIAT: &[&str];
  /// (ticker, CoinGecko id): BTC bitcoin, ETH ethereum, USDT tether, BNB binancecoin, SOL solana,
  /// USDC usd-coin, XRP ripple, DOGE dogecoin, ADA cardano, TRX tron, TON the-open-network,
  /// AVAX avalanche-2, SHIB shiba-inu, DOT polkadot, LINK chainlink, BCH bitcoin-cash,
  /// LTC litecoin, XLM stellar, XMR monero, ETC ethereum-classic, ATOM cosmos, UNI uniswap,
  /// NEAR near, APT aptos, SUI sui, POL polygon-ecosystem-token, ARB arbitrum, OP optimism,
  /// FIL filecoin, ICP internet-computer, PEPE pepe, DAI dai.
  pub const CRYPTO: &[(&str, &str)];
  /// $ USD, € EUR, £ GBP, ¥ JPY, ₽ RUB, ₹ INR, ₩ KRW, ₺ TRY, ₴ UAH, ₪ ILS, ₿ BTC.
  pub const SYMBOLS: &[(&str, &str)];
  /// A code or ticker in any case, or a symbol: `Unit { name: <CODE>, dim: Dim::Currency, factor: 0.0, offset: 0.0 }`.
  pub fn lookup(name: &str) -> Option<Unit>;
  pub fn is_crypto(code: &str) -> bool;
  #[derive(Debug, Clone, PartialEq)]
  pub struct Table { pub fetched: i64, pub values: HashMap<&'static str, f64> }
  /// `fiat` values are units per 1 USD; `crypto` values are the USD price of 1 coin.
  #[derive(Debug, Clone, Default, PartialEq)]
  pub struct Rates { pub fiat: Option<Table>, pub crypto: Option<Table> }
  impl Rates {
      /// USD is 1 with no table. Missing table: "no exchange rates yet"; missing code: "no rate for <CODE>".
      pub fn usd_worth(&self, code: &str) -> Result<f64, String>;
  }
  ```
- Changes `Used` (Task 2) to `pub struct Used { pub zone: bool, pub fiat: bool, pub crypto: bool }`.
- Changes, in `engine/mod.rs`:
  - `pub struct Context<'a> { pub now: i64, pub zone: ZoneId, pub rates: &'a Rates, used: Cell<Used> }` (the `PhantomData` goes);
  - `Context::new(now, zone, rates: &'a Rates) -> Context<'a>`.
  - Evaluation marks a feed as used whenever a conversion needs it, failed ones included. Converting to or from USD needs no feed.
- Produces, in `rates.rs`:
  ```rust
  pub const FIAT_KEY: &str = "rates:fiat";
  pub const CRYPTO_KEY: &str = "rates:crypto";
  /// "<fetched unix> CODE=value CODE=value …", values in Rust's `{}` for f64 (exact round trip).
  pub fn encode(table: &Table) -> String;
  /// `None` for garbage or a table with no valid pair; skips unknown codes and values that are
  /// not finite and > 0.
  pub fn decode(text: &str) -> Option<Table>;
  pub struct Feeds { pub rates: Rates, crypto_used_at: Option<i64>, /* Task 6: in flight, unsaved */ }
  pub fn with<R>(f: impl FnOnce(&mut Feeds) -> R) -> R;
  impl Feeds {
      /// Reads both keys; a read error is logged and leaves that feed empty.
      pub fn load(&mut self);
      pub fn mark_used(&mut self, used: Used, now: i64);
  }
  ```
- Produces, in `views.rs`:
  - `pub const STALE_AFTER: i64 = 86_400;`.
  - When a feed the answer used is older than that, the answer's subtitle is `format!("{query} · rates from {}", relative(ctx.now - fetched))`, using the oldest feed used.

**Grammar and arithmetic:**
- The lexer turns each `SYMBOLS` character into `Ident(<char>)`.
- A unit after a number is `units::lookup`, then `currency::lookup`.
- A symbol followed by a number is a money literal (`$100`). A symbol after a number works like a code (`100$`).
- Currency `+`, `-` and `/` convert through `usd_worth`. `100 usd / 4 usd` is a plain 25. Multiplying two currencies is an error.
- `Convert` between currencies is `n * worth(from) / worth(to)`.
- Currency and physical units never mix. The errors keep the existing wording, for example `cannot add USD and km`.

`lib.rs`:
- `init` calls `rates::with(Feeds::load)`.
- `with_context` lends `&feeds.rates`.
- After each evaluation in `search` and `text-changed`, it calls `rates::with(|f| f.mark_used(used, now))` with the `Used` that `with_context` returned. That is memory only, never storage.
- `mark_used` runs after `with_context` has returned, never inside it: both borrow the same `Feeds` cell, and a nested borrow would trap the plugin.

- [ ] **Step 1: Write the failing tests**

The engine `ctx()` gets rates, each table fetched at `now - 60`:
- `fiat`: `{USD: 1, EUR: 0.92, RUB: 93.125, GBP: 0.79, JPY: 150}`;
- `crypto`: `{BTC: 67000, ETH: 3500}`.

`fn bare()` is the same context with `Rates::default()`.

```rust
#[test]
fn currencies() {
    table(&[
        ("100 usd in rub", "9 312.50 RUB"), ("100 USD to RUB", "9 312.50 RUB"),
        ("$100 to €", "92.00 EUR"), ("100$ in eur", "92.00 EUR"),
        ("50 eur + 20 usd in gbp", "58.73 GBP"), ("¥1000 in usd", "6.67 USD"),
        ("100 rub", "100.00 RUB"), ("0.5 btc in usd", "33 500.00 USD"),
        ("1 eth in btc", "0.052238806 BTC"), ("1 btc in rub", "6 239 375.00 RUB"),
        ("100 usd * 2", "200.00 USD"), ("100 usd / 4 usd", "25"),
        ("1 usd in jpy in usd", "1.00 USD"), ("0.001 usd", "0.001 USD"), ("-12.3 eur", "-12.30 EUR"),
        ("100 usd + 5 km", "error: cannot add USD and km"),
        ("100 usd in km", "error: cannot convert USD to km"),
        ("100 usd * 2 eur", "error: cannot multiply USD and EUR"),
        ("100 xyz in usd", "error: unexpected 'xyz'"),
        ("100 usd in kzt", "error: no rate for KZT"),
        ("3 $ 4", "error: unexpected '4'"),
    ]);
}

#[test]
fn currencies_without_rates() {
    // With `bare()`:
    // "100 usd in rub" -> "error: no exchange rates yet"; "100 usd" -> "100.00 USD";
    // "1 btc in usd" -> "error: no exchange rates yet"; eval_query("100 usd in rub") is None.
}

#[test]
fn conversions_report_the_feeds_they_used() {
    // For each query: eval_query on a fresh ctx(), then ctx.used():
    // "100 usd in rub" {fiat: true, crypto: false}; "1 eth in btc" {false, true};
    // "50 eur + 1 btc in usd" {true, true}; "2 + 2" {false, false};
    // with bare(): "1 btc in usd" {false, true} (an attempt counts).
}

#[test]
fn currency_codes_stay_free_for_variables() {
    assert_eq!(run("try = 2\nall = 3\ntry * all"), ["try = 2", "all = 3", "6"]);
}
```

Then the deliberate changes to existing tests:
- Replace `("3 $ 4", "error: unexpected character '$'")` in `syntax_errors_are_messages_not_panics` with `("3 @ 4", "error: unexpected character '@'")`. The `$` case moves to `currencies` above.
- In `lexer.rs`, `comment_and_symbols` asserts `lex("@") == Err("unexpected character '@'".into())` and `lex("$") == Ok(vec![Tok::Ident("$".into())])`.
- `query_heuristic`, with rates:
  - answers: `("100 usd in rub", "9 312.50 RUB")`, `("$100 to €", "92.00 EUR")`, `("50 eur + 20 usd in gbp", "58.73 GBP")`, `("0.5 btc in usd", "33 500.00 USD")`, `("1 eth in btc", "0.052238806 BTC")`;
  - no answer: `"100 usd"`, `"$100"`, `"100$"`, `"btc"`, `"usd"`.
- `PIECES`: add `"usd"`, `"$"`, `"€"`, `"btc"` and `"in rub"`.

In `format.rs`, `money_table`:

```rust
assert_eq!(money(9312.5, false), "9 312.50"); assert_eq!(money(6_239_375.0, false), "6 239 375.00");
assert_eq!(money(0.05223880597014925, true), "0.052238806"); assert_eq!(money(12345.678901, true), "12 345.679");
assert_eq!(money(-12.3, false), "-12.30"); assert_eq!(money(0.004, false), "0.004");
assert_eq!(money(0.0, false), "0.00"); assert_eq!(money(1e16, false), "1e16");
```

Here `pub fn money(x: f64, crypto: bool) -> String` is the number part; `format` appends ` <CODE>`.

In `currency.rs`, `codes_are_unique_and_found`:
- `FIAT` and the tickers are unique, and no ticker is a fiat code;
- every code is found by `lookup` in upper and lower case;
- no unit name or alias equals a currency code case-insensitively, except `cup`;
- every symbol resolves.

In `rates.rs`:
- `encode_decode_round_trip`;
- `decode_rejects_garbage`: `""`, `"abc"`, `"123"`, `"x USD=1"` and `"1795099320 USD=abc"` are all `None`;
- `decode_skips_unknown_codes_and_bad_values`: `"1795099320 USD=1 XXX=2 EUR=-1 RUB=93.125"` gives `{USD: 1, RUB: 93.125}`.

Host tests in `calc.rs`:
- `fn seeded_rates(age: i64) -> Vec<(&'static str, String)>` stores the same two tables at `unix_now() - age`, in the `encode` format.

```rust
#[test]
fn stored_rates_convert_in_search_and_sheets() {
    // seeded_rates(60): answers("100 usd in rub")[0].title == "9 312.50 RUB",
    // subtitle Some("100 usd in rub"); answers("1 eth in btc")[0].title == "0.052238806 BTC";
    // the pad of "100 usd in rub\n$100 to €" annotates [note(0, "9 312.50 RUB"), note(1, "92.00 EUR")].
}
#[test]
fn rates_older_than_a_day_say_so() {
    // seeded_rates(3 * 86_400 + 60): subtitle Some("100 usd in rub · rates from 3 days ago");
    // seeded_rates(3600): subtitle Some("100 usd in rub").
}
#[test]
fn without_rates_a_currency_query_has_no_answer() {
    // start(): answers("100 usd in rub") is empty; answers("2+2")[0].title == "4";
    // a pad line "100 usd in rub" has an error annotation "no exchange rates yet".
}
#[test]
fn malformed_stored_rates_are_ignored() {
    // seed rates:fiat = "garbage", rates:crypto = "1 BTC=x": no currency answer, math still answers,
    // and calc.unsolicited is empty after settle(20 ms).
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path plugins/calc/Cargo.toml`, then `cargo test -p tomakast-plugin-host --test calc stored_rates`
Expected: FAIL (`currency` and `Rates` do not exist).

- [ ] **Step 3: Implement the interfaces above**

Build `FIAT` from one captured answer of the fiat URL (`curl -s https://open.er-api.com/v6/latest/USD`). Keep that file for Task 6 as `plugins/calc/tests/fixtures/er-api-latest-usd.json`, but do not commit it in this task.

- [ ] **Step 4: Run every check**

Run: the calc checks; the wasm build; `cargo test -p tomakast-plugin-host --test calc --test wit_coverage`.
Expected: all pass, and `calc.wasm` is at most 982,228 bytes (record the size).

- [ ] **Step 5: Commit**

```bash
git add plugins/calc/src crates/plugin-host/tests/calc.rs
git commit -m "feat(calc): fiat and crypto currencies on stored rates" -- plugins/calc/src crates/plugin-host/tests/calc.rs
```

---

### Task 5: Host: a test-only HTTP responder

This extends the scope into `crates/plugin-host`. It exists only under `#[cfg(any(test, feature = "testing"))]`, and changes nothing in production.

**Files:**
- Modify: `crates/plugin-host/src/imports/http.rs`, `src/imports/mod.rs`, `src/plugin.rs`, `src/lib.rs`
- Modify: `crates/plugin-host/tests/support/harness.rs`
- Test: `crates/plugin-host/tests/http.rs`

**Interfaces:**
- Produces:
  ```rust
  /// Test-only: answers checked requests instead of the network: (method, url) -> (status, body).
  #[cfg(any(test, feature = "testing"))]
  pub type Responder = Arc<dyn Fn(&str, &str) -> Result<(u16, Vec<u8>), String> + Send + Sync>;
  impl WasmPlugin {
      #[cfg(any(test, feature = "testing"))]
      pub fn with_http_responder(self, responder: Responder) -> WasmPlugin; // like `with_opener`
  }
  ```
  - `Http::fetch` still runs `prepare` (the allowlist, the methods, the headers), the 4-request cap and the in-flight count.
  - Only the job submitted to the pool calls the responder instead of `execute`. Its answer arrives as `http-done` with no headers.
  - `lib.rs` re-exports `Responder` under the same cfg.
- Produces, in the harness:
  - `PluginSetup` gains `pub responder: Responder`, and its `Default` is `offline()`.
  - `pub fn offline() -> Responder` always answers `Err("tests never reach the network")`.
  - `plugin_harness_with` installs the responder, and `Harness` keeps it for `reload`. The kitchen-sink harnesses install none: they still use the local `Server`.

- [ ] **Step 1: Write the failing test in `tests/http.rs`**

```rust
#[test]
fn a_test_responder_answers_only_allowed_requests() {
    // harness_built(allowlist ["example.com"], prefs {}, build: with_http_responder(r)), where r
    // records "<method> <url>" and answers Ok((200, b"faked".to_vec())).
    assert_eq!(fetch(&mut h, "https://example.com/x"), "Ok(1)");
    let answer = done(&mut h);
    assert_eq!((answer["status"].as_str(), answer["body"].as_str()), ("200", "faked"));
    let refused = fetch(&mut h, "https://example.org/x");
    assert!(refused.starts_with("Err") && refused.contains("not allowed"), "{refused}");
    assert_eq!(*seen.lock().unwrap(), ["GET https://example.com/x"]);
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p tomakast-plugin-host --test http a_test_responder`
Expected: FAIL to compile (`with_http_responder`).

- [ ] **Step 3: Implement the responder and the harness default**

- [ ] **Step 4: Run every check**

Run: `cargo clippy -p tomakast-plugin-host --all-targets --all-features -- -D warnings`; `cargo test -p tomakast-plugin-host`.
Expected: all pass. The `http` suite is otherwise unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/plugin-host/src/imports/http.rs crates/plugin-host/src/imports/mod.rs crates/plugin-host/src/plugin.rs crates/plugin-host/src/lib.rs crates/plugin-host/tests/support/harness.rs crates/plugin-host/tests/http.rs
git commit -m "test(plugin-host): a test-only HTTP responder after the allowlist" -- crates/plugin-host/src/imports/http.rs crates/plugin-host/src/imports/mod.rs crates/plugin-host/src/plugin.rs crates/plugin-host/src/lib.rs crates/plugin-host/tests/support/harness.rs crates/plugin-host/tests/http.rs
```

---

### Task 6: Fetching and refreshing rates

**Files:**
- Create: `plugins/calc/src/json.rs`, `plugins/calc/tests/fixtures/er-api-latest-usd.json`, `plugins/calc/tests/fixtures/coingecko-simple-price.json`
- Modify: `plugins/calc/plugin.toml`, `src/rates.rs`, `src/lib.rs`, `src/sheets.rs`
- Test: `src/json.rs`, `src/rates.rs`, `src/sheets.rs`, `crates/plugin-host/tests/calc.rs`

**Interfaces:**
- Consumes: `Feeds`, `encode` and `FIAT_KEY` (Task 4); `Responder` and `PluginSetup` (Task 5).
- Produces, in `json.rs`:
  ```rust
  /// The JSON values a rate body holds. Arrays are read and kept, nothing uses them.
  #[derive(Debug, Clone, PartialEq)]
  pub enum Json { Null, Bool(bool), Num(f64), Str(String), Arr(Vec<Json>), Obj(Vec<(String, Json)>) }
  /// RFC 8259, with string escapes (`\uXXXX` and surrogate pairs included). `None` for invalid
  /// JSON, trailing text, nesting deeper than 16, or a body over 64 KiB.
  pub fn parse(text: &str) -> Option<Json>;
  impl Json { pub fn get(&self, key: &str) -> Option<&Json>; }
  ```
- Produces, in `rates.rs`:
  ```rust
  pub const FIAT_URL: &str = "https://open.er-api.com/v6/latest/USD";
  pub fn crypto_url() -> String;   // see Global Constraints
  pub const TICK_MS: u64 = 600_000;
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub struct Due { pub fiat: bool, pub crypto: bool }
  /// The refresh policy of the Global Constraints; feeds in `in_flight` are never due.
  pub fn due(now: i64, rates: &Rates, crypto_used_at: Option<i64>, at_init: bool, in_flight: Due) -> Due;
  /// Status 200, `result == "success"`, at least one known code: the table, fetched at `now`.
  pub fn parse_fiat(status: u16, body: &[u8], now: i64) -> Option<Table>;
  /// Status 200, at least one known id with a `usd` price: the table by ticker.
  pub fn parse_crypto(status: u16, body: &[u8], now: i64) -> Option<Table>;
  impl Feeds {
      /// Starts the due fetches (`http::fetch`, `accept: application/json`), remembering the request
      /// ids; retries an unsaved write first. A failed `fetch` is logged.
      pub fn refresh(&mut self, now: i64, at_init: bool);
      /// An answer to one of its requests: parses it, keeps the table in memory and writes it with
      /// `Sheets::set_background`. Anything else is ignored. A bad answer is logged and changes nothing.
      pub fn answered(&mut self, done: HttpDone, now: i64);
  }
  ```
- Produces, in `sheets.rs`:
  ```rust
  /// Calc's recent writes (Ruling 8), apart from what counts them.
  pub struct WriteWindow { writes: VecDeque<Instant> }
  impl WriteWindow {
      pub fn room(&mut self, n: usize, keep: usize, now: Instant) -> bool;
      pub fn note(&mut self, now: Instant);
  }
  impl Sheets {
      /// One write that is not a sheet's (a rate feed): only when the budget less RESERVE has room.
      /// Ok(false): no room now. A failed `set` is returned as its error (logged by the caller); it
      /// never drops a sheet.
      pub fn set_background(&mut self, key: &str, value: &str) -> Result<bool, String>;
  }
  ```
  - `Sheets::room`, `set` and `remove` use `WriteWindow`.
  - The module doc lists the keys `rates:fiat` and `rates:crypto`.
  - The Ruling 8 paragraph says: rate writes count against the same budget, never use the reserve, and are at most one per feed per refresh (about 7 an hour). When they find no room, or the quota is full, the rates stay in memory.
- `lib.rs`:
  - `init`: `load_zone()`, then `Feeds::load`, then `timers::start(TICK_MS, true)` (a failure is logged), then `refresh(now, true)`. It returns no effects.
  - `Event::TimerFired(_)` → `refresh(now, false)`.
  - `Event::HttpDone(d)` → `answered(d, now)`.
  - Both return no effects.

The manifest's `[permissions]` becomes:

```toml
[permissions]
search = true
storage = true
clipboard = true
timers = true
http = ["open.er-api.com", "api.coingecko.com"]
```

- [ ] **Step 1: Capture the fixtures**

```bash
curl -s https://open.er-api.com/v6/latest/USD > plugins/calc/tests/fixtures/er-api-latest-usd.json
curl -s "$(<the crypto_url() string>)" > plugins/calc/tests/fixtures/coingecko-simple-price.json
```

Expected: the first holds `"result":"success"` and about 160 codes. The second holds every id of `CRYPTO`; fix any id CoinGecko does not return.

- [ ] **Step 2: Write the failing native tests**

In `json.rs`, `reads_what_rate_bodies_hold`:
- `parse(r#"{"a":1,"b":{"c":-2.5e-3},"d":[true,null],"e":"x\"\u00e9\ud83d\ude00"}"#)` reads all of it, and `e` is `Str("x\"é😀")`;
- `parse("")`, `parse("{")`, `parse("{} x")`, `parse("{\"a\":01}")`, `parse("[\"\\ud800\"]")` and `parse(&"[".repeat(17))` are `None`;
- both fixtures parse.

In `rates.rs`:
- `due_follows_the_policy`, with `now = 1_000_000`:
  - no tables: `due(now, &none, None, at_init, Due::default()) == Due { fiat: true, crypto: true }` for both values of `at_init` (a missing feed is always due);
  - fiat fetched at `now - 3540` is due, at `now - 3539` it is not;
  - crypto fetched at `now - 540` and used at `now - 60` is due, fetched at `now - 539` it is not;
  - crypto fetched at `now - 900` is not due when used at `now - 1801`, and is due when used at `now - 1799`;
  - crypto missing and used at `now - 10` is due;
  - at init, crypto fetched at `now - 100` is not due;
  - `in_flight.fiat` makes fiat not due.
- `parses_the_er_api_fixture`: `values["USD"] == 1.0`, `values.len() == FIAT.len()`, `fetched == now`.
- `fiat_codes_match_the_fixture`: the fixture's codes as a set equal `FIAT`.
- `rejects_bad_fiat_answers`:
  - status 500 with the fixture body, `<html>`, `{"result":"error","error-type":"invalid-key"}` and `{"result":"success","rates":{}}` are all `None`;
  - `{"result":"success","rates":{"USD":1,"EUR":-1,"XXX":2}}` gives `{USD: 1}`.
- `parses_the_coingecko_fixture`: every `CRYPTO` ticker is present and `> 0`.
- `rejects_bad_crypto_answers`: 429, `{}` and `{"bitcoin":{"eur":1}}` are `None`.
- `parsing_rates_is_fast` (`#[cfg_attr(debug_assertions, ignore)]`): each fixture parses in < 5 ms.

In `sheets.rs`, `background_writes_leave_the_reserve`, at `t0`:
- after 11 `note`s, `room(1, RESERVE, t0)` is `true`;
- after 12, it is `false`;
- at `t0 + WRITE_WINDOW` it is `true` again.

- [ ] **Step 3: Write the failing host tests in `calc.rs`**

Test fixtures:
- `const FIAT_BODY: &str = r#"{"result":"success","time_last_update_unix":1795046401,"rates":{"USD":1,"EUR":0.92,"RUB":93.125,"GBP":0.79,"JPY":150}}"#;`
- `const CRYPTO_BODY: &str = r#"{"bitcoin":{"usd":67000},"ethereum":{"usd":3500}}"#;`
- `fn fake_rates() -> (Responder, Arc<Mutex<Vec<String>>>)` serves `FIAT_BODY` for the fiat URL and `CRYPTO_BODY` for a URL starting with `https://api.coingecko.com/api/v3/simple/price?ids=`. It records `"<method> <url>"`.
- `fn wait_stored(calc: &mut Harness, key: &str) -> String` settles for up to 5 s until the key is stored.

```rust
#[test]
fn init_fetches_rates_and_search_uses_them() {
    // Rates arrive after init; their time is calc's, within 60 s of unix_now().
    // wait_stored("rates:fiat") contains "RUB=93.125"; wait_stored("rates:crypto") contains "BTC=67000".
    // answers("100 usd in rub")[0].title == "9 312.50 RUB"; answers("1 eth in btc")[0].title == "0.052238806 BTC".
    // The two requests: "GET https://open.er-api.com/v6/latest/USD", and one that starts with
    // "GET https://api.coingecko.com/api/v3/simple/price?ids=bitcoin,ethereum," and ends with
    // "&vs_currencies=usd". calc.unsolicited is empty.
}
#[test]
fn bad_rate_answers_store_nothing() {
    // For each responder (offline, status 500, body "<html>", body r#"{"result":"error"}"#, body "{}"):
    // settle(300 ms); stored("rates:fiat") and stored("rates:crypto") are None;
    // answers("100 usd in rub") is empty; answers("2+2")[0].title == "4"; unsolicited is empty.
}
#[test]
fn fresh_stored_rates_are_not_fetched_again() {
    // seeded_rates(60) + fake_rates(): settle(300 ms); no request was made.
    // fiat seeded 2 h old, crypto 60 s old: exactly one request, the fiat URL.
}
#[test]
fn rates_never_drop_a_sheet_when_storage_is_full() {
    // The responder fails until a gate opens. Type "1+1" into a new sheet (listed); fill_quota;
    // open the gate; calc.reload(); settle until two more requests were seen (four in all: the
    // first init's two failed).
    // answers("100 usd in rub")[0].title == "9 312.50 RUB" (memory); stored("rates:fiat") is None;
    // listed(&calc) is unchanged; no Toast in calc.unsolicited.
}
#[test]
fn calc_asks_for_web_requests_and_timers() {
    // Package::read(<calc's plugin folder from loaded_plugin("calc")>).manifest.permissions.describe() ==
    //   ["Store data on this computer (up to 1 MiB)", "Copy text to the clipboard",
    //    "Run timers in the background", "See what you type in the launcher search",
    //    "Make web requests to open.er-api.com, api.coingecko.com"]
    // and decide(Some(&grant), &package) == Decision::Ask for an Allow grant of the old set
    // { storage, clipboard, search } with another content hash, name "Calculator", author None.
}
#[test]
#[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
fn zone_and_currency_queries_stay_within_budget() {
    // seeded_rates(60): the best of 5 round trips of "now in New York" and of "100 usd in rub"
    // is ≤ 10 ms each.
}
```

- [ ] **Step 4: Run them to see them fail**

Run: `cargo test --manifest-path plugins/calc/Cargo.toml`, then `cargo test -p tomakast-plugin-host --test calc init_fetches_rates`
Expected: FAIL (`due` and `parse_fiat` do not exist; no request is made).

- [ ] **Step 5: Implement the interfaces above**

Read the bodies with `json::parse`:
- er-api: `get("result") == Str("success")`, and the numbers of `get("rates")`;
- CoinGecko: each top-level member's `get("usd")` number.

Map the keys back to the `&'static str` codes of `currency.rs`. Members that are not numbers are skipped.

- [ ] **Step 6: Run every check**

Run: the calc checks; the wasm build; `cargo test --release -p tomakast-plugin-host --test calc`; `cargo test -p tomakast-plugin-host --test wit_coverage --test http`.
Expected: all pass, and `calc.wasm` is at most 982,228 bytes (record the size).
- `wit_coverage` still passes: calc now also records `import http.fetch`, `import timers.start` and `event http-done`, all names the WIT has.
- The existing calc host tests pass unchanged: the offline default writes nothing.

- [ ] **Step 7: Commit**

```bash
git add plugins/calc/plugin.toml plugins/calc/src plugins/calc/tests/fixtures crates/plugin-host/tests/calc.rs
git commit -m "feat(calc): fetch fiat and crypto rates at init and on a 10-minute tick" -- plugins/calc/plugin.toml plugins/calc/src plugins/calc/tests/fixtures crates/plugin-host/tests/calc.rs
```

---

### Task 7: Docs, sizes and the acceptance checklist

**Files:**
- Create: `docs/experiments/2026-11-calc-conversions-acceptance.md`
- Modify: `docs/plugins/writing-a-plugin.md`, `docs/experiments/2026-11-plan2b2b-acceptance.md`

**Interfaces:** none (docs only).

- [ ] **Step 1: Update the worked example in `writing-a-plugin.md`**

- "asks for three permissions" becomes "asks for five permissions", with the new `[permissions]` block and the `[[preferences]]` block.
- Add one paragraph, **Rates in the background**:
  - `init` fetches stale rates and starts one 10-minute timer;
  - `http-done` stores one key per feed under the same write budget, never the reserve;
  - `search` only reads what is in memory and never waits;
  - all of it stays far inside the 1 s per 10 s background budget.
- Add one line on the `timezone` preference and how a user sets it in `config.toml` (`[plugins.calc] timezone = "Europe/Moscow"`).

- [ ] **Step 2: Update `2026-11-plan2b2b-acceptance.md` where it names calc's permissions**

Update the C1 row's expected list to the `describe()` order: "Store data on this computer (up to 1 MiB)", "Copy text to the clipboard", "Run timers in the background", "See what you type in the launcher search" and "Make web requests to open.er-api.com, api.coingecko.com". Add a note: "(since calc conversions; the 2b-2b run had the first, second and fourth)".

In Known limitations, the Ruling 8 sentence "Calc holds no `timers` permission" becomes: "Calc holds `timers` since calc conversions, but its 10-minute tick does not save waiting edits."

- [ ] **Step 3: Measure and write the acceptance note**

Run these and record their output:
- the wasm build, then `ls -l target/plugins/wasm32-wasip2/release/calc.wasm`. Compare against 326,868 bytes at `7299987`, and against the planning measurements in the Global Constraints;
- `cargo test --release --manifest-path plugins/calc/Cargo.toml -- --nocapture search_is_fast sheet_of_1000_lines_is_fast zone_sheet_of_10000_lines_is_fast parsing_rates_is_fast`;
- `cargo test --release -p tomakast-plugin-host --test calc -- --nocapture pad_and_search_stay_within_budget zone_and_currency_queries_stay_within_budget`.

`docs/experiments/2026-11-calc-conversions-acceptance.md` holds:
- **The numbers table.** Each measurement against its budget: 10 ms query, 50 ms sheet, 200 ms zone sheet, 5 ms parse, and the 640 KiB size gate.
- **This plan's rulings.**
  - jiff over chrono-tz, with both deltas;
  - a hand-written JSON reader for the rate bodies, instead of `serde_json` (+44 KiB);
  - answers that used the home zone without showing it name it in the subtitle;
  - soft names and the 20 reserved ones;
  - `cup` shadows CUP in lower case;
  - bare `now`, `time`, `today`, `tomorrow` and `yesterday` answer (an exception like `pi`), bare dates and clock times do not, and `0xff` does;
  - word time units are plural;
  - the age of rates is calc's fetch time;
  - one 10-minute tick, and a missing feed is always due;
  - rate writes never use the reserve and never drop sheets;
  - the test-only HTTP responder.
- **A user checklist** in the 2b-2b style (throwaway `TOMAKAST_HOME`, the same build and staging commands). One row each:

| # | What | Expected |
|---|---|---|
| K1 | Re-approval | The prompt lists the five permission lines, in the C1 order. |
| K2 | Currency | `100 usd in rub` answers within a few seconds of approval. |
| K3 | Crypto | `0.5 btc in usd` and `1 eth in btc` answer. |
| K4 | Zones | `now in Tokyo` and `5pm London in Moscow` answer. |
| K5 | Preference | `[plugins.calc] timezone = "Europe/Moscow"` in `config.toml` makes `now + 0 min` end in MSK. |
| K6 | Offline | Starting with the network off: math answers, a currency query does not, and there is no toast. |
| K7 | Units and bases | `100 F to C`, `255 in hex`, `1 GiB in MiB`. |
| K8 | Not math | `31 dec`, `5pm`, `100 usd`, `today in history`, `5pm meeting` get no calculator row; bare `now` and `today` do answer, showing the zone. |

- [ ] **Step 4: Commit**

```bash
git add docs/plugins/writing-a-plugin.md docs/experiments/2026-11-plan2b2b-acceptance.md docs/experiments/2026-11-calc-conversions-acceptance.md
git commit -m "docs(calc): conversions in the worked example; acceptance note and checklist" -- docs/plugins/writing-a-plugin.md docs/experiments/2026-11-plan2b2b-acceptance.md docs/experiments/2026-11-calc-conversions-acceptance.md
```

---

## Self-review

- **Spec coverage.**
  - Units, in every family: Task 1. Calendar month and year on dates: Task 2.
  - `now`/`today`/`tomorrow`/`yesterday`, date arithmetic, differences, zone conversions and unix time: Task 2.
  - The `timezone` preference: Task 3. Fiat by code and symbol, the ambiguous `¥` as JPY, and crypto by ticker: Task 4.
  - Fetching at init and on timers, storage with their time, no answer without rates, the subtitle for rates older than a day: Tasks 4 and 6.
  - Permissions and the re-ask: Tasks 6 and 7. Number bases: Task 1. The formats: Tasks 1, 2 and 4.
  - The bundled tz database with a measured size: the Global Constraints and Tasks 2 and 7.
  - The city table: Task 2. English only: the Global Constraints.
- **Type consistency.**
  - `Context::new` takes two arguments in Tasks 2–3 and three from Task 4; every caller is `with_context` and the test helpers.
  - `Value::{Num, Based, Moment}`, `Target::{Unit, Base, Zone, Date, Unix}`, `Rates`/`Table`/`Used`, `Feeds::{load, mark_used, refresh, answered}`, `Responder` and `PluginSetup` are named the same everywhere they appear.
- **Deliberate changes to existing tests:**
  - `1 week in day` → `7 days`;
  - `3 $ 4` moves to `3 @ 4`, and `lex("$")` becomes an `Ident`;
  - the `query_heuristic` additions on both sides;
  - the `PIECES` additions;
  - the harness defaults to offline.
