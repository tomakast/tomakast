//! Renderers of module-owned views, one per view kind, and their UI state.

pub(crate) mod action_panel;
pub(crate) mod detail;
pub(crate) mod form;
pub(crate) mod grid;
pub(crate) mod hotkey_recorder;
pub(crate) mod list;
pub(crate) mod rehome;
pub(crate) mod scratchpad;

use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    TestSupportExt as _, Window, div, img, prelude::FluentBuilder as _, px, rgb, svg,
};
use tomakast_core::{Accessory, Icon, View};

use crate::icons::{self, IconSpec};
use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};

/// UI state of a pushed view: what the view model does not hold.
pub(crate) enum ViewState {
    List(list::ListState),
    Grid(grid::GridState),
    Detail(detail::DetailState),
    Form(form::FormState),
    Scratchpad(scratchpad::ScratchpadState),
}

impl ViewState {
    /// State of a newly pushed view; it takes the focus.
    pub(crate) fn new(
        instance: u64,
        view: &View,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        match view {
            View::List(list) => ViewState::List(list::ListState::new(instance, list, window, cx)),
            View::Grid(_) => ViewState::Grid(grid::GridState::new(window, cx)),
            View::Detail(_) => ViewState::Detail(detail::DetailState::new(window, cx)),
            View::Form(form) => ViewState::Form(form::FormState::new(instance, form, window, cx)),
            View::Scratchpad(pad) => {
                ViewState::Scratchpad(scratchpad::ScratchpadState::new(instance, pad, window, cx))
            }
        }
    }

    /// Makes the view's inputs again in `window`, the window a kept screen opens in (see
    /// [`rehome`]). Nothing takes the focus.
    pub(crate) fn rehome(
        &mut self,
        instance: u64,
        view: &View,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) {
        match (self, view) {
            (ViewState::List(state), View::List(view)) => state.rehome(instance, view, window, cx),
            (ViewState::Form(state), View::Form(view)) => state.rehome(instance, view, window, cx),
            (ViewState::Scratchpad(state), View::Scratchpad(view)) => {
                state.rehome(instance, view, window, cx)
            }
            // Their focus handles are the app's, not the window's.
            (ViewState::Grid(_) | ViewState::Detail(_), _) => {}
            _ => tracing::warn!("a kept view's state is not of its kind"),
        }
    }

    /// Gives the focus back to the view, when it is on top again.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        match self {
            ViewState::List(state) => state.focus(window, cx),
            ViewState::Grid(state) => state.focus(window, cx),
            ViewState::Detail(state) => state.focus(window, cx),
            ViewState::Form(state) => state.focus(window, cx),
            ViewState::Scratchpad(state) => state.focus(window, cx),
        }
    }

    /// Id of the selected row or tile, to keep it selected across an update
    /// of `view`.
    pub(crate) fn selected_id(&self, view: &View) -> Option<String> {
        match (view, self) {
            (View::List(view), ViewState::List(state)) => {
                state.selected_row(view).map(|row| row.id.clone())
            }
            (View::Grid(view), ViewState::Grid(state)) => {
                state.selected_tile(view).map(|tile| tile.id.clone())
            }
            _ => None,
        }
    }
}

