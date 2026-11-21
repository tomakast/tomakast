# tomakast: widgets (tiles in the root view)

Date: 2026-11-19. Status: approved by the user (2026-11-19). Source: the calc feedback session (branch `fb-calc`).

> **Note (2026-11-21):** Home replaced the "always" set described here: a plugin no longer chooses tiles for the empty query (no `show-widgets`, no `[widgets] order/hidden`); the user's `[[home.widgets]]` list does, and a tile's `payload` became declared params. See `2026-11-21-home-widgets-design.md`.

## Why

Two wishes from the calc feedback met in one feature:

- The calculator's search answer should look like Raycast's: one wide card, the typed input on the left and the result on the right, not a plain list row.
- The user wants glanceable, live, interactive things the moment the launcher opens: weather (with weather effects drawn: rain, snow), currency and crypto rates, stocks, later a media control for what plays in the system.

A widget is a tile that a plugin draws and the core places in a horizontal strip in the root view, between the search field and the list.

## Goals

- **Where and when.** With an empty query, the strip shows the "always" tiles. While the user types, it shows the tiles whose condition matches the query (calc's answer for `2+2`; the weather tile for `weather`). An "always" tile that also matches the query stays visible.
- **Interactive.** A tile can be clicked and can hold controls. The keyboard moves the focus into the strip, between tiles and to the controls inside a tile.
- **Live.** A tile updates in real time, up to 60 frames per second, while it is on screen.
- **Free drawing.** One of the tile's components is a canvas on which the plugin draws anything: shapes, paths, gradients, text, images from bytes. Weather effects and a media player's cover art are drawn there.
- **The core owns the lifecycle.** A plugin never draws on its own: it implements a render function that the core calls when it wants a frame. Nothing is rendered while the launcher is hidden.
- **The open stays instant** (~50 ms, hotkey to frame): the strip never waits for a plugin.

## Non-goals

- The media player itself. In this design, "media player" means controlling what already plays in the system (MPRIS on Linux, Now Playing on macOS, SMTC on Windows), not playing media. That needs a platform import and a new permission and is a later feature. This design only has to make it possible (cover art from bytes, buttons, progress).
- Playing audio or video inside a tile.
- Free drawing in views (list, grid, detail, form, scratchpad). Views keep UI model A.
- A settings UI for widgets (the settings TOML is enough here).
- Core-side declarative animations (the plugin describes motion once and the core moves it). This can be added later on top of this design if the per-frame calls cost too much.
- GPU shaders from plugins.

## Anti-goals (failure, even if it "works")

- Opening the launcher gets slower because of tiles.
- Any CPU or GPU work for tiles while the launcher is hidden.
- A slow, hung or malicious widget makes the UI stutter or hang.
- A tile draws outside its frame, or over the core's own UI.
- A core prompt (permission review, confirmations) can appear inside the strip, where a tile could imitate it.

## Decided by the user

- Empty query: the "always" tiles. Typed query: the tiles whose condition matches, an "always" tile included when it matches.
- Tiles are clickable and interactive; the keyboard can focus a tile and the controls inside it.
- Real-time updates, capped at **60 per second** per tile.
- **No source label** on tiles (the plugin's name is not drawn on the tile).
- Free drawing, including images: **one of the components is a canvas** on which anything can be drawn. The canvas has native commands plus an **SVG layer** (option b of the canvas vocabulary).
- The media player is system playback control, and comes later.
- Calc's answer is **one double-width tile**: input on the left, result on the right.
- **The core controls the whole lifecycle.** The plugin implements a render function; the core calls it. The plugin has no way to keep rendering in the background.
- **The strip's look:** no scrollbar unless the strip is actively scrolling. When there are more tiles than fit, the left edge of the next tile peeks in from the right: it sits in the window's right margin, and tiles keep a minimal gap between them.
- **The contract is changed in place.** WIT 0.1 is edited directly (no `since_v0.2.0`), because no parallel session and no outside plugin depend on it yet. `FROZEN` is re-hashed and `FROZEN.md` records the change and its reason.

## Constraints

- **The WIT has no recursive types.** A component tree cannot be a recursive variant: it is sent as a flat list of nodes where each node names its parent by index.
- **The UI thread never waits on a plugin** (core spec). Render calls run on the plugin worker like every other call; the UI shows the last frame it has and swaps in a new one when it arrives. A frame for an older request is dropped.
- **GPUI** (`gpui-pre 0.3.12`, pinned) draws through `canvas()`: quads, paths, glyph runs and images, on Linux, macOS and Windows. Its limits shape the canvas vocabulary (builder check):
  - solid and linear gradients only, no radial;
  - clips are axis-aligned rectangles only;
  - quads, paths, glyphs and images take no transform (only SVG does), so rotated text and images are not native;
  - group opacity is crate-private, so opacity can only be per primitive.

  GPUI stays inside `tomakast-ui`; plugin-facing types stay GPUI-free.
- **The existing limits apply** unless this design says otherwise: deadlines are per call, effect sizes are capped, unsolicited effects are rate-limited, and the background budget covers timer and HTTP work. `docs/plugins/writing-a-plugin.md` has the table.
- Linux (KDE Wayland) and macOS first-class, Windows second.

## Approaches

How a tile's content is described:

1. **Components with a canvas (chosen).** A tile is a small tree of core components: row, column, overlay, text, icon, image, button, progress, spacer, and **canvas**. A canvas carries a list of drawing commands that the core paints with GPUI. Text is shaped by the system's and the theme's fonts, vectors stay sharp at any scale, and a frame is small even at 60 frames per second. The cost is a drawing vocabulary to define and keep. The user chose this ("one of the components should be a canvas").
2. **Pixel buffers.** The plugin rasterizes the tile itself and sends RGBA. The contract is tiny and anything is possible, but a HiDPI tile at 60 frames per second is about 45 MB/s across the WASM boundary, the plugin must bundle fonts (heavy wasm, text unlike the system's), and the image blurs when scaled. Rejected.
3. **Data templates only (model A for tiles).** The core offers fixed tile layouts ("big value", "input → result", "value + sparkline"). This is consistent and safe, but it cannot draw weather effects or a player. The user rejected it.

**The canvas's vocabulary**, after the builder check found what the pinned GPUI cannot draw (radial gradients, path clips, rotated text and images, group opacity):

- a. Only what GPUI paints natively. Fast and sharp, but those gaps stay gaps.
- b. **The native commands plus an SVG layer (chosen by the user).** SVG documents are rasterized by resvg, which GPUI already uses, and the raster is cached. It covers every gap at no new dependency, and fast animation stays native.
- c. The native commands plus images the plugin rasterizes itself in wasm. This stays possible through image bytes, but the cost and the fonts move into the plugin.
- d. Rasterize the whole canvas in the core with tiny-skia: CPU work every frame, and canvas text that differs from the UI's text.
- e. Upgrade GPUI, hoping a newer snapshot has the features.
- f. Fork GPUI. That goes against the "pin exact versions, upgrade deliberately" policy.

Core-side animation (option 1 plus keyframes the core runs) is kept as a later extension, see Non-goals.

## Design

### The strip

- It sits between the search field and the list in the root view only (not on top of a pushed view).
- **Width.** The content width `W` is the window width minus the root view's side margins. A quarter tile is `(W - 3 * gap) / 4` wide, a single tile `(W - gap) / 2`, a double tile `W`: four quarters, two singles or one double fill `W`, and two quarters with the gap between them are one single. (The quarter was added on 2026-11-20 for glanceable tiles such as weather; it keeps the strip's one height.) The strip's scroll viewport reaches the window's right edge, so the next tile's left edge shows in the right margin. Builder's call: the same at the left edge once the strip is scrolled.
- **Height.** One fixed height for every tile; the exact value is the builder's call (about 120–140 px, matched to the row rhythm).
- **Scroll.** Horizontal, by wheel or trackpad, and following the keyboard focus. The scrollbar is an overlay that shows only while the strip scrolls.
- **Empty strip.** No tiles, no strip: the list starts right under the search field, as today.
- **Order.** Matching condition tiles first, in plugin answer order (builder's note: the condition tiles of several plugins go by plugin id, each plugin's in its answer order). Then the "always" tiles, in the order of the settings TOML (`[widgets] order`); tiles not listed there follow in plugin approval order. `[widgets] hidden` hides tiles. Tiles are named `<plugin-id>/<widget-id>` there.

### Keyboard and pointer

- **↑ and ↓ move the selection over the screen** (the user's rule, revised 2026-11-20, see `2026-11-19-query-history-design.md`). The keyboard stays in the search field; only the selection moves. ↑ goes from the list's first row to the strip's first tile (or to the field when there are no tiles), at any query. From a tile, or from a control inside it, ↑ goes to the field, where nothing is highlighted. ↑ in the empty field recalls query history; with a typed query it does nothing. ↓ walks back: the field, then the strip's first tile, then the list's first row. ↓ in the field walks forward through a recalled query's history. Enter and Ctrl/Cmd+K in the field act on the list's first row.
- **From the list to the strip:** Tab from the search field, at any query, or ↑ as above. The first row stays selected at an empty query, as today. ↓ or Esc returns to the list. Typing always goes to the search field, and puts the selection back on the list's first row.
- **In the strip:** ←/→ move between tiles. Enter runs the tile's primary action. Ctrl/Cmd+K shows the tile's actions in the action panel. Tab moves through the focusable components inside the tile (buttons, and a canvas marked focusable). Shift+Tab goes back.
- **Inside a focused canvas,** keys go to the plugin, except Tab, Shift+Tab and Esc.
- **When a condition tile is present** for the typed query, it starts *selected*, but the keyboard stays in the search field (Raycast's behavior; my call). Typing, ←/→ (the caret) and Esc (hide) work as today. Only Enter and Ctrl/Cmd+K act on the selected tile: `2+2`, Enter copies `4`. ↓ moves the selection to the list, and ↑ to the field. The strip's own key map (←/→ between tiles, Tab inside a tile) is active only after the focus was moved into the strip explicitly (Tab, or ↑ as above). With an empty query the first list row keeps the selection, and there is no condition tile.
- **Pointer:** a click on a button runs its action, and a click on a canvas reaches the plugin with coordinates. Hover enter, move and leave over a canvas reach the plugin, coalesced to at most one move per frame.

### Lifecycle (core-driven)

- **The manifest declares the widgets:** `[[widgets]]` with `id`, `title` (for settings and the permission prompt) and `size` (`quarter`, `single` or `double`).
- **The "always" set.** At first, a plugin has no "always" tiles. It sets them with an effect, `show-widgets(ids)`, allowed in answer to `init`, `run` and any `on-event` (every call that returns effects). This lets a media tile appear only while something plays.
- **Condition tiles.** `search(query)` answers items and *widget answers* together. A widget answer names the widget id, carries a payload string (for calc, the result) and the tile's actions, like an item's. An answered tile is shown for that query. Its actions come back with that payload (my call), so what Enter copies is what `search` computed for this query, never a value from an older frame.
- **Swapping a tile atomically** (my call). The core asks for the first frame of a newly answered tile right after `search`, on the same plugin worker. The tile, its payload and its actions appear together when that frame arrives: until then the strip shows what it showed before. A tile the new answer no longer names disappears with the new answer. Enter and Ctrl/Cmd+K always act on what is on screen.
- **Render.** The new export `render-widget(id, context) -> frame` is called by the core only. It is called:
  - when the tile comes on screen;
  - after the plugin handled one of the tile's events;
  - after the plugin's effect `redraw-widget(id)` (in answer to `init`, `run` or any `on-event`: a timer, an HTTP answer, a widget event);
  - every frame while the last frame asked for one (`animate`);
  - on a size, scale or theme change.

  It is never called while the tile is off screen or the launcher is hidden. At most 60 renders per second per tile; a `redraw-widget` beyond that is merged into the next frame.
- **Shown and hidden.** `widget-shown` and `widget-hidden` events tell the plugin when a tile comes on screen and goes off it (scrolled away, query changed, launcher hidden). A plugin may use them to start or stop data timers. Rendering needs nothing from it: the core simply stops calling.
- **The context** passed to `render-widget`: the tile's size in logical pixels and the scale factor; the theme (dark or light, the core palette's colors, the UI and monospace font families, the text sizes); the time in ms since the tile was shown (for animation); the focused component and the pointer position, if any; the query, for a condition tile.
- **The frame:** the flat component list and `animate: bool`.
- **Instant open.** The core keeps the last frame of every "always" tile in memory and draws it at once when the launcher opens, then asks for fresh frames. A tile that has never rendered (the first open after the daemon starts) is drawn as an empty placeholder of its size. Builder's call: whether frames also persist across restarts.
- **Deadlines** (my call). `render-widget` has two:
  - a *soft* deadline of 16 ms, measured: a frame later than that is dropped and the last frame is kept; after repeated late frames (the number is the builder's), the core stops rendering that widget, shows an error toast and logs it;
  - a *hard* deadline of 100 ms, the existing epoch interrupt, like `search`: past it the plugin is broken by the existing rule, because an interrupted instance cannot be resumed.

  The WIT's package doc on deadlines gets this exception. Builder's note: the 5 ms epoch tick makes the hard deadline 100–110 ms. The soft deadline is measured, not interrupted.
- **Budget.** Render calls are core-started, so they count against no background budget. They are bounded by the 60-per-second cap, the deadline and on-screen visibility. The 30-per-second cap on unsolicited view updates does not apply to tiles, which are rendered by pull.

### Components

A frame is a list of nodes, each with an id (for focus and events), a parent index and a kind:

- **Layout:** `row`, `column` and `overlay` (children stacked on top of each other), with padding, gap, alignment and grow. Builder's call: the exact flexbox subset, kept small.
- **Content:**
  - `text`: a style (title, value, body, caption), a color (a palette token or `#rrggbb`), alignment and max lines;
  - `icon`: the existing `icon` variant;
  - `image`: an image id, see below;
  - `progress`: 0 to 1;
  - `spacer`.
- **Controls:** `button`, with a label and/or icon and an action id. It is focusable, and pressing it sends the plugin `widget-action`.
- **Canvas:** a size (fill or fixed), `focusable`, and a list of drawing commands in two layers (the user's choice, option 2 below). The commands are painted in order, so the layers interleave freely.
  - **Native commands**, what GPUI paints directly and fast, for anything animated:
    - rectangles (with corner radii), circles and ellipses, paths (move, line, quadratic, cubic, close), each filled or stroked;
    - solid colors and linear gradients;
    - text runs, not rotated (font family from the context or the theme's, size, color, position);
    - images by id, axis-aligned and scaled;
    - push and pop of a rectangular clip and of a transform (translate, scale, rotate). The core applies a transform to path and shape vertices on the CPU; text and images take only its translation and scale;
    - opacity per command (GPUI has no group opacity).
  - **SVG commands:** `svg(id, bounds)` draws an SVG document the plugin sent by id (like images). The core rasterizes it with resvg (already in GPUI's dependency tree, through its colored SVG image path) at the tile's scale, caches the raster, and rasterizes again only when the document, the bounds or the scale change. SVG covers what the native layer cannot: radial gradients, clip paths, group opacity, rotated text and images, filters.
    - Use it for rich art that changes rarely: a weather backdrop, a chart, cover art frames.
    - Fast motion (rain, snow) stays in the native layer on top.
    - *SVG safety:* no external references (no `href` to files or URLs, no external fonts; only `data:` images inside the document), no scripts (resvg has none). Caps on the document size, the node count and the filter region, and a raster time measured against the render deadline; a document over a cap is refused when sent. Builder's call: the exact caps, and whether text in SVG uses the system fonts through GPUI's fontdb or is refused.
  Coordinates are the canvas's logical pixels, and everything is clipped to the canvas and the tile.

**Images and SVG documents** are sent once, as bytes (PNG, JPEG or SVG) with an id, by an effect (`set-widget-image(id, bytes)`) or a frame field. Frames then name them by id, so 60 frames per second does not resend cover art. A plugin's images have a total size cap, and replacing an id frees the old bytes. This also closes WIT 0.2 candidate G7 (generated images) for tiles.

**Caps** (the builder sets the numbers; suggestions): 16 widgets per plugin; 256 nodes and 10 000 canvas commands per frame; node nesting ≤ 32; 1 MiB per frame; 8 MiB of images per plugin, 2 MiB per image. A frame that breaks a cap is dropped, as an effect breaking a cap is today, and logged.

### Events into `on-event`

The new event `widget(widget-id, widget-event)`:

- `shown` and `hidden`;
- `action(node-id, action-id)`: a button pressed, Enter on the tile's primary action, or an action from the action panel;
- `pointer`: down, up, move, enter and leave, with canvas coordinates and the node id;
- `key`: the key and its modifiers, only while a canvas has focus.

`action`, a completed click (a pointer `up` on the node that got the last `down`) and `key` are **user actions** (so `copy-text` and `open` are allowed in answer, and `push-view`, `pop-view` and `close-window` act as in answer to `run`). `shown`, `hidden`, a `down` alone, an `up` that completes no click, hover and move are not: their answer is unsolicited, and its `copy-text`, `push-view`, `pop-view` and `close-window` are dropped.

### Security

- A tile cannot draw outside its frame. The core draws the frame, the rounded corners and the focus ring itself.
- Core prompts and dialogs never appear inside the strip.
- Tile texts are data, like every other plugin string: they follow the same rules for control and formatting characters.
- A tile has no links of its own; opening goes through actions, which need the `open` permission and a user action.
- **Accepted risk (the user's call):** tiles carry no source label, so a tile can imitate a list row or other core UI inside its own frame. The permission prompt lists the plugin's widgets by title.
- **Permissions.** Showing tiles needs no permission (like `toast`). A condition tile comes from `search`, which already needs `search`.
- **Re-approval** (my call). The approval check treats the manifest's widgets like permissions: an update that adds a widget or changes a widget's title asks the user again. Because tiles carry no source label, a tile must never appear silently.

### Built-in modules

The core's internal module trait gets the same model: a module can supply "always" and condition tiles. Builder's call: whether a built-in tile is drawn from the same component list or as its own GPUI element (the trait already allows a module's own GPUI components). The future media control, a built-in with platform access, is the expected first user.

### Calc's answer tile

- A condition tile `answer`, `double`. The left half shows the input as calc understood it, in the monospace font. The right half shows the result, large, with its unit. Each half has a caption, like Raycast's: the left one names what was computed and the right one the result's kind or unit in words.
- Enter (the tile starts selected) and the action "Copy Result" copy the result. Ctrl/Cmd+K also offers "Copy Input + Result" (`2+2 = 4`).
- The tile replaces the `calc/answer` list item. Builder's call: whether the item stays as a fallback for a launcher that cannot show tiles; there is none today.
- `search` keeps its 100 ms deadline. Rendering the tile is a separate `render-widget` call.

## Contract changes (WIT 0.1, in place)

- **Manifest:** `[[widgets]]` (`id`, `title`, `size`); `docs/plugins/manifest.md` and the manifest freeze test are updated.
- **Exports:**
  - `search` answers a record of items and widget ids (instead of a plain list of items);
  - a new `render-widget(id, context) -> result<frame, string>`.
- **Effects:** `show-widgets`, `redraw-widget` and `set-widget-image` (PNG, JPEG or SVG bytes).
- **Events:** `widget(widget-id, widget-event)`.
- **Records and variants:** for the context, the frame, the components and the canvas commands, with caps in their doc comments like every other record.
- **Freeze records:** `FROZEN` is re-hashed; `FROZEN.md` records "edited in place: widgets, 2026-11-19, no consumers yet". `crates/plugin-host/tests/wit_coverage.rs` must cover every new case (kitchen-sink gets a widget). The SDK gets builders for frames and canvas commands, and the author guide gets a "Widgets" section.
- **Core spec updates:** the core spec's UI model section notes the exception (free drawing only inside tiles), and its anti-goal about inconsistent plugin UI notes that tiles are the deliberate exception.

## Scope of the first plan

My call:
- the framework (contract, host, core, strip, SDK builders, author guide);
- calc's answer tile;
- kitchen-sink widgets that cover every component, canvas command and event (the WIT coverage test needs them): a static tile, a tile with buttons, and a 60 fps animated canvas.

No weather, rates or stock plugin: those are later features built on this one. The acceptance measurements run on the kitchen-sink tiles (three "always" tiles, one of them the 60 fps canvas).

## Performance

- **Open:** cached frames are drawn on the first frame, and no plugin call sits on the open path.
- **Rendering:** only the tiles on screen are rendered (plus the peeking one, at the same rate). Two or three tiles at 60 frames per second, each render well under the deadline, fit inside the plugin worker. The UI thread only paints.
- **Hidden:** zero render calls and zero redraws. The existing "~0% CPU in the background" budget holds.
- **Acceptance measurements:**
  - the warm open with three "always" tiles, against the current number;
  - the CPU use with a 60 fps canvas tile on screen;
  - the CPU use with the launcher hidden (zero render calls).

## Left to the builder

- Exact WIT names and shapes within the above.
- Tile height, gap, margins and the left-edge peek.
- The flexbox subset and text styles.
- The cap numbers and the render deadline.
- How built-in modules supply tiles.
- Whether frames persist across daemon restarts.
- Whether calc keeps a list item as a fallback.
- The crate placement (core model types in `tomakast-core`, rendering in `tomakast-ui`, calls in `tomakast-plugin-host`).

## Related, not in this design

From the same feedback session, each with its own short design:

- calc conversions: units, dates and `now`, time zones, fiat (`open.er-api.com`) and crypto (CoinGecko) rates, number bases; English only;
- launcher query history: ↑ in an empty search, and a "Search History" command;
- theme icon names drawn on Linux (in progress).
