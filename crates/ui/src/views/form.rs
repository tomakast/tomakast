//! The form view: labelled fields, Tab between them, one submit.
//!
//! The view model ([`FormView`]) holds what the module sent; the typed
//! values live here, in [`FormState`]. A field the user edited is `dirty` and
//! an `UpdateView` never overwrites its value.

use gpui_kit::base::actions::Cancel;
use gpui_kit::base::{Checkbox, CheckboxIndicator};
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement as _, KeyDownEvent, ParentElement as _, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    TextAlign, Window, div, px, svg,
};
use tomakast_core::{FieldKind, FieldValue, FormView};

use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};

const INVALID_DATE: &str = "Enter a date as YYYY-MM-DD";
const DATE_PLACEHOLDER: &str = "YYYY-MM-DD";
/// The kit's input mask of a date field: four, two and two digits.
const DATE_MASK: &str = "9999-99-99";

enum Control {
    Text(Entity<InputState>),
    Multiline(Entity<TextareaState>),
    Date(Entity<InputState>),
    Checkbox(bool),
    /// The kit's select draws the trigger and the floating list of options;
    /// `value` is the field's value, which may name no current option.
    Dropdown {
        select: Entity<DropdownState>,
        value: String,
    },
}

/// An option of a form dropdown, as the kit's select lists it.
#[derive(Clone)]
pub(crate) struct DropdownOption {
    value: String,
    label: SharedString,
}

impl SelectItem for DropdownOption {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &String {
        &self.value
    }
}

pub(crate) type DropdownState = SelectState<Vec<DropdownOption>>;

