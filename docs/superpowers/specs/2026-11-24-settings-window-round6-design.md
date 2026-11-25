# Settings window, round 6: fonts, time zone, a Launcher page

Status: from the user's list of 2026-11-24, with their corrections to the first proposal
(fonts, text size everywhere, screens on other desktops). Extends round 5
(`2026-11-24-settings-window-round5-design.md`); where they disagree, this document wins.
Rulings marked **Ruling** are the session's calls, reported to the user, not asked.

## The points and what is built

1. **The settings window has a fixed size.** 900×640 logical pixels, as it opens today. No
   resize from its edges or corners, no maximize button in the title area, a double click on
   the title area does nothing (on every frame style: Breeze, Adwaita, Windows).
2. **The sidebar has no section titles.** "tomakast" and "Plugins" are gone; a gap
   (`SETTINGS_GROUP_GAP`, also the page's gap between groups since round 7) separates the two lists. The search's filtering of
   sections is unchanged.
3. **Theme options have icons.** System: `monitor`, Light: `sun`, Dark: `moon` (Lucide), in
   the dropdown's items and before the label on the dropdown button.
   (Points 4 and 5 as amended in round 7, `2026-11-24-settings-window-spacing-design.md`:
   the UI font defaults to the desktop's, and neither list puts its bundled family first.)
4. **Font** (`[appearance] font`): `Inter` (bundled, the default), then **System UI font**
   (value `system`), then every installed family, in a searchable list (as the time zone
   list). Applies to the launcher and the settings window.
5. **Monospace font** (`[appearance] mono_font`): `JetBrains Mono` (bundled, the default),
   then **System monospace font** (value `system`), then every installed family, searchable.
   Applies wherever monospace text is drawn (code blocks, sheets, strip text marked mono).
   **Ruling:** both lists hold every installed family; fonts do not tell reliably whether
   they are monospace.
6. **Text size** (`[appearance] text_size`): Small, Default (today's sizes), Large. Every
   text size of the launcher and the settings window is scaled: ×13/14, ×1, ×16/14 (row text
   13, 14, 16 px). **Ruling:** the settings window's one control width
   (`SETTINGS_CONTROL_WIDTH`) and its sidebar width scale with it, so controls stay one
   width and fit their text; heights stay. The kit's own sizes (rem-based `text_sm`, the
   kit theme's font size) scale by the same factor, and the strip's tiles are redrawn with
   it. Checked by capture at Large: tiles, every settings page.
   **Ruling:** widget tiles keep the sizes their elements give (a tile is laid out in a
   fixed cell); rows, the search, the footer and the settings window follow Text size.
   **Ruling:** the factor is held per UI thread (the launcher and the settings window share
   it; each test's app has its own).
7. **Monospace size** (`[appearance] mono_size`): an integer, 8–32, default 14 (today's
   monospace text). Its own number: not scaled by Text size. It sizes sheets (the
   scratchpad editor and its results) and code blocks; strip text marked mono keeps the size
   its element gives (a tile's layout depends on it).
   **Ruling:** "system" means the desktop's setting: KDE's `kdeglobals` `[General] font` /
   `fixed`, GNOME's `org.gnome.desktop.interface` `font-name` / `monospace-font-name`,
   elsewhere fontconfig's `sans-serif` / `monospace` match. Only the family is taken; the size
   is ours. A family that is not installed falls back to the bundled one.
8. **Time zone moves to Location** (`[location] timezone`, a time zone list, default: the
   computer's zone). The `local-time-zone` import answers it, so every plugin that asks for
   the time zone follows it (today: the calculator; weather takes each place's own zone from
   its forecast). The calculator loses its own `timezone` preference. Only the locale answer
   changes: the coarse location from the time zone (`location_sources`) keeps the computer's
   zone, so a chosen zone never moves the weather. A change applies at once, on reload.
   **Ruling:** file dates in the Files module keep the computer's zone (they are the
   computer's own clock).
   **Ruling:** at start the daemon moves an existing `[plugins.calc] timezone` to
   `[location] timezone` once (when the latter is not set) and removes the old key, so
   nobody sets it again and it is not listed as not recognized.
9. **A Launcher page**, after General, icon `search`:
   - **Return back home** (`[launcher] pop_to_root`; titled "Pop to root search" until round
     7; description "How long to wait after the launcher is hidden before its last search is
     cleared."): Immediately, After 30 seconds,
     After 90 seconds (default), After 3 minutes, After 10 minutes, Never. Opened again within
     that time after hiding, the launcher shows what it showed: the pushed views, the root
     query, the selection. Later (or "Immediately"), it opens fresh at root, as today; views
     kept that long are closed when the time runs out.
   - **Show on screen** (`[launcher] screen`): With the mouse pointer (default), With the
     active window, Primary screen.
   - **Remember search history** (`[launcher] history`, on by default): off, queries are no
     longer recorded. **Ruling:** what is recorded stays until cleared.
   - **Search history**: the number of saved searches and a **Clear…** button that asks first
     ("Delete all N saved searches? This can't be undone.") and empties the history. Disabled
     when there is none.

## Choosing the screen

The daemon asks the desktop which screen each choice means, at show, and opens the launcher
on that screen (`WindowOptions::display_id`; the layer surface's output on Wayland). Displays
are matched by output name (GPUI names a Wayland display's uuid `v5(DNS, name)`).

| Desktop | Mouse | Active window | Primary |
|---|---|---|---|
| KDE Plasma (Wayland) | KWin script | KWin script | KWin script (`workspace.screens[0]`) |
| Hyprland | `j/cursorpos` + `j/monitors` | `j/activewindow` | — |
| Sway | — (focused output) | focused window's output | — |
| niri | — (focused output) | focused window's output | — |
| X11 (any) | — | — | — |
| GNOME (Wayland) | — | — | — |

- **KDE:** a KWin script loaded once by the daemon reports the screen under the pointer, the
  active window's screen and the first screen to the daemon over D-Bus when one changes (the
  script compares, so a mouse move on the same screen sends nothing); a show reads the last
  report (no round trip at show). A script left by an earlier daemon is unloaded first; a
  KWin restart reloads it.
- **Ruling:** the dropdown lists only the choices the running desktop can honour (Sway, niri:
  "With the active window"; a dash in the table is a choice left out, "focused output" is
  what the desktop does then). Where none can (GNOME, which itself opens a window on the
  pointer's screen), the row is not shown.
- **Ruling:** macOS and Windows are left for later; the row is not shown there.
- **Ruling (as built):** X11 is left out: gpui's X11 screen is the whole X screen, not a
  monitor, so a monitor could only be picked by placing the window by hand, which cannot be
  checked here. The row is not shown on X11.
- **Ruling (as built):** Sway reads the focused workspace's output (`GET_WORKSPACES`), which
  is the focused window's; niri the focused window's workspace's output. Hyprland's monitor
  areas are its sizes over their scale, sides swapped when turned a quarter.
- Hyprland, Sway and niri cannot be run on the development machine; their replies are
  parsed from recorded samples in tests. KDE is checked live in a nested KWin with two
  virtual screens.

## Keeping the screen (pop to root)

The launcher window is still recreated on every show. **Ruling:** what is kept is the
launcher's view (`LauncherView`), moved into the new window, with its window-bound parts
redone (activation and appearance observers, theme and strip environment, focus). At hide,
with a time other than Immediately, its pushed views are not closed; tiles are told they are
hidden as today. A reply to a kept view's request finds it again (the kept view keeps its
ledger number). When the time runs out, or the daemon quits, the kept views are closed as a
hide closes them today (a sheet's unsent text is sent first). A hotkey or `tomakast run`
that opens onto a view drops what was kept first. Immediately is today's path exactly.

**Rulings (as built):**
- While hidden, replies to the kept screen's own requests, and a module's own updates of its
  views, are held and applied at the next show, in order, before the root search runs again
  (its selection kept by item). A copy in them is made at once; a toast or a close is dropped
  (nothing to show it in). Past 256 held replies the screen goes, as if its time ran out.
- The action panel, the hotkey recorder and Home edit mode close at every hide; they are not
  kept.
- A window the compositor closes keeps its screen as a hide does.
- The daemon has no orderly quit (it is stopped by a signal), so a kept screen's views are not
  told at a quit, as an open window's are not today.
- Until the daemon applies the settings, the controller's own default is Immediately, so a
  launcher built without settings (the tests) opens fresh.

## Build order

One commit (or a few) per point, `just check` green at each:
1. fixed window size; 2. sidebar gap; 3. theme icons; 8. time zone; 4–7. fonts and sizes;
9. the Launcher page (history, pop to root, then screens).

## Constraints

As round 5: English UI copy, one control width, no internal names in the UI, `just check`
green, the window checked live in both themes with screenshots, no push and no merge.
