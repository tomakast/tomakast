//! Module interface: providers of items and executors of their actions.

use crate::effect::Effect;
use crate::index::{Item, ItemId};
use crate::view::{ViewEvent, ViewId};

/// Identifies one action or view-event request, so its reply can be matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

/// Events sent to the core from module workers and module-owned threads.
#[derive(Clone, Debug)]
pub enum CoreEvent {
    ReplaceItems {
        provider: String,
        items: Vec<Item>,
    },
    ActionFinished {
        request: RequestId,
        item: ItemId,
        /// The query typed when the action was requested.
        query: String,
        result: Result<Vec<Effect>, String>,
    },
    /// A dynamic source's answer to the search of `generation`.
    SearchResults {
        provider: String,
        generation: u64,
        items: Vec<Item>,
    },
    ViewEventFinished {
        request: RequestId,
        /// The module that owns the view and answered.
        provider: String,
        view: ViewId,
        result: Result<Vec<Effect>, String>,
    },
}

/// Handle given to a module in `start`; cheap to clone and `Send`.
#[derive(Clone)]
pub struct ModuleContext {
    provider: String,
    tx: async_channel::Sender<CoreEvent>,
}

impl ModuleContext {
    pub(crate) fn new(provider: &str, tx: async_channel::Sender<CoreEvent>) -> ModuleContext {
        ModuleContext {
            provider: provider.to_string(),
            tx,
        }
    }

    /// Replaces all items of this module's provider. Items whose id belongs to
    /// another provider, or is not a valid id, are dropped with a warning.
    pub fn replace_items(&self, items: Vec<Item>) {
        let ev = CoreEvent::ReplaceItems {
            provider: self.provider.clone(),
            items: own_valid_items(&self.provider, items),
        };
        // The channel is unbounded; it only fails when the core is gone.
        if self.tx.try_send(ev).is_err() {
            tracing::debug!(module = %self.provider, "core is gone, items not delivered");
        }
    }
}

/// Keeps the items whose id is valid and belongs to `provider`; warns about the rest.
pub(crate) fn own_valid_items(provider: &str, items: Vec<Item>) -> Vec<Item> {
    items
        .into_iter()
        .filter(|item| {
            let problem = if !item.id.is_valid() {
                Some("invalid id")
            } else if item.id.provider() != provider {
                Some("foreign provider")
            } else {
                None
            };
            if let Some(problem) = problem {
                tracing::warn!(module = %provider, item = ?item.id.as_str(), "dropping item: {problem}");
            }
            problem.is_none()
        })
        .collect()
}

/// A provider of items. Every method is called on the module's own worker
/// thread (`module-<id>`), never on the UI thread; a call may block it.
pub trait Module: Send + 'static {
    /// Provider id: the prefix of every item id this module publishes.
    fn id(&self) -> &'static str;
    fn start(&mut self, ctx: ModuleContext);
    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>>;
    /// Whether [`search`](Self::search) answers queries; asked once, after `start`.
    fn is_dynamic_source(&self) -> bool {
        false
    }
    /// Items computed for `query` (calculator, converter). Only called when
    /// [`is_dynamic_source`](Self::is_dynamic_source) is true.
    fn search(&mut self, query: &str) -> anyhow::Result<Vec<Item>> {
        let _ = query;
        Ok(vec![])
    }
    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        let _ = (view, event);
        Ok(vec![])
    }
}
