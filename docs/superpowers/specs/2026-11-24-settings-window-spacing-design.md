# Settings window, round 7: one spacing grid

Status: from the user's note of 2026-11-24 ("the sidebar's spacing is not consistent with the
spacing inside a settings page"). A first build that widened the sidebar's spacing to the
page's was rejected ("go towards smaller spacing on the page rather than larger in the
sidebar"); this document records the second, built on the sidebar's values.

## What was wrong

- The sidebar's surfaces (search field, entries) stood 8 px from its edges; the page's (title,
  boxes) 28 px.
- The first sidebar entry started 8 px under the title area, the first box 24 px: the two
  columns began on different lines.
- The sidebar's gap between its sections was 20 px, the page's gap between groups 28 px.

## The grid: the sidebar's spacing

- **`SETTINGS_EDGE` = `ROW_GUTTER` (8).** From a column's edge to what stands in it, in both
  columns: the search field and the entries in the sidebar, the title and boxes on the page.
  Also from the title area down to the first entry and the first box, so they start on one
  line, and the space under the last of them.
- **`SETTINGS_GROUP_GAP` = 20.** Between the page's groups, as between the sidebar's sections.
- The sidebar is unchanged. Inside the surfaces nothing changes either: an entry's icon at
  8 px, a box row's text at 15 px.

## Follow-ups of the same round

- **The page title starts at the text's column**, where the groups' headings and the rows'
  titles do (`SETTINGS_TEXT_INSET` from the boxes' edge), not at the boxes' edge.
- **The pop-to-root setting is renamed** "Return back home", described as "How long to wait
  after the launcher is hidden before its last search is cleared." The key stays
  `[launcher] pop_to_root`.

- **The UI font defaults to the desktop's** (`[appearance] font` default `system`; when the
  desktop's font cannot be read, the bundled Inter). The list opens with **System UI font
  (family)**, which removes the key; every installed family follows alphabetically, Inter
  among them (listed even when only bundled), writing its name.
- **The monospace font keeps its default**, the bundled JetBrains Mono, but the list no longer
  puts it first: **System monospace font (family)** opens it (writing `system`), then every
  family alphabetically, JetBrains Mono among them, which removes the key.
- A file naming a default by name (`font = "system"`, `mono_font = "JetBrains Mono"`) stands
  on the default's entry.

## Approaches considered

- The page takes the sidebar's spacing (built).
- The sidebar takes the page's (first build, rejected: the sidebar looked loose).

## Unchanged

The title area's height and its controls; row and entry heights; control widths; the
launcher's own spacing.
