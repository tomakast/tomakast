//! Headless tests of the form view.
//!
//! As in `stack.rs`, module calls run on real worker threads and the test
//! thread hands each reply to the controller, as the daemon does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, TestAppContext};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, CoreUpdate, Effect, FieldKind, FieldValue, FormField, FormView, Item, ItemId, ItemKind,
    LocalNow, RankWeights, View, ViewEvent, ViewId,
};
use tomakast_ui::LauncherController;

const SUBMIT: &str = if cfg!(target_os = "macos") {
    "cmd-enter"
} else {
    "ctrl-enter"
};

fn items() -> Vec<Item> {
    vec![Item {
        id: ItemId::new("fake", "form"),
        title: "Form".into(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Command,
        actions: vec![Action {
            id: "run".into(),
            title: "Run".into(),
        }],
    }]
}

fn field(id: &str, label: &str, kind: FieldKind) -> FormField {
    FormField {
        id: id.into(),
        label: label.into(),
        kind,
    }
}

fn dropdown() -> FieldKind {
    FieldKind::Dropdown {
        options: vec![("a".into(), "Alpha".into()), ("b".into(), "Beta".into())],
        value: "a".into(),
    }
}

fn form(fields: Vec<FormField>) -> FormView {
    FormView {
        id: ViewId("fake/form".into()),
        title: "Form".into(),
        fields,
        submit_title: "Save".into(),
    }
}

fn push_mixed() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Form(form(vec![
        field(
            "t",
            "Text",
            FieldKind::Text {
                value: String::new(),
                placeholder: "type".into(),
            },
        ),
        field("c", "Check", FieldKind::Checkbox { value: false }),
        field("d", "Drop", dropdown()),
    ])))])
}

fn push_date() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Form(form(vec![field(
        "when",
        "When",
        FieldKind::Date { value: None },
    )])))])
}

fn push_text() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Form(form(vec![
        field(
            "t",
            "Text",
            FieldKind::Text {
                value: String::new(),
                placeholder: String::new(),
            },
        ),
        field("c", "Check", FieldKind::Checkbox { value: false }),
    ])))])
}

/// Answers a submit with a view that renames the field and tries to replace
/// the typed text and the checkbox.
fn rename_on_submit(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::UpdateView(View::Form(form(vec![
        field(
            "t",
            "Renamed",
            FieldKind::Text {
                value: "from the module".into(),
                placeholder: String::new(),
            },
        ),
        field("c", "Check", FieldKind::Checkbox { value: true }),
    ])))])
}

/// Like `rename_on_submit`, but the module turns the checkbox off.
fn unchecked_on_submit(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::UpdateView(View::Form(form(vec![
        field(
            "t",
            "Renamed",
            FieldKind::Text {
                value: "from the module".into(),
                placeholder: String::new(),
            },
        ),
        field("c", "Check", FieldKind::Checkbox { value: false }),
    ])))])
}

fn push_multiline() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Form(form(vec![
        field(
            "m",
            "Notes",
            FieldKind::Multiline {
                value: String::new(),
            },
        ),
        field(
            "t",
            "Text",
            FieldKind::Text {
                value: String::new(),
                placeholder: String::new(),
            },
        ),
    ])))])
}

fn push_empty_form() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Form(form(vec![])))])
}

fn nothing(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

fn setup(cx: &mut TestAppContext, module: FakeModule) -> Rc<RefCell<Core>> {
    let mut core = Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap();
    core.register(Box::new(module));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    let core = Rc::new(RefCell::new(core));
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        cx.set_global(LauncherController::new(core.clone()));
    });
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show(cx)));
    cx.run_until_parked();
    core
}

fn deliver(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let ev = wait_event(&core.borrow().events());
    let update: CoreUpdate = core.borrow_mut().apply(ev, LocalNow::now());
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.core_update(update, cx)));
    cx.run_until_parked();
}

fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "expected exactly the launcher window");
    windows[0]
}

fn press(cx: &mut TestAppContext, key: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn type_text(cx: &mut TestAppContext, text: &str) {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn label_of(cx: &mut TestAppContext, id: (&'static str, usize)) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .map(|e| e.label().unwrap_or_default().to_string())
    })
    .unwrap()
}

fn view_label(cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .map(|e| e.label().unwrap_or_default().to_string())
    })
    .unwrap()
}

fn value_of(cx: &mut TestAppContext, id: (&'static str, usize)) -> Option<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .map(|e| e.value().unwrap_or_default().to_string())
    })
    .unwrap()
}

fn focused(cx: &mut TestAppContext, id: (&'static str, usize)) -> bool {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused().unwrap_or(false)
    })
    .unwrap()
}

