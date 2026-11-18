//! Core orchestrator: modules, search engine and usage persistence.

use std::collections::HashMap;

use anyhow::{Context, anyhow};

use crate::engine::{Hit, SearchEngine};
use crate::index::ItemId;
use crate::module::{ActionOutcome, CoreEvent, Module, ModuleContext};
use crate::store::Store;
use crate::usage::{LocalNow, RankWeights, UsageModel};

const USAGE_HISTORY_DAYS: i64 = 180;
const DAY_SECS: i64 = 86_400;

pub struct Core {
    engine: SearchEngine,
    store: Store,
    modules: HashMap<String, Box<dyn Module>>,
    tx: async_channel::Sender<CoreEvent>,
    rx: async_channel::Receiver<CoreEvent>,
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
            modules: HashMap::new(),
            tx,
            rx,
        })
    }

    pub fn register(&mut self, m: Box<dyn Module>) {
        self.modules.insert(m.id().to_string(), m);
    }

    pub fn start_modules(&mut self) {
        for (id, m) in &mut self.modules {
            m.start(ModuleContext::new(id, self.tx.clone()));
        }
    }

    pub fn events(&self) -> async_channel::Receiver<CoreEvent> {
        self.rx.clone()
    }

    pub fn apply(&mut self, ev: CoreEvent) {
        match ev {
            CoreEvent::ReplaceItems { provider, items } => {
                self.engine.index_mut().replace_provider(&provider, items);
            }
        }
    }

    pub fn search(&mut self, query: &str, now: LocalNow) -> Vec<Hit> {
        self.engine.search(query, now)
    }

    /// Id of the primary (first) action of an indexed item.
    pub fn primary_action(&self, item: &ItemId) -> Option<String> {
        let entry = self.engine.index().get(item)?;
        entry.actions.first().map(|a| a.id.clone())
    }

    /// Runs the action in the owning module; usage is recorded only on success.
    /// A failure to persist usage is logged and does not fail the call.
    pub fn execute(
        &mut self,
        item: &ItemId,
        action_id: &str,
        query: &str,
        now: LocalNow,
    ) -> anyhow::Result<ActionOutcome> {
        let entry = self
            .engine
            .index()
            .get(item)
            .cloned()
            .ok_or_else(|| anyhow!("unknown item {}", item.as_str()))?;
        let module = self
            .modules
            .get_mut(item.provider())
            .ok_or_else(|| anyhow!("no module for provider {}", item.provider()))?;
        let outcome = module.run_action(&entry, action_id)?;
        let record = self.engine.record_launch(item, query, now);
        // The action already happened: a persistence failure must not turn it into an error.
        if let Err(err) = self.store.record_usage(&record) {
            tracing::error!(item = %item.as_str(), error = %err, "failed to persist usage");
        }
        Ok(outcome)
    }
}
