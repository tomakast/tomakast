//! Core orchestrator: module workers, search engine and usage persistence.
//!
//! The core never calls a module directly: requests go to the module's
//! worker and replies come back through [`Core::events`] into [`Core::apply`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, anyhow};

use crate::effect::Effect;
use crate::engine::{Hit, SearchEngine};
use crate::history::{QueryHistory, recordable};
use crate::host::ModuleHost;
use crate::index::{Action, Item, ItemId};
use crate::module::{CoreEvent, Module, RequestId, own_valid_items};
use crate::readings::{QueryReading, query_readings};
use crate::settings::WidgetSettings;
use crate::store::Store;
use crate::strip::{
    Applied, Render, StoppedWidget, StripTile, Tick, VisibleTile, WidgetEnv, WidgetStore,
};
use crate::usage::{LocalNow, RankWeights, UsageModel};
use crate::view::{ViewEvent, ViewId};
use crate::widget::{Frame, PointerKind, WidgetEvent, WidgetImage, WidgetKey};

const USAGE_HISTORY_DAYS: i64 = 180;
const DAY_SECS: i64 = 86_400;

/// What a reply of a replaced module instance, or an event for a view it pushed, fails with.
/// Only plugins are replaced at runtime (a changed plugin folder is reloaded).
pub const RELOADED: &str = "plugin reloaded";

/// Ranked hits for the query of search `generation`: static matches merged
/// with the dynamic answers received so far for that generation.
#[derive(Clone, Debug)]
pub struct SearchSnapshot {
    pub generation: u64,
    pub hits: Vec<Hit>,
}

/// What a [`Core::apply`] changed, for the UI.
#[derive(Clone, Debug)]
pub enum CoreUpdate {
    None,
    /// Static items changed; re-rank the current query with [`Core::refresh`].
    ItemsChanged,
    Results(SearchSnapshot),
    /// `effects` never contain `ReplaceItems` or a widget effect
    /// (`ShowWidgets`, `RedrawWidget`, `SetWidgetImage`): the core applied
    /// them, and `items_changed` says whether the items changed (re-rank as
    /// for `ItemsChanged`).
    ActionDone {
        request: RequestId,
        /// The module that ran the action.
        provider: String,
        /// The instance of `provider` that answered (see [`Core::send_view_event`]).
        module_instance: u64,
        effects: Vec<Effect>,
        items_changed: bool,
    },
    ActionFailed {
        request: RequestId,
        message: String,
    },
    /// Like `ActionDone`, for a view event.
    ViewEffects {
        request: RequestId,
        provider: String,
        /// The instance of `provider` that answered.
        module_instance: u64,
        view: ViewId,
        effects: Vec<Effect>,
        items_changed: bool,
    },
    ViewFailed {
        request: RequestId,
        provider: String,
        view: ViewId,
        message: String,
    },
    /// Effects a module sent on its own (a wake-up, or
    /// [`ModuleContext::emit`](crate::ModuleContext::emit)). As for
    /// `ActionDone`, `effects` never contain `ReplaceItems` or a widget effect.
    ModuleEffects {
        provider: String,
        /// The instance of `provider` that sent them.
        module_instance: u64,
        effects: Vec<Effect>,
        items_changed: bool,
    },
    /// The widget strip changed: re-read it.
    Widgets,
    /// A module's answer to a widget event: with the `request` of
    /// [`Core::send_widget_event`], or `None` for an event no one waits on.
    /// As for `ActionDone`, `effects` never contain `ReplaceItems` or a
    /// widget effect.
    WidgetEffects {
        request: Option<RequestId>,
        provider: String,
        /// The instance of `provider` that answered.
        module_instance: u64,
        effects: Vec<Effect>,
        items_changed: bool,
    },
    /// Widgets that drew late or failed too many times in a row, or that
    /// [`Core::stop_widget`] stopped; they render no more (but a condition
    /// tile's widget stopped for its frames, from the next showing, see
    /// [`Core::widgets_hidden`]). One apply may stop several.
    WidgetStopped {
        widgets: Vec<StoppedWidget>,
    },
}

