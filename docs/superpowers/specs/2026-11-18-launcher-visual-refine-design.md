# Launcher visual refinement: design

Date: 2026-11-18. Status: approved in conversation (the row layout and the screenshot loop were confirmed by the user). Builds on branch `plan-2b1-views`. Parent spec: `2026-11-18-tomakast-core-design.md` (UI section: transparent rounded window, blur where the platform provides it, light/dark from the system).

## Intent

The user finds the current launcher "very raw" and wants a refinement pass "inspired by Raycast, not 1:1". Their explicit asks:

1. The background behind the panel is blurred (backdrop blur).
2. The subtitle is shown on the same line, right after the title, not under it. The item's type (Application, Command, later File, …) is shown at the right edge of the row.
3. The selection highlight spans the full width of the list.
4. The window has a shadow.
5. The window appears instantly on toggle and hotkey, with no open or close animation.
6. Other good findings from Raycast's design.

Success: on the user's KDE machine the launcher looks finished, calm and native-feeling, and still opens in about 20 ms warm.

Non-goals: a pixel copy of Raycast (no Raycast assets, names or exact layouts); a settings UI for theming; custom fonts; macOS and Windows window chrome (plans 2a and 3 apply the same tokens there).

Anti-goals: anything that slows the warm open (target ~50 ms, currently ~20.7 ms); any open or close animation; a look that differs between root and pushed views.

## Spike findings (2026-11-18, KDE Plasma 6.8.3 Wayland)

Spike diff kept outside the repo; findings:

- `WindowBackgroundAppearance::Blurred` makes gpui use the `ext-background-effect` protocol, which KWin 6.8 supports. `window.set_client_corner_radii` rounds the blur region to the panel's corners. The blur is visible and correct.
- `window.set_client_inset(m)` excludes a margin from the blur region. gpui grows the layer surface by `2m` itself (a requested 784×524 became 848×588), so the window must be requested at the panel size, not the panel size plus the margin.
- A `BoxShadow` drawn in that margin renders. A black shadow is barely visible on dark backgrounds, so the edge has to come mostly from the 1 px border and the blur. The shadow also paints under the translucent panel and darkens it, which the panel opacity must account for.
- The transparent margin still receives pointer input.
- On macOS and Windows, `Blurred` covers the whole window and the platform draws its own shadow, so the margin must be 0 there (plans 2a and 3).

## Approaches

- **A. Restyle in place with a small set of design tokens (chosen).** Put all sizes and colors in `theme.rs`, build one shared row, a header and a footer, and use them in root and pushed views. Small diff, no new dependency, every view benefits.
- B. Switch to gpui-component's ready-made List, Popover and Kbd widgets. More consistent with the kit, but their default look is "web app", and the earlier finding was that kit chrome fights layer surfaces (white backdrop). Rejected.
- C. Draw everything with a custom element and paint the layout by hand. Most control and most code, and it loses accessibility roles. Rejected.

## Design

### Window and panel

- Panel 750×474 logical px (Raycast-like proportions), corner radius 12.
- Linux/Wayland: shadow margin 24 px through `set_client_inset`. On other platforms the margin is 0 and the platform shadow is used.
- Blur on (`Blurred`), with the blur region rounded to the panel.
- Panel fill: dark `#1c1c1e` at 0.78 opacity, light `#f6f6f6` at 0.80. Border: 1 px, white at 0.10 (dark) or black at 0.10 (light).
- Shadow: two layers, black 0.35 with blur 24 and offset y 8, plus black 0.20 with blur 4 and offset y 1.
- No animation in our code. If KWin animates the layer surface on map or unmap, stop it for our surface. If that is not possible, document the KWin setting that turns it off.

### Layout (root and pushed views share it)

```
╭──────────────────────────────────────────────────╮
│ [←]  Search apps and commands…                   │  header, 56 px
├──────────────────────────────────────────────────┤  divider
│ Suggestions                                      │  section label, 28 px
│▌🦊 Firefox  Web Browser             Application ▐│  row, 40 px
│  📁 Dolphin  File Manager           Application  │
│ Commands                                         │
│  ◇ Demo: List                          Command   │
├──────────────────────────────────────────────────┤  divider
│ ● Copied                  Open ↵ │ Actions Ctrl K │  footer, 40 px
╰──────────────────────────────────────────────────╯
```

