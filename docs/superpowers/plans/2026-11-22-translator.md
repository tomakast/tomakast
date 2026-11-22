# Translator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `translator` WASM plugin with Raycast's UX: a "Translate" command, a compose pad
that translates texts live, a dictionary for short input, and a root-search tile for
`<text> in <language>`. Behind it are three small launcher changes: the system's
languages, a compose pad, and Ctrl+Enter in a pad.

**Architecture:** The plugin (`plugins/translator`, its own crate like calc and weather)
keeps the provider (`google.rs`), the pure logic (`pair.rs`, `phrase.rs`, `render.rs`,
`pad.rs`, `picker.rs`, `recent.rs`, `cache.rs`) and the glue (`lib.rs`) apart. The pure
logic is tested natively against recorded Google answers. The launcher gains:
- `locale.languages()`;
- `scratchpad-view.output`, a markdown pane under a wrapping editor;
- Ctrl/Cmd+Enter in a pad, which runs the pad's first action after the pad's waiting edit
  is sent.

**Tech Stack:** Rust 2024, WIT 0.1 (wasmtime host), the guest SDK `tomakast-plugin`,
GPUI via gpui-kit 0.10, `sys-locale` 0.3.2.

**Spec:** `docs/superpowers/specs/2026-11-22-translator-design.md`. Read it first: this
plan argues from it and does not repeat its rules.

## Global Constraints

- The plugin id is `translator`, its name "Translator", its command `translate` titled
  "Translate" with icon `languages`. Its widget is `translation`, titled "Translation", size
  `double`, with params `text`, `from` and `to`.
- Permissions: `search`, `storage`, `clipboard`, `timers`,
  `http = ["translate.googleapis.com"]`. Nothing else (no `open`).
- Preference: `languages`, kind `text`, no default, titled "Your languages, first then
  second (codes or English names, such as ru, en); empty means the system's".
- Rest times: pad 400 ms, search 300 ms. Short input: ≤ 3 words, one line, ≤ 64 chars.
  Input cap: 30 000 chars. `output` cap: 256 KiB. Phrase text: 1–500 chars, no line
  break. Recent: 20 entries, each text ≤ 8 KiB, value < 200 KiB. Cache: 64 answers.
- The section caps of the pane are 8 translations per POS, 3 meanings per POS, 5 examples
  and 3 synonym groups per POS.
- Every WIT edit adds a `FROZEN.md` row, re-hashes `FROZEN` (`sha256sum plugin.wit > FROZEN`
  in the folder), and is recorded "edited in place (memory 'WIT freeze lifted')".
- No toasts for network failures. The only toast the plugin sends is "Still translating".
- Run the app only through `just` recipes. Your own runs use `just try approve nokeys`
  (Task 4), never a grabbed keyboard.
- Commit after every task. No attribution lines. Never commit screenshots.

## Review Focus

1. **A paste then Ctrl+Enter at once** must copy the pasted text's translation or say
   "Still translating", never the previous text's. Task 3 sends the waiting edit first,
   and Task 8 compares texts.
2. **Text that is markdown or HTML** (`# Title`, `*x*`, `<b>`, `[a](b)`, a leading `1.`)
   must show as typed in the pane. Task 7 adds a round-trip test.
3. **A word with no dictionary data** (a name, `qwzx`, a sentence) shows the translation
   alone, with no empty sections. Task 7 adds a test.
4. **The pair when the system gives nothing** (`languages()` empty, preference empty): the
   pair is (en, —) and English text asks for a target. Task 6 adds a test.
5. **An answer for an older text** arriving after a newer edit must not replace the pane.
   Task 8 adds a test.

---

### Task 1: `locale.languages()` in WIT, host and SDK

**Files:**
- Modify: `crates/plugin-api/wit/since_v0.1.0/plugin.wit` (interface `locale`), `FROZEN`,
  `FROZEN.md`
- Modify: `crates/plugin-host/src/imports/locale.rs`, `crates/plugin-host/src/imports/mod.rs`
  (`HostState` field `languages`, setter), `crates/plugin-host/src/plugin.rs`
  (`with_languages`), `crates/plugin-host/Cargo.toml` (`sys-locale = "0.3.2"`)
- Modify: `crates/plugin-sdk/src/imports.rs` (`locale::languages()`), `crates/plugin-sdk/README.md`
- Modify: `docs/plugins/writing-a-plugin.md` (the imports list names `locale` as the time
  zone and the languages)
