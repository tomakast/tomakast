//! The form view: labelled fields, Tab between them, one submit.
//!
//! The view model ([`FormView`]) holds what the module sent; the typed
//! values live here, in [`FormState`]. A field the user edited is `dirty` and
//! an `UpdateView` never overwrites its value.

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement as _, KeyDownEvent, MouseButton, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, TextAlign,
    Window, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::{FieldKind, FieldValue, FormView};

use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};

const INVALID_DATE: &str = "Enter a date as YYYY-MM-DD";
const DATE_PLACEHOLDER: &str = "YYYY-MM-DD";

enum Control {
    Text(Entity<InputState>),
    Multiline(Entity<TextareaState>),
    Date(Entity<InputState>),
    Checkbox(bool),
    /// `highlight` indexes the options while the list is `open`.
    Dropdown {
        value: String,
        open: bool,
        highlight: usize,
    },
}

struct Field {
    control: Control,
    focus: FocusHandle,
    /// The user changed the value; the module's updates leave it alone.
    dirty: bool,
    error: Option<&'static str>,
}

pub(crate) struct FormState {
    fields: Vec<Field>,
    /// Holds the focus when the form has no field, so that the keyboard
    /// still reaches the launcher (Esc pops the form).
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FormState {
    /// `instance`: the stack entry this state belongs to. The first field
    /// takes the focus.
    pub(crate) fn new(
        instance: u64,
        view: &FormView,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let mut fields = Vec::with_capacity(view.fields.len());
        for (ix, field) in view.fields.iter().enumerate() {
            let (control, focus) = match &field.kind {
                FieldKind::Text { value, placeholder } => {
                    let input = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder(placeholder.clone())
                            .default_value(value.clone())
                    });
                    subscriptions.push(watch(&input, instance, ix, cx));
                    let focus = input.focus_handle(cx);
                    (Control::Text(input), focus)
                }
                FieldKind::Date { value } => {
                    let input = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder(DATE_PLACEHOLDER)
                            .default_value(value.clone().unwrap_or_default())
                    });
                    subscriptions.push(watch(&input, instance, ix, cx));
                    let focus = input.focus_handle(cx);
                    (Control::Date(input), focus)
                }
                FieldKind::Multiline { value } => {
                    let input = cx.new(|cx| {
                        let mut state = TextareaState::new(window, cx);
                        state.set_auto_grow(3, 8, cx);
                        state.default_value(value.clone())
                    });
                    subscriptions.push(watch(&input, instance, ix, cx));
                    let focus = input.focus_handle(cx);
                    (Control::Multiline(input), focus)
                }
                FieldKind::Checkbox { value } => {
                    (Control::Checkbox(*value), cx.focus_handle().tab_stop(true))
                }
                FieldKind::Dropdown { options, value } => (
                    Control::Dropdown {
                        value: value.clone(),
                        open: false,
                        highlight: options.iter().position(|(v, _)| v == value).unwrap_or(0),
                    },
                    cx.focus_handle().tab_stop(true),
                ),
            };
            fields.push(Field {
                control,
                focus,
                dirty: false,
                error: None,
            });
        }
        let state = FormState {
            fields,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        state.focus(window, cx);
        state
    }

    /// Focuses the first field, or the form itself when it has none.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        match self.fields.first() {
            Some(field) => field.focus.focus(window, cx),
            None => self.focus.focus(window, cx),
        }
    }

    pub(crate) fn focus_field(
        &self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) {
        if let Some(field) = self.fields.get(ix) {
            field.focus.focus(window, cx);
        }
    }

    /// Index of the field that has the focus.
    pub(crate) fn focused(&self, window: &Window) -> Option<usize> {
        self.fields.iter().position(|f| f.focus.is_focused(window))
    }

    /// The user edited a text field: its value is theirs from now on, and an
    /// old validation error no longer applies.
    pub(crate) fn edited(&mut self, ix: usize) {
        if let Some(field) = self.fields.get_mut(ix) {
            field.dirty = true;
            field.error = None;
        }
    }

    pub(crate) fn toggle(&mut self, ix: usize) {
        if let Some(Field {
            control: Control::Checkbox(value),
            dirty,
            ..
        }) = self.fields.get_mut(ix)
        {
            *value = !*value;
            *dirty = true;
        }
    }

    /// Opens or closes the dropdown `ix`.
    pub(crate) fn toggle_open(&mut self, ix: usize) {
        if let Some(Field {
            control: Control::Dropdown { open, .. },
            ..
        }) = self.fields.get_mut(ix)
        {
            *open = !*open;
        }
    }

    /// Chooses option `option` of dropdown `ix` and closes it.
    pub(crate) fn pick(&mut self, view: &FormView, ix: usize, option: usize) {
        let Some(FieldKind::Dropdown { options, .. }) = view.fields.get(ix).map(|f| &f.kind) else {
            return;
        };
        if let Some(Field {
            control:
                Control::Dropdown {
                    value,
                    open,
                    highlight,
                },
            dirty,
            ..
        }) = self.fields.get_mut(ix)
            && let Some((chosen, _)) = options.get(option)
        {
            value.clone_from(chosen);
            *highlight = option;
            *open = false;
            *dirty = true;
        }
    }

    /// The dropdown whose list is open.
    pub(crate) fn open_dropdown(&self) -> Option<usize> {
        self.fields
            .iter()
            .position(|f| matches!(f.control, Control::Dropdown { open: true, .. }))
    }

    /// Closes the dropdowns that do not have the focus: a list stays open
    /// only while its field is the focused one, so Enter and the arrows never
    /// act on a list the user has left.
    pub(crate) fn close_unfocused_dropdowns(&mut self, window: &Window) {
        for field in &mut self.fields {
            if let Control::Dropdown { open, .. } = &mut field.control
                && !field.focus.is_focused(window)
            {
                *open = false;
            }
        }
    }

    pub(crate) fn close_dropdowns(&mut self) {
        for field in &mut self.fields {
            if let Control::Dropdown { open, .. } = &mut field.control {
                *open = false;
            }
        }
    }

    /// Moves the highlight of the open dropdown; stays inside the list.
    pub(crate) fn move_highlight(&mut self, view: &FormView, forward: bool) {
        let Some(ix) = self.open_dropdown() else {
            return;
        };
        let Some(FieldKind::Dropdown { options, .. }) = view.fields.get(ix).map(|f| &f.kind) else {
            return;
        };
        if let Control::Dropdown { highlight, .. } = &mut self.fields[ix].control {
            let last = options.len().saturating_sub(1);
            *highlight = if forward {
                (*highlight + 1).min(last)
            } else {
                highlight.saturating_sub(1)
            };
        }
    }

    /// Chooses the highlighted option of the open dropdown.
    pub(crate) fn pick_highlighted(&mut self, view: &FormView) {
        let Some(ix) = self.open_dropdown() else {
            return;
        };
        if let Control::Dropdown { highlight, .. } = self.fields[ix].control {
            self.pick(view, ix, highlight);
        }
    }

    /// Takes an `UpdateView`: `view` has the same fields as the current one
    /// (the stack checks). Values the user changed stay; options and labels,
    /// which the view model holds, are the module's.
    pub(crate) fn apply_update(
        &mut self,
        view: &FormView,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) {
        for (field, new) in self.fields.iter_mut().zip(&view.fields) {
            match (&mut field.control, &new.kind) {
                (Control::Text(input), FieldKind::Text { value, .. }) if !field.dirty => {
                    input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
                }
                (Control::Date(input), FieldKind::Date { value }) if !field.dirty => {
                    let text = value.clone().unwrap_or_default();
                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                }
                (Control::Multiline(input), FieldKind::Multiline { value }) if !field.dirty => {
                    input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
                }
                (Control::Checkbox(value), FieldKind::Checkbox { value: new }) if !field.dirty => {
                    *value = *new;
                }
                (
                    Control::Dropdown {
                        value, highlight, ..
                    },
                    FieldKind::Dropdown {
                        options,
                        value: new,
                    },
                ) => {
                    if !field.dirty {
                        value.clone_from(new);
                    }
                    *highlight = options.iter().position(|(v, _)| v == value).unwrap_or(0);
                }
                _ => {}
            }
        }
    }

    /// Validates the fields and returns their values in field order, or
    /// `None` after marking the invalid fields.
    pub(crate) fn submit_values(&mut self, cx: &Context<LauncherView>) -> Option<Vec<FieldValue>> {
        let mut values = Vec::with_capacity(self.fields.len());
        let mut valid = true;
        for field in &mut self.fields {
            field.error = None;
            values.push(match &field.control {
                Control::Text(input) => FieldValue::Text(input.read(cx).value().to_string()),
                Control::Multiline(input) => FieldValue::Text(input.read(cx).value().to_string()),
                Control::Checkbox(value) => FieldValue::Bool(*value),
                Control::Dropdown { value, .. } => FieldValue::Text(value.clone()),
                Control::Date(input) => {
                    let text = input.read(cx).value().trim().to_string();
                    if text.is_empty() {
                        FieldValue::Date(None)
                    } else if is_iso_date(&text) {
                        FieldValue::Date(Some(text))
                    } else {
                        field.error = Some(INVALID_DATE);
                        valid = false;
                        FieldValue::Date(None)
                    }
                }
            });
        }
        valid.then_some(values)
    }
}

