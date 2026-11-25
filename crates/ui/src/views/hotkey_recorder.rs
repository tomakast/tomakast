//! The hotkey recorder: opened from the action panel's "Assign Hotkey…" over a root row, it
//! shows the combination pressed, says why one cannot be used, and sends the change.

use gpui_kit::{
    AnyElement, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement as _,
    Keystroke, ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::hotkeys::{HotkeyTable, HotkeyTarget, Refusal};
use tomakast_core::keys::unshifted;
use tomakast_core::leader::LeaderRecording;
use tomakast_core::settings::Modifiers as Mods;
use tomakast_core::{Hotkey, ItemId};

use crate::chrome::key_chip;
use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};

/// What a key press in the recorder does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RecorderStep {
    /// The shown combination (or its refusal) changed.
    Shown,
    Save(Hotkey),
    Clear,
    Cancel,
    Ignored,
}

pub(crate) struct HotkeyRecorder {
    pub(crate) item: ItemId,
    title: String,
    /// The row has a hotkey, so Backspace clears it.
    has_hotkey: bool,
    shown: Option<Hotkey>,
    refusal: Option<String>,
    /// Whether the leader is down, so the next key is `Leader+<key>`.
    leader: LeaderRecording,
    pub(crate) focus: FocusHandle,
}

/// What a recorder shows while the leader is down and no key came yet.
pub(crate) const LEADER_DOWN: &str = "Leader + …";

/// The combination of `keystroke`; a shifted symbol counts as Shift and its key (GPUI
/// reports Ctrl+Shift+1 as Ctrl+`!`). `None` for a modifier alone.
pub(crate) fn hotkey_of(keystroke: &Keystroke) -> Option<Hotkey> {
    let mut mods = mods_of(&keystroke.modifiers);
    let key = keystroke.key.to_lowercase();
    if key.is_empty()
        || matches!(
            key.as_str(),
            "shift" | "control" | "alt" | "platform" | "cmd" | "super" | "fn" | "capslock"
        )
    {
        return None;
    }
    let key = match unshifted(&key) {
        Some(base) => {
            mods |= Mods::SHIFT;
            base.to_string()
        }
        None => key,
    };
    Some(Hotkey { mods, key })
}

/// The hotkey modifiers of GPUI's `m` (the platform key is Super).
pub(crate) fn mods_of(m: &gpui_kit::Modifiers) -> Mods {
    let mut mods = Mods::empty();
    for (on, flag) in [
        (m.control, Mods::CTRL),
        (m.alt, Mods::ALT),
        (m.shift, Mods::SHIFT),
        (m.platform, Mods::SUPER),
    ] {
        if on {
            mods |= flag;
        }
    }
    mods
}

/// Why `target` cannot take `hotkey` in the recorders' words (the launcher's and the
/// settings window's); `None`: it can. `title_of` names the item that has it.
pub(crate) fn refusal_text(
    table: &HotkeyTable,
    target: &HotkeyTarget,
    hotkey: &Hotkey,
    title_of: &dyn Fn(&ItemId) -> String,
) -> Option<String> {
    let refusal = table.check_target(target, hotkey).err()?;
    let hotkey = hotkey.label();
    Some(match refusal {
        // Only a leader change clashes, so `hotkey` is the new leader.
        Refusal::LeaderClash {
            leader_form,
            item,
            other,
        } => match table.of(&item) {
            Some(own) => format!(
                "With {hotkey}, {} is {}, already set on \"{}\".",
                own.hotkey.label(),
                leader_form.label(),
                title_of(&other)
            ),
            None => format!(
                "With {hotkey}, {} is set on \"{}\" and \"{}\".",
                leader_form.label(),
                title_of(&item),
                title_of(&other)
            ),
        },
        Refusal::LauncherClash { leader_form, item } => match table.of(&item) {
            Some(own) => format!(
                "With {hotkey}, {} is {}, the launcher hotkey.",
                own.hotkey.label(),
                leader_form.label()
            ),
            None => format!(
                "With {hotkey}, the hotkey of \"{}\" is {}, the launcher hotkey.",
                title_of(&item),
                leader_form.label()
            ),
        },
        other => other.text(&|item| Some(title_of(item))),
    })
}

