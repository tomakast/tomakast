# Search Across Layouts and Scripts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A query typed on the wrong layout, or in another script, finds what was meant. `ашкуащч` → Firefox, `afqks` → «Файлы», `хром` → Chrome, `fajly` → «Файлы». As-typed matches keep winning when match quality is equal.

**Architecture:** Each item text gets a script set and a Latin phonetic skeleton when indexed. Each keystroke builds a `Pattern` from the as-typed text, the layout remaps and the query's skeleton. The cross-script readings are scored by nucleo against the original text or the skeleton, with a penalty and a quality floor. The same readings travel to dynamic sources, through a new `search` parameter in WIT 0.1, which is edited in place, and through `query-changed`.

**Tech Stack:** Rust 2024 (toolchain 1.98), nucleo-matcher 0.3, GPUI (gpui-kit =0.10.0), wasmtime component model, wit-bindgen in `crates/plugin-sdk`. The table generator is Python 3 with libxkbcommon via ctypes.

**Spec:** `docs/superpowers/specs/2026-11-19-search-layouts-design.md`. Spike with the pair list and numbers: `docs/experiments/2026-11-search-layouts-spike.md`.

## Global Constraints

- Scripts: Latin (US layout), Cyrillic (Russian, Ukrainian, Belarusian, Kazakh, Kyrgyz, Tajik, Uzbek Cyrillic), Georgian, Armenian. No others.
- Cross-script readings only, never Latin↔Latin or within one non-Latin script.
- Fewer than two letters, more than one script among the letters, or an "other" script letter means as typed only.
- Score per text = max(as typed, layout × 7/8, skeleton × 3/4). Keywords ×3/4 and usage as today.
- A layout or skeleton score counts only if ≥ 0.8 × the score the same pattern gets against its own text.
- `readings`: at most 8, never equal to the query or to each other. Layout readings come first in table order, then translit.
- ≤ 5 ms per keystroke (median of 20) on 10k items for queries of two or more letters.
- WIT 0.1 is edited in place: re-hash `FROZEN`, and add a `FROZEN.md` row with the reason "the user's ruling, 2026-11-19, search-layouts feedback".
- Existing plugin and module sources (calc, kitchen-sink, demo, apps) compile unchanged.
- fmt, clippy (`-D warnings`) and the relevant tests pass before each commit (`.github/workflows/ci.yml`). No attribution lines. Commit on `fb-search` only.

## Review Focus

- **Caps Lock or Shift on the wrong layout** (`АШКУАЩЧ`): it should still find Firefox. Tested in Task 3 (`readings_ignore_case`).
- **Multi-word queries** (`мшыгфд ыегвшщ`): spaces stay spaces, and the query finds Visual Studio Code. Tested in Task 5 (`multi_word_layout_query`).
- **Calculator input** (`2+2*3`, `15%`): no letters, so no readings. calc gets exactly what it gets today. Tested in Task 3 (`no_readings_without_letters`).
- **Empty or blank query**: matches everything with score 0, with no readings, as today. Tested in Task 4 (`blank_query_has_no_readings`).
- **A huge pasted query** (64 KiB): no readings past 256 characters, so there is no 8× blow-up and no slowdown. Tested in Task 3 (`long_query_has_no_readings`).

---

### Task 1: Scripts and layout tables

**Files:**
- Create: `crates/core/src/script.rs`, `crates/core/src/layout/mod.rs`, `crates/core/src/layout/tables.rs` (generated), `crates/core/tools/gen_layouts.py`, `crates/core/tools/README.md`
- Modify: `crates/core/src/lib.rs` (modules, re-exports)
- Test: `crates/core/tests/layout.rs`