/// Reports edits and Ctrl/Cmd+Enter of a text field to the launcher.
fn watch<S>(
    input: &Entity<S>,
    instance: u64,
    ix: usize,
    cx: &mut Context<LauncherView>,
) -> Subscription
where
    S: gpui_kit::EventEmitter<InputEvent> + 'static,
{
    cx.subscribe(input, move |host, _, event: &InputEvent, cx| match event {
        InputEvent::Change => host.form_field_edited(instance, ix, cx),
        InputEvent::PressEnter {
            secondary: true, ..
        } => host.submit_form(instance, cx),
        _ => {}
    })
}

/// `YYYY-MM-DD` naming a real calendar day.
fn is_iso_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let digits = &text[range];
        digits
            .bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

pub(crate) fn render(
    instance: u64,
    view: &FormView,
    state: &FormState,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let fields = view
        .fields
        .iter()
        .zip(&state.fields)
        .enumerate()
        .map(|(ix, (def, field))| render_field(instance, ix, def, field, &palette, cx))
        .collect::<Vec<_>>();
    let submit = div()
        .id("form-submit")
        .role(Role::Button)
        .aria_label(view.submit_title.clone())
        .test_support()
        .self_end()
        .px_3()
        .py_1()
        .rounded(px(theme::CONTROL_RADIUS))
        .border_1()
        .border_color(palette.border)
        .cursor_pointer()
        .hover(|button| button.bg(palette.selected))
        .on_click(cx.listener(move |host, _, _, cx| host.submit_form(instance, cx)))
        .child(view.submit_title.clone());
    div()
        .id("form-view")
        .track_focus(&state.focus)
        .role(Role::Form)
        .aria_label(view.title.clone())
        .test_support()
        // Tab moves between fields, also out of a multi-line field, which
        // would otherwise take it as an indent. Capturing it runs before the
        // field's own key bindings.
        .capture_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
            let key = &event.keystroke;
            if key.key == "tab" && !key.modifiers.control && !key.modifiers.alt {
                if key.modifiers.shift {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                cx.stop_propagation();
            }
        }))
        .flex()
        .flex_col()
        .gap(px(theme::VIEW_GAP))
        .flex_1()
        .min_h_0()
        .p(px(theme::VIEW_PADDING))
        .text_size(px(theme::BODY_TEXT_SIZE))
        .overflow_y_scroll()
        .children(fields)
        .child(submit)
        .into_any_element()
}