impl HotkeyRecorder {
    pub(crate) fn new(
        item: ItemId,
        title: String,
        has_hotkey: bool,
        cx: &mut Context<LauncherView>,
    ) -> HotkeyRecorder {
        HotkeyRecorder {
            item,
            title,
            has_hotkey,
            shown: None,
            refusal: None,
            leader: LeaderRecording::default(),
            focus: cx.focus_handle(),
        }
    }

    /// Shows `hotkey` and why it cannot be used, if it cannot.
    pub(crate) fn show(
        &mut self,
        hotkey: Hotkey,
        table: &HotkeyTable,
        title_of: &dyn Fn(&ItemId) -> String,
    ) {
        let target = HotkeyTarget::Item(self.item.clone());
        self.refusal = refusal_text(table, &target, &hotkey, title_of);
        self.shown = Some(hotkey);
    }

    /// A combination pressed (in the recorder, or a tomakast hotkey the system fired), as
    /// the leader makes it: `Leader+<key>` while the leader is down, or when its modifiers
    /// are exactly a modifier leader's in hold mode.
    pub(crate) fn record(
        &mut self,
        pressed: Hotkey,
        table: &HotkeyTable,
        title_of: &dyn Fn(&ItemId) -> String,
    ) {
        let hotkey = self
            .leader
            .key(&pressed, table.leader.as_ref(), table.leader_mode);
        self.show(hotkey, table, title_of);
    }

    /// The leader went down: the next key is recorded as `Leader+<key>`.
    pub(crate) fn leader_pressed(&mut self) {
        self.leader.pressed();
        self.shown = None;
        self.refusal = None;
    }

    pub(crate) fn leader_released(&mut self, table: &HotkeyTable) {
        self.leader.released(table.leader_mode);
    }

    pub(crate) fn press(
        &mut self,
        keystroke: &Keystroke,
        table: &HotkeyTable,
        title_of: &dyn Fn(&ItemId) -> String,
    ) -> RecorderStep {
        let m = &keystroke.modifiers;
        let plain = !(m.control || m.alt || m.shift || m.platform || m.function);
        match keystroke.key.as_str() {
            "escape" if plain => return RecorderStep::Cancel,
            "enter" if plain => {
                return match (&self.shown, &self.refusal) {
                    (Some(hotkey), None) => RecorderStep::Save(hotkey.clone()),
                    _ => RecorderStep::Ignored,
                };
            }
            "backspace" if plain => {
                return if self.has_hotkey {
                    RecorderStep::Clear
                } else {
                    RecorderStep::Ignored
                };
            }
            _ => {}
        }
        match hotkey_of(keystroke) {
            Some(hotkey) => {
                self.record(hotkey, table, title_of);
                RecorderStep::Shown
            }
            None => RecorderStep::Ignored,
        }
    }

