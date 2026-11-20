# Spike: search across keyboard layouts and scripts (2026-11-19)

Throwaway check of approach A in `docs/superpowers/specs/2026-11-19-search-layouts-design.md`: before that design promised "хром" → Chrome, we confirmed that layout remaps and a phonetic skeleton on top of nucleo-matcher 0.3 find the right items, leave ordinary Latin queries alone, and fit the per-keystroke budget. The code lived in the session scratchpad and is not kept; this note records what it did and what it found.

## Setup

- **Layout tables** were generated, not typed. `xkbcli compile-keymap --layout <l>` was run for `us ru ua by kz kg tj ge am` and `uz --variant cyrillic`. Each `key <XXXX> { [ level1, level2 ] }` (or `symbols[1]= [...]`) was paired with the same key in `us`, and keysyms were turned into characters through libxkbcommon (`xkb_keysym_from_name`, `xkb_keysym_to_utf32`). That gives 74 to 93 pairs per layout, punctuation keys included (ru: `б ю ж э х ъ ё` ↔ `, . ; ' [ ] \``, and the shifted ones).
- **Cyrillic layouts agree** on every shared letter. They differ only on national letters: `і` (ua `s`, by `b`, kz `3`), `ғ қ ң ү ҳ` (kz, kg, tj, uz differ). So a query is remapped once per layout that has all its letters, and the results are deduplicated. Usually that gives one string, and two or three at most.
- **Skeleton**: lowercase, then transliterate (Cyrillic GOST-like; Georgian national system 2002; Armenian ISO 9985-like, apostrophes dropped), then fold Latin spellings: `shch sch → sh`, `ch → h`, `ck → k`, `c` before `e i y` → `s`, other `c` → `k`, `dzh dj zh → j`, `kh → h`, `gh → g`, `ph → f`, `th → t`, `ts → s`, `oo ou → u`, `ee → i`, `x → ks`, `q → k`, `w → v`, `y → i`. Then drop an `i` between a consonant and a vowel (`ля` → `la`), and collapse doubled letters. Examples: `хром` → `hrom`, `chrome` → `hrome`, `калькулятор` → `kalkulator`, `Calculator` → `kalkulator`, `ютуб` → `iutub`, `YouTube` → `iutube`.
- **Alternate skeleton** per text, only when it differs: `й → j` (so `nastrojki` finds «Настройки») and Georgian `ფ → f` (loanwords).
- **Scoring** per text: `max(as typed, layout × 7/8, skeleton × 3/4)`, times 3/4 for keywords, as today. Layout and skeleton patterns are tried only against texts in another script than the query. A cross-script hit must reach `RATIO` × the score the pattern gets against itself.
- **Corpus**: the real `apps` scan of the dev box (126 apps), once with no locale and once with `ru_RU, ru`. Fifty well-known names were added for the pair test (Telegram, Jira, YouTube…), plus Cyrillic («Файлы», «Настройки», «Калькулятор», «Облако», «Почта», «Яндекс Музыка»), Georgian (ფაილები, პარამეტრები) and Armenian (Կարգավորումներ, Ֆայլեր) items.

## Results

**Pairs** (query → expected item in the top 3). Levels: 0 = layout, 1 = letter-for-letter translit, 2 = `c/k`, `x`, 3 = digraphs and dropped letters, 4 = spelled by sound.

| Level | Passed | Examples |
|---|---|---|
| 0 | 11/11 | `ашкуащч` → Firefox, `еудупкфь` → Telegram, `afqks` → Файлы, `j,kfrj` → Облако, `фай` → Файлы still first |
| 1 | 28/28 | `опен` → Open, `телеграм`, `дискорд`, `обсидиан`, `fajly`/`faily`/`fayly` → Файлы, `nastrojki` → Настройки, `failebi` → ფაილები, `kargavorumner` → Կարգավորումներ |
| 2 | 7/7 | `калк` → Calculator (2nd, after «Калькулятор» as typed), `вскод` → Visual Studio Code, `фотошоп`, `спектакл` |
| 3 | 7/7 | `хром` → Chrome (2nd after Chromium), `джира` → Jira, `ютуб` → YouTube, `гугл` → Google, `зум` → Zoom, `либреофис` |
| 4 | 0/8 | `фаерфокс`, `стим`, `скайп`, `спотифай`, `тандерберд`, `кликхаус`, `ноушн`, `кейт` miss, as intended |

The first run missed `fajly` and `nastrojki`, because Latin `j` stands for `й` in Russian romanization but for `дж` in English names. The `й → j` alternate skeleton fixed both without breaking `джира` → Jira.