- Test: unit tests in `locale.rs`; `crates/plugin-host/tests/wit_coverage.rs` and
  `sdk_drift.rs` if they enumerate imports (follow their failures)

**Interfaces:**
- Produces (WIT): `languages: func() -> list<string>;` in `interface locale`, with the doc:
  the computer's languages, best first; ISO 639 codes of 2–3 lowercase ASCII letters,
  deduplicated, at most 16; empty when the launcher cannot tell; read when called; no
  permission. Sources: the OS's preferred languages, then (Linux) the KDE keyboard layouts.
- Produces (host): `pub type LanguagesSource = Arc<dyn Fn() -> Vec<String> + Send + Sync>;`,
  `pub(crate) fn system_languages() -> LanguagesSource`, `pub fn language_codes(raw:
  impl IntoIterator<Item = String>) -> Vec<String>` (the sanitizer), `pub fn
  kxkb_languages(kxkbrc: &str) -> Vec<String>` (parses `LayoutList=` from the `[Layout]`
  group and maps layouts), `WasmPlugin::with_languages(self, source: LanguagesSource) -> Self`.
- Produces (SDK): `tomakast_plugin::locale::languages() -> Vec<String>`.

- [ ] **Step 1: Write failing host unit tests** in `locale.rs`:
  - `language_codes(["en_US.UTF-8","ru_RU","C","POSIX","zh-Hans-CN","en","pt_BR","x","toolong"])`
    gives `["en","ru","zh","pt"]`;
  - a list of 40 distinct codes is cut to 16;
  - `kxkb_languages("[Layout]\nDisplayNames=,\nLayoutList=us,ru,ua\nUse=true\n")` gives
    `["en","ru","uk"]`;
  - `kxkb_languages("[Layout]\nLayoutList=us(intl),de(nodeadkeys),zz\n")` gives
    `["en","de"]` (variants stripped, unknown skipped);
  - no `[Layout]` group or no key gives `[]`.
- [ ] **Step 2: Run** `cargo test -p tomakast-plugin-host --lib locale`. Expected: FAIL (not
  defined).
- [ ] **Step 3: Implement.** `system_languages` reads `sys_locale::get_locales()`, then on
  Linux `$XDG_CONFIG_HOME/kxkbrc` (fallback `$HOME/.config/kxkbrc`; a missing file is
  empty), concatenates, and runs `language_codes`. The xkb table is the spec's list plus
  others the builder adds. The WIT function and the `Host` impl record the import as
  `locale.languages`. Add the SDK wrapper with the WIT doc. Edit the WIT, then re-hash and
  add the FROZEN.md row: "`locale.languages()`: the computer's languages, best first."
  Reason: "Translator design 2026-11-22: a default language pair that fits any user."
- [ ] **Step 4: Run** `cargo test -p tomakast-plugin-host` and `cargo test -p tomakast-plugin-api`.
  Expected: PASS (including the freeze test).
- [ ] **Step 5: Commit** "locale.languages(): the computer's languages for plugins".

### Task 2: The compose pad (`scratchpad-view.output`)

**Files:**
- Modify: `plugin.wit` (record `scratchpad-view`: new last field `output: option<string>`,
  with the doc from spec change 2), `FROZEN`, `FROZEN.md`
- Modify: `crates/core/src/view.rs` (`ScratchpadView.output: Option<String>`),
  `crates/core/tests/wit_parity.rs`
- Modify: `crates/plugin-host/src/convert.rs`, `crates/plugin-host/src/caps.rs`
  (`MAX_OUTPUT_BYTES = 256 * KIB`, checked for `output`)
- Modify: `crates/plugin-sdk/src/builders.rs` (`ScratchpadView::output`), SDK README
- Modify: every literal `ScratchpadView { .. }`: `modules/demo/src/lib.rs`,
  `plugins/kitchen-sink/src/lib.rs`, `crates/ui/tests/{scratchpad,stack}.rs`,
  `crates/plugin-host/src/{caps,convert}.rs` tests (`output: None`)
