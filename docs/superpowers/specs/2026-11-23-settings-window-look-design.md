# Settings window: look, layout and window frame

Date: 2026-11-23. Branch `claude/settings-gui-3345c2` (the settings window branch, not merged
yet). Builds on `docs/superpowers/specs/2026-11-22-settings-window-design.md`; everything that
spec says about what the window edits, how it writes `config.toml` and how it follows outside
edits stays as it is. This spec changes how the window looks and is laid out, adds two pages
(Appearance, About) and a config file row on General and gives the window its own frame.

## Why

The user tried the window. It works, but:

- it does not look like tomakast: the window uses the component library's default colours
  (near-black background, grey sidebar selection, bordered inputs), while the launcher has its
  own palette, rows, chips and spacing;
- the sidebar shows "Preferences" and "Commands" under every plugin; clicking them goes
  nowhere visible (the page is short, nothing scrolls);
- the sidebar has no icons and no grouping, so core settings, built-in modules and plugins run
  together;
- on Linux the system frame (KWin's Breeze title bar in the user's run) clashes with the window.
  Matching colours would fix it for one theme only; browsers (Chrome, Firefox) draw their own
  frame with controls that fit the desktop, and the user wants the same;
- the weather choices carry explanatory captions the user finds useless.

## Goals

- The settings window looks like part of the launcher: same palette, fonts, row style,
  selection, chips, spacing, light/dark behaviour. Opaque background (user's choice).
- A sidebar with three titled sections, an icon on every entry, no sub-entries:
  - **Core**: General, Appearance, Location, Hotkeys, About;
  - **Modules**: Home (and later Files, when the file-search branch merges);
  - **Plugins**: one entry per installed plugin, then the "(not installed)" entries.
- A new **Appearance** page with the theme setting: System / Light / Dark, applied at once to
  the launcher and the settings window.
- A new **About** page (name and version).
- A "Config file" row on General with Open and Show in folder; the daemon creates the file
  when it is missing.
- The window draws its own frame: a title area shared with the content and window controls
  (minimize, maximize/restore, close) that fit the platform and desktop.
- No captions on the weather choices.

## Non-goals

- Custom colour themes (user-made or bundled palettes beyond light/dark). Later work; the
  Appearance page is where they will go.
- Translucency or blur in the settings window (the user chose opaque).
- Any change to what the window edits or how it writes (see the 2026-11-22 spec).
- Pixel-exact copies of Breeze or Adwaita buttons, or reading the desktop's decoration theme.

## Anti-goals

- A window that looks like a generic component-library demo with tomakast's name on it.
- Sidebar entries that do nothing when clicked.
- A frame that loses what the system frame gave: moving, resizing from every edge, double-click
  to maximize, the window menu, snapping/tiling. A custom frame that cannot be resized on Linux
  is worse than the ugly one.
- Two sets of window controls (ours plus the system's).

## Approaches

- **A. Keep the component library's settings layout and restyle it.** Cheapest. Its sidebar has
  no section headings and shows only its own icon set (no plugin icons), and its sub-entries
  come with it. Fails the grouping goal.
- **B. Our own layout built from the launcher's pieces (chosen by the user).** Our own sidebar
  (sections, icons, search) and page area, drawn with the launcher's tokens; the library is kept
  for the controls themselves (text inputs, dropdowns, switches, number steppers, buttons,
  dialogs) and its Root (dialogs need it). Search over settings is ours to write.
- **C. Copy the library's settings layout into the project and modify it.** Fast start, but the
  project carries a fork of foreign code and its styling, and the item/group internals it relies
  on are private to the library, so the copy grows.

## Decisions

### Palette and type

- The window uses the launcher palette (`crates/ui/src/theme.rs`, `palette_for`): the panel fill
  (`0x1c1c1e` dark, `0xf6f6f6` light) **at full opacity** as the window background; text, muted
  text, dividers, borders, the selection tint, the chip tint and the field tint as the launcher
  uses them. Fonts: Inter and JetBrains Mono, as now.
- Light/dark follows the Appearance setting (below), whose default follows the system, like the
  launcher today.
- Controls taken from the library are restyled to match the launcher's form view (the form
  view's text field fill, `CONTROL_RADIUS`, `CONTROL_HEIGHT`), so a field in settings and a field
  in a launcher form look the same. The library's dropdown list already takes the launcher's
  popup colours (`paint_kit_popups`).
- Sizes come from the launcher tokens where one exists (row text 14, muted 13, small 12, section
  label height 28, row radius 8, action row height 32, edge padding 16). New sizes go into
  `theme.rs` as named tokens, not literals in the window code.

### Layout

```
┌──────────────────────┬───────────────────────────────────────────────┐
│ [🔍 Search…        ] │ Weather                          ─   ▢   ✕    │  ← title area (drag)
│                      ├───────────────────────────────────────────────┤
│ CORE                 │                                               │
│ ⚙  General           │  Preferences                                  │
│ 🎨 Appearance        │  Temperature                    [ °C      ▾ ] │
│ 📍 Location          │  Wind speed                     [ m/s     ▾ ] │
│ ⌨  Hotkeys           │  Pressure                       [ hPa     ▾ ] │
│ ⓘ  About             │                                               │
│                      │  Commands                                     │
│ MODULES              │  ☁ Refresh Weather              [ Record    ] │
│ 🏠 Home              │                                               │
│                      │                                               │
│ PLUGINS              │                                               │
│ 🧮 Calculator        │                                               │
│ ☁  Weather        ◀  │                                               │
│ ▫  gone (not inst.)  │                                               │
└──────────────────────┴───────────────────────────────────────────────┘
```

- Two columns: the sidebar (fixed width, about 240 px; the builder may make it resizable within
  a range) and the page. A divider between them.
- The top band of the window is the title area (height a theme token, about the launcher's
  header). Over the sidebar it holds the search field; over the page it holds the page title and,
  at the outer edge, the window controls. The band's empty parts move the window.
- The page scrolls under the title area; the sidebar scrolls on its own when it is too long.
- Sidebar entries: launcher-style rows (icon, title; selection is the launcher's selection tint
  with the row radius). Section labels in the launcher's section-label style (muted, small).
  A section with no entries (no plugins installed and no tables) is not shown.
- Plugin settings depend only on whether the plugins folder can be listed, not on whether
  plugins run (user's rule):
  - **The folder can be listed**: every plugin gets its page with editable preferences and
    command hotkeys, whatever its state, including when the plugin runtime itself cannot start
    (its database cannot be opened, the manager cannot be created or spawned). The daemon then
    builds the catalog from the folder and the manifests alone, each plugin in a "not running"
    state carrying the reason; the plugin page shows that state the way it shows "failed" today.
  - **The folder cannot be listed**: the Plugins section is not shown at all, and neither are the
    `[plugins.<id>]` tables (with no list, nothing can be said about them).
  - **Not listed yet** (the window opened right after the daemon started): the same as above;
    the section appears once the list arrives. This replaces the "Loading plugins…" read-only
    tables of the previous round.
  - The separate "Plugins" notice page ("Plugins are not running: …") is removed, and so are the
    "Changes apply after a restart" notes it drove on Location and Home.
- Page content: group titles (such as "Preferences", "Commands", "Tiles", "Not recognized") stay
  as headings inside the page; they are **not** sidebar entries. Rows: title (and the existing
  secondary line such as the tile's widget name), control on the right, the ↺ reset next to the
  control as today, error and notice lines under the row as today.

### Sidebar icons

- General `settings`, Appearance `palette`, Location `map-pin`, Hotkeys `keyboard`, About `info`,
  Home `house` (Lucide names from the bundled set; the builder may pick close equivalents).
- A plugin's entry: the icon of its first command that has one, drawn with the launcher's icon
  renderer (Lucide, emoji or image as the launcher draws them); otherwise the launcher's
  monogram of the plugin's name. "(not installed)" entries: monogram, greyed.

### Search

- The search field filters the sidebar and the page. A row matches when the query is in its
  title, its key name, its keywords (as given today) or its page's title (case-insensitive). A
  group shows when one of its rows matches; a page shows in the sidebar when one of its groups
  does. With a page title match the whole page matches.
- If the shown page has no match, the first matching page is shown. If nothing matches: an empty
  state "No settings match". Esc in the field clears a non-empty query; on an empty query it
  does what Esc does in the window without the field.
- A catalog change keeps the query (unlike the library's layout, Ruling Q of the first spec is
  no longer needed).

### Appearance page

- New built-in key `[appearance] theme`, a choice: `"system"` (default), `"light"`, `"dark"`.
  It joins the settings schema like every built-in key (parsed, shown, reset by removing the
  key, an invalid value shown in the error state and the default used).
- Titles: "Theme" with "System", "Light", "Dark".
- Applied at once and live, through the daemon's existing config watch, to the launcher and the
  settings window (the theme mode switch already used for system appearance). With `system`, the
  launcher keeps following the system appearance as today. `TOMAKAST_DEV_LIGHT` still forces
  light.
- The page has room for more appearance settings later; only Theme now.

### About page

- The app name and its version (the workspace version). No icon: the project has none yet;
  making one is separate work. No update check, no licences list, no links (none exist yet).

### Config file row on General

- General gets a row "Config file" showing the path of `config.toml`, with **Open** (the
  system's handler for the file) and **Show in folder**. (User: these belong to the settings,
  not to About.)
- The file always exists: the daemon creates `config.toml` (empty, with its folder) at start
  when it is missing, and the row's buttons create it the same way if it was deleted while the
  daemon ran. Nobody has to create it by hand.
- Open uses the project's `tomakast_platform::open_path`; Show in folder uses the freedesktop
  file manager call (`org.freedesktop.FileManager1.ShowItems` over D-Bus, zbus is already a
  dependency), falling back to opening the folder; `open -R` on macOS, `explorer /select,` on
  Windows. If the system cannot open or show it, a line "Could not open: <reason>" appears
  under the row, like a failed write.

### Window frame

- **Linux.** The window asks for client-side decorations. When it gets them:
  - the frame itself (shadow inset, 1 px border, resizing from every edge and corner, square
    corners on tiled edges) is drawn by the component library's Root, which every kit window
    gets and which cannot be turned off; the window sets the theme's window radius (a token,
    about 10 px) so the corners are rounded, all four (the library has one radius for all
    corners), and uses the library's border shadow. The window code draws no resize zones and
    sets no client inset of its own. *(Builder-check finding.)*
  - the title area moves the window, double-click maximizes/restores, right-click opens the
    window menu;
  - window controls at the outer edge of the title area, in the order and on the side the desktop
    is set to: on KDE from KWin's own setting (the decoration's buttons on the left and right in
    `kwinrc`, group `[org.kde.kdecoration2]`, keys `ButtonsOnLeft` / `ButtonsOnRight`; letters
    other than minimize `I`, maximize `A` and close `X` are dropped, and a missing key takes
    KWin's default, `MS` left and `HIAX` right, i.e. minimize, maximize, close on the right;
    `crates/platform/src/icon_theme.rs` reads `kdeglobals` the same way), elsewhere from the
    system's button layout as GPUI reports it; only the controls the window manager supports
    (GPUI reports which). The choice of controls and their order is a plain function of the
    decorations, the layout and the supported set, so headless tests (whose platform reports
    system decorations) can test it directly;
  - the controls' shape follows the desktop (from `XDG_CURRENT_DESKTOP`): KDE → Breeze-like
    (round icon buttons, close turns red on hover); GNOME and GTK desktops → Adwaita-like (small
    round filled buttons); others → the KDE shape. Colours come from our palette (light/dark).
  - When the compositor gives server-side decorations anyway (X11 without a compositor, or a
    compositor that refuses), the window draws no controls and no frame of its own; the title
    area is then just the top of the window.
- **macOS.** Transparent title bar with the system's traffic lights over the sidebar's top-left
  corner; the search field leaves room for them.
- **Windows.** The window draws its title area and Windows 11-style controls (wide rectangular
  buttons, close red on hover), marked as the system's caption buttons so snap layouts work.
- The window's title stays "Settings — tomakast" for the task bar and window lists, even though
  the title area shows the page title.

### Captions

- Remove the `description` of `temperature`, `wind` and `pressure` in `plugins/weather/plugin.toml`
  and the test that requires every weather preference to have one. The manifest keeps
  supporting `description` for other plugins.
- The Home page's hint about adding tiles stays.

## Constraints

- GPUI and the component library are pinned (`gpui-kit =0.10.0`); no upgrade, no fork.
- Only bundled assets (Lucide icons, Inter, JetBrains Mono); no new external crates beyond
  what the workspace has, unless a small one is unavoidable for reading `kwinrc` (an INI file;
  the builder can parse the two keys by hand).
- The window's headless tests find elements by their accessible labels; the new layout keeps
  labels for every interactive element (sidebar entries labelled with the page title, the search
  field, every control as today), so the existing tests port with selector changes at most.
- Other branches (file search, translator, leader-key, side-modifiers) are not touched. The Files
  page joins "Modules" when the file-search branch merges; the merge note says so.
- Platform code for the frame lives behind `cfg(target_os)`; `just check` cross-checks only the
  location crate for macOS/Windows, so the macOS and Windows frame paths are written carefully
  and listed as not run.

## Left to the builder

- Exact sizes of the sidebar, title area and control buttons (as named theme tokens).
- Whether the sidebar is resizable.
- How the library's controls are restyled (theme fields vs. per-control styling).
- How the existing page builders move off the library's page/group/item types (a small own
  model of page → groups → rows that the layout and search both read is the expected shape).
- The exact icon names.

## Done means

- The window matches the layout above in light and dark, on KDE Wayland by eye (screenshots for
  the user, not committed).
- Every goal above covered by headless tests where it can be (sidebar sections and order, no
  sub-entries, icons present, search filtering and the empty state, Appearance key parsing and
  live apply, About actions, frame controls' order from a given layout and their absence under
  server-side decorations).
- `just check` passes; the acceptance note gains a section for this round.