/// One list row, on one line: icon, title, the subtitle in the muted color
/// (it truncates before the title does), accessories at the right edge. The
/// row spans the list's width; its highlight is inset by the row gutter.
// One call per row kind; the parts are the row's own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn row(
    id: ElementId,
    title: &str,
    subtitle: Option<&str>,
    icon: IconSpec<'_>,
    accessories: &[Accessory],
    trailing: Option<AnyElement>,
    selected: bool,
    palette: &Palette,
) -> AnyElement {
    let icon = icon_element(icon, theme::ICON_SIZE, palette);
    let text = div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(theme::ROW_SUBTITLE_GAP))
        .text_size(theme::text(theme::ROW_TEXT_SIZE))
        .child(
            div()
                .min_w_0()
                .flex_shrink(theme::TITLE_SHRINK)
                .truncate()
                .child(title.to_string()),
        )
        .children(subtitle.map(|subtitle| {
            div()
                .min_w_0()
                .flex_shrink(theme::SUBTITLE_SHRINK)
                .truncate()
                .text_color(palette.muted)
                .child(subtitle.to_string())
        }));
    let accessories_shown = !accessories.is_empty();
    let accessories = accessories_shown.then(|| {
        div()
            .ml_auto()
            .flex_none()
            .pl(px(theme::ROW_ACCESSORY_GAP))
            .flex()
            .items_center()
            .gap(px(theme::ROW_ACCESSORY_GAP))
            .text_size(theme::text(theme::ACCESSORY_TEXT_SIZE))
            .text_color(palette.muted)
            .children(accessories.iter().enumerate().map(|(ix, accessory)| {
                let (text, pill) = match accessory {
                    Accessory::Text(text) => (text, false),
                    Accessory::Tag(tag) => (tag, true),
                };
                div()
                    .id(("accessory", ix))
                    .aria_label(text.clone())
                    .test_support()
                    .when(pill, |tag| {
                        tag.px(px(theme::TAG_PADDING_X))
                            .rounded_full()
                            .bg(palette.chip)
                            .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                    })
                    .child(text.clone())
            }))
    });
    div()
        .id(id)
        .role(Role::ListItem)
        .aria_label(title.to_string())
        .aria_selected(selected)
        .test_support()
        .w_full()
        .h(px(theme::ROW_HEIGHT))
        .px(px(theme::ROW_GUTTER))
        .child(
            div()
                .size_full()
                .px(px(theme::ROW_PADDING_X))
                .flex()
                .items_center()
                .gap(px(theme::ROW_ICON_GAP))
                .rounded(px(theme::ROW_RADIUS))
                .when(selected, |row| row.bg(palette.selected))
                .child(icon)
                .child(text)
                .children(accessories)
                .children(trailing.map(|trailing| {
                    div()
                        .when(accessories_shown, |el| el.pl(px(theme::ROW_ACCESSORY_GAP)))
                        .when(!accessories_shown, |el| el.ml_auto())
                        .flex_none()
                        .child(trailing)
                })),
        )
        .into_any_element()
}

/// The colour a bundled (monochrome) glyph is drawn in: the theme's text colour, so it is dark
/// on the light theme and light on the dark one.
fn bundled_glyph_color(palette: &Palette) -> gpui_kit::Hsla {
    palette.text
}

/// An icon or a tile image, `size` square: an image file, a bundled Lucide
/// glyph for a theme name, an emoji drawn as text, or a colour swatch. A theme
/// name outside the bundle is drawn as the file it resolves to in the desktop's
/// icon theme ([`crate::named_icon`], Linux only) once resolved. No icon, a
/// name in neither, an invalid emoji or colour and an image that does not load
/// show the spec's monogram instead.
pub(crate) fn icon_element(spec: IconSpec<'_>, size: f32, palette: &Palette) -> AnyElement {
    let IconSpec { icon, monogram } = spec;
    let fallback = {
        let letter = icons::monogram_letter(monogram);
        let (fill, border, text) = (palette.chip, palette.border, palette.muted);
        move || monogram_element(letter.clone(), fill, border, text, size)
    };
    let Some(icon) = icon.filter(|icon| icon.is_valid()) else {
        return fallback();
    };
    let image = |label: &str| {
        div()
            .id("icon")
            .role(Role::Image)
            .aria_label(label.to_string())
            .test_support()
            .flex_none()
            .size(px(size))
    };
    match icon {
        Icon::File(path) => img(path.clone())
            .flex_none()
            .size(px(size))
            .with_fallback(fallback)
            .into_any_element(),
        // Never `img(path)`: the bounded loader opens a plugin's file (I-7).
        Icon::PluginFile(path) => match crate::plugin_image::cached(path) {
            Some(image) => img(image)
                .flex_none()
                .size(px(size))
                .with_fallback(fallback)
                .into_any_element(),
            None => fallback(),
        },
        Icon::Named(name) => match icons::bundled_path(name) {
            Some(path) => image(name)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(path)
                        .size(px(size * theme::GLYPH_FRACTION))
                        .text_color(bundled_glyph_color(palette)),
                )
                .into_any_element(),
            // Not a Lucide name: the desktop's icon theme, resolved off the UI
            // thread; the monogram until then, when the theme has no such icon,
            // and outside Linux.
            None => match crate::named_icon::cached(name) {
                Some(path) => img(path)
                    .flex_none()
                    .size(px(size))
                    .with_fallback(fallback)
                    .into_any_element(),
                None => fallback(),
            },
        },
        // Up to 32 bytes can be several glyphs: they are clipped to the
        // square. The glyph is smaller than the square, whose line it fills,
        // so it is not clipped itself.
        Icon::Emoji(emoji) => image(emoji)
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(emoji_glyph_size(size)))
            .line_height(px(size))
            .child(emoji.clone())
            .into_any_element(),
        Icon::Color(color) => match parse_color(color) {
            Some(fill) => image(color)
                .rounded(px(size * theme::SWATCH_RADIUS_FRACTION))
                .bg(rgb(fill))
                .border_1()
                .border_color(palette.border)
                .into_any_element(),
            None => fallback(),
        },
    }
}

