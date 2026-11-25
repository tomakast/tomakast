//! The settings window's fields that keep what the user is entering apart from the file:
//! text and numbers (written on Enter or when the field loses focus; Esc puts the file's
//! value back), values picked from a searchable list (the city, a time zone, a choice of
//! many options) and hotkeys (recorded, then requested). Each is an entity the window owns
//! and a row of a settings page draws, with the launcher form's look. The kit's settings text
//! field writes on every keystroke and takes the value back on every render, so it fits
//! neither.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, IconName, IndexPath, Sizable as _, ThemeStyled as _};
use gpui_kit::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, Entity,
    EventEmitter, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseUpEvent, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window,
    anchored, deferred, div, linear_color_stop, linear_gradient, point,
    prelude::FluentBuilder as _, px,
};
use tomakast_core::Hotkey;
use tomakast_core::settings_schema::{NumberRange, ValueKind, value_fits};
use tomakast_places::cities::Cities;

use gpui_kit::base::animation::ease_out_cubic;
use gpui_kit::component::ActiveTheme as _;

use crate::chrome::key_chip;
use crate::theme;
use crate::views::hotkey_recorder::hotkey_of;

/// A value the user confirmed in a field: `None` removes the key, so the default applies.
pub(crate) struct Commit(pub Option<toml::Value>);

/// What a [`TextField`] holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TextKind {
    Text,
    /// A number within the range; typed text that is not one stays unwritten.
    Number(NumberRange),
}

/// A text or number field. Typing changes nothing in the file; Enter or leaving the field
/// writes (through [`Commit`]), Esc restores the file's value. Emptied, it removes the key.
pub(crate) struct TextField {
    input: Entity<InputState>,
    kind: TextKind,
    title: SharedString,
    /// The value the file holds (or the default), as text.
    file_value: String,
    disabled: bool,
    /// Why the typed text was not written.
    error: Option<String>,
    /// The field was emptied and sent as a removal: the file's answer is the default in
    /// effect, which the field then shows in place of the empty text.
    removing: bool,
    /// The file changed under the typed text and the user is asked which to keep: until
    /// then Enter or leaving the field writes nothing.
    held: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Commit> for TextField {}

impl TextField {
    pub(crate) fn new(
        kind: TextKind,
        title: impl Into<SharedString>,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TextField {
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).default_value(value.clone());
            match kind {
                TextKind::Text => input,
                // Up, Down and the stepper buttons step by `step`; the value is written on
                // Enter or blur as typed text is. The range is not given to the kit, which
                // would clamp on blur: a value out of range is refused with a hint on both.
                TextKind::Number(range) => input.step(range.step.unwrap_or(1.)),
            }
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit(window, cx),
                _ => {}
            },
        );
        TextField {
            input,
            kind,
            title: title.into(),
            file_value: value,
            disabled: false,
            error: None,
            removing: false,
            held: false,
            _subscriptions: vec![subscription],
        }
    }

    pub(crate) fn kind(&self) -> TextKind {
        self.kind
    }

    pub(crate) fn title(&self) -> &SharedString {
        &self.title
    }

    /// The file's value (or the default) as text: what Esc restores.
    pub(crate) fn file_value(&self) -> &str {
        &self.file_value
    }

    /// The accessible name, when what the field edits is renamed.
    pub(crate) fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
        if self.title != title {
            self.title = title;
            cx.notify();
        }
    }

    /// The typed text, when it differs from the file's value.
    pub(crate) fn pending(&self, cx: &App) -> Option<String> {
        let typed = self.input.read(cx).value();
        (typed != self.file_value.as_str()).then(|| typed.to_string())
    }

    /// The file's value now: what Esc restores. The typed text stays while it differs; the
    /// field shows the value when the typed text is this value (its own write coming back).
    /// `true` when the typed text stays and the value under it changed.
    pub(crate) fn set_from_file(
        &mut self,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let removing = std::mem::take(&mut self.removing);
        let keep_typed = self
            .pending(cx)
            .is_some_and(|typed| typed != value && !(removing && typed.trim().is_empty()));
        let changed = value != self.file_value;
        // Esc goes back to the file's value as it is now.
        self.file_value = value;
        if keep_typed {
            return changed;
        }
        self.error = None;
        self.show_file_value(window, cx);
        false
    }

    /// While held, Enter or leaving the field writes nothing.
    pub(crate) fn hold(&mut self, held: bool) {
        self.held = held;
    }

    /// Writes the typed text, as Enter does.
    pub(crate) fn keep_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(window, cx);
    }

    /// Drops the typed text for the file's value, as Esc does.
    pub(crate) fn use_file_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.show_file_value(window, cx);
    }

    pub(crate) fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }

    fn show_file_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.file_value.clone();
        self.input.update(cx, |input, cx| {
            if input.value() != value.as_str() {
                input.set_value(value, window, cx);
            }
        });
        cx.notify();
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.held {
            return;
        }
        let Some(typed) = self.pending(cx) else {
            return;
        };
        match parse_text(self.kind, &typed) {
            Ok(value) => {
                self.error = None;
                self.removing = value.is_none();
                // A number is shown as written ("05" as 5), so the file's answer matches it
                // and nothing stays typed.
                if let (TextKind::Number(_), Some(number)) = (self.kind, &value) {
                    let written = number.to_string();
                    if written != typed {
                        self.input
                            .update(cx, |input, cx| input.set_value(written, window, cx));
                    }
                }
                cx.emit(Commit(value));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn restore(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending(cx).is_none() && self.error.is_none() {
            cx.propagate();
            return;
        }
        self.error = None;
        self.show_file_value(window, cx);
    }
}

/// The value typed text stands for: `None` when empty (the key is removed).
fn parse_text(kind: TextKind, typed: &str) -> Result<Option<toml::Value>, String> {
    let trimmed = typed.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let range = match kind {
        TextKind::Text => return Ok(Some(toml::Value::String(typed.to_string()))),
        TextKind::Number(range) => range,
    };
    let value = match trimmed.parse::<i64>() {
        Ok(n) => toml::Value::Integer(n),
        Err(_) => match trimmed.parse::<f64>() {
            Ok(f) if f.is_finite() => toml::Value::Float(f),
            _ => return Err(number_hint(&range)),
        },
    };
    if value_fits(&ValueKind::Number(range), &value) {
        Ok(Some(value))
    } else {
        Err(number_hint(&range))
    }
}

/// What a number field takes, in words.
fn number_hint(range: &NumberRange) -> String {
    let what = if range.integer {
        "a whole number"
    } else {
        "a number"
    };
    match (range.min, range.max) {
        (Some(min), Some(max)) => format!("Enter {what} from {min} to {max}."),
        (Some(min), None) => format!("Enter {what} of at least {min}."),
        (None, Some(max)) => format!("Enter {what} of at most {max}."),
        (None, None) => format!("Enter {what}."),
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        div()
            .w(px(theme::settings_control_width()))
            .flex()
            .flex_col()
            .gap_1()
            .on_action(cx.listener(Self::restore))
            .child(match self.kind {
                TextKind::Text => Input::new(&self.input)
                    .aria_label(self.title.clone())
                    .disabled(self.disabled)
                    .bg(palette.field)
                    .border_color(palette.border)
                    .into_any_element(),
                TextKind::Number(_) => NumberInput::new(&self.input)
                    .accessibility_label(self.title.clone())
                    .disabled(self.disabled)
                    .bg(palette.field)
                    .border_color(palette.border)
                    .into_any_element(),
            })
            .children(self.error.clone().map(|error| {
                div()
                    .id("field-error")
                    .aria_label(error.clone())
                    .test_support()
                    .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                    .text_color(palette.error)
                    .child(error)
            }))
    }
}

/// A list of more entries than this has a search box over them; a shorter one is picked
/// from as it is.
pub(crate) const LONG_CHOICE: usize = 10;

/// An entry of a pick list: `value` is what the file holds, `title` what the user sees, with
/// a bundled icon before it when `icon` names one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PickItem {
    value: SharedString,
    title: SharedString,
    icon: Option<SharedString>,
}

