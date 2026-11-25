//! Launcher design tokens: sizes and colors. The light or dark mode follows
//! the system appearance reported by the launcher window, through the
//! gpui-component theme; error and success colors come from that theme.

use std::cell::Cell;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{
    App, BoxShadow, FontFeatures, Global, Hsla, Pixels, Window, black, hsla, px, rgb, white,
};
use tomakast_core::ThemeChoice;

/// Size of the launcher panel, in logical px, without the shadow margin.
pub(crate) const PANEL_SIZE: (f32, f32) = (750., 474.);
/// Width of the launcher panel's border.
pub(crate) const PANEL_BORDER: f32 = 1.;
/// Corner radius of the launcher panel.
pub(crate) const PANEL_RADIUS: f32 = 12.;
/// Transparent margin around the panel that holds its shadow, on Wayland;
/// see [`shadow_margin`]. It holds the whole of every [`SHADOW_LAYERS`]
/// layer, or the shadow is cut off at the surface edge.
const SHADOW_MARGIN: f32 = 44.;
/// Height of the header with the search input or view title.
pub(crate) const HEADER_HEIGHT: f32 = 56.;
/// Height of the footer with the context and the actions.
pub(crate) const FOOTER_HEIGHT: f32 = 40.;
/// Space between the footer's key hints (the primary action, a list's footer action,
/// Actions).
pub(crate) const FOOTER_HINT_GAP: f32 = 20.;
/// Height and minimum width of a key chip.
pub(crate) const CHIP_SIZE: f32 = 20.;
/// Horizontal padding inside a key chip.
pub(crate) const CHIP_PADDING_X: f32 = 4.;
/// Text size of the header's search input and view title.
pub(crate) const HEADER_TEXT_SIZE: f32 = 18.;
/// Text size of key chips, the footer context and toasts.
pub(crate) const SMALL_TEXT_SIZE: f32 = 12.;
/// Thickness of the dividers.
pub(crate) const DIVIDER_THICKNESS: f32 = 1.;
/// Diameter of the toast's colored dot.
pub(crate) const TOAST_DOT_SIZE: f32 = 8.;
/// Corner radius of a key chip.
pub(crate) const CHIP_RADIUS: f32 = 5.;
/// Height of a list row.
pub(crate) const ROW_HEIGHT: f32 = 40.;
/// Size of a row's icon; an emoji icon is drawn as text in a square this size.
pub(crate) const ICON_SIZE: f32 = 20.;
/// Corner radius of a colour swatch icon, as a fraction of its size.
pub(crate) const SWATCH_RADIUS_FRACTION: f32 = 0.2;
/// Font size of an emoji icon, as a fraction of its square (whose line
/// height it keeps), so the glyph's own extent is not clipped.
pub(crate) const EMOJI_GLYPH_FRACTION: f32 = 0.8;
/// Size of a bundled (Lucide) glyph, as a fraction of its icon square.
pub(crate) const GLYPH_FRACTION: f32 = 0.8;
/// Font size of a monogram's letter, as a fraction of its square.
pub(crate) const MONOGRAM_LETTER_FRACTION: f32 = 0.55;
/// Height of a section label.
pub(crate) const SECTION_HEIGHT: f32 = 28.;
/// Space between a row's highlight and the list edges.
pub(crate) const ROW_GUTTER: f32 = 8.;
/// Corner radius of a row's highlight.
pub(crate) const ROW_RADIUS: f32 = 8.;
/// Horizontal padding inside a row's highlight, before the icon and after
/// the accessories. With [`ROW_GUTTER`] it lines the icons and section
/// labels up with the footer text.
pub(crate) const ROW_PADDING_X: f32 = 8.;
/// Space between a row's icon and its title.
pub(crate) const ROW_ICON_GAP: f32 = 12.;
/// Space between a row's title and its subtitle.
pub(crate) const ROW_SUBTITLE_GAP: f32 = 8.;
/// Space between the row's text and its accessories, and between accessories.
pub(crate) const ROW_ACCESSORY_GAP: f32 = 8.;
/// How readily the title and the subtitle give up width when a row is too
/// narrow (flex shrink factors): the subtitle truncates first.
pub(crate) const TITLE_SHRINK: f32 = 1.;
pub(crate) const SUBTITLE_SHRINK: f32 = 1000.;
/// Text size of a row's title and subtitle.
pub(crate) const ROW_TEXT_SIZE: f32 = 14.;
/// Text size of a row's accessories.
pub(crate) const ACCESSORY_TEXT_SIZE: f32 = 13.;
/// Horizontal padding of a tag accessory's pill.
pub(crate) const TAG_PADDING_X: f32 = 6.;
/// Width of the action panel popover.
pub(crate) const POPOVER_WIDTH: f32 = 320.;
/// Tallest the action panel popover grows; its action list scrolls past it.
pub(crate) const POPOVER_MAX_HEIGHT: f32 = 320.;
/// Space between the popover and the footer above which it floats, and
/// between the popover and the panel's right edge.
pub(crate) const POPOVER_MARGIN: f32 = 8.;
/// Corner radius of the action panel popover.
pub(crate) const POPOVER_RADIUS: f32 = 10.;
/// Padding around the popover's action list.
pub(crate) const POPOVER_PADDING: f32 = 4.;
/// Height of an action row in the popover.
pub(crate) const ACTION_HEIGHT: f32 = 32.;
/// Horizontal padding inside an action row.
pub(crate) const ACTION_PADDING_X: f32 = 8.;
/// Corner radius of an action row's highlight.
pub(crate) const ACTION_RADIUS: f32 = 6.;
/// Horizontal padding of the popover's filter input.
pub(crate) const POPOVER_INPUT_PADDING_X: f32 = 12.;
/// Height of the popover's filter input row.
pub(crate) const POPOVER_INPUT_HEIGHT: f32 = 40.;
/// The left edge of all content: the header and footer padding, the root
/// row's icon (gutter plus padding) and the pushed views' padding.
pub(crate) const EDGE_PADDING: f32 = 16.;
/// Inner padding of detail, form and scratchpad views and the list's detail pane.
pub(crate) const VIEW_PADDING: f32 = EDGE_PADDING;
const _: () = assert!(ROW_GUTTER + ROW_PADDING_X == EDGE_PADDING);
/// How long a success toast stays.
pub(crate) const SUCCESS_TTL: Duration = Duration::from_secs(3);
/// How long an error toast stays.
pub(crate) const ERROR_TTL: Duration = Duration::from_secs(5);
/// How long the toast that offers to restore a removed Home tile stays: long
/// enough to read its keys and press them.
pub(crate) const UNDO_TOAST_TTL: Duration = Duration::from_secs(5);
/// Width of the list view's detail pane.
pub(crate) const DETAIL_WIDTH: f32 = 280.;
/// Size of a grid tile's image; an emoji or swatch tile image too.
pub(crate) const IMAGE_SIZE: f32 = 64.;
/// Height of a grid tile.
pub(crate) const TILE_HEIGHT: f32 = 104.;
/// Width of the scratchpad's annotation column, as a fraction of the view.
pub(crate) const COLUMN_WIDTH: f32 = 0.3;
/// Where the kit's multi-line editor (medium size) starts its text inside its
/// own box: left and top. The borderless scratchpad editor takes them off its
/// view padding, so its text lines up with the rest of the panel.
pub(crate) const EDITOR_TEXT_INSET: (f32, f32) = (10., 8.);
/// Text size of the body of detail, form and scratchpad views.
pub(crate) const BODY_TEXT_SIZE: f32 = 14.;
/// Text size of muted text in those views (metadata keys, form labels, errors).
pub(crate) const MUTED_TEXT_SIZE: f32 = 13.;
/// Text size of a module's error under a form field.
pub(crate) const FIELD_ERROR_TEXT_SIZE: f32 = 12.;
/// Width of the form's label column and of the detail metadata's key column.
pub(crate) const LABEL_COLUMN_WIDTH: f32 = 120.;
/// Space between a form's label column and its control.
pub(crate) const LABEL_GAP: f32 = 12.;
/// Space between a form's fields, and between a detail view's blocks.
pub(crate) const VIEW_GAP: f32 = 8.;
/// Corner radius of a grid tile.
pub(crate) const TILE_RADIUS: f32 = 8.;
/// Corner radius of form controls (the submit button and the dropdown list).
pub(crate) const CONTROL_RADIUS: f32 = 6.;
/// Height of a single-line form control: the kit's medium input height. The
/// form's labels, checkboxes and dropdowns take it too, so a label sits on
/// the line of its control's text.
pub(crate) const CONTROL_HEIGHT: f32 = 32.;
/// Space between a text field's edge and its text: the kit's medium input's.
pub(crate) const CONTROL_PADDING_X: f32 = 10.;
/// Where the text inside a text field starts, from the field's left: its 1 px edge and its
/// padding. A hotkey field's keys start there too, and so does a table's column title over
/// such fields.
pub(crate) const CONTROL_TEXT_INSET: f32 = 1. + CONTROL_PADDING_X;
/// Size of a form checkbox's box.
pub(crate) const CHECKBOX_SIZE: f32 = 16.;
/// Corner radius of a form checkbox's box.
pub(crate) const CHECKBOX_RADIUS: f32 = 4.;
/// Text size of a grid tile's title.
pub(crate) const TILE_TITLE_SIZE: f32 = 12.;
/// Most items the root list shows under "Suggestions".
pub(crate) const SUGGESTIONS_MAX: usize = 8;
/// Height of a widget tile: three 40 px rows.
pub(crate) const STRIP_TILE_HEIGHT: f32 = 120.;
/// Space between two widget tiles.
pub(crate) const STRIP_GAP: f32 = 8.;
/// Largest offset, in px, of a jiggling tile in the Home edit mode.
pub(crate) const EDIT_WOBBLE_PX: f32 = 1.;
/// Time of one jiggle of a tile in the Home edit mode.
pub(crate) const EDIT_WOBBLE_PERIOD: Duration = Duration::from_millis(300);
/// Opacity of the search field while the Home edit mode lasts: dimmed, as it
/// takes no typing then.
pub(crate) const EDIT_DIM_OPACITY: f32 = 0.4;
/// Size of the remove badge on a tile in the Home edit mode.
pub(crate) const EDIT_BADGE_SIZE: f32 = 18.;
/// Distance of the remove badge from the tile's top and left edges.
pub(crate) const EDIT_BADGE_INSET: f32 = 4.;
/// How long a press on a Home tile lasts before it enters the Home edit mode.
pub(crate) const EDIT_LONG_PRESS: Duration = Duration::from_millis(500);
/// How far, in px, a pressed pointer may move and still be a press: farther, a press
/// outside the Home edit mode is no long press, and one in the mode starts a drag.
pub(crate) const EDIT_DRAG_SLOP: f32 = 4.;
/// Width of the zone at each edge of the strip where a dragged tile scrolls the strip.
pub(crate) const EDIT_EDGE_ZONE: f32 = 24.;
/// How far the strip scrolls each frame while a dragged tile is at its edge.
pub(crate) const EDIT_EDGE_STEP: f32 = 8.;
/// Space above and below the widget strip.
pub(crate) const STRIP_PADDING_Y: f32 = 8.;
/// Corner radius of a widget tile.
pub(crate) const STRIP_TILE_RADIUS: f32 = 10.;
/// Width of a focused widget tile's ring.
pub(crate) const FOCUS_RING_WIDTH: f32 = 2.;
/// How far inside its row a ring drawn within a row's edges starts.
pub(crate) const FOCUS_RING_INSET: f32 = 1.;
/// The kit's focus ring: its translucent band's width and opacity around the ring line.
pub(crate) const FOCUS_BAND_WIDTH: f32 = 3.;
pub(crate) const FOCUS_BAND_OPACITY: f32 = 0.5;
/// How long a popover of the launcher's own takes to come in: the kit's lists' time.
pub(crate) const POPOVER_ENTER_DURATION: std::time::Duration =
    std::time::Duration::from_millis(150);
