//! The launcher view: the root search list, with module-owned views pushed
//! above it on a [`ViewStack`].

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, ScrollDelta, ScrollStrategy, Styled as _,
    Subscription, Task, UniformListScrollHandle, Window, div, px, uniform_list,
};
use tomakast_core::{
    Accessory, Action, Core, CoreUpdate, DYNAMIC_SEARCH_DEADLINE, FormView, Hit, ItemId, LocalNow,
    Pattern, SearchMode, SearchSnapshot, Toast, ToastKind, View, ViewEvent,
};

use crate::chrome;
use crate::root_sections::{RootRow, kind_label, root_rows};
use crate::stack::{Issuer, RequestKind, SharedLedger, StackEntry, Updated, ViewStack};
use crate::theme;
use crate::toast::ToastBar;
use crate::views::action_panel::{ActionPanel, PanelTarget};
use crate::views::form::FormState;
use crate::views::scratchpad::TEXT_DEBOUNCE;
use crate::views::{self, ViewState};
use crate::window::hide_launcher;

gpui_kit::actions!(
    launcher,
    [
        SelectPrev,
        SelectNext,
        SelectLeft,
        SelectRight,
        Confirm,
        Dismiss,
        ToggleActions,
        SubmitForm
    ]
);

/// Key context of the launcher view.
pub(crate) const KEY_CONTEXT: &str = "Launcher";
const PLACEHOLDER: &str = "Search apps and commands…";
/// Dev-only: pre-fills the root query on show (see the crate docs).
const DEV_QUERY_ENV: &str = "TOMAKAST_DEV_QUERY";

/// A row of the root list.
struct RootHit {
    hit: Hit,
    /// A dynamic answer to the previous query, shown until the current
    /// query's answers arrive (see [`LauncherView::search`]).
    held: bool,
}

