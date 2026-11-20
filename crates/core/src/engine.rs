//! Search engine: fuzzy matching combined with usage-aware ranking.

use std::collections::HashSet;
use std::sync::Arc;

use crate::index::{Index, Item, ItemId};
use crate::matcher::{Matcher, Pattern};
use crate::usage::{
    LocalNow, RankWeights, UsageModel, UsageRecord, UsageStats, combined_score, usage_value,
};

/// Match score of every dynamic-source item, so answers (calculator,
/// converter) sort above static matches.
pub const DYNAMIC_MATCH_SCORE: u32 = 1_000;

/// Score of every item of a pinned provider on the empty query: above any
/// usage score, so these items lead the list (and the launcher's
/// Suggestions). See [`SearchEngine::pin_provider`].
pub const PINNED_SCORE: f64 = f64::MAX;

#[derive(Clone, Debug)]
pub struct Hit {
    pub item: Arc<Item>,
    pub score: f64,
    /// Computed by a dynamic source for this query, not matched in the index.
    pub dynamic: bool,
}

/// Order of hits with equal scores; a total order. Variant order matters:
/// every dynamic hit sorts before every static one.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum TieBreak {
    /// Position in the dynamic answers: the providers' own order.
    Dynamic(usize),
    /// Lowercase title.
    Static(String),
}

pub struct SearchEngine {
    index: Index,
    matcher: Matcher,
    usage: UsageModel,
    weights: RankWeights,
    /// Providers whose items lead the empty query.
    pinned: HashSet<String>,
}

impl SearchEngine {
    pub fn new(usage: UsageModel, weights: RankWeights) -> SearchEngine {
        SearchEngine {
            index: Index::new(),
            matcher: Matcher::new(),
            usage,
            weights,
            pinned: HashSet::new(),
        }
    }

    /// Makes every item of `provider` lead the empty query with
    /// [`PINNED_SCORE`], above any usage; equal pinned items keep the title
    /// order. A typed query ranks them like any other item. Only for trusted
    /// built-in modules (the plugin review module): a module cannot pin
    /// itself, and its items get no usage records from the pin.
    pub fn pin_provider(&mut self, provider: &str) {
        self.pinned.insert(provider.to_string());
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn index_mut(&mut self) -> &mut Index {
        &mut self.index
    }

    /// Empty query: every item by usage, items of pinned providers first.
    /// Otherwise: matching items by match score plus usage.
    /// Ties are broken by case-insensitive title, ascending.
    pub fn search(&mut self, query: &str, now: LocalNow) -> Vec<Hit> {
        self.search_with(query, now, &[])
    }

    /// Like [`search`](Self::search), merged with `dynamic`: items a dynamic
    /// source computed for `query`. They are not matched again; each scores
    /// [`DYNAMIC_MATCH_SCORE`] plus its usage. A dynamic item whose id is
    /// indexed is skipped: the indexed item wins.
    ///
    /// Ties: dynamic items before static ones, dynamic items in the order of
    /// `dynamic`, static items by case-insensitive title.
    pub fn search_with(&mut self, query: &str, now: LocalNow, dynamic: &[Arc<Item>]) -> Vec<Hit> {
        let stats = self.usage.stats(query, now, &self.weights);
        let pattern = Pattern::parse(query);
        let no_usage = UsageStats::default();
        let mut hits = Vec::with_capacity(self.index.len() + dynamic.len());
        for (position, item) in dynamic.iter().enumerate() {
            if self.index.get(&item.id).is_some() {
                continue;
            }
            let usage = usage_value(stats.get(&item.id).unwrap_or(&no_usage), &self.weights);
            hits.push((
                TieBreak::Dynamic(position),
                Hit {
                    item: Arc::clone(item),
                    score: combined_score(DYNAMIC_MATCH_SCORE, usage, &self.weights),
                    dynamic: true,
                },
            ));
        }
        for item in self.index.iter() {
            let usage = usage_value(stats.get(&item.id).unwrap_or(&no_usage), &self.weights);
            let score = if pattern.is_empty() {
                if self.pinned.contains(item.id.provider()) {
                    PINNED_SCORE
                } else {
                    usage
                }
            } else {
                match self.matcher.score(&pattern, item) {
                    Some(m) => combined_score(m, usage, &self.weights),
                    None => continue,
                }
            };
            hits.push((
                TieBreak::Static(item.title.to_lowercase()),
                Hit {
                    item: Arc::clone(item),
                    score,
                    dynamic: false,
                },
            ));
        }
        // The tie-break key is computed once per hit, not once per comparison.
        hits.sort_by(|(ta, a), (tb, b)| b.score.total_cmp(&a.score).then_with(|| ta.cmp(tb)));
        hits.into_iter().map(|(_, hit)| hit).collect()
    }

    /// Adds a launch to the usage model and returns the record so the caller can persist it.
    pub fn record_launch(&mut self, item: &ItemId, query: &str, now: LocalNow) -> UsageRecord {
        let record = UsageRecord::new(item.clone(), query, now);
        self.usage.push(record.clone());
        record
    }
}