pub struct Core {
    engine: SearchEngine,
    store: Store,
    host: ModuleHost,
    /// The sender the workers use; the core answers a view event of a replaced instance
    /// through it, so the answer arrives like any other.
    tx: async_channel::Sender<CoreEvent>,
    rx: async_channel::Receiver<CoreEvent>,
    next_request: u64,
    /// Generation and text of the latest search.
    generation: u64,
    query: String,
    /// Answers of dynamic sources for the latest search, by provider. Ordered
    /// by provider id, so equal scores rank providers by id, then each
    /// provider's answer in its own order.
    dynamic: BTreeMap<String, Vec<Arc<Item>>>,
    history: QueryHistory,
    /// Providers whose runs are not recorded in the history.
    history_exempt: HashSet<String>,
    /// The query of the last run of an exempt provider; the next hide skips it once.
    skip_on_hide: Option<String>,
    /// By request: the query a condition tile was answered for, of a user
    /// action on it (an action, a key, a click) not answered yet; recorded
    /// in the history when it succeeds.
    tile_queries: HashMap<RequestId, String>,
    /// The widget and node of the last pointer press; a release is a click
    /// only on the node pressed.
    pressed: Option<(WidgetKey, String)>,
    widgets: WidgetStore,
    /// A [`CoreEvent::StripChanged`] is queued and not yet applied.
    strip_change_queued: bool,
}

impl Core {
    /// Loads the last 180 days of usage from `store`.
    pub fn new(store: Store, weights: RankWeights, now: LocalNow) -> anyhow::Result<Core> {
        let records = store
            .load_usage(now.unix - USAGE_HISTORY_DAYS * DAY_SECS)
            .context("loading usage history")?;
        let (tx, rx) = async_channel::unbounded();
        Ok(Core {
            engine: SearchEngine::new(UsageModel::new(records), weights),
            store,
            host: ModuleHost::new(tx.clone()),
            tx,
            rx,
            next_request: 0,
            generation: 0,
            query: String::new(),
            dynamic: BTreeMap::new(),
            history: QueryHistory::load(Store::open_in_memory()?)?,
            history_exempt: HashSet::new(),
            skip_on_hide: None,
            tile_queries: HashMap::new(),
            pressed: None,
            widgets: WidgetStore::default(),
            strip_change_queued: false,
        })
    }

    pub fn history(&self) -> &QueryHistory {
        &self.history
    }

    pub fn set_history(&mut self, history: QueryHistory) {
        self.history = history;
    }

    /// A successful run of an item of `provider` records nothing, and its query is
    /// remembered so the hide that follows skips it too.
    pub fn exempt_from_history(&mut self, provider: &str) {
        self.history_exempt.insert(provider.to_string());
    }

    /// The hide path: records the root query unless it is the query of the exempt run
    /// just before (one hide only). Returns whether it recorded.
    pub fn record_hidden_query(&mut self, query: &str, now: LocalNow) -> bool {
        if let Some(skipped) = self.skip_on_hide.take()
            && skipped == query.trim()
        {
            return false;
        }
        self.history.record(query, now.unix)
    }

    /// Adds a module. Before [`start_modules`](Self::start_modules) it waits
    /// for it; after, its worker starts at once. A module whose id is already
    /// registered replaces that one: the old worker exits after its current
    /// call, its items and dynamic answers are dropped at once, and nothing
    /// it sends later is applied. Re-rank with [`refresh`](Self::refresh).
    ///
    /// Reads `id()` here, on the caller's thread: it names the worker thread.
    /// An id that is empty, contains `/` or a control character is refused.
    /// `widgets()` and `approved_at()` are read here too, so the tiles a module
    /// shows from its `start` are known.
    ///
    /// Registered after `start_modules` (a reloaded plugin), it is asked the
    /// latest search once started, so its answer and its tiles come back,
    /// unless the launcher hid since (the next showing searches anew).
    pub fn register(&mut self, m: Box<dyn Module>) {
        let id = m.id().to_string();
        if !is_valid_module_id(&id) {
            tracing::error!(module = ?id, "refusing a module with an invalid id");
            return;
        }
        self.drop_provider(&id);
        let decls = m.widgets();
        if !decls.is_empty() {
            self.widgets.declare(&id, decls.clone(), m.approved_at());
        }
        self.host.register(id.clone(), m, decls);
        self.search_again(&id);
    }

