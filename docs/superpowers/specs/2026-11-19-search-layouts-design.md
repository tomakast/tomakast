# Search across keyboard layouts and scripts

Status: draft for the user's approval (2026-11-19). Spike: `docs/experiments/2026-11-search-layouts-spike.md`.

## Intent

The user, like many users, types in several languages and switches layouts all the time. Often they type before switching: «ашкуащч» instead of «firefox». Or they write a Latin name in Cyrillic letters: «опен» for «open», «хром» for Chrome. The launcher should find what they meant as if they had typed it right. Matches for the query as typed (Cyrillic, Georgian, Armenian or Latin) must still be found. The inspiration is IntelliJ's Search Everywhere. It remaps a wrong layout only, in one direction (non-Latin → Latin), with a fallback matcher (`FixingLayoutMatcher`, `MatcherWithFallback` in intellij-community `platform/util/text-matching`). This design goes further: both directions, plus transliteration.

## Goals

1. **Layout remap, both directions.** A query typed on a supported non-Latin layout also matches as the same keys on the US layout: `ашкуащч` → Firefox, `j,kfrj` → «Облако» (punctuation keys included). A Latin query also matches as the same keys on each supported layout: `afqks` → «Файлы».
2. **Transliteration, both directions, across scripts.** A Cyrillic, Georgian or Armenian query matches Latin names written the same way, and the reverse. The user's target is "level 3":
   - letter for letter: `опен` → Open, `fajly`/`faily`/`fayly` → «Файлы», `failebi` → ფაილები;
   - the usual spelling ambiguities: `калк` → Calculator (к = c), `вскод` → VS Code;
   - digraphs and letters left out: `хром` → Chrome (х = ch, no final e), `джира` → Jira, `ютуб` → YouTube, `гугл` → Google.
