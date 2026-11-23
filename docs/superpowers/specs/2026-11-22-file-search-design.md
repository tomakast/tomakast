# File search

Status: approved by the user, 2026-11-22. A built-in module, not a plugin.

## Intent

The user wants the launcher to "search apps, commands and files with smart ranking"
(backlog, 2026-11-18). Files must feel like part of the same launcher: one field, the same
fuzzy matching, the same keyboard-layout and transliteration tolerance, the same learning
from what the user opens, and no loss of the instant feel. The user decided the shape
(2026-11-22):

- File results are **mixed into the root search**, and a setting turns the mixing off.
- A **"Search Files" command** opens a screen that searches files only.
- That screen has a **list / grid toggle**; the grid shows **thumbnails**.

## Goals

- Find any file or folder under the covered folders by name, typed fuzzily, on the wrong
  layout or in another script, the same way apps are found today.
- Files in the root list help without crowding apps and commands out.
- Files the user opens rise in the ranking, like apps.
- Freshness: a file created or renamed a moment ago is found.
- Actions on a file: open, show in the file manager, copy path, open with….

## Non-goals

- Searching inside file contents (full-text). Not now; a system index could add it later.
- Searching the whole disk (`/usr`, `/etc`, system folders) by default. The user can add
  folders.
- File operations beyond the four actions (move, rename, delete, trash).
- A persisted on-disk index (see "Index freshness"; revisit only if the cold-start
  measurement says so).
- Changing the plugin interface (WIT). Everything new here is for built-in modules only.

## Anti-goals (failure even if it works)

- Typing in the root field gets slower, or the window opens slower.
- A query for an app shows a file the user never opened above the app. `fire` must still
  show Firefox first when `~/fire.txt` merely exists; a file the user opens by habit for
  that query may lead (see "Files in the root list").
- The root list fills with junk paths (build artefacts, caches, `node_modules`).
- The machine gets busy in the background (constant rescans, fans).

## Constraints

- Core (`crates/core`) runs on the UI thread; a module runs on its own worker and answers
  with effects. Per-keystroke budget on the UI thread: ≤ 5 ms for the static match (core
  spec). The open budget is ~50 ms.
- Dynamic sources answer within `DYNAMIC_SEARCH_DEADLINE` (100 ms); a superseded search is
  skipped by the host before it runs.
- Scale on this machine: about 470k non-hidden, non-ignored paths in the home folder
  (1.6M with hidden ones), 44k folders. A full walk with `fd` takes ~0.13 s warm.
- `max_user_watches` is 524288 here; other programs share it.
- Item ids are at most 256 bytes and may not contain control characters; file paths can
  break both rules.
- Linux (KDE Plasma, Wayland) and macOS are first-class, Windows second. Today the `apps`
  module is Linux-only with stubs elsewhere.
- Other sessions are changing shared code (a translator plugin, the settings GUI). Changes to
  core and UI stay small and focused.

## Approaches

1. **Own index (chosen).** The module walks the covered folders, keeps the paths in
   memory and watches for changes. The same on all three systems, no outside dependency,
   and the launcher's matcher (typos, layouts, transliteration, usage) applies to files as
   is. The walk is cheap (see above). Cost: names and paths only, no content search.
2. **System index** (Baloo, Spotlight, Windows Search). Gives content search, but Baloo is
   disabled on the user's machine, quality differs per system, and wrong-layout and
   transliterated queries do not work through it.
3. **Hybrid** (own index plus Spotlight on macOS). Double work for content search on one
   system; can be added later as an extra source if wanted.

The user picked approach 1 with the defaults below (2026-11-22).

## Design

### The module

A new built-in module `modules/files` (crate `tomakast-module-files`), provider id `files`,
registered in the daemon next to `apps`. It is a dynamic source: it answers root searches
from its own index. It also publishes one static item, the "Search Files" command.

### What is indexed

- **Roots:** the home folder by default. On macOS `~/Library` and on Windows `AppData` are
  excluded by default.