/// How far above its place it starts, sliding down: the kit's lists' travel.
pub(crate) const POPOVER_ENTER_OFFSET: f32 = -8.;
/// Text sizes of widget text styles: title, value, body, caption.
pub(crate) const WIDGET_TEXT_SIZES: (f32, f32, f32, f32) = (16., 28., 14., 12.);
/// Width of the settings window's sidebar.
pub(crate) const SETTINGS_SIDEBAR_WIDTH: f32 = 240.;
/// The icon before a dropdown's label (the theme's monitor, sun or moon).
pub(crate) const SETTINGS_CHOICE_ICON_SIZE: f32 = 16.;
/// The settings window's one margin, the sidebar's: from a column's edge to what stands in it
/// (the search field and the sidebar's entries; the page's title and boxes), and from the
/// title area down to the first of them, so the first entry and the first box start on one
/// line.
pub(crate) const SETTINGS_EDGE: f32 = ROW_GUTTER;
/// Height of the settings window's title area: the band with the search field over the
/// sidebar and the page title over the page.
pub(crate) const SETTINGS_TITLE_HEIGHT: f32 = 48.;
/// Width of every one-line control of a settings row: a text or number field, a dropdown of
/// any length (a theme, a unit, a widget's size, a city, a time zone, a language, a plugin's
/// choice) and a hotkey field; also the right column of a settings table (the Hotkeys page's,
/// a plugin's commands), so its title starts where the fields do. A value too long for it is
/// cut: a dropdown's with an ellipsis, a hotkey's keys with a fade (and the whole combination in
/// a tooltip). Every combination of up to two modifiers fits a hotkey field
/// (`common_combinations_fit_the_field`).
pub(crate) const SETTINGS_CONTROL_WIDTH: f32 = 220.;
/// Height of the line with the titles of a settings table's columns.
pub(crate) const SETTINGS_COLUMNS_HEIGHT: f32 = 30.;
/// Height of a group row of a settings table (one for each source on the Hotkeys page): a
/// little more than an item row's ([`SETTINGS_ROW_HEIGHT`]).
pub(crate) const SETTINGS_TABLE_GROUP_HEIGHT: f32 = 52.;
/// Size of the chevron of a settings table's group row.
pub(crate) const SETTINGS_TABLE_CHEVRON_SIZE: f32 = 14.;
/// Size of the source's icon in a settings table's group row; also the room it takes when the
/// group has none.
pub(crate) const SETTINGS_TABLE_SOURCE_ICON_SIZE: f32 = 16.;
/// Space between the chevron, the icon, the title and the count of a group row.
pub(crate) const SETTINGS_TABLE_GROUP_GAP: f32 = 8.;
/// How far in from a group row's chevron the rows of its group start: an item's icon starts
/// where the group's title does.
pub(crate) const SETTINGS_TABLE_INDENT: f32 = SETTINGS_TABLE_CHEVRON_SIZE
    + SETTINGS_TABLE_GROUP_GAP
    + SETTINGS_TABLE_SOURCE_ICON_SIZE
    + SETTINGS_TABLE_GROUP_GAP;