    /// Asks `provider`'s new worker the latest search, if there was one and
    /// the launcher did not hide since.
    fn search_again(&mut self, provider: &str) {
        if self.generation == 0
            || self.widgets.is_closed()
            || self.host.instance(provider).is_none()
        {
            return;
        }
        self.widgets.asked_again(provider);
        let stopped = self.widgets.stopped(provider);
        let readings = self.query_readings(&self.query);
        self.host.search_one(
            provider,
            self.generation,
            &self.query,
            &readings,
            self.widgets.env(),
            stopped,
        );
    }

    /// Stops `provider`'s worker after its current call and removes its items
    /// and dynamic answers; nothing it sends later is applied. Re-rank with
    /// [`refresh`](Self::refresh).
    pub fn unregister(&mut self, provider: &str) {
        self.drop_provider(provider);
    }

    /// Makes the items of `provider` lead the empty query, above any usage
    /// (see [`SearchEngine::pin_provider`]). The daemon pins the trusted
    /// `plugins` review module; nothing a module sends can pin it.
    pub fn pin_provider(&mut self, provider: &str) {
        self.engine.pin_provider(provider);
    }

    /// Starts a worker thread for each registered module; the worker calls
    /// `start` and then serves the module's requests. Returns at once.
    pub fn start_modules(&mut self) {
        self.host.start();
    }

    pub fn events(&self) -> async_channel::Receiver<CoreEvent> {
        self.rx.clone()
    }

    /// Applies one event from [`events`](Self::events). `now` dates the usage
    /// record of a successful action.
    pub fn apply(&mut self, ev: CoreEvent, now: LocalNow) -> CoreUpdate {
        match ev {
            CoreEvent::Instance {
                provider,
                instance,
                event,
            } => {
                if self.host.is_current(&provider, instance) {
                    self.apply_from(*event, instance, now)
                } else {
                    if let CoreEvent::WidgetEventFinished {
                        request: Some(request),
                        ..
                    } = inner_event(&event)
                    {
                        self.tile_queries.remove(request);
                    }
                    stale(&provider, self.host.is_registered(&provider), *event)
                }
            }
            // Only the core's own answers come unstamped (see `send_view_event`).
            ev => self.apply_from(ev, 0, now),
        }
    }