impl PickItem {
    pub(crate) fn new(value: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        PickItem {
            value: value.into(),
            title: title.into(),
            icon: None,
        }
    }

    /// With the bundled icon `name` before the title (none when the bundle lacks it).
    pub(crate) fn icon(mut self, name: &str) -> Self {
        self.icon = crate::icons::bundled_path(name).map(SharedString::from);
        self
    }

    /// The icon and the title, as the list and the field show an entry with an icon.
    fn with_icon(&self) -> Option<AnyElement> {
        let path = self.icon.clone()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    gpui_kit::component::Icon::empty()
                        .path(path)
                        .size(px(theme::SETTINGS_CHOICE_ICON_SIZE)),
                )
                .child(div().min_w_0().truncate().child(self.title.clone()))
                .into_any_element(),
        )
    }
}

impl SelectItem for PickItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn display_title(&self) -> Option<AnyElement> {
        self.with_icon()
    }

    /// A line of the open list, named by its title so it can be found and clicked.
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("pick-item-{}", self.value)))
            .aria_label(self.title.clone())
            .test_support()
            .min_w_0()
            .child(
                self.with_icon()
                    .unwrap_or_else(|| self.title.clone().into_any_element()),
            )
    }

    fn value(&self) -> &SharedString {
        &self.value
    }
}

/// The title of the city list's first entry, which removes the key; and what a plugin's
/// preference without a value shows when the plugin works its defaults out as it runs.
pub(crate) const AUTOMATIC: &str = "Automatic";

/// What a plugin's preference without a value shows when its plugin tells no defaults.
pub(crate) const NOT_SET: &str = "Not set";

type PickSelect = SelectState<SearchableVec<PickItem>>;

/// A value picked from a searchable list: the city, a time zone, a choice of many options. A
/// pick writes at once, so the field never holds an unwritten value. While the file has no
/// value the field shows the one in effect (`default`: the computer's zone, the plugin's own
/// default), or `unset` ("Automatic", "Not set") when that is not known; picking it writes
/// nothing. A value the
/// list does not have (written by hand) shows as written.
pub(crate) struct PickField {
    select: Entity<PickSelect>,
    /// The element id of the control; unique on its page.
    id: SharedString,
    title: SharedString,
    search_placeholder: SharedString,
    items: Vec<PickItem>,
    /// The file's value; empty: none.
    file_value: String,
    /// The file holds a value of the wrong type: any pick writes.
    file_invalid: bool,
    /// The value in effect while the file has none, when it is known.
    default: Option<String>,
    /// What the field shows while neither the file nor `default` gives it a value.
    unset: SharedString,
    disabled: bool,
    /// The pointer is down on the trigger: the kit's `Select` stops the press GPUI's own
    /// pressed style listens for, so it is tracked here.
    pressed: bool,
    _subscription: Subscription,
}

impl EventEmitter<Commit> for PickField {}

/// What a new [`PickField`] is.
pub(crate) struct PickSpec {
    pub(crate) id: SharedString,
    pub(crate) title: SharedString,
    pub(crate) search_placeholder: SharedString,
    pub(crate) items: Vec<PickItem>,
    /// What the field shows while neither the file nor the default gives it a value.
    pub(crate) unset: SharedString,
}

impl PickField {
    pub(crate) fn new(
        spec: PickSpec,
        value: String,
        invalid: bool,
        default: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PickField {
        let shown = shown_value(&value, default.as_deref());
        let selected = spec
            .items
            .iter()
            .position(|item| item.value.as_str() == shown)
            .map(IndexPath::new);
        let select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(spec.items.clone()), selected, window, cx)
                .searchable(spec.items.len() > LONG_CHOICE)
        });
        let subscription = cx.subscribe(
            &select,
            |this, _, event: &SelectEvent<SearchableVec<PickItem>>, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let shown = shown_value(&this.file_value, this.default.as_deref());
                if value.as_str() == shown && !this.file_invalid {
                    return;
                }
                let value = (!value.is_empty()).then(|| toml::Value::String(value.to_string()));
                cx.emit(Commit(value));
            },
        );
        PickField {
            select,
            id: spec.id,
            title: spec.title,
            search_placeholder: spec.search_placeholder,
            items: spec.items,
            file_value: value,
            file_invalid: invalid,
            default,
            unset: spec.unset,
            disabled: false,
            pressed: false,
            _subscription: subscription,
        }
    }

    pub(crate) fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
        if self.title != title {
            self.title = title;
            cx.notify();
        }
    }

    /// What the field shows while neither the file nor the default gives it a value.
    pub(crate) fn set_unset(&mut self, unset: SharedString, cx: &mut Context<Self>) {
        if self.unset != unset {
            self.unset = unset;
            cx.notify();
        }
    }

    /// Whether the field lists exactly `items`, in this order.
    pub(crate) fn lists(&self, items: &[PickItem]) -> bool {
        self.items == items
    }

    /// The file's value (empty: none); `invalid`: it holds one of the wrong type (shown as
    /// the default). `default`: the value in effect while the file has none.
    pub(crate) fn set_from_file(
        &mut self,
        value: String,
        invalid: bool,
        default: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_value = value;
        self.file_invalid = invalid;
        self.default = default;
        let shown = SharedString::from(shown_value(&self.file_value, self.default.as_deref()));
        // A pick whose write failed left the list on it: set it back. Only on a difference,
        // since setting it clears an open search.
        let shows = self.select.read(cx).selected_value().cloned();
        let listed = self.items.iter().any(|item| item.value == shown);
        let differs = match &shows {
            Some(shows) => *shows != shown,
            None => listed,
        };
        if differs {
            self.select.update(cx, |select, cx| {
                select.set_selected_value(&shown, window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }
}

/// The value a pick field stands on: the file's, else the default; empty when neither.
fn shown_value<'a>(file_value: &'a str, default: Option<&'a str>) -> &'a str {
    if file_value.is_empty() {
        default.unwrap_or_default()
    } else {
        file_value
    }
}

/// The city list: "Automatic" (which removes the key), then every city name once, the most
/// populous city of a name first.
pub(crate) fn city_items() -> Vec<PickItem> {
    let cities = Cities::builtin();
    let mut seen = HashSet::new();
    let names = cities
        .names()
        .filter(|name| seen.insert(*name))
        .map(|name| PickItem::new(name.to_string(), name.to_string()));
    std::iter::once(PickItem::new(SharedString::default(), AUTOMATIC))
        .chain(names)
        .collect()
}

/// The IANA time zone names of jiff's bundled database (the one calc uses), sorted.
pub(crate) fn zone_names() -> &'static [String] {
    static NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
        let mut names: Vec<String> = jiff::tz::TimeZoneDatabase::bundled()
            .available()
            .map(|name| name.as_str().to_string())
            .collect();
        names.sort();
        names.dedup();
        names
    });
    &NAMES
}