**Interfaces:**
- Produces:
  - `pub struct Scripts(u8)` (`Clone`, `Copy`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Default`): consts `Scripts::LATIN`, `CYRILLIC`, `GEORGIAN`, `ARMENIAN`, `OTHER`; `Scripts::of(text: &str) -> Scripts` (letters only, a mixed text sets several bits); `fn contains(self, other: Scripts) -> bool` (any bit in common); `fn union(self, other: Scripts) -> Scripts`; `fn single(self) -> Option<Scripts>` (exactly one bit).
  - `pub fn letter_count(text: &str) -> usize` (alphabetic chars).
  - `pub struct Layout { pub name: &'static str, pub script: Scripts, pub to_us: &'static [(char, char)] }`, with `to_us` sorted by its first char.
  - `pub static LAYOUTS: &[Layout]`. Order: `ru`, `ua`, `by`, `kz`, `kg`, `tj`, `uz`, then the macOS and Windows variants that differ, then `ge` (and variants), then `am` (and variants).
  - `impl Layout { pub fn to_latin(&self, s: &str) -> Option<String>; pub fn from_latin(&self, s: &str) -> String }`. `to_latin` returns `None` when a letter of `s` (case folded) is not in the table. Non-letters not in the table pass through. Both keep the case of the table entries, which have both levels.

- [ ] **Step 1: Write the generator.** `gen_layouts.py <out.rs>` builds tables from two sources:
  - **xkb**: `xkbcli compile-keymap --layout L [--variant V]` for `us`, `ru`, `ua`, `by`, `kz`, `kg`, `tj`, `uz(cyrillic)`, `ru(mac)`, `ua(macOS)`, `ge`, `am`, through libxkbcommon `xkb_keysym_from_name` / `xkb_keysym_to_utf32`. Parse both key forms: `key <AD01> { [ a, A ] }` and the multi-line `symbols[1]= [ ... ]`.
  - **CLDR release-43**: `keyboards/osx/{ru,uk,be,kk,ka,hy}-t-k0-osx.xml` and `keyboards/windows/{ru,uk,be,kk,ky,tg,uz,ka,hy}-t-k0-windows.xml`, against `en-t-k0-osx.xml` and `en-t-k0-windows.xml`. Downloaded into a cache dir given by `--cldr-cache`.
  - Pair the base and Shift levels of each key with the same key of the US layout. Keep pairs where both characters are printable and differ.
  - Drop a variant whose table equals an already emitted one of the same script. Name each kept table after its source (`ru`, `ru-mac`, `ka-win`…).
  - Write each table's source and the standard in a comment.
  - `crates/core/tools/README.md` says how to rerun it. Tests never run it.
- [ ] **Step 2: Generate `tables.rs`** and check it in. Run: `python3 crates/core/tools/gen_layouts.py crates/core/src/layout/tables.rs --cldr-cache "$SCRATCH/cldr"`. Expected: about 9 to 16 tables, ru with ~75 pairs.
- [ ] **Step 3: Write the failing tests** in `crates/core/tests/layout.rs`:

```rust
fn layout(name: &str) -> &'static Layout { LAYOUTS.iter().find(|l| l.name == name).unwrap() }

#[test] fn ru_to_latin() { assert_eq!(layout("ru").to_latin("ашкуащч").as_deref(), Some("firefox")); }
#[test] fn ru_punctuation_keys() { assert_eq!(layout("ru").from_latin("j,kfrj"), "облако"); }
#[test] fn ru_shift_level() { assert_eq!(layout("ru").to_latin("АШКУАЩЧ").as_deref(), Some("FIREFOX")); }
#[test] fn national_letters_differ() {
    assert_eq!(layout("ua").from_latin("s"), "і");
    assert_eq!(layout("ru").from_latin("s"), "ы");
}
#[test] fn unknown_letter_is_none() { assert_eq!(layout("ru").to_latin("файлі"), None); }
#[test] fn every_script_has_a_table() {
    for s in [Scripts::CYRILLIC, Scripts::GEORGIAN, Scripts::ARMENIAN] {
        assert!(LAYOUTS.iter().any(|l| l.script == s));
    }
}
#[test] fn scripts_of_text() {
    assert_eq!(Scripts::of("Параметры NVIDIA"), Scripts::CYRILLIC.union(Scripts::LATIN));
    assert_eq!(Scripts::of("2+2"), Scripts::default());
    assert_eq!(Scripts::of("ფაილი").single(), Some(Scripts::GEORGIAN));
}
```

- [ ] **Step 4: Run** `cargo test -p tomakast-core --test layout`. Expected: it fails to compile (no `Layout`).
- [ ] **Step 5: Implement** `script.rs` and `layout/mod.rs`. Ranges: Latin `a-z A-Z U+00C0–U+024F`; Cyrillic `U+0400–U+04FF`; Georgian `U+10A0–U+10FF`, `U+1C90–U+1CBF`; Armenian `U+0530–U+058F`; any other alphabetic char is `OTHER`. Lookups use binary search on `to_us`, and `from_latin` uses an inverse built once (`OnceLock`).
- [ ] **Step 6: Run** `cargo test -p tomakast-core --test layout`. Expected: PASS.
- [ ] **Step 7: Commit** `feat(core): script detection and keyboard layout tables generated from xkb and CLDR`.

### Task 2: Romanization and the phonetic skeleton

**Files:**
- Create: `crates/core/src/translit.rs`
- Test: `crates/core/tests/translit.rs`

**Interfaces:**
- Produces: `pub fn romanize(s: &str) -> String` (lowercase, transliterated, not folded); `pub fn skeleton(s: &str) -> String`; `pub fn skeleton_alt(s: &str) -> Option<String>` (`Some` only when it differs from `skeleton(s)`).

- [ ] **Step 1: Write the failing tests.** The rules are in the spike note ("Setup", skeleton), and the tables cite GOST-like (Cyrillic), the Georgian national system (2002) and ISO 9985 (Armenian).

```rust
#[test] fn skeleton_examples() {
    for (a, b) in [("хром", "hrom"), ("chrome", "hrome"), ("калькулятор", "kalkulator"),
                   ("Calculator", "kalkulator"), ("ютуб", "iutub"), ("YouTube", "iutube"),
                   ("джира", "jira"), ("Jira", "jira"), ("гугл", "gugl"), ("Google", "gugle"),
                   ("опен", "open"), ("Файлы", "faili"), ("fayly", "faili"),
                   ("ფაილები", "pailebi"), ("Կարգավորումներ", "kargavorumner")] {
        assert_eq!(skeleton(a), b, "{a}");
    }
}
#[test] fn alternate_skeleton() {
    assert_eq!(skeleton_alt("Настройки").as_deref(), Some("nastrojki"));
    assert_eq!(skeleton_alt("ფაილები").as_deref(), Some("failebi"));
    assert_eq!(skeleton_alt("Firefox"), None);
}
#[test] fn romanize_is_readable() {
    assert_eq!(romanize("Хром"), "hrom");
    assert_eq!(romanize("щука"), "shchuka");
}
```

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test translit`. Expected: it fails to compile.
- [ ] **Step 3: Implement** the three functions. Order: lowercase → per-char table (national letters included, `ъ ь` → empty) → digraphs with `c` (`shch sch → sh`, `tch → ch`, `ch → h`, `ck → k`) → the `c` rule (before `e i y` → `s`, else `k`) → the rest of the folds → drop an `i` between a consonant and a vowel other than `i` → collapse doubled letters. `romanize` stops after the per-char table. The alternate differs only in `й → j` and `ფ → f`.
- [ ] **Step 4: Run** `cargo test -p tomakast-core --test translit`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): romanization and phonetic skeleton for Cyrillic, Georgian and Armenian`.

### Task 3: Query readings

**Files:**
- Create: `crates/core/src/readings.rs`
- Test: `crates/core/tests/readings.rs`

**Interfaces:**
- Consumes: `Scripts`, `letter_count`, `LAYOUTS` (Task 1), `romanize` (Task 2).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ReadingKind { Layout, Translit }`
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct QueryReading { pub kind: ReadingKind, pub text: String, pub script: Scripts }`. `script` is what the text is in. It is not sent over WIT.
  - `pub const MAX_READINGS: usize = 8; pub const MAX_READING_QUERY_CHARS: usize = 256;`
  - `pub fn query_readings(query: &str, index_scripts: Scripts) -> Vec<QueryReading>`
  - `pub fn normalize_layout(query: &str) -> String`. For a query in one non-Latin script, it is the first table in `LAYOUTS` order whose `to_latin` succeeds. Otherwise the query. Always lowercase.

- [ ] **Step 1: Write the failing tests:**

```rust
const ALL: Scripts = /* LATIN | CYRILLIC | GEORGIAN | ARMENIAN via union */;

