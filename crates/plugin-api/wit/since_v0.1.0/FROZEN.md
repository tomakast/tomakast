# WIT 0.1 is frozen

`plugin.wit` in this folder is the tomakast plugin contract, version 0.1. It was frozen on
2026-11-19. `FROZEN` holds its sha256 in `sha256sum` format, so `sha256sum -c FROZEN` run in
this folder checks it. `crates/plugin-api/tests/freeze.rs` fails when the hash does not match,
or when this folder holds anything besides `plugin.wit`, `FROZEN` and this file.

## Policy

This policy is provisional (Ruling 15 of plan 2b-2b) until the user confirms it.

- **The numbers are contract.** Deadlines, memory, storage quota, write and toast rates, size
  caps and the background budget are doc comments of `plugin.wit`. Plugins are written against
  them, so they are covered by the hash like every other line.
- **Relaxing a limit** (a longer deadline, a larger quota or cap, a higher rate) breaks no
  plugin that kept to the old one. It may be done in place: edit `plugin.wit`, re-hash `FROZEN`
  (`sha256sum plugin.wit > FROZEN`), and add a row to the record below with the reason. Do it
  deliberately and in its own commit.
- **Tightening a limit, or any change of shape** (a type, a function, a field, a case, a
  permission, a rule that a plugin must newly obey) needs a new version folder,
  `wit/since_v0.2.0/`, and host bindings for it next to those of 0.1.
- A doc comment that only corrects wording to match what the host already does is a re-hash
  too, recorded below. A doc comment that promises something new is a change of shape.
- Exception, 2026-11-19 (the user's ruling): the widgets change of shape was made in place,
  because no parallel session and no outside plugin depended on 0.1 yet.

## Record

Every re-hash after the freeze, newest last.

| Date | Hash (first 12) | Change | Reason |
|---|---|---|---|
| 2026-11-19 | `e057c33cfa14` | Frozen. | End of plan 2b-2b, Task 4. |
| 2026-11-19 | `e9e0077e6a6a` | The package doc says what WASI gives: the wall clock, the monotonic clock and random numbers, and nothing else. | Ruling 14 (final review G1): calc and other plugins rely on the clocks, and the contract only said what WASI does not give. Made before the merge. |
| 2026-11-19 | `1168025ee612` | Widgets: [[widgets]] in plugin.toml, search answers search-answer, render-widget, show-widgets / redraw-widget / set-widget-image, event widget, the frame and canvas records; the `locale.local-time-zone` import. | Edited in place: widgets, 2026-11-19, no consumers yet (the user's ruling). |
| 2026-11-19 | `4c6d12a2da2d` | `icon.named` is a Lucide icon name from the launcher's bundled set, the same on every OS; a name outside the set shows the item's monogram. | kitchen-sink feedback: the host drew nothing for a theme name, and an OS theme does not exist on macOS or Windows. WIT 0.1 is edited in place while no outside plugin depends on it (the user, 2026-11-19, calc feedback). |
| 2026-11-20 | `5d8208a01deb` | Merge of the two rows above (branch `plan-2b2b-sdk-calc` into `fb-calc`): the widgets change and the Lucide `icon.named` doc comment, both kept. Each row above gives the hash of its own branch; this is the hash of the merged file. Then, in place: the `node` doc says nodes nest at most 32 deep and how an overlay is sized (widgets Task 9); the canvas docs say how it draws (line box, a text size of at most 200, non-zero fills, centred strokes, the gradient box, emoji opacity) and that a slow canvas is stopped (Task 10); the `widget-image` doc lists the SVG cost caps (32 primitives applied in all, 16 raster images and 4 Mpx, 10 turbulence octaves, morphology radius 32 device pixels, convolution order 9, no font fallback). Then, in place too (widgets Task 15): `icon.named` says what the launcher draws for a name outside the Lucide set (the desktop's icon theme on Linux, else the monogram), and the `effect` doc says that an answer to a widget event that is not a user action has its `push-view`, `pop-view` and `close-window` dropped. | Both branches edited WIT 0.1 in place (the user's rulings above); merged by keeping both changes. The node depth: widgets Task 9 review (a deep frame overflowed the launcher's stack). The canvas notes: Task 10 review (the docs did not say how the canvas draws; a canvas at the caps can paint for tens of ms). The SVG caps: widgets Task 11 review (a document within the old caps could rasterize for minutes); this row updated, no new row (the controller's ruling). The Task 15 wording: the doc said a name outside the set shows the monogram everywhere, and that such an answer's `push-view` was honoured as unsolicited; the launcher already did otherwise (the Linux theme-icon fallback, kept by the user's ruling; the host drops navigation that no user action asked for). |
