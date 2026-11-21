# Home edit mode: acceptance

Date: 2026-11-21. Branch `claude/nifty-mclaren-61c044` (Part 2 of the Home work; Part 1 is
the `home_file` edits and the "Add to Home" action).

## What was built

- **Ways in:** Ctrl/Cmd+K on a selected Home tile, then "Edit Home"; the "Edit Home" command in
  the search (with an empty Home: an error toast, "Home is empty: add widgets from the
  search"); right-click on a tile, then "Edit Home"; a long press (about 500 ms) on a Home tile.
- **In the mode:** the tiles jiggle (a dashed outline when the system asks for reduced
  motion), each has a remove badge, the search field is dimmed and typing is ignored, tiles
  draw but take no events.
  - Keys: ←/→ and Tab/Shift+Tab select; Shift+←/→ move; Delete/Backspace remove;
    Ctrl/Cmd+K opens the tile menu (sizes Quarter/Single/Double when more than one, the current
    one marked, then Remove); Ctrl/Cmd+Z restores removed tiles in turn while the mode lasts;
    Esc/Enter leave.
  - Mouse: drag to reorder, the badge removes, right-click opens the menu and a click on a
    menu row picks it, the strip's scrollbar scrolls, any other click outside the tiles
    leaves. Hiding the launcher leaves.
  - A toast after each removal: "Press [Ctrl] [Z] to restore widget", with key chips. It goes
    when the mode ends and when nothing is left to restore.
- **Every change is written to `config.toml` at once.** Only the Home list changes. Comments
  right above a removed entry come back on undo.

## Checked in tests

- The core's Home edits (`crates/core/tests/home_file.rs`): remove then undo is byte for byte
  for entries in the launcher's own form, CRLF files stay CRLF, inline arrays, comments handed
  back.
- The mode's pure functions (keys, moves, undo, drop position) and the GPUI tests of the mode
  (`crates/ui`): keyboard and mouse paths, and the five review points of the plan (an entry
  that cannot show between visible tiles; two identical tiles; the file edited by hand while
  the mode is open; a stray key does nothing; removing every tile, then leaving).
- `just check` (fmt, clippy for the workspace and two cross targets, the workspace tests, the
  guest crates) exits 0.

## Headless run

`just build`, then a daemon with its own throwaway `TOMAKAST_HOME` and `TOMAKAST_NO_KEYBOARD=1`
(the keyboard is not grabbed and no window is shown), with calc and weather installed and
approved and three Home entries (weather `now`, calc `answer`, weather `today`).

- `tomakast run home/edit-home` exited 0.
- The daemon's debug log has `effect without a window ignored effect="EditHome"`: a daemon
  run of the command has no window, so the effect is dropped, as designed.
- No errors from the launcher or the plugins. Two environmental lines appear: a
  `freedesktop_desktop_entry` "Unknown field code" error about some installed `.desktop` file,
  and a missing `.SystemUIFont` warning from GPUI.
- `config.toml` was unchanged afterwards.

The mode itself cannot be seen headless; the GPUI tests are its evidence. Not checked by eye:
the jiggle, the dashed outline and the drag feel. The checklist below is for that.

## Rulings

1. A removed entry's comment lines (right above it) are returned and undo puts them back.
   *Corrected in execution:* the plan said comments inside its table body travel with the
   table and remove then undo is byte for byte. In fact the undo writes the entry back in the
   launcher's own form (`widget`, then `size`, then its string parameters) with its comment
   lines above it; comments inside its body are not kept (see Known gaps). The file comes back
   byte for byte for an entry already in that form with no comment in its body.
2. Inline arrays, `Remove(0)`: the next element takes the removed one's place and indent,
   never its comments.
3. CRLF: if every `\n` is part of `\r\n`, the file is written with `\r\n` everywhere. A file
   with mixed endings is written as edited.
4. The write stays on the UI thread, with its fsync (median 1.1 ms, max 2.4 ms over 30 runs on
   NVMe btrfs). One write per user action: a drag writes once, on drop. Held Delete/Backspace,
   Ctrl/Cmd+Z, Enter, Esc and Ctrl/Cmd+K are ignored; held arrows repeat. A drag whose
   release is lost ends, with nothing written, on the next move without the button. A write over 16 ms is logged at `warn`.
5. The launcher's own write comes back from the file watcher; the core re-reads the file when
   that arrives, so a stale list never undoes a write. A write refused because the file
   changed re-reads it at once, so the strip shows the file and the next try works.
6. Selection is kept by Home index, not strip position: entries that cannot show sit between
   visible tiles.
7. "Edit Home" is an internal effect, honoured only as a reply to the open window and sent only
   by the built-in `home` module. A plugin cannot send it, and an action titled "Edit Home" is
   refused as "Add to Home" is.
8. Empty Home means the strip has no tile at the empty query, even if `config.toml` has
   entries that cannot show now.
9. A drop lands by the dragged tile's centre past the other tiles' centres.
10. Reduced motion: `TOMAKAST_REDUCED_MOTION=1` or `0` wins; on Linux KDE
    `AnimationDurationFactor=0` or GNOME `enable-animations` off (through the portal) means
    reduced; otherwise the tiles jiggle.
11. Toasts: "Home is empty" is an error; the undo hint is a success toast of 5 s, cleared
    when the mode ends or nothing is left to restore; a refused write because the file
    changed says "config.toml changed: try again".
12. Right-click on any strip tile opens its panel (in the mode: the tile menu). The panel's
    rows run by click as by Enter; a press on the panel never leaves the mode.
13. Leaving the mode puts the strip selection on the selected tile, or on the list when Home
    is empty.
14. A press on the strip's scrollbar track scrolls the strip and does not leave the mode.
15. The plugin ids `home` and `history` are reserved for the built-in modules.

## Known gaps and accepted limitations

- **Reduced motion on macOS and Windows is not read yet**; the tiles jiggle there.
- Comments in the body of a removed entry (inside its table) are lost if the user leaves the
  mode without undoing; a body comment before another key is lost on remove.
- Undoing two removals when a blank-line-separated comment becomes the first line of the file
  reorders that comment.
- Undo of an only entry in a CRLF file writes LF.
- Entries around another table or a malformed entry can come back shifted by one place.
- A file with mixed line endings is written with LF (through `toml_edit`).
- Undo exists only while the mode lasts.

## User checklist (result cells blank)

| Check | Result |
|---|---|
| Long press on a Home tile enters the mode; a shorter click does not | |
| Tiles jiggle; with reduced motion on, a dashed outline instead | |
| Drag a tile to a new place; `config.toml` shows the new order | |
| Badge removes; the toast offers Ctrl/Cmd+Z; Ctrl/Cmd+Z brings it back in place | |
| Shift+←/→, Delete, Ctrl/Cmd+K sizes work from the keyboard | |
| Esc, Enter, a click outside and hiding the launcher all leave | |
| "Edit Home" with an empty Home shows the toast and does not enter the mode | |