/// Size of the icon of an entry of the settings sidebar.
pub(crate) const SETTINGS_SIDEBAR_ICON_SIZE: f32 = 18.;
/// Space between two groups of a settings page, and between the sidebar's sections (the
/// launcher's pages, the plugins'), which have no titles.
pub(crate) const SETTINGS_GROUP_GAP: f32 = 20.;
/// Space above and below a row of a settings page.
pub(crate) const SETTINGS_ROW_PADDING_Y: f32 = 8.;
/// Least height of a row of a settings page.
pub(crate) const SETTINGS_ROW_HEIGHT: f32 = 48.;
/// Horizontal padding inside a settings group's box, before a row's title and after its
/// control; the group's heading and description line up with the titles.
pub(crate) const SETTINGS_ROW_PADDING_X: f32 = 14.;
/// Where the text of a settings box's rows starts from the box's edge (its 1 px border and the
/// rows' padding): a group's heading and a page's notes, outside any box, start there too.
pub(crate) const SETTINGS_TEXT_INSET: f32 = 1. + SETTINGS_ROW_PADDING_X;
/// The settings window's error banner: the gap after its icon, the height of its first
/// line (the icon is centred on it), and how strongly the error colour tints its edge and fill.
pub(crate) const BANNER_ICON_GAP: f32 = 10.;
pub(crate) const BANNER_LINE_HEIGHT: f32 = 20.;
pub(crate) const BANNER_BORDER_OPACITY: f32 = 0.35;
pub(crate) const BANNER_FILL_OPACITY: f32 = 0.08;
/// Room the page title keeps clear at its right for the window's controls, in the narrowest
/// window (Windows' three 46 px buttons, flush with the edge).
pub(crate) const SETTINGS_TITLE_CONTROLS_ROOM: f32 = 120.;
/// Corner radius of a settings group's box.
pub(crate) const SETTINGS_GROUP_RADIUS: f32 = 10.;
/// Space between a settings group's heading (and description) and its box.
pub(crate) const SETTINGS_HEADING_GAP: f32 = 8.;
/// Space between a row's title and its description.
pub(crate) const SETTINGS_DESCRIPTION_GAP: f32 = 2.;
/// Widest a settings page's groups grow; a wider window centres them.
pub(crate) const SETTINGS_PAGE_MAX_WIDTH: f32 = 680.;
/// Text size of the page title over a settings page.
pub(crate) const SETTINGS_PAGE_TITLE_SIZE: f32 = 15.;
/// Height of an entry of the settings sidebar.
pub(crate) const SETTINGS_ENTRY_HEIGHT: f32 = 32.;
/// Space between an entry's icon and its title in the settings sidebar.
pub(crate) const SETTINGS_ENTRY_ICON_GAP: f32 = 10.;
/// Widest the path of the Config file row grows; a longer one is cut at its start.
pub(crate) const SETTINGS_CONFIG_PATH_WIDTH: f32 = 260.;
/// Opacity of a settings row that cannot be changed, and of a greyed sidebar icon.
pub(crate) const DISABLED_OPACITY: f32 = 0.5;
/// Corner radius of the settings window when it draws its own frame (client-side
/// decorations on Linux); the kit has one radius for all four corners.
pub(crate) const SETTINGS_WINDOW_RADIUS: f32 = 10.;
/// Where macOS draws the settings window's traffic lights, from its top-left corner.
pub(crate) const SETTINGS_TRAFFIC_LIGHT_POSITION: (f32, f32) = (12., 14.);
/// Room left of the settings search field for the macOS traffic lights.
pub(crate) const SETTINGS_TRAFFIC_LIGHT_INSET: f32 = 80.;
/// Size of a Breeze-like window control (KDE and unknown desktops): a 22 px square in the
/// 48 px title area.
pub(crate) const BREEZE_BUTTON_SIZE: f32 = 22.;
/// The share of a Breeze button its glyph box and its hover circle take, centred (Breeze
/// draws an 18-unit glyph box in a 20-unit button).
pub(crate) const BREEZE_GLYPH_SHARE: f32 = 0.9;
/// Space between two Breeze window controls.
pub(crate) const BREEZE_BUTTON_GAP: f32 = 6.;
/// Space between the window's edge and the nearest Breeze window control: the same as the
/// gap from the title area's top to the buttons, which are centred vertically.
pub(crate) const BREEZE_EDGE_MARGIN: f32 =
    (SETTINGS_TITLE_HEIGHT - TITLE_BORDER - BREEZE_BUTTON_SIZE) / 2.;
