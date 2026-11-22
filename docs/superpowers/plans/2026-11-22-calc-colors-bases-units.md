# calc: colours, bases, units and city short names — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** calc reads, converts and shows colours (with a swatch on the answer tile and in
sheets), shows numbers in any base 2–36, knows energy, power, pressure, frequency, data
rate, fuel economy and a few more units, and knows city short names like `spb` and `bcn`.

**Architecture:** A colour is a new `Value` variant in calc's engine (`engine/color.rs`),
read by the existing lexer/parser and converted by `in`/`to`. A `#` is tried as a colour
first and falls back to a comment. Units grow the table in `units.rs`; `derive` moves from
(length, time) powers to four axes. The sheet swatch is one optional field on the
contract's `line-annotation`, drawn by the scratchpad view.

**Tech Stack:** Rust; calc is a `wasm32-wasip2` component built with the plugin SDK; the
launcher is GPUI.

**Spec:** `docs/superpowers/specs/2026-11-22-calc-colors-bases-units-design.md`

## Global Constraints

- `calc.wasm` at most **1,047,764 bytes**: `just build`, then
  `ls -l target/plugins/wasm32-wasip2/release/calc.wasm`. Check after Tasks 3, 6 and 7; if
  over, stop and report.
- No new crates in calc (hand-written parsing and colour maths).
- Every line that evaluates today keeps its answer; the only intended change is
  `10 l/100km` (no spaces), and `1 GB / 1 s` (an error today) becoming `1 GB/s`.
- All new names are soft (a variable wins); `tan` stays a function name that cannot be a
  variable.
- WIT 0.1 edited in place: re-hash `FROZEN`, add a row to `FROZEN.md`.
- Specs, plans, notes and the plugin's UI text in English. No attribution lines in commits.
  Do not push or merge.
- Run the app only through `just` recipes with `TOMAKAST_NO_KEYBOARD=1` and a throwaway
  `TOMAKAST_HOME`; one `just try` at a time on the machine.
- Calc tests: `cargo test --release --manifest-path plugins/calc/Cargo.toml`. Whole tree:
  `just check`.

## Review Focus

1. A sheet with ordinary comments (`# note`, `#todo`, `#add later`, `x = 5 #fee`) must look
   exactly as before. Test in Task 3 (`hash_comments_stay_comments`).
2. Searches that are not math must stay silent: `orange`, `red apple`, `#hashtag`,
   `meeting #42`. Test in Task 3 (`colour_search_rules`).
3. Time zones of several words after `in` must still win over the new short units:
   `5pm in St Louis`, `now in St Petersburg`. Test in Task 6
   (`multiword_zones_beat_units`).
4. A plugin that sends a bad swatch (`red`, `#12345`) gets its effect rejected with a toast,
   not a crash or a silent square. Test in Task 1 (`swatch_must_be_hex`).
5. A colour near the tile's own background (`#2a2a2e`, `#fff`, `transparent`) is still
   visible as a square. Covered by the hairline border; test in Task 4
   (`swatch_has_a_border`).

---

### Task 1: The sheet swatch in the contract and the launcher

**Files:**
- Modify: `crates/plugin-api/wit/since_v0.1.0/plugin.wit` (`line-annotation`),
  `FROZEN`, `FROZEN.md`
- Modify: `crates/core/src/view.rs` (`LineAnnotation`)
- Modify: `crates/plugin-host/src/convert.rs`, `crates/plugin-host/src/caps.rs`
- Modify: `crates/plugin-sdk/src/builders.rs` (`LineAnnotation::swatch`)
- Modify: `crates/ui/src/views/scratchpad.rs`
- Fix literals: `crates/plugin-host/tests/calc.rs` (`note`), `crates/ui/tests/scratchpad.rs`
  (`note`), any other `LineAnnotation { … }` literal the compiler finds.
- Test: `crates/plugin-host/src/caps.rs` tests, `crates/ui/tests/scratchpad.rs`,
  `crates/core/tests/wit_parity.rs`

