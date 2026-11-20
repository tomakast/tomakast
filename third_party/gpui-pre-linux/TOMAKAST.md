# gpui-pre-linux 0.3.12, patched

The crate as published on crates.io (`gpui-pre-linux` 0.3.12, pinned by `gpui-kit =0.10.0`),
with one tomakast change, wired in through `[patch.crates-io]` in the root `Cargo.toml`. The
commit that vendored it holds the published files unchanged; the change is the next commit.
Search for `tomakast patch` in `src/linux/wayland/window.rs`.

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

## Dropping it

Check on every gpui upgrade whether upstream sizes layer surfaces with their inset. If it
does, delete this folder, the `[patch.crates-io]` entry and the `third_party` exclude.
Otherwise re-apply the change to the new version: copy the published crate here unchanged
in one commit, then re-apply the `tomakast patch` hunks in the next.