/// The hairline under the settings title area, inside its height.
pub(crate) const TITLE_BORDER: f32 = 1.;
/// Size of a round, filled Adwaita-like window control (GNOME and GTK desktops).
pub(crate) const ADWAITA_BUTTON_SIZE: f32 = 24.;
/// Width of a Windows 11-style window control; it is as tall as the title area.
pub(crate) const WINDOWS_BUTTON_WIDTH: f32 = 46.;
/// Space between two round window controls.
pub(crate) const FRAME_BUTTON_GAP: f32 = 8.;
/// Size of a window control's glyph.
pub(crate) const FRAME_GLYPH_SIZE: f32 = 14.;
/// Fill of a hovered Windows 11-style close button.
pub(crate) const WINDOWS_CLOSE_HOVER: u32 = 0xc42b1c;

/// Opacity of the panel fill, dark and light; the blurred desktop shows through it.
const PANEL_OPACITY: (f32, f32) = (0.78, 0.80);
/// Opacity of the action panel popover fill: opaque, so no row under it shows
/// through (the spec's 0.92 left ghost text).
const POPOVER_OPACITY: f32 = 1.0;
/// Panel fill color, dark and light.
const PANEL_FILL: (u32, u32) = (0x1c1c1e, 0xf6f6f6);
/// Lightness of the text color, dark and light.
const TEXT_LIGHTNESS: (f32, f32) = (0.96, 0.10);
/// Opacity of the muted text relative to the text color, dark and light: on
/// the panel fill this gives 5.6:1 (dark) and 4.8:1 (light) contrast; 0.55
/// on light was 3.8:1.
const MUTED_OPACITY: (f32, f32) = (0.55, 0.62);
/// Opacity of the overlay tints (white on dark, black on light), dark and light.
const BORDER_OPACITY: (f32, f32) = (0.10, 0.10);
const DIVIDER_OPACITY: (f32, f32) = (0.06, 0.06);
const SELECTED_OPACITY: (f32, f32) = (0.08, 0.06);
const CHIP_OPACITY: (f32, f32) = (0.08, 0.06);
/// A hovered row that is not selected: half the selection's tint, so the two never look alike.
const HOVER_OPACITY: (f32, f32) = (0.05, 0.04);
/// The settings sidebar's selected entry: a firmer tint than a list's selection, as it marks
/// the page on display rather than a cursor.
const SIDEBAR_SELECTED_OPACITY: (f32, f32) = (0.10, 0.08);
/// The settings window's opaque surfaces, dark and light: the sidebar (a shade apart from the
/// page, darker on dark, greyer on light), and the boxes of a page's groups (lifted off the
/// page: lighter on dark, white on light).
const SIDEBAR_FILL: (u32, u32) = (0x141416, 0xebebed);
const GROUP_FILL: (u32, u32) = (0x252528, 0xffffff);
/// Edge of a settings group's box (white on dark, black on light).
const GROUP_BORDER_OPACITY: (f32, f32) = (0.06, 0.09);
/// Fill of the form's text fields, instead of the kit's opaque input background.
const FIELD_OPACITY: (f32, f32) = (0.05, 0.04);
/// The chrome bands (a view's title bar, a pad's toolbar, the footer): black over the panel,
/// so they sit apart from the fields by their shade, not only by a hairline.
const CHROME_OPACITY: (f32, f32) = (0.22, 0.045);
/// Panel shadow layers, black: (offset y, blur radius, opacity). GPUI's blur
/// radius is the Gaussian's sigma, half a CSS blur: these are CSS
/// `0 8px 24px` and `0 1px 4px`. A layer reaches 3 × blur + offset past the
/// panel, and [`SHADOW_MARGIN`] must hold that.
const SHADOW_LAYERS: [(f32, f32, f32); 2] = [(8., 12., 0.35), (1., 2., 0.20)];
/// Error and success colors of [`palette_for`], where no kit theme is at hand.
const FALLBACK_ERROR: Hsla = hsla(0., 0.72, 0.51, 1.);
const FALLBACK_SUCCESS: Hsla = hsla(142. / 360., 0.71, 0.45, 1.);