- **Skipped by default:** hidden files and folders (dot-names on Unix; the hidden attribute
  on Windows); anything ignored by `.ignore` files, and inside a git repository by its
  `.gitignore` files, `.git/info/exclude` and the global gitignore (as `fd` does); folders
  named `node_modules`, `target`, `__pycache__`, `.venv`, `venv`. Symbolic links are listed
  but not followed. Other file systems mounted under a root (network shares, FUSE) are not
  crossed.
- **Folders are results too**, not only files.
- **Settings** in `config.toml`, a new `[files]` table:

  ```toml
  [files]
  in_root = true            # mix file results into the root search
  roots = ["~"]             # folders to index
  exclude = ["~/Games"]     # folders or glob patterns to skip
  hidden = false            # also index hidden files and folders
  ```

  Editing the table applies live, like other settings (re-walk on a change of roots,
  exclusions or `hidden`).

### Index freshness

- The first walk runs on the module's own thread after start, so the daemon starts as
  before. Until it is done (about 2 s warm, up to ~8 s on a cold disk cache on this machine) the module answers with no files.
- Changes are picked up by watching the indexed folders (inotify on Linux through
  `notify`, as `apps` does; FSEvents and ReadDirectoryChangesW elsewhere through the same
  crate). A created, removed or renamed entry updates the index without a re-walk of
  everything.