/// The time zone list, each zone titled by its name. Made once: a settings window asks for
/// it on every change of the file or the catalog.
pub(crate) fn zone_items() -> &'static [PickItem] {
    static ITEMS: LazyLock<Vec<PickItem>> = LazyLock::new(|| {
        zone_names()
            .iter()
            .map(|name| PickItem::new(name.clone(), name.clone()))
            .collect()
    });
    &ITEMS
}

/// The zone the computer is in, as the plugins' `locale` import answers and calc reads it: the
/// list's spelling of `detected`, or "UTC" when it cannot be told or the list does not have
/// it.
pub(crate) fn computer_zone(detected: Option<&str>) -> String {
    detected
        .and_then(|zone| {
            zone_names()
                .iter()
                .find(|name| name.eq_ignore_ascii_case(zone))
        })
        .cloned()
        .unwrap_or_else(|| "UTC".to_string())
}

impl Render for PickField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        // A value the list does not have (written by hand) shows as written; with none, the
        // default, or `unset` when it is not known.
        let shown = match shown_value(&self.file_value, self.default.as_deref()) {
            "" => self.unset.clone(),
            value => SharedString::from(value.to_string()),
        };
        // The kit's trigger has no hover of its own: this box is the trigger's fill (the kit's
        // own one is transparent), and it takes the control buttons' hover and pressed fills.
        let fills = palette.control_fills();
        let disabled = self.disabled;
        let pressed = self.pressed && !disabled;
        div()
            .id(SharedString::from(format!("{}-fill", self.id)))
            .w(px(theme::settings_control_width()))
            .rounded(cx.theme().radius)
            .bg(if pressed { fills.active } else { fills.rest })
            .when(!disabled && !pressed, |fill| {
                fill.hover(move |style| style.bg(fills.hover))
            })
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left {
                    this.pressed = true;
                    cx.notify();
                }
            }))
            .capture_any_mouse_up(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                this.pressed = false;
                cx.notify();
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.pressed = false;
                    cx.notify();
                }),
            )
            .child(
                Select::new(&self.select)
                    .id(self.id.clone())
                    .accessibility_label(self.title.clone())
                    .placeholder(shown)
                    .search_placeholder(self.search_placeholder.clone())
                    .disabled(disabled)
                    .bg(gpui_kit::transparent_black())
                    .border_color(fills.border),
            )
    }
}

/// What an empty hotkey field reads.
pub(crate) const ADD_HOTKEY: &str = "Add hotkey";
/// What a recording hotkey field reads until a combination is pressed.
pub(crate) const PRESS_KEYS: &str = "Press keys…";
/// How a recording ends, under the field.
const RECORDING_HINT: &str = "↵ Keep · Esc Cancel";
/// The clear button inside a hotkey field and the cell of its state icon, and the icons in
/// them.
const HOTKEY_BUTTON_SIZE: f32 = 20.;
const HOTKEY_ICON_SIZE: f32 = 14.;
/// Least space between a hotkey field's keys and the icons at its right end.
const HOTKEY_END_GAP: f32 = 8.;
/// Space between a hotkey field's right edge and the clear button: as above and below it.
const HOTKEY_END_PADDING: f32 = (theme::CONTROL_HEIGHT - 2. - HOTKEY_BUTTON_SIZE) / 2.;
/// Space between a hotkey field and the hint under it.
const HOTKEY_OVERLAY_GAP: f32 = 4.;
/// Width of the fade over the end of keys too long for a hotkey field.
const HOTKEY_FADE_WIDTH: f32 = 24.;

/// The hotkey a recorded combination was confirmed as.
pub(crate) struct HotkeyCommit(pub Hotkey);

/// The user asked to clear the hotkey: an item's goes, the launcher's goes back to the
/// default.
pub(crate) struct HotkeyClear;

/// Why a combination cannot be used, in words; `None`: it can.
pub(crate) type HotkeyCheck = Rc<dyn Fn(&Hotkey, &App) -> Option<String>>;

/// How a hotkey stands besides its keys, in words: shown as an icon inside its field, the
/// words its tooltip and its field's accessible description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HotkeyStand {
    pub(crate) tone: StandTone,
    pub(crate) text: String,
}

/// Which icon a [`HotkeyStand`] shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StandTone {
    /// Saved; it works after a restart.
    Restart,
    /// No hotkeys in this run.
    Off,
    /// Refused by the system, not valid, or a change that did not go through.
    Problem,
}

/// A hotkey: click, press a combination, then Enter or click away to keep it, Esc to cancel.
/// The kept one goes out as [`HotkeyCommit`], the clear button's click as [`HotkeyClear`];
/// it shows as asked for until the file has it, or the change comes back refused. It is drawn
/// as a text field, and its size never changes: what it reads starts where a text field's
/// text does, the icon of how the hotkey stands and the clear button sit at its right end
/// when there are any, the recording hint and a refusal float under it.
pub(crate) struct HotkeyField {
    title: SharedString,
    /// The file's hotkey; `None`: it has none.
    file_value: Option<Hotkey>,
    /// What the field shows without a hotkey of its own (the launcher's default); clearing
    /// goes back to it.
    default: Option<Hotkey>,
    /// Sent, not yet in the file; `Some(None)`: asked to be cleared.
    requested: Option<Option<Hotkey>>,
    recording: bool,
    shown: Option<Hotkey>,
    refusal: Option<String>,
    stand: Option<HotkeyStand>,
    check: HotkeyCheck,
    disabled: bool,
    focus: FocusHandle,
    clear_focus: FocusHandle,
    /// The keys, as last laid out, ran past their room: they are cut under a fade, and the
    /// tooltip reads them whole. Set after layout; a change draws the field again.
    cut: Rc<Cell<bool>>,
    _blur: Subscription,
}

impl EventEmitter<HotkeyCommit> for HotkeyField {}
impl EventEmitter<HotkeyClear> for HotkeyField {}