/// The shadow margin for this session: only a Wayland layer surface draws its
/// own shadow (with the margin left out of the blur). X11, macOS and Windows
/// get no margin and the platform's shadow, if any.
pub(crate) fn shadow_margin() -> f32 {
    shadow_margin_for(crate::window::is_wayland())
}

fn shadow_margin_for(wayland: bool) -> f32 {
    if wayland { SHADOW_MARGIN } else { 0. }
}

/// The OpenType features of monospace text: JetBrains Mono's ligatures
/// (contextual alternates, `calt`) and standard ligatures (`liga`), on. They
/// are on by default too; naming them keeps them on whatever the default.
pub(crate) fn mono_features() -> FontFeatures {
    static FEATURES: LazyLock<FontFeatures> =
        LazyLock::new(|| FontFeatures(Arc::new(vec![("calt".into(), 1), ("liga".into(), 1)])));
    FEATURES.clone()
}

thread_local! {
    /// The factor of every text size (`[appearance] text_size`). One for the UI thread: the
    /// launcher and the settings window share it (and each test's app has its own).
    static TEXT_SCALE: Cell<f32> = const { Cell::new(1.) };
    /// The size of monospace text, in pixels (`[appearance] mono_size`).
    static MONO_SIZE: Cell<u8> = const { Cell::new(14) };
}

