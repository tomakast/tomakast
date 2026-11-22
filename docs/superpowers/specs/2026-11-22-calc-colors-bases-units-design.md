# calc: colours, number bases and more units

Date: 2026-11-22. Status: approved by the user (2026-11-22), with the city short names added. Source: the converter
session, started from the main session. The backlog's "Converter" entry is folded into calc.

## Why

The backlog had a separate "super-duper converter like Raycast's" plugin. The user decided
that calc already is that converter: it converts units, currencies (with crypto), time zones
and dates, both in the root search and in its sheets. What the converter was meant to add
goes into calc instead:

1. **Colours**, with the colour itself visible.
2. **Number bases**, beyond the hex, binary and octal that already work.
3. **More units**, toward what Raycast's calculator and everyday needs cover.

Text transforms (base64, URL encoding, letter case and so on) are not part of this. They
become separate plugins later.

## What exists today (so the builder does not redo it)

- Bases: `255 in hex` → `0xff`, `in bin` → `0b11111111`, `in oct` → `0o377`, `0xff in
  decimal` → `255`. The literals `0xff`, `0b1010` and `0o17` work in expressions, and a
  literal alone answers in search. Output is lowercase (`0xff`), a shipped ruling kept here.
  Whole numbers up to 2^53 only.
- Units (`plugins/calc/src/engine/units.rs`): length, mass, time, temperature, area, volume,
  speed, data sizes (`B KB MB GB TB`, `KiB MiB GiB TiB`, `bit`) and angle (`deg rad`).
  Length, time, area, volume and speed multiply and divide into each other (`derive`).
- `#` starts a comment in a sheet (`lexer.rs`), and search gives no answer to any query
  that contains `#` (`eval_query` in `engine/mod.rs`).
- The answer tile (`views.rs`, `draw_answer`) is two halves, input and result, each with a
  caption. It is drawn from the frame builder of the widget contract, which can draw a
  canvas of filled and stroked rectangles in any RGBA colour.
- A sheet annotation (`line-annotation` in the contract) is text and an error flag only.
- `calc.wasm` measured 972,417 bytes at the start of this work, against a gate of 982,228.

## Goals

Everything below works in the search answer and in sheets, and the two agree (the existing
`sheet_parity` tests extend to it).

### Colours

**Reading a colour.** All of these are colour values:

| Written | Notes |
|---|---|
| `#f80`, `#ff8800` | Hex, any case. |
| `#f808`, `#ff880080` | Hex with alpha. |
| `rgb(255, 136, 0)`, `rgba(255, 136, 0, 0.5)` | Comma form. A channel may be a percentage (`rgb(100%, 50%, 0%)`); alpha is a number 0–1 or a percentage. |
| `rgb(255 136 0)`, `rgb(255 136 0 / 50%)` | Modern space form, alpha after `/`. `rgba` and `rgb` are the same function. |
| `hsl(32, 100%, 50%)`, `hsla(…)`, `hsl(32deg 100% 50% / 0.5)` | Hue in degrees (a bare number, or `deg`/`rad`/`turn`); saturation and lightness as percentages. |
| `oklch(0.735 0.176 57.3)`, `oklch(73.5% 0.176 57.3 / 0.5)` | Lightness as a number 0–1 or a percentage; chroma a number; hue in degrees. |
| `orange`, `rebeccapurple`, `Orange` | The 148 CSS named colours, any case, plus `transparent`. |

The arguments of `rgb`/`hsl`/`oklch` are expressions, so variables work
(`rgb(r, 136, 0)`). In the space form an argument binds tighter than `+ -` and `/`, so `/`
always separates alpha and `rgb(255 -1 0)` is three arguments. Out-of-range channels are
clamped (`rgb(300, 0, 0)` is `#ff0000`); a wrong number of arguments is an error
(`rgb needs 3 or 4 values`).

**Converting.** `in`/`to` a form: `hex`, `rgb`, `hsl`, `oklch`.

- `#ff8800 in hsl` → `hsl(32, 100%, 50%)`
- `rgb(255, 136, 0) to hex` → `#ff8800`
- `orange in rgb` → `rgb(255, 165, 0)`
- `#ff8800 in oklch` → `oklch(0.744 0.181 56.5)`