pub struct LauncherView {
    core: Rc<RefCell<Core>>,
    ledger: SharedLedger,
    /// This window's number in the ledger.
    window_number: u64,
    input: Entity<InputState>,
    /// Search generation of `snapshot`; results of other generations are ignored.
    generation: u64,
    /// The core's latest hits for the current query.
    snapshot: Vec<Hit>,
    /// Dynamic answers to the previous query, kept until the current query's
    /// answers arrive or `DYNAMIC_SEARCH_DEADLINE` passes, so they do not
    /// flicker while typing.
    held: Vec<Hit>,
    /// Drops `held` at the deadline.
    held_timer: Option<Task<()>>,
    /// The root hits: `snapshot` merged with `held`, by rank.
    hits: Vec<RootHit>,
    /// Whether the current query is empty, which picks the root sections.
    query_empty: bool,
    /// The lines of the root list: section labels and `hits`, in the order shown.
    rows: Vec<RootRow>,
    /// Index into `hits`, never a label.
    selected: usize,
    scroll: UniformListScrollHandle,
    stack: ViewStack,
    /// The open action panel; it has the focus and the keys while open.
    panel: Option<ActionPanel>,
    /// Dev-only: keep the panel open (see the crate docs).
    dev_actions: bool,
    toasts: Entity<ToastBar>,
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
        ledger: SharedLedger,
        window_number: u64,
        requested_at: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LauncherView {
        // GPUI also runs appearance observers once right after the window
        // opens, but only after the first frame is drawn; syncing here makes
        // that first frame use the right theme.
        theme::sync(window, cx);
        let dev_query = std::env::var(DEV_QUERY_ENV).unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(PLACEHOLDER)
                .default_value(dev_query)
        });
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
            ledger,
            window_number,
            input,
            generation: 0,
            snapshot: Vec::new(),
            held: Vec::new(),
            held_timer: None,
            hits: Vec::new(),
            query_empty: true,
            rows: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            stack: ViewStack::default(),
            panel: None,
            dev_actions: views::action_panel::dev_actions(),
            toasts: cx.new(|_| ToastBar::default()),
            requested_at: Some(requested_at),
            was_active: window.is_window_active(),
            _subscriptions: subscriptions,
        };
        view.search(cx);
        view
    }

    pub(crate) fn stack(&self) -> &ViewStack {
        &self.stack
    }

    /// For tests: where the editor of the top scratchpad laid out the start
    /// of `line` in the last frame; `None` when that line is not laid out.
    #[doc(hidden)]
    pub fn scratchpad_line_bounds(&self, line: usize, cx: &App) -> Option<Bounds<Pixels>> {
        match self.stack.top()? {
            StackEntry {
                state: ViewState::Scratchpad(state),
                ..
            } => state.line_bounds(line, cx),
            _ => None,
        }
    }

    /// Whether pushed view `instance` is still on the stack.
    pub(crate) fn has_view(&self, instance: u64) -> bool {
        self.stack.contains(instance)
    }

    /// Applies new results of the root search: `ItemsChanged` or `Results`.
    pub(crate) fn root_update(&mut self, update: CoreUpdate, cx: &mut Context<Self>) {
        let (snapshot, answers) = match update {
            CoreUpdate::ItemsChanged => (self.core.borrow_mut().refresh(LocalNow::now()), false),
            CoreUpdate::Results(snapshot) => (snapshot, true),
            _ => return,
        };
        self.replace_hits(snapshot, answers, cx);
    }

    /// New hits for the current query, keeping the selected item selected
    /// when it is still there. Hits of an older search are ignored. With
    /// `answers` (a dynamic source answered the current query) the held
    /// answers to the previous query go.
    fn replace_hits(&mut self, snapshot: SearchSnapshot, answers: bool, cx: &mut Context<Self>) {
        if snapshot.generation != self.generation {
            return;
        }
        let selected = self.hits.get(self.selected).map(|h| h.hit.item.id.clone());
        if answers {
            self.held.clear();
            self.held_timer = None;
        }
        self.snapshot = snapshot.hits;
        self.merge_hits();
        self.reselect(selected);
        cx.notify();
    }

    /// New query: new hits, the selection back on the first row, scrolled to
    /// the top. The dynamic answers shown so far are held until the new
    /// query's answers arrive or `DYNAMIC_SEARCH_DEADLINE` passes; the
    /// deadline restarts only when fresh answers were shown, so held rows
    /// never outlive it while the user keeps typing.
    fn search(&mut self, cx: &mut Context<Self>) {
        let fresh: Vec<Hit> = self
            .snapshot
            .iter()
            .filter(|h| h.dynamic)
            .cloned()
            .collect();
        if !fresh.is_empty() {
            self.held = fresh;
            self.held_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(DYNAMIC_SEARCH_DEADLINE)
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.held.clear();
                    this.held_timer = None;
                    let selected = this.hits.get(this.selected).map(|h| h.hit.item.id.clone());
                    this.merge_hits();
                    this.reselect(selected);
                    cx.notify();
                });
            }));
        }
        let query = self.input.read(cx).value();
        // As the engine decides: a query of blanks is empty too.
        self.query_empty = Pattern::parse(&query).is_empty();
        let snapshot = self.core.borrow_mut().search(&query, LocalNow::now());
        self.generation = snapshot.generation;
        self.snapshot = snapshot.hits;
        self.merge_hits();
        // The first listed row, but not a held answer: Enter on it is
        // ignored, and a quick "fir" + Enter must run the first real match.
        self.selected = self
            .rows
            .iter()
            .filter_map(|row| row.hit())
            .find(|&ix| !self.hits[ix].held)
            .unwrap_or(0);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Selects the hit with id `id` when it is still listed; otherwise the
    /// first listed row.
    fn reselect(&mut self, id: Option<ItemId>) {
        self.selected = id
            .and_then(|id| self.hits.iter().position(|h| h.hit.item.id == id))
            .or_else(|| self.rows.iter().find_map(|row| row.hit()))
            .unwrap_or(0);
    }

    /// `hits` = `snapshot` plus the held answers it does not have, by score.
    fn merge_hits(&mut self) {
        let held = self
            .held
            .iter()
            .filter(|h| !self.snapshot.iter().any(|s| s.item.id == h.item.id))
            .map(|hit| RootHit {
                hit: hit.clone(),
                held: true,
            });
        let current = self.snapshot.iter().map(|hit| RootHit {
            hit: hit.clone(),
            held: false,
        });
        self.hits = held.chain(current).collect();
        self.hits
            .sort_by(|a, b| b.hit.score.total_cmp(&a.hit.score));
        let keys: Vec<_> = self
            .hits
            .iter()
            .map(|h| {
                (
                    h.hit.item.kind,
                    h.hit.item.title.as_str(),
                    // A dynamic or held answer is not an item with usage.
                    !h.hit.dynamic && !h.held && h.hit.score > 0.,
                )
            })
            .collect();
        self.rows = root_rows(&keys, self.query_empty);
    }

    /// Pushes a module-owned view; it takes the focus.
    pub(crate) fn push_view(
        &mut self,
        provider: &str,
        view: View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = match &view {
            View::List(list) => list.revision,
            View::Scratchpad(pad) => pad.revision,
            View::Grid(_) | View::Detail(_) | View::Form(_) => 0,
        };
        // The new view takes the focus from the panel.
        self.panel = None;
        let instance = self.stack.new_instance();
        let state = ViewState::new(instance, &view, window, cx);
        self.stack.push(StackEntry {
            instance,
            provider: provider.to_string(),
            view,
            revision,
            state,
        });
        cx.notify();
    }

    /// Applies an `UpdateView` from `provider` (revision rule in
    /// [`ViewStack::update`]). A reply of view `instance` updates that
    /// instance only; a root-action reply (`None`) updates the provider's
    /// topmost view with the same id. The selected row stays selected by id.
    pub(crate) fn update_view(
        &mut self,
        provider: &str,
        instance: Option<u64>,
        view: View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.id();
        let target = match instance {
            Some(instance) => self
                .stack
                .get(instance)
                .filter(|e| e.provider == provider && e.view.id() == id),
            None => self.stack.find(provider, id),
        };
        let Some(target) = target else {
            tracing::debug!("update for a view not on the stack ignored");
            return;
        };
        let instance = target.instance;
        // Read before the update: the selection indexes the current rows.
        let selected = target.state.selected_id(&target.view);
        match self.stack.update(instance, view) {
            Updated::Applied => {}
            Updated::Stale => {
                tracing::debug!("stale view update ignored");
                return;
            }
            Updated::Unknown => {
                tracing::debug!("update for a view not on the stack ignored");
                return;
            }
            Updated::KindChanged => {
                tracing::warn!("view update changes the view kind; ignored");
                return;
            }
            Updated::FieldsChanged => {
                tracing::warn!("form update changes the fields; ignored");
                return;
            }
        }
        if let Some(entry) = self.stack.get_mut(instance) {
            match (&entry.view, &mut entry.state) {
                (View::List(view), ViewState::List(state)) => {
                    let query = state.query(cx);
                    state.refilter(view, &query, selected.as_deref());
                }
                (View::Grid(view), ViewState::Grid(state)) => {
                    state.reselect(view, selected.as_deref());
                }
                (View::Form(view), ViewState::Form(state)) => {
                    state.apply_update(view, window, cx);
                }
                _ => {}
            }
        }
        cx.notify();
    }

    /// Pops the top view and gives the focus to the one below.
    pub(crate) fn pop_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.pop().is_none() {
            tracing::debug!("PopView on the root list ignored");
            return;
        }
        // The panel belongs to the view that went.
        self.panel = None;
        match self.stack.top() {
            Some(entry) => entry.state.focus(window, cx),
            None => self.input.update(cx, |input, cx| input.focus(window, cx)),
        }
        cx.notify();
    }

    /// A module's `PopView`: pops the top view only if it is the view that
    /// sent the event (`instance`), or, for a root-action reply (`None`), a
    /// view `provider` pushed. Another module's view is never popped.
    pub(crate) fn pop_for_reply(
        &mut self,
        provider: &str,
        instance: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owned = self.stack.top().is_some_and(|top| match instance {
            Some(instance) => top.instance == instance,
            None => top.provider == provider,
        });
        if owned {
            self.pop_view(window, cx);
        } else {
            tracing::debug!("PopView for a view that is not on top ignored");
        }
    }

    pub(crate) fn show_toast(&mut self, toast: Toast, cx: &mut Context<Self>) {
        self.toasts.update(cx, |toasts, cx| toasts.show(toast, cx));
    }

    pub(crate) fn show_error_toast(&mut self, message: String, cx: &mut Context<Self>) {
        let toast = Toast {
            kind: ToastKind::Error,
            title: message,
            message: None,
        };
        self.show_toast(toast, cx);
    }

    /// The filter text of pushed list `instance` changed. As in the root
    /// list, the selection goes back to the first row.
    pub(crate) fn list_query_changed(&mut self, instance: u64, cx: &mut Context<Self>) {
        let Some(entry) = self.stack.get_mut(instance) else {
            return;
        };
        let (View::List(view), ViewState::List(state)) = (&entry.view, &mut entry.state) else {
            return;
        };
        let text = state.query(cx);
        state.refilter(view, &text, None);
        if view.search == SearchMode::Module {
            entry.revision += 1;
            let event = ViewEvent::QueryChanged {
                revision: entry.revision,
                text,
            };
            self.send_view_event(instance, event, RequestKind::Query, cx);
        }
        cx.notify();
    }

    /// Sends `event` to the module that owns view `instance` and records it
    /// in the ledger; a failure to send shows an error toast.
    fn send_view_event(
        &mut self,
        instance: u64,
        event: ViewEvent,
        kind: RequestKind,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.stack.get(instance) else {
            return;
        };
        let sent = self
            .core
            .borrow_mut()
            .send_view_event(&entry.provider, entry.view.id(), event);
        match sent {
            Ok(request) => self.ledger.borrow_mut().issue(
                request,
                Issuer {
                    window: self.window_number,
                    provider: &entry.provider,
                    view: Some(instance),
                },
                kind,
                cx.background_executor().now(),
            ),
            Err(e) => {
                tracing::warn!(error = %e, "view event failed");
                self.show_error_toast(e.to_string(), cx);
            }
        }
    }

    /// The text of scratchpad `instance` was edited: a new revision at once,
    /// so module replies for the old text are stale from now on, and the
    /// text is sent once typing pauses for [`TEXT_DEBOUNCE`].
    pub(crate) fn scratchpad_edited(&mut self, instance: u64, cx: &mut Context<Self>) {
        let Some(StackEntry {
            revision,
            state: ViewState::Scratchpad(state),
            ..
        }) = self.stack.get_mut(instance)
        else {
            return;
        };
        *revision += 1;
        state.debounce(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TEXT_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.send_scratchpad_text(instance, cx));
        }));
    }

    /// Sends `TextChanged` with the current text and revision of scratchpad
    /// `instance`.
    fn send_scratchpad_text(&mut self, instance: u64, cx: &mut Context<Self>) {
        let Some(entry) = self.stack.get(instance) else {
            return;
        };
        let ViewState::Scratchpad(state) = &entry.state else {
            return;
        };
        let event = ViewEvent::TextChanged {
            revision: entry.revision,
            text: state.text(cx),
        };
        self.send_view_event(instance, event, RequestKind::Query, cx);
    }

    /// The wheel over the annotation column of scratchpad `instance`.
    pub(crate) fn scratchpad_scroll(
        &mut self,
        instance: u64,
        delta: ScrollDelta,
        cx: &mut Context<Self>,
    ) {
        if let Some(StackEntry {
            state: ViewState::Scratchpad(state),
            ..
        }) = self.stack.get(instance)
        {
            state.scroll(delta, cx);
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_form_focus(window);
        if let Some(panel) = &mut self.panel {
            panel.select_prev();
            cx.notify();
            return;
        }
        match self.stack.top_mut() {
            Some(StackEntry {
                state: ViewState::List(state),
                ..
            }) => state.select_prev(),
            Some(StackEntry {
                view: View::Grid(view),
                state: ViewState::Grid(state),
                ..
            }) => state.up(view),
            Some(StackEntry {
                view: View::Form(view),
                state: ViewState::Form(state),
                ..
            }) => state.move_highlight(view, false),
            Some(_) => return,
            None => self.step_root(false),
        }
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_form_focus(window);
        if let Some(panel) = &mut self.panel {
            panel.select_next();
            cx.notify();
            return;
        }
        match self.stack.top_mut() {
            Some(StackEntry {
                state: ViewState::List(state),
                ..
            }) => state.select_next(),
            Some(StackEntry {
                view: View::Grid(view),
                state: ViewState::Grid(state),
                ..
            }) => state.down(view),
            Some(StackEntry {
                view: View::Form(view),
                state: ViewState::Form(state),
                ..
            }) => state.move_highlight(view, true),
            Some(_) => return,
            None => self.step_root(true),
        }
        cx.notify();
    }

    /// Left and right move over a grid. Elsewhere they are not the launcher's:
    /// they go on to the text field that has the focus.
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        match self.stack.top_mut() {
            Some(StackEntry {
                view: View::Grid(view),
                state: ViewState::Grid(state),
                ..
            }) if self.panel.is_none() => {
                state.left(view);
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        match self.stack.top_mut() {
            Some(StackEntry {
                view: View::Grid(view),
                state: ViewState::Grid(state),
                ..
            }) if self.panel.is_none() => {
                state.right(view);
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    /// Moves the root selection to the next (`forward`) or previous listed
    /// hit, over section labels; it stays at either end.
    fn step_root(&mut self, forward: bool) {
        let Some(line) = self
            .rows
            .iter()
            .position(|r| *r == RootRow::Hit(self.selected))
        else {
            return;
        };
        let target = if forward {
            self.rows[line + 1..]
                .iter()
                .position(|r| r.hit().is_some())
                .map(|offset| line + 1 + offset)
        } else {
            self.rows[..line].iter().rposition(|r| r.hit().is_some())
        };
        let Some((target, ix)) = target.and_then(|t| Some((t, self.rows[t].hit()?))) else {
            return;
        };
        self.selected = ix;
        // Going up onto the first row of a section brings its label into view too.
        let reveal = match target.checked_sub(1) {
            Some(above) if !forward && matches!(self.rows[above], RootRow::Label(_)) => above,
            _ => target,
        };
        self.scroll.scroll_to_item(reveal, ScrollStrategy::Nearest);
    }

    /// Enter: the primary action of the selected row. Ignored while an action
    /// of the same view is pending, so a double Enter runs it once.
    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            self.confirm_panel(window, cx);
            return;
        }
        match self.stack.top() {
            Some(StackEntry {
                instance,
                state: ViewState::Form(_),
                ..
            }) => self.form_confirm(*instance, window, cx),
            Some(entry) => self.confirm_row(entry.instance, cx),
            None => self.confirm_root(cx),
        }
    }

    /// Closes the dropdowns of the top form that lost the focus.
    fn sync_form_focus(&mut self, window: &Window) {
        if let Some(StackEntry {
            state: ViewState::Form(state),
            ..
        }) = self.stack.top_mut()
        {
            state.close_unfocused_dropdowns(window);
        }
    }

    /// The form of pushed view `instance`, with its state.
    fn form_mut(&mut self, instance: u64) -> Option<(&FormView, &mut FormState)> {
        match self.stack.get_mut(instance)? {
            StackEntry {
                view: View::Form(view),
                state: ViewState::Form(state),
                ..
            } => Some((&*view, state)),
            _ => None,
        }
    }

    /// A text field of form `instance` was edited by the user.
    pub(crate) fn form_field_edited(&mut self, instance: u64, ix: usize, cx: &mut Context<Self>) {
        if let Some((_, state)) = self.form_mut(instance) {
            state.edited(ix);
            cx.notify();
        }
    }

    pub(crate) fn form_toggle(&mut self, instance: u64, ix: usize, cx: &mut Context<Self>) {
        if let Some((_, state)) = self.form_mut(instance) {
            state.toggle(ix);
            cx.notify();
        }
    }

    pub(crate) fn form_toggle_open(&mut self, instance: u64, ix: usize, cx: &mut Context<Self>) {
        if let Some((_, state)) = self.form_mut(instance) {
            state.toggle_open(ix);
            cx.notify();
        }
    }

    pub(crate) fn form_pick(
        &mut self,
        instance: u64,
        ix: usize,
        option: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some((view, state)) = self.form_mut(instance) {
            state.pick(view, ix, option);
            cx.notify();
        }
    }

    /// A click on a field takes the focus to it.
    pub(crate) fn form_focus(
        &mut self,
        instance: u64,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, state)) = self.form_mut(instance) {
            state.focus_field(ix, window, cx);
        }
    }

    /// Enter in a form, where no text field takes it: toggles the focused
    /// checkbox, opens the focused dropdown, or chooses the highlighted
    /// option of the open one.
    fn form_confirm(&mut self, instance: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_form_focus(window);
        let Some((view, state)) = self.form_mut(instance) else {
            return;
        };
        if state.open_dropdown().is_some() {
            state.pick_highlighted(view);
        } else if let Some(ix) = state.focused(window) {
            state.toggle(ix);
            state.toggle_open(ix);
        }
        cx.notify();
    }

    /// Validates form `instance` and sends `FormSubmitted`. Invalid fields
    /// show their error and nothing is sent. Ignored while an action of the
    /// view is pending, so a double submit sends once.
    pub(crate) fn submit_form(&mut self, instance: u64, cx: &mut Context<Self>) {
        let Some((view, state)) = self.form_mut(instance) else {
            return;
        };
        let ids: Vec<String> = view.fields.iter().map(|f| f.id.clone()).collect();
        let values = state.submit_values(cx);
        cx.notify();
        let Some(values) = values else {
            return;
        };
        let Some(entry) = self.stack.get(instance) else {
            return;
        };
        if self.gated(&entry.provider, Some(instance), cx) {
            return;
        }
        let event = ViewEvent::FormSubmitted {
            values: ids.into_iter().zip(values).collect(),
        };
        self.send_view_event(instance, event, RequestKind::Action, cx);
    }

    /// Ctrl+Enter (Cmd+Enter on macOS) in a form, outside a text field.
    fn submit_top_form(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        match self.stack.top() {
            Some(entry) if matches!(entry.state, ViewState::Form(_)) => {
                let instance = entry.instance;
                self.submit_form(instance, cx);
            }
            _ => cx.propagate(),
        }
    }

    /// Whether Enter on a row of `provider` in view instance `view` (`None`:
    /// the root list) waits for a pending action; see [`RequestLedger::action_pending`].
    ///
    /// [`RequestLedger::action_pending`]: crate::stack::RequestLedger::action_pending
    fn gated(&self, provider: &str, view: Option<u64>, cx: &Context<Self>) -> bool {
        let issuer = Issuer {
            window: self.window_number,
            provider,
            view,
        };
        let gated = self
            .ledger
            .borrow()
            .action_pending(issuer, cx.background_executor().now());
        if gated {
            tracing::debug!("Enter ignored: an action is pending");
        }
        gated
    }

    fn confirm_root(&mut self, cx: &mut Context<Self>) {
        let Some(RootHit { hit, held }) = self.hits.get(self.selected) else {
            return;
        };
        if *held {
            // The answer to the previous query; the core no longer knows it.
            tracing::debug!("Enter on a held answer ignored");
            return;
        }
        let Some(action) = hit.item.actions.first() else {
            return;
        };
        let (id, action_id) = (hit.item.id.clone(), action.id.clone());
        self.run_root_action(&id, &action_id, cx);
    }

    /// Runs action `action_id` of root item `id` with the current query.
    /// Returns `false` without running it while an action of the provider is
    /// pending (the Enter gate).
    fn run_root_action(&mut self, id: &ItemId, action_id: &str, cx: &mut Context<Self>) -> bool {
        if self.gated(id.provider(), None, cx) {
            return false;
        }
        let query = self.input.read(cx).value();
        // Returns at once; the reply arrives as a `CoreUpdate`.
        let result = self.core.borrow_mut().request_action(id, action_id, &query);
        match result {
            Ok(request) => self.ledger.borrow_mut().issue(
                request,
                Issuer {
                    window: self.window_number,
                    provider: id.provider(),
                    view: None,
                },
                RequestKind::Action,
                cx.background_executor().now(),
            ),
            Err(e) => {
                tracing::warn!(item = %id.as_str(), error = %e, "action failed");
                self.show_error_toast(e.to_string(), cx);
            }
        }
        true
    }

    fn confirm_row(&mut self, instance: u64, cx: &mut Context<Self>) {
        let Some(entry) = self.stack.get(instance) else {
            return;
        };
        let Some((row_id, actions)) = selected_target(entry) else {
            return;
        };
        let Some(action) = actions.first() else {
            return;
        };
        let (row_id, action_id) = (row_id.to_string(), action.id.clone());
        self.run_row_action(instance, row_id, action_id, cx);
    }

    /// Sends `ActionChosen` for row `row_id` of view `instance`. Returns
    /// `false` without sending it while an action of the view is pending (the
    /// Enter gate); `true` also when the view is gone.
    fn run_row_action(
        &mut self,
        instance: u64,
        row_id: String,
        action_id: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(entry) = self.stack.get(instance) else {
            return true;
        };
        if self.gated(&entry.provider, Some(instance), cx) {
            return false;
        }
        let event = ViewEvent::ActionChosen { row_id, action_id };
        self.send_view_event(instance, event, RequestKind::Action, cx);
        true
    }

    /// The selection the panel and the footer act on: the root item's or the
    /// top list row's actions, with what they target. A held answer is not
    /// one: the core no longer knows it.
    fn selection(&self) -> Option<(PanelTarget, &[Action])> {
        match self.stack.top() {
            Some(entry) => {
                let (row_id, actions) = selected_target(entry)?;
                let target = PanelTarget::Row {
                    instance: entry.instance,
                    row_id: row_id.to_string(),
                };
                Some((target, actions))
            }
            None => {
                let RootHit { hit, held } = self.hits.get(self.selected)?;
                (!held).then(|| {
                    (
                        PanelTarget::Root(hit.item.id.clone()),
                        &hit.item.actions[..],
                    )
                })
            }
        }
        .filter(|(_, actions)| !actions.is_empty())
    }

    /// Ctrl+K: opens the panel over the selection, or closes it. Nothing to
    /// open without a selection or without actions.
    fn toggle_actions(&mut self, _: &ToggleActions, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            self.close_panel(window, cx);
            return;
        }
        self.open_panel(window, cx);
    }

    /// Opens the panel over the selection; nothing without a selection.
    fn open_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, actions)) = self.selection() else {
            return;
        };
        let actions = actions.to_vec();
        self.panel = Some(ActionPanel::new(target, actions, window, cx));
        cx.notify();
    }

    /// Closes the panel and gives the focus back to the view below it.
    fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = None;
        match self.stack.top() {
            Some(entry) => entry.state.focus(window, cx),
            None => self.input.update(cx, |input, cx| input.focus(window, cx)),
        }
        cx.notify();
    }

    /// The panel's filter text changed.
    pub(crate) fn action_filter_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.panel {
            let query = panel.query(cx);
            panel.refilter(&query);
            cx.notify();
        }
    }

    /// Enter in the panel: runs the selected action on the panel's target
    /// and closes the panel. While an action of the same view is pending the
    /// Enter is ignored and the panel stays.
    fn confirm_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = &self.panel else {
            return;
        };
        let Some(action) = panel.selected_action() else {
            return;
        };
        let action_id = action.id.clone();
        let sent = match panel.target.clone() {
            PanelTarget::Root(id) => self.run_root_action(&id, &action_id, cx),
            PanelTarget::Row { instance, row_id } => {
                self.run_row_action(instance, row_id, action_id, cx)
            }
        };
        if sent {
            self.close_panel(window, cx);
        }
    }

    /// Esc: closes the panel; else back to the view below, or hide on the
    /// root list.
    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_form_focus(window);
        if self.panel.is_some() {
            self.close_panel(window, cx);
        } else if let Some((_, state)) = self
            .stack
            .top()
            .map(|e| e.instance)
            .and_then(|instance| self.form_mut(instance))
            && state.open_dropdown().is_some()
        {
            // Esc closes an open dropdown first.
            state.close_dropdowns();
            cx.notify();
        } else if self.stack.depth() > 1 {
            self.pop_view(window, cx);
        } else {
            cx.defer(hide_launcher);
        }
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

    /// The root list below the header: section labels and hits.
    fn render_root(&self, cx: &mut Context<Self>) -> AnyElement {
        uniform_list(
            "launcher-hits",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let palette = theme::palette(cx);
                range
                    .filter_map(|ix| match *this.rows.get(ix)? {
                        RootRow::Label(text) => {
                            Some(views::section_label(("section", ix).into(), text, &palette))
                        }
                        RootRow::Hit(hit) => {
                            let item = &this.hits.get(hit)?.hit.item;
                            let kind = Accessory::Text(kind_label(item.kind).to_string());
                            Some(views::row(
                                ("hit", ix).into(),
                                &item.title,
                                item.subtitle.as_deref(),
                                item.icon.as_ref(),
                                &[kind],
                                hit == this.selected,
                                &palette,
                            ))
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .into_any_element()
    }

    /// The header content of the root: the borderless search input.
    fn root_header_content(&self) -> AnyElement {
        chrome::header_input(&self.input, "launcher-input", "Search")
    }

    /// The header content of a pushed view: the filter input of a list view,
    /// otherwise the view's title.
    fn view_header_content(entry: &StackEntry) -> AnyElement {
        if let (View::List(view), ViewState::List(state)) = (&entry.view, &entry.state) {
            return views::list::filter_input(view, state);
        }
        div()
            .truncate()
            .text_size(px(theme::HEADER_TEXT_SIZE))
            .child(view_title(&entry.view).to_string())
            .into_any_element()
    }
}

/// The row, tile or detail view Enter and the action panel act on in a pushed
/// view: its id and its actions.
fn selected_target(entry: &StackEntry) -> Option<(&str, &[Action])> {
    match (&entry.view, &entry.state) {
        (View::List(view), ViewState::List(state)) => {
            let row = state.selected_row(view)?;
            Some((&row.id, &row.actions))
        }
        (View::Grid(view), ViewState::Grid(state)) => {
            let tile = state.selected_tile(view)?;
            Some((&tile.id, &tile.actions))
        }
        // A detail view has no rows: its actions act on the view itself, and
        // the row id of its events is the view's id.
        (View::Detail(view), ViewState::Detail(_)) => Some((&view.id.0, &view.actions)),
        _ => None,
    }
}

fn view_title(view: &View) -> &str {
    match view {
        View::List(v) => &v.title,
        View::Grid(v) => &v.title,
        View::Detail(v) => &v.title,
        View::Form(v) => &v.title,
        View::Scratchpad(v) => &v.title,
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure_show(window);
        self.sync_form_focus(window);
        if self.dev_actions && self.panel.is_none() {
            self.open_panel(window, cx);
        }
        let palette = theme::palette(cx);
        let primary = self
            .selection()
            .and_then(|(_, actions)| actions.first().map(|a| a.title.clone()));
        let (header, context, body) = match self.stack.top() {
            None => {
                let header = chrome::header(None, self.root_header_content(), cx);
                (header, "tomakast", self.render_root(cx))
            }
            Some(entry) => {
                let body = match (&entry.view, &entry.state) {
                    (View::List(view), ViewState::List(state)) => {
                        views::list::render(entry.instance, view, state, cx)
                    }
                    (View::Grid(view), ViewState::Grid(state)) => {
                        views::grid::render(view, state, cx)
                    }
                    (View::Detail(view), ViewState::Detail(state)) => {
                        views::detail::render(view, state, cx)
                    }
                    (View::Form(view), ViewState::Form(state)) => {
                        views::form::render(entry.instance, view, state, cx)
                    }
                    (View::Scratchpad(view), ViewState::Scratchpad(state)) => {
                        views::scratchpad::render(entry.instance, view, state, cx)
                    }
                    // A view's state is made for its kind.
                    _ => div().into_any_element(),
                };
                let title = view_title(&entry.view);
                let header = chrome::header(Some(title), Self::view_header_content(entry), cx);
                (header, title, body)
            }
        };
        let toast_showing = self.toasts.read(cx).is_showing();
        let left = chrome::footer_left(context, toast_showing);

        div()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::toggle_actions))
            .on_action(cx.listener(Self::submit_top_form))
            .size_full()
            // Transparent margin that holds the panel's shadow.
            .p(px(theme::shadow_margin()))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .rounded(px(theme::PANEL_RADIUS))
                    .overflow_hidden()
                    .bg(palette.panel)
                    .border_1()
                    .border_color(palette.border)
                    .shadow(palette.shadow.to_vec())
                    .text_color(palette.text)
                    .child(header)
                    // No padding: rows and labels bring their own gutter.
                    .child(div().flex_1().min_h_0().flex().flex_col().child(body))
                    .child(chrome::footer(
                        left,
                        &self.toasts,
                        primary.as_deref(),
                        &palette,
                    ))
                    .children(self.panel.as_ref().map(|panel| panel.render(&palette))),
            )
    }
}