/// The text size factor in effect.
pub(crate) fn text_scale() -> f32 {
    TEXT_SCALE.get()
}

/// `size` (one of the text sizes above) at the user's text size.
pub(crate) fn text(size: f32) -> Pixels {
    px(size * text_scale())
}

/// The size of monospace text: sheets and code blocks.
pub(crate) fn mono_text() -> Pixels {
    px(f32::from(MONO_SIZE.get()))
}

/// The size of the notes beside monospace text (a sheet's results): a pixel under it.
pub(crate) fn mono_note_text() -> Pixels {
    mono_text() - px(1.)
}

/// [`SETTINGS_CONTROL_WIDTH`] at the user's text size: controls stay one width and fit their
/// text.
pub(crate) fn settings_control_width() -> f32 {
    (SETTINGS_CONTROL_WIDTH * text_scale()).round()
}

/// [`SETTINGS_SIDEBAR_WIDTH`] at the user's text size.
pub(crate) fn settings_sidebar_width() -> f32 {
    (SETTINGS_SIDEBAR_WIDTH * text_scale()).round()
}

/// Sets the text sizes in effect; returns whether they changed.
pub(crate) fn set_text_sizes(scale: f32, mono: u8) -> bool {
    let scale_changed = TEXT_SCALE.replace(scale) != scale;
    let mono_changed = MONO_SIZE.replace(mono) != mono;
    scale_changed || mono_changed
}

/// Dev-only: forces the light mode whatever the system says (see the crate docs).
const DEV_LIGHT_ENV: &str = "TOMAKAST_DEV_LIGHT";

/// The user's theme choice (`[appearance] theme`), as a global.
struct ThemePreference(ThemeChoice);

impl Global for ThemePreference {}

fn preference(cx: &App) -> ThemeChoice {
    cx.try_global::<ThemePreference>()
        .map_or(ThemeChoice::System, |p| p.0)
}

/// Stores the user's theme choice and, when it changed, switches every open
/// window to it. With no window open the launcher applies it as it shows.
pub fn set_theme_preference(choice: ThemeChoice, cx: &mut App) {
    if preference(cx) == choice {
        return;
    }
    cx.set_global(ThemePreference(choice));
    // The theme is global: one window's switch refreshes them all.
    if let Some(window) = cx.windows().into_iter().next() {
        let _ = window.update(cx, |_, window, cx| sync(window, cx));
    }
}

/// Switches the theme to the user's choice, or the window's appearance. `window.appearance()` is
/// used rather than `cx.window_appearance()`, which is unreliable on Linux.
/// The theme is only changed when the mode differs, because a change
/// refreshes every window.
pub(crate) fn sync(window: &Window, cx: &mut App) {
    let mode = mode_for(
        preference(cx),
        ThemeMode::from(window.appearance()),
        crate::dev_flag(DEV_LIGHT_ENV),
    );
    if cx.theme().mode.is_dark() != mode.is_dark() {
        Theme::change(mode, None, cx);
    }
    paint_kit_popups(cx);
}

/// Paints the kit's popups (the form dropdown's list of options) like the
/// action panel: our popover fill, text and selection colours. A mode
/// change resets the kit's colours, so [`sync`] calls it after each, and
/// [`crate::init`] once; the theme is only written when a colour differs,
/// because a write refreshes every window.
pub(crate) fn paint_kit_popups(cx: &mut App) {
    let palette = palette_for(cx.theme().mode.is_dark());
    let theme = cx.theme();
    let wanted = [
        palette.popover,
        palette.text,
        palette.selected,
        palette.text,
        palette.selected,
        palette.selected,
    ];
    let current = [
        theme.popover,
        theme.popover_foreground,
        theme.accent,
        theme.accent_foreground,
        theme.list_active,
        theme.list_hover,
    ];
    if wanted == current {
        return;
    }
    Theme::update(cx, |theme| {
        theme.popover = palette.popover;
        theme.popover_foreground = palette.text;
        theme.accent = palette.selected;
        theme.accent_foreground = palette.text;
        theme.list_active = palette.selected;
        theme.list_hover = palette.selected;
    });
}

