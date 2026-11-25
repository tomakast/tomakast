//! The action panel: Ctrl+K (Cmd+K on macOS) over the current view lists the
//! selected row's actions, with a filter field of its own.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::cell::Cell;

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    MouseDownEvent, ParentElement as _, Pixels, Point, Role, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window,
    anchored, deferred, div, prelude::FluentBuilder as _, px,
};
use tomakast_core::{
    Action, ItemId, Matcher, MenuOption, Pattern, Scripts, TextKeys, TileKey, WidgetSize,
};

use crate::icons::IconSpec;

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
/// action, so only the selected row shows it, and only when it has no keys of its own.
fn action_shortcut(selected: bool) -> Option<&'static str> {
    selected.then_some("↵")
}

/// The chips of an action's usable shortcut, labelled `Ctrl+Shift+C`.
fn shortcut_chips(ix: usize, action: &Action, palette: &Palette) -> Option<AnyElement> {
    let chips = crate::shortcut::chips(&crate::shortcut::usable(action)?);
    Some(chips_element(ix, chips, palette))
}

/// Key chips in a row, labelled with their keys joined by `+`.
fn chips_element(ix: usize, chips: Vec<String>, palette: &Palette) -> AnyElement {
    {
        div()
            .id(("action-keys", ix))
            .aria_label(chips.join("+"))
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .children(chips.iter().map(|chip| key_chip(chip, palette)))
            .into_any_element()
    }
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
    /// The launcher's own "Edit Home", last on a Home tile's panel: it enters
    /// the Home edit mode with that tile selected.
    EditHome,
    /// In the Home edit mode's tile menu: a size the widget offers;
    /// `current` is the tile's own, which shows a check mark.
    Size { size: WidgetSize, current: bool },
    /// In the Home edit mode's tile menu: removes the tile.
    Remove,
    /// On a root row of plugin `0` that has preferences or commands: opens the settings
    /// window on the plugin's page.
    PluginSettings(String),
    /// In a toolbar button's menu: one of its options; `current` is the selected one, which
    /// shows a check mark.
    Choice { option: MenuOption, current: bool },
}

/// The title of the launcher's own entry that opens a plugin's settings page.
pub(crate) const PLUGIN_SETTINGS: &str = "Plugin Settings";

impl PanelEntry {
    fn title(&self) -> &str {
        match self {
            PanelEntry::Action(action) => &action.title,
            PanelEntry::AddToHome => ADD_TO_HOME,
            PanelEntry::EditHome => tomakast_core::EDIT_HOME,
            PanelEntry::Size { size, .. } => size_title(*size),
            PanelEntry::Remove => "Remove",
            PanelEntry::PluginSettings(_) => PLUGIN_SETTINGS,
            PanelEntry::Choice { option, .. } => &option.title,
        }
    }

    /// The texts the filter matches: the title, and a choice's subtitle and keywords.
    fn texts(&self) -> Vec<&str> {
        let mut texts = vec![self.title()];
        if let PanelEntry::Choice { option, .. } = self {
            texts.extend(option.subtitle.as_deref());
            texts.extend(option.keywords.iter().map(String::as_str));
        }
        texts
    }

    fn action(&self) -> Option<&Action> {
        match self {
            PanelEntry::Action(action) => Some(action),
            _ => None,
        }
    }

    /// Whether it is the tile's current size, or the menu's current choice.
    fn current(&self) -> bool {
        matches!(
            self,
            PanelEntry::Size { current: true, .. } | PanelEntry::Choice { current: true, .. }
        )
    }
}

/// The menu title of `size`.
pub(crate) fn size_title(size: WidgetSize) -> &'static str {
    match size {
        WidgetSize::Quarter => "Quarter",
        WidgetSize::Single => "Single",
        WidgetSize::Double => "Double",
    }
}

