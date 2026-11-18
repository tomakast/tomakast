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
        let title = pattern
            .0
            .score(Utf32Str::new(&item.title, &mut self.buf), &mut self.inner);
        let mut keyword: Option<u32> = None;
        for k in &item.keywords {
            let s = pattern
                .0
                .score(Utf32Str::new(k, &mut self.buf), &mut self.inner);
            keyword = keyword.max(s);
        }
        let keyword = keyword.map(|s| s * 3 / 4);
        title.max(keyword)
    }
}
