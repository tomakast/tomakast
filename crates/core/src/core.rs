//! Core orchestrator: module workers, search engine and usage persistence.
//!
//! The core never calls a module directly: requests go to the module's
//! worker and replies come back through [`Core::events`] into [`Core::apply`].

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, anyhow};

use crate::effect::Effect;
use crate::engine::{Hit, SearchEngine};
use crate::host::ModuleHost;
use crate::index::{Item, ItemId};
use crate::module::{CoreEvent, Module, RequestId, own_valid_items};
use crate::store::Store;
use crate::usage::{LocalNow, RankWeights, UsageModel};
use crate::view::{ViewEvent, ViewId};

const USAGE_HISTORY_DAYS: i64 = 180;
const DAY_SECS: i64 = 86_400;

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
    /// `effects` never contain `ReplaceItems`: the core applied them, and
    /// `items_changed` says whether it did (re-rank as for `ItemsChanged`).
    ActionDone {
        request: RequestId,
        /// The module that ran the action.
        provider: String,
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
}

pub struct Core {
    engine: SearchEngine,
    store: Store,
    host: ModuleHost,
    rx: async_channel::Receiver<CoreEvent>,
    next_request: u64,
    /// Generation and text of the latest search.
    generation: u64,
    query: String,
    /// Answers of dynamic sources for the latest search, by provider.
    dynamic: HashMap<String, Vec<Arc<Item>>>,
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
            host: ModuleHost::new(tx),
            rx,
            next_request: 0,
            generation: 0,
            query: String::new(),
            dynamic: HashMap::new(),
        })
    }

    /// Adds a module; it starts with the next [`start_modules`](Self::start_modules).
    /// Reads `id()` here, on the caller's thread: it names the worker thread.
    pub fn register(&mut self, m: Box<dyn Module>) {
        self.host.register(m);
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
                let items = own_valid_items(&provider, items)
                    .into_iter()
                    .map(Arc::new)
                    .collect();
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
                    let provider = item.provider().to_string();
                    let (effects, items_changed) = self.take_replace_items(&provider, effects);
                    CoreUpdate::ActionDone {
                        request,
                        provider,
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
                    let (effects, items_changed) = self.take_replace_items(&provider, effects);
                    CoreUpdate::ViewEffects {
                        request,
                        provider,
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
        }
    }

    /// Starts a new search generation: the static hits are ranked now, and
    /// every dynamic source is asked for `query` on its worker.
    pub fn search(&mut self, query: &str, now: LocalNow) -> SearchSnapshot {
        self.generation += 1;
        self.query = query.to_string();
        self.dynamic.clear();
        self.host.search(self.generation, query);
        self.refresh(now)
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
    pub fn send_view_event(
        &mut self,
        provider: &str,
        view: &ViewId,
        event: ViewEvent,
    ) -> anyhow::Result<RequestId> {
        let request = self.next_request_id();
        self.host.view_event(provider, request, view, event)?;
        Ok(request)
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

    /// Applies the `ReplaceItems` effects of `provider`'s reply to the index,
    /// validated as [`ModuleContext::replace_items`](crate::ModuleContext::replace_items)
    /// does, and returns the other effects and whether the items changed.
    fn take_replace_items(&mut self, provider: &str, effects: Vec<Effect>) -> (Vec<Effect>, bool) {
        let mut changed = false;
        let mut rest = Vec::with_capacity(effects.len());
        for effect in effects {
            match effect {
                Effect::ReplaceItems(items) => {
                    let items = own_valid_items(provider, items);
                    self.engine.index_mut().replace_provider(provider, items);
                    changed = true;
                }
                other => rest.push(other),
            }
        }
        (rest, changed)
    }

    fn next_request_id(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    /// The action already happened: a persistence failure is logged, not returned.
    fn record_usage(&mut self, item: &ItemId, query: &str, now: LocalNow) {
        let record = self.engine.record_launch(item, query, now);
        if let Err(err) = self.store.record_usage(&record) {
            tracing::error!(item = %item.as_str(), error = %err, "failed to persist usage");
        }
    }
}