`hex` already names a base target. It now dispatches on the value: a colour gets the hex
colour, a plain number gets `0x…` as today. `rgb`, `hsl` and `oklch` applied to a number are
an error (`rgb needs a color`).

**Showing a colour.** A colour keeps the form it should be shown in:

- converted with `in`/`to`: that form;
- written in hex with no conversion: shown as `rgb(…)` (echoing the hex would say nothing);
- written any other way with no conversion: shown as hex.

So `#ff8800` answers `rgb(255, 136, 0)`, and `orange`, `rgb(…)`, `hsl(…)` and `oklch(…)`
answer `#ffa500`-style hex. A variable keeps the form of what it holds
(`brand = #ff8800 in hsl`, then `brand` shows the HSL form).

Output formats:

- hex: lowercase, `#rrggbb`, or `#rrggbbaa` when alpha is below 1;
- rgb: `rgb(255, 136, 0)`, or `rgba(255, 136, 0, 0.5)` with alpha (alpha with at most 3
  decimals, trailing zeros trimmed);
- hsl: `hsl(32, 100%, 50%)`, each part with at most 1 decimal, trimmed; `hsla(…, 0.5)` with
  alpha;
- oklch: `oklch(0.735 0.176 57.3)`: lightness and chroma with 3 decimals, hue with 1
  decimal, trimmed; `oklch(… / 0.5)` with alpha. A grey (chroma below 0.0005) shows hue 0.

A colour is stored as sRGB channels in 0–1 (as floating point, so `in hsl in oklch` does not
lose precision to rounding on the way). OKLCH outside the sRGB gamut is clipped per channel
when it is read; the clipped colour is what every form and the swatch show.

**What answers in search.** By the same rule as `0xff` ("writing a value in another
notation is a conversion"): `#ff8800`, `rgb(…)`, `hsl(…)` and `oklch(…)` alone answer; a
conversion answers. A colour name alone (`orange`) does not answer, as `100 usd` alone does
not. In a sheet, `orange` alone is the colour.

**Names are soft.** A variable named `red` or `orange` wins over the colour. The function
names `rgb rgba hsl hsla oklch` are colours only when followed by `(`. `tan` is also a
function and stays one (it still cannot be a variable): `tan(1)` is the tangent,
`tan in hex` and `x = tan` are the colour.

**Arithmetic.** A colour takes part in no arithmetic: `#fff + 1` is an error
(`cannot add a color`), and so are `-`, `*`, `/`, `^`, `mod`, `%`, `of` and functions.
Mixing colours is a non-goal.

**The `#` rule.** A `#` is read as a colour only where a value can stand, and only if the
whole line then evaluates; otherwise everything from the `#` on is a comment, as today.
Concretely: the line is lexed with `#` followed by exactly 3, 4, 6 or 8 hex digits and then
no letter, digit or `_` read as a colour; if that line fails to lex, parse or evaluate, it is
evaluated again with `#` as a comment. Consequences:

- every line that evaluates today keeps its answer: `5 + 3 #fee` is still `8`, and
  `#add later` is still a comment;
- a line that is only a colour expression (`#fee`, `#bad in hsl`) now answers the colour
  where it used to be an empty comment line;
- search answers a query with `#` only when the colour reading evaluates (and is math, as
  above); `5 + 3 #note` still gets no answer.
- (my call) the price of keeping comments safe: a line that starts with a hex colour and
  then fails is a comment, so `#fff + 1` or the typo `#ff8800 in hls` shows nothing in a
  sheet. A comment such as `#add later` or `#add + more` reads exactly like such a line,
  so the two cannot be told apart. Colours written without `#` (`orange + 1`, `rgb(…) + 1`,
  a variable) show their error as usual.
- (my call) when both readings fail, the colour reading's error is shown if that reading
  parsed (`c = #fff + 1` says `cannot add a color`, not `expected a value at the end of the
  line`); otherwise the comment reading's (`issue #1234` says `unknown name 'issue'`, as
  before). Changed after the final review.

