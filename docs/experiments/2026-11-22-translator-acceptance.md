# Translator: acceptance (2026-11-22)

Branch `claude/translator-e1a83c`. Spec: `docs/superpowers/specs/2026-11-22-translator-design.md`.
Plan: `docs/superpowers/plans/2026-11-22-translator.md`.

## What was run

| Check | How | Result |
|---|---|---|
| Everything CI runs | `just check` | Pass (fmt, clippy native and wasm, the workspace's tests, the SDK, kitchen-sink, calc, weather and translator) |
| The plugin's logic | `cargo test` in `plugins/translator` (51 tests) | Pass |
| The plugin through the core | `cargo test -p tomakast-plugin-host --test translator` (8 tests) | Pass |
| The compose pad and Ctrl+Enter in the UI | `cargo test -p tomakast-ui --test scratchpad` (16 tests, 5 new) | Pass |
| `locale.languages()` | host unit tests, `kitchen_sink_reads_the_languages`, `wit_coverage` | Pass |
| Live load | `just try approve nokeys`, then `just run translator/translate` and `just log` | Plugin loaded in 52 ms, pad opened, no errors from the plugin |

### What the plugin tests use

- **Recorded answers.** The plugin tests run against real Google answers recorded on
  2026-11-22, in `plugins/translator/tests/fixtures/`:
  - `run` into Russian and `бежать` into English, with the dictionary;
  - a sentence;
  - a three-paragraph text with markdown in it;
  - `recieve`, for spelling;
  - `Raycast`, with no dictionary data.
- **Markdown escaping** is checked against the launcher's own parser (pulldown-cmark 0.13.6, as
  a native dev-dependency). Headings, emphasis, links, HTML, list markers, setext lines,
  indented code and `&` entities all render as typed.

### What the host tests cover

- the hint;
- one request after the text rests;
- "Still translating" before the answer (Review Focus 1);
- Russian text going to English;
- the phrase tile and Enter copying from it;
- `firefox`, `go to port`, `hello`, `log in` and `check in gym` sending nothing;
- offline, which shows one line and no toast.

## Found live, fixed

- **Ctrl+H for Recent Translations was ignored.** The launcher keeps Ctrl+H as a
  text-editing key and logged `action shortcut is a launcher or text-editing key; ignored`.
  The shortcut is now Ctrl+Shift+H, as calc's History. After the change the log shows no
  warning.

## Found by the final review, fixed

Each fix has a test that failed first.

- **An answer for the same text but another target could take the pane.** This happened
  after picking another language while a request was out. Now only the answer the pad is
  waiting for counts.
- **The host's four-request limit showed as "No connection."** Six Home tiles at once made
  the fifth say "Offline". Now one pad request is out at a time, and the latest text is
  sent when it returns. A refusal for the limit means "wait", and the waiting tiles are sent
  as requests end.
- **On an English-only pair, one English text stopped the pad from translating anything
  after it.** Now the script guess decides again. English text with no target copies
  nothing and asks for a target instead of copying the source.
- **"Still translating" stayed on screen.** It was a progress toast, which never expires; it
  now goes after 3 s.
- **Home tiles could fall out of the memory cache.** A tile shown by the search and then
  added to Home was also never stored. Home's translations are now kept apart and always
  stored, and after a reload they draw from storage with no request.
- **A huge root query could cost the search's 100 ms deadline.** The phrase check turns down
  anything longer than a phrase can be, and looks up a language name only near the end.

Measured: a compose pad with a 200 KiB pane renders a keystroke in about 3.6 ms in a release
build (56 ms in a debug build), so the pane needs no cache.

## Revision: the language bar (after the user's first look)

The user found that the screen did not show how to change languages, and asked for a flag
on every language. Option A was built:

- **The bar.** A row of buttons sits between the text and the translation: the source,
  ⇄, and the target.
- **The menus.** A language button opens a pop-up menu under it: a filter on top, then a
  flag, the English name and the native name per row, with the current one checked.
- **The flags.** There are 93 PNGs from flag-icons 7.6.0 (MIT), 64×48, 376 KB in all.
  Languages without a country show a globe.

Run for it:

- **UI tests:** `cargo test -p tomakast-ui --test scratchpad`, 7 new tests, all red first.
  They cover the bar only in a compose pad, a plain button after the waiting edit, the
  menu under its button by click and by Ctrl+P, filtering by `deut`, `немец` and `ytvtw`,
  choosing by Enter and by click, Esc closing only the menu, and the menu surviving an
  update.
- **Contract:** the caps of the toolbar and the menus, `wit_parity`, and kitchen-sink
  covering the new records (`wit_coverage`).
- **Plugin:** the bar's labels and choices before and after an answer, and asking again
  after a new choice.
- **Through the host:** choosing German from the bar asks again with `tl=de` and stores it.

A fresh review of the bar found no Critical issues. Fixed, each with a test that failed
first unless said otherwise:

- **The real 135-language menu was pushed up over its own button.** It is now placed under
  the button on every frame, and no taller than the room above the footer.
- **The current language was out of view when the menu opened, and filtering could hide
  the selected row.** Scrolling now waits until the list has been laid out.
- **The menu did not close like a pop-up.** A press outside it now closes it, and its
  button and its shortcut toggle it.
- **The plugin counted the answer from before a language change as current while a retry
  was out.** It now keeps it stale until a new answer is shown.
- **A retry could be lost while another request was out.** It is now asked again when that
  request ends. This one has no test: it needs a held request together with a cache hit.

## Not run here, for the user's try

`TOMAKAST_NO_KEYBOARD=1` gives the window no keyboard, so typing into the pad, the
pickers, the phrase in the root search and "Add to Home" were not driven by hand on this
machine. They are covered by the host and UI tests above, but not by a person's eyes.

One more gap: the real Google endpoint was reached with curl when the fixtures were
recorded, not through the launcher's HTTP client. The client's POST bodies and
`content-type` header are covered by the host's own http tests.

To try it:

```bash
just try approve
```

Then, in the launcher:

1. **The command.** Type `Translate`, Enter. Type or paste a text; the translation appears
   below after a short pause. Ctrl+Enter copies it. The bar above the translation shows the
   languages: click one (or Ctrl+P for the target, Ctrl+Shift+P for the source) to pick
   from the menu, ⇄ (Ctrl+S) swaps. Ctrl+Shift+H opens recent translations (⌘ on macOS).
2. **A word** (`run`, `бежать`): the dictionary follows the translation.
3. **The root search.** Type `hello in russian`, `привет на немецком` or
   `спасибо по-английски`: a tile shows the translation, and Enter copies it. Ctrl+K on the
   tile has "Open in Translator" and the launcher's "Add to Home".
4. **Your own pair.** Set it with `[plugins.translator] languages = "ru, en"` in
   `config.toml`; empty means the system's languages (here `en` from `LANG` and `ru` from
   KDE's layouts, so Russian ↔ English).