impl HotkeyField {
    pub(crate) fn new(
        title: impl Into<SharedString>,
        value: Option<Hotkey>,
        check: HotkeyCheck,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> HotkeyField {
        // Reached with Tab, started with Enter or Space.
        let focus = cx.focus_handle().tab_stop(true);
        let blur = cx.on_blur(&focus, window, |this, _, cx| this.keep(cx));
        HotkeyField {
            title: title.into(),
            file_value: value,
            default: None,
            requested: None,
            recording: false,
            shown: None,
            refusal: None,
            stand: None,
            check,
            disabled: false,
            focus,
            clear_focus: cx.focus_handle().tab_stop(true),
            cut: Rc::default(),
            _blur: blur,
        }
    }

    /// Shown while the file has no hotkey; the clear button then resets to it.
    pub(crate) fn with_default(mut self, default: Hotkey) -> HotkeyField {
        self.default = Some(default);
        self
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.recording
    }

    /// Whether the user is in the field: recording, or the focus is on it or its clear button.
    pub(crate) fn is_in_use(&self, window: &Window, cx: &App) -> bool {
        self.recording || self.focus.contains_focused(window, cx)
    }

    /// The accessible name, when what the field edits is renamed.
    pub(crate) fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
        if self.title != title {
            self.title = title;
            cx.notify();
        }
    }

    /// The file's hotkey now. A recording goes on; Esc then shows this one.
    pub(crate) fn set_from_file(
        &mut self,
        value: Option<Hotkey>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.requested.as_ref().is_none_or(|asked| *asked == value) {
            self.requested = None;
        }
        self.file_value = value;
        cx.notify();
    }

    /// The last request did not go through as asked: show the file's hotkey again.
    pub(crate) fn drop_request(&mut self, cx: &mut Context<Self>) {
        self.requested = None;
        cx.notify();
    }

    /// How the hotkey stands now; `None`: as it should.
    pub(crate) fn set_stand(&mut self, stand: Option<HotkeyStand>, cx: &mut Context<Self>) {
        if self.stand != stand {
            self.stand = stand;
            cx.notify();
        }
    }

    pub(crate) fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            if disabled {
                self.cancel(cx);
            }
            cx.notify();
        }
    }

    /// A tomakast hotkey the system fired while recording: shown as if pressed here.
    pub(crate) fn recorded_press(&mut self, hotkey: Hotkey, cx: &mut Context<Self>) {
        if self.recording {
            self.show(hotkey, cx);
        }
    }

    /// Focuses the field and starts recording, as a click does.
    pub(crate) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.recording {
            return;
        }
        self.recording = true;
        self.shown = None;
        self.refusal = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn show(&mut self, hotkey: Hotkey, cx: &mut Context<Self>) {
        self.refusal = (self.check)(&hotkey, cx);
        self.shown = Some(hotkey);
        cx.notify();
    }

    /// Ends recording without keeping anything.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        self.recording = false;
        self.shown = None;
        self.refusal = None;
        cx.notify();
    }

    /// Ends recording, sending the shown combination when it can be used and is new.
    fn keep(&mut self, cx: &mut Context<Self>) {
        if !self.recording {
            return;
        }
        let kept = match (&self.shown, &self.refusal) {
            (Some(hotkey), None) if Some(hotkey) != self.current().as_ref() => Some(hotkey.clone()),
            _ => None,
        };
        self.cancel(cx);
        if let Some(hotkey) = kept {
            self.requested = Some(Some(hotkey.clone()));
            cx.emit(HotkeyCommit(hotkey));
        }
    }

    /// The clear button: asks for the hotkey to go; the field shows the default meanwhile.
    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.disabled || self.recording || !self.clearable() {
            return;
        }
        self.requested = Some(None);
        cx.emit(HotkeyClear);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.recording {
            // Not when they reach here from the clear button, which takes its own.
            if (event.keystroke.key == "enter" || event.keystroke.key == "space")
                && self.focus.is_focused(window)
            {
                self.start(window, cx);
            }
            return;
        }
        cx.stop_propagation();
        if event.is_held {
            return;
        }
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;
        let plain = !(m.control || m.alt || m.shift || m.platform || m.function);
        match keystroke.key.as_str() {
            "escape" if plain => self.cancel(cx),
            "enter" if plain => {
                if self.shown.is_some() && self.refusal.is_none() {
                    self.keep(cx);
                }
            }
            _ => {
                if let Some(hotkey) = hotkey_of(keystroke) {
                    self.show(hotkey, cx);
                }
            }
        }
    }

    /// The field's own hotkey (asked for, or the file's), without the default.
    fn own(&self) -> Option<Hotkey> {
        match &self.requested {
            Some(asked) => asked.clone(),
            None => self.file_value.clone(),
        }
    }

    /// Whether the clear button would change anything: the field has a hotkey of its own,
    /// other than the default it resets to.
    fn clearable(&self) -> bool {
        match (self.own(), &self.default) {
            (Some(own), Some(default)) => own != *default,
            (own, _) => own.is_some(),
        }
    }

    /// The hotkey shown when not recording.
    fn current(&self) -> Option<Hotkey> {
        self.own().or_else(|| self.default.clone())
    }

    /// The clear button's accessible name, tooltip and icon: the launcher's resets to the
    /// default, an item's clears.
    fn clear_words(&self) -> (String, &'static str, IconName) {
        match self.default {
            Some(_) => (
                format!("Reset {}", self.title),
                "Reset to default",
                IconName::Undo2,
            ),
            None => (
                format!("Clear hotkey of {}", self.title),
                "Clear",
                IconName::Close,
            ),
        }
    }
}

