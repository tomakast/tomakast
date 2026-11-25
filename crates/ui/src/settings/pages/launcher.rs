//! Launcher: how soon it opens on an empty search again (pop to root), which screen it opens
//! on, whether searches are remembered, and the saved searches themselves (their number, and
//! clearing them after a question).

use gpui_kit::component::Disableable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window, div,
};

use super::appearance::choice_row;
use super::error_line;
use super::general::switch_row;
use crate::settings::SettingsPage;
use crate::settings::layout::{Group, Page, PageIcon, Row, Section, control_button};
use crate::settings::window::{HISTORY_CLEAR, POP_TO_ROOT, SCREEN, SettingsWindow};
use crate::theme::{self, Palette};

const HISTORY: &[&str] = &["launcher", "history"];

pub(super) fn page(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    palette: &Palette,
) -> Page {
    let mut showing = Group::new().row(choice_row(view, this, POP_TO_ROOT));
    if !view.screen_choices().is_empty() {
        showing = showing.row(choice_row(view, this, SCREEN));
    }
    let history = Group::new()
        .title("History")
        .row(switch_row(view, this, HISTORY))
        .row(saved_searches(view, this, palette));
    Page::new(
        SettingsPage::Launcher,
        Section::Launcher,
        "Launcher",
        PageIcon::Named("search"),
    )
    .group(showing)
    .group(history)
}

/// The number of saved searches, and a button that clears them after asking. Not tied to the
/// file: it works while the file has an error.
fn saved_searches(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    palette: &Palette,
) -> Row {
    let count = view.history_len();
    let error = view.errors.get(HISTORY_CLEAR).cloned();
    let this = this.clone();
    let palette = palette.clone();
    Row::new("Search history", move |_, _, cx| {
        let shown = saved_text(count);
        let this = this.clone();
        let button = control_button("history-clear", &palette, cx)
            .label("Clear…")
            .accessibility_label("Clear search history")
            .disabled(count == 0)
            .on_click(move |_, window, cx| ask(this.clone(), count, window, cx));
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("history-count")
                            .aria_label(shown.clone())
                            .test_support()
                            .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                            .text_color(palette.muted)
                            .child(shown),
                    )
                    .child(button),
            )
            .children(
                error
                    .clone()
                    .map(|error| error_line("history-error".into(), error, &palette)),
            )
    })
    .keywords([
        "history", "clear", "delete", "queries", "searches", "recent",
    ])
}

/// "No saved searches", "1 saved search", "N saved searches".
fn saved_text(count: usize) -> SharedString {
    match count {
        0 => "No saved searches".into(),
        1 => "1 saved search".into(),
        n => format!("{n} saved searches").into(),
    }
}

/// Asks before clearing the `count` saved searches: deleting them cannot be undone.
fn ask(view: WeakEntity<SettingsWindow>, count: usize, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, _| {
        let title = match count {
            1 => "Delete the saved search?".to_string(),
            n => format!("Delete all {n} saved searches?"),
        };
        let view = view.clone();
        let delete = Button::new("history-delete")
            .label("Delete")
            .accessibility_label("Delete")
            .danger()
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let _ = view.update(cx, |view, cx| view.clear_history(cx));
            });
        let cancel = Button::new("history-cancel")
            .label("Cancel")
            .accessibility_label("Cancel")
            .on_click(|_, window, cx| window.close_dialog(cx));
        dialog
            .title(line("history-question", title))
            .child(line("history-warning", "This can't be undone.".to_string()))
            .footer(DialogFooter::new().child(cancel).child(delete))
            // Enter does not delete.
            .on_ok(|_, _, _| false)
    });
}

fn line(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .aria_label(text.clone())
        .test_support()
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::saved_text;

    #[test]
    fn the_count_reads_as_a_phrase() {
        assert_eq!(saved_text(0).as_ref(), "No saved searches");
        assert_eq!(saved_text(1).as_ref(), "1 saved search");
        assert_eq!(saved_text(42).as_ref(), "42 saved searches");
    }
}