- Watching is capped so the module never takes more than a share of the system's watch
  limit (builder's call: at most half of `max_user_watches`, and never above 200k). Folders
  past the cap are kept fresh by a periodic re-walk (every 30 minutes, as below).
- A full re-walk as a safety net: at most every 30 minutes, and only while the machine is
  not on battery saver if that is cheap to know (builder's call). The anti-goal "the
  machine gets busy" sets the limit: a re-walk must stay a short burst.

### Memory layout

The index stores entries compactly, as a tree: per entry its name, its parent entry, a
folder flag and the modification time; full paths are built when needed. The match keys a
name needs (lowercase, script set, transliteration skeleton) are computed once at indexing,
as the core index does for items, but only where they differ from the plain name, so most
Latin names cost nothing extra. Budget: at most ~150 MB of resident memory for 500k entries
(measured in the acceptance note).

### Matching and ranking inside the module

- The module uses the core's matcher (`Matcher`, `Pattern`, `TextKeys`) so a file's match
  score is on the same scale as an app's, and so layouts and transliteration work the same.
- The query is matched against the **file name**. When the query contains `/` (or `\` on
  Windows), the part before the last separator must also match the parent path (`proj/read`
  finds `~/Projects/x/README.md`).
- Inside the module, among names that match, these lift a result: a better name match
  (word starts, whole-word, prefix); files the user opened through the launcher (the
  module keeps its own small record of opened paths, so they survive the top-N cut before
  the core's usage applies); on Linux, files in KDE's recently used list
  (`~/.local/share/recently-used.xbel`); a recent modification time; shallower depth.
  Exact weights are the builder's call; the acceptance note lists a set of example queries
  and their expected top results on this machine.
- Speed: a search over 500k names must fit well inside the 100 ms deadline. Target ≤ 30 ms
  at the 95th percentile on this machine. Builder's call how: a cheap pre-filter (for
  example a per-name character-set mask) before the full scoring, and/or using several
  cores.

### Files in the root list

- Only for typed queries of at least **3 characters**; shorter queries show no files
  (builder may tune after trying it).
- The module answers with at most **3 files** for the root list.
- **Ranking against apps — one rule for every kind** (the user, 2026-11-22: "it does not
  matter whether it is an app, a command, a file or anything else: everything is ranked
  with usage"). Today every dynamic item scores a fixed 1000, above any static match (meant
  for calculator answers). Files are ranked like static matches instead: the name-match
  score plus usage, with the same formula and weights as apps and commands. Core change: a
  dynamic source can give each item its own match score; the engine then combines it with
  usage exactly as for an indexed item. Answer-style items (calculator) keep the fixed
  score. This is Rust-side only (built-in modules); the WIT interface is unchanged.
- **Ties without usage** (spec author's call, to keep the anti-goal): a file carries a
  small penalty, a few match points, smaller than what any usage adds. So an unopened
  `fire.txt` sits below an unopened Firefox at an equal match, while a file opened by habit
  for that query climbs above it. The user picked this behaviour (2026-11-22): a file
  opened repeatedly for a query may lead; a single open should not flip the order unless
  the usage formula itself does that for apps too.
- **Usage:** opening a file through the launcher records usage like any item, so a file
  opened often rises in the root list for the same query. This includes opening it from
  the "Search Files" screen (decided by the spec author): today only actions on root rows
  record usage, so the core gains a small internal effect for built-in modules (no WIT case,
  like `SetRootQuery`) that records a launch of an item under a query; the module sends it
  with the screen's query when a file is opened there.
- **Which score is compared with apps** (decided by the spec author): the module's own
  signals (recency, depth, KDE's recent list, its record of opens) only pick which 3 files
  it sends. The score the engine compares with apps is the name-match score plus the core's
  usage, minus the small tie penalty.
- A new item kind **File** (shown as "File" or "Folder" on the right of the row, like
  "Application" today). Files never appear in the empty-query list.
- With `in_root = false` the module returns no files for root searches; the "Search Files"
  command still works.

### Item identity

An item id is `files/<hash>`, where `<hash>` is a 128-bit hash of the full path's bytes in
hex. Paths can be longer than 256 bytes or contain characters ids may not hold. The same
path always gets the same id, so usage records stay attached to it. The module maps ids
back to paths. The title is the file name (control and format characters replaced for
display), the subtitle the parent folder with the home folder shown as `~`.

### Icons

Root rows and list rows show the theme icon for the file's type (folder, text, image,
archive…), resolved from the extension through the system MIME database on Linux; a
generic file icon elsewhere.

### The "Search Files" screen

- The "Search Files" command (kind Command) opens a screen whose field searches files only,
  with the same matching, no 3-character minimum and no 3-result cap (up to 100 results).
  Empty field: the most recently opened and recently modified files.
- **List layout** (default): rows with the type icon, name, parent folder, and the
  modification date on the right.
- **Grid layout**: tiles with a thumbnail and the name. Files without a thumbnail show
  their large type icon.
- **Toggle**: an action in the screen's action panel ("Show as Grid" / "Show as List") with
  a keyboard shortcut (builder's call: one that does not clash with existing shortcuts). The
  typed query and, where possible, the selected file are kept. The choice is remembered
  for the next time as the module's internal state in its own data file, not as a
  `config.toml` key (spec author's call: it is a view preference, and the settings GUI
  owns the config surface).
- Core / UI change (settled while planning): the list view gets a layout, rows or a grid of
  tiles with a column count. In the grid layout each row is drawn as a tile whose image is
  the row's icon (the thumbnail) and whose caption is the row's title. The list already has
  the search field, the revision and module-side filtering, so the toggle is an ordinary
  update of the same view and keeps the query and the selection. Plugin lists always use
  rows; the WIT `list-view` record is unchanged.
- Thumbnails (Linux): read from the shared freedesktop thumbnail cache
  (`~/.cache/thumbnails/{large,normal}`, keyed by the MD5 of the file URI, valid when the
  stored modification time matches). For images (PNG, JPEG, WebP, GIF) without one, the
  module makes the thumbnail in the background, writes it into that cache per the spec
  (so other apps reuse it), and updates the grid. Bounds: skip images over 50 MB or 100
  megapixels; decoding never runs on the UI thread. Videos and PDFs get a thumbnail only
  when the cache already has one. macOS and Windows: type icons only for now.

### Actions

On every file and folder row, root and screen alike:

| Action | Default key | Linux | macOS | Windows |
|---|---|---|---|---|
| Open (folders open in the file manager) | Enter | yes | yes | yes |
| Show in file manager | builder's call, same as `apps` reveal | yes (FileManager1 D-Bus, as `apps`) | yes (`open -R`) | yes (`explorer /select,`) |
| Copy path | same as `apps` copy-path | yes | yes | yes |
| Open with… | builder's call | yes | later | later |

"Open with…" pushes a list of the applications that declare the file's type (from the
`.desktop` files' `MimeType=`, default application first); choosing one launches it with the
file. It reuses the `apps` module's scanning and launching code rather than duplicating it.
Opening closes the window, as launching an app does.

## Acceptance

- `just check` passes; new code has tests (walk with exclusions, watch updates, matching
  with `/`, id hashing, ranking against apps, the root cap and minimum, the grid search
  field, thumbnail cache lookup).
- An acceptance note `docs/experiments/2026-11-file-search-acceptance.md` with timings on
  this machine: cold and warm first walk, entry count, resident memory, per-keystroke
  module search (median, 95th percentile) for root and screen queries, UI-thread time per
  keystroke stays within the 5 ms budget (each module answer adds one re-rank, as calc and
  weather already do), window open time unchanged, and the example queries with their top
  results (`fire` still shows Firefox first).

## Left to the builder

Marked "builder's call" above: the watch cap and re-walk schedule, the pre-filter and
parallelism, ranking weights, the shortcuts, the exact way settings reach the module and how
a settings change restarts the walk, and the MIME-to-icon lookup.

## Changes after approval

Made during planning and building by the builder (not asked of the user); each is recorded
with its reason in the acceptance note and the build ledger.

- The grid is a layout of the list view, not a grid view with a search field (planning).
- The first walk takes about 2 s warm and up to ~8 s on a cold disk cache; it is
  single-threaded to keep memory in budget.
- Re-walks when watching is off happen every 30 minutes, not 5.
- `.gitignore`, `.git/info/exclude` and the global gitignore apply only inside a git
  repository, as fd does; `.ignore` files apply everywhere.
- The acceptance line on UI-thread time became "within the 5 ms budget": each module answer
  adds one re-rank, as calc and weather already do.
- Thumbnails are decoded out of process in the `tomakast-raster` helper (suggested by the
  main session), so a decoder fault cannot take the launcher down.
- Root answers skip queries with no letter (arithmetic such as `10/2`); the Search Files
  screen still searches them.
- A malformed `[files]` value warns and keeps that key's default instead of failing the
  whole config.
- With the user's "everything is ranked with usage", a single open of a file for a query
  already lifts it above an equal app, as one open of another app would.
- When nothing but files matches, the root list leads with **"Search Files for “query”"**
  (the user, 2026-11-23: offer to keep searching files rather than show more of them). The
  module sends it with its files as a fallback item (`ItemKind::Fallback`); the core keeps
  a fallback only when every other hit is a file or folder, and it ranks first as a
  dynamic answer. Enter opens the Search Files screen with the query already typed. The
  root list keeps at most 3 files. (A briefer try that filled the list with up to 8 files
  was dropped.)
- A `gitignore` switch in `[files]` (the user, 2026-11-23: "needs a parameter; gitignore is
  honoured by default, and it can be turned off"): `gitignore = false` stops `.gitignore`,
  `.git/info/exclude` and the global gitignore from hiding anything; `.ignore` files and the
  built-in names still apply. Hidden files already have their switch, `hidden`.
- The index is **saved between runs** (the user, 2026-11-23): written after every walk to
  `<data>/files/index.bin` (mode 0600; a temp file renamed over the old one) and read back
  at start when the coverage (`[files]` roots, exclude, hidden, gitignore) is the same;
  the first walk then runs in the background and swaps in. Until any index is in place the
  "Search Files" screen shows the **busy line** under its field, as the translator's pad
  does (a list view's internal `busy` field; plugins cannot set it). The root list shows it
  too while a query files would answer (three characters, a letter) waits for the index:
  the module's search answer says `busy` (an internal field of the core's answer, never
  set from a plugin), the core keeps the line on across keystrokes until an answer is not
  busy, and once the index is in the module sends the internal `SearchAgain` effect, so the
  files join the root list without another key.