#[test] fn non_latin_query_gets_latin_layout_and_translit() {
    let r = query_readings("ашкуащч", ALL);
    assert_eq!(r[0], QueryReading { kind: ReadingKind::Layout, text: "firefox".into(), script: Scripts::LATIN });
    assert!(r.iter().any(|x| x.kind == ReadingKind::Translit && x.text == "ashkuashchch"));
}
#[test] fn latin_query_gets_layouts_of_index_scripts_only() {
    let r = query_readings("afqks", Scripts::LATIN.union(Scripts::CYRILLIC));
    assert!(r.iter().any(|x| x.text == "файлы"));
    assert!(r.iter().all(|x| x.script == Scripts::CYRILLIC && x.kind == ReadingKind::Layout));
}
#[test] fn readings_are_distinct_and_capped() {
    let r = query_readings("sdfg", ALL);
    assert!(r.len() <= MAX_READINGS);
    let mut t: Vec<_> = r.iter().map(|x| &x.text).collect(); t.dedup();
    assert_eq!(t.len(), r.len());
    assert!(r.iter().all(|x| x.text != "sdfg"));
}
#[test] fn readings_ignore_case() { assert_eq!(query_readings("АШКУАЩЧ", ALL)[0].text.to_lowercase(), "firefox"); }
#[test] fn one_letter_mixed_or_other_has_none() {
    assert!(query_readings("ф", ALL).is_empty());
    assert!(query_readings("фa", ALL).is_empty());   // Cyrillic + Latin
    assert!(query_readings("αβγ", ALL).is_empty());  // Greek: other
}
#[test] fn no_readings_without_letters() { assert!(query_readings("2+2*3", ALL).is_empty()); assert!(query_readings("15%", ALL).is_empty()); }
#[test] fn long_query_has_no_readings() { assert!(query_readings(&"ф".repeat(300), ALL).is_empty()); }
#[test] fn normalize_layout_maps_wrong_layout() {
    assert_eq!(normalize_layout("Ашкуащч"), "firefox");
    assert_eq!(normalize_layout("Fire"), "fire");
}
```

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test readings`. Expected: it fails to compile.
- [ ] **Step 3: Implement** `query_readings` as the spec's "Readings of a query":
  - A non-Latin query gets layout remaps to Latin through every table of its script where `to_latin` succeeds, then the `romanize` reading (translit, script Latin).
  - A Latin query gets `from_latin` through every table whose script is in `index_scripts`, with no translit.
  - Deduplicate, drop readings equal to the query, cap at `MAX_READINGS`.
