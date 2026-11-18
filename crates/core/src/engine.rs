//! Search engine: fuzzy matching combined with usage-aware ranking.

use std::sync::Arc;

use crate::index::{Index, Item, ItemId};
use crate::matcher::{Matcher, Pattern};
use crate::usage::{
    LocalNow, RankWeights, UsageModel, UsageRecord, UsageStats, combined_score, usage_value,
};

/// Match score of every dynamic-source item, so answers (calculator,
/// converter) sort above static matches.
pub const DYNAMIC_MATCH_SCORE: u32 = 1_000;

#[derive(Clone, Debug)]
pub struct Hit {
    pub item: Arc<Item>,
    pub score: f64,
    /// Computed by a dynamic source for this query, not matched in the index.
    pub dynamic: bool,
}

pub struct SearchEngine {
    index: Index,
    matcher: Matcher,
    usage: UsageModel,
    weights: RankWeights,
}

impl SearchEngine {
    pub fn new(usage: UsageModel, weights: RankWeights) -> SearchEngine {
        SearchEngine {
            index: Index::new(),
            matcher: Matcher::new(),
            usage,
            weights,
        }
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn index_mut(&mut self) -> &mut Index {
        &mut self.index
    }

    /// Empty query: every item by usage. Otherwise: matching items by match score plus usage.
    /// Ties are broken by case-insensitive title, ascending.
    pub fn search(&mut self, query: &str, now: LocalNow) -> Vec<Hit> {
        self.search_with(query, now, &[])
    }

    /// Like [`search`](Self::search), merged with `dynamic`: items a dynamic
    /// source computed for `query`. They are not matched again; each scores
    /// [`DYNAMIC_MATCH_SCORE`] plus its usage.
    pub fn search_with(&mut self, query: &str, now: LocalNow, dynamic: &[Arc<Item>]) -> Vec<Hit> {
        let stats = self.usage.stats(query, now, &self.weights);
        let pattern = Pattern::parse(query);
        let no_usage = UsageStats::default();
        let mut hits = Vec::with_capacity(self.index.len() + dynamic.len());
        for item in dynamic {
            let usage = usage_value(stats.get(&item.id).unwrap_or(&no_usage), &self.weights);
            hits.push((
                item.title.to_lowercase(),
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
                usage
            } else {
                match self.matcher.score(&pattern, item) {
                    Some(m) => combined_score(m, usage, &self.weights),
                    None => continue,
                }
            };
            hits.push((
                item.title.to_lowercase(),
                Hit {
                    item: Arc::clone(item),
                    score,
                    dynamic: false,
                },
            ));
        }
        // The lowercase title is computed once per hit, not once per comparison.
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
