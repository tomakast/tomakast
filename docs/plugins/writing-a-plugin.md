# Writing a tomakast plugin (API 0.1)

A tomakast plugin is a WebAssembly component in a folder, next to a `plugin.toml` manifest.
The launcher loads it into a sandbox and calls it when the user searches, runs one of its
commands, works in one of its views or uses one of its tiles, and when it wants a tile drawn.
The plugin answers with *effects*: things it asks the launcher to do.

Where to go next:

- **Start here:** the Rust SDK, [`crates/plugin-sdk/README.md`](../../crates/plugin-sdk/README.md).
  It has a hello-world plugin, the build command and how to install a plugin.
- **The contract:** [`crates/plugin-api/wit/since_v0.1.0/plugin.wit`](../../crates/plugin-api/wit/since_v0.1.0/plugin.wit).
  Its doc comments hold every rule; this guide summarises them, and the WIT wins where they
  differ.
- **The manifest:** [`manifest.md`](manifest.md), every key of `plugin.toml`.
- **A real plugin:** [`plugins/calc`](../../plugins/calc), explained at the end of this guide.

API 0.1 is frozen: `plugin.wit` and the manifest format do not change. A later API gets its own
folder (`wit/since_v0.2.0/`), and the launcher keeps running 0.1 plugins. (While no outside
plugin exists, the user may rule a change to 0.1 in place: the widgets on 2026-11-19, and
`readings` for `search` and `query-changed` on 2026-11-20; `FROZEN.md` next to the WIT records
every such edit.)

## The execution model

A plugin is a set of short handlers. Only the launcher calls them, and each call must return
quickly. The plugin has five exports:

| Export | When the launcher calls it | It answers |
|---|---|---|
| `init` | Once after the plugin is loaded, and again after every reload. | Effects (for example `replace-items`). |
| `search(query, readings)` | On every keystroke in the launcher search, only with the `search` permission. `readings` are the query read on other keyboard layouts or romanized (see below). | Up to 50 items and 16 widget answers (tiles) for this query. |
| `run(item, action-id)` | When the user runs an action of the plugin's item: a command from `plugin.toml` (action id `run`) or an item `search` returned (the whole item comes back). | Effects. |
| `on-event(event)` | For an event of one of the plugin's views or tiles, a fired timer, an HTTP answer, or a change of the plugin's preferences. | Effects. |
| `render-widget(id, context)` | When the launcher wants a frame of one of the plugin's tiles: only while the tile is on screen, at most 60 times per second (see [Widgets](#widgets)). | A frame. |

`readings` let a plugin find what the user typed on the wrong layout or in another script:
`ашкуащч` has the layout reading `firefox`, and `опен` the romanization (`translit`) `open`.
There are at most 8, none equal to the query or to another. A list in `module` search mode gets
the same readings in each `query-changed`. With the Rust SDK, `Plugin::search(query)` keeps
working: the export calls `Plugin::search_with(query, readings)`, which calls `search(query)`
unless the plugin implements it:

```rust
fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
    let texts = std::iter::once(query).chain(readings.into_iter().map(|r| r.text));
    for text in texts {
        if text.eq_ignore_ascii_case("firefox") {
            return Ok(vec![Item::new("my-plugin/firefox", "Firefox")?.action("open", "Open")?].into());
        }
    }
    Ok(Vec::new().into())
}
```

The plugin never waits. For "later", it asks the host: `timers.start` fires `timer-fired` into
`on-event`, and `http.fetch` returns at once and delivers `http-done` into `on-event` when the
answer is in. A blocking call (such as `std::thread::sleep`) stops the plugin.

The plugin is also not a long-lived process. The host may unload it and load it again; a
reload, a stop and an update all end the instance. So:

- keep in `storage` everything that must survive; memory is a cache;
- timers end with the instance, so re-arm them in `init`;
- unanswered HTTP requests of an ended instance are dropped.

