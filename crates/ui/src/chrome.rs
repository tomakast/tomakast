//! Shared launcher chrome: key chips, the header with its divider and the
//! footer. Root and pushed views use the same pieces.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AnyElement, BoxShadow, Context, ElementId, Entity, Hsla, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, TestSupportExt as _, div, hsla, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, px, relative,
};

use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};
use crate::toast::ToastBar;

/// The shortcut that opens the action panel, as key chip labels.
pub(crate) fn shortcut_label() -> [&'static str; 2] {
    if cfg!(target_os = "macos") {
        ["⌘", "K"]
    } else {
        ["Ctrl", "K"]
    }
}

/// What the footer shows on its left.
pub(crate) enum FooterLeft {
    /// The context: the app name at the root, the view title in pushed views.
    Context(String),
    /// The toast bar, while a toast is showing.
    Toast,
}

/// A toast replaces the context while it is showing.
pub(crate) fn footer_left(context: &str, toast_showing: bool) -> FooterLeft {
    if toast_showing {
        FooterLeft::Toast
    } else {
        FooterLeft::Context(context.to_string())
    }
}

/// A key hint: a small rounded box with `label` in it.
pub(crate) fn key_chip(label: &str, palette: &Palette) -> AnyElement {
    div()
        .flex_none()
        .min_w(px(theme::CHIP_SIZE))
        .h(px(theme::CHIP_SIZE))
        .px(px(theme::CHIP_PADDING_X))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::CHIP_RADIUS))
        .bg(palette.chip)
        .text_size(px(theme::SMALL_TEXT_SIZE))
        .child(label.to_string())
        .into_any_element()
}

/// How a busy view shows it, in place of the header's divider.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Busy {
    /// A segment sweeping across; how far, 0 to 1 (eased by the caller).
    Moving(f32),
    /// The work is done, the sweep completing: the glow runs from where it was (`from`, as
    /// [`Busy::Moving`]'s) to the right edge, filling the line behind it; `t` goes 0 to 1.
    Finishing { from: f32, t: f32 },
    /// Then the whole line, lit, fades; its opacity, 1 to 0.
    Done(f32),
    /// The desktop asks for reduced motion: a still, dimmed line.
    Still,
}

/// One sweep of the busy line.
pub(crate) const BUSY_PERIOD: std::time::Duration = std::time::Duration::from_millis(1600);
/// When the work is done: the light runs out past the right edge in this…
pub(crate) const BUSY_FINISH: std::time::Duration = std::time::Duration::from_millis(300);
/// …then the whole line lights up and fades in this.
pub(crate) const BUSY_FLASH: std::time::Duration = std::time::Duration::from_millis(300);
/// The busy line's height, and its moving segment's share of the width.
const BUSY_HEIGHT: f32 = 2.;
const BUSY_SEGMENT: f32 = 0.4;

/// The busy line: the track in the divider's colour, a segment in the accent sweeping
/// from beyond the left edge to beyond the right one.
/// The running light's colour at `warmth` (0 white, 1 gold), and its halo's: a sheen of
/// gold over white, deeper on a light theme so it reads there too.
fn light_colour(warmth: f32, palette: &Palette) -> Hsla {
    let dark = palette.text.l > 0.5;
    // A barely-there sheen: the warm end is a pale, soft gold, and only half of it shows.
    let (white, gold) = if dark {
        (
            hsla(43. / 360., 0.10, 0.96, 1.),
            hsla(42. / 360., 0.55, 0.82, 1.),
        )
    } else {
        (
            hsla(43. / 360., 0.05, 0.30, 1.),
            hsla(40. / 360., 0.45, 0.36, 1.),
        )
    };
    let k = warmth.clamp(0., 1.) * MAX_WARMTH;
    hsla(
        white.h + (gold.h - white.h) * k,
        white.s + (gold.s - white.s) * k,
        white.l + (gold.l - white.l) * k,
        1.,
    )
}

/// How warm the light is with its peak at `peak` (a share of the width): goldest in the
/// middle of the line, whiter at its ends, so it shimmers as it runs.
fn warmth_at(peak: f32) -> f32 {
    (std::f32::consts::PI * peak.clamp(0., 1.)).sin()
}

/// The running light: two soft halves meeting at its peak, its left edge at `left` (a
/// share of the width), at `strength` (0 to 1).
fn glow(left: f32, strength: f32, palette: &Palette) -> [AnyElement; 2] {
    let half = BUSY_SEGMENT / 2.;
    let colour = light_colour(warmth_at(left + half), palette);
    let lit = colour.opacity(0.85 * strength);
    let clear = colour.opacity(0.);
    let part = |left: f32, from, to| {
        div()
            .absolute()
            .top_0()
            .h_full()
            .left(relative(left))
            .w(relative(half))
            .bg(linear_gradient(
                90.,
                linear_color_stop(from, 0.),
                linear_color_stop(to, 1.),
            ))
            .into_any_element()
    };
    [part(left, clear, lit), part(left + half, lit, clear)]
}

