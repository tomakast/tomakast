//! Toasts: one short message, shown in the footer's left slot in place of
//! the context. A new toast replaces the current one.

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    StatefulInteractiveElement as _, Styled as _, Task, TestSupportExt as _, Window, div, px,
};
use tomakast_core::{Toast, ToastKind};

use crate::theme;

#[derive(Default)]
pub(crate) struct ToastBar {
    current: Option<Toast>,
    /// Clears `current`; replacing the toast cancels it. `None` for progress
    /// toasts, which stay until replaced.
    expiry: Option<Task<()>>,
}

impl ToastBar {
    /// Whether a toast is showing.
    pub(crate) fn is_showing(&self) -> bool {
        self.current.is_some()
    }

    pub(crate) fn show(&mut self, toast: Toast, cx: &mut Context<Self>) {
        let ttl = match toast.kind {
            ToastKind::Success => Some(theme::SUCCESS_TTL),
            ToastKind::Error => Some(theme::ERROR_TTL),
            ToastKind::Progress => None,
        };
        self.current = Some(toast);
        self.expiry = ttl.map(|ttl| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ttl).await;
                let _ = this.update(cx, |this, cx| {
                    this.current = None;
                    this.expiry = None;
                    cx.notify();
                });
            })
        });
        cx.notify();
    }
}

impl Render for ToastBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let bar = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .truncate();
        let Some(toast) = &self.current else {
            return bar.into_any_element();
        };
        let label = match &toast.message {
            Some(message) => format!("{}: {message}", toast.title),
            None => toast.title.clone(),
        };
        let (role, dot) = match toast.kind {
            ToastKind::Success => (Role::Status, palette.success),
            ToastKind::Error => (Role::Alert, palette.error),
            ToastKind::Progress => (Role::Status, palette.muted),
        };
        bar.id("launcher-toast")
            .role(role)
            .aria_label(label.clone())
            .test_support()
            .child(
                div()
                    .flex_none()
                    .size(px(theme::TOAST_DOT_SIZE))
                    .rounded_full()
                    .bg(dot),
            )
            .child(div().truncate().child(label))
            .into_any_element()
    }
}
