# Settings window, round 5: polish

Status: from the user's review of 2026-11-24 (screenshots of the General launcher hotkey row and
the Hotkeys page's Weather group). Extends the round 4 design
(`2026-11-23-settings-window-round4-design.md`); where they disagree, this document wins.
Rulings marked **Ruling** are the session's calls, reported to the user, not asked.

## Intent

Round 4 got the structure right; the user now judges the finish and finds it rough: a hotkey
field that looks broken, a Hotkeys page with no hierarchy, oversized window controls, a dark
dropdown that darkens on hover, a drag that moves a tiny handle, a page that scrolls in jerks.
The bar is "a sweet": every page, both themes, looks deliberate, aligned and calm, as a native
settings app would.

## The points and what is built

1. **The hotkey field looks like a field.** One look everywhere (General's launcher hotkey, the
   Hotkeys table, a plugin's Commands table), the same box as the window's text fields: their
   fill, border, radius and height (`CONTROL_HEIGHT`). Inside, content starts at the text
   fields' inner padding, left-aligned: chips when it has a hotkey, "Add hotkey" in the muted
   colour when empty, "Press keys…" while recording. The status icon (after a restart, off,
   refused) is no longer a reserved slot at the left: when there is one, it sits at the right,
   before the ×; when there is none, nothing takes its room. The width is fixed (nothing moves
   when it fills or records) and is the one width every one-line control of the window has
   (see "One width" below). Recording keeps today's ring.
2. **Columns line up.** In a table, the "Hotkey" column title starts exactly where the text
   inside the hotkey fields starts (field left edge + inner padding). **Ruling:** the "Name"
   column title starts at the table's leftmost content: on the Hotkeys table, where the group
   rows' chevrons start (as in Raycast); on a table without group rows (a plugin's Commands),
   where the rows' icons start.
3. **The Hotkeys page is one table, as in Raycast.** One box holds the whole page: the column
   titles once at the top, then for each source a group row (chevron, the source's icon, its
   name in semibold, the count muted), then its items indented under it so an item's icon
   lines up with the group's title text. Item titles are regular weight in the text colour;
   group rows have no fill of their own (hover tint only) and a little more height than item
   rows; rows are split by the hairline dividers the other boxes use. No per-group boxes and
   no repeated column titles. Open/closed, the count, search revealing a closed group, Enter
   and Space on a group row, lazily made fields, bound items first: all as in round 4. The
   "Not used" entries are the last group of the same table. A plugin page's Commands table is
   the same table without group rows.
4. **Applications is open when the page is first shown**, whatever it holds. Other groups
   keep the round 4 rule (open when they hold a hotkey).
5. **The Hotkeys page scrolls smoothly.** Scrolling with Applications open (100+ rows) runs at
   the display's frame rate on this machine in the dev build: no rebuilding of the whole
   hotkeys model, of every page or of sorted item lists on each frame; work is redone only when
   what it depends on changes (the index, the file, the hotkey table, the query, open groups).
   App icons are decoded once. A frame of that page renders well under a frame's budget; the
   report gives the measured time before and after.
6. **Window controls are smaller.** On KDE, Breeze's proportions, a
   22 px button (the hover circle), the 18-unit glyph box inside it, 6 px between buttons,
   centred vertically in the title area, the right margin equal to the gap from the title
   area's top to the buttons. GNOME and Windows styles are unchanged.
7. **Dropdowns lighten on hover in the dark theme.** Every dropdown button (size, choices, time
   zone, languages) and every other control button in the window: hover adds the window's
   hover tint over the control's own fill (lighter on dark, darker on light), never a darker
   kit shade. Pressed is a step further the same way.
8. **A widget is dragged whole.** On the Widgets page, pressing anywhere on a widget's line
   (outside its size dropdown and remove button) and dragging moves the widget; the drag image
   is the whole line, at its width, slightly lifted (shadow) and a little translucent, and the
   line it came from stays in place dimmed until the drop. The grip stays as the hint. The
   drop target line and one write per drop are unchanged.
9. **Icons follow the theme.** In the light theme, "Search Files" (and every other built-in
   item or module icon, in the settings window and in the launcher) is drawn dark on light,
   not light on light. Monochrome icons take the text colour of the theme; coloured app icons
   stay as they are.

## One width (after the user's review of the finished round)

The user found the hotkey field far too long (283 px, sized for four modifiers and the widest
key) and the one-line controls of uneven length (112 px short dropdowns next to 283 px fields).
Every one-line control of a settings row is now one width, 220 px: text and number fields,
every dropdown (a theme, a unit, a widget's size, the city, a time zone, a language, a
plugin's choice) and the hotkey field; the right column of a settings table is that width too,
so its "Hotkey" title still starts where the fields' text does. **Ruling:** 220 px, sized for
the common case: every combination of up to two modifiers and any key fits next to the ×,
and most of three. A longer combination is cut under a fade at the end of the keys' room and
read whole in the field's tooltip (and by assistive technology, as before). A dropdown's value
too long for the width ends in an ellipsis; its opened list is unchanged. This reverses round
5's "smallest that fits the longest combination" and "short dropdowns at least 112 px".

## Constraints

- Plain user-facing English; no internal names in the UI.
- Every click, key press or drag gets a GPUI test; layout claims (alignment, widths) get a
  bounds test. What stays unverified by eye is said in the report.
- Screenshots go to the user, never into the repository. No push, no merge into main.
- After the points, a polish pass: every page in both themes is captured and looked at; any
  misalignment, uneven spacing or inconsistent control found is fixed in the same round.