fn render_field(
    instance: u64,
    ix: usize,
    def: &tomakast_core::FormField,
    field: &Field,
    palette: &Palette,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let label = div()
        .id(("form-label", ix))
        .role(Role::Label)
        .aria_label(def.label.clone())
        .test_support()
        .w(px(theme::LABEL_COLUMN_WIDTH))
        .flex_none()
        // One control-high line: the label's text centers on the line of
        // its control's (first) line of text.
        .line_height(px(theme::CONTROL_HEIGHT))
        .truncate()
        .text_align(TextAlign::Right)
        .text_size(px(theme::MUTED_TEXT_SIZE))
        .text_color(palette.muted)
        .child(def.label.clone());
    let control = match &field.control {
        Control::Text(input) | Control::Date(input) => Input::new(input)
            .id(("form-field", ix))
            .aria_label(def.label.clone())
            .bg(palette.field)
            .border_color(palette.border)
            .into_any_element(),
        Control::Multiline(input) => div()
            .id(("form-field", ix))
            .flex_1()
            .child(
                Textarea::new(input)
                    .aria_label(def.label.clone())
                    .bg(palette.field)
                    .border_color(palette.border),
            )
            .into_any_element(),
        Control::Checkbox(value) => {
            let value = *value;
            div()
                .id(("form-field", ix))
                .role(Role::CheckBox)
                .aria_label(def.label.clone())
                .test_support()
                .track_focus(&field.focus)
                .flex_1()
                .h(px(theme::CONTROL_HEIGHT))
                .flex()
                .items_center()
                .cursor_pointer()
                .on_key_down(cx.listener(move |host, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "space" {
                        host.form_toggle(instance, ix, cx);
                        cx.stop_propagation();
                    }
                }))
                // Not `on_click`: GPUI also fires it on the key-up of Enter and
                // Space, which the key handlers below already take.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |host, _, window, cx| {
                        host.form_focus(instance, ix, window, cx);
                        host.form_toggle(instance, ix, cx);
                    }),
                )
                .child(if value { "☑" } else { "☐" })
                .into_any_element()
        }
        Control::Dropdown {
            value,
            open,
            highlight,
        } => {
            let FieldKind::Dropdown { options, .. } = &def.kind else {
                return div().into_any_element();
            };
            let current = options
                .iter()
                .find(|(v, _)| v == value)
                .map_or(value.as_str(), |(_, label)| label.as_str());
            let list = open.then(|| {
                div()
                    .id(("form-options", ix))
                    .role(Role::ListBox)
                    .aria_label(def.label.clone())
                    .test_support()
                    .flex()
                    .flex_col()
                    .rounded(px(theme::CONTROL_RADIUS))
                    .border_1()
                    .border_color(palette.border)
                    .children(options.iter().enumerate().map(|(option_ix, (_, label))| {
                        div()
                            .id(("form-option", option_ix))
                            .role(Role::ListBoxOption)
                            .aria_label(label.clone())
                            .test_support()
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .when(option_ix == *highlight, |option| {
                                option.bg(palette.selected)
                            })
                            .on_click(cx.listener(move |host, _, _, cx| {
                                host.form_pick(instance, ix, option_ix, cx)
                            }))
                            .child(label.clone())
                    }))
            });
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .id(("form-field", ix))
                        .role(Role::ComboBox)
                        .aria_label(def.label.clone())
                        .test_support()
                        .track_focus(&field.focus)
                        .h(px(theme::CONTROL_HEIGHT))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .on_key_down(cx.listener(move |host, event: &KeyDownEvent, _, cx| {
                            if event.keystroke.key == "space" {
                                host.form_toggle_open(instance, ix, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |host, _, window, cx| {
                                host.form_focus(instance, ix, window, cx);
                                host.form_toggle_open(instance, ix, cx);
                            }),
                        )
                        .child(format!("{current} ▾")),
                )
                .children(list)
                .into_any_element()
        }
    };
    let error = field.error.map(|message| {
        div()
            .id(("form-error", ix))
            .role(Role::Alert)
            .aria_label(message)
            .test_support()
            .pl(px(theme::LABEL_COLUMN_WIDTH + theme::LABEL_GAP))
            .text_size(px(theme::MUTED_TEXT_SIZE))
            .text_color(palette.error)
            .child(message)
    });
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(theme::LABEL_GAP))
                .child(label)
                .child(control),
        )
        .children(error)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::is_iso_date;

    #[test]
    fn iso_dates() {
        for ok in ["2026-11-18", "2028-02-29", "2000-02-29", "0001-01-01"] {
            assert!(is_iso_date(ok), "{ok}");
        }
        for bad in [
            "2026-13-40",
            "2027-02-29",
            "1900-02-29",
            "2026-00-10",
            "2026-04-31",
            "2026-1-1",
            "26-11-18",
            "2026/11/18",
            "2026-11-1a",
            "+026-11-18",
            "",
        ] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }
}