**The swatch on the answer tile** (variant A, the user's pick). The result half of the tile
gets a square swatch at its right edge, about 72 logical pixels, corner radius 10, the
result text and its caption to its left (right-aligned as today). A hairline border in the
theme's border colour keeps a colour close to the tile's own background visible. A colour
with alpha is drawn over a small light/dark checkerboard inside the square, so the
transparency shows. The result caption is `Color`, followed by the CSS name when the colour
is exactly a named colour (`Color · orange`). Home tiles of a colour answer show the swatch
too (they evaluate their input afresh, which never needs rates for a colour).

**Copying.** Enter copies the result as shown (unchanged). A colour answer also has
`Copy as HEX`, `Copy as RGB`, `Copy as HSL` and `Copy as OKLCH` in the action panel, on the
search tile and on its Home tile. They copy the colour in that form, without alpha forms
changing (a colour with alpha copies `#rrggbbaa`, `rgba(…)`, and so on).

**The swatch in sheets** (the user said yes to the contract change). The contract's
`line-annotation` gains an optional swatch:

```wit
record line-annotation {
  line: u32,
  text: string,
  is-error: bool,
  /// A colour square drawn before `text`: `#rrggbb` or `#rrggbbaa` (hex digits, any
  /// case). Breaking this rule rejects the whole effect with an error toast, as a bad
  /// `icon.color` does.
  swatch: option<string>,
}
```

- calc sets it on every line whose value is a colour.
- The launcher draws a small square (about 12 logical pixels, corner radius 3, hairline
  border, the colour's alpha applied over the panel) at the start of the annotation, before
  the text, level with the line.
- The change follows the "WIT freeze lifted" practice: `plugin.wit` is edited in place, the
  `FROZEN` hash re-computed and a row added to `FROZEN.md`. The change touches the core view
  model, the host conversion and caps check, the SDK builder (a `.swatch(…)` setter on
  `LineAnnotation`) and the scratchpad view. Other plugins compile unchanged (they build
  annotations through the SDK).

### Number bases

Kept as they are, plus:

- the target words `hexadecimal`, `binary` and `octal` (same as `hex`, `bin`, `oct`); not
  `dec`, which is December (my call);
- any base from 2 to 36: `255 in base 36` → `73`, `255 in base 2` → `0b11111111`. Bases 2, 8
  and 16 print with their prefix as today; every other base prints its digits (lowercase
  letters past 9) with no prefix. `base 1` and `base 37` are errors
  (`base must be 2 to 36`). The same limits as today apply: whole numbers, at most 2^53.

There is no literal syntax for other bases; reading one back is a non-goal.

### More units

New dimensions and units, each with its usual long names and plurals as aliases. All new
names are soft (a variable of the name wins), as the soft-names ruling of the conversions
acceptance note requires.

| Dimension (caption) | Units |
|---|---|
| Energy | `J kJ MJ cal kcal Wh kWh eV BTU` |
| Power | `W kW MW GW hp` (mechanical horsepower) |
| Pressure | `Pa hPa kPa MPa bar mbar atm psi mmHg torr` |
| Frequency | `Hz kHz MHz GHz` |
| Data (added) | `PB`, `PiB`; bits `kbit Mbit Gbit Tbit` with aliases `Kb kb Mb Gb Tb` |
| Data rate | `bps kbps Mbps Gbps` (aliases `Kbps`), `B/s KB/s MB/s GB/s KiB/s MiB/s GiB/s` |
| Fuel economy | `mpg` (US), `L/100km` (aliases `l/100km`), `km/L` (aliases `km/l`) |
| Length (added) | `nm`, `µm` (aliases `um`, `micron`), `nmi` (nautical mile) |
| Time (added) | `µs` (alias `us`), `ns` |
| Mass (added) | `st` (stone), `ct` (carat) |
| Volume (added) | `cl`, `dl`, `cm3` (aliases `cm³`, `cc`) |
| Angle (added) | `grad` (aliases `gon`), `turn`, `arcmin`, `arcsec` |

Values: `cal` is the thermochemical calorie (4.184 J); `BTU` the international table BTU
(1055.05585262 J); `hp` 745.69987158227022 W; `psi` 6894.757293168 Pa; `mmHg`
133.322387415 Pa; `torr` 101325/760 Pa; `nmi` 1852 m; `st` 6.35029318 kg; `ct` 0.2 g; `mpg`
0.425143707 km/L.

**Units that make each other.** The existing `derive` grows so that:

- power × time = energy, energy / time = power, energy / power = time
  (`2 kW * 3 h` → `6 kWh`, `1 kWh / 100 W` → `36000 s`, and `in h` gives `10 h`);
- data / data rate = time, data rate × time = data, data / time = data rate
  (`1 GB / 100 Mbps` → `80 s`, `10 MB/s * 1 min` → `600 MB`, `1 GB / 1 s` → `1 GB/s`, which
  used to be the error `cannot divide GB by s`).

Energy and data join length and time as base axes, so this is one rule, not special cases.
The result unit (my call, one step added to the existing rule):

1. the unit whose size the result has (as today);
2. else a unit of the result's dimension whose size, in base units, equals one operand's
   unit size (`MB/s` is 8e6 bit/s, so `MB/s × min` lands in `MB`, 8e6 bit);
3. else the base unit of its dimension (as today).

Step 2 changes no answer pinned today (`1 km / 1 min` is still `16.66666667 m/s`,
`1 cm * 1 m` still `0.01 m2`, `10 m/s * 1 min` still `600 m`); the existing tests check
that.

**Fuel economy is inverse.** `L/100km` grows as `mpg` falls (`10 L/100km in mpg` →
`23.52 mpg`). A unit gets an "inverse" flag: its base value is `factor / number`. Adding and
subtracting fuel economies of the same unit adds their numbers; of different units converts
the right side to the left's unit first. `L/100km` is written with a number inside, so the
lexer/parser reads `L/100km` and `l/100km`, written with no spaces at all, as one unit name
(my call). With any space (`10 l / 100 km`) it stays volume over length, an area, as today.
This is the one deliberate exception to the "same answer" anti-goal: `10 l/100km` used to
answer `0.1 mm2`, which nobody who types it means.

**Zones of several words win over a one-word unit after `in`/`to`.** `st` (stone) must not
break `5pm in St Louis`, `now in St Petersburg` or `in St Johns`: when the words after
`in`/`to` name a zone of more than one word, the zone wins; `10 st in kg` still converts.
The new names are checked against currency codes, tickers and city names by a test, as the
duplicate-name test checks units.

**Not added** (with reasons, so it is not re-proposed): pixels and typographic points (they
need a DPI, and `pt` is already the pint); imperial (UK) gallons and pints (names collide
with US ones); light years and astronomical units; frequency × time.

### City short names

Added by the user after the first draft (2026-11-22): known short forms of cities, like
`msk`, `bcn`, `nyc`, `spb`, work wherever a city does (`now in spb`, `5pm bcn in msk`,
`15:00 nyc in ekb`). Today `msk` already works (as the `MSK` abbreviation) and so does `nyc`
(a city row); `spb` and `bcn` do not.

They go into the existing city table in `zones.rs` (any case, as every zone name). The set
(my call; each maps to the zone of that city):

- Russian everyday forms: `spb`, `piter` (Moscow time), `ekb` (Yekaterinburg), `nsk`
  (Novosibirsk), `kzn` (Moscow time), `vvo` (Vladivostok), `krr` (Krasnodar, Moscow time).
- English everyday forms: `la`, `sf`, `dc`, `philly`, `vegas`, `nola`, `hk`, `kl`, `hcmc`,
  `joburg`.
- IATA metropolitan and main airport codes of large cities: `lon`, `par`, `ber`, `bcn`,
  `rom`, `ams`, `bru`, `vie`, `prg`, `waw`, `bud`, `lis`, `ath`, `mow`, `dxb`, `tlv`, `cai`,
  `jnb`, `nbo`, `del`, `bom`, `blr`, `bkk`, `hkg`, `tyo`, `sel`, `bjs`, `pek`, `sha`, `syd`,
  `mel`, `akl`, `yto`, `yyz`, `yvr`, `yul`, `mex`, `sao`, `gru`, `bue`, `scl`, `lim`, `bog`,
  `chi`, `lax`, `sfo`, `sea`, `bos`, `mia`, `atl`, `den`, `phx`, `las`, `dfw`, `jfk`.

Left out because they mean something else in calc, so the other meaning keeps winning:
`mad` (MAD, the Moroccan dirham; Madrid stays `madrid`), `ist` (`IST`, India Standard
Time), `sin` (the sine function), `mil` (a common word for a thousandth of an inch, kept
free), `was`. A test checks the whole set against currency codes and tickers, unit names
(new ones included), months, colour targets, keywords and the abbreviation table, as
`tables_do_not_collide` does now.

### Backlog

`docs/superpowers/backlog.md`: the "Converter" row changes to say it is folded into calc
(colours, bases and units, by this design), and a new row "Text transforms" is added for
base64, URL encoding, letter case and similar, as separate WASM plugins later. Only those two
rows change.

## Non-goals

- Mixing, lightening or other colour arithmetic.
- Colour forms beyond hex, rgb, hsl, oklch and names (no `hwb`, `lab`, `lch`, `color()`,
  CMYK); a nearest-name search; `in name`.
- Literals in bases other than 2, 8 and 16.
- Text transforms.

## Anti-goals

- A line that works today giving a different answer or an error. Test it with every
  existing engine and sheet test, and with the `#` cases above.
