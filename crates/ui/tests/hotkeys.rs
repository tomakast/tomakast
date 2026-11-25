//! Headless tests of item hotkeys in the launcher: the panel entry, the row chips and the
//! recorder. Set up like `action_panel.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, BorrowAppContext as _, QuitMode, Role, TestAppContext,
};
use tomakast_core::hotkeys::{HotkeyOrigin, HotkeyState, HotkeyTable, HotkeyTarget, ItemHotkey};
use tomakast_core::index::Action;
use tomakast_core::store::Store;
use tomakast_core::testing::{FakeModule, wait_event};
use tomakast_core::{
    Core, Effect, Hotkey, Item, ItemId, ItemKind, ListRow, ListView, LocalNow, RankWeights,
    SearchMode, View, ViewId,
};
use tomakast_ui::{HotkeyRequests, LauncherController};

const TOGGLE: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};

type Requests = Rc<RefCell<Vec<(ItemId, Option<Hotkey>)>>>;

struct FakeRequests(Requests);

impl HotkeyRequests for FakeRequests {
    fn assign(&self, target: HotkeyTarget, hotkey: Option<Hotkey>, origin: HotkeyOrigin) {
        let HotkeyTarget::Item(item) = target else {
            panic!("the launcher's recorder sets item hotkeys");
        };
        assert_eq!(origin, HotkeyOrigin::Launcher);
        self.0.borrow_mut().push((item, hotkey));
    }
}

fn hk(s: &str) -> Hotkey {
    Hotkey::parse(s).unwrap()
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

fn item(local: &str, title: &str) -> Item {
    Item {
        id: ItemId::new("fake", local),
        title: title.to_string(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Application,
        source: None,
        actions: vec![action("open", "Open"), action("reveal", "Reveal in Files")],
    }
}

/// With an empty query: Files, Firefox, Terminal.
fn items() -> Vec<Item> {
    vec![
        item("firefox", "Firefox"),
        item("files", "Files"),
        item("terminal", "Terminal"),
    ]
}

fn push_sub() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::PushView(View::List(ListView {
        id: ViewId("fake/sub".into()),
        title: "Sub".into(),
        placeholder: "Filter…".into(),
        search: SearchMode::CoreFilter,
        query: String::new(),
        revision: 0,
        rows: vec![ListRow {
            id: "alpha".into(),
            section: None,
            title: "Alpha".into(),
            subtitle: None,
            icon: None,
            accessories: Vec::new(),
            detail_markdown: None,
            actions: vec![action("pick", "Pick")],
        }],
        show_detail: false,
        layout: tomakast_core::ListLayout::Rows,
        footer_action: None,
        busy: false,
    }))])
}

fn new_core() -> Core {
    Core::new(
        Store::open_in_memory().unwrap(),
        RankWeights::default(),
        LocalNow::now(),
    )
    .unwrap()
}

/// The launcher open over `module`'s items; `requests` (when given) are hotkey support.
fn setup(
    cx: &mut TestAppContext,
    module: FakeModule,
    requests: Option<Requests>,
) -> Rc<RefCell<Core>> {
    let mut core = new_core();
    core.register(Box::new(module));
    core.start_modules();
    let ev = wait_event(&core.events());
    core.apply(ev, LocalNow::now());
    let core = Rc::new(RefCell::new(core));
    cx.update(|cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        tomakast_ui::init(cx);
        let mut controller = LauncherController::new(core.clone());
        if let Some(requests) = requests {
            controller.set_hotkey_requests(Rc::new(FakeRequests(requests)));
        }
        cx.set_global(controller);
    });
    show(cx);
    core
}

fn show(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.show(cx)));
    cx.run_until_parked();
}

fn hide(cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_global::<LauncherController, _>(|c, cx| c.hide(cx)));
    cx.run_until_parked();
}

fn deliver(cx: &mut TestAppContext, core: &Rc<RefCell<Core>>) {
    let ev = wait_event(&core.borrow().events());
    let update = core.borrow_mut().apply(ev, LocalNow::now());
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

fn panel_titles(cx: &mut TestAppContext) -> Vec<String> {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::MenuItem)
            .iter()
            .filter_map(|e| e.label().map(str::to_string))
            .collect()
    })
    .unwrap()
}

fn has_label(cx: &mut TestAppContext, label: &str) -> bool {
    let w = window(cx);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.try_find_by_label(label).is_some()
    })
    .unwrap()
}