/// The halo around the light's peak at `peak` (a share of the width): a soft bloom, widest
/// and brightest at the centre, spilling a little above and below the line. Not clipped to
/// the line.
fn bloom(peak: f32, strength: f32, palette: &Palette) -> AnyElement {
    let colour = light_colour(warmth_at(peak), palette);
    let width = BUSY_SEGMENT * 0.45;
    div()
        .id("view-busy-bloom")
        .test_support()
        .absolute()
        .top_0()
        .h(px(BUSY_HEIGHT))
        .left(relative(peak - width / 2.))
        .w(relative(width))
        .rounded(px(BUSY_HEIGHT))
        .shadow(vec![
            BoxShadow::new(px(0.), px(0.), colour.opacity(0.55 * strength)).blur_radius(px(8.)),
            BoxShadow::new(px(0.), px(0.), colour.opacity(0.35 * strength)).blur_radius(px(16.)),
        ])
        .into_any_element()
}

/// The glow of the lit line, `width` of it from `left` (shares of the line), at `strength`.
fn line_glow(left: f32, width: f32, strength: f32, palette: &Palette) -> AnyElement {
    let colour = light_colour(0.3, palette);
    div()
        .id("view-busy-glow")
        .test_support()
        .absolute()
        .top_0()
        .left(relative(left))
        .h(px(BUSY_HEIGHT))
        .w(relative(width))
        .rounded(px(BUSY_HEIGHT))
        .shadow(vec![
            BoxShadow::new(px(0.), px(0.), colour.opacity(0.45 * strength)).blur_radius(px(8.)),
            BoxShadow::new(px(0.), px(0.), colour.opacity(0.25 * strength)).blur_radius(px(16.)),
        ])
        .into_any_element()
}

/// How far the light warms towards gold at most: a hint, not a colour.
const MAX_WARMTH: f32 = 0.5;

/// How lit the line is when the work is done.
const BUSY_LIT: f32 = 0.6;

/// The busy line: a track the light runs along, clipped to the line, and the light's halo
/// over it, not clipped.
fn busy_line(busy: Busy, palette: &Palette) -> AnyElement {
    let line = || {
        div()
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .overflow_hidden()
    };
    let half = BUSY_SEGMENT / 2.;
    let (line, halo): (_, Vec<AnyElement>) = match busy {
        Busy::Still => (line().bg(palette.muted.opacity(0.5)), Vec::new()),
        Busy::Moving(progress) => {
            let left = progress * (1. + BUSY_SEGMENT) - BUSY_SEGMENT;
            (
                line().children(glow(left, 1., palette)),
                vec![bloom(left + half, 1., palette)],
            )
        }
        Busy::Finishing { from, t } => {
            // The light runs on, out past the right edge; the lit part grows out from where it
            // was, to the left edge and after the light, and is whole only once it is gone.
            let start = from * (1. + BUSY_SEGMENT) - BUSY_SEGMENT;
            let left = start + (1. - start) * t;
            let origin = (start + half).clamp(0., 1.);
            let lit_left = origin * (1. - t);
            let lit_right = (left + half).clamp(origin, 1.);
            let fill = light_colour(0.3, palette).opacity(BUSY_LIT * t.sqrt());
            (
                line()
                    .child(
                        div()
                            .id("view-busy-fill")
                            .test_support()
                            .absolute()
                            .top_0()
                            .h_full()
                            .left(relative(lit_left))
                            .w(relative(lit_right - lit_left))
                            .bg(fill),
                    )
                    .children(glow(left, 1., palette)),
                // The lit part glows as it grows; the running light's halo goes with it.
                vec![
                    bloom(left + half, 1., palette),
                    line_glow(lit_left, lit_right - lit_left, t, palette),
                ],
            )
        }
        // The glow fades faster than the line: it is gone while the line still shows.
        Busy::Done(alpha) => (
            line().bg(light_colour(0.3, palette).opacity(BUSY_LIT * alpha)),
            vec![line_glow(0., 1., alpha.powi(3), palette)],
        ),
    };
    div()
        .id("view-busy")
        .role(Role::ProgressIndicator)
        .aria_label(if matches!(busy, Busy::Done(_)) {
            "Done"
        } else {
            "Working"
        })
        .test_support()
        .absolute()
        .left_0()
        .bottom_0()
        .w_full()
        .h(px(BUSY_HEIGHT))
        .child(line)
        .children(halo)
        .into_any_element()
}