**Interfaces:**
- Produces: WIT `swatch: option<string>` on `line-annotation` (doc comment as in the spec);
  core `LineAnnotation { line, text, is_error, swatch: Option<String> }`;
  core `pub fn swatch_rgba(s: &str) -> Option<u32>` in `view.rs`: `#rrggbb` → `0xrrggbbff`,
  `#rrggbbaa` → `0xrrggbbaa`, hex digits any case, else `None`;
  SDK `LineAnnotation::swatch(self, color: impl Into<String>) -> LineAnnotation`.

- [ ] **Step 1: Write the failing tests**
  - caps: `swatch_must_be_hex`: a scratchpad whose annotation has swatch `#a1b2c3`,
    `#A1B2C3FF` or none passes; `red`, `#12345`, `#a1b2c3d` and `a1b2c3` fail with a message
    containing `is not a #rrggbb or #rrggbbaa colour`.
  - core: `swatch_rgba("#ff8800") == Some(0xff8800ff)`, `swatch_rgba("#ff880080") ==
    Some(0xff880080)`, `swatch_rgba("#ff88") == None`.
  - SDK: `LineAnnotation::note(0, "x").swatch("#ff8800").swatch == Some("#ff8800".into())`.
  - ui: `annotation_swatch_is_drawn`: a pad whose annotation on line 0 has swatch
    `#ff8800` renders an element with test id `("scratchpad-annotation-swatch", 0)`; a line
    without a swatch has none.
- [ ] **Step 2: Run them and see them fail** (`cargo test -p tomakast-core -p
  tomakast-plugin-host -p tomakast-plugin-sdk -p tomakast-ui`; compile errors count).
- [ ] **Step 3: Implement.** WIT field and doc; re-hash `FROZEN` the way `FROZEN.md`
  describes and add a row (date 2026-11-22, change "`line-annotation` gains `swatch`",
  reason "calc colours: the user's ruling, 2026-11-22; edited in place"). Convert the field
  in `convert.rs`. In `caps.rs`, check each annotation's swatch with `swatch_rgba`. In the
  scratchpad view, a swatch row child before the text: 12×12 logical px, `rounded(3)`, the
  colour (with its alpha) as background, `border_1()` in the palette's border colour, a
  6 px gap to the text, vertically centred on the line.
- [ ] **Step 4: Run the tests and see them pass**, plus `wit_parity`.
- [ ] **Step 5: Commit** `feat(contract): a sheet annotation can carry a colour swatch`.

### Task 2: The colour model (`engine/color.rs`)

**Files:**
- Create: `plugins/calc/src/engine/color.rs` (with its unit tests)
- Modify: `plugins/calc/src/engine/mod.rs` (`mod color; pub use color::{Color, Form};`)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq)]
  pub struct Color { pub r: f64, pub g: f64, pub b: f64, pub a: f64, pub form: Form }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Form { Hex, Rgb, Hsl, Oklch }
  pub fn from_hex(digits: &str) -> Option<Color>      // no '#'; 3, 4, 6 or 8 digits; form Rgb
  pub fn named(name: &str) -> Option<Color>           // 148 CSS names + transparent, any case; form Hex
  pub fn from_rgb(r: f64, g: f64, b: f64, a: f64) -> Color   // 0–255 channels, a 0–1; clamped; form Hex
  pub fn from_hsl(h_deg: f64, s: f64, l: f64, a: f64) -> Color  // s, l in 0–1; form Hex
  pub fn from_oklch(l: f64, c: f64, h_deg: f64, a: f64) -> Color // clipped to sRGB; form Hex
  pub fn format(c: &Color, form: Form) -> String
  pub fn name_of(c: &Color) -> Option<&'static str>  // exact 8-bit match and a == 1
  pub fn rgba8(c: &Color) -> [u8; 4]
  pub fn swatch_hex(c: &Color) -> String             // "#rrggbb", or "#rrggbbaa" when a < 1
  ```
  Channels are sRGB in 0–1. OKLCH uses Björn Ottosson's matrices and the sRGB transfer
  function.

- [ ] **Step 1: Write the failing tests** (`color.rs` `mod tests`):
  - `from_hex("f80") == from_hex("FF8800")`; `from_hex("ff880080")` has `a == 128/255`;
    `from_hex("ff88")` is `#ffff8888`; `from_hex("ff8")` and `from_hex("ff8800f")` are `None`.
  - formats of `#ff8800`: Hex `#ff8800`, Rgb `rgb(255, 136, 0)`, Hsl `hsl(32, 100%, 50%)`,
    Oklch `oklch(0.744 0.181 56.5)`.
  - `#ff0000` Oklch `oklch(0.628 0.258 29.2)`; `#ffffff` Oklch `oklch(1 0 0)`;
    `#008000` Oklch `oklch(0.52 0.177 142.5)`.
  - alpha: `#ff880080` Hex `#ff880080`, Rgb `rgba(255, 136, 0, 0.502)`, Hsl
    `hsla(32, 100%, 50%, 0.502)`, Oklch `oklch(0.744 0.181 56.5 / 0.502)`.
  - `named("Orange")` formats Hex `#ffa500`; `named("transparent")` Hex `#00000000`;
    `named("nope")` is `None`; the table has 149 entries.
  - `name_of(named("orange"))` is `Some("orange")`; `name_of(from_hex("ff8801"))` is `None`.
  - round trips: `from_hsl(32, 1, 0.5, 1)` Hex `#ff8800`; Oklch of
    `from_oklch(0.744, 0.181, 56.5, 1)` Hex is `#ff8800` (±1 per channel is a failure: pick
    an exact case); `from_oklch(0.9, 0.4, 150, 1)` (out of gamut) gives channels in 0–1.
  - `from_rgb(300, -5, 0, 1)` Hex `#ff0000`.
- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement.** Format rules from the spec: hex lowercase; rgb integers;
  alpha with at most 3 decimals trimmed; hsl parts at most 1 decimal trimmed; oklch L and C
  3 decimals trimmed, H 1 decimal trimmed, hue 0 when C < 0.0005. Store the name table as
  one `&[(&str, u32)]` sorted by name.
- [ ] **Step 4: Run and see them pass.**
- [ ] **Step 5: Commit** `feat(calc): a colour model with hex, rgb, hsl, oklch and CSS names`.

### Task 3: Colours in expressions and the `#` rule

**Files:**
- Modify: `plugins/calc/src/engine/lexer.rs` (`Tok::Hex(String)`, `Tok::Comma`,
  `lex_colors`)
- Modify: `plugins/calc/src/engine/parser.rs` (colour calls, `Target::Color(Form)`, names)
- Modify: `plugins/calc/src/engine/eval.rs` (colour values, conversion dispatch, errors)
- Modify: `plugins/calc/src/engine/mod.rs` (`Value::Color`, two-pass `eval_line` and
  `eval_query`, `is_mathy`)
- Modify: `plugins/calc/src/engine/format.rs` (`Value::Color` → `color::format(c, c.form)`)
- Modify: `plugins/calc/src/sheet_parity.rs` (colour rows)
- Test: `engine/mod.rs` tests, `sheet_parity.rs`

**Interfaces:**
- Consumes: Task 2's `color` API.
- Produces: `Value::Color(Color)`; `Value::quantity()` is `None` for it;
  `pub fn lex_colors(line: &str) -> Result<Vec<Tok>, String>`: as `lex`, but `#` followed by
  exactly 3, 4, 6 or 8 hex digits and then not a letter, digit or `_` is `Tok::Hex(digits)`
  (any other `#` still ends the line); `,` lexes to `Tok::Comma` in both functions
  (`describe` gives `,`).
  `Expr::ColorCall { func: ColorFn, args: Vec<Expr>, alpha: Option<Box<Expr>> }` with
  `enum ColorFn { Rgb, Hsl, Oklch }` (`rgba`/`hsla` map to `Rgb`/`Hsl`).