/// Rounds the corners of kit windows that draw their own frame to
/// [`SETTINGS_WINDOW_RADIUS`]. Only the kit's Root reads the radius, and only the settings
/// window has one (the launcher is opened without it), so the launcher is unaffected. The
/// theme is only written when the radius differs, because a write refreshes every window.
pub(crate) fn round_kit_windows(cx: &mut App) {
    let radius = px(SETTINGS_WINDOW_RADIUS);
    if cx.theme().window_radius != radius {
        Theme::update(cx, |theme| theme.window_radius = radius);
    }
}

/// The theme mode: light when forced by `dev_light`, else the user's choice,
/// which `System` leaves to the system's mode.
fn mode_for(choice: ThemeChoice, system: ThemeMode, dev_light: bool) -> ThemeMode {
    if dev_light {
        return ThemeMode::Light;
    }
    match choice {
        ThemeChoice::System => system,
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
    }
}

/// Colors the launcher view paints with.
#[derive(Clone, PartialEq)]
pub(crate) struct Palette {
    pub panel: Hsla,
    pub border: Hsla,
    pub divider: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub selected: Hsla,
    /// A hovered row that is not selected.
    pub hover: Hsla,
    pub chip: Hsla,
    pub field: Hsla,
    /// The fill of the chrome bands: darker than the panel in both themes.
    pub chrome: Hsla,
    pub popover: Hsla,
    /// The settings window's sidebar, its selected entry, and the boxes of its pages' groups
    /// with their edge.
    pub sidebar: Hsla,
    pub sidebar_selected: Hsla,
    pub group: Hsla,
    pub group_border: Hsla,
    pub shadow: [BoxShadow; 2],
    pub error: Hsla,
    pub success: Hsla,
}

/// The fills of a filled control button of the settings window (a dropdown, Remove), each one
/// opaque, as the control looks over the group box it sits in, so that a translucent tint
/// cannot read darker than what is under it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ControlFills {
    pub rest: Hsla,
    /// The rest fill with the window's hover tint added: lighter on dark, darker on light.
    pub hover: Hsla,
    /// A step further the same way: the hover tint twice.
    pub active: Hsla,
    /// The edge of the control, opaque too.
    pub border: Hsla,
}

impl Palette {
    /// The fills of a control button over the group box.
    pub(crate) fn control_fills(&self) -> ControlFills {
        let rest = self.group.blend(self.field);
        let hover = rest.blend(self.hover);
        ControlFills {
            rest,
            hover,
            active: hover.blend(self.hover),
            border: self.group.blend(self.border),
        }
    }
}

/// The palette for the current theme mode.
pub(crate) fn palette(cx: &App) -> Palette {
    let theme = cx.theme();
    Palette {
        error: theme.danger,
        success: theme.success,
        ..palette_for(theme.mode.is_dark())
    }
}

