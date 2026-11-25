# gpui-pre-linux 0.3.12, patched

The crate as published on crates.io (`gpui-pre-linux` 0.3.12, pinned by `gpui-kit =0.10.0`),
with tomakast's changes, wired in through `[patch.crates-io]` in the root `Cargo.toml`. The
commit that vendored it holds the published files unchanged; the changes follow.
Search for `tomakast patch` in `src/linux/wayland/window.rs` and `src/linux/platform.rs`.

## The change: layer surfaces with a client inset are centred

The launcher is a layer-shell surface with no anchor, which the compositor centres, and a
transparent client inset around the panel that holds its shadow (`Window::set_client_inset`).

- **Before:** `set_geometry` gave a layer surface the size of the window geometry, which leaves
  the inset out. The compositor centred that box. The buffer is the inset larger on every side
  and starts at the box's corner, so the panel showed the inset to the right of and below the
  centre: 24 px with the old shadow margin, 44 px with the new one.
- **After:**
  - `set_geometry` asks for the whole buffer: the geometry plus the inset on each side;
  - the layer `configure` subtracts the inset again, because the xdg configure path it hands
    over to treats a configured size as the geometry and adds the inset back.

  The application asks for the panel plus the inset as the window size
  (`tomakast_ui::window::surface_size`). xdg toplevels and pop-ups are unchanged.

## The change: the digit row is the US one in shortcuts

`keystroke_from_xkb` names a key by the active layout. With Ctrl, Alt or Super held, the digit
row (xkb keycodes 10 to 21) is now named as on a US layout, shifted or not: on Russian,
Ctrl+Shift+2 was `ctrl-"` and is now `ctrl-@`, as on US; on French, Ctrl+1 was `ctrl-&` and is
now `ctrl-1`. Letters already fell back to the US key for non-Latin layouts (`guess_ascii`).
Needed by the hotkey recorder, which records the physical key the OS APIs bind.

## The change: a window that is not resizable has a fixed size

`is_resizable: false` was not told to the compositor on Wayland. An xdg toplevel created with it
now sets its minimum and maximum size to its size, so KWin offers no resize and no maximize
(the settings window). Resizable windows keep the GPU's texture limit as their maximum.

## Dropping it

Check on every gpui upgrade whether upstream sizes layer surfaces with their inset. If it
does, delete this folder, the `[patch.crates-io]` entry and the `third_party` exclude.
Otherwise re-apply the change to the new version: copy the published crate here unchanged
in one commit, then re-apply the `tomakast patch` hunks in the next.
