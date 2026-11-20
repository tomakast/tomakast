//! Launcher query history: the entries and the rule for what is worth keeping.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::store::Store;

/// The most entries kept; the newest win.
pub const HISTORY_CAP: usize = 1000;

/// A query shorter than this (trimmed, in chars) is not recorded.
pub const MIN_QUERY_CHARS: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: String,
    pub query: String,
    pub at_unix: i64,
}

/// The trimmed query, or `None` when it is too short to record.
pub(crate) fn recordable(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    (trimmed.chars().count() >= MIN_QUERY_CHARS).then_some(trimmed)
}

/// The launcher's query history: newest first, at most [`HISTORY_CAP`], shared by clones.
#[derive(Clone)]
pub struct QueryHistory {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    store: Store,
    entries: Vec<HistoryEntry>,
}

impl QueryHistory {
    /// Prunes `store` to [`HISTORY_CAP`], then loads it newest first.
    pub fn load(store: Store) -> anyhow::Result<QueryHistory> {
        store.prune_query_history(HISTORY_CAP)?;
        let entries = store.load_query_history(HISTORY_CAP)?;
        Ok(QueryHistory {
            inner: Arc::new(Mutex::new(Inner { store, entries })),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records `query` used at `at_unix` if [`recordable`]; returns whether it did.
    /// A store error is logged (no query text) and the change stays in memory.
    pub fn record(&self, query: &str, at_unix: i64) -> bool {
        let Some(query) = recordable(query) else {
            return false;
        };
        let mut inner = self.lock();
        let mut entry = match inner.entries.iter().position(|e| e.query == query) {
            Some(i) => inner.entries.remove(i),
            None => HistoryEntry {
                id: uuid::Uuid::new_v4().to_string(),
                query: query.to_string(),
                at_unix,
            },
        };
        entry.at_unix = at_unix;
        inner.entries.insert(0, entry);
        let dropped = (inner.entries.len() > HISTORY_CAP).then(|| inner.entries.pop());
        let id = inner.entries[0].id.clone();
        match inner.store.upsert_query(&id, query, at_unix) {
            Ok(id) => inner.entries[0].id = id,
            Err(err) => tracing::error!(error = %err, "failed to persist a history entry"),
        }
        if let Some(dropped) = dropped.flatten()
            && let Err(err) = inner.store.delete_query(&dropped.id)
        {
            tracing::error!(id = %dropped.id, error = %err, "failed to drop an old history entry");
        }
        true
    }

    pub fn entries(&self) -> Vec<HistoryEntry> {
        self.lock().entries.clone()
    }

    /// The query `back` steps from the newest (0 = newest).
    pub fn nth_query(&self, back: usize) -> Option<String> {
        self.lock().entries.get(back).map(|e| e.query.clone())
    }

    /// Disk first; memory only after the store succeeded.
    pub fn delete(&self, id: &str) -> anyhow::Result<bool> {
        let mut inner = self.lock();
        let removed = inner.store.delete_query(id)?;
        inner.entries.retain(|e| e.id != id);
        Ok(removed)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let mut inner = self.lock();
        inner.store.clear_query_history()?;
        inner.entries.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordable_trims_and_counts_chars() {
        assert_eq!(recordable("  Firefox "), Some("Firefox"));
        assert_eq!(recordable("é"), None);
        assert_eq!(recordable("éé"), Some("éé"));
        assert_eq!(recordable("   "), None);
        assert_eq!(recordable(" a "), None);
    }
}