    /// Applies an event of module instance `instance`, which is current.
    fn apply_from(&mut self, ev: CoreEvent, instance: u64, now: LocalNow) -> CoreUpdate {
        match ev {
            CoreEvent::ReplaceItems { provider, items } => {
                self.engine.index_mut().replace_provider(&provider, items);
                CoreUpdate::ItemsChanged
            }
            CoreEvent::SearchResults {
                provider,
                generation,
                items,
            } => {
                if generation != self.generation {
                    return CoreUpdate::None;
                }
                let mut shadowed = 0;
                let items = own_valid_items(&provider, items)
                    .into_iter()
                    .filter(|item| {
                        let shadows = self.engine.index().get(&item.id).is_some();
                        shadowed += usize::from(shadows);
                        !shadows
                    })
                    .map(Arc::new)
                    .collect();
                // Debug, once per answer: a dynamic module answers every keystroke, and a
                // plugin could otherwise write a warning per item per keystroke.
                if shadowed > 0 {
                    tracing::debug!(
                        module = %provider,
                        "dropped {shadowed} dynamic items: indexed items have their ids"
                    );
                }
                self.dynamic.insert(provider, items);
                CoreUpdate::Results(self.refresh(now))
            }
            CoreEvent::ActionFinished {
                request,
                item,
                query,
                result,
            } => match result {
                Ok(effects) => {
                    self.record_usage(&item, &query, now);
                    self.record_run(item.provider(), &query, now);
                    let provider = item.provider().to_string();
                    let (effects, items_changed) = self.take_core_effects(&provider, effects);
                    CoreUpdate::ActionDone {
                        request,
                        provider,
                        module_instance: instance,
                        effects,
                        items_changed,
                    }
                }
                Err(message) => CoreUpdate::ActionFailed { request, message },
            },
            CoreEvent::ViewEventFinished {
                request,
                provider,
                view,
                result,
            } => match result {
                Ok(effects) => {
                    let (effects, items_changed) = self.take_core_effects(&provider, effects);
                    CoreUpdate::ViewEffects {
                        request,
                        provider,
                        module_instance: instance,
                        view,
                        effects,
                        items_changed,
                    }
                }
                Err(message) => CoreUpdate::ViewFailed {
                    request,
                    provider,
                    view,
                    message,
                },
            },
            CoreEvent::ModuleEffects { provider, effects } => {
                let (effects, items_changed) = self.take_core_effects(&provider, effects);
                CoreUpdate::ModuleEffects {
                    provider,
                    module_instance: instance,
                    effects,
                    items_changed,
                }
            }
            CoreEvent::WidgetRendered {
                provider,
                widget,
                seq,
                took,
                result,
            } => {
                let key = WidgetKey::new(provider, widget);
                let applied = self.widgets.rendered(&key, seq, took, result);
                self.strip_applied(&key.provider, applied)
            }
            CoreEvent::WidgetAnswers {
                provider,
                generation,
                answers,
            } => {
                if generation != self.generation {
                    let mut answers = answers;
                    if self.widgets.answer_images(&provider, &mut answers) {
                        self.strip_changed();
                    }
                    return CoreUpdate::None;
                }
                let applied = self
                    .widgets
                    .answered(&provider, generation, &self.query, answers);
                self.strip_applied(&provider, applied)
            }
            CoreEvent::WidgetEventFinished {
                request,
                provider,
                widget,
                result,
            } => {
                if self.widgets.mark_dirty(&provider, &widget) {
                    self.strip_changed();
                }
                let query = request.and_then(|request| self.tile_queries.remove(&request));
                match (result, request) {
                    (Ok(effects), request) => {
                        if let Some(query) = query {
                            self.record_run(&provider, &query, now);
                        }
                        let (effects, items_changed) = self.take_core_effects(&provider, effects);
                        CoreUpdate::WidgetEffects {
                            request,
                            provider,
                            module_instance: instance,
                            effects,
                            items_changed,
                        }
                    }
                    (Err(message), Some(request)) => CoreUpdate::ActionFailed { request, message },
                    (Err(message), None) => {
                        tracing::warn!(module = %provider, %widget, "widget event failed: {message}");
                        CoreUpdate::None
                    }
                }
            }
            CoreEvent::StripChanged => {
                self.strip_change_queued = false;
                CoreUpdate::Widgets
            }
            CoreEvent::Instance { provider, .. } => {
                tracing::warn!(module = %provider, "a nested instance event is dropped");
                CoreUpdate::None
            }
        }
    }

    /// Starts a new search generation: the static hits are ranked now, and
    /// every dynamic source is asked for `query` on its worker.
    pub fn search(&mut self, query: &str, now: LocalNow) -> SearchSnapshot {
        self.generation += 1;
        self.query = query.to_string();
        self.dynamic.clear();
        if self.widgets.searched(self.generation) {
            self.strip_changed();
        }
        let readings = self.query_readings(query);
        let widgets = &self.widgets;
        self.host.search(
            self.generation,
            query,
            &readings,
            widgets.env(),
            |provider| widgets.stopped(provider),
        );
        self.refresh(now)
    }

    /// Readings of `query` on other layouts and scripts, against the scripts
    /// present in the index.
    pub fn query_readings(&self, query: &str) -> Vec<QueryReading> {
        query_readings(query, self.engine.index().scripts())
    }

    /// Re-ranks the latest search without asking the modules again, e.g.
    /// after [`CoreUpdate::ItemsChanged`].
    pub fn refresh(&mut self, now: LocalNow) -> SearchSnapshot {
        let dynamic: Vec<Arc<Item>> = self.dynamic.values().flatten().cloned().collect();
        SearchSnapshot {
            generation: self.generation,
            hits: self.engine.search_with(&self.query, now, &dynamic),
        }
    }

    /// Id of the primary (first) action of a known item.
    pub fn primary_action(&self, item: &ItemId) -> Option<String> {
        let entry = self.find(item)?;
        entry.actions.first().map(|a| a.id.clone())
    }