/// A monogram, `size` square: `letter` in the muted text colour on a neutral
/// fill with a hairline border, the same for every item; the square alone
/// when there is no letter.
fn monogram_element(
    letter: Option<String>,
    fill: Hsla,
    border: Hsla,
    text: Hsla,
    size: f32,
) -> AnyElement {
    div()
        .id("icon")
        .role(Role::Image)
        .aria_label(letter.clone().unwrap_or_default())
        .test_support()
        .flex_none()
        .size(px(size))
        .overflow_hidden()
        .rounded(px(size * theme::SWATCH_RADIUS_FRACTION))
        .bg(fill)
        .border_1()
        .border_color(border)
        .flex()
        .items_center()
        .justify_center()
        .text_color(text)
        .text_size(px(size * theme::MONOGRAM_LETTER_FRACTION))
        .line_height(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .children(letter)
        .into_any_element()
}

/// Font size of an emoji icon in a `size` square.
fn emoji_glyph_size(size: f32) -> f32 {
    size * theme::EMOJI_GLYPH_FRACTION
}

/// `#rrggbb` as `0xrrggbb`.
fn parse_color(color: &str) -> Option<u32> {
    let hex = color
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))?;
    u32::from_str_radix(hex, 16).ok()
}

/// A section label of a list: muted, semibold, lined up with the rows'
/// icons; never selectable. Lists are uniform (every line as tall as a row),
/// so the label sits at the bottom of a row-tall line.
pub(crate) fn section_label(id: ElementId, text: &str, palette: &Palette) -> AnyElement {
    div()
        .id(id)
        .role(Role::Heading)
        .aria_label(text.to_string())
        .test_support()
        .w_full()
        .h(px(theme::ROW_HEIGHT))
        .flex()
        .items_end()
        .child(
            div()
                .h(px(theme::SECTION_HEIGHT))
                .min_w_0()
                .px(px(theme::EDGE_PADDING))
                .flex()
                .items_center()
                .truncate()
                .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.muted)
                .child(text.to_string()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{bundled_glyph_color, emoji_glyph_size, icon_element, parse_color};
    use crate::icons::IconSpec;
    use crate::theme;
    use gpui_kit::ParentElement as _;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    /// "Search Files" (`Named("search")`) is drawn through the bundled-glyph branch of the icon
    /// every list row, tile and settings row uses, in the theme's text colour: dark on light.
    /// (The headless harness observes bounds and labels, not a glyph's colour, so the colour is
    /// checked through the one function that picks it.)
    #[gpui_kit::test]
    fn the_search_icon_is_a_bundled_glyph_in_the_text_colour(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, QuitMode};
        struct Icons;
        impl gpui_kit::Render for Icons {
            fn render(
                &mut self,
                _: &mut gpui_kit::Window,
                _: &mut gpui_kit::Context<Self>,
            ) -> impl gpui_kit::IntoElement {
                let palette = theme::palette_for(false);
                let icon = tomakast_core::Icon::Named("search".into());
                let spec = IconSpec {
                    icon: Some(&icon),
                    monogram: "Search Files",
                };
                gpui_kit::div().child(icon_element(spec, theme::ICON_SIZE, &palette))
            }
        }
        let (w, _) = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Icons)).unwrap()
        });
        cx.run_until_parked();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find_by_label("search").is_some());
        })
        .unwrap();
        let light = theme::palette_for(false);
        assert_eq!(bundled_glyph_color(&light), light.text);
        assert!(light.text.l < 0.4 && light.text.a == 1., "{:?}", light.text);
    }

    /// The test support observes an element's bounds, not its font size, so
    /// the glyph's size is checked here; the headless tests check the square.
    #[test]
    fn emoji_glyph_is_smaller_than_its_square() {
        for size in [theme::ICON_SIZE, theme::IMAGE_SIZE] {
            let glyph = emoji_glyph_size(size);
            assert!(glyph < size, "{glyph} in {size}");
            assert_eq!(glyph, size * theme::EMOJI_GLYPH_FRACTION);
        }
        assert_eq!(emoji_glyph_size(theme::ICON_SIZE), 16.);
    }

    #[test]
    fn colors_parse_only_as_six_hex_digits() {
        assert_eq!(parse_color("#ff8800"), Some(0xff8800));
        assert_eq!(parse_color("#FFFFFF"), Some(0xffffff));
        for bad in ["ff8800", "#ff880", "#ff88000", "#gg8800", "#+f8800", ""] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }
}