**Regression guard**: every prefix of 1 to 6 letters of every Latin word of every app title, as the query. The run compared the feature off with the feature on.

| Index | Queries | Top-1 changed | Baseline top 5 displaced | Extra hits (all queries) |
|---|---|---|---|---|
| no locale (all Latin) | 685 | 0 | 0 | 0 |
| ru locale, RATIO 0 | 619 | 1 | 108 | 3801 |
| ru locale, RATIO 0.7 | 619 | 1 | 80 | 848 |
| ru locale, RATIO 0.8 | 619 | 1 | 55 | 506 |
| ru locale, RATIO 0.9 | 619 | 1 | 33 | 361 (but `вскод` misses) |

The one top-1 change was the single-letter query `x` («Параметры NVIDIA X Server» over «VRML to X3D»). The design therefore skips cross-script readings for queries with fewer than two letters. At 0.8 the displaced entries are tail fuzzy hits in places 2 to 5 (`disk` → «Диспетчер разделов от KDE» after Yandex Disk and Filelight), never a strong match pushed down. **0.8 was kept.**

**Time**: 10,000 items built from the ru-locale scan, with real keywords: 50,652 texts. Times are ms per keystroke, the average of 20 runs per prefix, on a Ryzen 7 7800X3D, single thread. Each value is the whole rank: build the query, score, sort.

| Query typed | Off | On (RATIO 0.8) | Per-prefix (on) |
|---|---|---|---|
| `firefox` | avg 1.66, max 3.07 | avg 3.42, max 4.93 | 4.9 3.8 3.0 2.7 2.5 3.6 3.4 |
| `ашкуащч` | — | avg 3.30, max 8.23 | 8.2 2.8 2.7 2.5 2.5 2.2 2.1 |
| `калькулятор` | — | avg 3.25, max 8.21 | 8.2 5.7 3.3 3.1 2.6 … 1.9 |
| `fajly` | — | avg 3.20, max 4.89 | 4.9 3.4 2.8 2.5 2.4 |
| `j,kfrj` | — | avg 2.62, max 3.09 | 3.1 2.7 2.6 2.6 2.4 2.2 |

The 8 ms peaks are one-letter Cyrillic queries, which the two-letter rule removes. The worst remaining case is the two-letter `ка` at 5.7 ms, just over the spec's 5 ms for 10k items. But this corpus has five texts per item, and the real index has 126 apps. Precomputing skeletons for 10k items took 120 ms. That happens once, at index time, not at open.

## Implementation measurements (2026-11-20)

Measured on the shipped implementation (`crates/core`), release build, Ryzen 7 7800X3D, single thread, machine idle (load average about 2; a first run under load 20 gave 2 to 5 times worse numbers and was discarded). Medians of 20 runs per query. Every run has 1,000 launch records seeded, so usage scoring is in the timing.

Commands:

- `cargo test --release -p tomakast-core --test engine -- --ignored --nocapture --test-threads=1` (the two benchmarks; they must not run in parallel)
- `cargo test --release -p tomakast-core --test across -- --ignored --nocapture stress_across` (the stress corpus)

**`search_10k_under_5ms`** (10k Latin items, no cross-script work): `f` 1.74 ms, `fi` 2.54 ms, `fir` 3.51 ms, `fire` 1.99 ms.

**`search_across_10k_under_5ms`** (10k items, Latin and Cyrillic, two keywords each; prefixes of 2+ letters): all medians between 2.96 and 3.86 ms. `firefox` 3.0 to 3.8, `ашкуащч` 3.1 to 3.8, `калькулятор` 3.0 to 3.9, `fajly` 3.0 to 3.6.