- [ ] **Step 4: Run** `cargo test -p tomakast-core --test readings`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): query readings across layouts and scripts`.

### Task 4: Cross-script matcher

**Files:**
- Modify: `crates/core/src/matcher.rs` (whole file), `crates/core/src/lib.rs`
- Test: `crates/core/tests/matcher.rs` (existing tests adapted, new ones added)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `pub struct TextKeys { scripts: Scripts, skeleton: Utf32String, alt: Option<Utf32String> }` with `TextKeys::new(text: &str) -> TextKeys` and `fn scripts(&self) -> Scripts`.
  - `pub struct ItemKeys { pub title: TextKeys, pub keywords: Vec<TextKeys> }` with `ItemKeys::new(item: &Item) -> ItemKeys` and `fn scripts(&self) -> Scripts` (the union).
  - `Pattern::parse(query: &str) -> Pattern`: as typed only, the same as today. It is kept for `launcher_view`'s emptiness check and for callers without keys.
  - `Pattern::across(query: &str, scripts: Scripts, m: &mut Matcher) -> Pattern`: as typed plus the readings for `scripts`. Layout readings become patterns over the original text of texts whose scripts contain the reading's script. The query's `skeleton` (not the translit reading) becomes a pattern over the skeletons of texts in any script other than the query's. Each cross pattern stores its floor, `(self_score as f64 * CROSS_MIN_RATIO) as u32`.
  - `Pattern::readings(&self) -> &[QueryReading]` and `Pattern::is_empty(&self) -> bool` (unchanged).
  - `pub const LAYOUT_FACTOR: (u32, u32) = (7, 8); pub const TRANSLIT_FACTOR: (u32, u32) = (3, 4); pub const CROSS_MIN_RATIO: f64 = 0.8;`
  - `Matcher::score(&mut self, pattern: &Pattern, item: &Item, keys: &ItemKeys) -> Option<u32>`
  - `Matcher::score_text(&mut self, pattern: &Pattern, text: &str, keys: &TextKeys) -> Option<u32>`

- [ ] **Step 1: Adapt the existing tests** to the new signatures (`ItemKeys::new(&it)` and `TextKeys::new(text)` next to each call). Add:

```rust
#[test] fn layout_reading_matches_with_penalty() {
    let mut m = Matcher::new();
    let ff = item("Firefox", &[]);
    let typed = m.score(&Pattern::parse("firefox"), &ff, &ItemKeys::new(&ff)).unwrap();
    let p = Pattern::across("ашкуащч", Scripts::LATIN, &mut m);
    assert_eq!(m.score(&p, &ff, &ItemKeys::new(&ff)), Some(typed * 7 / 8));
}
#[test] fn skeleton_matches_across_scripts_only() {
    let mut m = Matcher::new();
    let chrome = item("Google Chrome", &[]);
    let p = Pattern::across("хром", Scripts::LATIN, &mut m);
    assert!(m.score(&p, &chrome, &ItemKeys::new(&chrome)).is_some());
    let calc = item("calc", &[]);
    let p = Pattern::across("kalk", Scripts::LATIN, &mut m);  // Latin vs Latin: no skeleton
    assert!(m.score(&p, &calc, &ItemKeys::new(&calc)).is_none());
}
#[test] fn as_typed_wins_at_equal_quality() {
    let mut m = Matcher::new();
    let files = item("Файлы", &[]);
    let p = Pattern::across("фай", Scripts::CYRILLIC.union(Scripts::LATIN), &mut m);
    let typed = m.score(&Pattern::parse("фай"), &files, &ItemKeys::new(&files)).unwrap();
    assert_eq!(m.score(&p, &files, &ItemKeys::new(&files)), Some(typed));
}
#[test] fn scattered_cross_script_hit_is_dropped() {
    let mut m = Matcher::new();
    let both = Scripts::CYRILLIC.union(Scripts::LATIN);
    let scattered = item("Азбука сосна", &[]);   // skeleton "azbuka sosna": a..b..s
    let contiguous = item("Абсент", &[]);        // skeleton "absent"
    let p = Pattern::across("abs", both, &mut m);
    assert!(m.score(&p, &scattered, &ItemKeys::new(&scattered)).is_none());
    assert!(m.score(&p, &contiguous, &ItemKeys::new(&contiguous)).is_some());
}
#[test] fn blank_query_has_no_readings() {
    let mut m = Matcher::new();
    let p = Pattern::across("   ", Scripts::LATIN.union(Scripts::CYRILLIC), &mut m);
    assert!(p.is_empty() && p.readings().is_empty());
}
#[test] fn score_text_2000_rows_under_5ms() { /* 2000 TextKeys built once; median of 20 runs of
   score_text over all rows for "ашкуащч" prefixes of ≥ 2 letters < 5 ms (release or debug as
   the existing timing test does) */ }
