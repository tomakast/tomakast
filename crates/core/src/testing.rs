//! Test doubles. Available with the `testing` feature.

use std::sync::{Arc, Mutex};

use crate::index::{Item, ItemId};
use crate::module::{ActionOutcome, Module, ModuleContext};

/// Module with id `fake` that publishes fixed items and records executed actions.
pub struct FakeModule {
    items: Vec<Item>,
    result: fn() -> anyhow::Result<ActionOutcome>,
    pub calls: Arc<Mutex<Vec<(ItemId, String)>>>,
}

impl FakeModule {
    pub fn new(items: Vec<Item>, result: fn() -> anyhow::Result<ActionOutcome>) -> FakeModule {
        FakeModule {
            items,
            result,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Module for FakeModule {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.replace_items(self.items.clone());
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<ActionOutcome> {
        self.calls
            .lock()
            .unwrap()
            .push((item.id.clone(), action_id.to_string()));
        (self.result)()
    }
}