/// The header: for a pushed view (`view_title` is its title) the back chip
/// first, and the whole header is the view's toolbar; then `content` (the
/// search input, the filter input or the view title), with a divider below.
pub(crate) fn header(
    view_title: Option<&str>,
    content: AnyElement,
    busy: Option<Busy>,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let back = view_title.map(|_| {
        div()
            .id("view-back")
            .role(Role::Button)
            .aria_label("Back")
            .test_support()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| this.pop_view(window, cx)))
            .child(key_chip("←", &palette))
    });
    // A pushed view's title bar is chrome; the root's header is the search field.
    let bar = div()
        .flex_none()
        .flex()
        .flex_col()
        .when(view_title.is_some(), |bar| bar.bg(palette.chrome))
        .child(
            div()
                .h(px(theme::HEADER_HEIGHT))
                .px(px(theme::EDGE_PADDING))
                .flex()
                .items_center()
                .gap_3()
                .children(back)
                .child(div().flex_1().min_w_0().child(content)),
        )
        // The divider always; the busy line over it, taking no room, so nothing moves
        // when it comes and goes.
        .child(
            div()
                .relative()
                .h(px(theme::DIVIDER_THICKNESS))
                .bg(palette.divider)
                .children(busy.map(|busy| busy_line(busy, &palette))),
        );
    match view_title {
        // The toolbar of a pushed view holds the back chip and the content.
        Some(title) => div()
            .id("view-header")
            .role(Role::Toolbar)
            .aria_label(title.to_string())
            .test_support()
            .flex_none()
            .child(bar)
            .into_any_element(),
        None => bar.into_any_element(),
    }
}

/// The borderless input in the header, shared by the root search and the
/// list view's filter. Without the kit's inner padding its text starts where
/// the header's padding ends, in line with the section labels, the row icons
/// and the footer text.
pub(crate) fn header_input(
    state: &Entity<InputState>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> AnyElement {
    Input::new(state)
        .id(id)
        .aria_label(label)
        .appearance(false)
        .focus_bordered(false)
        .px_0()
        .text_size(px(theme::HEADER_TEXT_SIZE))
        .into_any_element()
}

/// The footer, with a divider above: `left` on the left; on the right the
/// primary action with `↵`, a vertical divider, then "Actions" with the
/// shortcut chips while the selection has a panel to open (`actions`).
/// Without a primary action the right side is the "Actions" hint alone;
/// without either only the left side shows.
/// `primary_keys`: the chips of the key that runs the primary action, `↵` when empty (a
/// pad's first action runs on Ctrl+Enter, Enter types a newline there).
pub(crate) fn footer(
    left: FooterLeft,
    toasts: &Entity<ToastBar>,
    primary: Option<&str>,
    primary_keys: &[String],
    actions: bool,
    palette: &Palette,
) -> AnyElement {
    let left = match left {
        FooterLeft::Context(context) => div()
            .id("footer-context")
            .role(Role::Status)
            .aria_label(context.clone())
            .test_support()
            .truncate()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(palette.muted)
            .child(context)
            .into_any_element(),
        FooterLeft::Toast => toasts.clone().into_any_element(),
    };
    let right = (actions || primary.is_some()).then(|| {
        let shortcut = shortcut_label();
        let actions_label = format!("{}+{} Actions", shortcut[0], shortcut[1]);
        let primary = primary.map(|primary| {
            let (label, chips) = if primary_keys.is_empty() {
                (format!("⏎ {primary}"), vec!["↵".to_string()])
            } else {
                (
                    format!("{} {primary}", primary_keys.join("+")),
                    primary_keys.to_vec(),
                )
            };
            [
                div()
                    .id("footer-primary")
                    .aria_label(label)
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(primary.to_string())
                    .children(chips.iter().map(|chip| key_chip(chip, palette)))
                    .into_any_element(),
                div()
                    .w(px(theme::DIVIDER_THICKNESS))
                    .h(px(theme::DIVIDER_V_HEIGHT))
                    .bg(palette.divider)
                    .into_any_element(),
            ]
        });
        let actions = actions.then(|| {
            div()
                .id("footer-actions")
                .aria_label(actions_label)
                .test_support()
                .flex()
                .items_center()
                .gap_1()
                .child("Actions")
                .child(key_chip(shortcut[0], palette))
                .child(key_chip(shortcut[1], palette))
        });
        div()
            .id("launcher-footer")
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(palette.muted)
            .children(primary.into_iter().flatten())
            .children(actions)
    });
    div()
        .flex_none()
        .flex()
        .flex_col()
        .bg(palette.chrome)
        .child(div().h(px(theme::DIVIDER_THICKNESS)).bg(palette.divider))
        .child(
            div()
                .h(px(theme::FOOTER_HEIGHT))
                .px(px(theme::EDGE_PADDING))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().flex_1().min_w_0().child(left))
                .children(right),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_left_prefers_toast() {
        assert!(matches!(footer_left("tomakast", true), FooterLeft::Toast));
        match footer_left("tomakast", false) {
            FooterLeft::Context(context) => assert_eq!(context, "tomakast"),
            FooterLeft::Toast => panic!("no toast is showing"),
        }
    }

    #[test]
    fn shortcut_label_per_platform() {
        let expected = if cfg!(target_os = "macos") {
            ["⌘", "K"]
        } else {
            ["Ctrl", "K"]
        };
        assert_eq!(shortcut_label(), expected);
    }
}