- Search answering things that are not math: bare colour names, a sentence with a `#tag`.
- `calc.wasm` past the new gate.
- Breaking other plugins' builds or other sessions' work with the contract change.

## Constraints

- `calc.wasm` gate: **1,047,764 bytes** (the old 982,228 plus 64 KiB), the user's ruling of
  2026-11-22. No colour or unit crate: hand-written, like the rest of the engine. Size is
  measured and recorded in the acceptance note.
- Search speed: the existing budgets still hold (10 ms for a 1 KiB query through the host;
  engine-only figures recorded in the conversions acceptance note). The `#` rule may lex a
  line twice; that is fine within budget.
- Specs, plans and notes in English; the plugin's UI text is English.
- WIT 0.1 is edited in place (the user's ruling, repeated in this session's brief).
- Other sessions work in parallel (translator, settings GUI, file search): the shared-code
  change is the one `line-annotation` field and its plumbing, nothing else.

## Approaches

1. **Colours as a new kind of value in calc's engine (chosen).** A colour becomes one more
   value next to numbers, money and moments, read by the existing lexer and parser and
   converted by the existing `in`/`to`. Variables, `prev`, sheets and search all work with
   no extra plumbing, and search and sheets agree by construction. New units are rows in the
   existing table, plus two small generalisations (more base axes in `derive`, the inverse
   flag).