```

- [ ] **Step 2: Run** `cargo test -p tomakast-core --test matcher`. Expected: it fails to compile.
- [ ] **Step 3: Implement.** `score` keeps today's rule (max of title, keyword × 3/4) on top of the per-text rule `max(as typed, layout × 7/8, skeleton or alt × 3/4)`. A cross score below its floor counts as no match.
- [ ] **Step 4: Run** `cargo test -p tomakast-core --test matcher`. Expected: PASS. If `scattered_cross_script_hit_is_dropped` fails at 0.8, report it. Do not change the ratio.
- [ ] **Step 5: Commit** `feat(core): matcher scores layout and skeleton readings across scripts`.

### Task 5: Index keys, engine, acceptance corpus and benchmark

**Files:**
- Modify: `crates/core/src/index.rs` (`Index`), `crates/core/src/engine.rs:79-135`
- Create: `crates/core/tests/across.rs`, `crates/core/tests/corpus/mod.rs`
- Modify: `crates/core/tests/engine.rs` (new benchmark next to `search_10k_under_5ms`)

**Interfaces:**
- Consumes: Task 4.
- Produces:
  - `Index` stores `(Arc<Item>, ItemKeys)` per id. `iter()` is unchanged (`&Arc<Item>`).
  - `pub fn iter_keyed(&self) -> impl Iterator<Item = (&Arc<Item>, &ItemKeys)>`
  - `pub fn scripts(&self) -> Scripts`: the union over all items, updated in `replace_provider`.
  - `Engine::search_with` builds `Pattern::across(query, self.index.scripts(), &mut self.matcher)` and scores with keys.

- [ ] **Step 1: Write the corpus** in `corpus/mod.rs` as `pub fn corpus() -> Vec<Item>`, all names public. It contains:
  - every name in the spike note's "Added names" list;
  - KCalc, Chromium, Konsole, Yandex Disk, Filelight, VRML to X3D;
  - about 100 more common desktop apps in English;
  - about 30 public Russian KDE/GNOME names, «Калькулятор», «Параметры системы», «Диспетчер разделов от KDE», «Системный монитор», «Информация о системе» and «Редактор меню» among them.
- [ ] **Step 2: Write the failing tests** in `across.rs`:
  - `pairs_levels_0_to_3`: table-driven over every level 0–3 pair in the spike note's "Full pair list". It asserts the expected title is in the top 3 of `engine.search(q, now)`. For `калк`, the first hit is «Калькулятор». For `хром`, the first hit is «Chromium». For `yandex`, the first hit is «Yandex Disk». Level 4 is not tested.
  - `multi_word_layout_query`: `мшыгфд ыегвшщ` → «Visual Studio Code» first.
  - `latin_only_corpus_is_unchanged`: over the corpus's Latin-only items and every 1–6 letter prefix of every Latin title word, the hit order equals a reference ranking by `Pattern::parse`. The reference is computed in the test with the same `combined_score` and tie-break.
  - `no_top1_change_for_latin_prefixes`: the same queries over the full corpus, top 1 compared with the reference.
- [ ] **Step 3: Write the benchmark** `search_across_10k_under_5ms` in `crates/core/tests/engine.rs`, in the shape of `search_10k_under_5ms`:
  - 10k items, each a title plus two keywords, every fifth item Cyrillic;
  - queries `firefox`, `ашкуащч`, `калькулятор`, `fajly`, every prefix of two or more letters;
  - assert the median of 20 runs is < 5 ms; print each median.
- [ ] **Step 4: Run** `cargo test -p tomakast-core --test across --test engine`. Expected: it fails to compile.
- [ ] **Step 5: Implement** the `Index` and `Engine` changes.
- [ ] **Step 6: Run** `cargo test -p tomakast-core --test across --test engine -- --nocapture`, and `cargo test --release -p tomakast-core --test engine search_across -- --nocapture`. Expected: PASS. Note the printed medians.
  - If a pair fails, tune the folding rules or table entries, not the constants, and record each change in the spike note.
  - If the benchmark fails, stop and report the medians.
- [ ] **Step 7: Commit** `feat(core): index keeps match keys; search finds items across layouts and scripts`.

### Task 6: Usage history modulo layout

**Files:**
- Modify: `crates/core/src/usage.rs:103-140`
- Test: `crates/core/tests/usage.rs`

**Interfaces:**
- Consumes: `normalize_layout` (Task 3).
- Produces: `UsageModel` keeps a private `keys: Vec<String>` parallel to `records`, holding `normalize_layout(&r.query)`, filled in `new` and `push`. `stats` compares `keys[i].starts_with(&normalize_layout(query))`. The stored `UsageRecord` is unchanged.

- [ ] **Step 1: Write the failing test** `query_hits_ignore_layout`:
  - a record with query `ашкуащч` for item A;
  - `stats("fire", …)[A].query_hits == 1`, and `stats("ашку", …)[A].query_hits == 1`;
  - a record `firefox` counts for `ашк`.
- [ ] **Step 2: Run** `cargo test -p tomakast-core --test usage`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p tomakast-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): usage history matches queries modulo keyboard layout`.

