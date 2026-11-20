//! Readings of a query: the same text read as typed on another keyboard layout, or romanized.

use crate::layout::LAYOUTS;
use crate::script::{Scripts, letter_count};
use crate::translit::romanize;

/// At most this many readings are produced for a query.
pub const MAX_READINGS: usize = 8;
/// Queries longer than this (in characters) get no readings.
pub const MAX_READING_QUERY_CHARS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingKind {
    /// The same keys on another layout.
    Layout,
    /// A romanization of a non-Latin query.
    Translit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryReading {
    pub kind: ReadingKind,
    pub text: String,
    /// The script the text is in. Not sent over WIT.
    pub script: Scripts,
}

/// The script of a query that can have readings: two or more letters, all in one of
/// Latin, Cyrillic, Georgian or Armenian.
pub(crate) fn readable_script(query: &str) -> Option<Scripts> {
    if query.chars().count() > MAX_READING_QUERY_CHARS || letter_count(query) < 2 {
        return None;
    }
    Scripts::of(query)
        .single()
        .filter(|s| !s.contains(Scripts::OTHER))
}

/// Readings of `query` other than as typed. `index_scripts` are the scripts present in the
/// index; a Latin query is only remapped to those.
pub fn query_readings(query: &str, index_scripts: Scripts) -> Vec<QueryReading> {
    let Some(script) = readable_script(query) else {
        return Vec::new();
    };
    let mut out: Vec<QueryReading> = Vec::new();
    let mut push = |kind, text: String, script| {
        let lower = text.to_lowercase();
        if lower != query.to_lowercase() && !out.iter().any(|r| r.text.to_lowercase() == lower) {
            out.push(QueryReading { kind, text, script });
        }
    };
    if script == Scripts::LATIN {
        for layout in LAYOUTS.iter().filter(|l| index_scripts.contains(l.script)) {
            push(ReadingKind::Layout, layout.from_latin(query), layout.script);
        }
    } else {
        for layout in LAYOUTS.iter().filter(|l| l.script == script) {
            if let Some(text) = layout.to_latin(query) {
                push(ReadingKind::Layout, text, Scripts::LATIN);
            }
        }
        push(ReadingKind::Translit, romanize(query), Scripts::LATIN);
    }
    out.truncate(MAX_READINGS);
    out
}

/// For a query in one non-Latin script, its reading through the first layout table that
/// covers it. Otherwise the query. Always lowercase.
pub fn normalize_layout(query: &str) -> String {
    // Lowercase first: capitals on shifted punctuation keys (`Х` is `{`) do not map back to
    // the base key of their lowercase letter.
    let query = query.to_lowercase();
    let query = query.as_str();
    if let Some(script) = Scripts::of(query)
        .single()
        .filter(|s| *s != Scripts::LATIN && !s.contains(Scripts::OTHER))
        && let Some(text) = LAYOUTS
            .iter()
            .filter(|l| l.script == script)
            .find_map(|l| l.to_latin(query))
    {
        return text.to_lowercase();
    }
    query.to_lowercase()
}
