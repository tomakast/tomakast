//! The launcher view: the root search list, with module-owned views pushed
//! above it on a [`ViewStack`].

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyContext, KeyDownEvent, ParentElement as _, Pixels,
    Render, ScrollDelta, ScrollStrategy, Styled as _, Subscription, Task, TestSupportExt as _,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use tomakast_core::hotkeys::{HotkeyOrigin, HotkeyTarget};
use tomakast_core::{
    Accessory, Action, Core, CoreUpdate, DYNAMIC_SEARCH_DEADLINE, FormView, Hit, ItemId, LocalNow,
    Pattern, SearchMode, SearchSnapshot, TileKey, Toast, ToastKind, View, ViewEvent,
};

use crate::chrome;
use crate::hotkeys::{self, SharedHotkeys};
use crate::icons::IconSpec;
use crate::markdown::{LinkOpener, LinkPolicy, PolicyOpener};
use crate::root_sections::{RootRow, kind_label, root_rows};
use crate::shortcut;
use crate::stack::{Issuer, RequestKind, SharedLedger, StackEntry, Updated, ViewStack};
use crate::strip::keys::{Focus, Key, Outcome};
use crate::strip::{self, CanvasPaintBudget, StripState};
use crate::theme;
use crate::toast::ToastBar;
use crate::views::action_panel::{ActionPanel, PanelEntry, PanelTarget};
use crate::views::form::FormState;
use crate::views::hotkey_recorder::{HotkeyRecorder, RecorderStep};
use crate::views::scratchpad::TEXT_DEBOUNCE;
use crate::views::{self, ViewState};
use crate::window::LauncherController;
use crate::window::hide_launcher;
use home_edit::HomeEditState;
use strip_input::PendingKey;

mod home_edit;
mod panel_input;
mod strip_input;
mod toolbar;

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
        SubmitForm,
        OpenSettings,
        StripNext,
        StripPrev
    ]
);

/// Key context of the launcher view.
pub(crate) const KEY_CONTEXT: &str = "Launcher";
/// Key context of the launcher view on the root list with no panel open:
/// Tab and Shift+Tab move through the widget strip there.
pub(crate) const ROOT_KEY_CONTEXT: &str = "LauncherRoot";
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

/// A query recalled from the history into the root field by up/down.
struct Recall {
    /// How far back in the history it is, 0 being the newest entry.
    back: usize,
    /// What was put in the field; an edit makes the field differ from it.
    text: String,
}

pub struct LauncherView {
    core: Rc<RefCell<Core>>,
    ledger: SharedLedger,
    /// This window's number in the ledger.
    window_number: u64,
    /// Whether the links in a module's views open.
    links: Rc<dyn LinkPolicy>,
    input: Entity<InputState>,
    /// Search generation of `snapshot`; results of other generations are ignored.
    generation: u64,
    /// The core's latest hits for the current query.
    snapshot: Vec<Hit>,
    /// A dynamic source is still getting ready to answer the query (the file index is being
    /// built): the root list shows the busy line.
    root_busy: bool,
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
    /// The recalled query shown in the root field and not edited since.
    recall: Option<Recall>,
    scroll: UniformListScrollHandle,
    /// The widget strip above the root list.
    strip: StripState,
    /// The strip's own focus: a focused canvas has it, and takes the keys.
    strip_focus: FocusHandle,
    /// The Enter or Ctrl+K that waits for the tiles of the search in flight.
    pending: Option<PendingKey>,
    stack: ViewStack,
    /// The open action panel; it has the focus and the keys while open.
    panel: Option<ActionPanel>,
    /// Since when the view on top has been busy, for the moving line under the header.
    busy_since: std::cell::Cell<Option<std::time::Instant>>,
    /// When the pad on top stopped being busy, and where the busy line's glow was then.
    busy_end: std::cell::Cell<Option<(std::time::Instant, f32)>>,
    /// The toolbar button whose open menu a press on it just closed: the click that ends
    /// that press must not open the menu again.
    menu_closed_by: Option<(u64, String)>,
    /// Item hotkeys: the table shown on rows and where the recorder sends changes.
    hotkeys: SharedHotkeys,
    /// The open hotkey recorder, in place of the panel; it has the focus and every key.
    recorder: Option<HotkeyRecorder>,
    /// The Home edit mode, while it lasts.
    home_edit: Option<HomeEditState>,
    /// Dev-only: keep the panel open (see the crate docs).
    dev_actions: bool,
    toasts: Entity<ToastBar>,
    /// When the show request was received (hotkey activation or IPC, via
    /// `show_at`), taken by the first render to log the time to the first frame.
    requested_at: Option<Instant>,
    /// Deactivation hides the window only after it has been active, so a
    /// window that never takes the keyboard does not close itself.
    was_active: bool,
    /// The core heard the hide (see [`close_all`](Self::close_all)); a hide
    /// the busy core missed leaves it unset, so the release retries it.
    closed: bool,
    /// For as long as the view lives.
    _subscriptions: Vec<Subscription>,
    /// For the window it is in now: its search field's edits, its activation and appearance.
    /// Made again when a kept screen opens in a new window.
    _window_subscriptions: Vec<Subscription>,
}

impl LauncherView {
    /// Builds the view of a newly opened window: an empty query, focused, with
    /// the results of the empty query. Must not touch the `LauncherController`
    /// global, which is leased while the window opens.
    // The window's shared handles; a struct of them would only be unpacked here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        core: Rc<RefCell<Core>>,
        ledger: SharedLedger,
        window_number: u64,
        links: Rc<dyn LinkPolicy>,
        hotkeys: SharedHotkeys,
        requested_at: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LauncherView {
        // GPUI also runs appearance observers once right after the window
        // opens, but only after the first frame is drawn; syncing here makes
        // that first frame use the right theme. The tiles' first renders use
        // it too: the env is set before the first search.
        theme::sync(window, cx);
        let mut strip_state = StripState::default();
        strip::send_env(&mut strip_state, &mut core.borrow_mut(), window, cx);
        let dev_query = std::env::var(DEV_QUERY_ENV).unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(PLACEHOLDER)
                .default_value(dev_query)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let window_subscriptions = Self::watch_window(&input, window, cx);
        // A window closed without `hide` (by the compositor) still closes its views, when
        // the screen is not kept; after `hide` the stack is empty already.
        let subscriptions = vec![cx.on_release(|this, cx| this.close_all(cx))];
        let mut view = LauncherView {
            core,
            ledger,
            window_number,
            links,
            input,
            generation: 0,
            snapshot: Vec::new(),
            root_busy: false,
            held: Vec::new(),
            held_timer: None,
            hits: Vec::new(),
            query_empty: true,
            rows: Vec::new(),
            selected: 0,
            recall: None,
            scroll: UniformListScrollHandle::new(),
            strip: strip_state,
            strip_focus: cx.focus_handle(),
            pending: None,
            stack: ViewStack::default(),
            panel: None,
            menu_closed_by: None,
            busy_since: Default::default(),
            busy_end: Default::default(),
            hotkeys,
            recorder: None,
            home_edit: None,
            dev_actions: views::action_panel::dev_actions(),
            toasts: cx.new(|_| ToastBar::default()),
            requested_at: Some(requested_at),
            was_active: window.is_window_active(),
            closed: false,
            _subscriptions: subscriptions,
            _window_subscriptions: window_subscriptions,
        };
        view.search(cx);
        let notices = std::mem::take(&mut view.hotkeys.borrow_mut().notices);
        for notice in notices {
            view.show_error_toast(notice, cx);
        }
        view
    }