### Task 7: Readings reach native modules and module-filtered lists

**Files:**
- Modify: `crates/core/src/module.rs:160-168`, `crates/core/src/host.rs` (`Command::Search`, `search`, the worker), `crates/core/src/core.rs:283-290`, `crates/core/src/view.rs:172-175`, `crates/ui/src/launcher_view.rs:525-545`
- Test: `crates/core/tests/host.rs`, `crates/ui/tests/stack.rs`, plus constructors of `ViewEvent::QueryChanged` in `crates/plugin-host/tests/*` and `crates/core/tests/host.rs`

**Interfaces:**
- Consumes: `QueryReading`, `query_readings` (Task 3), `Index::scripts` (Task 5).
- Produces:
  - `Module::search_with(&mut self, query: &str, readings: &[QueryReading]) -> anyhow::Result<Vec<Item>>`, which defaults to `self.search(query)`.
  - `Command::Search { generation, query, readings: Vec<QueryReading> }`. The worker calls `search_with`.
  - `Core::query_readings(&self, query: &str) -> Vec<QueryReading>` (uses `engine.index().scripts()`). `Core::search` computes the readings once and passes them to `host.search(generation, query, readings)`.
  - `ViewEvent::QueryChanged { revision: u64, text: String, readings: Vec<QueryReading> }`. `list_query_changed` fills it from `core.borrow().query_readings(&text)`.