    /// Queues the action on the owning module's worker and returns at once.
    /// The reply is a [`CoreEvent::ActionFinished`]; usage is recorded when
    /// a successful one is applied.
    pub fn request_action(
        &mut self,
        item: &ItemId,
        action_id: &str,
        query: &str,
    ) -> anyhow::Result<RequestId> {
        let entry = self
            .find(item)
            .ok_or_else(|| anyhow!("unknown item {}", item.as_str()))?;
        let entry = Item::clone(&entry);
        let request = self.next_request_id();
        self.host.run_action(request, entry, action_id, query)?;
        Ok(request)
    }

    /// Queues a view event on `provider`'s worker and returns at once. The
    /// reply is a [`CoreEvent::ViewEventFinished`].
    ///
    /// `module_instance` is the instance of `provider` that pushed the view
    /// (the `module_instance` of the update that carried the `PushView`). A
    /// view of an instance that has since been replaced or stopped stays on
    /// the UI's stack, but its events never reach another instance: they fail
    /// with [`RELOADED`] (replaced) or "module <id> was stopped".
    pub fn send_view_event(
        &mut self,
        provider: &str,
        module_instance: u64,
        view: &ViewId,
        event: ViewEvent,
    ) -> anyhow::Result<RequestId> {
        let request = self.next_request_id();
        if !self.host.is_current(provider, module_instance) {
            let message = stale_message(provider, self.host.is_registered(provider));
            // Unbounded, and the core holds the receiver: it cannot fail.
            let _ = self.tx.try_send(CoreEvent::ViewEventFinished {
                request,
                provider: provider.to_string(),
                view: view.clone(),
                result: Err(message),
            });
            return Ok(request);
        }
        self.host.view_event(provider, request, view, event)?;
        Ok(request)
    }

    /// Sets what the UI's tiles look like; renders from now on use it. One
    /// that differs from the last asks every tile on screen to draw again.
    pub fn set_widget_env(&mut self, env: WidgetEnv) {
        if self.widgets.set_env(env) {
            self.strip_changed();
        }
    }

    /// Sets the user's `[widgets]` order and hidden tiles.
    pub fn set_widget_settings(&mut self, settings: WidgetSettings) {
        if self.widgets.set_settings(settings) {
            self.strip_changed();
        }
    }

    /// The tiles of the strip, in order: the condition tiles, providers by
    /// id and each in its answer order, then the always tiles, those of
    /// `[widgets] order` first and the rest by approval time (built-ins
    /// first), provider id and declared order. `[widgets] hidden` tiles, and
    /// the always tiles [`stop_widget`](Self::stop_widget) stopped, are left
    /// out; an always tile that is also answered shows once, as a condition
    /// tile.
    pub fn strip(&self) -> Vec<StripTile> {
        self.widgets.tiles()
    }

    /// One tick of the UI's frame clock with the tiles on screen: tiles that
    /// came on screen are told `Shown` and draw, those that left are told
    /// `Hidden`, and a visible tile renders when it is asked to draw again or
    /// its last frame animates, with no render in flight, at most every
    /// [`MIN_RENDER_INTERVAL`](crate::MIN_RENDER_INTERVAL), unless it is
    /// stopped. Returns whether a tile on screen still wants a frame: the UI
    /// should tick again on its next one.
    pub fn widget_tick(&mut self, now: Instant, visible: &[VisibleTile]) -> bool {
        let Tick {
            hidden,
            shown,
            renders,
        } = self.widgets.tick(now, visible);
        for key in hidden {
            self.tell(&key, WidgetEvent::Hidden);
        }
        for key in shown {
            self.tell(&key, WidgetEvent::Shown);
        }
        for (key, render) in renders {
            self.issue(&key, render, now);
        }
        self.widgets.busy()
    }

    /// The launcher was hidden: every tile on screen is told `Hidden`, and
    /// nothing renders until a tick shows tiles again. The condition tiles
    /// go, and none is kept until the next [`search`](Self::search), so the
    /// next showing starts with none; a widget stopped for late or failed
    /// frames as a condition tile draws again from then on (an always
    /// tile's widget stays stopped).
    pub fn widgets_hidden(&mut self) {
        // A press of the last showing pairs with no release of the next.
        self.pressed = None;
        for key in self.widgets.hide_all() {
            self.tell(&key, WidgetEvent::Hidden);
        }
        if self.widgets.close() {
            self.strip_changed();
        }
    }

    /// The actions of `key`: its condition tile's, else those of its last
    /// frame; none for a tile answered for an older query.
    pub fn widget_actions(&self, key: &WidgetKey) -> Vec<Action> {
        self.widgets.actions(key)
    }