- [ ] **Step 1: Write the failing tests** in `engine/mod.rs`:
  - `colours`: table (via `one`):
    `#ff8800` → `rgb(255, 136, 0)`; `#f80 in hex` → `#ff8800`; `#ff8800 in hsl` →
    `hsl(32, 100%, 50%)`; `#FF8800 to oklch` → `oklch(0.744 0.181 56.5)`;
    `rgb(255, 136, 0)` → `#ff8800`; `rgb(255 136 0 / 50%) in hex` → `#ff880080`;
    `rgba(255, 136, 0, 0.5) in rgb` → `rgba(255, 136, 0, 0.5)`;
    `rgb(100%, 50%, 0%) in rgb` → `rgb(255, 128, 0)`; `hsl(32, 100%, 50%) to hex` →
    `#ff8800`; `hsl(32deg 100% 50%)` → `#ff8800`; `hsl(0.5 turn, 100%, 50%) in hex` →
    `#00ffff`; `oklch(0.628 0.258 29.2) in hex` → `#ff0000`; `oklch(62.8% 0.258 29.2) in hex`
    → `#ff0000`; `orange in rgb` → `rgb(255, 165, 0)`; `Orange` → `#ffa500`;
    `tan in hex` → `#d2b48c`; `tan(0)` → `0`; `rgb(300, 0, 0)` → `#ff0000`;
    `rgb(1, 2)` → `error: rgb needs 3 or 4 values`; `255 in rgb` →
    `error: rgb needs a color`; `#fff in bin` → `error: bin needs a plain number`;
    `orange + 1` → `error: cannot add a color`; `-orange` → `error: cannot negate a color`;
    `sqrt(orange)` → `error: sqrt needs a plain number`; `255 in hex` → `0xff` (unchanged).
  - sheets: `run("brand = #ff8800\nbrand in hsl\nb = #ff8800 in hsl\nb")` →
    `["brand = rgb(255, 136, 0)", "hsl(32, 100%, 50%)", "b = hsl(32, 100%, 50%)",
    "hsl(32, 100%, 50%)"]`; `run("r = 10\nrgb(r, 0, 0) in hex")` → second line `#0a0000`;
    `run("red = 5\nred + 1")` → `["red = 5", "6"]`.
  - `hash_comments_stay_comments`: `run("# note\n#todo\n#add later\n#add + more\nx = 5 #fee\n5 + 3 #fee\n#fee")`
    → `["empty", "empty", "empty", "empty", "x = 5", "8", "rgb(255, 238, 238)"]`;
    `one("#fff + 1")` → `empty`; `one("c = #fff + 1")` → `error: cannot add a color`;
    `one("#ff8800 in hls")` → `empty`.
  - `colour_search_rules`: `eval_query` answers `#ff8800`, `#ff8800 in hsl`,
    `rgb(255, 136, 0)`, `orange in hex`; gives `None` for `orange`, `red apple`,
    `#hashtag`, `meeting #42`, `5 + 3 #note`, `#fff + 1`.
  - `sheet_parity.rs`: add rows `#ff8800 in hsl`, `orange in rgb`, `rgb(255, 136, 0)`,
    `#f80`.
- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement.**
  - Parser: `Tok::Hex(d)` in `prefix` is `Expr::Lit(Value::Color(from_hex(d)))` (the lexer
    only emits valid digit counts). In `name()`, `rgb rgba hsl hsla oklch` followed by `(`
    parse a colour call: if a `Comma` follows the first argument, arguments are
    comma-separated full expressions (`expr(0)`), the 4th is alpha; otherwise
    space-separated arguments parsed with `expr(6)`, then an optional `/ alpha`. Missing
    `(` after these words leaves them names. A function name not followed by `(` but naming
    a colour (`tan`) is `Expr::Name`. `TARGET_WORDS` gains `rgb hsl oklch` →
    `Target::Color(Form)`.
  - Eval: a colour argument that `is_percent` scales its range (rgb channels 255, alpha 1,
    hsl s/l 1, oklch L 1); a hue with an `Angle` unit converts to degrees; others must be
    plain. `Target::Base(Some(16))` on a colour gives `Form::Hex`, other bases the existing
    "needs a plain number" error; `Target::Color(form)` on a colour sets `form`, on anything
    else `"{word} needs a color"`. A name with no variable, time word, unit or currency
    meaning that `color::named` knows is that colour. Any arithmetic, `%`, `of`, negation or
    function on a colour: `cannot add a color`, `cannot subtract a color`,
    `cannot multiply a color`, `cannot divide a color`, `cannot negate a color`,
    `{func} needs a plain number`, `a percentage needs a plain number`.
  - `eval_line`: lex with `lex_colors`; if no `Tok::Hex` came out, go on as today. Else try
    parse+eval; on failure re-run the line with `lex` (comment reading) and use that result
    unless it is an `Error`, in which case return the colour reading's error.
  - `eval_query`: drop the `contains('#')` early return; lex with `lex_colors`; anything
    that fails is `None` as before. `is_mathy`: `Expr::Lit(Value::Color(_))` and
    `Expr::ColorCall` are math; a bare colour name is not.
