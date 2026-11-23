//! Fuzzy matching of a query against items, as typed and across keyboard layouts and scripts.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern as NucleoPattern};
use nucleo_matcher::{Config, Matcher as NucleoMatcher, Utf32Str, Utf32String};

use crate::index::Item;
use crate::readings::{QueryReading, ReadingKind, query_readings, readable_script};
use crate::script::Scripts;
use crate::translit::{skeleton, skeleton_alts};

/// Penalty on a layout-remap score, as (numerator, denominator).
pub const LAYOUT_FACTOR: (u32, u32) = (7, 8);
/// Penalty on a skeleton score, as (numerator, denominator).
pub const TRANSLIT_FACTOR: (u32, u32) = (3, 4);
/// A cross score counts only when it reaches this share of the score its pattern gets
/// against its own text.
pub const CROSS_MIN_RATIO: f64 = 0.8;

/// What a text needs for cross-script matching, computed once per text.
#[derive(Clone, Debug)]
pub struct TextKeys {
    scripts: Scripts,
    groups: Vec<SkeletonGroup>,
}

/// The skeleton (and its alternates) of the words of a text in one script.
#[derive(Clone, Debug)]
struct SkeletonGroup {
    script: Scripts,
    skeleton: Utf32String,
    alts: Vec<Utf32String>,
}

impl SkeletonGroup {
    fn new(script: Scripts, text: &str) -> SkeletonGroup {
        SkeletonGroup {
            script,
            skeleton: Utf32String::from(skeleton(text)),
            alts: skeleton_alts(text)
                .into_iter()
                .map(Utf32String::from)
                .collect(),
        }
    }
}

/// Every script a text can be split into, one bit each.
const SCRIPTS: [Scripts; 5] = [
    Scripts::LATIN,
    Scripts::CYRILLIC,
    Scripts::GEORGIAN,
    Scripts::ARMENIAN,
    Scripts::OTHER,
];

impl TextKeys {
    /// A text in one script has one skeleton group, the skeleton of the whole text. A text in
    /// several scripts has one group per script, built from its whitespace-separated words in
    /// that script; a word in several scripts, or with no letters, goes to every group.
    pub fn new(text: &str) -> TextKeys {
        let scripts = Scripts::of(text);
        let groups = if scripts == Scripts::default() {
            Vec::new()
        } else if scripts.single().is_some() {
            vec![SkeletonGroup::new(scripts, text)]
        } else {
            let words: Vec<(&str, Scripts)> = text
                .split_whitespace()
                .map(|w| (w, Scripts::of(w)))
                .collect();
            SCRIPTS
                .into_iter()
                .filter(|s| scripts.contains(*s))
                .map(|s| {
                    let part: Vec<&str> = words
                        .iter()
                        .filter(|(_, ws)| ws.contains(s) || *ws == Scripts::default())
                        .map(|(w, _)| *w)
                        .collect();
                    SkeletonGroup::new(s, &part.join(" "))
                })
                .collect()
        };
        TextKeys { scripts, groups }
    }

    pub fn scripts(&self) -> Scripts {
        self.scripts
    }
}

/// [`TextKeys`] for an item's title and each of its keywords, in order.
#[derive(Clone, Debug)]
pub struct ItemKeys {
    pub title: TextKeys,
    pub keywords: Vec<TextKeys>,
}

impl ItemKeys {
    pub fn new(item: &Item) -> ItemKeys {
        ItemKeys {
            title: TextKeys::new(&item.title),
            keywords: item.keywords.iter().map(|k| TextKeys::new(k)).collect(),
        }
    }

    /// The scripts of the title and all keywords.
    pub fn scripts(&self) -> Scripts {
        self.keywords
            .iter()
            .fold(self.title.scripts, |acc, k| acc.union(k.scripts))
    }
}

/// A pattern over a reading of the query, with the least raw score that counts.
struct Cross {
    pattern: NucleoPattern,
    floor: u32,
}

impl Cross {
    /// `None` when the pattern is empty or does not match its own text.
    fn new(text: &str, m: &mut Matcher) -> Option<Cross> {
        let pattern = nucleo_pattern(text);
        if pattern.atoms.is_empty() {
            return None;
        }
        let own = pattern.score(Utf32Str::new(text, &mut m.buf), &mut m.inner)?;
        Some(Cross {
            pattern,
            floor: (own as f64 * CROSS_MIN_RATIO) as u32,
        })
    }