- [ ] **Step 1: Write the failing test** `dynamic_source_receives_readings` in `crates/core/tests/host.rs`. A testing module overrides `search_with` and records its arguments. After `core.search("ашкуащч", now)`, the recorded readings contain `firefox` (Layout). Extend the stack test in `crates/ui/tests/stack.rs` that checks `QueryChanged` so it asserts `readings` is filled for `ашкуащч`.
- [ ] **Step 2: Run** `cargo test -p tomakast-core --test host`. Expected: it fails to compile.
- [ ] **Step 3: Implement.** Add `readings: vec![]` to the existing constructors of `QueryChanged` in tests. Pattern matches `{ .. }` and by field stay as they are.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core, ui): dynamic sources and module-filtered lists receive query readings`.

### Task 8: WIT 0.1 in place, plugin host and SDK

**Files:**
- Modify: `crates/plugin-api/wit/since_v0.1.0/plugin.wit` (`record query-changed` near line 333, the `search` export near line 620, and a new `enum query-reading-kind` and `record query-reading` in `interface types`), `FROZEN`, `FROZEN.md`
- Modify: `crates/core/tests/wit_parity.rs` (`TYPES`, `EXPORTS`)
- Modify: `crates/plugin-host/src/plugin.rs:653-656` (implement `search_with`; `search` delegates with `&[]`), the generated `call_search` call, `crates/plugin-host/src/convert.rs:302-303`
- Modify: `crates/plugin-sdk/src/lib.rs:95-125` (trait, `Guest` glue, re-exports), `crates/plugin-sdk/README.md`, `docs/plugins/writing-a-plugin.md`
- Test: `crates/plugin-host/tests/plugin.rs`, `crates/plugin-host/tests/wit_coverage.rs`

**Interfaces:**
- Consumes: Task 7.
- Produces:
  - WIT, verbatim from the spec's "Dynamic sources" block: `enum query-reading-kind { layout, translit }`, `record query-reading { kind: query-reading-kind, text: string }`, `export search: func(query: string, readings: list<query-reading>) -> result<list<item>, string>`, plus `readings: list<query-reading>` as the last field of `record query-changed`, with doc comments ("at most 8, never equal to `query` or to each other").
  - SDK: `fn search_with(query: String, readings: Vec<QueryReading>) -> Result<Vec<Item>, String>`, which defaults to `Self::search(query)`. `Guest::search(query, readings)` calls `T::search_with`. `QueryReading` and `QueryReadingKind` are re-exported.

- [ ] **Step 1: Update `wit_parity.rs`** (`("query-reading", Record(&["kind", "text"]))`, `("query-reading-kind", Enum(&["layout", "translit"]))`, `query-changed` with `readings`, and the new `search` signature). Write `search_gets_readings` in `crates/plugin-host/tests/plugin.rs`:
  - a test guest built like the existing ones echoes `readings.len()` and the first text as an item title;
  - `search_with("ашкуащч", &readings)` returns a title containing `firefox`.
- [ ] **Step 2: Run** `cargo test -p tomakast-core --test wit_parity` and `cargo test -p tomakast-plugin-host --test plugin`. Expected: FAIL.
- [ ] **Step 3: Edit `plugin.wit`, then re-hash.** Run `cd crates/plugin-api/wit/since_v0.1.0 && sha256sum plugin.wit > FROZEN`. Add the `FROZEN.md` row: "`search` takes `readings`, and `query-changed` carries them | The user's ruling (2026-11-19, search-layouts feedback): WIT 0.1 is edited in place; readings let plugins match a query typed on the wrong layout or in another script."
- [ ] **Step 4: Implement** the host conversion (core `QueryReading` → WIT, dropping `script`), `WasmPlugin::search_with`, the SDK and the docs. The docs give a short example of `search_with` and say that `search` keeps working.
- [ ] **Step 5: Run.**
  - `cargo test --workspace`;
  - the CI guest steps (`cargo clippy --manifest-path crates/plugin-sdk/Cargo.toml --all-targets --target wasm32-wasip2 -- -D warnings`, `cargo test --manifest-path crates/plugin-sdk/Cargo.toml`);
  - kitchen-sink clippy;
  - `cargo test --release --manifest-path plugins/calc/Cargo.toml`.

  Expected: PASS, with no edit to `plugins/calc` or `plugins/kitchen-sink` sources.
- [ ] **Step 6: Commit** `feat(plugin-api, plugin-host, sdk): search and query-changed carry query readings (WIT 0.1 in place)`.

### Task 9: List views and the action panel

**Files:**
- Modify: `crates/ui/src/views/list.rs:55-110`, `crates/ui/src/views/action_panel.rs:85-110`, `crates/ui/src/launcher_view.rs:429-433`
- Test: `crates/ui/tests/stack.rs` (or the existing list-filter test file)

**Interfaces:**
- Consumes: `TextKeys`, `Pattern::across`, `Matcher::score_text` (Task 4).
- Produces:
  - `ListState` holds `row_keys: Vec<TextKeys>` and `row_scripts: Scripts`, rebuilt by `pub(crate) fn reindex(&mut self, view: &ListView)`. It is called in `ListState::new` and in the update path before `refilter` (`launcher_view.rs:431`).
  - `refilter` uses `Pattern::across(query, self.row_scripts, &mut self.matcher)`.
  - `ActionPanel` builds its keys once in its constructor.

- [ ] **Step 1: Write the failing tests:**
  - `core_filter_list_matches_wrong_layout`: rows "Firefox" and "Files", query `ашкуащч`, so only Firefox is visible;
  - `action_panel_matches_wrong_layout`: action "Copy", query `сщзн`, visible.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): list filter and action panel match across layouts and scripts`.