/// The check mark after the current size, labelled for a screen reader.
fn current_mark(ix: usize, palette: &Palette) -> AnyElement {
    let mark = crate::icons::bundled_path("check").map(|path| {
        gpui_kit::svg()
            .path(path)
            .size(theme::text(theme::ROW_TEXT_SIZE))
            .text_color(palette.text)
    });
    div()
        .id(("action-current", ix))
        .aria_label("Current size")
        .test_support()
        .flex_none()
        .flex()
        .items_center()
        // A check character when the bundle lacks the icon.
        .when_none(&mark, |div| div.child("\u{2713}"))
        .children(mark)
        .into_any_element()
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
    /// The tile of Home entry `0` (an index in the core's Home list) in the
    /// Home edit mode: its menu of sizes and Remove.
    HomeEdit(usize),
    /// The menu of toolbar button `button` of the compose pad of pushed view `instance`.
    Toolbar { instance: u64, button: String },
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
    /// Match keys of each entry's texts ([`PanelEntry::texts`]), built once: the entries are
    /// fixed.
    keys: Vec<Vec<TextKeys>>,
    scripts: Scripts,
    /// A toolbar menu: placed under its button on every frame ([`MenuPlace`]). Otherwise
    /// the Ctrl+K panel, over the bottom right corner.
    is_menu: bool,
    scroll: ScrollHandle,
    /// The selected row must be scrolled into view once the list has been laid out: a
    /// scroll asked of a list never laid out is lost.
    scroll_pending: Cell<bool>,
    /// Chips shown on the first entry instead of its own shortcut: a pad's first action,
    /// run by Ctrl+Enter (Cmd+Enter on macOS).
    first_keys: Option<Vec<String>>,
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
        Self::open(target, entries, false, "Filter actions…", window, cx)
    }

    /// A toolbar button's menu over `entries`, with its current choice selected; it hangs
    /// where [`render`](Self::render) is told.
    pub(crate) fn menu(
        target: PanelTarget,
        entries: Vec<PanelEntry>,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        let mut panel = Self::open(target, entries, true, "Filter…", window, cx);
        if let Some(ix) = panel.entries.iter().position(PanelEntry::current) {
            panel.select(ix);
        }
        panel
    }

    fn open(
        target: PanelTarget,
        entries: Vec<PanelEntry>,
        is_menu: bool,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        input.update(cx, |input, cx| input.focus(window, cx));
        let input_events = cx.subscribe(&input, |host, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                host.action_filter_changed(cx);
            }
        });
        let visible = (0..entries.len()).collect();
        let keys: Vec<Vec<TextKeys>> = entries
            .iter()
            .map(|e| e.texts().into_iter().map(TextKeys::new).collect())
            .collect();
        let scripts = keys
            .iter()
            .flatten()
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
            is_menu,
            scroll: ScrollHandle::new(),
            scroll_pending: Cell::new(false),
            first_keys: None,
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
                entry
                    .texts()
                    .into_iter()
                    .zip(&keys[*ix])
                    .any(|(text, keys)| matcher.score_text(&pattern, text, keys).is_some())
            })
            .map(|(ix, _)| ix)
            .collect();
        self.selected = 0;
        self.scroll_pending.set(true);
    }

    pub(crate) fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.scroll_pending.set(true);
    }

    pub(crate) fn select_next(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
        self.scroll_pending.set(true);
    }

    /// Selects row `ix` of those the filter shows; `false`, and nothing
    /// changes, when there is no such row.
    pub(crate) fn select(&mut self, ix: usize) -> bool {
        let shown = ix < self.visible.len();
        if shown {
            self.selected = ix;
            self.scroll_pending.set(true);
        }
        shown
    }

    /// The modules' actions of the row when the panel opened, filtered or
    /// not: those a shortcut may run.
    pub(crate) fn actions(&self) -> impl Iterator<Item = &Action> {
        self.entries.iter().filter_map(PanelEntry::action)
    }

    /// Shows `chips` on the first entry: the key that runs it outside the panel.
    pub(crate) fn with_first_keys(mut self, chips: Vec<String>) -> Self {
        self.first_keys = Some(chips);
        self
    }

    /// Whether it is a toolbar menu.
    pub(crate) fn is_menu(&self) -> bool {
        self.is_menu
    }

    /// Asks the list to scroll the selected row into view, once it has been laid out;
    /// `true` while it has not been, and another frame is needed.
    fn scroll_selected(&self) -> bool {
        if !self.scroll_pending.get() {
            return false;
        }
        if self.scroll.bounds().size.height <= px(0.) {
            return true;
        }
        self.scroll.scroll_to_item(self.selected);
        self.scroll_pending.set(false);
        false
    }

    pub(crate) fn selected_entry(&self) -> Option<&PanelEntry> {
        self.entries.get(*self.visible.get(self.selected)?)
    }

    /// The popover, floating above the bottom right corner of the launcher:
    /// the actions, then the filter field under a divider. A click on a row
    /// runs its entry. The popover hides what is under it from the pointer,
    /// so a press on it is the panel's alone: in the Home edit mode it does
    /// not count as a press outside the tiles.
    /// A toolbar menu is drawn at `place`, and not at all without one (its button has not
    /// been painted yet: the next frame has it).
    pub(crate) fn render(
        &self,
        palette: &Palette,
        place: Option<MenuPlace>,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> AnyElement {
        if self.scroll_selected() || (self.is_menu && place.is_none()) {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        let actions = self
            .visible
            .iter()
            .enumerate()
            .filter_map(|(ix, &entry_ix)| {
                let shortcut = action_shortcut(ix == self.selected);
                let entry = self.entries.get(entry_ix)?;
                let first_keys = self
                    .first_keys
                    .clone()
                    .filter(|_| entry_ix == 0 && entry.action().is_some());
                let title = entry.title().to_string();
                // A row's own keys, else the Enter hint when it is selected: not both.
                let keys = match first_keys {
                    Some(chips) => Some(chips_element(ix, chips, palette)),
                    None => entry
                        .action()
                        .and_then(|action| shortcut_chips(ix, action, palette)),
                };
                if let PanelEntry::Choice { option, current } = entry {
                    return Some(choice_row(
                        ix,
                        option,
                        *current,
                        ix == self.selected,
                        palette,
                        cx,
                    ));
                }
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
                        .text_size(theme::text(theme::ROW_TEXT_SIZE))
                        .when(ix == self.selected, |row| row.bg(palette.selected))
                        .on_click(cx.listener(move |this, e, window, cx| {
                            this.panel_row_clicked(ix, e, window, cx)
                        }))
                        .child(div().min_w_0().truncate().child(title))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_2()
                                .children(entry.current().then(|| current_mark(ix, palette)))
                                .children(keys.or_else(|| {
                                    shortcut.map(|label| {
                                        div()
                                            .id(("action-enter", ix))
                                            .test_support()
                                            .child(key_chip(label, palette))
                                            .into_any_element()
                                    })
                                })),
                        )
                        .into_any_element(),
                )
            });
        let list = div()
            .id("action-list")
            .test_support()
            .flex_1()
            .min_h_0()
            .p(px(theme::POPOVER_PADDING))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(actions);
        let filter = div()
            .flex_none()
            .h(px(theme::POPOVER_INPUT_HEIGHT))
            .px(px(theme::POPOVER_INPUT_PADDING_X))
            .flex()
            .items_center()
            .child(
                Input::new(&self.input)
                    .id("action-input")
                    .aria_label(if self.is_menu {
                        "Filter"
                    } else {
                        "Filter actions"
                    })
                    .appearance(false)
                    .focus_bordered(false)
                    // Without the kit's inner padding the placeholder
                    // lines up with the action titles.
                    .px_0(),
            );
        let divider = || div().h(px(theme::DIVIDER_THICKNESS)).bg(palette.divider);
        let frame = || {
            div()
                .id("action-panel")
                .role(Role::Menu)
                .test_support()
                .occlude()
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
        };
        if self.is_menu {
            let Some(place) = place else {
                return div().into_any_element();
            };
            // A toolbar menu: the filter on top, hanging under its button, no taller than
            // the room down to the footer; a press outside it closes it.
            let menu = frame()
                .aria_label("Menu")
                .max_h(place.max_height)
                .on_mouse_down_out(cx.listener(|host, e: &MouseDownEvent, window, cx| {
                    host.menu_pressed_outside(e.position, window, cx)
                }))
                .child(filter)
                .child(divider())
                .child(list);
            return deferred(
                anchored()
                    .position(place.anchor)
                    .snap_to_window_with_margin(px(theme::POPOVER_MARGIN))
                    .child(menu),
            )
            .with_priority(1)
            .into_any_element();
        }
        frame()
            .aria_label("Actions")
            .absolute()
            .right(px(theme::POPOVER_MARGIN))
            .bottom(px(theme::FOOTER_HEIGHT + theme::POPOVER_MARGIN))
            .child(list)
            .child(divider())
            .child(filter)
            .into_any_element()
    }
}

