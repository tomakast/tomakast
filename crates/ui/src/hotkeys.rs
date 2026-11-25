//! Item hotkeys in the launcher: the daemon's table shown on root rows, the panel entry that
//! opens the recorder, and the requests the recorder sends back.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
    prelude::FluentBuilder as _,
};
use tomakast_core::Hotkey;
use tomakast_core::hotkeys::{HotkeyOrigin, HotkeyState, HotkeyTable, HotkeyTarget, ItemHotkey};

use crate::chrome::key_chip;
use crate::theme::{self, Palette};

/// Where a recorder sends a change: `Some` to assign, `None` to clear an item's hotkey or
/// reset the launcher's to the default. The answer comes back as a new table
/// ([`crate::LauncherController::set_hotkey_table`]) and, when the change did not go
/// through, a notice for `origin`.
pub trait HotkeyRequests {
    fn assign(&self, target: HotkeyTarget, hotkey: Option<Hotkey>, origin: HotkeyOrigin);
}

/// What the controller and its windows share about hotkeys.
#[derive(Default)]
pub(crate) struct HotkeyUi {
    /// `None`: no hotkey support in this run, so no panel entry.
    pub(crate) requests: Option<Rc<dyn HotkeyRequests>>,
    pub(crate) table: HotkeyTable,
    /// Notices that arrived while the window was closed, shown at the next open.
    pub(crate) notices: Vec<String>,
}

pub(crate) type SharedHotkeys = Rc<RefCell<HotkeyUi>>;

/// The panel entry's action id. Module ids cannot hold control characters
/// ([`tomakast_core::is_valid_id`]), so no module action can be this one.
pub(crate) const ASSIGN_HOTKEY: &str = "\u{1}assign-hotkey";

/// The panel entry's title: assign, or change when the item has a hotkey.
pub(crate) fn entry_title(has_hotkey: bool) -> &'static str {
    if has_hotkey {
        "Change Hotkey…"
    } else {
        "Assign Hotkey…"
    }
}

/// The accessible label of a row's hotkey, with its state.
pub(crate) fn label(hotkey: &ItemHotkey) -> String {
    let shown = match &hotkey.state {
        HotkeyState::Active {
            trigger: Some(trigger),
        } => trigger.clone(),
        _ => hotkey.hotkey.label(),
    };
    let suffix = match &hotkey.state {
        HotkeyState::Active { .. } => String::new(),
        HotkeyState::AfterRestart => " (after restart)".into(),
        HotkeyState::Refused { reason } => format!(" (not set: {reason})"),
        HotkeyState::Off => " (off)".into(),
        HotkeyState::NoLeader => " (off: no leader key set)".into(),
    };
    format!("Hotkey {shown}{suffix}")
}

/// A root row's hotkey: key chips, dimmed with a word when it is not working.
pub(crate) fn chips(ix: usize, hotkey: &ItemHotkey, palette: &Palette) -> AnyElement {
    let note = match &hotkey.state {
        HotkeyState::Active { .. } => None,
        HotkeyState::AfterRestart => Some("after restart"),
        HotkeyState::Refused { .. } => Some("not set"),
        HotkeyState::Off => Some("off"),
        HotkeyState::NoLeader => Some("no leader key set"),
    };
    let keys: Vec<String> = match &hotkey.state {
        HotkeyState::Active {
            trigger: Some(trigger),
        } => vec![trigger.clone()],
        _ => crate::shortcut::chips(&hotkey.hotkey),
    };
    div()
        .id(("row-hotkey", ix))
        .aria_label(label(hotkey))
        .test_support()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .when_some(note, |el, _| el.opacity(0.55))
        .children(keys.iter().map(|k| key_chip(k, palette)))
        .children(note.map(|note| {
            div()
                .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                .text_color(palette.muted)
                .child(note)
        }))
        .into_any_element()
}