2. **A separate colour reader in front of the engine.** Recognise colour queries with their
   own parser before the engine. Smaller change to the engine, but variables
   (`brand = #ff8800`, `brand in hsl`) and `prev` would not work in sheets, and search and
   sheets would drift apart. Rejected.
3. **A colour library crate** (`palette`, `csscolorparser`). Saves a few hundred lines of
   formulas but spends a large share of the size budget, and calc's rule so far is
   hand-written parsing with no parser crates. Rejected.

For the sheet swatch, the alternative to the contract field was to smuggle the colour inside
the annotation text with a marker the launcher would strip. Rejected: an invisible
convention in a text field, which other plugins would trip over.

## Decided by the user (2026-11-22)

- Swatch look on the tile: variant A, a square beside the result.
- The swatch in sheets, with the small shared contract change.
- The unit list above as proposed.
- The `calc.wasm` gate raised by 64 KiB, to 1,047,764 bytes.
- City short names (`msk`, `bcn`, `nyc`, `spb` and the like), added after the first draft;
  the set is my call.

The proposals in the first message that the user did not correct are taken as agreed: the
`#` rule, the colour forms understood and produced, what a bare colour shows, bare names not
answering in search, soft names, the copy actions, lowercase `0xff`, and the backlog edit.

## Left to the builder

- The exact OKLCH formulas (Björn Ottosson's reference matrices) and the sRGB transfer
  function; tests pin known values (for example `#ff0000` → `oklch(0.628 0.258 29.2)`).
- How the lexer marks a colour token, and how the two-pass `#` rule is wired (it must give
  the same result in `Sheet::eval` and `eval_query`).
- Where the colour code lives (a new `engine/color.rs` is the natural place) and how the
  148-name table is stored compactly.
- The checkerboard's cell size, and the exact swatch size if 72 px does not fit the tile's
  padding.
- Error message wording, in the existing style (`cannot add a color`, `rgb needs a color`).
- Tests: engine tables for every form and conversion, the `#` cases, every new unit and the
  derive cases; a tile test for the swatch and the copy actions; host tests for the
  annotation swatch (valid, invalid rejected); a scratchpad UI test that the square is drawn.
- The acceptance note `docs/experiments/2026-11-calc-colors-acceptance.md`: the size against
  the gate, search and sheet timings, and a user checklist using the `just` recipes.