impl Render for HotkeyField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let shown = if self.recording {
            self.shown.clone()
        } else {
            self.current()
        };
        let value = match &shown {
            Some(hotkey) => hotkey.label(),
            None if self.recording => PRESS_KEYS.to_string(),
            None => ADD_HOTKEY.to_string(),
        };
        let clearable = !self.recording && !self.disabled && self.clearable();
        // The chips never wrap: a combination too long for the field (four modifiers, or three
        // and a long key, next to the icons) is cut under a fade, and read whole in a tooltip.
        let cut = shown.is_some() && self.cut.get();
        let body = match &shown {
            Some(hotkey) => div()
                .id("hotkey-chips")
                .test_support()
                .flex_none()
                .flex()
                .gap_1()
                .children(
                    crate::shortcut::chips(hotkey)
                        .iter()
                        .map(|k| key_chip(k, &palette)),
                )
                .into_any_element(),
            None => div()
                .id("hotkey-placeholder")
                .test_support()
                .flex_none()
                // The size of a text field's text and placeholder.
                .text_sm()
                .text_color(palette.muted)
                .child(value.clone())
                .into_any_element(),
        };
        // The text fields' box, whatever it holds; recording or focused, the focus ring the
        // kit's text fields draw (its border and the band around it).
        let focused = self.recording || self.focus.is_focused(window);
        let stand = self.stand.clone().map(|stand| {
            let (icon, color) = match stand.tone {
                StandTone::Restart => ("clock", palette.muted),
                StandTone::Off => ("circle-off", palette.muted),
                StandTone::Problem => ("circle-alert", palette.error),
            };
            let text = SharedString::from(stand.text);
            div()
                .id("hotkey-stand")
                .test_support()
                .flex_none()
                .size(px(HOTKEY_BUTTON_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .child(icon_svg(icon).text_color(color))
                .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
        });
        // The reset buttons' button: their hover, press and focus ring.
        let clear = clearable.then(|| {
            let (label, tooltip, icon) = self.clear_words();
            div().flex_none().child(
                Button::new("hotkey-clear")
                    .icon(icon)
                    .ghost()
                    .xsmall()
                    .accessibility_label(label)
                    .tooltip(tooltip)
                    .track_focus(&self.clear_focus)
                    .on_click(cx.listener(|this, _, window, cx| {
                        // Not a click on the field, which would start recording.
                        cx.stop_propagation();
                        this.clear(cx);
                        // The button goes with the hotkey: the focus stays in the field.
                        this.focus.focus(window, cx);
                    })),
            )
        });
        // Over the rows under the field, out of their flow, so nothing moves.
        let overlay = self.recording.then(|| {
            let refusal = self.refusal.clone().map(|text| {
                div()
                    .id("hotkey-refusal")
                    .aria_label(text.clone())
                    .test_support()
                    .text_color(palette.error)
                    .child(text)
            });
            div().absolute().top_full().right_0().child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(HOTKEY_OVERLAY_GAP)))
                        .snap_to_window_with_margin(px(theme::POPOVER_MARGIN))
                        .child(
                            // The lists' surface, and their way in: it slides down and fades in.
                            div()
                                .id("hotkey-overlay")
                                .test_support()
                                .occlude()
                                .relative()
                                .max_w(px(theme::POPOVER_WIDTH))
                                .flex()
                                .flex_col()
                                .items_end()
                                .gap_1()
                                .px_2()
                                .py_1()
                                .popover_style(cx)
                                .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                                .children(refusal)
                                .child(div().text_color(palette.muted).child(RECORDING_HINT))
                                .with_animation(
                                    "hotkey-overlay-enter",
                                    Animation::new(theme::POPOVER_ENTER_DURATION)
                                        .with_easing(ease_out_cubic),
                                    |overlay, delta| {
                                        overlay
                                            .top(px(theme::POPOVER_ENTER_OFFSET * (1. - delta)))
                                            .opacity(delta)
                                    },
                                ),
                        ),
                )
                .with_priority(1),
            )
        });
        let whole = SharedString::from(value.clone());
        let fill = palette.control_fills().rest;
        div()
            .id("hotkey-field")
            .role(Role::Button)
            .aria_label(self.title.clone())
            .aria_value(value)
            .when_some(self.stand.as_ref(), |el, stand| {
                el.aria_description(SharedString::from(stand.text.clone()))
            })
            .test_support()
            .track_focus(&self.focus)
            .key_context("SettingsHotkeyField")
            .on_key_down(cx.listener(Self::key_down))
            .relative()
            .flex_none()
            .w(px(theme::settings_control_width()))
            .h(px(theme::CONTROL_HEIGHT))
            .pl(px(theme::CONTROL_PADDING_X))
            .pr(px(HOTKEY_END_PADDING))
            .flex()
            .items_center()
            .rounded(cx.theme().radius)
            .bg(palette.field)
            .border_1()
            .border_color(palette.border)
            .when(focused && !self.disabled, |el| {
                el.focus_ring_style(window, cx)
            })
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .when(!self.disabled, |el| {
                // Enter and Space start recording in `key_down`; their keyboard
                // click on release would start it again after Enter kept a
                // combination.
                el.cursor_pointer().on_click(cx.listener(
                    |this, event: &gpui_kit::ClickEvent, window, cx| {
                        if !event.is_keyboard() {
                            this.start(window, cx);
                        }
                    },
                ))
            })
            .when(cut, |el| {
                el.tooltip(move |window, cx| Tooltip::new(whole.clone()).build(window, cx))
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(body)
                    // The keys' room, to tell after layout whether they run past it.
                    .child(div().absolute().top_0().left_0().size_full())
                    .on_children_prepainted({
                        let cut = self.cut.clone();
                        let field = cx.entity_id();
                        move |children, _, cx| {
                            let now = match children.as_slice() {
                                [keys, room, ..] => keys.right() > room.right(),
                                _ => false,
                            };
                            if cut.replace(now) != now {
                                cx.notify(field);
                            }
                        }
                    })
                    .when(cut, |el| {
                        el.child(
                            div()
                                .id("hotkey-fade")
                                .test_support()
                                .absolute()
                                .top_0()
                                .right_0()
                                .h_full()
                                .w(px(HOTKEY_FADE_WIDTH))
                                // To the field's fill as it shows on a settings box, opaque:
                                // the field's own tint again would light a box over the keys.
                                .bg(linear_gradient(
                                    90.,
                                    linear_color_stop(fill.opacity(0.), 0.),
                                    linear_color_stop(fill, 1.),
                                )),
                        )
                    }),
            )
            .when(stand.is_some() || clear.is_some(), |el| {
                el.child(
                    div()
                        .flex_none()
                        .ml(px(HOTKEY_END_GAP))
                        .flex()
                        .items_center()
                        .children(stand)
                        .children(clear),
                )
            })
            .children(overlay)
    }
}

/// A bundled icon at the size a hotkey field draws them, in the current text colour.
fn icon_svg(name: &str) -> gpui_kit::Svg {
    gpui_kit::svg()
        .path(crate::icons::bundled_path(name).unwrap_or_default())
        .size(px(HOTKEY_ICON_SIZE))
}