- [ ] **Step 4: Run all calc tests and see them pass** (every existing test unchanged).
- [ ] **Step 5: Build and record the wasm size** (gate in Global Constraints).
- [ ] **Step 6: Commit** `feat(calc): colours in expressions; # is a colour only where it works`.

### Task 4: The colour answer: swatch, caption, copy actions, sheet swatch

**Files:**
- Modify: `plugins/calc/src/views.rs`, `plugins/calc/src/lib.rs`
- Test: `views.rs` tests; `crates/plugin-host/tests/calc.rs`

**Interfaces:**
- Consumes: `Value::Color`, `color::{format, name_of, rgba8, swatch_hex, Form}`, SDK
  `LineAnnotation::swatch`.
- Produces: `Answer.swatch: Option<[u8; 4]>`; `pub fn color_copy(input: &str, form: Form,
  ctx: &Context) -> Option<String>` in `views.rs`; action ids `copy-hex`, `copy-rgb`,
  `copy-hsl`, `copy-oklch` titled `Copy as HEX`, `Copy as RGB`, `Copy as HSL`,
  `Copy as OKLCH`.

- [ ] **Step 1: Write the failing tests** (views):
  - `colour_answer_parts`: `answer_parts("#ff8800 in hsl")` has result
    `hsl(32, 100%, 50%)`, result caption `Color`, swatch `[255, 136, 0, 255]`;
    `answer_parts("orange in hex")` has result caption `Color · orange`.
  - `colour_tile_has_a_swatch`: the frame for `#ff8800 in hsl` has a canvas node 72×72
    whose first command is a rounded rect (radius 10) filled `0xff8800ff`;
    `swatch_has_a_border`: it also has a stroked rect in `ColorToken::Border`;
    a translucent colour (`#ff880080`) has checkerboard rects before the colour rect, none
    outside the rounded square; a number answer has no canvas.
  - `colour_actions`: the search answer for `#ff8800` has actions `copy`, `copy-both`,
    `copy-hex`, `copy-rgb`, `copy-hsl`, `copy-oklch` (in this order); a number answer only
    the first two; the Home frame for `#ff8800` has `copy-live`, `copy-both-live` and the
    four colour actions.
  - `color_copy("#ff8800", Form::Oklch)` is `oklch(0.744 0.181 56.5)`;
    `color_copy("2 + 2", Form::Hex)` is `None`.
  - `annotations` of `"#ff8800\n2 + 2"`: line 0 has swatch `#ff8800`, line 1 none;
    `#ff880080` gets swatch `#ff880080`.
  - host (`tests/calc.rs`): typing `#ff8800 in hsl` in a sheet gives an annotation with
    text `hsl(32, 100%, 50%)` and swatch `#ff8800`; the search answer for it carries the
    four colour actions.
- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement.** Variant A layout from the spec: the right half becomes a row
  of the existing text column (grow) and the 72×72 canvas, gap 14. The canvas draws the
  checkerboard only when alpha < 1, keeping every cell inside the rounded square (the clip
  is rectangular), then the colour rect, then the 1 px border stroke. `answer_event` in
  `lib.rs` handles the four ids by `color_copy(input, …)` on both the search tile and a
  Home tile.
- [ ] **Step 4: Run calc and host tests and see them pass.**
- [ ] **Step 5: Commit** `feat(calc): colour answers show a swatch and copy in every form`.

### Task 5: Any base and the base words

