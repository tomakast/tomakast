//! Item model and the in-memory index shared by all providers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Longest valid item id, in bytes.
const MAX_ID_LEN: usize = 256;

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

    /// Whether an id coming from a module may be indexed: it contains `/`,
    /// has no newline or other control character, and is at most 256 bytes.
    pub fn is_valid(&self) -> bool {
        self.0.len() <= MAX_ID_LEN && self.0.contains('/') && !self.0.chars().any(char::is_control)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_id_validation() {
        assert!(ItemId::new("apps", "firefox.desktop").is_valid());
        // Not constructible through the public API; modules could only send it over a boundary.
        assert!(!ItemId("nofslash".into()).is_valid());
        assert!(!ItemId::new("a", "b\nc").is_valid());
        assert!(!ItemId::new("a", "b\u{7f}").is_valid());
        let at_limit = ItemId::new("a", &"x".repeat(MAX_ID_LEN - 2));
        assert_eq!(at_limit.as_str().len(), MAX_ID_LEN);
        assert!(at_limit.is_valid());
        assert!(!ItemId::new("a", &"x".repeat(MAX_ID_LEN - 1)).is_valid());
    }
}