- **Header:**
  - search input without a border or background;
  - 18 px text, placeholder in the muted color, 16 px horizontal padding;
  - in a pushed view, a back chip on the left (`←` in a small rounded key-like box). For list and grid views the header holds the view's own filter input; for detail, form and scratchpad views it holds the view title;
  - a 1 px divider below (white 0.06 / black 0.06).
- **Section label:** 12 px, muted, semibold, 28 px tall, with the same left padding as a row's icon.
- **Row:**
  - 40 px tall; its highlight spans the full list width minus an 8 px gutter on each side, with radius 8;
  - 20 px icon;
  - title at 14 px in the text color, then the subtitle on the same line at 14 px in the muted color with a 8 px gap, and the subtitle truncates first;
  - accessories at the right edge at 13 px in the muted color; tags as small pills;
  - selection fill: white 0.08 (dark) or black 0.06 (light).
- **Root sections:**
  - empty query: "Suggestions" holds up to 8 items that have any usage, by rank, then "Applications" and "Commands", each alphabetical, with no item listed twice;
  - typed query: one "Results" section, ranked as today;
  - section labels are not selectable, and arrow keys skip them.
- **Root accessory:** the item kind, shown as "Application" or "Command". An item with no kind label shows nothing.
- **Footer:**
  - 40 px tall, with a divider above;
  - left: the context (the app name "tomakast" at the root, the view title in pushed views) at 12 px in the muted color; a toast replaces it while shown, with a colored dot (green success, red error, neutral info);
  - right: the primary action title plus a `↵` key chip, a thin vertical divider, then "Actions" plus `Ctrl` `K` key chips (`⌘` `K` on macOS).
- **Key chips:** 12 px text, min-width 20 px, height 20 px, radius 5, fill white 0.08 / black 0.06.
- **Toasts:** live in the footer, not floating. Success hides after 3 s, error after 5 s.
- **Action panel (Ctrl/Cmd+K):**
  - a floating popover anchored to the bottom-right corner above the footer, 320 px wide;
  - its own 1 px border and shadow; the fill is more opaque than the panel (0.92);
  - rows 32 px, each action's shortcut as key chips on the right;
  - the filter input sits at the bottom of the popover with a divider above it (as in Raycast).
- **Other views:**
  - list views use the same row and section label;
  - grid tiles get radius 8, a selection fill instead of a border, and the title below each tile in 12 px;
  - detail, form and scratchpad views get 16 px inner padding and use the same text sizes (body 14, muted 13);
  - form labels sit right-aligned in a left column, like the macOS settings forms.

### Colors

Own tokens in `theme.rs` for dark and light, chosen by the system appearance as today. The accent color is not used yet (no element needs it). Error and success colors stay from the kit theme.

## Left to the builder

- Exact font weights.
- The muted color's exact value (around 55% of the text color).
- The divider and the chip rendering details.
- How the root sections are built: a pure function from hits plus a usage flag to a list of section labels and rows, so it can be unit-tested.
- Whether "Suggestions" needs a minimum usage threshold; any usage is enough to start with.

## Verification

- Unit tests:
  - the section builder (empty query vs typed, no duplicates, the cap of 8, label rows skipped by navigation);
  - the kind label mapping;
  - footer content selection (context vs toast).
- Screenshots of root (empty and typed query), Ctrl+K, a toast and each demo view, in dark and light, taken with `TOMAKAST_NO_KEYBOARD=1`. Kept in `docs/experiments/2026-11-visual-refine.md`, together with the Raycast-inspired checklist above.
- Warm-open latency re-measured with the plan 2b-1 protocol: the median must stay within +2 ms of 20.7 ms.

## Decisions during implementation

- Popover opacity is 1.0 (the spec said 0.92): at 0.92 and 0.97 the rows under the popover showed through as ghost text.
- A section label sits at the bottom of a 40 px slot, not in a 28 px line.
- The grid header shows the view's title: the grid model has no filter.
- Action rows show only `↵`, on the selected row: the core has no per-action shortcut yet.
- Light muted text is 62% of the text color.
- The layer-shell namespace is `utility`, because KWin's scale effect animates other namespaces.
- Screenshots are kept out of git: they show the user's desktop through the blur.
