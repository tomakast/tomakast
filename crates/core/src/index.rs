//! Item model and the in-memory index shared by all providers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Stable item identifier of the form `<provider-id>/<item-id>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(String);

impl ItemId {
    pub fn new(provider: &str, local: &str) -> ItemId {
        ItemId(format!("{provider}/{local}"))
    }

    /// Parses a full identifier; returns `None` when there is no `/`.
    pub fn parse(s: &str) -> Option<ItemId> {
        s.contains('/').then(|| ItemId(s.to_string()))
    }

    pub fn provider(&self) -> &str {
        self.0.split_once('/').map_or(self.0.as_str(), |(p, _)| p)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Application,
    Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    File(PathBuf),
    Name(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub title: String,
}

/// A launchable entry. The first action is the primary one; `actions` is non-empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub title: String,
    pub subtitle: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<Icon>,
    pub kind: ItemKind,
    pub actions: Vec<Action>,
}

#[derive(Default)]
pub struct Index {
    items: HashMap<ItemId, Arc<Item>>,
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    /// Removes every item of `provider` and inserts `items` in their place.
    pub fn replace_provider(&mut self, provider: &str, items: Vec<Item>) {
        self.items.retain(|id, _| id.provider() != provider);
        for item in items {
            self.items.insert(item.id.clone(), Arc::new(item));
        }
    }

    pub fn get(&self, id: &ItemId) -> Option<&Arc<Item>> {
        self.items.get(id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<Item>> {
        self.items.values()
    }
}