/// Where a toolbar menu hangs: its top left corner, in window coordinates, and its height
/// at most.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MenuPlace {
    pub anchor: Point<Pixels>,
    pub max_height: Pixels,
}

/// A row of a toolbar menu: the option's icon, title and subtitle, and a check mark on the
/// current one. A click chooses it.
fn choice_row(
    ix: usize,
    option: &MenuOption,
    current: bool,
    selected: bool,
    palette: &Palette,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let icon = option.icon.as_ref().map(|icon| {
        crate::views::icon_element(
            IconSpec {
                icon: Some(icon),
                monogram: &option.title,
            },
            theme::ICON_SIZE,
            palette,
        )
    });
    let subtitle = option.subtitle.clone().filter(|s| *s != option.title);
    div()
        .id(("action", ix))
        .role(Role::MenuItem)
        .aria_label(option.title.clone())
        .test_support()
        .h(px(theme::ACTION_HEIGHT))
        .px(px(theme::ACTION_PADDING_X))
        .flex()
        .items_center()
        .gap_2()
        .flex_none()
        .rounded(px(theme::ACTION_RADIUS))
        .text_size(theme::text(theme::ROW_TEXT_SIZE))
        .when(selected, |row| row.bg(palette.selected))
        .on_click(cx.listener(move |this, e, window, cx| this.panel_row_clicked(ix, e, window, cx)))
        .children(icon)
        .child(div().flex_none().child(option.title.clone()))
        .children(subtitle.map(|s| {
            div()
                .min_w_0()
                .truncate()
                .text_color(palette.muted)
                .child(s)
        }))
        .child(div().flex_1())
        .children(current.then(|| current_mark(ix, palette)))
        .into_any_element()
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
