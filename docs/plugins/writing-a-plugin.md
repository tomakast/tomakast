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
- **Real plugins:** [`plugins/calc`](../../plugins/calc) and the smaller
  [`plugins/weather`](../../plugins/weather), explained at the end of this guide.

API 0.1 is frozen: `plugin.wit` and the manifest format do not change. A later API gets its own
folder (`wit/since_v0.2.0/`), and the launcher keeps running 0.1 plugins. (While no outside
plugin exists, the user may rule a change to 0.1 in place: the widgets on 2026-11-19, and
`readings` for `search` and `query-changed` on 2026-11-20, and `location` later that day;
`FROZEN.md` next to the WIT records every such edit.)

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
only way out: `storage`, `preferences`, `open`, `http`, `timers`, `log`, `locale` (the
computer's time zone) and `location` (where the computer is, approximately).

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
| `location` | The `location` import: the computer's approximate location. |

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
| `render-widget`, soft deadline | 16 ms of CPU time, measured around the call on the plugin's worker thread (wall time on a system without a thread clock); 64 ms in a debug build of the launcher. Time the thread waits for a core on a loaded machine does not count. | The frame is dropped and the tile keeps its last one. 5 late or failed frames in a row stop the widget, with an error toast and a log line: a Home tile's until the plugin is reloaded, a search tile's until the launcher is next opened. |
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
| Unsolicited `set-widget-image` | 30 per second | Dropped. |
| Renders of one tile | 60 per second | A `redraw-widget` beyond is merged into the next render. |
| Items in `replace-items` | 1000 | The answer is rejected. |
| Items from `search` | 50 | The answer is rejected. |
| Widget answers from `search` | 16, each naming a declared widget once, with params of the widget's declared keys, ≤ 4 KiB together, and one of its sizes | The answer is rejected. |
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

And nine effects:

| Effect | Does |
|---|---|
| `push-view` | Shows a view on top of the launcher's stack. |
| `update-view` | Replaces a view the plugin showed. |
| `pop-view` | Pops the plugin's view. |
| `close-window` | Hides the launcher. |
| `toast` | A short notice: success (3 s), error (5 s) or progress. |
| `copy-text` | Copies to the clipboard (`clipboard`, and a user action). |
| `replace-items` | Replaces the plugin's own items in the launcher search (`index`). |
| `redraw-widget` | Asks for a new `render-widget` of every tile of one of its widgets. |
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
[`manifest.md`](manifest.md#widgets)): an `id`, a `title`, its `sizes` and the `params` it
takes. A tile is 120 px high, whatever its size. In the 750 px panel a `quarter` tile is
173.5 px wide, a quarter of the strip, a `single` one 355 px, half of it, and a `double` one
718 px, all of it. Four quarters, two singles or one double fill the strip, and two quarters
with the gap between them are exactly one single, so tiles of all sizes mix in one row. A
quarter has room for a glance: an icon, one large value and a caption. Draw for the `width`
and `height` of the render context, not for these numbers. When more tiles than fit are
shown, the next one peeks in at the right edge, and the strip scrolls sideways.

**Params.** A widget's params are the strings that say what a tile is about: a city, a time
zone, an expression. The manifest declares their keys; a tile's values are strings, at most
4 KiB together. They come back in every render context and every event of the tile
(`param(&context.params, "city")` in the Rust SDK). Two tiles of one widget with the same
params are the same tile to the plugin. A widget that takes none is the same on every tile.

**Which tiles show.** A plugin cannot put a tile on Home, or keep one there. Tiles come from
two places:

- **Home** is the user's. It shows at an empty query, and is the `[[home.widgets]]` list in
  the user's `config.toml`, in that order: each entry names a `widget` (`<plugin-id>/<widget-id>`),
  an optional `size` (one of the widget's `sizes`, else the first) and the widget's params as
  string keys beside them:

  ```toml
  [[home.widgets]]
  widget = "weather/now"
  size = "quarter"

  [[home.widgets]]
  widget = "weather/now"
  city = "Barcelona"
  ```

  Home starts empty. The user builds it from the search. The edge rules:
  - An entry the launcher cannot show (its plugin is not loaded, or does not declare the
    widget) stays in the file and comes back once the plugin does.
  - A `size` the widget does not offer shows the widget's default size.
  - A param the widget does not declare is not passed to it, and stays in the file.
  - An old `[widgets]` table is ignored, with a warning in the log.
  - The launcher writes only its own entries ("Add to Home" adds one at the end of the
    list) and keeps the rest of the file as written: other tables, comments, and entries it
    cannot read stay where they are.
- **Search tiles** come from `search`. Its answer holds items and up to 16 *widget answers*.
  A widget answer names a widget, optionally a size, the params the tile is about (the
  Rust builder's `.param(key, value)`) and the tile's actions, as an item's. The tile shows
  for that query: the tiles of several plugins by plugin id, then each
  plugin's in its answer order. The params come back in every render context and every event
  of the tile, so the frames and the actions use what `search` computed for this query. A new
  tile appears with its first frame, its params and its actions together. A tile from an
  older answer may stay on screen until the plugin's next answer (a failed search counts as
  an empty one), but Enter and the actions never act on it. Hiding the launcher drops the
  search tiles: it opens with Home. A user action on a search tile records its query in the
  launcher's search history, as running an item does. A plugin reloaded while the launcher is
  open is asked the current query again once its `init` returns, so its tile comes back.

**"Add to Home".** Every search tile has the launcher's own action "Add to Home" in
Ctrl/Cmd+K, after the plugin's actions. It writes the tile's widget, size and params to
`[[home.widgets]]`, so Barcelona's weather stays Barcelona's. The plugin does nothing for it,
and cannot imitate it: an action of the plugin titled "Add to Home" (in any case or spacing)
is invalid, and the answer holding it is rejected.

**The `home` event.** The plugin learns which of its own tiles are on Home from the
`home` event: the list of its widgets and params, each once, in Home order. It comes after
`init` and whenever Home changes, only the latest list is kept, and it is a background event:
it is not a user action, and runs when the launcher is hidden too. A plugin keeps its data
fresh for them there, for example fetching the weather of each city on Home, and forgets a
city no tile names any more.

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
focused node and the pointer; the tile's params; and, for a search tile, the query.
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
- The frame's `actions` are a Home tile's (Enter runs the first, Ctrl/Cmd+K shows them
  all); a search tile's come from its answer. `animate` asks for the next frame. `images`
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

`on-event` gets `widget` events, with the widget id, the tile's params, and one
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

[[widgets.params]]
key = "result"
title = "Result"

[[widgets.params]]
key = "input"
title = "Input"
```

A command's `icon` names a Lucide icon (lucide.dev), which the launcher bundles and draws the
same on every OS: `calculator` and `file-clock` are both in the set. A name outside the set is
looked up in the desktop's icon theme on Linux (then `hicolor`); a name found in neither, and
any such name on macOS and Windows, shows the plugin's monogram.

**Search and its tile** (`src/lib.rs`, `src/views.rs`). `search` evaluates the query with
calc's own engine (`src/engine/`, plain Rust, tested natively). When the query is clearly
math, it answers no item and one widget answer: the double tile `answer`, like Raycast's.
- The tile's params are `result` and `input` (`4` and `2+2`); on Home the tile evaluates its `input` again each time it draws, so a date or a rate is not stale. The actions are "Copy
  Result" (`copy`, the first, so Enter runs it) and "Copy Input + Result" (`copy-both`,
  `2+2 = 4`), in the action panel.
- `render-widget("answer")` draws the tile from its params, not from a value of its own, so
  a search tile's frame shows what this query's `search` computed. A row holds two columns. On the
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

## Location

`location.request()` asks where the computer is; it needs `location = true`. The answer
comes later, as `event.location`: the coordinates rounded to 0.1° with `accuracy-km`, how far
the real place may be (never below 11), or why there are none. The launcher tries, in turn:

1. the city the user set in `config.toml`, `[location] city = "…"`, from the built-in list
   (when set, nothing else is tried);
2. the operating system's location service: the XDG desktop portal on Linux (GeoClue behind
   it, which must be installed and turned on), Core Location on macOS, the Windows location
   service on Windows; the desktop may ask the user first;
3. on Linux, only with `[location] wifi_lookup = true`: the Wi-Fi networks around, looked up
   in BeaconDB (their addresses and the computer's IP go there; its IP estimate is turned
   off, and refused if it comes);
4. the main place of the computer's time zone, `accuracy-km` 500.

A plugin cannot tell the sources apart, and needs nothing more than the permission for any;
an answer coarser than about 50 km is worth marking (the weather tile says `≈ Madrid, ES`).
The launcher finds the location at most once at a time for all plugins and keeps the answer
for 10 minutes (a coarser one, or a failure, for 1), so asking again costs nothing; a
request while one of the plugin's waits is answered by the same event. When the user
changes the `[location]` settings, a plugin that asked before gets the new answer unasked.
The event is a background event, not a user action. `just locate` tries the sources from a
terminal (all but Wi-Fi) and prints what plugins would get; `just locate-wifi` adds the
Wi-Fi lookup.

## A smaller example: weather

[`plugins/weather`](../../plugins/weather) shows the weather from Open-Meteo, a free API
that needs no key, in ten widgets the user puts on Home. The plugin puts nothing there. Every
widget takes the param `city`, and the sizes are `quarter` and `single`, the first being the
default:

- `now`: the condition's icon and word, the temperature and the place; also a `single`.
- `today`: the four parts of the day side by side; also a `double` that adds the day's high
  and low, sunrise, sunset, the feel, the wind and the rain.
- `humidity`, `wind`, `uv`, `pressure`, `precipitation`, `air-quality`, `pollen`, `daylight`:
  the value now, a small chart of the day, and in the `single` a detail line.

It is a pattern for any tile fed by the network:

- **The manifest** asks for `http` to three Open-Meteo hosts (the forecast, the geocoder and
  the air quality), `timers`, `storage`, `search`, `clipboard` and `location`. The air
  quality host is new in this version, so a launcher that approved an older weather asks
  again before it loads. `fahrenheit` switches to °F and mph. `mmhg` shows pressure in
  mm Hg; it only changes the drawing and never fetches.
- **Where a tile looks.** A tile with a `city` shows that city. The param is a city of the
  built-in list (`crates/places/src/cities.tsv`, shared with the launcher: about 6,300
  cities of 100,000 people or more from GeoNames, CC BY 4.0, built by
  `crates/places/tools/cities.py`), `latitude, longitude`, or a name to geocode, kept and
  shown as typed. Absent, blank or `auto` means where the computer is. That is asked with
  `location.request`, only while such a tile is on Home or is the search's tile, then every
  3 hours, and half an hour after it could not be told or was told only coarsely. It is
  named after the most populous city of the list within 20 km (a city's districts are on
  the list too, and a rounded position may fall in one), else the nearest within 50 km,
  with `≈` before it when the location is coarser than 50 km; without a location the tile
  says "No location".
- **The search.** Each widget has words, in English and Russian, with or without a city:
  `wind in Berlin`, `berlin wind`, `ветер в Берлин`, `uv index`, `weather today`. The table
  is in `src/search.rs`. A bare query that is a city of the list is its `now`. Matching
  against the list in the plugin, not against the geocoder, keeps what the user types on the
  computer: nothing is sent until the query names a city or asks for a widget. The search
  tile shows the widget's default size, and its `city` names the place (`Berlin, DE`), so a
  late answer or Enter finds its own city, and "Add to Home" keeps it. Its place is fetched
  once the query has rested 300 ms (typing `San Francisco` passes `San`, a city in Mali),
  and a word still being typed (`uv ind`, `weather tod`) asks for nothing. The search tile
  going off screen stops keeping its place fresh; coming back starts it again. Enter copies
  the weather in one line.
- **The day** (`src/day.rs`) runs from 06:00 to 06:00 in the place's own time: morning 6 to
  12, afternoon 12 to 18, evening 18 to 24, night 0 to 6 of the next date. At 02:00 the day
  is still yesterday's. "Now" at a place comes from the answer's own time, not from the
  computer's clock or time zone. A part already over is drawn faded.
- **One store of places** (`src/places.rs`). Every place has one forecast request and, only
  while an `air-quality` or `pollen` tile shows it, one air request, however many tiles name
  it and however they spell it: places are kept by their coordinates. A forecast 15 minutes
  old is fetched again; a failure waits 1, 2, 4, 8, then 15 minutes. At most four requests
  are out at once, and a request the host refuses for that reason is not a failure, it waits
  for the next turn. A one-minute timer, a tile coming on screen and a change of units look
  whether a fetch is due. Drawing never fetches.
- **Home keeps its places fresh.** The `home` event lists the plugin's Home tiles and is
  sent while the launcher is hidden too, so those places are refreshed every 15 minutes
  while it is hidden and a tile is current when it shows. A place no tile names any more is
  forgotten, in memory and in storage, unless the search or the location still names it.
  Places only searched stay in a small cache. With an empty Home and no search tile the
  plugin sends nothing.
- **Storage is per place:** `forecast:<key>` and `air:<key>`, where `<key>` is the city's
  key, or `@here` for the location, for the places of Home only: a place only searched, the
  location included, stays in memory. A value is a few header lines and the raw answer, read
  back by the same parser as a fresh one, so a restarted launcher draws at once. The key
  `last` of older versions is removed at `init`.
- **Faint icons are SVG on a canvas.** An icon node always takes the theme's text colour,
  and nodes have no opacity; only canvas commands do. So a faded icon is a Lucide SVG the
  plugin ships (`icons/`, ISC), coloured with the theme's text colour and drawn with
  `Draw::svg(..).opacity(..)`, the SVG sent in the frame (`src/ink.rs`). A bright icon stays
  an icon node. Any plugin that needs a faded or muted icon can copy this.
- **Calm failures.** No network, a timeout (the host's 30 s), a bad answer: the tiles keep
  the last reading and say how old it is, or say why there is none ("Loading", "No
  location", "Unknown city", "No weather"). Enter on a tile, or the "Refresh Weather"
  command, tries at once and asks again for a city the geocoder did not know. There is no
  toast, and the log has one warning per run of failures.
- **Drawing** (`src/tile.rs`, `src/today.rs`, `src/measures.rs`) reads memory only. An
  answer that changes what the tiles show sends `redraw-widget` for every widget.

## Building and installing

See the [SDK README](../../crates/plugin-sdk/README.md):

```sh
cargo build --release --target wasm32-wasip2
```

Then copy `plugin.toml` and the built `.wasm`, renamed to `plugin.wasm`, into
`<plugins folder>/<id>/`. The launcher notices the folder, shows the user the permissions the
plugin asks for, and loads it once the user allows them.