**Files:**
- Modify: `engine/mod.rs` (`Base`), `engine/parser.rs`, `engine/eval.rs`, `engine/format.rs`
- Test: `engine/mod.rs` tests

**Interfaces:**
- Produces: `pub struct Base(pub u32)` (2–36) replacing the enum; `Target::Base(Option<Base>)`
  stays; `parser::base_word` returns `String` (`hex`, `bin`, `oct`, `decimal`, or
  `base 36`).

- [ ] **Step 1: Write the failing tests** (`bases_any`): `255 in base 36` → `73`;
  `255 in base 2` → `0b11111111`; `255 in base 16` → `0xff`; `255 in base 10` → `255`;
  `35 in base 36` → `z`; `-255 in base 36` → `-73`; `255 in hexadecimal` → `0xff`;
  `5 in binary` → `0b101`; `8 in octal` → `0o10`; `255 in base 1` →
  `error: base must be 2 to 36`; `255 in base 37` → same; `2.5 in base 3` →
  `error: base 3 needs a whole number`; `31 dec in Tokyo` unchanged. Existing base tests
  pass unchanged.
- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement.** `target_at` reads `base` followed by a plain decimal number
  token; the words join `TARGET_WORDS`; `format::based` prints prefixes for 2, 8, 16 and
  bare lowercase digits otherwise.
- [ ] **Step 4: Run and see them pass.**
- [ ] **Step 5: Commit** `feat(calc): any base from 2 to 36, and the words binary, octal, hexadecimal`.

### Task 6: More units

**Files:**
- Modify: `engine/units.rs`, `engine/parser.rs` (`target_at`, `merge_compound_units`),
  `engine/lexer.rs` (`L/100km`), `engine/eval.rs` (inverse units in `+`/`-`),
  `views.rs` (`dim_words`)
- Test: `units.rs` and `engine/mod.rs` tests