    pub(crate) fn render(
        &self,
        unavailable: bool,
        palette: &Palette,
        cx: &mut Context<LauncherView>,
    ) -> AnyElement {
        let shown: AnyElement = match &self.shown {
            Some(hotkey) => div()
                .id("hotkey-shown")
                .aria_label(hotkey.label())
                .test_support()
                .flex()
                .gap_1()
                .children(
                    crate::shortcut::chips(hotkey)
                        .iter()
                        .map(|k| key_chip(k, palette)),
                )
                .into_any_element(),
            None => {
                let text = if self.leader.is_down() {
                    LEADER_DOWN
                } else {
                    "Press a combination…"
                };
                div()
                    .id("hotkey-shown")
                    .aria_label(text)
                    .test_support()
                    .text_color(palette.muted)
                    .child(text)
                    .into_any_element()
            }
        };
        let command = format!("tomakast run {}", self.item.as_str());
        let note = |id: &'static str, text: String| {
            div()
                .id(id)
                .aria_label(text.clone())
                .test_support()
                .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                .text_color(palette.muted)
                .child(text)
        };
        div()
            .id("hotkey-recorder")
            .role(Role::Dialog)
            .aria_label(format!("Hotkey for \"{}\"", self.title))
            .test_support()
            .track_focus(&self.focus)
            .absolute()
            .right(px(theme::POPOVER_MARGIN))
            .bottom(px(theme::FOOTER_HEIGHT + theme::POPOVER_MARGIN))
            .w(px(theme::POPOVER_WIDTH))
            .p(px(theme::ACTION_PADDING_X))
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(theme::ACTION_RADIUS))
            .bg(palette.panel)
            .border(px(theme::PANEL_BORDER))
            .border_color(palette.border)
            .shadow(palette.shadow.to_vec())
            .text_size(theme::text(theme::ROW_TEXT_SIZE))
            .child(div().child(format!("Hotkey for \"{}\"", self.title)))
            .child(shown)
            .children(self.refusal.clone().map(|text| {
                div()
                    .id("hotkey-refusal")
                    .aria_label(text.clone())
                    .test_support()
                    .text_size(theme::text(theme::SMALL_TEXT_SIZE))
                    .child(text)
            }))
            .when(unavailable, |el| {
                el.child(note(
                    "hotkey-unavailable",
                    format!(
                        "Hotkeys are not available here: bind \"{command}\" in your system settings."
                    ),
                ))
                .child(
                    div()
                        .id("hotkey-copy")
                        .role(Role::Button)
                        .aria_label("Copy")
                        .test_support()
                        .cursor_pointer()
                        .child(key_chip("Copy", palette))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(command.clone()))
                        })),
                )
            })
            .child(note(
                "hotkey-hint",
                "Nothing appears? The system or another app holds that combination.".into(),
            ))
            .child(note(
                "hotkey-keys",
                if self.has_hotkey {
                    "↵ Save · ⌫ Clear · Esc Cancel".into()
                } else {
                    "↵ Save · Esc Cancel".into()
                },
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use tomakast_core::hotkeys::{HotkeyState, ItemHotkey};

    use super::*;

    #[test]
    fn a_leader_clash_names_the_leader_the_hotkey_and_the_other_item() {
        let item = |id: &str, hotkey: &str| ItemHotkey {
            item: ItemId::parse(id).unwrap(),
            hotkey: Hotkey::parse(hotkey).unwrap(),
            state: HotkeyState::Active { trigger: None },
        };
        let table = HotkeyTable {
            launcher: Some(Hotkey::parse("alt+space").unwrap()),
            leader: Some(Hotkey::parse_leader("F12").unwrap()),
            items: vec![
                item("apps/firefox", "leader+f"),
                item("apps/kitty", "ctrl+f"),
            ],
            ..HotkeyTable::default()
        };
        let title_of = |id: &ItemId| match id.as_str() {
            "apps/kitty" => "Kitty".to_string(),
            other => other.to_string(),
        };
        assert_eq!(
            refusal_text(
                &table,
                &HotkeyTarget::Leader,
                &Hotkey::parse_leader("Ctrl").unwrap(),
                &title_of,
            )
            .as_deref(),
            Some("With Ctrl, Leader+F is Ctrl+F, already set on \"Kitty\".")
        );
        let table = HotkeyTable {
            launcher: Some(Hotkey::parse("ctrl+space").unwrap()),
            items: vec![item("apps/firefox", "leader+space")],
            ..table
        };
        assert_eq!(
            refusal_text(
                &table,
                &HotkeyTarget::Leader,
                &Hotkey::parse_leader("Ctrl").unwrap(),
                &title_of,
            )
            .as_deref(),
            Some("With Ctrl, Leader+Space is Ctrl+Space, the launcher hotkey.")
        );
    }
}
