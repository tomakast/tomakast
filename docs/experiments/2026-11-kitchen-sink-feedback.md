# Kitchen-sink feedback session (plugin host UX)

Date: 2026-11-19. Branch `fb-kitchen-sink`, cut from `plan-2b2b-sdk-calc` (`229c7aa`).
The user tried the plugin host through the kitchen-sink plugin. Each item records what they
asked for, what was decided and where it landed.

## K1. A default icon when there is none

- **Asked:** rows without an icon showed blank space. Theme names from plugins
  (`icon = "face-smile"`) were blank too: the host never resolved them, and only the `apps`
  module looks names up in the freedesktop theme.
- **Decided by the user:**
  - the fallback is a monogram, one neutral colour for all;
  - a theme name is looked up in the bundled Lucide set only, not in an OS theme.
- **Design:**
  - **Named icons.** `Icon::Named(name)` draws the Lucide glyph `icons/<name>.svg` in the text
    colour, at 0.8 of the icon square. Only `[a-z0-9-]+` names are looked up, against the set of
    embedded file names, so a plugin's name never reaches another asset path. The daemon
    registers `gpui_kit::assets::AllAssets` (all 1 830 icons, about +1 MiB of binary) through
    `tomakast_ui::assets()`.
  - **Monogram.** It is drawn when there is no icon, the name is outside the set, an emoji or
    colour is invalid, or an image file fails to load. It is the first alphanumeric character of
    a text, uppercased, semibold in the muted text colour, on a rounded square filled with the
    neutral chip tint (the key chips' fill) and edged with the hairline border colour. It looks
    the same for every item: the user rejected per-provider colours ("one neutral colour, like
    the background, but visually separated").
  - **Which text the letter comes from.** Root rows use the item's source (a plugin's name)
    when it has one, so every row of a plugin shows that plugin's letter; other root rows use
    their own title. Rows of a pushed list use the row title, and grid tiles use the caption.
- **Contract:** the `icon.named` doc comment in WIT 0.1 now says the name is a Lucide name.
  It was edited in place and re-hashed (`FROZEN.md` record), per the user's ruling that 0.1 is
  edited in place while no outside plugin depends on it. `docs/plugins/manifest.md`, the author
  guide and the SDK README use Lucide names. kitchen-sink's command icon is now
  `face-slightly-smiling`, and the demo grid's is `image`.
- **Left for the calc session:** `plugins/calc/plugin.toml` still names
  `accessories-calculator` and `document-open-recent`, which are not Lucide names, so calc's
  commands show the monogram "C" until they become `calculator` and `file-clock`.

## K2. The window shadow was cut off, and the panel sat off centre

- **Asked:** the shadow looked cut off at some point.
- **Cause:** GPUI's blur radius is a Gaussian's sigma, half a CSS blur, and a shadow is drawn
  out to 3 × blur past the panel (`vs_shadow` in the pinned gpui-pre-wgpu shaders). The 24 px
  blur therefore reached 80 px below the panel, but the transparent margin of the layer surface
  was 24 px, so the shadow stopped in a hard edge there. A screenshot showed the step 36 device
  px (24 logical px at scale 1.5) below the panel.
- **Fix:** the layers are CSS `0 8px 24px` and `0 1px 4px` (sigma 12 and 2), the margin is
  44 px, and `theme::tests::panel_shadow_fits_in_the_margin` keeps every layer inside it.
- **Found on the way, fixed with the user's go-ahead:** the panel never sat in the centre. With
  a client inset, gpui-pre-linux gave a layer surface the size of the window geometry only. The
  compositor centred that box, and the inset hung off its right and bottom, so the panel was
  24 px (now it would have been 44 px) right of and below the centre. gpui-pre-linux 0.3.12 is
  vendored under `third_party/` with a two-hunk patch (`TOMAKAST.md` there): a layer surface's
  size includes the inset, and its configure subtracts it again. The launcher asks for the panel
  plus the margin (`window::surface_size`).
- **Checked:**
  - a `WAYLAND_DEBUG=1` trace gives one `configure(838, 562)` per show and the same
    `set_size(838, 562)` over 24 shows, so the size does not grow;
  - the panel is centred horizontally to the pixel. The 16 px left vertically is KDE's work
    area (the top panel) and was there before;
  - warm open is a median of 18.3 ms (p95 18.7) over 49 shows, and idle CPU is 1 tick in 10 s.
- **Clicks on the margin (the user asked for it next):** the transparent margin took pointer
  input, so a click just beside the panel never reached the window below, and the margin had
  grown to 44 px. On Wayland the launcher now sets its input region to the panel
  (`window::input_region`, through `Window::set_input_region`). A `WAYLAND_DEBUG=1` trace shows
  `wl_region.add(44, 44, 750, 474)` handed to `wl_surface.set_input_region`. Where there is no
  margin (X11, macOS, Windows) the whole window keeps taking input.