    /// Whether a dynamic source that declared widgets was asked the latest
    /// search with an env and has not yet answered it with its tiles. A
    /// module still in `start` counts as one, as long as it was asked: one
    /// registered when the search was made, or registered (reloaded) after
    /// it while the launcher showed (see [`register`](Self::register)). It
    /// answers once started, if it is one.
    pub fn widget_answers_pending(&self) -> bool {
        self.widgets
            .answers_pending(|provider| self.host.may_be_dynamic(provider))
    }

    /// Issues one render of `key` on its module's worker and returns whether
    /// it did. It does not while no env is set, before the module started,
    /// for an unknown or stopped widget, or within
    /// [`MIN_RENDER_INTERVAL`](crate::MIN_RENDER_INTERVAL) (less
    /// [`RENDER_INTERVAL_JITTER`](crate::strip::RENDER_INTERVAL_JITTER)) of
    /// the last issue. Nothing changes when it does not. The reply is a [`CoreEvent::WidgetRendered`].
    pub fn render_widget(&mut self, key: &WidgetKey, now: Instant) -> bool {
        match self.widgets.next_render(key, now) {
            Some(render) => self.issue(key, render, now),
            None => false,
        }
    }

    /// Sends `render` of `key` to its module's worker; returns whether it did.
    fn issue(&mut self, key: &WidgetKey, render: Render, now: Instant) -> bool {
        let sent = self.host.render_widget(
            &key.provider,
            &key.widget,
            render.seq,
            render.context.clone(),
        );
        match sent {
            Ok(()) => {
                self.widgets.issued(key, &render, now);
                tracing::trace!(target: "widgets", provider = %key.provider, widget = %key.widget, "render");
                true
            }
            Err(err) => {
                tracing::debug!(widget = %key, %err, "render not issued");
                false
            }
        }
    }

