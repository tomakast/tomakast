//! Module interface: providers of items and executors of their actions.

use crate::index::Item;

/// What the UI should do after an action ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    HideWindow,
    KeepOpen,
}

/// Events modules send to the core from any thread.
#[derive(Clone, Debug)]
pub enum CoreEvent {
    ReplaceItems { provider: String, items: Vec<Item> },
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
    /// another provider are dropped with a warning.
    pub fn replace_items(&self, items: Vec<Item>) {
        let items: Vec<Item> = items
            .into_iter()
            .filter(|item| {
                let ok = item.id.provider() == self.provider;
                if !ok {
                    tracing::warn!(
                        module = %self.provider,
                        item = %item.id.as_str(),
                        "dropping item with foreign provider"
                    );
                }
                ok
            })
            .collect();
        let ev = CoreEvent::ReplaceItems {
            provider: self.provider.clone(),
            items,
        };
        // The channel is unbounded; it only fails when the core is gone.
        if self.tx.try_send(ev).is_err() {
            tracing::debug!(module = %self.provider, "core is gone, items not delivered");
        }
    }
}

pub trait Module: Send + 'static {
    /// Provider id: the prefix of every item id this module publishes.
    fn id(&self) -> &'static str;
    fn start(&mut self, ctx: ModuleContext);
    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<ActionOutcome>;
}
