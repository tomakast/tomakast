//! Toasts: one short message, shown in the footer's left slot in place of
//! the context. A new toast replaces the current one.

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    StatefulInteractiveElement as _, Styled as _, Task, TestSupportExt as _, Window, div, px,
};
use tomakast_core::{Toast, ToastKind};

use crate::chrome::key_chip;
use crate::theme;

/// A part of a toast drawn with key chips: text, or a key drawn as a chip.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ToastPart {
    Text(String),
    Key(String),
}

/// The toast on show: a module's or the launcher's message, or the
/// launcher's line with key chips in it.
enum Current {
    Plain(Toast),
    Keys(ToastKind, Vec<ToastPart>),
}

#[derive(Default)]
pub(crate) struct ToastBar {
    current: Option<Current>,
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
        self.replace(Current::Plain(toast), ttl, cx);
    }

    /// Shows `parts` in a line, the keys as key chips. It asks for a key, so
    /// a success stays as long as an error, to be read and acted on.
    pub(crate) fn show_keys(
        &mut self,
        kind: ToastKind,
        parts: Vec<ToastPart>,
        cx: &mut Context<Self>,
    ) {
        let ttl = match kind {
            ToastKind::Success => Some(theme::UNDO_TOAST_TTL),
            ToastKind::Error => Some(theme::ERROR_TTL),
            ToastKind::Progress => None,
        };
        self.replace(Current::Keys(kind, parts), ttl, cx);
    }

    /// Clears the toast if it is a line with key chips: the keys it asks for
    /// no longer do what it says. Any other toast stays.
    pub(crate) fn clear_keys(&mut self, cx: &mut Context<Self>) {
        if matches!(self.current, Some(Current::Keys(..))) {
            self.current = None;
            self.expiry = None;
            cx.notify();
        }
    }

    fn replace(
        &mut self,
        current: Current,
        ttl: Option<std::time::Duration>,
        cx: &mut Context<Self>,
    ) {
        self.current = Some(current);
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
        let (kind, label, content): (_, _, Vec<AnyElement>) = match &self.current {
            None => return bar.into_any_element(),
            Some(Current::Plain(toast)) => {
                let label = match &toast.message {
                    Some(message) => format!("{}: {message}", toast.title),
                    None => toast.title.clone(),
                };
                let text = div().truncate().child(label.clone()).into_any_element();
                (toast.kind, label, vec![text])
            }
            // Each key is a chip; the label reads the line as words.
            Some(Current::Keys(kind, parts)) => {
                let words: Vec<&str> = parts
                    .iter()
                    .map(|part| match part {
                        ToastPart::Text(text) | ToastPart::Key(text) => text.as_str(),
                    })
                    .collect();
                let content = parts
                    .iter()
                    .enumerate()
                    .map(|(ix, part)| match part {
                        ToastPart::Text(text) => {
                            div().flex_none().child(text.clone()).into_any_element()
                        }
                        ToastPart::Key(key) => div()
                            .id(("toast-key", ix))
                            .aria_label(key.clone())
                            .test_support()
                            .flex_none()
                            .child(key_chip(key, &palette))
                            .into_any_element(),
                    })
                    .collect();
                (*kind, words.join(" "), content)
            }
        };
        let (role, dot) = match kind {
            ToastKind::Success => (Role::Status, palette.success),
            ToastKind::Error => (Role::Alert, palette.error),
            ToastKind::Progress => (Role::Status, palette.muted),
        };
        bar.id("launcher-toast")
            .role(role)
            .aria_label(label)
            .test_support()
            .child(
                div()
                    .flex_none()
                    .size(px(theme::TOAST_DOT_SIZE))
                    .rounded_full()
                    .bg(dot),
            )
            .children(content)
            .into_any_element()
    }
}