3. **As typed first when equal.** Every item scores the best of its readings (as typed, layout remap, transliteration). Remaps and transliteration carry a small penalty, the same idea as keywords scoring 3/4 of a title (the user's choice, option B). At equal match quality the as-typed match wins. A clean remap (`firefox`) still beats a scattered as-typed match.
4. **Everywhere the core matches text**: the global search (`Engine::search_with`), core-filtered list views (`views/list.rs`) and the action panel (`views/action_panel.rs`).
5. **Dynamic sources get the readings too.** Plugins with the `search` permission, native dynamic modules, and lists a plugin filters itself (`query-changed`) receive the query as typed plus its readings (user's choice, option C). WIT 0.1 is edited in place for this (the user: "забей на замороженный wit").
6. **Usage follows the layout.** Usage history compares queries modulo layout: launching Firefox with `ашкуащч` counts for `fire` later, and the reverse.

## Supported scripts and layouts

- **Cyrillic**: Russian, Ukrainian, Belarusian, Kazakh, Kyrgyz, Tajik, Uzbek (Cyrillic).
- **Georgian, Armenian.**
- **Latin** is the other side of every remap: the US layout.

The user scoped it as "Cyrillic plus the post-Soviet countries with their own alphabet". Other scripts (Greek, Hebrew, Arabic, CJK…) are out for now. Adding one later means adding tables, not changing the design.

## Non-goals

- **Spelled-by-sound guessing ("level 4")**: `фаерфокс` → Firefox, `стим` → Steam, `скайп` → Skype. The user agreed to stop at level 3. Guessing English pronunciation brings many false matches.
- **Transliteration within one script.** Latin against Latin is matched as today, so `kalk` does not start finding `calc` (the user: "речь именно о разных алфавитах, а не языках"). The same holds within Cyrillic.
- **Reading the user's layouts from the OS** (xkb on KDE, TIS on macOS, HKL on Windows). Built-in tables cover the chosen scripts on every OS. IntelliJ's trick of learning `key code → char` pairs from key events is a possible later step.
- **Showing which reading matched.** The launcher has no match highlighting today, and this adds none.

## Anti-goals

- Ordinary Latin queries getting noisier. With an all-Latin index the results must be identical to today's.
- A noticeably slower launcher.
- Losing as-typed Cyrillic results.

## Constraints

- **Speed**: ≤ 5 ms per keystroke on 10k items (core spec), and the ~50 ms open budget. Readings of the query are computed per keystroke, and readings of items once, when they are indexed.
- **nucleo-matcher 0.3** stays the scorer.
- **Parallel branches** `fb-calc` and `fb-kitchen-sink` edit `plugins/calc`, `plugins/kitchen-sink`, the WIT (fb-kitchen-sink already re-hashed `FROZEN`) and soon `launcher_view.rs` (fb-calc's query history plan). Keep commits focused, and keep existing plugin and module sources compiling unchanged.

## Approaches

- **A. Query readings plus a precomputed phonetic skeleton per item (chosen).** Every item text gets a Latin skeleton once, at index time. Each keystroke runs nucleo a few times: as typed, per layout remap, and the skeleton of the query against item skeletons. Little new code, the same scorer, and the spike shows it fits.
- **B. Query expansion only.** Enumerate spellings of the query (`калк` → kalk, calk, kalc, calc…) and match each. The number of variants explodes on long queries, a cap loses matches, and every variant is another pass. Worse and slower than A.
- **C. Our own fuzzy matcher** with per-letter alternatives (х ≈ h | kh | ch). The most precise, but it replaces nucleo's scoring with our own, is the most work, and its speed is unknown.

The user picked A after the trade-offs above.

## Design

### Readings of a query

The **script** of a query is the set of scripts of its letters: Latin, Cyrillic, Georgian, Armenian, or other. Digits, spaces and punctuation have none.

- **Fewer than two letters, more than one script, or "other"**: only as typed. The spike's single top-1 change came from the one-letter query `x`.
- **One non-Latin script**:
  - *layout remaps*: for each supported layout of that script whose table has every letter of the query, map every character through it. Characters not in the table stay as they are. Deduplicate. The Cyrillic layouts agree except on national letters (`і қ ғ ң ү ҳ`), so this is usually one string and at most a few. Each remap is matched against Latin texts.
  - *skeleton*: the query's skeleton, matched against the skeletons of texts in any other script.
- **Latin only**:
  - *layout remaps*: for each supported layout whose script occurs in the index, map through the inverted table (punctuation keys included). Deduplicate. Each is matched against texts of that script. Scripts absent from the index cost nothing.
  - *skeleton*: the query's skeleton, matched against the skeletons of non-Latin texts.

### Item side

For each item text (title and each keyword), keep its script set (a mixed text such as «Параметры NVIDIA X Server» counts for each of its scripts) and its skeleton, computed when the item is indexed. Where the alternate rules change it, also keep one alternate skeleton (see below). Texts in Latin only still get a skeleton, because a Cyrillic query is matched against it. Memory is about one extra copy of the text, which is fine at app-search scale. If file search later puts hundreds of thousands of paths into the core index, its own design decides whether paths get skeletons.

### The skeleton

Lowercase, transliterate, fold. The spike's rules are the starting point (`docs/experiments/2026-11-search-layouts-spike.md` lists them):

- **transliteration**: Cyrillic per a GOST-like table, national letters included; Georgian per the national system (2002); Armenian per ISO 9985; apostrophes dropped;
- **folding**: `ch → h`, `c` before `e i y` → `s`, other `c` → `k`, `zh dzh dj → j`, `kh → h`, `ph → f`, `ts → s`, `oo ou → u`, `x → ks`, `q → k`, `w → v`, `y → i`, drop `i` between a consonant and a vowel, collapse doubled letters;
- **alternate skeleton**, only where it differs: `й → j` (Russian romanization writes `j` for `й`, English names write it for `дж`) and Georgian `ფ → f` (loanwords).

The builder may tune the rules. The pair list in the spike note is the acceptance test.

### Scoring

For each item text, the score is the best of:
- as typed: nucleo score, as today;
- a layout remap: nucleo score × 7/8;
- the skeleton (or the alternate): nucleo score × 3/4.

A layout or skeleton score counts only if it reaches 0.8 × the score the same pattern gets against its own text. That cuts scattered cross-script fuzzy hits: at 0.8 extra hits fell from 3801 to 506, and no strong match was pushed down. Then the keyword factor (×3/4) and the item rule (best text) apply as today, and so does usage (`combined_score`). The factors and the ratio are named constants, tuned against the spike's pair list and regression guard.

### List views and the action panel

These filter rows the core never indexed (`score_text`). The same readings apply. Row skeletons are computed once, when the view is pushed or updated, not on every keystroke. The cap is 2000 rows per list.

### Dynamic sources (plugins and native modules)

Dynamic answers are not matched by the core: each gets the fixed `DYNAMIC_MATCH_SCORE`, so the remap penalty cannot apply to them. They get the readings instead and decide themselves.

**WIT** (edited in place in `since_v0.1.0`, re-hashed, with a `FROZEN.md` row giving the reason; the user's ruling):

```wit
/// Another reading of the query than as typed.
enum query-reading-kind {
  /// The same keys on another layout: "ашкуащч" → "firefox", "afqks" → "файлы".
  layout,
  /// A romanization of a non-Latin query: "опен" → "open".
  translit,
}

record query-reading {
  kind: query-reading-kind,
  text: string,
}

/// `readings`: at most 8, never equal to `query` or to each other.
export search: func(query: string, readings: list<query-reading>) -> result<list<item>, string>;
```

- **Layout readings** are the remaps above. For a Latin query, they are limited to scripts that occur in the core index.
- **The translit reading** is a readable romanization (`хром` → `hrom`), the transliteration step without the folding. The skeleton is an internal key, not text. A Latin query gets no translit reading, because romanizing Latin into Cyrillic is guesswork.

**SDK**: `Plugin::search(query: String)` stays. A new `Plugin::search_with(query: String, readings: Vec<QueryReading>)` defaults to calling `search(query)`, and the export calls `search_with`. calc and kitchen-sink compile and behave as before.

**Native modules**: `Module::search(&mut self, query: &str)` stays. A new `Module::search_with(&mut self, query: &str, readings: &[QueryReading])` defaults to it, and the host calls `search_with`. The query readings are a core type that the plugin host converts.

**Lists the plugin filters itself** (`search-mode::module`) *(author's call: the same goal, and the in-place window is open now)*: the WIT `query-changed` view event gets the same `readings: list<query-reading>`, and so do core `ViewEvent::QueryChanged`, the host's conversion and the SDK type. No plugin uses module mode today. Existing matches on `QueryChanged` keep compiling (demo matches `{ .. }`, and calc and kitchen-sink read fields by name).

More than 8 readings: keep the layout readings first, in table order, then the translit one, and drop the rest. Readings are computed once per keystroke and shared by the core match and every dynamic source.

The permission prompt ("See what you type in the launcher search") stays true and needs no change.

### Usage history

`UsageModel::stats` counts a record when its query starts with the current one. Both sides are compared in a layout-normalized form: a query in one non-Latin script is remapped to Latin when every letter has a table entry, and anything else is left as is. Where the layouts of a script disagree on a national letter, the first layout in table order that has all the letters wins (Russian first). Records keep what was typed, so no migration is needed. The normalized form of each record is computed once when records load, or once per record, not per keystroke.

### Tables

- **Layout tables are generated**, not typed by hand, from `xkbcli compile-keymap` against `us`, through libxkbcommon keysym names. The spike did this; its parser and quirks are in the spike note. The output is checked-in Rust tables, with the generator script kept next to them (`crates/core/tools/` or similar, the builder's call). Tests do not need xkb.
- **Several layouts per script** where they exist. The builder checks the macOS and Windows defaults for Russian (macOS «Russian» vs «Russian – PC» punctuation and `ё`; xkb has `ru(mac)` and `ua(macOS)`), Georgian and Armenian. xkb has no Mac variant for `ge` or `am`, so those come from Apple's published keyboard layouts and are cited in the table. Each default that differs from xkb's adds a table. A query is remapped through every layout of its script that has all its letters, so extra tables cost little.
- **Transliteration tables** cite their standard in a comment.

## Acceptance

- **Pairs**: the full pair list in the spike note ("Full pair list"), levels 0–3, as tests. Each expected item lands in the top 3 of a fixed corpus checked into the tests. Level 4 is not tested either way, so the rules may get looser later. *(Author's call.)*
- **Corpus** *(author's call)*: synthetic and public, not a snapshot of anyone's machine. It holds the spike's added names, plus about 100 common desktop apps under their English names, plus about 30 of them under their public Russian KDE/GNOME translations («Параметры системы», «Диспетчер разделов», «Системный монитор»…), so the as-typed Cyrillic competitors the spike met (`калк` vs «Калькулятор», `хром` vs Chromium) are present. Where an expected item is not first, the test asserts the item that beats it, as the spike saw it.
- **Regression guard**: over the corpus's Latin-only items, the result order for every 1–6 letter prefix of every Latin title word is identical with the feature on and off. Over the full corpus no top-1 result changes for those queries.
- **Benchmark** *(author's call)*: the same shape as `search_10k_under_5ms` in `crates/core/tests/engine.rs`, which takes the median of 20 runs. It uses 10k items, each with a title and two keywords, a fifth of them Cyrillic. Queries are typed letter by letter: `firefox`, `ашкуащч`, `калькулятор`, `fajly`. The assertion is a median ≤ 5 ms for every prefix of two or more letters. The spike's heavier corpus (five texts per item, 5.7 ms worst) is re-measured and written to `docs/experiments/` as a stress figure, not asserted.
- **Manual check by the user** through `try.sh`: `ашкуащч`, `хром`, `опен`, `afqks` against their real apps.

## Touch list

`crates/core/src/matcher.rs` (new reading and skeleton code, likely split into modules), `index.rs` (per-text precompute), `engine.rs`, `usage.rs`, `module.rs`, `host.rs`; `crates/ui/src/views/list.rs`, `views/action_panel.rs`; `crates/plugin-api/wit/since_v0.1.0/plugin.wit`, `FROZEN`, `FROZEN.md`, `crates/plugin-api/tests/freeze.rs`; `crates/core` view events (`ViewEvent::QueryChanged`); `crates/plugin-host` (search call, view-event conversion, `wit_coverage.rs`); `crates/plugin-sdk` and its README; `docs/plugins/writing-a-plugin.md`; core spec ("Search and ranking", the `search` export); roadmap "WIT 0.2 candidates" (no new gap, but note the in-place change).

## Left to the builder

- Exact folding rules and table contents, within the acceptance tests.
- Penalty factors and the threshold ratio (starting at 7/8, 3/4, 0.8).
- Module layout inside `tomakast-core`, type names, where the generator lives.
- How list views cache row skeletons.
- Whether to parallelize the scan, if the benchmark asks for it.

## Merge notes

- fb-kitchen-sink re-hashed `FROZEN` and added a `FROZEN.md` row. Merging it and this branch conflicts on one line in each. Resolve by keeping both rows and re-hashing.
- calc and kitchen-sink keep `search(query)` through the SDK default, so their branches need no edit.