    /// Queues `event` for `key`'s module, with the payload of its condition
    /// tile, and returns at once. The reply is a
    /// [`CoreEvent::WidgetEventFinished`] with the returned request; `Shown`,
    /// `Hidden` and pointer `Move`, `Enter` and `Leave` get none.
    ///
    /// An action, a key or a pointer event is refused on a condition tile
    /// answered for an older query than the latest search's, and, with
    /// [`RELOADED`], on a tile that was on the strip when its plugin was
    /// reloaded and that the new instance does not show.
    ///
    /// A user action on a condition tile (an action, a key, a click: a
    /// pointer release on the node the last press was on) runs from the
    /// query the tile was answered for: once it succeeds, that query is
    /// recorded in the history, as for an item's run. A press alone, or a
    /// release elsewhere, is not one.
    pub fn send_widget_event(
        &mut self,
        key: &WidgetKey,
        event: WidgetEvent,
    ) -> anyhow::Result<Option<RequestId>> {
        if !self.host.is_registered(&key.provider) {
            return Err(anyhow!("no module for provider {}", key.provider));
        }
        if !self.widgets.is_declared(key) {
            return Err(anyhow!("no widget {key}"));
        }
        if !matches!(event, WidgetEvent::Shown | WidgetEvent::Hidden) {
            if self.widgets.is_stale(key) {
                return Err(anyhow!("answered for an older query"));
            }
            if self.widgets.is_gone(key) {
                return Err(anyhow!(RELOADED));
            }
        }
        // Only what the user did gets a request; showing, hiding and hovering do not.
        let request = match &event {
            WidgetEvent::Shown | WidgetEvent::Hidden => None,
            WidgetEvent::Pointer(p)
                if matches!(
                    p.kind,
                    PointerKind::Move | PointerKind::Enter | PointerKind::Leave
                ) =>
            {
                None
            }
            WidgetEvent::Action { .. } | WidgetEvent::Key(_) | WidgetEvent::Pointer(_) => {
                Some(self.next_request_id())
            }
        };
        // A user action on a condition tile runs from the query it was answered for.
        let user_action = match &event {
            WidgetEvent::Action { .. } | WidgetEvent::Key(_) => true,
            WidgetEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    self.pressed = Some((key.clone(), p.node.clone()));
                    false
                }
                PointerKind::Up => self
                    .pressed
                    .take()
                    .is_some_and(|(pressed, node)| pressed == *key && node == p.node),
                PointerKind::Move | PointerKind::Enter | PointerKind::Leave => false,
            },
            WidgetEvent::Shown | WidgetEvent::Hidden => false,
        };
        let query = self.widgets.answered_query(key).filter(|_| user_action);
        let payload = self.widgets.payload(key);
        self.host
            .widget_event(&key.provider, request, &key.widget, payload, event)?;
        if let (Some(request), Some(query)) = (request, query) {
            self.tile_queries.insert(request, query);
        }
        Ok(request)
    }

    /// Tells `key`'s module `event`, which no one waits on.
    fn tell(&self, key: &WidgetKey, event: WidgetEvent) {
        let payload = self.widgets.payload(key);
        if let Err(err) = self
            .host
            .widget_event(&key.provider, None, &key.widget, payload, event)
        {
            tracing::debug!(widget = %key, %err, "widget event not sent");
        }
    }

    /// Holds widget frames to `deadline` in place of
    /// [`soft_render_deadline`](crate::soft_render_deadline), so a test can
    /// make a frame late without depending on how loaded the machine is.
    #[cfg(any(test, feature = "testing"))]
    pub fn set_soft_render_deadline(&mut self, deadline: std::time::Duration) {
        self.widgets.set_soft_deadline(deadline);
    }

    /// Stops `key`'s widget for this module instance, as one that drew late
    /// too often is: it renders no more and its condition tile goes. Unlike
    /// that one, its always tile leaves the strip with its frame, and the
    /// stop does not end when the launcher hides: the UI stops one whose
    /// frames it cannot paint fast enough. Returns the
    /// [`CoreUpdate::WidgetStopped`] to show, or nothing for an unknown or
    /// already stopped widget.
    pub fn stop_widget(&mut self, key: &WidgetKey, reason: &str) -> CoreUpdate {
        let applied = self.widgets.stop(key, reason);
        self.strip_applied(&key.provider, applied)
    }

    /// The frame `key` shows: its condition tile's, else its last always frame.
    pub fn widget_frame(&self, key: &WidgetKey) -> Option<Arc<Frame>> {
        self.widgets.frame(key)
    }

    /// Image `id` of `provider`, with its version: a number that grows every
    /// time an image is stored, so a replaced image never has its old version.
    pub fn widget_image(&self, provider: &str, id: &str) -> Option<(u64, WidgetImage)> {
        self.widgets.image(provider, id)
    }

    /// The ids of `provider`'s always tiles, in its order.
    pub fn always_widgets(&self, provider: &str) -> Vec<String> {
        self.widgets.always(provider)
    }

    /// The running instance of `provider`, if it has started.
    pub fn module_instance(&self, provider: &str) -> Option<u64> {
        self.host.instance(provider)
    }

    /// An indexed item, or a dynamic answer of the latest search.
    fn find(&self, item: &ItemId) -> Option<Arc<Item>> {
        if let Some(entry) = self.engine.index().get(item) {
            return Some(entry.clone());
        }
        self.dynamic
            .get(item.provider())?
            .iter()
            .find(|entry| entry.id == *item)
            .cloned()
    }

    /// Applies the effects of `provider`'s reply that the core owns, and
    /// returns the others and whether the items changed. `ReplaceItems` is
    /// validated as [`ModuleContext::replace_items`](crate::ModuleContext::replace_items)
    /// does; `ShowWidgets`, `RedrawWidget` and `SetWidgetImage` change the strip.
    fn take_core_effects(&mut self, provider: &str, effects: Vec<Effect>) -> (Vec<Effect>, bool) {
        let mut changed = false;
        let mut strip_changed = false;
        let mut rest = Vec::with_capacity(effects.len());
        for effect in effects {
            match effect {
                Effect::ReplaceItems(items) => {
                    let items = own_valid_items(provider, items);
                    self.engine.index_mut().replace_provider(provider, items);
                    changed = true;
                }
                Effect::ShowWidgets(ids) => strip_changed |= self.widgets.show(provider, ids),
                Effect::RedrawWidget(id) => strip_changed |= self.widgets.mark_dirty(provider, &id),
                Effect::SetWidgetImage(image) => {
                    strip_changed |= self.widgets.set_image(provider, image)
                }
                other => rest.push(other),
            }
        }
        if strip_changed {
            self.strip_changed();
        }
        (rest, changed)
    }

    /// Queues one [`CoreEvent::StripChanged`] unless one is already queued.
    fn strip_changed(&mut self) {
        if !self.strip_change_queued {
            self.strip_change_queued = true;
            // Unbounded, and the core holds the receiver: it cannot fail.
            let _ = self.tx.try_send(CoreEvent::StripChanged);
        }
    }

    /// What a frame or an answer applied to the store means for the UI:
    /// every stopped widget is logged and reported.
    fn strip_applied(&mut self, provider: &str, applied: Applied) -> CoreUpdate {
        if applied.changed {
            self.strip_changed();
        }
        if applied.stopped.is_empty() {
            return CoreUpdate::None;
        }
        for stopped in &applied.stopped {
            tracing::warn!(module = %provider, widget = %stopped.title, "widget stopped: {}", stopped.reason);
        }
        CoreUpdate::WidgetStopped {
            widgets: applied.stopped,
        }
    }

    /// Stops `provider`'s worker, if any, and forgets its items, answers and widgets.
    fn drop_provider(&mut self, provider: &str) {
        self.host.stop(provider);
        self.engine
            .index_mut()
            .replace_provider(provider, Vec::new());
        self.dynamic.remove(provider);
        if self.widgets.forget(provider) {
            self.strip_changed();
        }
    }

    fn next_request_id(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    /// A run of `provider` from `query` succeeded: the query is recorded in
    /// the history, unless `provider` is exempt; then the next hide skips it.
    fn record_run(&mut self, provider: &str, query: &str, now: LocalNow) {
        if self.history_exempt.contains(provider) {
            self.skip_on_hide = recordable(query).map(str::to_string);
        } else {
            self.history.record(query, now.unix);
            self.skip_on_hide = None;
        }
    }

    /// The action already happened: a persistence failure is logged, not returned.
    fn record_usage(&mut self, item: &ItemId, query: &str, now: LocalNow) {
        let record = self.engine.record_launch(item, query, now);
        if let Err(err) = self.store.record_usage(&record) {
            tracing::error!(item = %item.as_str(), error = %err, "failed to persist usage");
        }
    }
}

