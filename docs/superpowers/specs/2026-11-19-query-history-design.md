# Launcher query history (short design)

Date: 2026-11-19. Status: approved by the user (2026-11-19). Source: the calc feedback session.

**Why.** The user wants access to what they typed in the launcher before: all queries, not only the calculator's.

**Decided by the user.** Both ways:
- ↑ in an empty search field recalls earlier queries, like a shell;
- a "Search History" command lists them.

**Calls (mine).**

*What is recorded:*
- A query is recorded when an action runs from it. The `usage` table already stores the query with every run, so that part needs no new data.
- A non-empty query is also recorded when the launcher hides without a run, so `2+2` looked at and closed is kept. Queries of one character are skipped.
- These go to a new core table, `query_history(id, query, at_unix, modified_at)`, following the sync groundwork (stable ids, timestamps).
- The list is deduplicated (the newest use wins) and capped at 1000 entries.

*Privacy:*
- History is core data, never offered to plugins.
- It is a built-in module, not a plugin.
- A "Clear Search History" action empties it.

*↑ in an empty search field:*
- Each press shows the previous query in the field, selected so that typing replaces it; ↓ goes forward.
- Editing a recalled query makes it a new query.
- **The rule for ↑** (the user's call, revised on 2026-11-19, and again on 2026-11-20):
  - With an empty query, the first list row stays selected, as today, so Enter right after opening still runs the top suggestion.
  - *2026-11-20:* ↑ moves the selection up the screen: the list, then the widget strip, then the search field ("first I get to the widget, then to the field above"). The keyboard stays in the field throughout; only the selection moves.
  - ↑ on the first row, at any query, selects the strip's first tile, or the field when the strip has no tiles. ↑ on a tile, or on a control in it, selects the field. In the field nothing is highlighted.
  - ↑ in the field recalls history when the field is empty. With a typed (or edited) query it does nothing. Entering the field from the strip recalls nothing by itself.
  - *Superseded (2026-11-19):* ↑ on the first row of an empty query recalled history directly.
- **What follows from the rule** (my call):
  - While a recalled query is shown and not edited, the selection is in the field: ↑ keeps going back and ↓ goes forward. Past the newest entry the field is empty again, and the selection stays in the field.
  - ↓ in the field otherwise selects the strip's first tile, or the list's first row when the strip has none. ↓ in the strip goes to the list.
  - Enter and Ctrl/Cmd+K in the field act on the list's first row, and the footer shows its hint.
  - A recalled query whose condition tile answers (a recalled `2+2`) has the tile selected, as before. ↑ and ↓ still walk the history there, and Enter copies the result.
  - Typing puts the selection back on the list's first row (a selected condition tile keeps its selection while its new answer is on the way, as before).
  - Tab enters the strip from anywhere, including an empty query.
- **Clear and Delete** (the user's call): they erase only the history. The queries kept in `usage` for ranking stay.

*The "Search History" command:*
- A core-filtered list, newest first, each row showing the query and "2 h ago".
- Enter puts the query in the root search field and runs the search.
- Actions: Delete Entry, Clear Search History.

**Approaches.**
- *Chosen:* a built-in module with a core table.
- *Rejected:* a plugin with a new "read search history" permission. History is as sensitive as what the user types, and nothing a plugin adds is worth widening the permission surface for.
- *Rejected:* deriving the list only from `usage`. It would miss queries that ran nothing.

**Left to the builder.**
- The module's crate placement.
- How the recall is wired into the search field.
- Whether the empty-query suggestions show one or two recent queries. Off by default; that is a later UX call.
