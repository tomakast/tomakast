# Settings window, round 4: behaviour and structure

Status: from the user's review of 2026-11-23 (eleven points, Russian, quoted by number below).
Extends `2026-11-22-settings-window-design.md` and `2026-11-23-settings-window-look-design.md`;
where they disagree, this document wins. Rulings marked **Ruling** are the session's calls,
reported to the user, not asked.

## Intent

The window looks settled; now it must behave like a native settings app and be organised the
way a user thinks: one place per setting, words the rest of the product uses, controls that
do not jump, lists that can be searched and dragged.

## The points and what is built

1. **Click on empty space takes the focus off a field.** The window root tracks a focus handle
   of its own; GPUI focuses the innermost focusable element under a click and stops there, so a
   field still takes its own clicks and a click anywhere else focuses the root (the field blurs:
   a text field commits as on blur; a recording hotkey field ends as on blur).
2. **The selected sidebar entry is not bold.** Same weight as the others; the selection fill
   alone marks it. No other selected state changes weight.
3. **Time zone: a searchable list, its default shown.** A new value kind `timezone` (manifest
   `kind = "timezone"`): a searchable dropdown of the IANA zone names (jiff's bundled
   database), the computer's zone shown as the value while the file has none. Nothing is
   written until the user picks. A value written by hand that is not in the list (calc accepts
   a city) shows as written. The calculator's `timezone` preference becomes this kind, its
   title "Time zone", no hint text.
   **Ruling:** shown, not written — like every other default in the window, and it keeps
   following the computer when its zone changes.
4. **Translator: two fields.** The `languages` text preference is replaced by `first`
   ("Primary language") and `second` ("Secondary language"), each a `choice` of Google's
   languages (the plugin's own list, `langs.rs`, written into `plugin.toml`; a plugin test
   keeps the two equal). A `choice` with more than 10 options is a searchable dropdown, for
   every plugin. What an unset one stands for comes from the plugin itself: a new effect
   `preference-default(key, value)` (WIT `effect` case; plugins only; the host keeps the last
   value per plugin and key and the settings window shows it as the value of an unset
   preference). The translator sends both from `init`, from `pair::automatic` over
   `locale.languages()`; an unknown second is not sent and the field shows "Automatic".
   **Ruling:** two fields, not chips (the user offered either; two fields reuse point 3's
   control). **Ruling:** an old `languages = "…"` key is not migrated; it shows under "Not
   used by this plugin" with Remove (the product has one user, on a dev build).
5. **Command hotkeys are recognisable as hotkeys.** A plugin page's "Commands" group is drawn
   as the Hotkeys page's table (point 8): a column header "Name · Hotkey", each command with its
   icon, its hotkey cell reading "Add hotkey" while it has none. "Not set" is gone everywhere
   for hotkeys.
6. **One section for the launcher's own pages.** Core and Modules merge into one sidebar
   section titled "tomakast": General, Appearance, Location, Hotkeys, Widgets, About (About
   last). Plugins stays its own section.
   **Ruling:** "tomakast" — "General" would repeat the page name, "Core" is an internal word.
7. **The launcher hotkey is on General only**, not on Hotkeys.
8. **Hotkeys is a table, as in Raycast.** Columns Name and Hotkey (tomakast has no aliases and
   no enable switches). Rows are grouped by where the item comes from: Applications first, then
   built-in modules, then plugins by name, each group a collapsible header with the source's
   icon and name. A group is open when it holds an item with a hotkey, closed otherwise; the
   window's search opens a closed group that holds a match. Every indexed item is listed, so
   "Add hotkey…" (the item search) goes. An item with a hotkey that is not indexed now is
   listed greyed in its source's group (or a group "Not available" when the source is unknown),
   still removable. `[hotkeys]` entries the launcher does not use stay as greyed rows with why
   and Remove, in a last group "Not used". Hotkey fields are created when their group opens,
   not on every render.
9. **The hotkey field never moves anything.** A fixed width that fits the longest combination,
   a fixed height; empty it reads "Add hotkey" (muted); recording it reads "Press keys…" in
   place, then the pressed chips. The recording hint (Enter keeps, Esc cancels) and a refusal
   are drawn in an overlay under the field, out of the row's flow. Clearing a hotkey is an ×
   inside the field (shown while it has one and is not recording), not a reset button that
   appears beside it. How the hotkey stands (applies after a restart, off, refused by the
   system) is an icon inside the field with the text as its tooltip and accessible
   description, not a line under it.
10. **Window controls look like KDE's.** On KDE (Breeze style): glyphs and button size matched
    to a real Breeze title bar on this machine (screenshot), sized for the 48 px title area:
    minimize a down chevron, maximize an up chevron, restore a diamond, close an ×; the hover
    circle as Breeze draws it, close red. GNOME and Windows styles are unchanged.
11. **Home becomes Widgets.** The page is "Widgets" (sidebar and title). One line per widget:
    a drag handle, the plugin's icon (as its sidebar entry derives it), the widget's title over
    the plugin's name and its params as a short summary, the size dropdown, and a remove
    button. Order changes by drag and drop (one write per drop, through the existing move
    edit); up/down buttons go. Params are not edited here (they come from Add to Home in the
    launcher) and are not shown as fields.
    "Tile" leaves every user-visible string and accessible name, in the settings window and in
    the launcher: it is a widget.
    **Ruling:** params as a summary, not fields — they were most of the clutter, and the
    launcher is where a widget is made.

## Constraints

- Plain user-facing English; no internal names in the UI.
- Every point that is a click, key press or drag gets a GPUI test that simulates it (no
  pointer tool exists here for checking by hand). What stays unverified is said in the report.
- Screenshots go to the user, never into the repository. No push, no merge into main.
- WIT 0.1 is edited in place (the freeze is lifted while no outside plugins exist); FROZEN.md
  records the new effect case.
