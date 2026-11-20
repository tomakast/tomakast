//! The detail view: rendered markdown above a table of metadata.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, Context, FocusHandle, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, div, px,
};
use tomakast_core::DetailView;

use crate::launcher_view::LauncherView;
use crate::markdown::{self, LinkOpener};
use crate::theme;

/// A detail view has no widget of its own; it holds the focus so that the
/// launcher's keys (Esc, Ctrl+K) reach it.
pub(crate) struct DetailState {
    focus: FocusHandle,
}

impl DetailState {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<LauncherView>) -> Self {
        let state = DetailState {
            focus: cx.focus_handle(),
        };
        state.focus(window, cx);
        state
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        self.focus.focus(window, cx);
    }
}

/// `links` opens the markdown's links.
pub(crate) fn render(
    view: &DetailView,
    state: &DetailState,
    links: Rc<dyn LinkOpener>,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let body = div()
        .id("detail-markdown")
        .role(Role::Document)
        .aria_label(markdown::plain_text(&view.markdown))
        .test_support()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(markdown::render_with(&view.markdown, links, cx));
    let metadata = (!view.metadata.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(palette.divider)
            .children(view.metadata.iter().enumerate().map(|(ix, (key, value))| {
                div()
                    .id(("detail-meta", ix))
                    .role(Role::Row)
                    .aria_label(format!("{key}: {value}"))
                    .test_support()
                    .flex()
                    .gap(px(theme::LABEL_GAP))
                    .text_size(px(theme::MUTED_TEXT_SIZE))
                    .child(
                        div()
                            .w(px(theme::LABEL_COLUMN_WIDTH))
                            .flex_none()
                            .truncate()
                            .text_color(palette.muted)
                            .child(key.clone()),
                    )
                    .child(div().flex_1().min_w_0().child(value.clone()))
            }))
    });
    div()
        .id("detail-view")
        .track_focus(&state.focus)
        .flex()
        .flex_col()
        .gap(px(theme::VIEW_GAP))
        .flex_1()
        .min_h_0()
        .p(px(theme::VIEW_PADDING))
        .text_size(px(theme::BODY_TEXT_SIZE))
        .child(body)
        .children(metadata)
        .into_any_element()
}