/// The palette for dark or light mode. Its error and success colors are
/// fallbacks; [`palette`] takes them from the kit theme.
pub(crate) fn palette_for(dark: bool) -> Palette {
    let pick = |(d, l): (f32, f32)| if dark { d } else { l };
    // Overlay tints: white on dark, black on light.
    let tint = |opacity| {
        let base = if dark { white() } else { black() };
        base.alpha(pick(opacity))
    };
    let fill = Hsla::from(rgb(if dark { PANEL_FILL.0 } else { PANEL_FILL.1 }));
    let text = hsla(0., 0., pick(TEXT_LIGHTNESS), 1.);
    Palette {
        panel: fill.alpha(pick(PANEL_OPACITY)),
        border: tint(BORDER_OPACITY),
        divider: tint(DIVIDER_OPACITY),
        text,
        muted: text.alpha(pick(MUTED_OPACITY)),
        selected: tint(SELECTED_OPACITY),
        hover: tint(HOVER_OPACITY),
        chip: tint(CHIP_OPACITY),
        field: tint(FIELD_OPACITY),
        chrome: black().alpha(pick(CHROME_OPACITY)),
        popover: fill.alpha(POPOVER_OPACITY),
        sidebar: Hsla::from(rgb(if dark { SIDEBAR_FILL.0 } else { SIDEBAR_FILL.1 })),
        sidebar_selected: tint(SIDEBAR_SELECTED_OPACITY),
        group: Hsla::from(rgb(if dark { GROUP_FILL.0 } else { GROUP_FILL.1 })),
        group_border: tint(GROUP_BORDER_OPACITY),
        shadow: SHADOW_LAYERS.map(|(y, blur, opacity)| {
            BoxShadow::new(px(0.), px(y), black().alpha(opacity)).blur_radius(px(blur))
        }),
        error: FALLBACK_ERROR,
        success: FALLBACK_SUCCESS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chrome bands are darker than the panel, by a shade one can see, in both themes.
    #[test]
    fn chrome_bands_stand_apart() {
        for dark in [true, false] {
            let palette = palette_for(dark);
            assert!(
                palette.chrome.a >= 0.04,
                "dark {dark}: {:?}",
                palette.chrome
            );
            assert_eq!(palette.chrome.l, 0., "a darkening");
        }
    }

    /// A control button lightens on hover and again when pressed in the dark theme, darkens in
    /// the light one, and every fill is opaque.
    #[test]
    fn control_buttons_step_away_from_their_rest_fill_like_the_hover_tint() {
        for dark in [true, false] {
            let fills = palette_for(dark).control_fills();
            let (rest, hover, active) = (fills.rest.l, fills.hover.l, fills.active.l);
            if dark {
                assert!(rest < hover && hover < active, "{rest} {hover} {active}");
            } else {
                assert!(rest > hover && hover > active, "{rest} {hover} {active}");
            }
            for fill in [fills.rest, fills.hover, fills.active, fills.border] {
                assert_eq!(fill.a, 1., "dark {dark}: {fill:?}");
            }
        }
    }

    #[test]
    fn palette_has_light_and_dark_values() {
        let dark = palette_for(true);
        let light = palette_for(false);
        assert_ne!(dark.panel, light.panel);
        assert_ne!(dark.text, light.text);
        assert_ne!(dark.selected, light.selected);
        assert_eq!(dark.panel.a, 0.78);
        assert_eq!(light.panel.a, 0.80);
    }

    /// WCAG contrast of `fg` (with its alpha) over the opaque color `bg`.
    fn contrast(fg: Hsla, bg: Hsla) -> f32 {
        let (fg, bg) = (gpui_kit::Rgba::from(fg), gpui_kit::Rgba::from(bg));
        let lum = |r: f32, g: f32, b: f32| {
            let lin = |c: f32| {
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        };
        let over = |f: f32, b: f32| f * fg.a + b * (1. - fg.a);
        let fg_lum = lum(over(fg.r, bg.r), over(fg.g, bg.g), over(fg.b, bg.b));
        let bg_lum = lum(bg.r, bg.g, bg.b);
        (fg_lum.max(bg_lum) + 0.05) / (fg_lum.min(bg_lum) + 0.05)
    }

    #[test]
    fn muted_text_is_readable_on_the_panel_fill() {
        for dark in [true, false] {
            let palette = palette_for(dark);
            let ratio = contrast(palette.muted, palette.panel.alpha(1.));
            assert!(ratio >= 4.5, "dark = {dark}: muted contrast {ratio:.2}");
        }
    }

    /// The settings window's surfaces stand apart from the page, and its muted text (section
    /// labels, descriptions) reads on each of them.
    #[test]
    fn settings_surfaces_stand_apart_and_stay_readable() {
        for dark in [true, false] {
            let palette = palette_for(dark);
            let page = palette.panel.alpha(1.);
            for (name, surface) in [("sidebar", palette.sidebar), ("group", palette.group)] {
                assert_ne!(surface, page, "dark {dark}: {name} is the page's fill");
                let ratio = contrast(palette.muted, surface);
                assert!(ratio >= 4.5, "dark {dark}: muted on {name} {ratio:.2}");
            }
            assert!(
                palette.hover.a < palette.selected.a,
                "dark {dark}: hover under selection"
            );
            assert!(
                palette.selected.a < palette.sidebar_selected.a,
                "dark {dark}"
            );
        }
    }

    #[test]
    fn mode_follows_the_choice_then_the_system() {
        use ThemeChoice::*;
        assert!(mode_for(System, ThemeMode::Dark, false).is_dark());
        assert!(!mode_for(System, ThemeMode::Light, false).is_dark());
        assert!(mode_for(Dark, ThemeMode::Light, false).is_dark());
        assert!(!mode_for(Light, ThemeMode::Dark, false).is_dark());
        assert!(!mode_for(Dark, ThemeMode::Dark, true).is_dark()); // dev flag wins
    }

    #[test]
    fn shadow_margin_only_on_wayland() {
        assert_eq!(shadow_margin_for(true), SHADOW_MARGIN);
        assert_eq!(shadow_margin_for(false), 0.);
    }

    /// GPUI draws a shadow out to 3 × its blur radius (a Gaussian's sigma)
    /// past the offset bounds (`vs_shadow` in the pinned gpui's shaders). A
    /// layer that reaches past the margin is cut off at the surface edge.
    #[test]
    fn panel_shadow_fits_in_the_margin() {
        for (y, blur, _) in SHADOW_LAYERS {
            let reach = 3. * blur + y.abs();
            assert!(
                reach <= SHADOW_MARGIN,
                "a layer with offset {y} and blur {blur} reaches {reach} px, past the {SHADOW_MARGIN} px margin"
            );
        }
    }
}
