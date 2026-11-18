//! The action panel: Ctrl+K (Cmd+K on macOS) over the current view lists the
//! selected row's actions, with a filter field of its own.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::{Action, ItemId, Matcher, Pattern};

use crate::chrome::key_chip;
use crate::launcher_view::LauncherView;
use crate::theme::{self, Palette};

/// Dev-only: opens the panel whenever a row with actions is selected (see
/// the crate docs).
const DEV_ACTIONS_ENV: &str = "TOMAKAST_DEV_ACTIONS";

/// Whether `TOMAKAST_DEV_ACTIONS` is set.
pub(crate) fn dev_actions() -> bool {
    crate::dev_flag(DEV_ACTIONS_ENV)
}

/// The key hint shown on a row of the panel. Actions carry no shortcuts of
/// their own; Enter runs the selected action, so only the selected row shows it.
fn action_shortcut(selected: bool) -> Option<&'static str> {
    selected.then_some("↵")
}

/// What the panel's actions act on.
#[derive(Clone)]
pub(crate) enum PanelTarget {
    /// An item of the root list.
    Root(ItemId),
    /// A row of pushed view `instance`.
    Row { instance: u64, row_id: String },
}

pub(crate) struct ActionPanel {
    pub target: PanelTarget,
    /// The actions of the row when the panel opened.
    actions: Vec<Action>,
    input: Entity<InputState>,
    /// Indices into `actions` that pass the filter.
    visible: Vec<usize>,
    /// Index into `visible`.
    selected: usize,
    matcher: Matcher,
    _input_events: Subscription,
}

impl ActionPanel {
    /// A panel over `actions`, with the focus on its filter field.
    pub(crate) fn new(
        target: PanelTarget,
        actions: Vec<Action>,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter actions…"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let input_events = cx.subscribe(&input, |host, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                host.action_filter_changed(cx);
            }
        });
        let visible = (0..actions.len()).collect();
        ActionPanel {
            target,
            actions,
            input,
            visible,
            selected: 0,
            matcher: Matcher::new(),
            _input_events: input_events,
        }
    }

    pub(crate) fn query(&self, cx: &Context<LauncherView>) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Keeps the actions whose title matches `query`; the first is selected.
    pub(crate) fn refilter(&mut self, query: &str) {
        let pattern = Pattern::parse(query);
        let matcher = &mut self.matcher;
        self.visible = self
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| matcher.score_text(&pattern, &action.title).is_some())
            .map(|(ix, _)| ix)
            .collect();
        self.selected = 0;
    }

    pub(crate) fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn select_next(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    pub(crate) fn selected_action(&self) -> Option<&Action> {
        self.actions.get(*self.visible.get(self.selected)?)
    }

    /// The popover, floating above the bottom right corner of the launcher:
    /// the actions, then the filter field under a divider.
    pub(crate) fn render(&self, palette: &Palette) -> AnyElement {
        let actions = self.visible.iter().enumerate().filter_map(|(ix, &action)| {
            let shortcut = action_shortcut(ix == self.selected);
            let action = self.actions.get(action)?;
            Some(
                div()
                    .id(("action", ix))
                    .role(Role::MenuItem)
                    .aria_label(action.title.clone())
                    .test_support()
                    .h(px(theme::ACTION_HEIGHT))
                    .px(px(theme::ACTION_PADDING_X))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .flex_none()
                    .rounded(px(theme::ACTION_RADIUS))
                    .text_size(px(theme::ROW_TEXT_SIZE))
                    .when(ix == self.selected, |row| row.bg(palette.selected))
                    .child(div().min_w_0().truncate().child(action.title.clone()))
                    .children(shortcut.map(|label| key_chip(label, palette))),
            )
        });
        div()
            .id("action-panel")
            .role(Role::Menu)
            .aria_label("Actions")
            .test_support()
            .absolute()
            .right(px(theme::POPOVER_MARGIN))
            .bottom(px(theme::FOOTER_HEIGHT + theme::POPOVER_MARGIN))
            .w(px(theme::POPOVER_WIDTH))
            .max_h(px(theme::POPOVER_MAX_HEIGHT))
            .flex()
            .flex_col()
            .rounded(px(theme::POPOVER_RADIUS))
            .overflow_hidden()
            .bg(palette.popover)
            .border_1()
            .border_color(palette.border)
            .shadow(palette.shadow.to_vec())
            .child(
                div()
                    .id("action-list")
                    .flex_1()
                    .min_h_0()
                    .p(px(theme::POPOVER_PADDING))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .children(actions),
            )
            .child(div().h(px(theme::DIVIDER_THICKNESS)).bg(palette.divider))
            .child(
                div()
                    .flex_none()
                    .h(px(theme::POPOVER_INPUT_HEIGHT))
                    .px(px(theme::POPOVER_INPUT_PADDING_X))
                    .flex()
                    .items_center()
                    .child(
                        Input::new(&self.input)
                            .id("action-input")
                            .aria_label("Filter actions")
                            .appearance(false)
                            .focus_bordered(false)
                            // Without the kit's inner padding the placeholder
                            // lines up with the action titles.
                            .px_0(),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_selected_action_shows_enter() {
        assert_eq!(action_shortcut(true), Some("↵"));
        assert_eq!(action_shortcut(false), None);
    }
}