fn set_table(cx: &mut TestAppContext, table: HotkeyTable) {
    cx.update(|cx| {
        cx.update_global::<LauncherController, _>(|c, cx| c.set_hotkey_table(table, cx))
    });
    cx.run_until_parked();
}

fn table_with(local: &str, hotkey: &str, state: HotkeyState) -> HotkeyTable {
    HotkeyTable {
        launcher: Some(hk("alt+space")),
        items: vec![ItemHotkey {
            item: ItemId::new("fake", local),
            hotkey: hk(hotkey),
            state,
        }],
        ..HotkeyTable::default()
    }
}

fn nothing(_: &ViewId, _: tomakast_core::ViewEvent) -> anyhow::Result<Vec<Effect>> {
    Ok(vec![])
}

fn closes() -> anyhow::Result<Vec<Effect>> {
    Ok(vec![Effect::CloseWindow])
}

#[gpui_kit::test]
fn no_entry_without_hotkey_support(cx: &mut TestAppContext) {
    setup(cx, FakeModule::new(items(), closes), None);
    press(cx, TOGGLE);
    assert_eq!(panel_titles(cx), ["Open", "Reveal in Files"]);
}

#[gpui_kit::test]
fn entry_is_last_for_an_indexed_root_item(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    press(cx, TOGGLE);
    assert_eq!(
        panel_titles(cx),
        ["Open", "Reveal in Files", "Assign Hotkey…"]
    );
}

#[gpui_kit::test]
fn entry_says_change_when_the_item_has_one(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    // Files is the first row of the empty query.
    set_table(
        cx,
        table_with("files", "ctrl+alt+f", HotkeyState::Active { trigger: None }),
    );
    press(cx, TOGGLE);
    assert_eq!(
        panel_titles(cx),
        ["Open", "Reveal in Files", "Change Hotkey…"]
    );
}

#[gpui_kit::test]
fn row_shows_its_hotkey(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    set_table(
        cx,
        table_with("files", "ctrl+alt+f", HotkeyState::Active { trigger: None }),
    );
    assert!(has_label(cx, "Hotkey Ctrl+Alt+F"));
}

#[gpui_kit::test]
fn row_shows_the_state(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    set_table(
        cx,
        table_with("files", "ctrl+alt+f", HotkeyState::AfterRestart),
    );
    assert!(has_label(cx, "Hotkey Ctrl+Alt+F (after restart)"));
    set_table(cx, table_with("files", "ctrl+alt+f", HotkeyState::Off));
    assert!(has_label(cx, "Hotkey Ctrl+Alt+F (off)"));
    set_table(
        cx,
        table_with(
            "files",
            "ctrl+alt+f",
            HotkeyState::Refused {
                reason: "used by kitty: Launch".into(),
            },
        ),
    );
    assert!(has_label(
        cx,
        "Hotkey Ctrl+Alt+F (not set: used by kitty: Launch)"
    ));
}

#[gpui_kit::test]
fn no_entry_for_rows_of_a_pushed_view(cx: &mut TestAppContext) {
    let core = setup(
        cx,
        FakeModule::new(items(), push_sub).with_view_events(nothing),
        Some(Requests::default()),
    );
    press(cx, "enter");
    deliver(cx, &core);
    press(cx, TOGGLE);
    assert_eq!(panel_titles(cx), ["Pick"]);
}

#[gpui_kit::test]
fn notice_while_hidden_shows_at_next_open(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    hide(cx);
    cx.update(|cx| {
        cx.update_global::<LauncherController, _>(|c, cx| {
            c.hotkey_notice(
                "Ctrl+Alt+T for Kitty: used by kitty: Launch; nothing changed".into(),
                cx,
            )
        })
    });
    show(cx);
    assert!(has_label(
        cx,
        "Ctrl+Alt+T for Kitty: used by kitty: Launch; nothing changed"
    ));
}

/// Ctrl+K on the first row (Files), then the hotkey entry, which is last.
fn open_recorder(cx: &mut TestAppContext) {
    press(cx, TOGGLE);
    let entries = panel_titles(cx).len();
    for _ in 1..entries {
        press(cx, "down");
    }
    press(cx, "enter");
}

fn recording(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<LauncherController>().is_recording(cx))
}

#[gpui_kit::test]
fn recorder_shows_the_pressed_combination(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    open_recorder(cx);
    assert!(recording(cx));
    assert!(has_label(cx, "Press a combination…"));
    press(cx, "ctrl-alt-t");
    assert!(has_label(cx, "Ctrl+Alt+T"));
}

