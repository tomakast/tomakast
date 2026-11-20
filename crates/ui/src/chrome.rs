//! Shared launcher chrome: key chips, the header with its divider and the
//! footer. Root and pushed views use the same pieces.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AnyElement, Context, ElementId, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, div, px,
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

/// The header: for a pushed view (`view_title` is its title) the back chip
/// first, and the whole header is the view's toolbar; then `content` (the
/// search input, the filter input or the view title), with a divider below.
pub(crate) fn header(
    view_title: Option<&str>,
    content: AnyElement,
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
    let bar = div()
        .flex_none()
        .flex()
        .flex_col()
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
        .child(div().h(px(theme::DIVIDER_THICKNESS)).bg(palette.divider));
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
/// shortcut chips. Without a primary action only the left side shows.
pub(crate) fn footer(
    left: FooterLeft,
    toasts: &Entity<ToastBar>,
    primary: Option<&str>,
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
    let right = primary.map(|primary| {
        let shortcut = shortcut_label();
        let label = format!("⏎ {primary}");
        let actions_label = format!("{}+{} Actions", shortcut[0], shortcut[1]);
        div()
            .id("launcher-footer")
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(palette.muted)
            .child(
                div()
                    .id("footer-primary")
                    .aria_label(label)
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(primary.to_string())
                    .child(key_chip("↵", palette)),
            )
            .child(
                div()
                    .w(px(theme::DIVIDER_THICKNESS))
                    .h(px(theme::DIVIDER_V_HEIGHT))
                    .bg(palette.divider),
            )
            .child(
                div()
                    .id("footer-actions")
                    .aria_label(actions_label)
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child("Actions")
                    .child(key_chip(shortcut[0], palette))
                    .child(key_chip(shortcut[1], palette)),
            )
    });
    div()
        .flex_none()
        .flex()
        .flex_col()
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