- Modify: `crates/ui/src/views/scratchpad.rs`, `crates/ui/src/launcher_view.rs` (pass the
  markdown link opener to the pad's render)
- Modify: `docs/plugins/writing-a-plugin.md` (views table: the scratchpad's compose mode)
- Test: `crates/ui/tests/scratchpad.rs`, `caps.rs` unit tests

**Interfaces:**
- Produces (core): `ScratchpadView { .., output: Option<String> }`.
- Produces (SDK): `impl ScratchpadView { pub fn output(self, markdown: impl Into<String>) -> ScratchpadView }`.
- Produces (UI element ids): `scratchpad-output` (the markdown pane). The editor keeps
  `scratchpad-editor`, and `scratchpad-annotations` is absent in compose mode.

- [ ] **Step 1: Write failing tests.**
  - `caps.rs`: a pad with `output` of exactly 256 KiB passes, and one byte more is
    rejected with a message naming "scratchpad output".
  - `crates/ui/tests/scratchpad.rs`:
    - `compose_pad_shows_output_below_the_editor`: push a pad with `output:
      Some("**Hola**")`. `scratchpad-output` exists, its bottom is below the editor's, and
      `scratchpad-annotations` does not exist.
    - `compose_pad_wraps`: a 400-character line with no breaks lays out higher than one
      line height, and the editor has no horizontal scroll.
    - `compose_pad_output_updates_at_the_revision`: an `update-view` with the current
      revision replaces the output text. One with an older revision is ignored.
    - `pad_without_output_is_unchanged`: the existing annotation tests pass untouched.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test scratchpad` and
  `cargo test -p tomakast-plugin-host --lib caps`. Expected: FAIL / compile errors.
- [ ] **Step 3: Implement.**
  - WIT, core, host convert, caps and SDK.
  - In `scratchpad.rs`, create the editor with `set_soft_wrap(view.output.is_some())`, and
    set it again when an update flips it.
  - In compose mode, render a column:
    - the editor, about 40% of the height, in the UI font (not mono), the body size;
    - a divider in `palette.divider`;
    - `markdown::render_with(output, links, cx)` in a `flex_1`, `overflow_y_scroll` div with
      id `scratchpad-output`, `Role::Document`, and its aria label from
      `markdown::plain_text`.

  Then the WIT edit, re-hash and FROZEN.md row: "`scratchpad-view.output`: a compose pad
  (wrapping editor, markdown pane, 256 KiB)".
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo test -p tomakast-plugin-api`.
  Expected: PASS.
- [ ] **Step 5: Commit** "Compose pad: a scratchpad with a markdown output pane under a
  wrapping editor".

### Task 3: Ctrl/Cmd+Enter in a pad, and the waiting edit before any pad action

**Files:**
- Modify: `crates/ui/src/launcher_view.rs` (`submit_top_form` handles a scratchpad on top;
  `run_row_action` sends `unsent_text` first when the view is a scratchpad)
- Modify: `docs/plugins/writing-a-plugin.md` (calc's pad paragraph: Ctrl+Enter runs the
  first action), the WIT doc of `scratchpad-view` ("Enter types a newline… Ctrl+Enter, Cmd+Enter
  on macOS, runs the first action"), FROZEN row (a doc change, same commit)
- Test: `crates/ui/tests/scratchpad.rs`

**Interfaces:**
- Consumes: `LauncherView::unsent_text(instance, cx) -> Option<ViewEvent>` (exists).

- [ ] **Step 1: Write failing tests.**
  - `ctrl_enter_runs_the_pads_first_action`: on a pad with actions `[copy, other]`, typing
    `ctrl-enter` sends `ActionChosen { row_id: <view id>, action_id: "copy" }` and inserts no
    newline.
  - `pad_action_sends_the_waiting_edit_first`: type `x` and press `ctrl-enter` within the
    30 ms debounce. The module receives `TextChanged { text: "x", .. }` before the
    `ActionChosen`.
  - The same check through the action panel (Ctrl+K, Enter on the action).
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test scratchpad`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - In `submit_top_form`, a scratchpad on top runs its first action, if any, through
    `run_row_action` with the view's id as `row_id`.
  - In `run_row_action`, before the `ActionChosen` of a scratchpad instance, send
    `unsent_text` as a `Query` event, so the module sees the edit first.

  Then the WIT doc edit and re-hash.
- [ ] **Step 4: Run** `cargo test -p tomakast-ui` and `cargo test -p tomakast-plugin-api`.
  Expected: PASS.
- [ ] **Step 5: Commit** "Ctrl/Cmd+Enter runs a pad's first action; a pad action follows its
  waiting edit".

### Task 4: Plugin skeleton, the language table, and the `just` recipes

**Files:**
- Create: `plugins/translator/{Cargo.toml,plugin.toml}`, `src/lib.rs` (empty `Plugin` with
  `init` only), `src/json.rs` (copy of `plugins/weather/src/json.rs`, header naming the copy,
  `MAX_BYTES = 1024 * 1024`, `MAX_DEPTH = 16`), `src/langs.rs`
- Modify: `justfile` (`build`, `try`, `check`, `fmt`: add `translator`; `try` takes a second
  parameter `keys=""`, and `nokeys` keeps `TOMAKAST_NO_KEYBOARD=1` instead of unsetting it;
  `try` installs and approves translator too; update the recipe comments)

**Interfaces:**
- Produces:
  ```rust
  pub struct Lang { pub code: &'static str, pub english: &'static str, pub native: &'static str,
                    pub ru: Option<RuForms> }
  pub struct RuForms { pub nominative: &'static str, pub prepositional: &'static str,
                       pub adverb: &'static str }   // "немецкий", "немецком", "по-немецки"
  pub static LANGS: &[Lang];                                  // Google's list, English A–Z
  pub fn by_code(code: &str) -> Option<&'static Lang>;        // "zh"→zh-CN, "iw"→he, any case
  pub fn by_name(name: &str) -> Option<&'static Lang>;        // exact, case-insensitive: english,
                                                              // native, nominative, prepositional
  pub fn by_adverb(word: &str) -> Option<&'static Lang>;      // "по-немецки" (with the "по-")
  pub fn name(code: &str) -> String;                          // English name, else the code
  ```
- The Cargo.toml mirrors calc's: `crate-type = ["cdylib","rlib"]`, an empty `[workspace]`,
  the same release profile, and dependencies on `tomakast-plugin` only.

- [ ] **Step 1: Write failing tests** in `langs.rs`:
  - `by_name("German")`, `by_name("deutsch")`, `by_name("немецкий")` and
    `by_name("немецком")` are all `de`; `by_adverb("по-английски")` is `en`;
    `by_code("zh")` is `zh-CN`;
  - `by_name("port")` is `None`, and `by_name("de")` is `None` (codes are not names);
  - every code is unique; every English, native and Russian form maps to one language only
    (no form shared by two);
  - at least 40 languages have `ru` forms, and `LANGS.len() >= 130`.
- [ ] **Step 2: Run** `CARGO_TARGET_DIR=target/plugins cargo test --manifest-path plugins/translator/Cargo.toml`.
  Expected: FAIL.
- [ ] **Step 3: Implement** the table from Google Translate's supported-language list.
  Native names are lowercase-compared, as all names are. Then the manifest per Global
  Constraints, and the justfile changes.
- [ ] **Step 4: Run** the tests, then `just _guest plugins/translator` and `just build`.
  Expected: PASS, and `target/plugins/wasm32-wasip2/release/translator.wasm` exists.
- [ ] **Step 5: Commit** "translator: skeleton, language table, just recipes (try … nokeys)".

### Task 5: The Google provider

**Files:**
- Create: `plugins/translator/src/google.rs`
- Create: `plugins/translator/tests/fixtures/`. Record each answer once with curl (POST,
  the Task's query), saved as returned:
  - `word-run-en-ru.json` (`run`, `tl=ru`, dictionary);
  - `word-bezhat-ru-en.json` (`бежать`, `tl=en`, dictionary);
  - `sentence-en-ru.json` (`The quick brown fox jumps over the lazy dog.`, no dictionary);
  - `text-en-ru.json` (a 6.5k three-paragraph text with `*`, `_`, `#`, no dictionary);
  - `spell-en-ru.json` (`recieve`, dictionary);
  - `name-en-ru.json` (`Raycast`, dictionary: no `dict`).

**Interfaces:**
- Produces:
  ```rust
  pub const HOST: &str = "translate.googleapis.com";
  pub const MAX_CHARS: usize = 30_000;
  #[derive(Clone, Debug, PartialEq, Eq, Hash)]
  pub struct Request { pub text: String, pub from: Option<String>, pub to: String }
  pub fn is_short(text: &str) -> bool;                         // spec: ≤3 words, one line, ≤64 chars
  pub fn cut(text: &str) -> (&str, bool);                      // ≤ MAX_CHARS at a sentence/line end; true if cut
  pub fn http_request(req: &Request) -> HttpRequest;           // POST, dt parts when is_short
  #[derive(Clone, Debug, PartialEq)]
  pub struct Translation { pub text: String, pub src: String, pub translit: Option<String>,
      pub src_translit: Option<String>, pub spell: Option<String>, pub dict: Vec<PosTerms>,
      pub definitions: Vec<PosDefinitions>, pub examples: Vec<String>, pub synsets: Vec<PosSynonyms> }
  pub struct PosTerms { pub pos: String, pub terms: Vec<String> }
  pub struct PosDefinitions { pub pos: String, pub entries: Vec<Definition> }
  pub struct Definition { pub gloss: String, pub example: Option<String>, pub labels: Vec<String> }
  pub struct PosSynonyms { pub pos: String, pub groups: Vec<Vec<String>> }
  #[derive(Clone, Debug, PartialEq)]
  pub enum Failure { Refused, Status(u16), Offline, Unexpected }
  impl Failure { pub fn message(&self) -> &'static str or String }   // spec wording
  pub fn parse(status: u16, body: &[u8]) -> Result<Translation, Failure>;
  ```
  `Offline` is made by `lib.rs` from an `http-done` error. `parse` never returns it.

- [ ] **Step 1: Record the fixtures** with curl (scratchpad shell, not committed scripts).
- [ ] **Step 2: Write failing tests:**
  - `run` gives `text == "бегать"`, `src == "en"`, the first `dict` POS `"verb"` with
    `"бежать"` among its terms, a definition with an example, an example without `<b>`,
    and a synonyms group with `"sprint"`;
  - `бежать` gives `src == "ru"` and a Russian gloss;
  - the long text keeps its 2 blank lines between paragraphs, and its length is within
    ±30% of the source;
  - `recieve` gives `spell == Some("receive")`;
  - `Raycast` gives empty `dict`, `definitions` and `synsets`;
  - `parse(429, ..)` is `Refused`, so is `parse(503, ..)`; `parse(500, ..)` is `Status(500)`;
    `parse(200, b"<html>")` is `Unexpected`;
  - `is_short("run")`, `is_short("take off")` and `is_short("бежать быстро очень")` are
    true; `is_short("a b c d")` and `is_short("a\nb")` are false;
  - `cut` on 40 000 chars of sentences ends at a `.` or line end, at ≤ 30 000 chars, with
    `true`;
  - `http_request` URL-encodes the body: the text `a&b=c ё` round-trips through a form
    decoder; the URL holds `sl=auto` when `from` is `None`.
- [ ] **Step 3: Run** the plugin's tests. Expected: FAIL.
- [ ] **Step 4: Implement** `google.rs` per spec "Provider: Google". Entities in examples are
  decoded (`&#39;`, `&quot;`, `&amp;`) after `<b>`/`</b>` are stripped.
- [ ] **Step 5: Run** the tests. Expected: PASS.
- [ ] **Step 6: Commit** "translator: Google provider and recorded answers".

### Task 6: The pair and the target

**Files:** Create `plugins/translator/src/pair.rs`.

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq)] pub struct Pair { pub first: String, pub second: Option<String> }
  #[derive(Clone, Debug, PartialEq, Default)] pub struct Choice { pub from: Option<String>, pub to: Option<String> }
  pub fn from_preference(value: &str) -> Option<Pair>;          // "ru, en" / "German, English"
  pub fn automatic(system: &[String]) -> Pair;                  // spec "Automatic"
  pub fn target_for(pair: &Pair, src: &str) -> Option<String>;  // the rule
  pub fn first_target(pair: &Pair, choice: &Choice, text: &str, previous_src: Option<&str>) -> Option<String>;
  pub fn retry_target(pair: &Pair, choice: &Choice, asked: &str, src: &str) -> Option<String>;
  pub fn label(pair: &Pair) -> String;                          // "Русский ↔ English" (native names)
  ```

- [ ] **Step 1: Write failing tests:**
  - `automatic(["en","ru"])` is (ru, en); `automatic(["de"])` is (de, en);
    `automatic([])` and `automatic(["en"])` are (en, None);
  - `from_preference("ru, en")` and `from_preference("Russian, English")` give (ru, en);
    `from_preference("xx")` gives `None`;
  - `target_for` under (ru, en): `ru` gives `en`; `uk`, `en` and `de` give `ru`. Under (en,
    None): `en` gives `None`;
  - `first_target` under (ru, en) with no choice: `"привет"` gives `en`, `"hello"` gives `ru`.
    Under (de, en): `"hello"` gives `de`. With `previous_src = Some("en")`, `"hallo"` under
    (de, en) gives `de`;
  - a choice with `to = Some("fr")` makes `first_target` `fr` and `retry_target` `None`;
  - `retry_target(pair (ru,en), asked "en", src "uk")` is `Some("ru")`;
    `retry_target((de,en), asked "de", src "de")` is `Some("en")`; when the rule agrees with
    `asked` the answer is `None`;
  - a choice with `from = Some("de")` under (ru, en) gives `first_target` `ru`.
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement.** The script of text: the most frequent script among its letters
  (Latin, Cyrillic, Greek, Arabic, Hebrew, CJK, Hangul, Kana, Devanagari, Thai, Georgian,
  Armenian, other). A small `fn script_of_lang(code) -> Script` covers the table.
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** "translator: the language pair and target rules".

### Task 7: The pane and the title

**Files:** Create `plugins/translator/src/render.rs`.

**Interfaces:**
- Consumes: `google::{Translation, Failure}`, `langs::name`.
- Produces:
  ```rust
  pub const OUTPUT_CAP: usize = 256 * 1024;
  pub fn escape(text: &str) -> String;                             // spec "Rendering the pane"
  pub fn pane(t: &Translation, input_cut: bool) -> String;         // always ≤ OUTPUT_CAP bytes
  pub fn hint(pair_label: &str) -> String;                         // empty field
  pub fn pick_target_hint() -> String;                             // "Pick a target language: Ctrl+P"
  pub fn failure(f: &Failure) -> String;
  pub fn title(from: &str, detected: bool, to: &str, busy: bool) -> String;
      // "English → Русский (detected)", " · Translating…" appended when busy
  ```

- [ ] **Step 1: Write failing tests:**
  - for `"# T\n*x* _y_ [a](b) <b>\n1. z"`, `escape` then the launcher's
    `markdown::plain_text` gives the same text. The plugin cannot link the UI crate, so
    instead assert the exact escaped string and that no markdown construct survives:
    every `*`, `_`, `#`, `[`, `]`, `(`, `)`, `<`, `>`, `` ` ``, `|`, `!`, `\` is preceded by
    `\`, a leading `1.` becomes `1\.`, and each single `\n` becomes `  \n`;
  - `pane(word run)` has the sections "Translations", "Meanings", "Examples", "Synonyms" in
    that order, at most 8 terms per POS, 3 meanings per POS and 5 examples;
  - `pane(Raycast)` and `pane(sentence)` contain none of those headings;
  - `pane(spell)` starts with "Did you mean" and `receive`;
  - a translation of 300 KiB yields a pane of at most `OUTPUT_CAP` bytes ending in the
    notice; `input_cut = true` adds "Translated the first 30 000 characters.";
  - `title("en", true, "ru", false)` is `"English → Russian (detected)"`, built from
    English names by `langs::name`, to match the launcher's English UI.
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** "translator: the pane's markdown".

### Task 8: The pad, the pickers, recent translations

**Files:** Create `plugins/translator/src/{cache.rs,pad.rs,picker.rs,recent.rs}`. Modify
`src/lib.rs` (command, view events, timers, http, storage, preferences).

**Interfaces:**
- Produces:
  ```rust
  // cache.rs
  pub struct Cache;  impl Cache { pub fn get(&mut self, r: &Request) -> Option<&Translation>;
                                  pub fn put(&mut self, r: Request, t: Translation); }   // LRU, 64
  // pad.rs: pure; lib.rs does the I/O
  pub const VIEW: &str = "pad";
  pub struct Pad { /* text, revision, choice (session), last_src, shown: Option<(Request, Result<Translation,Failure>)>, asked: Option<Request>, retried: bool */ }
  impl Pad {
      pub fn new(text: String, choice: Choice) -> Pad;
      pub fn edited(&mut self, revision: u64, text: String);
      pub fn request(&self, pair: &Pair, stored: &Choice) -> Option<Request>;  // None: empty, or no target
      pub fn asking(&mut self, r: Request);
      pub fn answered(&mut self, r: &Request, a: Result<Translation, Failure>, pair: &Pair, stored: &Choice) -> Option<Request>; // the retry
      pub fn current(&self) -> Option<&Translation>;            // only if it answers today's text
      pub fn view(&self, pair: &Pair, stored: &Choice) -> ScratchpadView;   // title, output, actions, revision
      pub fn swapped(&self) -> Option<Pad>;
  }
  // picker.rs
  pub enum Side { Target, Source }   // view ids "to-lang", "from-lang"; row ids "auto"/"detect"/codes
  pub fn view(side: Side, query: &str, readings: &[String], pair: &Pair, revision: u64) -> ListView;
  // recent.rs
  pub struct Recent { pub text: String, pub from: Option<String>, pub to: String }
  pub fn load(raw: &str) -> Vec<Recent>;  pub fn save(list: &[Recent]) -> String;
  pub fn remember(list: &mut Vec<Recent>, entry: Recent);    // dedupe by text, newest first, caps
  pub fn view(list: &[Recent]) -> ListView;                  // id "recent", rows "r0".."r19"
  ```
  Action ids on the pad: `copy`, `copy-source` (`ctrl+shift+c`), `swap` (`ctrl+s`),
  `to-lang` (`ctrl+p`), `from-lang` (`ctrl+shift+p`), `recent` (`ctrl+h`).
  Storage keys: `target`, `source`, `recent`.

- [ ] **Step 1: Write failing native tests** for `Pad`, `picker`, `recent` and `cache`:
  - `edited` then `request` under (ru, en): `"hello"` asks `to = ru`. An answer with `src
    "uk"` for a Cyrillic text returns a retry to `ru` once. A second `answered` returns
    `None`;
  - **Review Focus 5:** `answered` for a request whose text is not the pad's current text
    leaves `current()` unchanged, and `view().output` still shows the previous pane;
  - **Review Focus 1:** after `edited` to a new text with no answer yet, `current()` is
    `None`;
  - the `view()` of an empty pad shows `render::hint`. Under (en, None), English text shows
    `pick_target_hint` and `request` is `None`;
  - `swapped()` has the translation as text and `Choice { from: old to, to: detected src }`;
  - picker: the query `немец` or `deut` keeps German, `german` too; the first row is `auto`
    (Target) or `detect` (Source);
  - recent: `remember` 25 entries keeps 20, newest first; the same text twice keeps one;
    a 20 KiB text is cut to 8 KiB; `load(save(x)) == x`; `load("garbage")` is empty;
  - cache: the 65th put evicts the least recently got.
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement** the modules, then `lib.rs`:
  - `run(translate)` pushes `Pad::new("", Choice::default()).view(..)`;
  - `text-changed` → `edited`, restart the 400 ms timer `pad-rest`. The timer then gets
    `request`: from the cache answer at once, else `http.fetch` (remember request-id →
    purpose);
  - `http-done` → `google::parse`, or `Failure::Offline` on error → `answered`. Then a
    retry fetch, or cache put and `update-view` with the pad's last revision;
  - the actions:
    - `copy`: `current()` → `copy-text` and `remember`, else toast "Still translating";
    - `copy-source`: copy the text;
    - `swap`: push the swapped pad;
    - `to-lang` / `from-lang`: push the picker;
    - `recent`: push the recent view;
  - picker `query-changed` → `update-view`. Picker `action-chosen` → write `target`/`source`
    in storage (remove the key for `auto`/`detect`), clear that side of the pad's session
    choice, then push the pad (`VIEW`) with its text, which returns to it, and translate
    at once;
  - recent `action-chosen` → push a new `Pad` with that text and its session choice;
  - pad `closed` → `remember` if `current()` is not empty, then drop the pad;
  - `preferences-changed` → recompute the pair and translate an open pad again;
  - the pair is recomputed on every pad open: preference, else
    `automatic(&locale::languages())`.
- [ ] **Step 4: Run** the native tests and `just _guest plugins/translator`. Expected: PASS.
- [ ] **Step 5: Commit** "translator: the pad, language pickers and recent translations".

### Task 9: Root-search phrase, the tile, Home

**Files:** Create `plugins/translator/src/{phrase.rs,tile.rs}`. Modify `src/lib.rs`
(`search_with`, `render_widget`, widget events, `home`).

**Interfaces:**
- Produces:
  ```rust
  // phrase.rs
  #[derive(Debug, PartialEq)] pub struct Phrase { pub text: String, pub to: &'static str }
  pub fn parse(query: &str) -> Option<Phrase>;
  pub fn parse_any(query: &str, readings: &[QueryReading]) -> Option<Phrase>;  // as typed, then readings in order
  // tile.rs
  pub const WIDGET: &str = "translation";
  pub enum TileState<'a> { Busy, Done(&'a Translation), Failed(&'a Failure) }
  pub fn frame(ctx: &WidgetContext, text: &str, from: Option<&str>, to: &str, state: TileState) -> WidgetFrame;
  pub fn home_key(from: Option<&str>, to: &str, text: &str) -> String;   // "home:" + FNV-1a 64 hex of "from|to|text"
  ```
  Tile actions: `copy` "Copy Translation" (first) and `open` "Open in Translator". The
  Home value is `"<src>\n<translation>"`.

- [ ] **Step 1: Write failing tests:**
  - phrase matches:
    - `hello in russian` gives ("hello", ru);
    - `"good night" into French` gives ("good night", fr);
    - `привет на немецком`, `привет на немецкий` and `спасибо по-английски` each give the
      expected language;
    - `«как дела» in english` gives ("как дела", en);
  - phrase misses: `go to port`, `log in`, `check in gym`, `in russian`, `hello in de`,
    `hello in port`, and a 501-character text;
  - `parse_any("ghbdtn yf fyukbqcrjv", [layout "привет на английском"])` gives ("привет",
    en);
  - the `tile::frame` texts for `Done(run)` contain `run`, `бегать`, "English (detected)"
    and "Russian". `Busy` shows "Translating…", and `Failed(Refused)` shows "No translation";
  - `home_key` is stable and differs for different `to`.
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - `search_with` → `parse_any`. On a match, answer one `WidgetAnswer` (`translation`,
    `double`, params, the two actions). Arm the 300 ms one-shot `search-rest`, replacing
    the last. The timer fetches if the cache misses and no request for it is out.
    `search` never fetches.
  - `render_widget` reads the cache, the in-flight set and the failures by request.
  - The widget `action`:
    - `copy`: the cached translation → `copy-text`, else toast "Still translating";
    - `open`: push `Pad::new(text, Choice { from, to: Some(to) })` and translate.
  - The `home` event: for each tile, load `home_key` from storage into the cache. A miss
    is fetched and stored on answer. Keys no tile names are removed.
- [ ] **Step 4: Run** the tests and `just _guest plugins/translator`. Expected: PASS.
- [ ] **Step 5: Commit** "translator: root-search phrase, the translation tile, Home".

### Task 10: The plugin through the host, docs, acceptance

**Files:**
- Create: `crates/plugin-host/tests/translator.rs` (the `plugin_harness_with` pattern from
  `weather.rs`; a fake Google that answers by the URL's `tl` and `dt` parameters with the
  Task 5 fixtures; `WasmPlugin::with_languages(Arc::new(|| vec!["en".into(), "ru".into()]))`)
- Modify: `docs/plugins/writing-a-plugin.md` ("A networked example: translator", a short
  section like weather's: provider, pad, pickers, phrase, privacy)
- Create: `docs/experiments/2026-11-22-translator-acceptance.md`
- Modify: `docs/superpowers/backlog.md` (the Translator line points to the spec, marked done)

- [ ] **Step 1: Write the host tests:**
  - `translate_opens_a_compose_pad` (a pushed pad with `output: Some(hint)`);
  - `typing_translates_after_rest`: text-changed `hello`, then within 1 s an `update-view`
    with output containing the fixture's translation, and exactly one request to
    `translate.googleapis.com`;
  - `ctrl_enter_copy_before_answer_says_still_translating`;
  - `phrase_shows_a_tile_and_enter_copies`;
  - `other_queries_send_nothing`: `firefox`, `go to port`, `hello` → zero requests after
    500 ms;
  - `offline_is_calm`: offline responder → the pane says "No connection.", no toast,
    plugin alive.
- [ ] **Step 2: Run** `cargo test -p tomakast-plugin-host --test translator`. Expected: PASS
  (implementation exists; fix what fails).
- [ ] **Step 3: Run** `just check`. Expected: every step passes.
- [ ] **Step 4: Acceptance by hand** with `just try approve nokeys`, `just toggle` and
  `just log`. In the note, record for each item what was typed, what showed, and pass or
  fail:
  - a word, a sentence and a pasted three-paragraph text, both directions;
  - German → Russian;
  - Ctrl+P / Ctrl+Shift+P / Ctrl+S;
  - Ctrl+Enter;
  - `hello in russian`, `привет на немецком`, the wrong layout;
  - offline (`http` refused);
  - "Add to Home".

  Screenshots stay out of git.
- [ ] **Step 5: Commit** "translator: host tests, docs, acceptance".
