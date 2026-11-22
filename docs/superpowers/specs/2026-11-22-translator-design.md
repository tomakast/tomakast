# Translator

Status: draft for the user's approval (2026-11-22); builder check done. The user approved the sketch in chat
(Google without a key, a "Translate" command, an explicit phrase in the root search, any
language, a default pair from the system's languages). This document adds what the sketch
did not know: the launcher's search field cannot hold a text (see "Why the text field is a
pad").

## Intent

The user wants a translator in tomakast with roughly Raycast's UX. When a dictionary knows
the word, it should also show meanings, examples and synonyms. Translating **texts** comes
first, and they are not only big ones: a sentence, a paragraph, a pasted email. The
dictionary is a bonus for short input. Every language Google knows is in scope. Russian
and English are the user's own pair, not a limit.

## What Raycast does (researched, 2026-11-22)

From the Raycast manual (`manual.raycast.com/translate`) and the 1.57 changelog:

- **Command "Translate".** It has a source field on top and the translation below, and the
  translation updates as you type or paste. The source defaults to "Detect Language", and
  the detected language shows as a subtitle. The target is the last one chosen, or the one
  set in settings.
- **Actions.** Copy Translation is the default, and Paste to Active App can replace it as
  the default. There are also Copy Source Text, Swap Languages (the translation becomes the
  source) and Change Source/Target Language. The language pickers use fuzzy search.
- **Shortcuts.** Cmd+Enter runs the primary action. Cmd+S swaps, Cmd+P picks the target and
  Shift+Cmd+P picks the source. Esc clears the field, or closes it when the field is empty.
- **Root search.** `hello in german` and `"good night" into french` translate inline. The
  language name has at least 3 characters and is fuzzy-matched. Language codes are not
  matched.
- **No dictionary.** Raycast shows no meanings, examples or synonyms. That part is ours.

## Goals

1. **A "Translate" command** opens a translator: a multi-line field for the text with the
   translation below it. It translates as the user types or pastes. The source language is
   detected, and the target follows the user's language pair. Any language can be picked as
   source or target.
2. **Texts work.** Line breaks and paragraphs survive both ways. A text of 25 000
   characters translates whole.
3. **A dictionary for short input.** For a word or a short phrase (at most 3 words, one
   line), the translation is followed by more: the other translations by part of speech,
   meanings with examples, more examples, synonyms, transliteration, and "Did you mean" for
   a misspelling. A part is shown only when Google has it.
4. **Root search, only on an explicit phrase.** `hello in russian`, `привет на немецком` and
   `спасибо по-английски` each show a tile with the translation. Enter copies it. Nothing
   else typed in the root search leaves the computer.
5. **Any language.** Every language of Google Translate is in the pickers. The default pair
   comes from the system's languages, so it fits a German user as well as this one.
6. **Calm failures.** No network, a refusal or an answer that cannot be parsed shows one
   line in the translation pane. There is no toast, and the plugin never breaks.

## Non-goals

- **Pasting into the active app, and translating the selected text.** A plugin can only
  copy (`copy-text`). Pasting and reading the selection are launcher features for later.
- **A provider other than Google, and API keys.** DeepL behind an optional key is a
  follow-up (see Approaches).
- **Text-to-speech, dictation, and "Continue in AI chat".**
- **Custom translate commands** (Raycast's "Translate to Spanish" with its own hotkey).
- **Reading GNOME's input sources** for the default languages. KDE and the OS's language
  list are read; see "Default languages".

## Anti-goals

- **Sending the root search to Google.** Translating every query as the user types would
  send the user's app and file searches to Google. Only the explicit phrase triggers a
  request.
- **Gluing lines together.** A translator that turns `one⏎two` into `onetwo` fails the main
  use case.
- **Toasts or error rows** when the network is down.
- **Breaking calc's pad.** The pad change below must leave calc's pad exactly as it is.

## Constraints

- **The plugin.** It is a WASM plugin in `plugins/translator`, modelled on `plugins/weather`
  (network, rest timers, calm failures) and `plugins/calc` (its pad, its search tile). It
  needs the permissions `search`, `http = ["translate.googleapis.com"]`, `timers`,
  `storage` and `clipboard`.
- **Deadlines.** Search gets 100 ms and `on-event` 1 s. At most 4 HTTP requests are in
  flight. The background budget is 1 s per 10 s.
- **Shared code.** Other sessions work on the settings GUI and file search. The core and
  UI changes are kept to the three items under "Launcher changes".
- **WIT 0.1** may be edited in place (memory "WIT freeze lifted"). Every edit adds a
  `FROZEN.md` row and a re-hash.

## Approaches

1. **Google's no-key endpoint (chosen).** This is the endpoint browser extensions use:
   `translate.googleapis.com/translate_a/single?client=gtx`. One request returns the
   translation, the detected language and, with the `dt` parts, the dictionary: the other
   translations by part of speech with reverse translations, meanings with examples (in the
   source language, so Russian words get Russian meanings), examples, synonyms,
   transliteration and spelling. Probed on 2026-11-22:
   - `run` (en→ru) and `бежать` (ru→en) both came with meanings and examples.
   - A 6.5k text and a 26k text came back whole by POST, line breaks kept.

   It works with no setup. The risk: the endpoint is unofficial and can be rate-limited or
   closed. The plugin keeps the provider behind one module, so a second one can be added
   later.
2. **DeepL with the user's key, plus Wiktionary for the dictionary.** This is the official
   route, and DeepL's text quality is better. Without a key, though, nothing works. It
   needs two providers, and Wiktionary's REST definitions are English glosses only: no
   Russian meanings for Russian words, and HTML to strip.
3. **Approach 1 plus an optional DeepL key for texts.** This is the natural next step. It
   was deferred because it cannot be tested here without a key.

The user picked 1 in chat.

## Design

### What the user sees: the "Translate" command

The command "Translate" (Lucide icon `languages`) opens the translator pad:

```
┌──────────────────────────────────────────────────────────────┐
│ The quick brown fox jumps over the lazy dog.                 │  ← the text: multi-line,
│ It was a bright cold day in April.                           │    wraps, Enter = newline
├──────────────────────────────────────────────────────────────┤
│ Быстрая бурая лиса перепрыгивает через ленивую собаку.        │  ← the translation pane
│ Был ясный холодный апрельский день.                           │    (markdown, scrolls)
│                                                              │
└──────────────────────────────────────────────────────────────┘
  Translator · English → Русский (detected)           Ctrl+K
```

For a word, the pane continues with the dictionary:

```
  бегать
  ─────────────────────────────
  run · /rən/ · begat'
  Translations
    verb   бежать, работать, управлять, бегать, идти, течь …
    noun   пробег, бег, рейс, прогон …
  Meanings
    verb   move at a speed faster than a walk …
           "the dog ran across the road"
           pass or cause to pass quickly in a particular direction …
  Examples
    a cross-country run · Bobby set off at a run …
  Synonyms
    verb   sprint, race, dart, rush, dash, hurry …
```

- **When it translates.** It translates once the text has rested 400 ms. The pad's own
  30 ms debounce sends the text, and the plugin waits on a timer. While a request is out,
  the pane keeps the last translation, with "Translating…" in the title. An answer for an
  older text is dropped.
- **An empty field** shows a hint in the pane: "Type or paste text to translate.
  English ↔ Русский. Ctrl+P: target language, Ctrl+Shift+P: source language." It names the
  current pair.
- **The title** (shown in the footer as `Translator · <title>`) reads
  `English → Русский (detected)`, or `English → Русский` when the source was picked.
- **Actions** (Ctrl/Cmd+K, in this order):
  1. **Copy Translation**: the first action, run by **Ctrl+Enter** (Cmd+Enter on macOS; see
     "Launcher changes"). It copies the translation of the text as it is now. If the text
     changed since the last answer, it copies nothing and toasts "Still translating".
  2. **Copy Source Text** (`ctrl+shift+c`).
  3. **Swap Languages** (`ctrl+s`): the translation becomes the text, and source and target
     swap. Both are then fixed: the source is no longer detected.
  4. **Translate To…** (`ctrl+p`): pushes the language list (below).
  5. **Translate From…** (`ctrl+shift+p`): pushes the language list with "Detect language"
     first.
  6. **Recent Translations** (`ctrl+shift+h`; plain Ctrl+H is a text-editing key the launcher keeps): pushes the history list (below).

  If a shortcut collides with a launcher key, the action still runs from the panel (the
  WIT rule).
- **The language list** is a `list-view` in `module` search mode. `core-filter` matches
  titles only, and `deutsch`, `немецкий` and `german` must all find German. The first row
  is "Automatic (Русский ↔ English)" for the target, or "Detect language" for the source.
  Then come all Google languages: the pair first, then the recently picked, then the rest
  A–Z. Each row's title is the English name and its subtitle is the native name. The
  plugin filters on `query-changed`: a case-insensitive prefix of any word of the English,
  native or Russian name, checked against the query and its readings. It answers with
  `update-view` at that revision. Choosing a row stores the choice and returns to
  the pad by pushing the pad's view id again with the current text. The WIT's "push an id
  on the stack" rule pops the list and replaces the pad. The pad then translates again.
  The cursor moves to the end of the text, which is accepted.
- **Recent Translations** is a `list-view` (`core-filter`) of the last 20 texts, newest
  first. Each row shows the first line of the text, with `English → Русский` as an
  accessory. Choosing a row opens the pad with that text and its languages.
- **Esc** pops the pad, as for every view.

### Default languages: the pair

The plugin keeps a **pair** (first, second): the user's own language and the other one.

- **Text in the first language** goes to the second.
- **Text in any other language** goes to the first.
- **From the preference.** `languages` (text, no default) is "first, second" as codes or
  English names: `ru, en`, `German, English`. If it is empty or does not parse, the pair is
  automatic, with a warning in the log.
- **Automatic.** The plugin reads the system's languages with the new
  `locale.languages()` (see "Launcher changes"): an ordered list of codes. The first is the
  first language that is not English, else English. The second is English when the first is
  not, else nothing.
  - On this machine `LANG=en_US.UTF-8` and KDE's layouts are `us,ru`, so the list is
    `[en, ru]` and the pair is (ru, en).
  - A German system gives (de, en).
  - An English-only system gives (en, —). English text then has no target until the user
    picks one with Translate To…. The pane says so: "Pick a target language: Ctrl+P".
- **A picked target** ("Translate To…") overrides the pair: every text goes to it until
  "Automatic" is picked. A picked source ("Translate From…") turns detection off until
  "Detect language" is picked. Both are kept in storage. Only the two pickers write them.
- **Languages for one pad only.** Some flows fix the languages for one pad only and write
  nothing to storage: Swap Languages, "Open in Translator" from a search tile, and opening
  a Recent Translations row. Such a pad keeps its own `from` and `to`, carried in the pad's
  state in the plugin (keyed by the pad's view id) and in the view it pushes again. It ends
  when that pad closes. The next "Translate" starts from the stored choices again. So
  `hello in german` does not send every later text to German.
- **Detecting before asking.** Google needs the target in the same request that detects
  the source. **The pair rule wins over saving a request.**
  - **The guess.** The plugin guesses the source first:
    - the source Google detected for this pad's previous text, if any (people keep
      typing in one language);
    - else by script against the pair. With (ru, en), Cyrillic text is `ru`, so it asks
      for `en`; other text asks for `ru`. When the script cannot tell (German and
      English), it guesses "not the first language", so it asks for the first.
  - **The check.** When the answer comes, the plugin applies the rule to Google's `src`. If
    the target the rule gives differs from the one asked for, it asks once more with that
    target. Examples: Ukrainian under (ru, en) is retried into Russian, and English under
    (de, en) is retried into German, unless the previous text was English already.
  - **Afterwards.** The second answer is final, whatever its `src`. Most texts take one
    request, and a change of language costs one more.

### Root search: the explicit phrase

`search` matches only a query that ends with a language phrase:

- **English:** `<text> in <language>` or `<text> into <language>`: `hello in russian`,
  `good night into French`. `to` is not a trigger: `go to port` must not send anything.
- **Russian:** `<text> на <language>` or `<text> по-<adverb>`: `привет на немецком`,
  `привет на немецкий`, `спасибо по-английски`.
- **Language names** must match whole, case-insensitive, from the language table: the
  English name, the native name (`deutsch`, `español`, `français`, `русский`) and the
  Russian forms (the nominative `немецкий`, the prepositional `немецком`, the adverb
  `по-немецки`). There is no fuzzy or prefix matching: `in port` is not Portuguese. Codes
  (`de`) are not matched.
- **The text** is what comes before the phrase, trimmed and stripped of one pair of
  surrounding quotes (`"…"`, `«…»`, `'…'`). It must be 1–500 characters, with no line
  break.
- **Readings.** When the query as typed does not match, its readings are tried in order.
  So `ghbdtn yf fyukbqcrjv` (the wrong layout) is read as `привет на английском`. The text
  sent is taken from the reading that matched.

A match answers one widget answer and no items:

- **The widget.** `translation` is a `double` tile, as calc's answer is. Its params are
  `text`, `from` (empty for detect) and `to`. Its actions are "Copy Translation" (first,
  so Enter copies) and "Open in Translator" (pushes the pad with the text and these
  languages).
- **When it fetches.** As in weather, the request starts once the query has rested 300 ms,
  through a timer armed in `search`. `search` itself never sends anything.
- **The tile** reads the result from memory by `(text, from, to)`, as `render-widget` never
  fetches:
  - **left:** the text (monospace off, at most 2 lines) over a caption with the source
    language ("English (detected)");
  - **right:** the translation, large, at most 2 lines, over the target language's name.

  While the request is out the right side says "Translating…". After a failure it says
  "No translation", and the caption says why ("Offline", "Google refused").
- **Enter before the answer** copies nothing and answers with the toast "Still
  translating".

**On Home** ("Add to Home" is the launcher's and cannot be removed): the tile translates
its `text` into `to` once and keeps the answer in storage (`home:<hash of from|to|text>`).
It is never refreshed: a translation does not go stale. The `home` event says which tiles
are on Home. A missing answer is fetched then, and a key no Home tile names any more is
removed.

### Provider: Google

- **The request.** `POST https://translate.googleapis.com/translate_a/single`.
  - Query: `client=gtx&sl=<from|auto>&tl=<to>&hl=en&dj=1&dt=t`, plus
    `&dt=bd&dt=md&dt=ex&dt=ss&dt=rm` for short input (the dictionary parts).
  - Body: `q=<text>`, form-urlencoded UTF-8.
- **The answer** (`dj=1`, JSON).
  - `sentences[].trans` is joined in order and keeps Google's line breaks.
  - The `sentences[]` entry with `translit`/`src_translit` gives the transliterations.
  - `src` is the detected language. `spell.spell_res` is "Did you mean".
  - `dict[]` is POS → terms, with `entry[].reverse_translation`.
  - `definitions[]` is POS → `gloss`, `example` and `label_info`.
  - `examples.example[].text` holds HTML `<b>`, which is stripped.
  - `synsets[]` is POS → synonym groups.
  - Unknown fields are ignored. A missing part is not shown.
- **Short input** is at most 3 words, one line and at most 64 characters.
- **Long input.** Text over 30 000 characters is cut at the last sentence or line end
  before that point. The pane ends with "Translated the first 30 000 characters."
  The plugin also measures the finished markdown in bytes. If it would pass the
  `output` cap, it cuts the translation at a line end so that it fits, and adds the same
  kind of notice. An oversized effect is never sent, because the host would reject it
  with an error toast.
- **Failures.**
  - HTTP 429 or 503 gives "Google refused: too many requests. Try again in a minute." Any
    other status gives "Google answered <status>."
  - A transport error gives "No connection." Unparseable JSON gives "Google answered
    something unexpected."
  - Each failure shows one line in the pane and one log warning per run of failures. The
    next edit tries again.
- **Cache.** Answers are cached in memory (the last 64, keyed by `(text, from, to)`), so
  swapping back, the tile and the pad cost nothing.
- **One provider module** (`src/google.rs`) has the request builder and the parser,
  natively tested against recorded answers. The rest of the plugin sees a plain
  `Translation` struct.

### Rendering the pane

The pane is markdown. The translation is escaped so that it shows exactly as Google
returned it:

- markdown punctuation (`\ * _ # [ ] ( ) < > ` ~ | !` and a leading `-`, `+`, `1.`) is
  backslash-escaped;
- a single line break is kept as a hard break;
- an empty line separates paragraphs.

The dictionary parts are built from fixed markdown with escaped values. Section names are
in English, like the rest of the launcher, and parts of speech come from Google with
`hl=en`. Each part is capped: 8 translations per part of speech, 3 meanings per part of
speech with their examples, 5 examples, and 3 synonym groups per part of speech.

### Storage

| Key | Value |
|---|---|
| `target` | The picked target code, or absent for automatic. |
| `source` | The picked source code, or absent for detect. |
| `recent` | JSON: the last 20 `{text, from, to}`, newest first. A text is cut to 8 KiB, and the whole value stays under 200 KiB. |
| `home:<hash>` | A Home tile's translation. |

A text joins `recent` when it is copied, or when the pad closes with a translated text that
is not empty. One write per event keeps it far under 20 writes per second.

### Launcher changes (shared code, kept small)

1. **`locale.languages() -> list<string>`** (WIT, host, SDK). It returns the computer's
   languages, best first, as ISO 639 codes: 2–3 lowercase ASCII letters, deduplicated, at
   most 16. There is no permission, like `local-time-zone`. The host reads, in order:
   - **The OS's preferred languages**, through the `sys-locale` crate (already in
     `Cargo.lock`, 0.3.2: `get_locales()`):
     - Unix: `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`;
     - macOS: the preferred languages;
     - Windows: the user's preferred UI languages.

     Each entry is cut to its language (`en_US.UTF-8` → `en`, `zh-Hans-CN` → `zh`). `C` and
     `POSIX` are dropped.
   - **On Linux, KDE's keyboard layouts.** `LayoutList` comes from
     `$XDG_CONFIG_HOME/kxkbrc` (default `~/.config/kxkbrc`), mapped through a small table
     of xkb layout → language (`us`/`gb` → en, `ru` → ru, `ua` → uk, `by` → be, `de` → de,
     `fr` → fr, `es` → es, `it` → it, `pt`/`br` → pt, `pl` → pl, `cz` → cs, `tr` → tr,
     `kz` → kk, `ge` → ka, `am` → hy, `il` → he, `ara` → ar, `ir` → fa, `jp` → ja,
     `kr` → ko, `cn` → zh, and more as the builder sees fit). Unknown layouts are skipped.

   The value is computed when called, so it follows changes, and stays cheap: one file read
   and one env read. An empty list means the host could not tell. Doc comment, FROZEN row,
   re-hash, freeze test, SDK wrapper and `writing-a-plugin.md` all follow.
2. **`scratchpad-view` gains `output: option<string>`** (WIT, core model, host conversion,
   SDK builder, UI). When it is `some`, the pad is a *compose pad*:
   - the editor wraps lines, and there is no annotation column (annotations are ignored);
   - below the editor, a pane renders `output` as markdown (the same renderer as the
     detail view). Its cap is **256 KiB**, the same as the pad's text, not the detail
     view's 64 KiB: a 25 000-character text translated into a script of 3 bytes per
     character (Hindi, Thai, Georgian) is about 75 KB. The WIT doc and the host's cap
     check say so. The builder checks that a 200 KiB pane still scrolls and types smoothly.
     If it does not, the pane renders lazily, or the cap comes down and the plugin's
     30 000 limit with it. The pane scrolls on its own.
   - The editor takes about 40% of the height and the pane the rest. The split is the
     builder's call, within reason.

   `update-view` replaces `output` and the title under the scratchpad's revision rule, the
   same as for annotations. With `none`, nothing changes: calc's pad is untouched.
   Markdown links open only with the `open` permission, as in every view (the translator
   does not ask for it).
3. **Ctrl+Enter (Cmd+Enter on macOS) in a pad runs the pad's first action.** Today Enter
   types a newline and the actions run only from the panel. It also works for calc (Copy
   Last Result). It is a launcher key, not a plugin shortcut, so the WIT's shortcut rules
   (no Enter) do not change. The text editor already treats Ctrl+Enter as "submit" and
   inserts no newline.
   - **Before any pad action** (from this key, the panel or a shortcut), the launcher sends
     the pad's waiting edit as `text-changed` first, as it does before `closed`. So
     Ctrl+Enter right after a paste acts on the pasted text.

### Why the text field is a pad

The sketch put the text in a list view's search field. That field is a single-line gpui-kit
input, and pasting `one\ntwo` into it gives `onetwo` (gpui-kit
`tests/integration/input/editing.rs`, `multiline_clipboard_normalization…`). The only
multi-line editor a plugin has is the scratchpad. That editor does not wrap, and it shows
per-line annotations instead of a result. Change 2 gives the scratchpad a wrapped mode with
a result pane. This is the smallest change that makes texts work, and it reuses the pad's
revision and debounce rules.

## Revision (2026-11-22): a language bar and pop-up language menus

The user tried the first build and gave two pieces of feedback. First, how to change
languages was not obvious from the screen: only Ctrl+K and the shortcuts showed it.
Second, if the language list stays full-screen, every language needs a flag. Of two ways
to fix it the user picked A: a pop-up list under a button, as Raycast and the Ctrl+K panel
do, so the pad stays in view. The other way, B, kept the full-screen list and added flags.

### What the user sees

A bar sits between the text and the translation:

```
┌────────────────────────────────────────────────────┐
│ The quick brown fox jumps over the lazy dog.       │
├────────────────────────────────────────────────────┤
│ [🇬🇧 English (detected) ▾]   ⇄   [🇷🇺 Russian ▾]     │
├────────────────────────────────────────────────────┤
│ Быстрая бурая лиса перепрыгивает через…            │
└────────────────────────────────────────────────────┘
```

- **The left button** is the source.
  - It shows the detected language with "(detected)", "Detect language" before any answer,
    or the picked source.
  - A click, or Ctrl+Shift+P, opens its menu: "Detect language", then every language.
- **⇄** swaps the languages (Ctrl+S), as Swap Languages did.
- **The right button** is the target.
  - A click, or Ctrl+P, opens its menu: "Automatic (Russian ↔ English)", then every
    language.
- **The menu** pops up under its button, over the pane:
  - a filter field on top, then rows with a flag, the English name and the native name;
  - the current choice has a check mark;
  - typing filters by the English, native and Russian names (`немец`, `deut`, `german`);
  - ↑/↓ and Enter choose, a click chooses, Esc closes it.

  Choosing does what the full-screen picker did: the choice is stored and the text is
  translated again. The pad never leaves the screen.
- **Ctrl+K** keeps Copy Translation, Copy Source Text and Recent Translations. Swap and the
  two pickers move to the bar, and their shortcuts with them.
- **Flags.** Each language has its main country's flag. English shows the UK's, Portuguese
  Portugal's, Spanish Spain's, Arabic Saudi Arabia's, and Chinese China's or Taiwan's. A
  language with no country (Esperanto, Latin, Yiddish, Kurdish, Hawaiian and the like) shows
  a neutral globe icon. The flags are PNGs made from flag-icons (MIT, lipis/flag-icons),
  shipped in the plugin's `flags/` folder with the licence. Emoji flags are not used:
  Windows draws them as letters.

### Launcher change 4: a bar of buttons with menus in a compose pad (WIT, host, SDK, UI)

`scratchpad-view` gains `toolbar: list<toolbar-button>`, shown only in a compose pad
(`output` set), as a row between the editor and the pane:

```wit
record menu-option {
  id: string,              // the id rule
  title: string,           // one line, ≤ 1 KiB
  subtitle: option<string>,
  icon: option<icon>,
  keywords: list<string>,  // matched by the filter too; ≤ 8
}
record toolbar-button {
  id: string,
  label: string,           // one line, ≤ 1 KiB
  icon: option<icon>,
  shortcut: option<string>, // the action shortcut rules
  options: list<menu-option>, // ≤ 300; empty: a plain button
  selected: option<string>, // the option shown with a check mark
}
```

- **Caps.** At most 8 buttons. A cap or a bad shortcut rejects the effect, as for actions.
- **A plain button** (no options), on a click or its shortcut, sends `action-chosen` with
  `row-id` = the view's id and `action-id` = the button's id, as a pad action does.
- **A button with options**, on a click or its shortcut, opens the menu anchored under it.
  It is the Ctrl+K panel's pop-up with the filter on top:
  - it matches the title and the keywords with the launcher's matcher, layouts and
    transliteration included;
  - choosing sends `action-chosen` with `row-id` = the option's id and `action-id` = the
    button's id.

  The pad's waiting edit is sent first, as before any pad action. Opening the menu sends
  nothing.
- **Update rule.** `update-view` replaces the toolbar under the pad's revision rule.
  Calc's pad has no toolbar and does not change.

## Testing

- **The plugin, natively** (`cargo test` in `plugins/translator`):
  - the Google parser against recorded answers saved under `tests/fixtures/`: a word en→ru
    and ru→en with the dictionary, a sentence, a 6.5k multi-paragraph text, and an answer
    with `spell`;
  - the phrase grammar: matches, misses such as `go to port`, `log in`, `in port` and
    `check in gym`, quotes, the 500-character cap, readings;
  - the pair logic and the script guess;
  - markdown escaping round-trips (text with `*`, `#`, `_`, links and line breaks shows as
    written);
  - the truncation at 30 000 characters;
  - the language table (every Google code has an English name, and the Russian forms are
    unique).
- **The host:** the `languages` sanitizer, the `kxkbrc` parse and the layout table.
- **The UI:** a compose pad renders its output and wraps. A pad without output is
  unchanged (calc's existing tests pass). Ctrl+Enter runs the first action.
- **`just check` passes.**
- **`just` recipes.** `just build`, `just try` and `just check` include the translator, as
  they do calc and weather. `just try` grabs the keyboard by design (it unsets
  `TOMAKAST_NO_KEYBOARD`). The builder's own runs must not grab the user's keyboard, so
  `just try` gains an opt-out, for example `just try approve nokeys`, that keeps
  `TOMAKAST_NO_KEYBOARD=1`. Its home is already throwaway.
- **Acceptance** (`docs/experiments/2026-11-22-translator-acceptance.md`): run with
  that recipe. Record:
  - a word, a sentence and a pasted multi-paragraph text in both directions;
  - a third language (German) to Russian;
  - the pickers and swap;
  - the root phrase in English, in Russian and on the wrong layout;
  - offline behaviour;
  - "Add to Home".

## Left to the builder

- The exact split and spacing of the compose pad, and how the pane looks in both themes.
- The full language table (codes, English, native and Russian names). Take it from
  Google's supported list. Fill the Russian forms for at least the 40 most used languages;
  the others need only the English and native names.
- The xkb → language table beyond the entries listed.
- Regional codes. `locale.languages()` gives bare codes; the plugin's language table maps
  Google's regional ones (`zh-CN`/`zh-TW`, `pt-BR`/`pt-PT`). A bare `zh` means `zh-CN`
  and a bare `pt` means `pt`.
- Text in a pad that was never translated is lost when the launcher hides. Recent
  Translations keeps what was translated. That is accepted.
- The wording of hints and errors, within the meaning above.
- Whether "Recent Translations" also reads Home's keys (it does not need to).
