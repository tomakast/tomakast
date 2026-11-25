//! The question the settings window asks when an outside edit of `config.toml` changed a value
//! the user is typing: keep the typed value (written on top of the new file) or use the
//! file's. One question at a time; the others wait in the window's queue. Closing it without
//! an answer (Esc) leaves the typed text in its field, unwritten.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::{
    App, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window, div,
};

use super::fields::TextField;
use super::window::SettingsWindow;

/// A field whose typed value an outside edit changed under it.
pub(crate) struct Conflict {
    pub(crate) field: Entity<TextField>,
}

/// The user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Write the typed value on top of the new file.
    KeepMine,
    /// Drop the typed value for the file's.
    UseFiles,
    /// Closed without an answer: the typed value stays in its field, unwritten.
    Later,
}

/// Opens the question about `conflict`, the first of `view`'s queue. The dialog reads the
/// field each time it is drawn, so it shows the latest values; it stays about this field
/// while it closes, when the queue has moved on.
pub(crate) fn ask(
    view: WeakEntity<SettingsWindow>,
    conflict: &Conflict,
    window: &mut Window,
    cx: &mut App,
) {
    let asked = conflict.field.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let field = asked.read(cx);
        let typed = field
            .pending(cx)
            .unwrap_or_else(|| field.file_value().to_string());
        let theirs = format!("In the file: {}", shown(field.file_value()));
        let title = format!(
            "config.toml changed while you were editing {}.",
            field.title()
        );
        let keep = answer_button("conflict-keep", "Keep mine", Answer::KeepMine, &view).primary();
        let footer = DialogFooter::new().child(keep).child(answer_button(
            "conflict-use-file",
            "Use the file's",
            Answer::UseFiles,
            &view,
        ));
        let on_cancel = view.clone();
        dialog
            .title(line("conflict-title", title))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(line("conflict-yours", format!("Yours: {}", shown(&typed))))
                    .child(line("conflict-file", theirs)),
            )
            .footer(footer)
            // Enter, likely still pressed for the field, answers nothing.
            .on_ok(|_, _, _| false)
            // Esc, the close button and the backdrop.
            .on_cancel(move |_, window, cx| {
                answer(&on_cancel, Answer::Later, window, cx);
                true
            })
    });
}

/// `value` as the question shows it.
fn shown(value: &str) -> &str {
    if value.is_empty() { "(empty)" } else { value }
}

fn line(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .aria_label(text.clone())
        .test_support()
        .child(text)
}

fn answer_button(
    id: impl Into<ElementId>,
    label: &'static str,
    choice: Answer,
    view: &WeakEntity<SettingsWindow>,
) -> Button {
    let view = view.clone();
    Button::new(id)
        .label(label)
        .accessibility_label(SharedString::from(label))
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            answer(&view, choice, window, cx);
        })
}

fn answer(view: &WeakEntity<SettingsWindow>, choice: Answer, window: &mut Window, cx: &mut App) {
    view.update(cx, |view, cx| view.answer(choice, window, cx))
        .ok();
}