type Events = Arc<Mutex<Vec<(ViewId, ViewEvent)>>>;

fn pushed(cx: &mut TestAppContext, module: FakeModule) -> (Rc<RefCell<Core>>, Events) {
    let events = module.view_events.clone();
    let core = setup(cx, module);
    press(cx, "enter");
    deliver(cx, &core);
    (core, events)
}

#[gpui_kit::test]
fn form_submits_values_in_order(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_mixed).with_view_events(nothing),
    );

    // The first field has the focus.
    type_text(cx, "a");
    press(cx, "tab");
    press(cx, "space");
    press(cx, "tab");
    // Open the dropdown, move to "Beta" and choose it.
    press(cx, "enter");
    press(cx, "down");
    press(cx, "enter");
    press(cx, SUBMIT);
    deliver(cx, &core);

    assert_eq!(
        *events.lock().unwrap(),
        [(
            ViewId("fake/form".into()),
            ViewEvent::FormSubmitted {
                values: vec![
                    ("t".into(), FieldValue::Text("a".into())),
                    ("c".into(), FieldValue::Bool(true)),
                    ("d".into(), FieldValue::Text("b".into())),
                ]
            }
        )]
    );
}

#[gpui_kit::test]
fn invalid_date_blocks_submit(cx: &mut TestAppContext) {
    let (_core, events) = pushed(
        cx,
        FakeModule::new(items(), push_date).with_view_events(nothing),
    );
    assert_eq!(label_of(cx, ("form-error", 0)), None);

    type_text(cx, "2026-13-40");
    press(cx, SUBMIT);
    std::thread::sleep(Duration::from_millis(100));

    assert!(events.lock().unwrap().is_empty(), "submit must be blocked");
    assert_eq!(
        label_of(cx, ("form-error", 0)).as_deref(),
        Some("Enter a date as YYYY-MM-DD")
    );
}

#[gpui_kit::test]
fn valid_and_empty_dates_submit(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_date).with_view_events(nothing),
    );
    type_text(cx, "2028-02-29");
    press(cx, SUBMIT);
    deliver(cx, &core);
    assert_eq!(
        events.lock().unwrap().last().map(|(_, e)| e.clone()),
        Some(ViewEvent::FormSubmitted {
            values: vec![("when".into(), FieldValue::Date(Some("2028-02-29".into())))]
        })
    );
}

#[gpui_kit::test]
fn update_view_keeps_typed_text(cx: &mut TestAppContext) {
    let (core, _events) = pushed(
        cx,
        FakeModule::new(items(), push_text).with_view_events(rename_on_submit),
    );
    type_text(cx, "mine");
    press(cx, SUBMIT);
    deliver(cx, &core);

    // The label is the module's; the typed text and the toggled box are not.
    assert_eq!(label_of(cx, ("form-label", 0)).as_deref(), Some("Renamed"));
    assert_eq!(value_of(cx, ("form-field", 0)).as_deref(), Some("mine"));
}

#[gpui_kit::test]
fn tab_moves_focus_between_fields(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_mixed).with_view_events(nothing),
    );
    assert!(focused(cx, ("form-field", 0)));
    press(cx, "tab");
    assert!(focused(cx, ("form-field", 1)));
    press(cx, "tab");
    assert!(focused(cx, ("form-field", 2)));
    press(cx, "shift-tab");
    assert!(focused(cx, ("form-field", 1)));
    press(cx, "shift-tab");
    assert!(focused(cx, ("form-field", 0)));
}

#[gpui_kit::test]
fn update_view_keeps_a_toggled_checkbox(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_text).with_view_events(unchecked_on_submit),
    );
    type_text(cx, "mine");
    press(cx, "tab");
    press(cx, "space");
    press(cx, SUBMIT);
    deliver(cx, &core);
    // The module's update turned the box off; the user's choice stands.
    press(cx, SUBMIT);
    deliver(cx, &core);

    assert_eq!(
        events.lock().unwrap().last().map(|(_, e)| e.clone()),
        Some(ViewEvent::FormSubmitted {
            values: vec![
                ("t".into(), FieldValue::Text("mine".into())),
                ("c".into(), FieldValue::Bool(true)),
            ]
        })
    );
}

#[gpui_kit::test]
fn update_view_replaces_untouched_values(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_text).with_view_events(rename_on_submit),
    );
    press(cx, SUBMIT);
    deliver(cx, &core);
    press(cx, SUBMIT);
    deliver(cx, &core);

    assert_eq!(
        events.lock().unwrap().last().map(|(_, e)| e.clone()),
        Some(ViewEvent::FormSubmitted {
            values: vec![
                ("t".into(), FieldValue::Text("from the module".into())),
                ("c".into(), FieldValue::Bool(true)),
            ]
        })
    );
}