WASI gives the plugin no files, environment, arguments, stdio or sockets. It gives the wall
clock, the monotonic clock and random numbers, and nothing else. The imports are the plugin's
only way out: `storage`, `preferences`, `open`, `http`, `timers`, `log` and `locale` (the
computer's time zone).

## Permissions and user actions

A plugin gets nothing beyond the permissions in its `plugin.toml` that the user approved (see
[`manifest.md`](manifest.md#permissions)):

| Permission | Allows |
|---|---|
| `search` | The `search` export is called; without it, never. The prompt says "See what you type in the launcher search". |
| `storage` | The `storage` import. |
| `clipboard` | The `copy-text` effect. |
| `open` | The `open` import, and links in the plugin's markdown. |
| `timers` | The `timers` import. |
| `index` | The `replace-items` effect. |
| `http = [...]` | The `http` import, to the listed hosts. |

`preferences`, `log` and `locale` need no permission, and neither does showing tiles. An
import the plugin may not call fails with an error, except `timers.cancel`, which then does
nothing. An effect it may not send is dropped, and the user sees a notice (at most one per
plugin per 10 s).

Some things also need a **user action**: a call that the user started. User actions are:

- `run`;
- `on-event` with an `action-chosen` or `form-submitted` view event;
- `on-event` with a widget event that is an `action`, a `key`, or a completed click: a pointer
  `up` on the node that got the last `down`.

Everything else is not: `init`, `search`, `render-widget`, `query-changed`, `text-changed`,
`closed` (which is also sent when the launcher hides), a widget's `shown` and `hidden`, a
pointer `down` alone, an `up` that completes no click, `move`, `enter` and `leave`, and timer,
HTTP and preference events. `copy-text` and the `open` functions need a user action, and one
call opens at most one URL or path. In answer to a widget event, `push-view`, `pop-view` and
`close-window` need one too: without it they are dropped.

## Deadlines and budgets

| Limit | Value | Over it |
|---|---|---|
| `search` deadline | 100 ms | The plugin is broken. An answer after the next keystroke is dropped anyway. |
| `init` deadline | 2000 ms | Broken. |
| `run`, `on-event` deadline | 1000 ms | Broken. |
| `render-widget`, soft deadline | 16 ms of CPU time, measured around the call on the plugin's worker thread (wall time on a system without a thread clock); 64 ms in a debug build of the launcher. Time the thread waits for a core on a loaded machine does not count. | The frame is dropped and the tile keeps its last one. 5 late or failed frames in a row stop the widget, with an error toast and a log line: an always tile's until the plugin is reloaded, a condition tile's until the launcher is next opened. |
| `render-widget`, hard deadline | 100 ms (100–110 ms with the 5 ms epoch tick) | Broken. |
| Linear memory | 64 MiB | Broken. |
| One call's return value, one import call's arguments | 4 MiB | Broken. |
| Background budget: `on-event` for timers and HTTP answers | 1 s per 10 s of time spent in the call (including host imports) | Timers are suspended for 60 s; the third time within 10 minutes, the plugin is stopped as broken. `render-widget` calls count against no budget. |
| `storage` quota | 1 MiB in total (keys, values, 32 bytes per entry) | The `set` fails with an error that starts with `quota: `. |
| `storage` key and value size | keys ≤ 256 bytes, values ≤ 256 KiB | The call fails with a plain error (not `quota: `). |
| `storage` writes (`set`, `remove`, failed ones too) | 20 per second | The write fails. |
| Toasts | 10 per 10 s | The rest are dropped. |
| Unsolicited `push-view` and `update-view` | 30 per second, together | Dropped. |
| Unsolicited `replace-items` | 1 per 5 s (`init`'s counts) | Dropped. |
| Unsolicited `show-widgets` and `set-widget-image` | 30 per second, together | Dropped. |
| Renders of one tile | 60 per second | A `redraw-widget` beyond is merged into the next render. |
| Items in `replace-items` | 1000 | The answer is rejected. |
| Items from `search` | 50 | The answer is rejected. |
| Widget answers from `search` | 16, each naming a declared widget once, payload ≤ 4 KiB | The answer is rejected. |
| Widgets in `plugin.toml` | 16 | The manifest does not parse; the plugin is not loaded. |
| One frame | 256 nodes nesting at most 32 deep; 10 000 canvas commands and path segments; 1 MiB in all (its images included); clips and transforms balanced per canvas and nested at most 32 deep; text nodes ≤ 4 KiB | The frame is dropped and logged; it counts as a failed frame. |
| Images | PNG or JPEG ≤ 2 MiB, SVG ≤ 512 KiB; 64 images and 8 MiB per plugin | The answer or the frame is rejected. A PNG or JPEG over 1024x1024 pixels, and an SVG that breaks [its rules](#images-and-svg), draw nothing. |
| Timers | 16 active; repeating ≥ 250 ms, one-shot ≥ 10 ms | `start` fails. |
| HTTP | 4 requests at a time; body ≤ 4 MiB; 30 s per request; ≤ 5 redirects | `fetch` fails, or `http-done` carries an error. |
| Log | 4 KiB per message, 100 lines per second | Truncated, or dropped. |

**Broken** means: the user sees an error toast, the plugin is unloaded, and it stays unloaded
until its folder changes or the launcher restarts. A trap (a Rust panic) breaks the plugin too.
Return `Err` for an error the user can recover from. The launcher shows the error of `run` and
`on-event` to the user; an error of `search` is only logged (the search shows no row for it).

Effects and items also have size caps (ids ≤ 256 bytes, titles ≤ 1 KiB and one line, 2000 list
rows, 64 KiB of markdown, and more; the SVG rules are in [Images and SVG](#images-and-svg)).
An effect that breaks one is rejected as a whole, with an error toast. The caps are listed on the records in `plugin.wit`; the SDK's builders check ids
as you build.

## Views and effects

A plugin shows its UI as data. There are five views:

| View | For |
|---|---|
| `list-view` | A searchable list, with sections, accessories and an optional detail pane. The launcher filters it (`core-filter`) or sends every query to the plugin (`module`, as `query-changed`, with the query's readings). |
| `grid-view` | Tiles: an image or icon and a caption. |
| `detail-view` | Markdown with metadata. Markdown is inert: no HTML, no remote images. |
| `form-view` | Text, multiline, checkbox, dropdown and date fields, sent as `form-submitted`. |
| `scratchpad-view` | A free text editor with an annotation at the end of each line, sent as `text-changed`. |

And ten effects:

| Effect | Does |
|---|---|
| `push-view` | Shows a view on top of the launcher's stack. |
| `update-view` | Replaces a view the plugin showed. |
| `pop-view` | Pops the plugin's view. |
| `close-window` | Hides the launcher. |
| `toast` | A short notice: success (3 s), error (5 s) or progress. |
| `copy-text` | Copies to the clipboard (`clipboard`, and a user action). |
| `replace-items` | Replaces the plugin's own items in the launcher search (`index`). |
| `show-widgets` | Replaces the plugin's "always" tiles (see [Widgets](#widgets)). |
| `redraw-widget` | Asks for a new `render-widget` of one of its tiles. |
| `set-widget-image` | Sends an image (PNG, JPEG or SVG) its tiles name by id. |

Which view an effect acts on depends on where it comes from:

- **In answer to a view event**, `update-view` and `pop-view` act on the view the event came
  from.
- **In answer to `run`**, and to a widget event that is a user action, `update-view` acts on
  the topmost view of the plugin with that id; `pop-view` pops the top view only if it is the
  plugin's.
- **Unsolicited** (from `init`, a timer, an HTTP answer or a preference change), `update-view`
  acts on the topmost view of the plugin with that id. `push-view` works only while the
  launcher is open and its top view is the plugin's. `pop-view` and `close-window` are ignored,
  and `copy-text` is dropped.
- **In answer to a widget event that is not a user action** (`shown`, `hidden`, a hover),
  effects are unsolicited, and `push-view`, `pop-view`, `close-window` and `copy-text` are
  dropped.

One answer pushes at most one view, and the stack holds at most 32.

**View ids.** The stack holds at most one view of a plugin with a given id. Pushing an id that
the plugin already has on the stack does not stack a copy: the launcher returns to that view.
The views above it are popped and get their `closed`, and the view is replaced by the pushed
one. An unsolicited push does this only when every view above that one is the plugin's too.
So a drill-down must put the level in the view id: a file browser that pushes `dir` for
every folder replaces the folder instead of going deeper, and Esc leaves the browser; one
that pushes `dir:<path>` stacks a view per folder.

**Revisions.** In a list and a scratchpad, what the user types belongs to the user.
`revision` counts the user's edits, and every `query-changed` and `text-changed` carries the
new revision. An `update-view` is applied only when its `revision` equals the last one the
launcher sent, and it never changes the typed text: it updates the rows, or the annotations.
A form update keeps what the user entered.

**`closed`** is sent once when a view leaves the screen: popped, or the launcher hidden. A
scratchpad's last edit arrives as `text-changed` before it. Save what must persist there; the
view will not come back.

## Widgets

A widget is a tile in the strip between the launcher's search field and its list, in the
root view only. The plugin draws its tiles; the launcher decides when, and draws them. The
Rust builders are in the [SDK README](../../crates/plugin-sdk/README.md#widgets).

**The manifest** declares each widget under `[[widgets]]` (at most 16, see
[`manifest.md`](manifest.md#widgets)): an `id`, a `title` and a `size`. A tile is 120 px
high. In the 750 px panel a `single` tile is 355 px wide, half the strip, and a `double` one
718 px, all of it. When more tiles than fit are shown, the next one peeks in at the right
edge, and the strip scrolls sideways.

**Which tiles show.**

- **"Always" tiles** show at an empty query, and stay while the user types. A plugin starts
  with none. The `show-widgets(ids)` effect replaces its always set, in answer to `init`,
  `run` or `on-event`; most plugins send it from `init`. A media tile can come and go with
  what plays. The user can order and hide them in `config.toml` (`[widgets] order` and
  `hidden`, by `<plugin-id>/<widget-id>`); the rest follow in approval order.
- **Condition tiles** come from `search`. Its answer holds items and up to 16 *widget
  answers*. A widget answer names a widget, a `payload` (at most 4 KiB) and the tile's
  actions, as an item's. The tile shows for that query, before the always tiles: the
  condition tiles of several plugins by plugin id, then each plugin's in its answer order. The
  payload comes back in every render context and every event of the tile, so the frames and
  the actions use what `search` computed for this query. A new tile appears with its first
  frame, its payload and its actions together. A tile from an older answer may stay on screen
  until the plugin's next answer (a failed search counts as an empty one), but Enter and the
  actions never act on it. Hiding the launcher drops the condition tiles: it opens with none.
  A user action on a condition tile records its query in the launcher's search history, as
  running an item does. A plugin reloaded while the launcher is open is asked the current
  query again once its `init` returns, so its tile comes back.

**Rendering.** The launcher calls `render-widget(id, context)`; the plugin never draws on
its own. It is called when a tile comes on screen; after the plugin handled one of the
tile's events; after a `redraw-widget(id)` effect (from a timer, an HTTP answer or any other
call); on the next frame while the last frame set `animate`; and when the size, the scale
or the theme change. That is at most 60 times per second per tile, and only for the tiles in
view and the one peeking in: nothing renders while a tile is off screen or the launcher is
hidden. The launcher keeps every tile's last frame and draws it at once when it opens, then
asks for a fresh one. Frames are not kept across launcher restarts: until its first frame, a
tile is an empty placeholder. The `shown` and `hidden` events say when a tile comes on screen
and leaves it; a plugin may start and stop its data timers there, but rendering needs nothing
from it.

The context gives the tile's size in logical pixels and the scale; the theme (dark or
light, the palette's colours, the UI and monospace fonts, and the text sizes: title 16, value
28, body 14, caption 12); the milliseconds since the tile came on screen, for animation; the
focused node and the pointer; and, for a condition tile, the query and the payload.
The launcher ships its fonts, whatever the machine has installed: the UI font is Inter and the
monospace font is JetBrains Mono, with its ligatures on.

**The frame** is a flat list of nodes, because WIT has no recursive types. Node 0 is the
root; every other node names an earlier row, column or overlay as its parent, and has at
most 32 ancestors.

- Layout: `row`, `column` and `overlay`, with padding, a gap, `align` (start, center, end,
  stretch) and `justify` (start, center, end, space-between). Every node may `grow` into free
  space and may have a fixed `width` and `height`. An overlay takes its size from its first
  child; the later ones are stacked on it at the same origin.
- Content: `text` (a style, a palette colour or `0xRRGGBBAA`, alignment, a line limit, the
  monospace font; at most 4 KiB, and line breaks are allowed), `icon` (drawn at most as big
  as the tile is high), `image` (by id, see below), `progress` (0 to 1) and `spacer`.
- Controls: `button`, with a label, an icon or both, and an action id. It is focusable, and
  pressing it sends `action`.
- `canvas`, below.
- The frame's `actions` are an always tile's (Enter runs the first, Ctrl/Cmd+K shows them
  all); a condition tile's come from its answer. `animate` asks for the next frame. `images`
  carries images with the frame.

The launcher draws each tile's background, its rounded corners and its focus ring. What a
frame draws is clipped to its tile, and no text or size reaches past it.

**The canvas** paints its commands in order, in its own logical pixels, clipped to the node:
rectangles (with a corner radius), ellipses and paths (move, line, quadratic, cubic, close),
each filled (non-zero winding) or stroked (centred on the outline); text; images; SVG
documents; and push and pop of a clip and of a transform. They are what GPUI paints natively,
so they have its limits:

- a paint is a solid colour or a linear gradient of **two stops**, which spans the shape's
  axis-aligned box. There are no radial gradients;
- clips are **axis-aligned rectangles**. Under a rotation, a clip clips the box around the
  rotated rectangle;
- a transform (scale, rotate, translate) moves the vertices of shapes and paths; **text and
  images do not rotate**, and take only its translation and scale. A stroke's width scales
  by the mean of the two scales;
- opacity is **per command**: there is no group opacity, and emoji glyphs ignore a text's
  opacity;
- a text is one line (at most 1 KiB), placed by the top-left of its line box, 1.25 × its size
  high. The size is 1 to 200, and at most 200 after a transform's scale too;
- a stroke too complex to tessellate in one piece is drawn in halves, and a fill is tried
  once more, coarser. A path that still fails draws nothing, and is logged once.

For everything else (radial gradients, clip paths, group opacity, rotated text and images,
filters) draw an SVG document with the `svg` command. Use SVG for art that changes rarely,
such as a backdrop or a chart, and keep fast motion in the native commands on top of it.

The caps are not a speed promise. The launcher times each tile's canvases as they paint, and
stops a widget whose canvases paint too slowly: 5 paints over 8 ms in a row, or one over
50 ms (except the tile's first paint each time it comes on screen, which may load fonts).
Debug builds of the launcher allow 4 times as long. The stopped tile leaves the strip until
the plugin is reloaded, with an error toast. Aim for at most about 1,000 primitives per tile.

### Images and SVG

A tile names images by id. Send each once, with the `set-widget-image` effect (from `init`,
for example) or in a frame's `images`, not with every frame: sending the same bytes under the
same id again costs nothing, and new bytes replace the old ones.

- PNG and JPEG of at most 1024x1024 pixels and 2 MiB; SVG of at most 512 KiB; at most 64
  images and 8 MiB per plugin.
- The launcher decodes images and rasterizes SVG outside its own process, in the
  `tomakast-raster` helper, which ships next to the `tomakast` binary. The helper runs with
  memory and CPU limits (on Unix) and is stopped when a job runs past its time.
- An SVG is rasterized at the size it shows at, on the tile's scale, and again only when the
  document, its box or the scale change.
- An SVG names nothing outside itself: no external references, no scripts, no external
  fonts. Only `data:` PNG and JPEG images are allowed inside.
- An SVG has at most 4096 elements (counted again once `<use>` copies are made), 32 filter
  primitives, and 32 primitives applied in all (a filter on many elements counts once per
  element). It draws at most 16 raster images, of 4 Mpx decoded in all. `feTurbulence` has at
  most 10 octaves, `feMorphology` a radius of at most 32 device pixels and `feConvolveMatrix`
  an order of at most 9. A filter region is at most twice the raster's size, and the filters'
  work in all stays under a weighted budget. A filter inside a pattern, a mask, a clip path
  or an `feImage` is refused.
- SVG text uses only the system fonts it names: a character none of them has is not drawn.
- A document that breaks these rules, takes more than 16 ms to rasterize, or makes the helper
  fail draws nothing, and stays refused until the plugin is reloaded.

### Events and user actions

`on-event` gets `widget` events, with the widget id, the payload of a condition tile, and one
of:

- `shown` and `hidden`;
- `action`: a button pressed, Enter on the tile, a click on the tile outside its controls
  (both run its first action), or an action chosen in the action panel;
- `pointer`: `down`, `up`, `move`, `enter` and `leave` over a canvas that has an id, in the
  canvas's logical pixels. The moves of one frame arrive as one;
- `key`: a key and its modifiers, while a focusable canvas has the keyboard.

`action`, `key` and a completed click (an `up` on the node that got the last `down`) are user
actions: their answer is treated as one to `run`, so `copy-text`, `open`, `push-view` and
`close-window` work. The answer to any other widget event is unsolicited, and its
`copy-text`, `push-view`, `pop-view` and `close-window` are dropped. A tile has no links of
its own: opening goes through an action.

**The keyboard.** The keyboard stays in the search field; ↑ and ↓ move the selection over the
screen. ↑ on the first list row selects the strip's first tile, at any query, and ↑ on a tile
selects the search field, where ↑ recalls the query history if the field is empty. ↓ walks
back: the strip's first tile, then the list. Tab moves from the search field into the strip,
at any query. In the strip, ←/→ move between tiles,
Tab and Shift+Tab through a tile's buttons and focusable canvases, Enter runs the focused
button or the tile's first action, Ctrl/Cmd+K shows the tile's actions, and ↓ or Esc go back
to the list. A focused canvas gets every key but Tab, Shift+Tab and Esc. Anywhere else,
typing goes to the search field.

A condition tile starts selected while the keyboard stays in the search field: Enter and
Ctrl/Cmd+K act on it (`2+2`, Enter copies `4`), ↓ moves the selection to the list and ↑ to
the search field. While
the search is still running, they wait up to 116 ms for its answer, then act on the tile
answered for this query, or else on the list's first row. While the user types on, the
selected tile keeps its selection until its new answer comes, so the highlight does not jump
to the list and back.

**No source label.** Tiles do not show the plugin's name, so a tile could look like anything
inside its frame. That is why the approval prompt lists your tiles by title, and why adding a
widget or renaming one asks the user again. A widget that was approved before, dropped by an
update and declared again with the same id and title, asks nothing.

## A worked example: calc

The calculator, [`plugins/calc`](../../plugins/calc), uses most of this. It is built on the SDK.

**The manifest** asks for five permissions, adds two commands, one preference and one widget:

```toml
id = "calc"
name = "Calculator"
version = "0.1.0"
api = "0.1.0"

[[commands]]
id = "calculator"
title = "Calculator"
icon = "calculator"

[[commands]]
id = "history"
title = "Calculator History"
icon = "file-clock"

[[preferences]]
key = "timezone"
title = "Time zone (IANA name such as Europe/Moscow, or a city; leave empty for the computer's)"
kind = "text"

[permissions]
search = true
storage = true
clipboard = true
timers = true
http = ["open.er-api.com", "api.coingecko.com"]

[[widgets]]
id = "answer"
title = "Calculator answer"
size = "double"
```

A command's `icon` names a Lucide icon (lucide.dev), which the launcher bundles and draws the
same on every OS: `calculator` and `file-clock` are both in the set. A name outside the set is
looked up in the desktop's icon theme on Linux (then `hicolor`); a name found in neither, and
any such name on macOS and Windows, shows the plugin's monogram.

**Search and its tile** (`src/lib.rs`, `src/views.rs`). `search` evaluates the query with
calc's own engine (`src/engine/`, plain Rust, tested natively). When the query is clearly
math, it answers no item and one widget answer: the double tile `answer`, like Raycast's.
- The payload is the result and the input on two lines (`4\n2+2`). The actions are "Copy
  Result" (`copy`, the first, so Enter runs it) and "Copy Input + Result" (`copy-both`,
  `2+2 = 4`), in the action panel.
- `render-widget("answer")` draws the tile from the payload, not from a value of its own, so
  a frame always shows what this query's `search` computed. A row holds two columns. On the
  left is the input, in the monospace font, over a caption: what was computed ("Expression",
  "Conversion"), or a note such as the zone used or the age of the rates. On the right is the
  result, large, over its kind in words ("Number", "Length", "Date and time"). That is seven
  nodes, and it renders in about a millisecond.
- The tile starts selected, so Enter on `2+2` copies `4`: the action arrives as a widget
  `action` event, a user action, and calc answers `copy-text`. Calc keeps no list row as a
  fallback.
- Anything that is not math, an engine error included, answers nothing, so search never
  shows an error row or tile. The search answers well inside the 100 ms deadline: the engine
  evaluates a 1 KiB query in microseconds.

**The scratchpad.** The "Calculator" command (`run` with `calc/calculator`, `run`) pushes a
`scratchpad-view` with id `sheet:<id>`, holding the current sheet's text and one annotation per
non-empty line. On every `text-changed`, calc evaluates the text and answers `update-view`
with the event's revision, so a late answer to an old revision is dropped. The pad's actions
(Copy Last Result, New Sheet, History) arrive as `action-chosen`.

**Never losing work.** Calc keeps its sheets in `storage`, which is the truth: every call reads
it again, so a reloaded plugin finds what the last one saved. The keys are `sheet:<id>` (the
text), `sheet-edited:<id>`, `sheets` (ids, newest first), `current` and `next-id`. Calc saves
on `text-changed` and on `closed` (neither needs a user action). Storage takes 20 writes per
second, so calc counts its own writes and keeps a reserve for `closed` and the actions (see
`src/sheets.rs`). Above about 12 edits per second, the newest text waits in memory for the next
event, for `closed` or for the next `search` (which the launcher sends on every keystroke).
A `search` makes only saves that need no sheet dropped (see below). Picking one to drop reads
every sheet, which does not belong under search's 100 ms deadline, so such a save waits for
the next call that is not a search.
A new sheet only takes an id; it is listed by its first save of text that is not blank, so
a sheet nobody typed in leaves nothing behind.

**Storage full.** A `set` refused for the quota fails with an error that starts with `quota: `.
Calc matches that prefix: it drops the oldest sheet that is neither current nor the one being
saved (an empty one first), tries again, and shows a toast that names every dropped sheet. The
100-sheet cap drops the same way, with the same toast. A sheet over 64 KiB is still computed,
but saved cut.

**History.** "Calculator History" pushes a `list-view` with id `history`, one row per sheet.
Opening a row makes that sheet current and pushes its pad. If that pad is already on the
stack under the history list, the launcher returns to it instead of stacking a second one.

**Rates in the background.** Fiat and crypto rates come from `open.er-api.com` and
`api.coingecko.com`.
- `init` fetches the rates that are stale or missing and starts one repeating 10-minute timer.
  Each tick starts at most two fetches.
- `http-done` stores one key per feed, one `set` per answer, under the same write budget as
  the sheets. It never uses the reserve. If there is no room, or the storage quota refuses,
  the rates stay in memory: no sheet is dropped and no toast is shown. An answer over 8 KiB
  is refused unread.
- `search` only reads the rates in memory. It never waits for the network and never starts a
  fetch. With no rates, a currency query gets no answer.
- All of it stays far inside the 1 s per 10 s background budget.

**The time zone.** Calc reads the computer's time zone with `locale.local-time-zone`, on every
evaluation, so it follows a change. That zone is used for `now`, `today` and every moment
without a zone. The `timezone` preference (text, no default) overrides it. A user sets it in
`config.toml`:

```toml
[plugins.calc]
timezone = "Europe/Moscow"
```

It accepts an IANA name, a city or an abbreviation. Empty (or not set) means the computer's
zone. A value that names no zone falls back to `UTC`, with a warning in the launcher's log.

## Building and installing

See the [SDK README](../../crates/plugin-sdk/README.md):

```sh
cargo build --release --target wasm32-wasip2
```

Then copy `plugin.toml` and the built `.wasm`, renamed to `plugin.wasm`, into
`<plugins folder>/<id>/`. The launcher notices the folder, shows the user the permissions the
plugin asks for, and loads it once the user allows them.