#[gpui_kit::test]
fn recorder_captures_launcher_keys(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    open_recorder(cx);
    press(cx, TOGGLE);
    assert!(recording(cx), "Ctrl+K is recorded, not the panel key");
    let expected = if cfg!(target_os = "macos") {
        "Super+K"
    } else {
        "Ctrl+K"
    };
    assert!(has_label(cx, expected));
}

#[gpui_kit::test]
fn recorder_saves_on_enter(cx: &mut TestAppContext) {
    let requests = Requests::default();
    setup(cx, FakeModule::new(items(), closes), Some(requests.clone()));
    open_recorder(cx);
    press(cx, "ctrl-alt-t");
    press(cx, "enter");
    assert_eq!(
        *requests.borrow(),
        [(ItemId::new("fake", "files"), Some(hk("ctrl+alt+t")))]
    );
    assert!(!recording(cx));
}

#[gpui_kit::test]
fn recorder_refuses_a_taken_combination(cx: &mut TestAppContext) {
    let requests = Requests::default();
    setup(cx, FakeModule::new(items(), closes), Some(requests.clone()));
    set_table(
        cx,
        table_with(
            "firefox",
            "ctrl+alt+t",
            HotkeyState::Active { trigger: None },
        ),
    );
    open_recorder(cx);
    press(cx, "ctrl-alt-t");
    press(cx, "enter");
    assert!(requests.borrow().is_empty());
    assert!(recording(cx));
    assert!(has_label(
        cx,
        "Already set on \"Firefox\". Clear it there first."
    ));
}

#[gpui_kit::test]
fn recorder_refuses_without_modifier(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    open_recorder(cx);
    press(cx, "t");
    assert!(has_label(cx, "Add Ctrl, Alt or Super."));
}

#[gpui_kit::test]
fn recorder_records_shifted_digit_as_shift_and_digit(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    open_recorder(cx);
    // How GPUI reports Ctrl+Shift+1 on a US layout.
    press(cx, "ctrl-!");
    assert!(has_label(cx, "Ctrl+Shift+1"));
}

#[gpui_kit::test]
fn recorder_refuses_a_non_latin_key(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    open_recorder(cx);
    press(cx, "ctrl-е");
    // Named as its chip names it.
    assert!(has_label(cx, "Е cannot be a hotkey."));
}

#[gpui_kit::test]
fn backspace_clears(cx: &mut TestAppContext) {
    let requests = Requests::default();
    setup(cx, FakeModule::new(items(), closes), Some(requests.clone()));
    set_table(
        cx,
        table_with("files", "ctrl+alt+f", HotkeyState::Active { trigger: None }),
    );
    open_recorder(cx);
    press(cx, "backspace");
    assert_eq!(*requests.borrow(), [(ItemId::new("fake", "files"), None)]);
    assert!(!recording(cx));
}

#[gpui_kit::test]
fn escape_returns_to_the_panel(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    open_recorder(cx);
    press(cx, "escape");
    assert!(!recording(cx));
    assert_eq!(
        panel_titles(cx),
        ["Open", "Reveal in Files", "Assign Hotkey…"]
    );
}

#[gpui_kit::test]
fn a_fired_hotkey_is_recorded(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    set_table(
        cx,
        table_with(
            "firefox",
            "ctrl+alt+t",
            HotkeyState::Active { trigger: None },
        ),
    );
    open_recorder(cx);
    cx.update(|cx| {
        cx.update_global::<LauncherController, _>(|c, cx| c.recorded_press(hk("ctrl+alt+t"), cx))
    });
    cx.run_until_parked();
    assert!(has_label(cx, "Ctrl+Alt+T"));
    assert!(has_label(
        cx,
        "Already set on \"Firefox\". Clear it there first."
    ));
}

#[gpui_kit::test]
fn recorder_explains_unavailable_hotkeys(cx: &mut TestAppContext) {
    setup(
        cx,
        FakeModule::new(items(), closes),
        Some(Requests::default()),
    );
    set_table(
        cx,
        HotkeyTable {
            unavailable: Some("TOMAKAST_NO_KEYBOARD=1".into()),
            ..HotkeyTable::default()
        },
    );
    open_recorder(cx);
    assert!(has_label(
        cx,
        "Hotkeys are not available here: bind \"tomakast run fake/files\" in your system settings."
    ));
    assert!(has_label(cx, "Copy"));
}
