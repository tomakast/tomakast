//! The launcher view: a search field over a virtualized list of hits.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, TestSupportExt as _,
    UniformListScrollHandle, Window, div, img, prelude::FluentBuilder as _, px, uniform_list,
};
use tomakast_core::{ActionOutcome, Core, Hit, Icon, LocalNow};

use crate::theme;
use crate::window::hide_launcher;

gpui_kit::actions!(launcher, [SelectPrev, SelectNext, Confirm, Dismiss]);

/// Key context of the launcher view.
pub(crate) const KEY_CONTEXT: &str = "Launcher";
const PLACEHOLDER: &str = "Search apps and commands…";
/// How long an action error stays in the bottom line.
const ERROR_TTL: Duration = Duration::from_secs(4);
const ROW_HEIGHT: f32 = 44.;
const ICON_SIZE: f32 = 24.;

pub struct LauncherView {
    core: Rc<RefCell<Core>>,
    input: Entity<InputState>,
    hits: Vec<Hit>,
    selected: usize,
    scroll: UniformListScrollHandle,
    error: Option<SharedString>,
    /// Clears `error` after `ERROR_TTL`; replacing it cancels the previous timer.
    error_timer: Option<Task<()>>,
    /// When the show request was received (hotkey activation or IPC, via
    /// `show_at`), taken by the first render to log the time to the first frame.
    requested_at: Option<Instant>,
    /// Deactivation hides the window only after it has been active, so a
    /// window that never takes the keyboard does not close itself.
    was_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl LauncherView {
    /// Builds the view of a newly opened window: an empty query, focused, with
    /// the results of the empty query. Must not touch the `LauncherController`
    /// global, which is leased while the window opens.
    pub(crate) fn new(
        core: Rc<RefCell<Core>>,
        requested_at: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LauncherView {
        // GPUI also runs appearance observers once right after the window
        // opens, but only after the first frame is drawn; syncing here makes
        // that first frame use the right theme.
        theme::sync(window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(PLACEHOLDER));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.search(cx);
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                if this.was_active && !active {
                    cx.defer(hide_launcher);
                }
                this.was_active = active;
            }),
            cx.observe_window_appearance(window, |_, window, cx| theme::sync(window, cx)),
        ];
        let mut view = LauncherView {
            core,
            input,
            hits: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            error: None,
            error_timer: None,
            requested_at: Some(requested_at),
            was_active: window.is_window_active(),
            _subscriptions: subscriptions,
        };
        view.search(cx);
        view
    }

    /// Recomputes the hits for the current query, keeping the selected item
    /// selected when it is still there.
    pub(crate) fn core_changed(&mut self, cx: &mut Context<Self>) {
        let selected = self.hits.get(self.selected).map(|h| h.item.id.clone());
        self.hits = self.run_search(cx);
        self.selected = selected
            .and_then(|id| self.hits.iter().position(|h| h.item.id == id))
            .unwrap_or(0);
        cx.notify();
    }

    fn run_search(&self, cx: &App) -> Vec<Hit> {
        let query = self.input.read(cx).value();
        self.core.borrow_mut().search(&query, LocalNow::now())
    }

    /// New query: new hits, the selection back on the first row, scrolled to the top.
    fn search(&mut self, cx: &mut Context<Self>) {
        self.hits = self.run_search(cx);
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix != self.selected {
            self.selected = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected.saturating_sub(1), cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.hits.len().saturating_sub(1);
        self.select((self.selected + 1).min(last), cx);
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(self.selected) else {
            return;
        };
        let item = hit.item.clone();
        let Some(action) = item.actions.first() else {
            return;
        };
        let query = self.input.read(cx).value();
        let result = self
            .core
            .borrow_mut()
            .execute(&item.id, &action.id, &query, LocalNow::now());
        match result {
            Ok(ActionOutcome::HideWindow) => cx.defer(hide_launcher),
            Ok(ActionOutcome::KeepOpen) => {}
            Err(e) => {
                tracing::warn!(item = %item.id.as_str(), error = %e, "action failed");
                self.show_error(e.to_string(), cx);
            }
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.defer(hide_launcher);
    }

    fn show_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        self.error_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ERROR_TTL).await;
            let _ = this.update(cx, |this, cx| {
                this.error = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Logs the time from receiving the show request to the first frame
    /// callback (hotkey receipt -> frame, the spec's open-latency metric).
    fn measure_show(&mut self, window: &Window) {
        let Some(requested_at) = self.requested_at.take() else {
            return;
        };
        window.on_next_frame(move |_, _| {
            tracing::debug!(
                ms = requested_at.elapsed().as_secs_f64() * 1000.0,
                "launcher show request -> on_next_frame"
            );
        });
    }
}

fn row(hit: &Hit, ix: usize, selected: bool, palette: &theme::Palette) -> AnyElement {
    let item = &hit.item;
    let icon = match &item.icon {
        Some(Icon::File(path)) => img(path.clone()).size(px(ICON_SIZE)).into_any_element(),
        // Icon names are resolved to files by the modules; anything else is blank space.
        _ => div().size(px(ICON_SIZE)).into_any_element(),
    };
    let text = div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(div().truncate().child(item.title.clone()))
        .children(item.subtitle.clone().map(|subtitle| {
            div()
                .truncate()
                .text_sm()
                .text_color(palette.muted)
                .child(subtitle)
        }));
    div()
        .id(("hit", ix))
        .role(Role::ListItem)
        .aria_label(item.title.clone())
        .test_support()
        .h(px(ROW_HEIGHT))
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .rounded(px(8.))
        .when(selected, |row| row.bg(palette.selected))
        .child(icon)
        .child(text)
        .into_any_element()
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure_show(window);
        let palette = theme::palette(cx);
        let error_line = div().h(px(20.)).text_sm().text_color(palette.error);
        let error_line = match &self.error {
            Some(message) => error_line
                .id("launcher-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .test_support()
                .child(message.clone())
                .into_any_element(),
            None => error_line.into_any_element(),
        };
        let hits = uniform_list(
            "launcher-hits",
            self.hits.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let palette = theme::palette(cx);
                range
                    .filter_map(|ix| {
                        let hit = this.hits.get(ix)?;
                        Some(row(hit, ix, ix == this.selected, &palette))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        div()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(theme::PANEL_RADIUS))
            .overflow_hidden()
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .text_color(palette.text)
            .child(
                Input::new(&self.input)
                    .id("launcher-input")
                    .aria_label("Search"),
            )
            .child(hits)
            .child(error_line)
    }
}