fn dropdown_options(options: &[(String, String)]) -> Vec<DropdownOption> {
    options
        .iter()
        .map(|(value, label)| DropdownOption {
            value: value.clone(),
            label: label.clone().into(),
        })
        .collect()
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
                        // The kit's mask takes digits only and puts the
                        // dashes in; whether it is a real day is checked on
                        // submit. The placeholder comes after: a mask sets
                        // its own.
                        InputState::new(window, cx)
                            .mask_pattern(DATE_MASK)
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
                FieldKind::Dropdown { options, value } => {
                    let selected = options.iter().position(|(v, _)| v == value);
                    let select = cx.new(|cx| {
                        SelectState::new(
                            dropdown_options(options),
                            selected.map(IndexPath::new),
                            window,
                            cx,
                        )
                    });
                    subscriptions.push(watch_select(&select, instance, ix, cx));
                    let focus = select.focus_handle(cx);
                    (
                        Control::Dropdown {
                            select,
                            value: value.clone(),
                        },
                        focus,
                    )
                }
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

    /// Makes the fields' inputs and lists again in `window`, with what they hold (see
    /// [`super::rehome`]); nothing takes the focus.
    pub(crate) fn rehome(
        &mut self,
        instance: u64,
        view: &FormView,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) {
        let mut subscriptions = Vec::with_capacity(self.fields.len());
        for (ix, field) in self.fields.iter_mut().enumerate() {
            let kind = view.fields.get(ix).map(|field| &field.kind);
            match &mut field.control {
                Control::Text(input) => {
                    let placeholder = match kind {
                        Some(FieldKind::Text { placeholder, .. }) => placeholder.clone(),
                        _ => Default::default(),
                    };
                    let new =
                        super::rehome::input(input, |s| s.placeholder(placeholder), window, cx);
                    subscriptions.push(watch(&new, instance, ix, cx));
                    field.focus = new.focus_handle(cx);
                    *input = new;
                }
                Control::Date(input) => {
                    let new = super::rehome::input(
                        input,
                        |s| s.mask_pattern(DATE_MASK).placeholder(DATE_PLACEHOLDER),
                        window,
                        cx,
                    );
                    subscriptions.push(watch(&new, instance, ix, cx));
                    field.focus = new.focus_handle(cx);
                    *input = new;
                }
                Control::Multiline(input) => {
                    let new = super::rehome::textarea(
                        input,
                        |mut state, _, cx| {
                            state.set_auto_grow(3, 8, cx);
                            state
                        },
                        window,
                        cx,
                    );
                    subscriptions.push(watch(&new, instance, ix, cx));
                    field.focus = new.focus_handle(cx);
                    *input = new;
                }
                // Its focus handle is the app's.
                Control::Checkbox(_) => {}
                Control::Dropdown { select, value } => {
                    let options = match kind {
                        Some(FieldKind::Dropdown { options, .. }) => options.as_slice(),
                        _ => &[],
                    };
                    let selected = options.iter().position(|(v, _)| v == value);
                    let new = cx.new(|cx| {
                        SelectState::new(
                            dropdown_options(options),
                            selected.map(IndexPath::new),
                            window,
                            cx,
                        )
                    });
                    subscriptions.push(watch_select(&new, instance, ix, cx));
                    field.focus = new.focus_handle(cx);
                    *select = new;
                }
            }
        }
        self._subscriptions = subscriptions;
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

    /// Closes the open dropdown list, as Escape in it does: the focus goes
    /// back to its field. An open select keeps the focus in its list, a
    /// handle of its own, rather than on its field's.
    /// Returns whether one was open.
    pub(crate) fn close_open_dropdown(&self, window: &mut Window, cx: &mut App) -> bool {
        let mut closed = false;
        for field in &self.fields {
            if let Control::Dropdown { select, .. } = &field.control {
                let list = select.read(cx).focus_handle(cx);
                if list != field.focus {
                    list.dispatch_action(&Cancel, window, cx);
                    closed = true;
                }
            }
        }
        closed
    }

    /// The user chose `value` in dropdown `ix`.
    pub(crate) fn choose(&mut self, ix: usize, value: String) {
        if let Some(Field {
            control: Control::Dropdown { value: current, .. },
            dirty,
            ..
        }) = self.fields.get_mut(ix)
        {
            *current = value;
            *dirty = true;
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
                    Control::Dropdown { select, value },
                    FieldKind::Dropdown {
                        options,
                        value: new,
                    },
                ) => {
                    if !field.dirty {
                        value.clone_from(new);
                    }
                    // Silent: the kit sends no `Confirm` for either.
                    select.update(cx, |select, cx| {
                        select.set_items(dropdown_options(options), window, cx);
                        select.set_selected_value(value, window, cx);
                    });
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
/// Tells the launcher view of each choice in dropdown `ix` of form `instance`.
fn watch_select(
    select: &Entity<DropdownState>,
    instance: u64,
    ix: usize,
    cx: &mut Context<LauncherView>,
) -> Subscription {
    cx.subscribe(
        select,
        move |host, _, event: &SelectEvent<Vec<DropdownOption>>, cx| {
            if let SelectEvent::Confirm(Some(value)) = event {
                host.form_choose(instance, ix, value.clone(), cx);
            }
        },
    )
}

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
    window: &Window,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let fields = view
        .fields
        .iter()
        .zip(&state.fields)
        .enumerate()
        .map(|(ix, (def, field))| render_field(instance, ix, def, field, &palette, window, cx))
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
        .capture_key_down(cx.listener(move |host, event: &KeyDownEvent, window, cx| {
            let key = &event.keystroke;
            if key.key == "tab" && !key.modifiers.control && !key.modifiers.alt {
                let back = key.modifiers.shift;
                let step = move |window: &mut Window, cx: &mut App| {
                    if back {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                };
                // Out of an open dropdown: its list closes, as on Escape, and
                // gives the focus back to its field; the step starts there,
                // once the select has settled.
                if host.form_close_open_dropdown(instance, window, cx) {
                    window.defer(cx, step);
                } else {
                    step(window, cx);
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
        .text_size(theme::text(theme::BODY_TEXT_SIZE))
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
    window: &Window,
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
        .text_size(theme::text(theme::MUTED_TEXT_SIZE))
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
            let host = cx.entity().downgrade();
            div()
                .flex_1()
                .h(px(theme::CONTROL_HEIGHT))
                .flex()
                .items_center()
                .child(
                    // The kit's unstyled checkbox: focus, Space (and a click)
                    // toggle it, role and toggled state; the look is ours.
                    // Enter toggles it through the launcher's `Confirm`.
                    Checkbox::new(("form-field", ix))
                        .checked(*value)
                        .track_focus(&field.focus)
                        .accessibility_label(def.label.clone())
                        .cursor_pointer()
                        .on_change(move |_, _, window, cx| {
                            host.update(cx, |host, cx| {
                                host.form_focus(instance, ix, window, cx);
                                host.form_toggle(instance, ix, cx);
                            })
                            .ok();
                        })
                        .child(checkbox_box(
                            *value,
                            field.focus.is_focused(window),
                            palette,
                        )),
                )
                .into_any_element()
        }
        Control::Dropdown { select, value } => {
            let FieldKind::Dropdown { options, .. } = &def.kind else {
                return div().into_any_element();
            };
            // A value that names no option (the module dropped it) shows as
            // itself, as the placeholder of an empty selection.
            let current = options
                .iter()
                .find(|(v, _)| v == value)
                .map_or(value.as_str(), |(_, label)| label.as_str());
            div()
                .flex_1()
                .child(
                    Select::new(select)
                        .id(("form-field", ix))
                        .accessibility_label(def.label.clone())
                        .placeholder(current.to_string())
                        .bg(palette.field)
                        .border_color(palette.border),
                )
                .into_any_element()
        }
    };
    // The form's own validation error is newer than the module's, which
    // stays until the module clears it.
    let error = match (field.error, &def.error) {
        (Some(message), _) => Some(error_line(
            ("form-error", ix).into(),
            theme::MUTED_TEXT_SIZE,
            message.to_string(),
            palette,
        )),
        (None, Some(message)) => Some(error_line(
            ElementId::Name(format!("field-error-{}", def.id).into()),
            theme::FIELD_ERROR_TEXT_SIZE,
            message.clone(),
            palette,
        )),
        (None, None) => None,
    };
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

/// The box of a form checkbox: a hairline square in the field tint, filled
/// with the text colour and a check mark when checked; the border turns to
/// the text colour while it has the focus.
fn checkbox_box(checked: bool, focused: bool, palette: &Palette) -> AnyElement {
    let mark = checked
        .then(|| crate::icons::bundled_path("check"))
        .flatten()
        .map(|path| {
            svg()
                .path(path)
                .size(px(theme::CHECKBOX_SIZE * theme::GLYPH_FRACTION))
                .text_color(palette.panel.alpha(1.))
        });
    CheckboxIndicator::new()
        .checked(checked)
        .size(px(theme::CHECKBOX_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::CHECKBOX_RADIUS))
        .border_1()
        .border_color(if focused {
            palette.text
        } else {
            palette.border
        })
        .bg(palette.field)
        .styles(|styles| styles.checked(|style| style.bg(palette.text).border_color(palette.text)))
        .children(mark)
        .into_any_element()
}

/// An error under a field, in line with the controls.
fn error_line(id: ElementId, text_size: f32, message: String, palette: &Palette) -> AnyElement {
    div()
        .id(id)
        .role(Role::Alert)
        .aria_label(message.clone())
        .test_support()
        .pl(px(theme::LABEL_COLUMN_WIDTH + theme::LABEL_GAP))
        .text_size(px(text_size))
        .text_color(palette.error)
        .child(message)
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
