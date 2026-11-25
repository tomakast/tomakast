//! Moving a kept launcher screen into a new window (pop to root): the kit's inputs listen to
//! the window they were made in (focus, blur, activation: the caret), so each is made again
//! in the new window with the old one's text, selection and scroll.

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::{AppContext as _, Context, Entity, Window};

/// A new single-line input for `window`, built by `build`, holding `old`'s text and
/// selection.
pub(crate) fn input<V: 'static>(
    old: &Entity<InputState>,
    build: impl FnOnce(InputState) -> InputState,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Entity<InputState> {
    let (text, selected) = {
        let old = old.read(cx);
        (old.value(), old.selected_range())
    };
    let new = cx.new(|cx| build(InputState::new(window, cx)).default_value(text));
    new.update(cx, |new, cx| new.set_selected_range(selected, cx));
    new
}

/// A new multi-line input for `window`, built by `build`, holding `old`'s text, selection
/// and scroll.
pub(crate) fn textarea<V: 'static>(
    old: &Entity<TextareaState>,
    build: impl FnOnce(TextareaState, &mut Window, &mut Context<TextareaState>) -> TextareaState,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Entity<TextareaState> {
    let (text, selected, scroll) = {
        let old = old.read(cx);
        (old.value(), old.selected_range(), old.scroll_offset())
    };
    let new = cx.new(|cx| {
        let state = TextareaState::new(window, cx);
        build(state, window, cx).default_value(text)
    });
    new.update(cx, |new, cx| {
        new.set_selected_range(selected, cx);
        new.set_scroll_offset(scroll, cx);
    });
    new
}