/// Why a request of a stopped instance of `provider` failed: [`RELOADED`]
/// when another instance runs under the id (`replaced`), else it was stopped.
fn stale_message(provider: &str, replaced: bool) -> String {
    if replaced {
        RELOADED.to_string()
    } else {
        format!("module {provider} was stopped")
    }
}

/// What an event of a replaced or unregistered module instance becomes: its
/// replies fail, so whoever waits for them hears back; the rest is dropped.
fn stale(provider: &str, replaced: bool, event: CoreEvent) -> CoreUpdate {
    let message = stale_message(provider, replaced);
    match event {
        CoreEvent::ActionFinished { request, .. } => CoreUpdate::ActionFailed { request, message },
        CoreEvent::ViewEventFinished {
            request,
            provider,
            view,
            ..
        } => CoreUpdate::ViewFailed {
            request,
            provider,
            view,
            message,
        },
        CoreEvent::WidgetEventFinished {
            request: Some(request),
            ..
        } => CoreUpdate::ActionFailed { request, message },
        CoreEvent::Instance { event, .. } => stale(provider, replaced, *event),
        CoreEvent::ReplaceItems { .. }
        | CoreEvent::SearchResults { .. }
        | CoreEvent::ModuleEffects { .. }
        | CoreEvent::WidgetRendered { .. }
        | CoreEvent::WidgetAnswers { .. }
        | CoreEvent::WidgetEventFinished { request: None, .. }
        | CoreEvent::StripChanged => {
            tracing::debug!(module = %provider, "dropping an event of a stopped instance");
            CoreUpdate::None
        }
    }
}

/// The event inside any instance stamps.
fn inner_event(event: &CoreEvent) -> &CoreEvent {
    match event {
        CoreEvent::Instance { event, .. } => inner_event(event),
        event => event,
    }
}

/// A module id names its worker thread and prefixes its item ids.
fn is_valid_module_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/') && !id.chars().any(char::is_control)
}