/// The kit's focus ring colour.
#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{AnyWindowHandle, MouseButton, QuitMode, TestAppContext};

    /// A window with one text field and something else to focus.
    struct Host {
        field: Entity<TextField>,
        other: FocusHandle,
        commits: Vec<Option<toml::Value>>,
        _commits: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.field.clone())
                .child(div().track_focus(&self.other).size(px(20.)))
        }
    }

    fn host(
        cx: &mut TestAppContext,
        kind: TextKind,
        value: &str,
    ) -> (AnyWindowHandle, Entity<Host>) {
        let value = value.to_string();
        let opened = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let field = cx.new(|cx| TextField::new(kind, "Name", value, window, cx));
                cx.new(|cx| Host {
                    _commits: cx.subscribe(&field, |host: &mut Host, _, Commit(value), _| {
                        host.commits.push(value.clone())
                    }),
                    field,
                    other: cx.focus_handle(),
                    commits: Vec::new(),
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        // Active, as for the user: focus moves then report blurs.
        cx.update_window(opened.0, |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        opened
    }

    fn on_window<R>(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        let out = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                let out = f(window, cx);
                window.render_frame(cx);
                out
            })
            .unwrap();
        cx.run_until_parked();
        out
    }

    /// Clicks into the field and types `text` at its end.
    fn type_into(cx: &mut TestAppContext, w: AnyWindowHandle, text: &str) {
        on_window(cx, w, |window, cx| {
            let at = window.find_by_label("Name").bounds().center();
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
            window.press("end", cx);
            window.input(text, cx);
        });
    }

    fn typed(cx: &mut TestAppContext, host: &Entity<Host>) -> String {
        cx.update(|cx| {
            host.read(cx)
                .field
                .read(cx)
                .input
                .read(cx)
                .value()
                .to_string()
        })
    }

    fn commits(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<Option<toml::Value>> {
        cx.update(|cx| host.read(cx).commits.clone())
    }

    #[gpui_kit::test]
    fn text_field_writes_only_on_enter_or_blur(cx: &mut TestAppContext) {
        let (w, host) = host(cx, TextKind::Text, "ab");
        type_into(cx, w, "c");
        assert_eq!(typed(cx, &host), "abc");
        assert!(commits(cx, &host).is_empty());
        on_window(cx, w, |window, cx| window.press("enter", cx));
        assert_eq!(
            commits(cx, &host),
            [Some(toml::Value::String("abc".into()))]
        );

        // The write comes back from the file; then a blur writes the next change.
        cx.update_window(w, |_, window, cx| {
            host.read(cx).field.clone().update(cx, |field, cx| {
                field.set_from_file("abc".into(), window, cx);
                assert_eq!(field.pending(cx), None);
            })
        })
        .unwrap();
        type_into(cx, w, "d");
        on_window(cx, w, |window, cx| {
            let other = host.read(cx).other.clone();
            window.focus(&other, cx);
        });
        assert_eq!(
            commits(cx, &host).last(),
            Some(&Some(toml::Value::String("abcd".into())))
        );
    }

    #[gpui_kit::test]
    fn esc_restores_the_file_value(cx: &mut TestAppContext) {
        let (w, host) = host(cx, TextKind::Text, "ab");
        type_into(cx, w, "zz");
        on_window(cx, w, |window, cx| window.press("escape", cx));
        assert_eq!(typed(cx, &host), "ab");
        on_window(cx, w, |window, cx| window.press("enter", cx));
        assert!(commits(cx, &host).is_empty());
    }

    #[gpui_kit::test]
    fn the_file_does_not_overwrite_typed_text(cx: &mut TestAppContext) {
        let (w, host) = host(cx, TextKind::Text, "ab");
        type_into(cx, w, "c");
        cx.update_window(w, |_, window, cx| {
            host.read(cx).field.clone().update(cx, |field, cx| {
                field.set_from_file("other".into(), window, cx);
                assert_eq!(field.pending(cx).as_deref(), Some("abc"));
            })
        })
        .unwrap();
        assert_eq!(typed(cx, &host), "abc");
        // Esc goes back to the file as it is now.
        on_window(cx, w, |window, cx| window.press("escape", cx));
        assert_eq!(typed(cx, &host), "other");
    }

    #[gpui_kit::test]
    fn a_number_out_of_range_is_not_written(cx: &mut TestAppContext) {
        let range = NumberRange {
            min: Some(1.),
            max: Some(10.),
            step: None,
            integer: true,
        };
        let (w, host) = host(cx, TextKind::Number(range), "5");
        type_into(cx, w, "0");
        on_window(cx, w, |window, cx| window.press("enter", cx));
        assert!(commits(cx, &host).is_empty());
        assert!(on_window(cx, w, |window, _| {
            window
                .try_find_by_label("Enter a whole number from 1 to 10.")
                .is_some()
        }));
    }

    #[test]
    fn text_is_kept_as_typed_and_empty_removes() {
        assert_eq!(
            parse_text(TextKind::Text, " a b "),
            Ok(Some(toml::Value::String(" a b ".into())))
        );
        assert_eq!(parse_text(TextKind::Text, "  "), Ok(None));
    }

    #[test]
    fn numbers_are_checked_against_their_range() {
        let range = NumberRange {
            min: Some(1.),
            max: Some(10.),
            step: Some(1.),
            integer: true,
        };
        let kind = TextKind::Number(range);
        assert_eq!(parse_text(kind, "5"), Ok(Some(toml::Value::Integer(5))));
        assert_eq!(
            parse_text(kind, "11"),
            Err("Enter a whole number from 1 to 10.".into())
        );
        assert_eq!(
            parse_text(kind, "2.5"),
            Err("Enter a whole number from 1 to 10.".into())
        );
        assert_eq!(
            parse_text(kind, "x"),
            Err("Enter a whole number from 1 to 10.".into())
        );
        let any = TextKind::Number(NumberRange::default());
        assert_eq!(parse_text(any, "2.5"), Ok(Some(toml::Value::Float(2.5))));
        assert_eq!(parse_text(any, "inf"), Err("Enter a number.".into()));
    }

    #[test]
    fn the_city_list_starts_with_automatic_and_names_each_city_once() {
        let items = city_items();
        assert_eq!(items[0].title.as_str(), AUTOMATIC);
        assert_eq!(items[0].value.as_str(), "");
        let mut seen = HashSet::new();
        assert!(items.iter().all(|item| seen.insert(item.value.clone())));
        assert!(items.iter().any(|item| item.value.as_str() == "Berlin"));
    }

    #[test]
    fn the_zone_list_is_jiffs_bundled_database() {
        let names = zone_names();
        assert!(names.len() > 400, "{}", names.len());
        for zone in [
            "Europe/Moscow",
            "Asia/Tokyo",
            "America/Argentina/Buenos_Aires",
            "UTC",
        ] {
            assert!(names.iter().any(|n| n == zone), "{zone}");
        }
        assert!(names.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_computers_zone_is_spelled_as_the_list_or_utc() {
        assert_eq!(computer_zone(Some("Europe/Berlin")), "Europe/Berlin");
        assert_eq!(computer_zone(Some("europe/berlin")), "Europe/Berlin");
        assert_eq!(computer_zone(Some("Mars/Base")), "UTC");
        assert_eq!(computer_zone(None), "UTC");
    }

    /// A window with a text field over a hotkey field, as a settings page draws them.
    struct Pair {
        text: Entity<TextField>,
        hotkey: Entity<HotkeyField>,
    }

    impl Render for Pair {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.text.clone())
                .child(self.hotkey.clone())
        }
    }

    const FORECAST: &str = "Forecast";

    /// A window with an empty text field "Name" and an empty hotkey field "Forecast".
    fn pair(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<HotkeyField>) {
        let (w, pair) = cx.update(|cx| {
            cx.set_quit_mode(QuitMode::Explicit);
            crate::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                let text =
                    cx.new(|cx| TextField::new(TextKind::Text, "Name", String::new(), window, cx));
                let check: HotkeyCheck = Rc::new(|_, _| None);
                let hotkey = cx.new(|cx| HotkeyField::new(FORECAST, None, check, window, cx));
                cx.new(|_| Pair { text, hotkey })
            })
            .unwrap()
        });
        cx.run_until_parked();
        let hotkey = cx.update(|cx| pair.read(cx).hotkey.clone());
        (w, hotkey)
    }

    fn update_hotkey(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        field: &Entity<HotkeyField>,
        f: impl FnOnce(&mut HotkeyField, &mut Window, &mut Context<HotkeyField>),
    ) {
        cx.update_window(w, |_, window, cx| {
            field.update(cx, |field, cx| f(field, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn hotkey(text: &str) -> Hotkey {
        Hotkey::parse(text).unwrap()
    }

    type Rect = gpui_kit::Bounds<gpui_kit::Pixels>;

    fn find(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> Rect {
        on_window(cx, w, |window, _| window.find(id).bounds())
    }

    fn field_box(cx: &mut TestAppContext, w: AnyWindowHandle) -> Rect {
        on_window(cx, w, |window, _| window.find_by_label(FORECAST).bounds())
    }

    /// The smallest painted box with an edge (its fill, edge, corners) around `inner`, and its
    /// bounds.
    fn painted_box(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        inner: Rect,
    ) -> (gpui_kit::Quad, Rect) {
        on_window(cx, w, |window, _| {
            let scale = window.scale_factor();
            let center = inner.center().scale(scale);
            let quad = window
                .painted_quads()
                .into_iter()
                .filter(|quad| quad.border_widths.left.0 > 0. && quad.bounds.contains(&center))
                .min_by(|a, b| {
                    a.bounds
                        .size
                        .width
                        .partial_cmp(&b.bounds.size.width)
                        .unwrap()
                })
                .unwrap_or_else(|| panic!("no box around {inner:?}"));
            let b = quad.bounds;
            let bounds = gpui_kit::Bounds::new(
                gpui_kit::point(px(b.origin.x.0 / scale), px(b.origin.y.0 / scale)),
                gpui_kit::size(px(b.size.width.0 / scale), px(b.size.height.0 / scale)),
            );
            (quad, bounds)
        })
    }

    /// The hotkey field is the text field's box: its fill, edge, corners and height, empty or
    /// not, and its text starts where the text field's does.
    #[gpui_kit::test]
    fn a_hotkey_field_is_drawn_as_a_text_field(cx: &mut TestAppContext) {
        let (w, field) = pair(cx);
        let input = on_window(cx, w, |window, _| {
            window
                .find_all_by_label("Name")
                .into_iter()
                .map(|e| e.bounds())
                .max_by(|a, b| a.size.width.partial_cmp(&b.size.width).unwrap())
                .unwrap()
        });
        let (text_box, text_frame) = painted_box(cx, w, input);
        assert_eq!(
            input.left() - text_frame.left(),
            px(theme::CONTROL_TEXT_INSET)
        );
        let (empty, words) = (field_box(cx, w), find(cx, w, "hotkey-placeholder"));
        let (empty_box, empty_frame) = painted_box(cx, w, empty);
        assert_eq!(empty_frame, empty, "the field's own box");
        update_hotkey(cx, w, &field, |field, window, cx| {
            field.set_from_file(Some(hotkey("ctrl+alt+w")), window, cx)
        });
        let set = field_box(cx, w);
        let (set_box, set_frame) = painted_box(cx, w, set);
        assert_eq!(set_frame, set, "the field's own box");
        for quad in [&empty_box, &set_box] {
            assert_eq!(quad.background, text_box.background);
            assert_eq!(quad.border_color, text_box.border_color);
            assert_eq!(quad.border_widths, text_box.border_widths);
            assert_eq!(quad.corner_radii, text_box.corner_radii);
            assert_eq!(quad.bounds.size.height, text_box.bounds.size.height);
        }
        assert_eq!(
            words.left() - empty.left(),
            px(theme::CONTROL_TEXT_INSET),
            "{words:?} {empty:?}"
        );
    }

    /// As a text field, the hotkey field shows the focus ring while it has the focus, recording
    /// or not; without the focus, the text fields' edge.
    #[gpui_kit::test]
    fn a_focused_hotkey_field_has_the_ring(cx: &mut TestAppContext) {
        let (w, field) = pair(cx);
        let edge = |cx: &mut TestAppContext| {
            let bounds = field_box(cx, w);
            painted_box(cx, w, bounds).0.border_color
        };
        // The band the kit's text fields draw around themselves when focused: an edged box
        // wider than the field.
        let band = |cx: &mut TestAppContext| {
            let bounds = field_box(cx, w);
            on_window(cx, w, |window, _| {
                let scale = window.scale_factor();
                let center = bounds.center().scale(scale);
                let width = bounds.size.width.scale(scale);
                window.painted_quads().into_iter().any(|quad| {
                    quad.border_widths.left.0 > 0.
                        && quad.bounds.contains(&center)
                        && quad.bounds.size.width > width
                })
            })
        };
        let (ring, border) = cx.update(|cx| {
            use gpui_kit::component::ActiveTheme as _;
            (cx.theme().ring, theme::palette(cx).border)
        });
        assert_eq!(edge(cx), border);
        assert!(!band(cx));
        update_hotkey(cx, w, &field, |field, window, cx| field.start(window, cx));
        assert_eq!(edge(cx), ring, "recording");
        assert!(band(cx), "recording");
        update_hotkey(cx, w, &field, |field, _, cx| field.cancel(cx));
        assert_eq!(edge(cx), ring, "focused");
        assert!(band(cx), "focused");
    }

    /// Focused, the hotkey field draws the focused text field's ring: the same edge, and the
    /// same band, as far out, in the same colours, with the same corners.
    #[gpui_kit::test]
    fn the_hotkey_ring_is_the_text_fields(cx: &mut TestAppContext) {
        let (w, field) = pair(cx);
        // The edge and the band around the box at `inner`: the band is the smallest edged quad
        // wider than the box.
        let ring = |cx: &mut TestAppContext, inner: Rect| {
            let (edge, frame) = painted_box(cx, w, inner);
            let band = on_window(cx, w, |window, _| {
                let scale = window.scale_factor();
                let center = inner.center().scale(scale);
                let width = frame.size.width.scale(scale);
                window
                    .painted_quads()
                    .into_iter()
                    .filter(|quad| {
                        quad.border_widths.left.0 > 0.
                            && quad.bounds.contains(&center)
                            && quad.bounds.size.width > width
                    })
                    .min_by(|a, b| {
                        a.bounds
                            .size
                            .width
                            .partial_cmp(&b.bounds.size.width)
                            .unwrap()
                    })
                    .expect("a band around the focused box")
            });
            let outset = band.bounds.size.width.0 - edge.bounds.size.width.0;
            (
                edge.border_color,
                band.border_color,
                band.border_widths,
                band.corner_radii,
                outset,
            )
        };
        let input = on_window(cx, w, |window, _| {
            window
                .find_all_by_label("Name")
                .into_iter()
                .map(|e| e.bounds())
                .max_by(|a, b| a.size.width.partial_cmp(&b.size.width).unwrap())
                .unwrap()
        });
        let at = input.center();
        cx.update_window(w, |_, window, cx| {
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let text = ring(cx, input);
        update_hotkey(cx, w, &field, |field, window, cx| {
            field.focus.focus(window, cx)
        });
        let bounds = field_box(cx, w);
        let hotkey = ring(cx, bounds);
        assert_eq!(hotkey, text);
    }

    /// The kit's medium input, which the text fields are, pads its text by the inset's padding.
    #[test]
    fn the_text_inset_is_the_kit_inputs() {
        assert_eq!(
            px(theme::CONTROL_PADDING_X),
            gpui_kit::component::Size::Medium.input_px()
        );
    }

    /// Empty, with a hotkey and recording, the field keeps its size, and what it reads starts at
    /// the text fields' inset.
    #[gpui_kit::test]
    fn a_hotkey_field_keeps_its_box_and_text_start(cx: &mut TestAppContext) {
        let (w, field) = pair(cx);
        let start = |field: Rect, text: Rect| text.left() - field.left();
        let inset = px(theme::CONTROL_TEXT_INSET);
        let size = gpui_kit::size(
            px(theme::settings_control_width()),
            px(theme::CONTROL_HEIGHT),
        );

        let empty = field_box(cx, w);
        assert_eq!(start(empty, find(cx, w, "hotkey-placeholder")), inset);
        update_hotkey(cx, w, &field, |field, window, cx| field.start(window, cx));
        let recording = field_box(cx, w);
        assert_eq!(start(recording, find(cx, w, "hotkey-placeholder")), inset);
        update_hotkey(cx, w, &field, |field, _, cx| field.cancel(cx));
        update_hotkey(cx, w, &field, |field, window, cx| {
            field.set_from_file(Some(hotkey("ctrl+alt+w")), window, cx)
        });
        let set = field_box(cx, w);
        assert_eq!(start(set, find(cx, w, "hotkey-chips")), inset);
        for bounds in [empty, recording, set] {
            assert_eq!(bounds.size, size);
        }
    }

    /// How the hotkey stands is an icon at the right, before the clear button; with none, the
    /// keys still start at the inset.
    #[gpui_kit::test]
    fn the_state_icon_sits_before_the_clear_button(cx: &mut TestAppContext) {
        let (w, field) = pair(cx);
        update_hotkey(cx, w, &field, |field, window, cx| {
            field.set_from_file(Some(hotkey("ctrl+alt+w")), window, cx)
        });
        let bare = (find(cx, w, "hotkey-chips"), find(cx, w, "hotkey-clear"));
        update_hotkey(cx, w, &field, |field, _, cx| {
            field.set_stand(
                Some(HotkeyStand {
                    tone: StandTone::Restart,
                    text: "Applies after a restart".into(),
                }),
                cx,
            )
        });
        let field = field_box(cx, w);
        let (chips, stand, clear) = (
            find(cx, w, "hotkey-chips"),
            find(cx, w, "hotkey-stand"),
            find(cx, w, "hotkey-clear"),
        );
        assert_eq!((chips, clear), bare, "the icon takes no room from the keys");
        assert!(chips.right() < stand.left(), "{chips:?} {stand:?}");
        assert!(stand.right() <= clear.left(), "{stand:?} {clear:?}");
        assert!(clear.right() <= field.right(), "{clear:?} {field:?}");
        assert!(stand.left() > field.center().x, "{stand:?} {field:?}");
    }

    /// A test app that lays text out with the bundled Inter, as the window does.
    #[cfg(target_os = "linux")]
    fn inter_cx() -> TestAppContext {
        let text_system = std::sync::Arc::new(
            gpui_wgpu::CosmicTextSystem::new_without_system_fonts(crate::fonts::UI_FAMILY),
        );
        TestAppContext::build_with_text_system(gpui_kit::TestDispatcher::new(0), None, text_system)
    }

    /// Every combination of up to two modifiers and any key fits the field next to its clear
    /// button, whole and unfaded. Measured with the bundled Inter.
    #[cfg(target_os = "linux")]
    #[test]
    fn common_combinations_fit_the_field() {
        let mut cx = inter_cx();
        let cx = &mut cx;
        let (w, field) = pair(cx);
        let modifiers = [
            "",
            "ctrl+",
            "alt+",
            "shift+",
            "super+",
            "ctrl+alt+",
            "ctrl+shift+",
            "ctrl+super+",
            "alt+shift+",
            "alt+super+",
            "shift+super+",
        ];
        for mods in modifiers {
            for key in tomakast_core::keys::SUPPORTED_KEYS {
                let combination = hotkey(&format!("{mods}{key}"));
                update_hotkey(cx, w, &field, |field, window, cx| {
                    field.set_from_file(Some(combination.clone()), window, cx)
                });
                let (chips, clear) = (find(cx, w, "hotkey-chips"), find(cx, w, "hotkey-clear"));
                let label = combination.label();
                assert!(
                    chips.right() + px(HOTKEY_END_GAP) <= clear.left(),
                    "{label}: {chips:?} {clear:?}"
                );
                let fade = on_window(cx, w, |window, _| window.try_find("hotkey-fade"));
                assert!(fade.is_none(), "{label} is faded");
            }
        }
    }

    /// The fade shows exactly when the keys run past the room the icons leave them (with the
    /// state icon or without), and covers the end of that room.
    #[cfg(target_os = "linux")]
    #[test]
    fn keys_too_long_for_the_field_fade_out() {
        let mut cx = inter_cx();
        let cx = &mut cx;
        let (w, field) = pair(cx);
        let modifiers = [
            "ctrl+alt+shift+",
            "ctrl+alt+super+",
            "ctrl+shift+super+",
            "alt+shift+super+",
            "ctrl+alt+shift+super+",
        ];
        let mut faded = 0;
        for stand in [false, true] {
            update_hotkey(cx, w, &field, |field, _, cx| {
                field.set_stand(
                    stand.then(|| HotkeyStand {
                        tone: StandTone::Problem,
                        text: "Refused".into(),
                    }),
                    cx,
                )
            });
            for mods in modifiers {
                for key in tomakast_core::keys::SUPPORTED_KEYS {
                    let combination = hotkey(&format!("{mods}{key}"));
                    update_hotkey(cx, w, &field, |field, window, cx| {
                        field.set_from_file(Some(combination.clone()), window, cx)
                    });
                    let chips = find(cx, w, "hotkey-chips");
                    let first_icon = if stand {
                        "hotkey-stand"
                    } else {
                        "hotkey-clear"
                    };
                    let room_end = find(cx, w, first_icon).left() - px(HOTKEY_END_GAP);
                    let fade = on_window(cx, w, |window, _| {
                        window.try_find("hotkey-fade").map(|fade| fade.bounds())
                    });
                    let label = combination.label();
                    assert_eq!(
                        fade.is_some(),
                        chips.right() > room_end,
                        "{label} (state icon: {stand}): {chips:?} ends past {room_end:?}?"
                    );
                    if let Some(fade) = fade {
                        faded += 1;
                        assert_eq!(fade.right(), room_end, "{label}");
                        assert_eq!(fade.size.width, px(HOTKEY_FADE_WIDTH), "{label}");
                    }
                }
            }
        }
        assert!(faded > 0, "the widest combinations do not fit");
    }
}
