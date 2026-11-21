//! Launcher design tokens: sizes and colors. The light or dark mode follows
//! the system appearance reported by the launcher window, through the
//! gpui-component theme; error and success colors come from that theme.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, BoxShadow, FontFeatures, Hsla, Window, black, hsla, px, rgb, white};

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
/// Height of the vertical divider in the footer.
pub(crate) const DIVIDER_V_HEIGHT: f32 = 16.;
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
/// Text sizes of widget text styles: title, value, body, caption.
pub(crate) const WIDGET_TEXT_SIZES: (f32, f32, f32, f32) = (16., 28., 14., 12.);

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
/// Fill of the form's text fields, instead of the kit's opaque input background.
const FIELD_OPACITY: (f32, f32) = (0.05, 0.04);
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

/// Dev-only: forces the light mode whatever the system says (see the crate docs).
const DEV_LIGHT_ENV: &str = "TOMAKAST_DEV_LIGHT";

/// Switches the theme to the window's appearance. `window.appearance()` is
/// used rather than `cx.window_appearance()`, which is unreliable on Linux.
/// The theme is only changed when the mode differs, because a change
/// refreshes every window.
pub(crate) fn sync(window: &Window, cx: &mut App) {
    let mode = mode_for(
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

/// The theme mode for the system's mode: light when forced by `dev_light`.
fn mode_for(system: ThemeMode, dev_light: bool) -> ThemeMode {
    if dev_light { ThemeMode::Light } else { system }
}

/// Colors the launcher view paints with.
#[derive(Clone)]
pub(crate) struct Palette {
    pub panel: Hsla,
    pub border: Hsla,
    pub divider: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub selected: Hsla,
    pub chip: Hsla,
    pub field: Hsla,
    pub popover: Hsla,
    pub shadow: [BoxShadow; 2],
    pub error: Hsla,
    pub success: Hsla,
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
        chip: tint(CHIP_OPACITY),
        field: tint(FIELD_OPACITY),
        popover: fill.alpha(POPOVER_OPACITY),
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

    #[test]
    fn dev_light_forces_the_light_mode() {
        assert!(mode_for(ThemeMode::Dark, false).is_dark());
        assert!(!mode_for(ThemeMode::Light, false).is_dark());
        assert!(!mode_for(ThemeMode::Dark, true).is_dark());
        assert!(!mode_for(ThemeMode::Light, true).is_dark());
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
