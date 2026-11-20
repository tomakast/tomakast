# gpui-kit components vs. our hand-drawn UI

Date: 2026-11-20. Branch `gpui-kit-review`. Kit: `gpui-kit =0.10.0`, which brings `gpui-component 0.10.0`, `gpui-base 0.10.0` and `gpui-pre =0.3.12`. Sources read are under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`. Below, `C/` is `gpui-component-0.10.0/src` and `B/` is `gpui-base-0.10.0/src`.

Question: we draw most of the launcher ourselves. Which pieces could be gpui-kit components instead, and what would each swap gain and cost? This note is analysis only; no product code changed.

## Summary

- **There is a recorded reason for drawing our own.** The visual-refine spec (`docs/superpowers/specs/2026-11-18-launcher-visual-refine-design.md`, "Approaches", B) considered gpui-component's List, Popover and Kbd and rejected them. Its reasons: the default look is "web app", and kit chrome fights layer surfaces. The window probe (`docs/experiments/2026-11-window-probe.md`) adds a hard constraint. The launcher window is opened with bare `cx.open_window`, without the kit Root, because that Root paints an opaque backdrop and a client-side frame around the layer surface. Everything that needs the Root's per-window state is therefore missing in our window.
- **What we already use from the kit:** init, theme and light/dark mode, `Input`/`Textarea` (search, filters, form text fields, scratchpad), `Scrollbar` (widget strip), the layer-shell window kind, assets, and the test harness. The launcher window does not use the kit Root (`crates/ui/src/window.rs:5-7`).
- **Verdict for the launcher's core: keep ours.** That covers the result list and rows, header and footer, key chips, action panel, toasts, detail and review views, markdown and the widget strip. The kit versions either cannot run in our window (notifications, dialogs, kit tooltips), or would replace a small amount of code with a different keyboard model, theme-driven colours and more work per frame, and give the user nothing new.
- **Worth adopting: the form controls.**
  - Checkbox (small).
  - Select for our dropdown (moderate; one key conflict to fix).
  - A masked date input (small).
  - Later, when new field types arrive: kit `NumberInput`, masked `Input` for passwords, and `Switch`.
  These run in our window (checked by mounting them, see [Mount check](#mount-check-kit-controls-in-a-bare-window)) and replace hand-drawn controls that are visibly cruder: a text glyph `☑`/`☐` and an inline list that pushes the form down.
- **Open decision for future screens:** do plugin preferences and a plugin manager live **inside the launcher panel**, or in **a separate ordinary settings window**? Inside the panel, the kit's form controls fit, but its settings framework, dialogs and notifications do not. In a separate window opened through the kit, the whole kit works. See [Pieces we will need soon](#pieces-we-will-need-soon).

## The constraint that shapes everything: no kit Root in the launcher window

The kit's per-window state is `WindowState` (`C/root.rs:26`). `gpui_component::init` registers it as a plugin of `gpui_base::Root`, and only `gpui_kit::open_window` creates a Root. Our window has none, so kit components fall into three classes. Checkbox, Switch, Select and List were mounted and driven ([Mount check](#mount-check-kit-controls-in-a-bare-window), [Measurement](#measurement-kit-list-vs-our-list)). The rest are classified from reading their code.

| Class | Components | In our window |
|---|---|---|
| Plain elements and self-contained state | Kbd, Tag, Badge, Separator, DescriptionList, Alert, StatusBar, Toolbar, Spinner, Skeleton, Button, Checkbox, Switch, Radio, Slider, Input variants, List, Command, Form, Settings, DataTable | Work. |
| Floating popups (`deferred` + `Positioner`) | Select, Combobox, DatePicker, Popover, PopupMenu/DropdownMenu, ContextMenu | Work. The popup is clamped into the window (`B/positioner.rs`). For a client-decorated window the clamp also leaves out the client inset (`frame_insets`, `B/positioner.rs:206,431-445`). A layer surface reports client decorations (window probe), so on Wayland a popup stays inside the panel and out of the transparent shadow margin, where clicks fall through. On macOS and Windows the margin is 0 and the window is the panel. This is read from code only: a test window has no inset. |
| Need `WindowState` | `window.push_notification`, `open_dialog`, `open_alert_dialog`, sheets | **Panic** (`WindowState::update` → `expect(ROOT_MISSING)`, `C/root.rs:145-152`; callers in `C/window_ext.rs`, `C/dialog/dialog.rs:595,626,713`). |
| | Kit tooltips (`.tooltip()` on Button, Switch…) | **Silently never show**: the overlay lookup returns `None` (`C/root.rs:616-640`). GPUI's own `.tooltip(..)` still works. |
| | Focus-trapping and Tab traversal | Base Root binds Tab/Shift+Tab (`B/root.rs:44-45`); without it Tab does nothing unless we handle it, as our form already does. |

`gpui_component::init` is still required, and we call it: it installs the theme, the base `GlobalState` used by every popup, and all key bindings. The Root would also set the rem size to the theme's font size. The mount check shows both are 16 px, so kit sizes come out as designed even without it.

Two ways out, if a future screen needs the third class:

1. **Kit Root in the launcher window.** The window probe describes the fix: create layer surfaces with server decorations, so `window_border()` passes through, and override the Root's background to transparent. We now vendor `gpui-pre-linux` (`third_party/gpui-pre-linux`), so this is one more `tomakast patch` hunk there, not a fork. Cost: about half a day on Linux. macOS and Windows are unverified: the white fill is a theme-colour background and needs the same transparent override there, and the Root also sets font and text colour. Not needed by anything recommended below.
2. **A separate ordinary window** for settings and the plugin manager, opened with `gpui_kit::open_window`, where the whole kit works. It is a normal decorated window, not the overlay, so it has none of the layer-shell problems. Its first open has no prewarm, and the probe measured a cold window at about 200 ms. That is fine for a settings window and wrong for the launcher.

## Measurement: kit List vs our list

The claim that matters for the result list is render cost per keystroke and per open, because the window is recreated on every show. Measured with a throwaway in-crate test, not committed.

- **Setup:**
  - 10 000 rows in 3 sections, 40 px rows, in a 750×378 viewport (the panel's list area);
  - release build, headless `TestAppContext`;
  - each sample is the state update plus `window.draw`, timed around both;
  - 30 open samples (new window, state, first frame) and 300 samples of each kind of update.
- **Variants:**
  1. ours: `uniform_list` with `views::row` and `views::section_label`, as `render_root` does;
  2. kit `List` + `ListDelegate` with kit-styled `ListItem` rows holding the same content;
  3. kit `List` with our `views::row` inside each `ListItem`, which isolates the List's own cost.
- **What "keystroke" means:** the visible set changes (1/1 to 1/5 of the rows), the selection resets to the first row and the list scrolls to the top. "Arrow" moves the selection by one.
- **Caveat:** the harness uses GPUI's no-op text system and no GPU. The numbers are element build, layout and scene building, which is where the two lists differ. They exclude glyph shaping and rasterizing, which both lists pay the same for.

| Variant | Open + first frame, median (p90) | Keystroke, median (p90) | Arrow, median (p90) | Rows built per frame |
|---|---|---|---|---|
| ours | 0.64 ms (0.71) | 0.56 ms (0.58) | 0.58 ms (0.59) | 18.9 |
| kit List + kit ListItem | 1.04 ms (1.24) | 0.86 ms (0.93) | 0.90 ms (0.98) | 25.9 |
| kit List + our row | 1.34 ms (1.60) | 1.16 ms (1.37) | 1.23 ms (1.25) | 25.9 |

Reading:

- The kit List costs about twice as much as ours for the same rows.
- It builds about 7 more rows per frame. Part of that is its measure pass: on every render it lays out one item, the section header and the footer as roots to measure them (`C/list/list.rs` `prepare_items_if_needed`). The rest is the base virtual list building more rows around the viewport than `uniform_list` does. That split is not measured.
- **Both are far inside the budgets**: about 50 ms for an open, and a frame per keystroke. Speed is not the deciding factor. Behaviour and look are (next section).

## Piece by piece

Effort is for one person who knows the code, including test updates. Test impact: the UI tests (`crates/ui/tests`, about 8 700 lines) drive the launcher almost entirely by keys. There are about 108 Enter, 68 Up, 50 Down, 34 Escape and 18 Tab presses, and they find elements by id (`form-field`, `form-options`, `action-panel`, `view-header` …). A swap that changes what a key does breaks tests well beyond the piece itself.

### Root result list and pushed list view → kit `List` (`C/list/`)

- **Gives:**
  - sections whose headers can have their own height (our labels are row-tall because `uniform_list` needs one height);
  - a built-in empty state, a loading skeleton and load-more paging;
  - ARIA position in set.
- **Costs and fights:**
  - **Keyboard and focus model.** The List's keys (up, down, enter, ctrl-enter, escape) are bound in the `List` context and act only while the list or its own search field has focus (`C/list/list.rs:26-42`). Our focus stays in the header search input, so none of them would fire. We would drive the selection from outside through `set_selected_index` and `scroll_to_item`, which is what we do with `uniform_list` today.
  - **Behaviour differences:**
    - navigation wraps around at both ends (`list/cache.rs` `next`/`prev`), while ours stops at the ends;
    - Escape clears the selection (`on_action_cancel`);
    - a single click confirms the row;
    - scrolling to the selection uses `ScrollStrategy::Top`.
    Each of these differs from today's launcher and from the tests.
  - **Search plumbing** that we would not use: a search field of its own, and a per-query task with a fixed 100 ms "searching" spinner (`start_search`). Our search (nucleo, keyboard layouts and scripts) is in the core and stays there either way.
  - **Look.**
    - `ListItem` hard-codes `py_1 px_3 gap_x_1 text_base` and then applies our style.
    - **After** our style it paints the selection with `theme.list_active` or `theme.accent`, and the hover with `tokens.list_hover` (`C/list/list_item.rs`). To keep our rounded, inset highlight we would rewrite those theme tokens.
  - Render cost ×2 (measured above).
- **Plugin view model:** `ListView` rows map to sections easily. Nothing gained.
- **Effort:** 3–5 days with look regressions and test rewrites. **Verdict: keep ours.** If we ever want shorter section labels, use the base variable-height `v_virtual_list` (`B/virtual_list.rs`) under our own rows, not the List.

### Row (`views::row`) → `ListItem`

Covered above: the theme repaints the selection over our style, and the fixed paddings break our 16 px edge alignment (`EDGE_PADDING = ROW_GUTTER + ROW_PADDING_X`). **Keep ours.**

### Grid view → none

The kit has no selectable tile grid; `grid_layout` is a layout helper. **Keep ours.**

### Header and footer (`chrome.rs`) → `StatusBar`, `Toolbar`

- `StatusBar` is a styled strip: `py_1 px_2 text_xs`, with its own border and background tokens. It has no behaviour.
- `Toolbar` adds roving focus on Left/Right (`B/toolbar.rs:122-140`), which would collide with our Left/Right through the widget strip.
- **Keep ours.**

### Key chips → `Kbd`

- `Kbd` formats a GPUI `Keystroke`: on Linux and Windows, `Ctrl+Shift+C` and **`Win+K` for the super key** (`C/kbd.rs:140-147`, checked); on macOS, symbols.
- Ours (`shortcut::chips`) shows one chip per key, names super "Super", and takes the plugin's shortcut data, which are not GPUI actions.
- `Kbd` takes its radius and colours from the theme.
- **Keep ours.**

### Tag accessory → `Tag`

`Tag::custom(bg, fg, border)` could take our tokens, but ours is one `div` and Tag brings its own paddings and radius. **Keep ours.**

### Action panel (Ctrl+K) → `Command`, `PopupMenu` or `Popover`

**`Command` (`C/command/`)** is the closest match: a search field over grouped items with shortcut hints. But:

- **Filtering is a plain case-insensitive substring match** on the label and keywords (`C/command/item.rs:114-128`). We filter actions with the core matcher across keyboard layouts and scripts (fb-search), so a query typed in the wrong layout would stop finding actions.
- **Shortcut hints come only from GPUI actions bound in the keymap.** Plugin actions carry their shortcut as data, so every row would need a custom child.
- Behaviour differences:
  - Escape first clears the query and only a second press leaves, while ours closes the panel at once;
  - navigation wraps around;
  - the item list is rebuilt from the builder on every render, and rows are measured when it changes.

**`PopupMenu`** has no filtering of any kind. Its shortcut column also comes only from bound actions, and an input inside it is clicked away (`C/menu/popup_menu.rs`).

**`Popover`** would only replace our absolutely positioned `div`. It moves focus into itself, and Escape works only while focus is inside it.

**Effort:** about 2 days, plus the loss of cross-layout action search. **Verdict: keep ours.**

### Toasts → `Notification`

- `window.push_notification` panics without the Root.
- A `NotificationList` can be mounted by hand, but it is a full-window overlay with top-right stacks 382 px wide (`C/notification.rs`), not a footer element.
- It animates: 400 ms enter, 200 ms exit, auto-hide after 5 s, and a 50 ms timer while active. The visual-refine spec lists any animation as an anti-goal.
- The base `ToastStack`/`ToastManager` (`B/toast.rs`) could sit inline, but it is built around a spring-animated stack too.
- **Keep ours.**

### Detail view, list detail pane and the plugin review view → kit markdown (`TextView`) and `DescriptionList`

The plugin review view is a `DetailView` built by `plugin-host` (`plugins_module.rs:383`), so it shares this row.

- The kit's markdown renders images with `img(uri)` (`B/text/inline_flow.rs:342,1766`), which fetches URLs and reads files.
- Without a handler, its links open through `App::open_url` (`B/text/editor_host.rs:101`).
- It parses raw HTML by default; `raw_html(false)` turns that off (`B/text/markdown_ext.rs:293-300`).
- Our markdown is for untrusted plugin text and promises that nothing is fetched, HTML stays literal, and links open only after `checked_url` (`crates/ui/src/markdown.rs:1-10`). The kit renderer could be configured toward that, but every kit upgrade would have to be re-audited against those rules.
- `DescriptionList` is bordered by default, has no `Styled`, and uses its own label colour.
- **Keep ours.**

### Form view (`views/form.rs`)

| Field | Today | Kit candidate | Gain | Cost | Verdict |
|---|---|---|---|---|---|
| Text, multi-line | kit `Input`, `Textarea` | (already kit) | — | — | keep |
| Checkbox | a text glyph `☑`/`☐` in a focusable div; Space toggles, and Enter toggles through the launcher's `Confirm` (`form_confirm`, `launcher_view.rs:1106-1118`) | `Checkbox` (controlled: `.checked`, `.on_change`) | A drawn box with a check mark and a focus ring; CheckBox role with its toggled state | Space **and Enter** toggle it (checked). `form_confirm` must then stop toggling checkboxes, or Enter would toggle twice. Indicator colours come only from the global theme (`primary`, `input`). The mark fades over 180 ms, the theme's motion token, read from code. | **adopt** (≈0.5 day) |
| Dropdown | our own list that opens inline and pushes the fields below down; Up/Down/Enter | `Select` over `SelectState` | A popup that floats over the form; optional search for long option lists; ComboBox role; silent setters (`set_selected_value` emits nothing), so module updates do not count as user edits | **Ctrl+Enter on a focused select opens it instead of submitting** (checked). Fix, checked: bind Ctrl+Enter to our `SubmitForm` in the `Select` context after kit init. A closed select then submits. With the popup open, Ctrl+Enter picks the highlighted option and closes the popup, because the popup list's own binding is deeper; a second Ctrl+Enter submits. On macOS the same applies to Cmd+Enter. Popup colours come from the theme's popover token. Tests that read `form-options` change. | **adopt** (≈1–1.5 days) |
| Date | text input checked as `YYYY-MM-DD` on submit | `DatePicker` | A calendar | No typing: the trigger is not an input; calendar days are mouse-only; its time field takes Tab | **keep ours**, but **adopt** `InputState::mask_pattern("9999-99-99")` and `validate` (`B/input/base/state.rs:4269,13204`) so the format is enforced while typing (≈0.5 day) |
| Submit | our bordered `div` | `Button` with `ButtonVariant::Custom` | Focusable, Enter/Space activate | Sizes and radius from the theme | adopt together with the checkbox, optional |
| Layout | label column right-aligned, error line under the field | `Form`/`Field` | Required marker, description slot | Labels are not right-aligned; no error slot (only `description_fn`) | keep ours |

All of these keep our form-level Tab handling. Without the Root, Tab is not handled by anything else (checked).

### Scratchpad → (already the kit `Textarea`)

Keep.

### Widget strip → (kit `Scrollbar` already)

Tiles render plugin frames through our own node, canvas and image code, with the bounded image loader. There is no kit equivalent. **Keep ours.**

### Icons and monograms → `Icon`, `Avatar`

Our `icon_element` loads plugin files only through the bounded loader (`plugin_image::cached`), never `img(path)`, and resolves theme icons off the UI thread. Kit `Avatar` and image-based icons take a URI or path straight to `img`. **Keep ours.**

## Mount check: kit controls in a bare window

A throwaway in-crate test, not committed, mounted kit `Checkbox`, `Switch` and `Select` in a test window. A test window is a bare `open_window`, as the launcher's is. The view had our `Launcher` key context and `Dismiss`/`SubmitForm` handlers, and `tomakast_ui::init`'s bindings. Key down and key up were simulated.

| Step | Result |
|---|---|
| Theme font size / window rem | 16 px / 16 px |
| `focus_next` from nothing | focuses the checkbox |
| Space, Space on the checkbox | on, then off |
| Space, Enter, Space on the checkbox | on, off, on: Enter toggles it too (the host's `Confirm` is unhandled here) |
| Space, Tab, Space with no Tab handler | on, focus stays on the checkbox, off |
| Escape on the checkbox | reaches our `Dismiss` (the view would pop) |
| Space on the switch | toggles it |
| Escape on a closed select | reaches our `Dismiss` |
| Ctrl+Enter on a closed select | **opens the popup; our `SubmitForm` never runs** |
| Escape with the popup open | closes the popup and does not reach `Dismiss` |
| Second Escape | reaches `Dismiss` |
| Down, Down, Enter on the select | picks the second option |
| With Ctrl+Enter → `SubmitForm` bound in `Select`: Ctrl+Enter on a closed select | `SubmitForm` runs |
| Same binding, popup open: Down, Down, Ctrl+Enter | picks the second option and closes the popup; `SubmitForm` does not run |

## Pieces we will need soon

### Plugin preferences screen

Needed field types: text, password, number, checkbox or switch, dropdown, file path, and maybe a date, a multi-select or a colour.

| Field type | Kit component | Works in the launcher window |
|---|---|---|
| Text | `Input` | yes (in use) |
| Password | `InputState::masked(true)` (+ mask toggle) | yes |
| Number | `NumberInput` (step, min and max via the state) | yes |
| Checkbox or switch | `Checkbox`, `Switch` | yes (checked) |
| Dropdown | `Select` | yes, with the Ctrl+Enter fix |
| Multi-select | `Combobox::multiple` | yes (the same popup machinery as Select; not mounted) |
| File path | `Input` + a button calling GPUI's `prompt_for_paths` | yes; the kit has no file field |
| Colour | `ColorPicker` | popup via `Popover`; not mounted |
| Date | masked `Input` (see the form) | yes |

The kit's `Settings` framework (`C/setting/`) is a whole-window layout:

- a resizable sidebar with search, pages and groups, and items with title, description and a control;
- it binds data through `value`/`set_value` closures.

It does not fit a pushed launcher view:

- pages, groups and items render only inside `Settings`;
- it has no password or file field types (an unknown type hits `unimplemented!`, `C/setting/item.rs:246`);
- the text field re-reads its value on every render, so our "an edited field is not overwritten" rule would have to live in the closures;
- its dropdown is a menu, not a `Select`.

It does fit a separate settings window.

### Plugin manager

- **In the launcher:** our list view with an action panel (enable, disable, review, remove) already fits. There is no kit component to add, and a confirmation would be a pushed detail view, since kit dialogs need the Root.
- **In a separate window:** `DataTable` (virtualized rows and columns, with its own arrow and Home/End keys), `Switch` per row, `Button`, `Tabs`, `GroupBox`, and the kit `Dialog` for confirming a removal. All of these work there.

## Upgrade cost

The core spec notes that gpui-component breaks on every minor release (0.6 → 0.10 in two months). Everything we draw ourselves depends only on GPUI's element API, which the kit re-exports unchanged. Every kit component we adopt adds surface that an upgrade can change:

- its API;
- its defaults (paddings, motion tokens, colours);
- its key bindings, which are the source of the Select/Ctrl+Enter conflict.

The recommended form controls are a small, contained addition. Swapping the list or the action panel would put the launcher's most-used paths on that churn.

## Recommendation and order of work

1. **Keep the launcher's core ours.** That covers the list, rows, chrome, chips, action panel, toasts, markdown, detail and review views, and the strip. It is cheaper, faster, matches the agreed look, and keeps the untrusted-content rules.
2. **Form controls:**
   - Checkbox (0.5 day).
   - Masked, validated date input (0.5 day).
   - Select for the dropdown, with Ctrl+Enter bound to submit in the `Select` context (1–1.5 days).
   - Kit theme tokens (`primary`, `popover`, `input`) set from our palette, so the controls match the panel.
   - **Done on this branch (2026-11-20):** `768acad` checkbox, `d04fca8` select, `6918614` date mask. The checkbox is the base (unstyled) kit checkbox drawn with our tokens, not the styled one: the styled one keeps its own focus handle, which the form cannot move. The select needed two fixes besides the submit binding. Tab out of an open list closes it first, as Escape does; the kit closes it only when its field loses focus, not its list. Its popup takes our popover, text and selection colours (`theme::paint_kit_popups`). Behaviour changes: Up or Down on a closed dropdown opens it, and a value that is no longer an option opens the list with nothing highlighted.
3. **When the plugin WIT gains new field types**, use the kit components from the preferences table: password, number, switch, multi-select.
4. **Decided (2026-11-20): plugin preferences and the plugin manager live in a separate settings window**, opened through the kit, so `Settings`, `DataTable`, `Dialog`, notifications and tooltips are available there. The user also chose all three form swaps from step 2. The original question was where preferences and the plugin manager should live. If it is a separate settings window, the kit's `Settings`, `DataTable` and `Dialog` become available there, at the cost of a second window kind on every OS. If it is the launcher panel, build them from our views plus the form controls from step 2.
5. **Only if** the launcher itself needs kit dialogs, tooltips or notifications, add the layer-shell decorations hunk to `third_party/gpui-pre-linux` and the transparent Root background, then verify on macOS and Windows.

## Method notes

- The kit code was read in place, partly by two read-only helper passes whose load-bearing claims were re-checked against the source. These were Select's Escape and Ctrl+Enter handling, the Root dependencies, the settings framework's render visibility and `unimplemented!`, and the positioner's clamp. Only Checkbox, Switch, Select and List were run. Claims about Settings, DataTable, Combobox, ColorPicker, Command, Notification and the rest come from reading.
- The benchmark and mount checks were throwaway `#[cfg(test)]` modules inside `crates/ui` (they need the crate-private `views::row`). They were deleted after the runs; the numbers above are the record.
  - Benchmark: `cargo test --release -p tomakast-ui --lib kit_bench -- --nocapture --test-threads=1`.
  - Mount check: `cargo test -p tomakast-ui --lib kit_mount -- --nocapture`.