**Interfaces:**
- Produces: `Dim::{Energy, Power, Pressure, Frequency, DataRate, Fuel}`; `Unit.inverse:
  bool` (only fuel's `L/100km`); `dim_words`: `Energy`, `Power`, `Pressure`, `Frequency`,
  `Data rate`, `Fuel economy`.

- [ ] **Step 1: Write the failing tests** (`even_more_units`, via `one`):
  `1 kWh in J` → `3600000 J`; `1 kcal in kJ` → `4.184 kJ`; `1 BTU in J` →
  `1055.055853 J`; `1 eV in J` → `1.602176634e-19 J`; `1 hp in W` → `745.6998716 W`;
  `1 atm in hPa` → `1013.25 hPa`; `30 psi in bar` → `2.068427188 bar`; `760 mmHg in atm` →
  `1.000000142 atm`; `760 torr in atm` → `1 atm`; `2.4 GHz in MHz` → `2400 MHz`;
  `1 PB in TB` → `1000 TB`; `1 PiB in TiB` → `1024 TiB`; `100 Mbps in MB/s` → `12.5 MB/s`;
  `1 Gbit in MB` → `125 MB`; `10 L/100km in mpg` → `23.52145833 mpg`; `30 mpg in L/100km` →
  `7.840486111 L/100km`; `20 km/L in L/100km` → `5 L/100km`; `5 L/100km + 1 L/100km` →
  `6 L/100km`; `10 l / 100 km` → `0.1 mm2` (unchanged); `1 nmi in km` → `1.852 km`;
  `500 nm in µm` → `0.5 µm`; `1 ms in µs` → `1000 µs`; `1 µs in ns` → `1000 ns`;
  `10 st in kg` → `63.5029318 kg`; `5 ct in g` → `1 g`; `1 l in dl` → `10 dl`;
  `1 cl in ml` → `10 ml`; `1 cc in ml` → `1 ml`; `100 grad in deg` → `90 deg`;
  `1 turn in deg` → `360 deg`; `60 arcmin in deg` → `1 deg`; `3600 arcsec in deg` → `1 deg`.
- [ ] **Step 2: Test derive** (`new_units_make_each_other`): `2 kW * 3 h` → `6 kWh`;
  `1 kWh / 100 W` → `36000 s`; `1 kWh / 100 W in h` → `10 h`; `1 kWh / 1 h` → `1 kW`;
  `1 GB / 100 Mbps` → `80 s`; `10 MB/s * 1 min` → `600 MB`; `1 GB / 1 s` → `1 GB/s`
  (update the old error row); every existing row of `units_of_one_dimension_make_another`
  unchanged.
- [ ] **Step 3: Test names** (`multiword_zones_beat_units` and `new_names_are_soft`):
  `5pm in St Louis` and `now in St Petersburg` answer moments; `x = 5\nW = 3\nW + x` →
  `8`; `units::names_are_unique` covers new names; no new unit name or alias is a currency
  code or ticker (`currency::lookup`), a month, or a `zones::find` hit, except the old
  ones already allowed.
- [ ] **Step 4: Run and see them fail.**
- [ ] **Step 5: Implement.** Factors from the spec. `powers` becomes axes (length, time,
  data, energy) with base sizes: data base bit, data rate bit/s, energy J, power W; the
  result unit rule gains step 2 (a unit of the result dimension whose SI size equals an
  operand unit's SI size). `convert`/`convert_interval` handle `inverse`. The lexer turns
  `L/100km` and `l/100km` with no spaces into one `Ident("L/100km")`. `target_at`: a zone
  match of more than one token wins over a one-token unit or currency.
- [ ] **Step 6: Run and see them pass; build and record the size.**
- [ ] **Step 7: Commit** `feat(calc): energy, power, pressure, frequency, data rate, fuel economy and more`.

### Task 7: City short names

**Files:**
- Modify: `plugins/calc/src/engine/zones.rs` (`CITIES`, `tables_do_not_collide`)
- Test: `zones.rs` tests, `engine/mod.rs` table

- [ ] **Step 1: Write the failing tests**: `find("spb")`, `find("SPB")` and `find("piter")`
  → `Europe/Moscow`; `find("bcn")` → `Europe/Madrid`; `find("ekb")` →
  `Asia/Yekaterinburg`; `find("nsk")` → `Asia/Novosibirsk`; `find("la")` →
  `America/Los_Angeles`; `find("mad")`, `find("ist")` and `find("sin")` resolve to no
  short-name row (MAD stays a currency after `in`, IST stays India time); engine:
  `5pm bcn in msk` → `Thu, 19 Nov 2026, 19:00 MSK`; `100 usd in mad` still converts to
  MAD. `tables_do_not_collide` extended: no short name is a currency code or ticker, a unit
  name, a month, a keyword (including `rgb hsl oklch base binary octal hexadecimal`) or an
  abbreviation.
- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement** the spec's list as `CITIES` rows under a `// Short names`
  comment.
- [ ] **Step 4: Run and see them pass.**
- [ ] **Step 5: Commit** `feat(calc): city short names such as spb, bcn and ekb`.

### Task 8: Backlog, acceptance note, full check

**Files:**
- Modify: `docs/superpowers/backlog.md` (the Converter row; a new Text transforms row)
- Create: `docs/experiments/2026-11-calc-colors-acceptance.md`

- [ ] **Step 1: Backlog.** Converter row: "Folded into calc (2026-11-22): colours, number
  bases, more units and city short names; see the calc colours design." New row: "Text
  transforms | base64, URL encoding, letter case and similar | Which transforms; one plugin
  or several | WASM plugin(s) | Any time".
- [ ] **Step 2: Measure.** `just build`; record the `calc.wasm` size against 1,047,764.
  Run the conversions note's timing tests (`search_is_fast`,
  `sheet_of_1000_lines_is_fast`, `zone_sheet_of_10000_lines_is_fast`, and the host's
  `pad_and_search_stay_within_budget`) and record the figures.
- [ ] **Step 3: Write the acceptance note**: the numbers table, the rulings made on the
  user's behalf (from the spec's "my call" items and anything decided during the build),
  and a user checklist driven by `just try` (colour tile with swatch, copy actions, sheet
  swatch, `#` comments unchanged, bases, a few new units, derive cases, short city names).
- [ ] **Step 4: `just check`** passes.
- [ ] **Step 5: Commit** `docs: calc colours acceptance; converter folded into calc in the backlog`.