### Task 10: Docs, measurements, build for the user

**Files:**
- Modify: `docs/superpowers/specs/2026-11-18-tomakast-core-design.md` (the "Search and ranking" matching bullet, and the `search(query)` line near 157), `docs/superpowers/plans/2026-11-18-roadmap.md` (status line: WIT 0.1 `search` changed in place), `docs/experiments/2026-11-search-layouts-spike.md` (add the "Implementation measurements" section)

- [ ] **Step 1: Re-measure the spike's stress corpus**: 10k items, five texts each, from `crates/core/tests/corpus` repeated. Use a throwaway test or an ignored test (`#[ignore]`, named `stress_across_scripts`), run with `cargo test --release -p tomakast-core -- --ignored --nocapture stress_across`. Record the medians and the Task 5 benchmark medians in the spike note.
- [ ] **Step 2: Update the core spec and the roadmap.** Cover the matching across layouts and scripts, the new `search` signature and `query-changed.readings`.
- [ ] **Step 3: Rebuild for the user:** `bash .superpowers/feedback/build.sh /home/tomakado/dev/github.com/tomakast/tomakast/.worktrees/fb-search`. Expected: build OK.
- [ ] **Step 4: Headless smoke check** with its own `TOMAKAST_HOME` in the scratchpad and `TOMAKAST_NO_KEYBOARD=1`: the daemon starts, calc loads (log line), and nothing panics. Kill only that process.
- [ ] **Step 5: Commit** `docs: search across layouts in the core spec, roadmap and measurements`.
- [ ] **Step 6: Hand to the user** with `bash .superpowers/feedback/try.sh` and the manual checks: `ашкуащч`, `хром`, `опен`, `afqks`, and `калк` (calc's answer is untouched for `2+2`).