    fn score(
        &self,
        haystack: Utf32Str<'_>,
        m: &mut NucleoMatcher,
        factor: (u32, u32),
    ) -> Option<u32> {
        self.pattern
            .score(haystack, m)
            .filter(|s| *s >= self.floor)
            .map(|s| s * factor.0 / factor.1)
    }
}

/// A layout remap, matched against the original text of texts in its script.
struct LayoutCross {
    script: Scripts,
    cross: Cross,
}

/// The query's skeleton, matched against a text's skeleton groups in other scripts.
struct SkeletonCross {
    query_script: Scripts,
    cross: Cross,
}

/// A parsed query. Built with `Pattern::new` semantics, so `^ $ ! '` are plain characters.
pub struct Pattern {
    typed: NucleoPattern,
    readings: Vec<QueryReading>,
    layouts: Vec<LayoutCross>,
    skeleton: Option<SkeletonCross>,
}

fn nucleo_pattern(text: &str) -> NucleoPattern {
    NucleoPattern::new(
        text,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    )
}

impl Pattern {
    /// The query as typed only.
    pub fn parse(query: &str) -> Pattern {
        Pattern {
            typed: nucleo_pattern(query),
            readings: Vec::new(),
            layouts: Vec::new(),
            skeleton: None,
        }
    }

    /// The query as typed plus its readings for an index holding `scripts`.
    pub fn across(query: &str, scripts: Scripts, m: &mut Matcher) -> Pattern {
        let readings = query_readings(query, scripts);
        let layouts = readings
            .iter()
            .filter(|r| r.kind == ReadingKind::Layout)
            .filter_map(|r| {
                Some(LayoutCross {
                    script: r.script,
                    cross: Cross::new(&r.text, m)?,
                })
            })
            .collect();
        let skeleton = readable_script(query).and_then(|query_script| {
            Some(SkeletonCross {
                query_script,
                cross: Cross::new(&skeleton(query), m)?,
            })
        });
        Pattern {
            typed: nucleo_pattern(query),
            readings,
            layouts,
            skeleton,
        }
    }

    pub fn readings(&self) -> &[QueryReading] {
        &self.readings
    }

    pub fn is_empty(&self) -> bool {
        self.typed.atoms.is_empty()
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
    /// An empty pattern matches everything with score 0. `keys` are `ItemKeys::new(item)`.
    pub fn score(&mut self, pattern: &Pattern, item: &Item, keys: &ItemKeys) -> Option<u32> {
        if pattern.is_empty() {
            return Some(0);
        }
        let title = self.score_text(pattern, &item.title, &keys.title);
        let mut keyword: Option<u32> = None;
        for (k, k_keys) in item.keywords.iter().zip(&keys.keywords) {
            keyword = keyword.max(self.score_text(pattern, k, k_keys));
        }
        let keyword = keyword.map(|s| s * 3 / 4);
        title.max(keyword)
    }

    /// Score of `text` alone: the best of as typed, a layout remap × [`LAYOUT_FACTOR`] and
    /// the skeleton (or an alternate) × [`TRANSLIT_FACTOR`]. `None` when nothing matches.
    /// An empty pattern matches everything with score 0. `keys` are `TextKeys::new(text)`.
    pub fn score_text(&mut self, pattern: &Pattern, text: &str, keys: &TextKeys) -> Option<u32> {
        if pattern.is_empty() {
            return Some(0);
        }
        let haystack = Utf32Str::new(text, &mut self.buf);
        let mut best = pattern.typed.score(haystack, &mut self.inner);
        for layout in pattern
            .layouts
            .iter()
            .filter(|l| keys.scripts.contains(l.script))
        {
            best = best.max(layout.cross.score(haystack, &mut self.inner, LAYOUT_FACTOR));
        }
        if let Some(sk) = &pattern.skeleton {
            for group in keys.groups.iter().filter(|g| g.script != sk.query_script) {
                for text_skeleton in std::iter::once(&group.skeleton).chain(&group.alts) {
                    best = best.max(sk.cross.score(
                        text_skeleton.slice(..),
                        &mut self.inner,
                        TRANSLIT_FACTOR,
                    ));
                }
            }
        }
        best
    }
}