**`stress_across_scripts`** (the spike's stress corpus: 10k items, five texts each, built by repeating `crates/core/tests/corpus`; per-prefix medians, ms):

| Query typed | Per-prefix medians |
|---|---|
| `firefox` | 5.56 5.28 5.13 4.96 5.90 5.68 |
| `ашкуащч` | 5.45 5.31 5.19 5.07 4.83 4.76 |
| `калькулятор` | 6.67 5.75 5.71 5.39 5.13 4.95 4.80 4.70 4.56 4.43 |
| `fajly` | 5.59 5.24 5.07 4.94 |
| `j,kfrj` | 4.05 5.13 5.08 4.91 4.81 |

Usage scoring toggled on the same corpus (`STRESS_NO_USAGE=1` skips the 1,000 launch records; machine idle, load average about 1, `--test-threads=1`; medians per prefix, ms):

| Query typed | With 1,000 usage records | Without usage records |
|---|---|---|
| `firefox` | 5.54 5.27 5.09 4.92 5.87 5.67 | 5.22 4.95 4.76 4.56 5.60 5.33 |
| `ашкуащч` | 5.43 5.26 5.13 4.99 4.77 4.69 | 5.07 4.96 4.80 4.70 4.48 4.40 |
| `калькулятор` | 6.47 5.66 5.61 5.33 5.10 4.96 4.78 4.69 4.54 4.44 | 6.17 5.33 5.29 5.02 4.78 4.64 4.46 4.38 4.25 4.09 |
| `fajly` | 5.64 5.22 5.06 4.92 | 5.24 4.90 4.74 4.59 |
| `j,kfrj` | 4.02 5.10 5.07 4.91 4.80 | 3.72 4.78 4.74 4.59 4.50 |

Usage scoring costs about 0.3 to 0.4 ms, so it is not the cause: without it the stress corpus still measures 3.7 to 6.2 ms, mostly above 4.4. The cost is the matching itself, five texts per item over 10k items (50k texts per keystroke, each scored for the query and its readings). It is over the 5 ms budget by up to about 1.5 ms on the first prefixes. This is accepted, not optimized: the budget is the engine benchmark's shape (10k items, title plus two keywords), which stays under 4 ms. The real index has about 126 apps. The stress figure is the margin to watch if the index grows.

## Full pair list

Each line is query → expected item, by level. Level 4 is listed for reference; it is not a target.

- **0 (layout):** `ашкуащч` → Firefox; `еудупкфь` → Telegram; `ыдфсл` → Slack; `сфдс` → Calculator; `лщтыщду` → Konsole; `afqks` → Файлы; `yfcnhjqrb` → Настройки; `j,kfrj` → Облако; `gjxnf` → Почта; as typed: `фай` → Файлы, `настр` → Настройки.
- **1 (letter for letter):** `опен` → Open; `телеграм` → Telegram; `дискорд` → Discord; `слак` → Slack; `терминал` → Terminal; `фирефокс` → Firefox; `гимп` → GIMP; `блендер` → Blender; `крита` → Krita; `постман` → Postman; `обсидиан` → Obsidian; `докер` → Docker Desktop; `дольфин` → Dolphin; `консоле` → Konsole; `fajly`, `faily`, `fayly` → Файлы; `nastrojki`, `nastroiki`, `nastroyki` → Настройки; `kalkulyator` → Калькулятор; `dokumenty` → Документы; `pochta` → Почта; `failebi` → ფაილები; `parametrebi` → პარამეტრები; `kargavorumner` → Կարգավորումներ; `fayler` → Ֆայլեր; `файлеби` → ფაილები.
- **2 (c/k, x):** `калк`, `калькулятор` → Calculator; `вскод` → Visual Studio Code; `спектакл` → Spectacle; `долфин` → Dolphin; `фотошоп` → Photoshop; `yandex` → Яндекс Музыка.
- **3 (digraphs, dropped letters):** `хром` → Google Chrome; `хромиум` → Chromium; `джира` → Jira; `ютуб` → YouTube; `гугл` → Google; `зум` → Zoom; `либреофис` → LibreOffice Writer.
- **4 (by sound, not a target):** `ноушн` → Notion; `тандерберд` → Thunderbird; `кликхаус` → ClickHouse; `стим` → Steam; `скайп` → Skype; `спотифай` → Spotify; `фаерфокс` → Firefox; `кейт` → Kate.

Added names in the corpus, besides the dev box's apps: Telegram, Discord, Slack, Zoom, Terminal, Firefox, GIMP, Blender, Krita, Inkscape, Calculator, Visual Studio Code, Steam, Google Chrome, Chromium, Jira, YouTube, Google, Photoshop, Dolphin, Konsole, Kate, Spectacle, OBS Studio, VLC media player, Postman, Docker Desktop, ClickHouse, Skype, Open, Spotify, Notion, Obsidian, Thunderbird, LibreOffice Writer, Settings, System Monitor, Файлы, Настройки, Калькулятор, Документы, Облако, Почта, Яндекс Музыка, ფაილები, პარამეტრები, Կարգավորումներ, Ֆայլեր. The dev box's own apps included KCalc, Chromium and the ru-locale KDE names («Параметры системы», «Диспетчер разделов от KDE»…).

## Open points carried into the design

- Armenian and Georgian have several layouts (xkb `am`: basic, phonetic, eastern, western; `ge`: basic, qwerty, ergonomic). The spike used the xkb defaults only. The macOS and Windows defaults for both still need checking.
- macOS «Russian» (not «Russian – PC») moves some punctuation and `ё`. Its table needs its own source.