    /// Listens to the root search field `input` and to `window`.
    fn watch_window(
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![
            cx.subscribe(input, |this, _, event: &InputEvent, cx| {
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
            cx.observe_window_appearance(window, |this, window, cx| {
                theme::sync(window, cx);
                match this.core.try_borrow_mut() {
                    Ok(mut core) => strip::send_env(&mut this.strip, &mut core, window, cx),
                    // The next frame's tick sends it.
                    Err(_) => this.strip.env_scale = None,
                }
            }),
        ]
    }

    /// The window goes, and the screen is kept for the next show (pop to root): what lives
    /// only while it is shown goes (the action panel, the hotkey recorder, the Home edit
    /// mode, a key waiting for tiles), the pushed views stay, and their modules hear only a
    /// scratchpad's unsent text; the tiles are told they are hidden. Needs no window: the
    /// compositor may have closed it.
    pub(crate) fn detach(&mut self, cx: &mut Context<Self>) {
        self.panel = None;
        self.recorder = None;
        self.pending = None;
        self.menu_closed_by = None;
        self.forget_home_edit();
        let instances: Vec<u64> = self.stack.instances().collect();
        for instance in instances {
            if let Some(event) = self.unsent_text(instance, cx)
                && let Err(e) = self.issue_view_event(instance, event, RequestKind::Query, cx)
            {
                tracing::debug!(error = %e, "unsent text of a kept view not delivered");
            }
        }
        // Answers held for a query of this window: the next show searches again.
        self.held.clear();
        self.held_timer = None;
        if !self.closed {
            match self.core.try_borrow_mut() {
                Ok(mut core) => {
                    core.widgets_hidden();
                    self.closed = true;
                }
                Err(_) => tracing::warn!("the core is busy; tiles not told they are hidden"),
            }
        }
    }

    /// The kept screen opens in `window`: its search fields are made again there with their
    /// text and selection, the window is listened to, the theme and the tiles' surroundings
    /// are the window's, and the focus goes where it was (the top view, else the root
    /// field). [`search_again`](Self::search_again) then brings the root list up to date.
    pub(crate) fn attach(
        &mut self,
        requested_at: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        theme::sync(window, cx);
        strip::send_env(&mut self.strip, &mut self.core.borrow_mut(), window, cx);
        let input = views::rehome::input(&self.input, |s| s.placeholder(PLACEHOLDER), window, cx);
        self._window_subscriptions = Self::watch_window(&input, window, cx);
        self.input = input;
        for entry in self.stack.entries_mut() {
            entry.state.rehome(entry.instance, &entry.view, window, cx);
        }
        match self.stack.top() {
            Some(top) => top.state.focus(window, cx),
            None => self.input.update(cx, |input, cx| input.focus(window, cx)),
        }
        self.requested_at = Some(requested_at);
        self.was_active = window.is_window_active();
        self.closed = false;
        let notices = std::mem::take(&mut self.hotkeys.borrow_mut().notices);
        for notice in notices {
            self.show_error_toast(notice, cx);
        }
        cx.notify();
    }

    /// Searches the root query again on a kept screen, keeping the selected row selected
    /// when it is still there: what the core has may have changed while it was hidden.
    pub(crate) fn search_again(&mut self, cx: &mut Context<Self>) {
        let selected = self.hits.get(self.selected).map(|h| h.hit.item.id.clone());
        self.search(cx);
        self.reselect(selected);
        cx.notify();
    }

    /// Whether the hotkey recorder is open.
    pub(crate) fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// A tomakast hotkey fired while the recorder was open: shown as if pressed in it.
    pub(crate) fn recorded_press(&mut self, hotkey: tomakast_core::Hotkey, cx: &mut Context<Self>) {
        let table = self.hotkeys.borrow().table.clone();
        let core = self.core.clone();
        if let Some(recorder) = &mut self.recorder {
            recorder.record(hotkey, &table, &|item| title_of(&core, item));
            cx.notify();
        }
    }

    /// The leader went down (`true`) or up while the recorder was open; `false` when it is
    /// not open.
    pub(crate) fn recorded_leader(&mut self, pressed: bool, cx: &mut Context<Self>) -> bool {
        let table = self.hotkeys.borrow().table.clone();
        let Some(recorder) = &mut self.recorder else {
            return false;
        };
        if pressed {
            recorder.leader_pressed();
        } else {
            recorder.leader_released(&table);
        }
        cx.notify();
        true
    }

    /// A key-down while the recorder is open.
    fn recorder_key(
        &mut self,
        keystroke: &gpui_kit::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let table = self.hotkeys.borrow().table.clone();
        let core = self.core.clone();
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        let step = recorder.press(keystroke, &table, &|item| title_of(&core, item));
        let item = recorder.item.clone();
        let requests = self.hotkeys.borrow().requests.clone();
        match step {
            RecorderStep::Shown | RecorderStep::Ignored => {}
            RecorderStep::Cancel => {
                self.close_recorder(window, cx);
                self.open_panel(window, cx);
            }
            RecorderStep::Save(hotkey) => {
                self.close_recorder(window, cx);
                if let Some(requests) = requests {
                    requests.assign(
                        HotkeyTarget::Item(item),
                        Some(hotkey),
                        HotkeyOrigin::Launcher,
                    );
                }
            }
            RecorderStep::Clear => {
                self.close_recorder(window, cx);
                if let Some(requests) = requests {
                    requests.assign(HotkeyTarget::Item(item), None, HotkeyOrigin::Launcher);
                }
            }
        }
        cx.notify();
    }

    fn close_recorder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recorder = None;
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
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

    /// For tests: the ids of the pushed views, bottom to top.
    #[doc(hidden)]
    pub fn pushed_view_ids(&self) -> Vec<String> {
        self.stack.ids().map(|id| id.0.clone()).collect()
    }

    /// For tests: the selection of the root search field, as byte offsets.
    #[doc(hidden)]
    pub fn query_selection(&self, cx: &App) -> std::ops::Range<usize> {
        self.input.read(cx).selected_range()
    }

    /// For tests: the keys of the strip's tiles as last drawn, in order.
    #[doc(hidden)]
    pub fn strip_tile_keys(&self) -> Vec<TileKey> {
        self.strip
            .tiles
            .iter()
            .map(|tile| tile.key.clone())
            .collect()
    }

    /// For tests: the title of the selected root hit.
    #[doc(hidden)]
    pub fn root_selection(&self) -> Option<String> {
        self.hits
            .get(self.selected)
            .map(|h| h.hit.item.title.clone())
    }

    /// Whether pushed view `instance` is still on the stack.
    pub(crate) fn has_view(&self, instance: u64) -> bool {
        self.stack.contains(instance)
    }

    /// The module and module instance that pushed the top view; `None` on the
    /// root list.
    pub(crate) fn top_owner(&self) -> Option<(&str, u64)> {
        self.stack
            .top()
            .map(|entry| (entry.provider.as_str(), entry.module_instance))
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
        self.root_busy = snapshot.busy;
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
        let query = self.input.read(cx).value();
        // A user edit is the only change that makes the field differ from the
        // recalled text: the edited query is a new one.
        if self.recall.as_ref().is_some_and(|r| *r.text != *query) {
            self.recall = None;
        }
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
        // As the engine decides: a query of blanks is empty too.
        self.query_empty = Pattern::parse(&query).is_empty();
        // A new query: the selection is back on the list's first row, a tile
        // answered for the last one is never acted on, and a waiting Enter
        // goes. A selected tile keeps its selection while its new answer is
        // on the way, so the highlight does not jump to the list and back;
        // it takes no action meanwhile. An empty query keeps no tile
        // selected.
        if self.query_empty || !matches!(self.strip.focus, Focus::TileSelected(_)) {
            self.strip.focus = Focus::List;
        }
        self.strip.moved = false;
        self.pending = None;
        let snapshot = self.core.borrow_mut().search(&query, LocalNow::now());
        self.generation = snapshot.generation;
        self.snapshot = snapshot.hits;
        self.root_busy = snapshot.busy;
        self.merge_hits();
        self.selected = self.first_row();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// The first listed row, but not a held answer: Enter on it is ignored,
    /// and a quick "fir" + Enter must run the first real match.
    fn first_row(&self) -> usize {
        self.rows
            .iter()
            .filter_map(|row| row.hit())
            .find(|&ix| !self.hits[ix].held)
            .unwrap_or(0)
    }

    /// Selects the hit with id `id` when it is still listed; otherwise the
    /// first listed row. In the field, always its first row.
    fn reselect(&mut self, id: Option<ItemId>) {
        if self.strip.focus == Focus::Field {
            self.selected = self.first_row();
            return;
        }
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

    /// Pushes a module-owned view; it takes the focus. A push onto a full stack
    /// ([`crate::stack::MAX_PUSHED_VIEWS`]) is dropped with a warning.
    ///
    /// Ruling 9: a view whose id `provider` already has on the stack is not stacked
    /// twice (an unsolicited push does so only over `provider`'s own views, Ruling 9b,
    /// checked by the caller). The push returns to that entry: the views above it are popped (each
    /// gets its pending text and `Closed`, as for Esc), and the entry is replaced by
    /// the pushed view as a fresh push (new instance, revision and state), so replies
    /// to the old content are dropped. A copy left below could otherwise send its
    /// stale text back to the module later.
    pub(crate) fn push_view(
        &mut self,
        provider: &str,
        module_instance: u64,
        view: View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A view over the root ends the Home edit mode, whose keys it would take.
        self.leave_home_edit(window, cx);
        if let Some(depth) = self.stack.position(provider, view.id()) {
            while self.stack.len() > depth + 1 {
                self.pop_view(window, cx);
            }
            if let Some(old) = self.stack.top().map(|entry| entry.instance) {
                // An edit still waiting for its debounce goes to the module first.
                if let Some(event) = self.unsent_text(old, cx)
                    && let Err(e) = self.issue_view_event(old, event, RequestKind::Query, cx)
                {
                    tracing::debug!(error = %e, "pending text of a replaced view not delivered");
                }
            }
            self.stack.pop();
        } else if self.stack.is_full() {
            tracing::warn!(module = provider, "view stack is full; push dropped");
            return;
        }
        let revision = match &view {
            View::List(list) => list.revision,
            View::Scratchpad(pad) => pad.revision,
            View::Grid(_) | View::Detail(_) | View::Form(_) => 0,
        };
        // Once per push, not per update: unsolicited updates come often.
        shortcut::warn_unusable(provider, &view);
        // The new view takes the focus from the panel.
        self.panel = None;
        let instance = self.stack.new_instance();
        let state = ViewState::new(instance, &view, window, cx);
        self.stack.push(StackEntry {
            instance,
            provider: provider.to_string(),
            module_instance,
            view,
            revision,
            state,
        });
        cx.notify();
    }

    /// Applies an `UpdateView` from `owner` (provider, module instance;
    /// revision rule in [`ViewStack::update`]). A reply of view `instance`
    /// updates that instance only; a root-action reply (`None`) updates the
    /// topmost view with the same id that this module instance pushed. The
    /// selected row stays selected by id.
    pub(crate) fn update_view(
        &mut self,
        owner: (&str, u64),
        instance: Option<u64>,
        view: View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (provider, module_instance) = owner;
        let id = view.id();
        let target = match instance {
            Some(instance) => self.stack.get(instance).filter(|e| {
                e.provider == provider && e.module_instance == module_instance && e.view.id() == id
            }),
            None => self.stack.find(provider, module_instance, id),
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
                    state.reindex(view);
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

    /// Pops the top view, after sending it `Closed`, and gives the focus to
    /// the one below.
    pub(crate) fn pop_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(top) = self.stack.top().map(|entry| entry.instance) else {
            tracing::debug!("PopView on the root list ignored");
            return;
        };
        self.close_view(top, cx);
        self.stack.pop();
        // The panel belongs to the view that went.
        self.panel = None;
        match self.stack.top() {
            Some(entry) => entry.state.focus(window, cx),
            None => self.input.update(cx, |input, cx| input.focus(window, cx)),
        }
        cx.notify();
    }

    /// A built-in module's `SetRootQuery`: closes the panel and every pushed
    /// view (each gets `Closed`), puts `text` in the root search field with
    /// the caret at its end, and searches. `set_value` emits no `Change`, so
    /// the search is run here.
    pub(crate) fn set_root_query(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A new query ends the Home edit mode, which shows Home at the empty query.
        self.leave_home_edit(window, cx);
        self.panel = None;
        self.recall = None;
        while self.stack.top().is_some() {
            self.pop_view(window, cx);
        }
        self.input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.search(cx);
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

    /// The window is going: closes every pushed view, top to bottom, as
    /// popping them would, and empties the stack; the tiles on screen are told
    /// they are hidden. `hide` calls it before it removes the window; a window
    /// closed another way (by the compositor) calls it when the view is
    /// released. The stack is empty after the first call, so each view is
    /// closed once; the core hears the hide once too, so a release that
    /// comes after the next window searched never drops that one's tiles.
    pub(crate) fn close_all(&mut self, cx: &mut App) {
        self.panel = None;
        self.forget_home_edit();
        while let Some(top) = self.stack.top().map(|entry| entry.instance) {
            self.close_view(top, cx);
            self.stack.pop();
        }
        if self.closed {
            return;
        }
        // Not `borrow_mut`, as in `issue_view_event`: a view released while
        // the core is borrowed must not panic. Only a hide the core heard
        // counts: with the core busy, the next call (the release) tries again.
        match self.core.try_borrow_mut() {
            Ok(mut core) => {
                core.widgets_hidden();
                self.closed = true;
            }
            Err(_) => tracing::warn!("the core is busy; tiles not told they are hidden"),
        }
    }

    /// Tells the module that view `instance`, still on the stack, goes: a
    /// scratchpad's unsent text first, then `Closed`. Its reply finds the
    /// view gone and is dropped. The window may be going, so failures are
    /// logged, not toasted.
    fn close_view(&mut self, instance: u64, cx: &mut App) {
        let events = self.unsent_text(instance, cx).into_iter();
        for event in events.chain([ViewEvent::Closed]) {
            if let Err(e) = self.issue_view_event(instance, event, RequestKind::Query, cx) {
                // The module is gone or replaced; nobody to tell.
                tracing::debug!(error = %e, "view event of a closing view not delivered");
            }
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
            let readings = self.core.borrow().query_readings(&text);
            let event = ViewEvent::QueryChanged {
                revision: entry.revision,
                text,
                readings,
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
        if let Err(e) = self.issue_view_event(instance, event, kind, cx) {
            tracing::warn!(error = %e, "view event failed");
            self.show_error_toast(e, cx);
        }
    }

    /// Sends `event` to the module that owns view `instance` and records it
    /// in the ledger. Nothing is sent once the view is off the stack.
    fn issue_view_event(
        &mut self,
        instance: u64,
        event: ViewEvent,
        kind: RequestKind,
        cx: &App,
    ) -> Result<(), String> {
        let Some(entry) = self.stack.get(instance) else {
            return Ok(());
        };
        // Not `borrow_mut`: a view released while the core is borrowed must
        // not panic.
        let request = self
            .core
            .try_borrow_mut()
            .map_err(|_| "the core is busy".to_string())?
            .send_view_event(
                &entry.provider,
                entry.module_instance,
                entry.view.id(),
                event,
            )
            .map_err(|e| e.to_string())?;
        let issuer = Issuer {
            window: self.window_number,
            provider: &entry.provider,
            view: Some(instance),
        };
        let now = cx.background_executor().now();
        self.ledger.borrow_mut().issue(request, issuer, kind, now);
        Ok(())
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
    /// `instance`, if it has an unsent edit.
    fn send_scratchpad_text(&mut self, instance: u64, cx: &mut Context<Self>) {
        if let Some(event) = self.unsent_text(instance, cx) {
            self.send_view_event(instance, event, RequestKind::Query, cx);
        }
    }

    /// The `TextChanged` of scratchpad `instance`'s unsent edit, if it has
    /// one; the edit counts as sent from now on.
    fn unsent_text(&mut self, instance: u64, cx: &App) -> Option<ViewEvent> {
        let Some(StackEntry {
            revision,
            state: ViewState::Scratchpad(state),
            ..
        }) = self.stack.get_mut(instance)
        else {
            return None;
        };
        state.take_pending().then(|| ViewEvent::TextChanged {
            revision: *revision,
            text: state.text(cx),
        })
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
        if let Some(panel) = &mut self.panel {
            panel.select_prev();
            cx.notify();
            return;
        }
        if !self.recalled_tile() && self.strip_key(Key::Up, "up", window, cx) != Outcome::Unhandled
        {
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
            Some(_) => return,
            None => {
                if self.in_history() {
                    self.recall_older(window, cx);
                } else {
                    self.step_root(false);
                }
            }
        }
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.panel {
            panel.select_next();
            cx.notify();
            return;
        }
        if !self.recalled_tile()
            && self.strip_key(Key::Down, "down", window, cx) != Outcome::Unhandled
        {
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
            Some(_) => return,
            None => {
                if self.in_history() {
                    self.recall_newer(window, cx);
                } else {
                    self.step_root(true);
                }
            }
        }
        cx.notify();
    }

    /// Left (`forward`: Right) on a pushed list drawn as a grid, with no
    /// panel open: the selection moves to the neighbouring tile of its row
    /// (at a row's edge it stays), and the key is taken from the field either
    /// way. Elsewhere `false`.
    fn grid_list_step(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if self.panel.is_some() {
            return false;
        }
        let Some(StackEntry {
            state: ViewState::List(state),
            ..
        }) = self.stack.top_mut()
        else {
            return false;
        };
        if !state.is_grid() {
            return false;
        }
        if forward {
            state.select_right();
        } else {
            state.select_left();
        }
        cx.notify();
        true
    }

    /// Left and right move over a grid and through the widget strip; from
    /// the search field they reach the strip first (see
    /// [`Self::field_left`]). Elsewhere they are not the launcher's: they go
    /// on to the text field that has the focus.
    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.strip_key(Key::Left, "left", window, cx) != Outcome::Unhandled
            || self.grid_list_step(false, cx)
        {
            return;
        }
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

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.strip_key(Key::Right, "right", window, cx) != Outcome::Unhandled
            || self.grid_list_step(true, cx)
        {
            return;
        }
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

    /// The value of the root search field.
    pub(crate) fn root_query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Whether the root selection is the first listed hit, or nothing is listed.
    fn on_first_row(&self) -> bool {
        match self.rows.iter().find_map(|row| row.hit()) {
            Some(first) => first == self.selected,
            None => true,
        }
    }

    /// Whether Up and Down at the root walk the history: the selection is
    /// in the field, or on the tile a recalled query selected.
    fn in_history(&self) -> bool {
        self.strip.focus == Focus::Field || self.recalled_tile()
    }

    /// Up in the root field: back through a recalled query's history, or,
    /// in the empty field, to the newest entry. A typed query has none.
    fn recall_older(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = match &self.recall {
            Some(recall) => recall.back + 1,
            None if self.root_query(cx).is_empty() => 0,
            None => return,
        };
        // The history may have changed since the recall; an index past its end
        // is a no-op.
        let older = self.core.borrow().history().nth_query(back);
        if let Some(text) = older {
            self.show_recalled(back, text, window, cx);
        }
    }

    /// Down in the root field: forward through a recalled query's history,
    /// and past the newest entry to the empty field, where the selection
    /// stays.
    fn recall_newer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Recall { back, .. }) = self.recall else {
            return;
        };
        let newer = back
            .checked_sub(1)
            .and_then(|newer| Some((newer, self.core.borrow().history().nth_query(newer)?)));
        match newer {
            Some((back, text)) => self.show_recalled(back, text, window, cx),
            None => {
                self.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.recall = None;
                self.search(cx);
                self.stay_in_field(window, cx);
            }
        }
    }

    /// After a search run from the field, which put the selection on the
    /// list: it goes back to the field. A selected tile kept for the new
    /// answer keeps it, as while typing.
    fn stay_in_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.strip.focus == Focus::List {
            self.set_strip_focus(Focus::Field, window, cx);
        }
    }

    /// Puts history entry `back` (`text`) in the root field, selected so that
    /// typing replaces it, and searches it. `set_value` emits no `Change`.
    fn show_recalled(
        &mut self,
        back: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.update(cx, |input, cx| {
            input.set_value(text.clone(), window, cx);
            input.select_all(window, cx);
        });
        self.recall = Some(Recall { back, text });
        self.search(cx);
        self.stay_in_field(window, cx);
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
        self.strip.moved = true;
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
            // Enter types a newline in the pad; its actions run from the panel.
            Some(StackEntry {
                state: ViewState::Scratchpad(_),
                ..
            }) => {}
            Some(entry) => self.confirm_row(entry.instance, cx),
            None => self.root_press(Key::Enter, window, cx),
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

    /// Closes the open dropdown list of form `instance`, if any; returns
    /// whether one was open.
    pub(crate) fn form_close_open_dropdown(
        &mut self,
        instance: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.form_mut(instance)
            .is_some_and(|(_, state)| state.close_open_dropdown(window, cx))
    }

    /// The user chose `value` in dropdown `ix` of form `instance`.
    pub(crate) fn form_choose(
        &mut self,
        instance: u64,
        ix: usize,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, state)) = self.form_mut(instance) {
            state.choose(ix, value);
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

    /// Enter in a form, where no text field or dropdown takes it: toggles
    /// the focused checkbox. A dropdown opens and chooses with its own keys.
    fn form_confirm(&mut self, instance: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, state)) = self.form_mut(instance) else {
            return;
        };
        if let Some(ix) = state.focused(window) {
            state.toggle(ix);
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

    /// Ctrl+Enter (Cmd+Enter on macOS): submits a form on top (outside a text field), or
    /// runs the first action of a pad on top.
    fn submit_top_form(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        match self.stack.top() {
            Some(entry) if matches!(entry.state, ViewState::Form(_)) => {
                let instance = entry.instance;
                self.submit_form(instance, cx);
            }
            Some(StackEntry {
                instance,
                view: View::Scratchpad(pad),
                ..
            }) => {
                let (instance, row_id) = (*instance, pad.id.0.clone());
                if let Some(action) = pad.actions.first() {
                    let action_id = action.id.clone();
                    self.run_row_action(instance, row_id, action_id, cx);
                }
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
        // A pad's waiting edit goes first, so the action acts on the text as it is.
        if let Some(edit) = self.unsent_text(instance, cx) {
            self.send_view_event(instance, edit, RequestKind::Query, cx);
        }
        let event = ViewEvent::ActionChosen { row_id, action_id };
        self.send_view_event(instance, event, RequestKind::Action, cx);
        true
    }

    /// The selection the panel and the footer act on: the root item's or the
    /// top list row's actions, with what they target. A held answer is not
    /// one: the core no longer knows it. A strip tile is one even with no
    /// actions of its own: a search tile's panel has the launcher's "Add to
    /// Home", a Home tile's its "Edit Home".
    fn selection(&self) -> Option<(PanelTarget, &[Action])> {
        if let (None, Some(key)) = (self.stack.top(), self.strip.focus.tile()) {
            let tile = self.strip.tiles.iter().find(|tile| &tile.key == key)?;
            let actions = strip::tile_actions(tile);
            let selected = !actions.is_empty()
                || strip::takes_add_to_home(tile)
                || strip::takes_edit_home(tile);
            return selected.then(|| (PanelTarget::Tile(key.clone()), actions));
        }
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

    /// Ctrl+, (Cmd+, on macOS): opens the settings window and hides the launcher, as the
    /// Settings item does.
    fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.defer(|cx| crate::open_settings_from_launcher(None, cx));
    }

    /// Ctrl+K: opens the panel over the selection, or closes it. Nothing to
    /// open without a selection or without actions.
    fn toggle_actions(&mut self, _: &ToggleActions, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            self.close_panel(window, cx);
            return;
        }
        match self.stack.top() {
            None => self.root_press(Key::Actions, window, cx),
            Some(_) => self.open_panel(window, cx),
        }
    }

    /// Opens the panel over the selection, a search tile's with the
    /// launcher's "Add to Home" last, a Home tile's with its "Edit Home";
    /// nothing without a selection.
    fn open_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, actions)) = self.selection() else {
            return;
        };
        let mut entries: Vec<PanelEntry> =
            actions.iter().cloned().map(PanelEntry::Action).collect();
        if let PanelTarget::Tile(key) = &target
            && let Some(tile) = self.strip.tiles.iter().find(|tile| &tile.key == key)
        {
            if strip::takes_add_to_home(tile) {
                entries.push(PanelEntry::AddToHome);
            }
            if strip::takes_edit_home(tile) {
                entries.push(PanelEntry::EditHome);
            }
        }
        if let PanelTarget::Root(item) = &target {
            let hotkeys = self.hotkeys.borrow();
            if hotkeys.requests.is_some() && self.core.borrow().is_indexed(item) {
                entries.push(PanelEntry::Action(Action {
                    id: hotkeys::ASSIGN_HOTKEY.into(),
                    title: hotkeys::entry_title(hotkeys.table.of(item).is_some()).into(),
                    shortcut: None,
                }));
            }
            drop(hotkeys);
            if has_plugin_settings(item.provider(), cx) {
                entries.push(PanelEntry::PluginSettings(item.provider().to_string()));
            }
        }
        let on_pad = matches!(
            &target,
            PanelTarget::Row { instance, .. }
                if matches!(self.stack.get(*instance), Some(StackEntry { view: View::Scratchpad(pad), .. }) if !pad.actions.is_empty())
        );
        let mut panel = ActionPanel::new(target, entries, window, cx);
        if on_pad {
            panel = panel.with_first_keys(shortcut::submit_chips());
        }
        self.panel = Some(panel);
        cx.notify();
    }

    /// Closes the panel and gives the focus back to the view below it, or
    /// to the Home edit mode while it lasts.
    fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = None;
        match (self.home_edit_focus(), self.stack.top()) {
            (Some(focus), _) => window.focus(focus, cx),
            (None, Some(entry)) => entry.state.focus(window, cx),
            (None, None) => self.input.update(cx, |input, cx| input.focus(window, cx)),
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

    /// Enter in the panel: runs the selected entry on the panel's target
    /// and closes the panel. While an action of the same view is pending the
    /// Enter is ignored and the panel stays; the launcher's own entries are
    /// not held back.
    fn confirm_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = &self.panel else {
            return;
        };
        let Some(entry) = panel.selected_entry().cloned() else {
            return;
        };
        let target = panel.target.clone();
        match entry {
            PanelEntry::Action(action) if action.id == hotkeys::ASSIGN_HOTKEY => {
                if let PanelTarget::Root(item) = target {
                    self.open_recorder(item, window, cx);
                }
            }
            PanelEntry::Action(action) => {
                // A pad action named as one of its toolbar buttons runs the button.
                if let Some((instance, ix)) = self.toolbar_button_named(&target, &action.id) {
                    self.close_panel(window, cx);
                    self.toolbar_clicked(instance, ix, window, cx);
                    return;
                }
                if self.run_on(target, action.id, cx) {
                    self.close_panel(window, cx);
                }
            }
            PanelEntry::AddToHome => {
                if let PanelTarget::Tile(key) = target {
                    self.add_to_home(&key, cx);
                }
                self.close_panel(window, cx);
            }
            PanelEntry::EditHome | PanelEntry::Size { .. } | PanelEntry::Remove => {
                self.confirm_home_entry(entry, target, window, cx);
            }
            PanelEntry::PluginSettings(plugin) => {
                self.close_panel(window, cx);
                // As Ctrl+, does, on the plugin's page.
                let page = format!("plugin:{plugin}");
                cx.defer(move |cx| crate::open_settings_from_launcher(Some(page), cx));
            }
            PanelEntry::Choice { option, .. } => {
                if let PanelTarget::Toolbar { instance, button } = target
                    && !self.run_row_action(instance, option.id, button, cx)
                {
                    return;
                }
                self.close_panel(window, cx);
            }
        }
    }

    /// Opens the hotkey recorder over root item `item`, in place of the panel.
    fn open_recorder(&mut self, item: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = None;
        let title = title_of(&self.core, &item);
        let has_hotkey = self.hotkeys.borrow().table.of(&item).is_some();
        let recorder = HotkeyRecorder::new(item, title, has_hotkey, cx);
        recorder.focus.focus(window, cx);
        self.recorder = Some(recorder);
        cx.notify();
    }

    /// Runs action `action_id` on `target`; `false` while the Enter gate
    /// holds it back.
    fn run_on(&mut self, target: PanelTarget, action_id: String, cx: &mut Context<Self>) -> bool {
        match target {
            PanelTarget::Root(id) => self.run_root_action(&id, &action_id, cx),
            PanelTarget::Row { instance, row_id } => {
                self.run_row_action(instance, row_id, action_id, cx)
            }
            PanelTarget::Tile(key) => self.run_tile_action(&key, None, action_id, cx),
            // The mode's menu has no module actions; a toolbar menu has choices only.
            PanelTarget::HomeEdit(_) | PanelTarget::Toolbar { .. } => true,
        }
    }

    /// A key that is the shortcut of one of the selection's actions runs it,
    /// as the panel would; with the panel open, one of the panel's actions.
    /// The launcher's and the text fields' own keys are bound and handled
    /// before this listener sees them. None while the Home edit mode lasts:
    /// the keys are the mode's.
    fn shortcut_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.recorder.is_some() {
            cx.stop_propagation();
            if !event.is_held {
                self.recorder_key(&event.keystroke, window, cx);
            }
            return;
        }
        if event.is_held || self.home_editing() {
            return;
        }
        if self.panel.as_ref().is_none_or(ActionPanel::is_menu)
            && let Some((instance, ix)) = self.toolbar_shortcut(event)
        {
            cx.stop_propagation();
            self.toolbar_clicked(instance, ix, window, cx);
            return;
        }
        let found = match &self.panel {
            Some(panel) => shortcut::find(panel.actions(), &event.keystroke)
                .map(|action| (panel.target.clone(), action.id.clone())),
            None => self.selection().and_then(|(target, actions)| {
                shortcut::find(actions, &event.keystroke).map(|action| (target, action.id.clone()))
            }),
        };
        let Some((target, action_id)) = found else {
            // Typing in the strip, outside a canvas, goes to the field.
            if self.strip.focus.tile().is_some() {
                let name = event.keystroke.key.clone();
                self.strip_key(Key::Other, &name, window, cx);
            }
            return;
        };
        cx.stop_propagation();
        if let Some((instance, ix)) = self.toolbar_button_named(&target, &action_id) {
            self.close_panel(window, cx);
            self.toolbar_clicked(instance, ix, window, cx);
            return;
        }
        if self.run_on(target, action_id, cx) && self.panel.is_some() {
            self.close_panel(window, cx);
        }
    }

    /// The footer's context for a view of `provider` titled `title`: for a plugin's view the
    /// plugin's name alone (Ruling AN: a plugin cannot pass its view off as one of the
    /// launcher's; the header has the title), else the title.
    fn footer_context(&self, provider: &str, title: &str) -> String {
        plugin_context(self.links.plugin_name(provider).as_deref(), title)
    }

    /// Opens the links of `provider`'s views if the link policy allows it,
    /// asked on every click.
    fn link_opener(&self, provider: &str) -> Rc<dyn LinkOpener> {
        Rc::new(PolicyOpener {
            provider: provider.to_string(),
            policy: self.links.clone(),
        })
    }

    /// Esc: closes the panel; else back to the view below, or hide on the
    /// root list.
    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            self.close_panel(window, cx);
        } else if self.strip_key(Key::Escape, "escape", window, cx) != Outcome::Unhandled {
            // Out of a canvas, or out of the strip.
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
                            // Where the item comes from (a plugin's name), else its kind.
                            let source = item
                                .source
                                .clone()
                                .unwrap_or_else(|| kind_label(item.kind).to_string());
                            let kind = Accessory::Text(source);
                            // A plugin's rows share its letter; others show their own.
                            let icon = IconSpec {
                                icon: item.icon.as_ref(),
                                monogram: item.source.as_deref().unwrap_or(&item.title),
                            };
                            let trailing = this
                                .hotkeys
                                .borrow()
                                .table
                                .of(&item.id)
                                .map(|h| hotkeys::chips(ix, h, &palette));
                            Some(views::row(
                                ("hit", ix).into(),
                                &item.title,
                                item.subtitle.as_deref(),
                                icon,
                                &[kind],
                                trailing,
                                // Not while the keyboard is on a tile.
                                hit == this.selected && this.strip.focus == Focus::List,
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

    /// The header content of the root: the borderless search input, dimmed
    /// while the Home edit mode lasts.
    fn root_header_content(&self) -> AnyElement {
        let input = chrome::header_input(&self.input, "launcher-input", "Search");
        match self.home_editing() {
            true => div()
                .opacity(theme::EDIT_DIM_OPACITY)
                .child(input)
                .into_any_element(),
            false => input,
        }
    }

    /// Whether pushed view `entry` is busy: a busy pad or list.
    fn view_busy(entry: &StackEntry) -> bool {
        match &entry.view {
            View::Scratchpad(pad) => pad.busy,
            View::List(list) => list.busy,
            _ => false,
        }
    }

    /// The busy line of the header on screen (`busy`: the root list or the pushed view is
    /// busy), moving (the next frame is asked for) unless the desktop asks for reduced
    /// motion. When the work stops, the sweep hurries from where it is to the right edge,
    /// then the whole line lights up and fades: the work ends with a full stop, never
    /// halfway. Timed by the executor's clock.
    fn busy_line(&self, busy: bool, window: &mut Window, cx: &App) -> Option<chrome::Busy> {
        if self.strip.reduced_motion {
            self.busy_since.set(None);
            self.busy_end.set(None);
            return busy.then_some(chrome::Busy::Still);
        }
        let now = cx.background_executor().now();
        let period = chrome::BUSY_PERIOD.as_secs_f32();
        // Eased in and out: where the glow is at `elapsed` into the sweeps.
        let swept = |elapsed: std::time::Duration| {
            let phase = elapsed.as_secs_f32() % period / period;
            phase * phase * (3. - 2. * phase)
        };
        let since = match (self.busy_since.get(), busy) {
            (Some(since), _) => since,
            (None, true) => now,
            (None, false) => return None,
        };
        window.request_animation_frame();
        if busy {
            self.busy_since.set(Some(since));
            self.busy_end.set(None);
            return Some(chrome::Busy::Moving(swept(
                now.saturating_duration_since(since),
            )));
        }
        // Done: from where the glow was when the work ended.
        let (ended, from) = self
            .busy_end
            .get()
            .unwrap_or_else(|| (now, swept(now.saturating_duration_since(since))));
        self.busy_end.set(Some((ended, from)));
        let after = now.saturating_duration_since(ended);
        let (finish, flash) = (chrome::BUSY_FINISH, chrome::BUSY_FLASH);
        if after < finish {
            // Eased out: it hurries, then settles at the edge.
            let t = after.as_secs_f32() / finish.as_secs_f32();
            let out = 1. - (1. - t).powi(3);
            return Some(chrome::Busy::Finishing { from, t: out });
        }
        if after < finish + flash {
            let t = (after - finish).as_secs_f32() / flash.as_secs_f32();
            return Some(chrome::Busy::Done(1. - t));
        }
        self.busy_since.set(None);
        self.busy_end.set(None);
        None
    }

    /// The header content of a pushed view: the filter input of a list view,
    /// otherwise the view's title.
    fn view_header_content(entry: &StackEntry) -> AnyElement {
        if let (View::List(view), ViewState::List(state)) = (&entry.view, &entry.state) {
            return views::list::filter_input(view, state);
        }
        div()
            .truncate()
            .text_size(theme::text(theme::HEADER_TEXT_SIZE))
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
        // A detail view and a scratchpad have no rows: their actions act on
        // the view itself, and the row id of their events is the view's id.
        (View::Detail(view), ViewState::Detail(_)) => Some((&view.id.0, &view.actions)),
        (View::Scratchpad(view), ViewState::Scratchpad(_)) => Some((&view.id.0, &view.actions)),
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

/// "<plugin> · <title>" for a plugin's view, the title alone otherwise.
fn plugin_context(plugin: Option<&str>, title: &str) -> String {
    plugin.unwrap_or(title).to_owned()
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure_show(window);
        // The kit's rem-sized text (inputs, markdown headings) follows the user's text size,
        // as the settings window's Root does from the theme.
        window.set_rem_size(gpui_kit::component::ActiveTheme::theme(&**cx).font_size);
        if self.dev_actions && self.panel.is_none() && !self.home_editing() {
            self.open_panel(window, cx);
        }
        let palette = theme::palette(cx);
        // Enter does not run a scratchpad's first action: no "⏎" hint for it.
        let on_pad = matches!(
            self.stack.top(),
            Some(StackEntry {
                state: ViewState::Scratchpad(_),
                ..
            })
        );
        // A selected tile answered for an older query shows no hint: Enter
        // waits for its new answer and runs that tile's action if it lands in
        // time, else the list row's, so neither is sure until then.
        let selection = self.selection().filter(|_| !on_pad);
        // Ctrl+K opens a panel whenever there is a selection: a strip tile
        // without actions of its own has the launcher's "Add to Home" or
        // "Edit Home". The Home edit mode has a footer of its own.
        let mut has_panel = selection.is_some();
        let mut primary = selection
            .as_ref()
            .and_then(|(_, actions)| actions.first().map(|a| a.title.clone()));
        // A pad: its first action runs on Ctrl+Enter (Cmd+Enter), and Ctrl+K has them all.
        let mut primary_keys = Vec::new();
        if let Some(StackEntry {
            view: View::Scratchpad(pad),
            ..
        }) = self.stack.top()
            && let Some(first) = pad.actions.first()
        {
            has_panel = true;
            primary = Some(first.title.clone());
            primary_keys = shortcut::submit_chips();
        }
        // A list's footer action: its title and keys while the selected row has it.
        let extra = match self.stack.top() {
            Some(StackEntry {
                view: View::List(list),
                ..
            }) => list.footer_action.as_deref().and_then(|id| {
                let (_, actions) = selection.as_ref()?;
                let action = actions.iter().find(|a| a.id == id)?;
                let hotkey = shortcut::usable(action)?;
                Some((action.title.clone(), shortcut::chips(&hotkey)))
            }),
            _ => None,
        };
        // The frame clock: a frame drawn while the core is borrowed (a window
        // update inside an apply) shows the last tick's tiles and ticks on the
        // next frame.
        let now = cx.background_executor().now();
        let budget = cx
            .try_global::<CanvasPaintBudget>()
            .copied()
            .unwrap_or_default();
        let tiles = match self.core.try_borrow_mut() {
            Ok(mut core) => {
                strip::sync_scale(&mut self.strip, &mut core, window, cx);
                let root = self.stack.top().is_none();
                let ticked = strip::tick(&mut self.strip, &mut core, root, now, &budget, cx);
                if ticked.wants_frame {
                    window.request_animation_frame();
                }
                for (provider, request) in ticked.requests {
                    let issuer = Issuer {
                        window: self.window_number,
                        provider: &provider,
                        view: None,
                    };
                    self.ledger
                        .borrow_mut()
                        .issue(request, issuer, RequestKind::Query, now);
                }
                // Shown as any stopped widget is, once this frame is drawn.
                for update in ticked.stopped {
                    cx.defer(move |cx| {
                        if cx.has_global::<LauncherController>() {
                            gpui_kit::BorrowAppContext::update_global::<LauncherController, _>(
                                cx,
                                |c, cx| c.core_update(update, cx),
                            );
                        }
                    });
                }
                ticked.tiles
            }
            Err(_) => {
                // Not judged: a later paint is not added to these.
                self.strip.paints.take();
                window.request_animation_frame();
                self.strip.tiles.clone()
            }
        };
        self.sync_strip_focus(window, cx);
        let edit = self.home_edit_frame(now, window);
        let (header, mut context, body) = match self.stack.top() {
            None => {
                let header = div()
                    .id("launcher-header")
                    .test_support()
                    .flex_none()
                    .child(chrome::header(
                        None,
                        self.root_header_content(),
                        self.busy_line(self.root_busy, window, cx),
                        cx,
                    ))
                    .into_any_element();
                let list = self.render_root(cx);
                let focus = self.home_edit_focus().unwrap_or(&self.strip_focus);
                let body = strip::root_body(
                    &self.strip,
                    &tiles,
                    &palette,
                    list,
                    focus,
                    edit.as_ref(),
                    cx,
                );
                (header, "tomakast".to_owned(), body)
            }
            Some(entry) => {
                let body = match (&entry.view, &entry.state) {
                    (View::List(view), ViewState::List(state)) => {
                        let links = self.link_opener(&entry.provider);
                        views::list::render(entry.instance, view, state, links, cx)
                    }
                    (View::Grid(view), ViewState::Grid(state)) => {
                        views::grid::render(view, state, cx)
                    }
                    (View::Detail(view), ViewState::Detail(state)) => {
                        views::detail::render(view, state, self.link_opener(&entry.provider), cx)
                    }
                    (View::Form(view), ViewState::Form(state)) => {
                        views::form::render(entry.instance, view, state, window, cx)
                    }
                    (View::Scratchpad(view), ViewState::Scratchpad(state)) => {
                        let links = self.link_opener(&entry.provider);
                        views::scratchpad::render(entry.instance, view, state, links, window, cx)
                    }
                    // A view's state is made for its kind.
                    _ => div().into_any_element(),
                };
                let title = view_title(&entry.view);
                let busy = self.busy_line(Self::view_busy(entry), window, cx);
                let content = match (&entry.view, &entry.state) {
                    (View::Scratchpad(pad), ViewState::Scratchpad(state)) => {
                        views::scratchpad::header_toolbar(entry.instance, pad, state, cx)
                    }
                    _ => None,
                }
                .unwrap_or_else(|| Self::view_header_content(entry));
                let header = chrome::header(Some(title), content, busy, cx);
                (header, self.footer_context(&entry.provider, title), body)
            }
        };
        if let Some(footer) = self.home_edit_footer() {
            (context, primary, has_panel) = footer;
        }
        let toast_showing = self.toasts.read(cx).is_showing();
        let left = chrome::footer_left(&context, toast_showing);

        let mut key_context = KeyContext::default();
        if self.recorder.is_some() {
            // No launcher binding matches: every key reaches the recorder.
            key_context.add(RECORDER_KEY_CONTEXT);
        } else if self.home_editing() && self.stack.top().is_none() && self.panel.is_none() {
            // No launcher binding fires: every key is the mode's.
            key_context.add(home_edit::KEY_CONTEXT);
        } else {
            key_context.add(KEY_CONTEXT);
            if self.stack.top().is_none() && self.panel.is_none() {
                key_context.add(ROOT_KEY_CONTEXT);
            }
        }
        let unavailable = self.hotkeys.borrow().table.unavailable.is_some();
        let recorder = self
            .recorder
            .as_ref()
            .map(|recorder| recorder.render(unavailable, &palette, cx));
        div()
            .key_context(key_context)
            .on_action(cx.listener(Self::strip_next))
            .on_action(cx.listener(Self::strip_prev))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            // The field's own Left and Right, before it moves its caret.
            .capture_action(cx.listener(Self::field_left))
            .capture_action(cx.listener(Self::field_right))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::toggle_actions))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::submit_top_form))
            .on_key_down(cx.listener(Self::shortcut_key))
            // In the Home edit mode, a mouse down outside the tiles leaves it.
            .capture_any_mouse_down(cx.listener(Self::home_edit_outside_down))
            // The window has no kit Root to name the theme's font: every
            // text in it is in that family unless it names another.
            .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
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
                    .border(px(theme::PANEL_BORDER))
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
                        &primary_keys,
                        extra
                            .as_ref()
                            .map(|(title, chips)| (title.as_str(), chips.as_slice())),
                        has_panel,
                        &palette,
                    ))
                    .children({
                        let place = self.menu_place(window);
                        self.panel
                            .as_ref()
                            .map(|panel| panel.render(&palette, place, window, cx))
                    })
                    .children(recorder),
            )
    }
}

/// Key context of the hotkey recorder: no binding names it.
const RECORDER_KEY_CONTEXT: &str = "HotkeyRecorder";

/// The title of indexed item `item`, else its id.
fn title_of(core: &Rc<RefCell<Core>>, item: &ItemId) -> String {
    core.try_borrow()
        .ok()
        .and_then(|core| core.item_title(item))
        .unwrap_or_else(|| item.as_str().to_string())
}

/// Whether `provider` is a plugin the settings window has a page with something to set for:
/// preferences or commands.
fn has_plugin_settings(provider: &str, cx: &App) -> bool {
    cx.try_global::<crate::SettingsController>()
        .is_some_and(|settings| {
            settings.catalog().plugins.iter().any(|plugin| {
                plugin.id == provider
                    && (!plugin.preferences.is_empty() || !plugin.commands.is_empty())
            })
        })
}
