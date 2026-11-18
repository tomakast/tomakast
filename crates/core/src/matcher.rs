//! Fuzzy matching of a query against items.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern as NucleoPattern};
use nucleo_matcher::{Config, Matcher as NucleoMatcher, Utf32Str};

use crate::index::Item;

/// A parsed query. Built with `Pattern::new` semantics, so `^ $ ! '` are plain characters.
pub struct Pattern(NucleoPattern);

impl Pattern {
    pub fn parse(query: &str) -> Pattern {
        Pattern(NucleoPattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        ))
    }

    pub fn is_empty(&self) -> bool {
        self.0.atoms.is_empty()
    }
}

pub struct Matcher {
    inner: NucleoMatcher,
    buf: Vec<char>,
}

impl Default for Matcher {
    fn default() -> Self {
        Matcher::new()
    }
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher {
            inner: NucleoMatcher::new(Config::DEFAULT),
            buf: Vec::new(),
        }
    }

    /// Score = max(title score, best keyword score * 3 / 4); `None` when nothing matches.
    /// An empty pattern matches everything with score 0.
    pub fn score(&mut self, pattern: &Pattern, item: &Item) -> Option<u32> {
        if pattern.is_empty() {
            return Some(0);
        }
        let title = self.score_text(pattern, &item.title);
        let mut keyword: Option<u32> = None;
        for k in &item.keywords {
            keyword = keyword.max(self.score_text(pattern, k));
        }
        let keyword = keyword.map(|s| s * 3 / 4);
        title.max(keyword)
    }

    /// Score of `text` alone; `None` when it does not match. An empty pattern
    /// matches everything with score 0.
    pub fn score_text(&mut self, pattern: &Pattern, text: &str) -> Option<u32> {
        if pattern.is_empty() {
            return Some(0);
        }
        pattern
            .0
            .score(Utf32Str::new(text, &mut self.buf), &mut self.inner)
    }
}
