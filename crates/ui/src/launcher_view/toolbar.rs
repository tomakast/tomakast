//! A compose pad's toolbar: a plain button runs as a pad action, a button with options
//! opens its menu under it (the action panel, anchored), by a click or by its shortcut.

use gpui_kit::{Context, KeyDownEvent, Pixels, Point, Window, point, px};
use tomakast_core::{Action, View};

use super::LauncherView;
use crate::shortcut;
use crate::stack::StackEntry;
use crate::theme;
use crate::views::ViewState;
use crate::views::action_panel::{ActionPanel, MenuPlace, PanelEntry, PanelTarget};

/// The gap between a button and the menu under it.
const MENU_GAP: f32 = 4.;
/// A menu is never shorter than this: with less room it moves up instead.
const MENU_MIN_HEIGHT: f32 = 160.;

impl LauncherView {
    /// Button `ix` of the toolbar of pushed view `instance` was clicked (or its shortcut
    /// pressed).
    pub(crate) fn toolbar_clicked(
        &mut self,
        instance: u64,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(StackEntry {
            view: View::Scratchpad(pad),
            ..
        }) = self.stack.get(instance)
        else {
            return;
        };
        let Some(button) = pad.toolbar.get(ix).filter(|_| pad.output.is_some()) else {
            return;
        };
        // The click that ends the press which closed this button's menu.
        let this_button = Some((instance, button.id.clone()));
        if self.menu_closed_by.take() == this_button {
            return;
        }
        // The button of the open menu (its shortcut, or a click) closes it.
        if let Some(panel) = &self.panel
            && let PanelTarget::Toolbar {
                instance: open,
                button: open_button,
            } = &panel.target
            && (*open, open_button) == (instance, &button.id)
        {
            self.close_panel(window, cx);
            return;
        }
        if button.options.is_empty() {
            let (row_id, action_id) = (pad.id.0.clone(), button.id.clone());
            self.run_row_action(instance, row_id, action_id, cx);
            return;
        }
        let entries = button
            .options
            .iter()
            .map(|option| PanelEntry::Choice {
                option: option.clone(),
                current: button.selected.as_deref() == Some(option.id.as_str()),
            })
            .collect();
        let target = PanelTarget::Toolbar {
            instance,
            button: button.id.clone(),
        };
        self.panel = Some(ActionPanel::menu(target, entries, window, cx));
        cx.notify();
    }

    /// Where the open toolbar menu hangs this frame: under its button as last painted, no
    /// taller than the room down to the footer. `None` for the Ctrl+K panel, and while the
    /// button has not been painted.
    pub(crate) fn menu_place(&self, window: &Window) -> Option<MenuPlace> {
        let (instance, ix) = self.menu_button()?;
        let StackEntry {
            state: ViewState::Scratchpad(state),
            ..
        } = self.stack.get(instance)?
        else {
            return None;
        };
        let button = state.toolbar_bounds(ix)?;
        let anchor = point(button.left(), button.bottom() + px(MENU_GAP));
        let floor = window.viewport_size().height
            - px(theme::shadow_margin() + theme::FOOTER_HEIGHT + theme::POPOVER_MARGIN);
        let room = (floor - anchor.y).max(px(MENU_MIN_HEIGHT));
        Some(MenuPlace {
            anchor,
            max_height: room.min(px(theme::POPOVER_MAX_HEIGHT)),
        })
    }

    /// The pushed view and the button index of the open toolbar menu.
    fn menu_button(&self) -> Option<(u64, usize)> {
        let PanelTarget::Toolbar { instance, button } = &self.panel.as_ref()?.target else {
            return None;
        };
        let StackEntry {
            view: View::Scratchpad(pad),
            ..
        } = self.stack.get(*instance)?
        else {
            return None;
        };
        let ix = pad.toolbar.iter().position(|b| &b.id == button)?;
        Some((*instance, ix))
    }

    /// A mouse press outside the open toolbar menu closes it. A press on the menu's own
    /// button is remembered, so that its click does not open the menu again.
    pub(crate) fn menu_pressed_outside(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((instance, ix)) = self.menu_button() else {
            return;
        };
        let on_button = match self.stack.get(instance) {
            Some(StackEntry {
                state: ViewState::Scratchpad(state),
                ..
            }) => state
                .toolbar_bounds(ix)
                .is_some_and(|b| b.contains(&position)),
            _ => false,
        };
        self.menu_closed_by = match (&self.panel, on_button) {
            (Some(panel), true) => match &panel.target {
                PanelTarget::Toolbar { instance, button } => Some((*instance, button.clone())),
                _ => None,
            },
            _ => None,
        };
        self.close_panel(window, cx);
    }

    /// The toolbar button a pad action `action_id` on `target` stands for: the button of
    /// the same id in that compose pad's toolbar.
    pub(crate) fn toolbar_button_named(
        &self,
        target: &PanelTarget,
        action_id: &str,
    ) -> Option<(u64, usize)> {
        let PanelTarget::Row { instance, .. } = target else {
            return None;
        };
        let StackEntry {
            view: View::Scratchpad(pad),
            ..
        } = self.stack.get(*instance)?
        else {
            return None;
        };
        pad.output.as_ref()?;
        let ix = pad.toolbar.iter().position(|b| b.id == action_id)?;
        Some((*instance, ix))
    }

    /// The toolbar button of the compose pad on top whose shortcut `event` is.
    pub(crate) fn toolbar_shortcut(&self, event: &KeyDownEvent) -> Option<(u64, usize)> {
        let StackEntry {
            instance,
            view: View::Scratchpad(pad),
            ..
        } = self.stack.top()?
        else {
            return None;
        };
        pad.output.as_ref()?;
        let as_actions: Vec<Action> = pad
            .toolbar
            .iter()
            .map(|b| Action {
                id: b.id.clone(),
                title: b.label.clone(),
                shortcut: b.shortcut.clone(),
            })
            .collect();
        let found = shortcut::find(&as_actions, &event.keystroke)?;
        let ix = pad.toolbar.iter().position(|b| b.id == found.id)?;
        Some((*instance, ix))
    }
}