#[gpui_kit::test]
fn tab_leaves_a_multiline_field(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_multiline).with_view_events(nothing),
    );
    type_text(cx, "x");
    press(cx, "tab");
    assert!(focused(cx, ("form-field", 1)));
    type_text(cx, "y");
    assert_eq!(value_of(cx, ("form-field", 1)).as_deref(), Some("y"));
}

fn push_drop_then_check() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::Form(form(vec![
        field("d", "Drop", dropdown()),
        field("c", "Check", FieldKind::Checkbox { value: false }),
    ])))])
}

fn last_submit(events: &Events) -> Option<ViewEvent> {
    events.lock().unwrap().last().map(|(_, e)| e.clone())
}

#[gpui_kit::test]
fn open_dropdown_does_not_hijack_keys_after_tab(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_drop_then_check).with_view_events(nothing),
    );
    press(cx, "enter"); // opens the dropdown
    assert!(label_of(cx, ("form-options", 0)).is_some());
    press(cx, "tab");
    // The list closed with the focus; Enter toggles the checkbox, Down is inert.
    assert!(label_of(cx, ("form-options", 0)).is_none());
    press(cx, "down");
    press(cx, "enter");
    press(cx, SUBMIT);
    deliver(cx, &core);
    assert_eq!(
        last_submit(&events),
        Some(ViewEvent::FormSubmitted {
            values: vec![
                ("d".into(), FieldValue::Text("a".into())),
                ("c".into(), FieldValue::Bool(true)),
            ]
        })
    );
}

#[gpui_kit::test]
fn escape_closes_the_dropdown_before_popping(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_drop_then_check).with_view_events(nothing),
    );
    press(cx, "enter");
    assert!(label_of(cx, ("form-options", 0)).is_some());
    press(cx, "escape");
    assert!(label_of(cx, ("form-options", 0)).is_none());
    assert!(label_of(cx, ("form-field", 0)).is_some(), "form still open");
    press(cx, "escape");
    assert!(label_of(cx, ("form-field", 0)).is_none(), "form popped");
}

#[gpui_kit::test]
fn empty_form_does_not_trap_the_keyboard(cx: &mut TestAppContext) {
    pushed(
        cx,
        FakeModule::new(items(), push_empty_form).with_view_events(nothing),
    );
    assert!(view_label(cx, "form-view").is_some(), "form is open");
    press(cx, "escape");
    assert!(view_label(cx, "form-view").is_none(), "form popped");
}

fn two_options_to_one(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::UpdateView(View::Form(form(vec![
        field(
            "d",
            "Drop",
            FieldKind::Dropdown {
                options: vec![("a".into(), "Alpha".into()), ("c".into(), "Gamma".into())],
                value: "a".into(),
            },
        ),
        field("c", "Check", FieldKind::Checkbox { value: false }),
    ])))])
}

#[gpui_kit::test]
fn update_view_changes_options_and_keeps_a_chosen_value(cx: &mut TestAppContext) {
    let (core, events) = pushed(
        cx,
        FakeModule::new(items(), push_drop_then_check).with_view_events(two_options_to_one),
    );
    // Choose "Beta" (b), submit; the module then replaces the options.
    press(cx, "enter");
    press(cx, "down");
    press(cx, "enter");
    press(cx, SUBMIT);
    deliver(cx, &core);
    // "b" is no longer an option but the user's choice stands.
    press(cx, SUBMIT);
    deliver(cx, &core);
    assert_eq!(
        last_submit(&events),
        Some(ViewEvent::FormSubmitted {
            values: vec![
                ("d".into(), FieldValue::Text("b".into())),
                ("c".into(), FieldValue::Bool(false)),
            ]
        })
    );
    // The new options are shown.
    press(cx, "enter");
    assert_eq!(label_of(cx, ("form-option", 1)).as_deref(), Some("Gamma"));
}

fn changes_fields(_: &ViewId, _: ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::UpdateView(View::Form(form(vec![field(
        "other",
        "Other",
        FieldKind::Checkbox { value: false },
    )])))])
}

#[gpui_kit::test]
fn update_view_with_other_fields_is_ignored(cx: &mut TestAppContext) {
    let (core, _events) = pushed(
        cx,
        FakeModule::new(items(), push_text).with_view_events(changes_fields),
    );
    press(cx, SUBMIT);
    deliver(cx, &core);
    assert_eq!(label_of(cx, ("form-label", 0)).as_deref(), Some("Text"));
    assert_eq!(label_of(cx, ("form-label", 1)).as_deref(), Some("Check"));
}
