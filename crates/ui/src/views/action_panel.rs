//! The action panel: Ctrl+K (Cmd+K on macOS) over the current view lists the
//! selected row's actions, with a filter field of its own.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::{Action, ItemId, Matcher, Pattern, Scripts, TextKeys, TileKey};

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

/// The Enter hint shown on a row of the panel: Enter runs the selected
/// action, so only the selected row shows it.
fn action_shortcut(selected: bool) -> Option<&'static str> {
    selected.then_some("↵")
}

/// The chips of an action's usable shortcut, labelled `Ctrl+Shift+C`.
fn shortcut_chips(ix: usize, action: &Action, palette: &Palette) -> Option<AnyElement> {
    let chips = crate::shortcut::chips(&crate::shortcut::usable(action)?);
    Some(
        div()
            .id(("action-keys", ix))
            .aria_label(chips.join("+"))
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .children(chips.iter().map(|chip| key_chip(chip, palette)))
            .into_any_element(),
    )
}

/// The title of the launcher's own action that appends a search tile to Home.
pub(crate) const ADD_TO_HOME: &str = tomakast_core::ADD_TO_HOME;

/// One entry of the panel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PanelEntry {
    /// An action of the row or tile, which its module runs.
    Action(Action),
    /// The launcher's own "Add to Home", after a search tile's actions; the
    /// launcher runs it, so no action of a module can stand for it.
    AddToHome,
}

impl PanelEntry {
    fn title(&self) -> &str {
        match self {
            PanelEntry::Action(action) => &action.title,
            PanelEntry::AddToHome => ADD_TO_HOME,
        }
    }

    fn action(&self) -> Option<&Action> {
        match self {
            PanelEntry::Action(action) => Some(action),
            PanelEntry::AddToHome => None,
        }
    }
}

/// What the panel's actions act on.
#[derive(Clone)]
pub(crate) enum PanelTarget {
    /// An item of the root list.
    Root(ItemId),
    /// A row of pushed view `instance`.
    Row { instance: u64, row_id: String },
    /// A tile of the widget strip.
    Tile(TileKey),
}

pub(crate) struct ActionPanel {
    pub target: PanelTarget,
    /// The entries of the row when the panel opened.
    entries: Vec<PanelEntry>,
    input: Entity<InputState>,
    /// Indices into `entries` that pass the filter.
    visible: Vec<usize>,
    /// Index into `visible`.
    selected: usize,
    matcher: Matcher,
    /// Match keys of the entry titles, built once: the entries are fixed.
    keys: Vec<TextKeys>,
    scripts: Scripts,
    _input_events: Subscription,
}

impl ActionPanel {
    /// A panel over `entries`, with the focus on its filter field.
    pub(crate) fn new(
        target: PanelTarget,
        entries: Vec<PanelEntry>,
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
        let visible = (0..entries.len()).collect();
        let keys: Vec<TextKeys> = entries.iter().map(|e| TextKeys::new(e.title())).collect();
        let scripts = keys
            .iter()
            .fold(Scripts::default(), |all, k| all.union(k.scripts()));
        ActionPanel {
            target,
            entries,
            input,
            visible,
            selected: 0,
            matcher: Matcher::new(),
            keys,
            scripts,
            _input_events: input_events,
        }
    }

    pub(crate) fn query(&self, cx: &Context<LauncherView>) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Keeps the entries whose title matches `query`; the first is selected.
    pub(crate) fn refilter(&mut self, query: &str) {
        let pattern = Pattern::across(query, self.scripts, &mut self.matcher);
        let matcher = &mut self.matcher;
        let keys = &self.keys;
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(ix, entry)| {
                matcher
                    .score_text(&pattern, entry.title(), &keys[*ix])
                    .is_some()
            })
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

    /// The modules' actions of the row when the panel opened, filtered or
    /// not: those a shortcut may run.
    pub(crate) fn actions(&self) -> impl Iterator<Item = &Action> {
        self.entries.iter().filter_map(PanelEntry::action)
    }

    pub(crate) fn selected_entry(&self) -> Option<&PanelEntry> {
        self.entries.get(*self.visible.get(self.selected)?)
    }

    /// The popover, floating above the bottom right corner of the launcher:
    /// the actions, then the filter field under a divider.
    pub(crate) fn render(&self, palette: &Palette) -> AnyElement {
        let actions = self.visible.iter().enumerate().filter_map(|(ix, &entry)| {
            let shortcut = action_shortcut(ix == self.selected);
            let entry = self.entries.get(entry)?;
            let title = entry.title().to_string();
            Some(
                div()
                    .id(("action", ix))
                    .role(Role::MenuItem)
                    .aria_label(title.clone())
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
                    .child(div().min_w_0().truncate().child(title))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(
                                entry
                                    .action()
                                    .and_then(|action| shortcut_chips(ix, action, palette)),
                            )
                            .children(shortcut.map(|label| key_chip(label, palette))),
                    ),
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
