//! The root-search phrase that asks for a translation: `<text> in|into <language>` and
//! `<text> на <language>` / `<text> по-<language>`, as Raycast's `hello in german`.
//!
//! Only a whole language name matches (no prefixes, no codes), and `to` is not a trigger, so
//! `go to port`, `log in` and `check in gym` send nothing.

use tomakast_plugin::QueryReading;

use crate::langs;

/// The longest text a phrase translates, in characters.
pub const MAX_TEXT: usize = 500;
/// The longest query looked at, in bytes: [`MAX_TEXT`] characters of up to 4 bytes, quotes,
/// a marker and a language name.
const MAX_QUERY_BYTES: usize = MAX_TEXT * 4 + 128;
/// No language name is longer than this, in bytes (`азербайджанском` is 30).
const MAX_NAME_BYTES: usize = 64;

#[derive(Debug, PartialEq)]
pub struct Phrase {
    pub text: String,
    /// Google's code of the target.
    pub to: &'static str,
}

/// The phrase in `query`, if it is one.
pub fn parse(query: &str) -> Option<Phrase> {
    let query = query.trim();
    // The text, a marker and the longest language name: anything longer is no phrase, and
    // checking it would cost search's deadline.
    if query.len() > MAX_QUERY_BYTES || query.contains('\n') {
        return None;
    }
    // `<text> по-немецки`: the last word is an adverb.
    if let Some((text, last)) = query.rsplit_once(char::is_whitespace)
        && let Some(lang) = langs::by_adverb(last)
        && let Some(text) = clean(text)
    {
        return Some(Phrase {
            text,
            to: lang.code,
        });
    }
    // `<text> in|into|на <language>`, the marker searched from the right.
    for (at, marker) in markers(query).into_iter().rev() {
        let (text, name) = (&query[..at], &query[at + marker.len()..]);
        if name.len() > MAX_NAME_BYTES {
            // Markers further left only leave longer tails.
            break;
        }
        if let Some(lang) = langs::by_name(name)
            && let Some(text) = clean(text)
        {
            return Some(Phrase {
                text,
                to: lang.code,
            });
        }
    }
    None
}

/// [`parse`] of the query as typed, else of its readings in order (`ghbdtn yf fyukbqcrjv`
/// typed on the wrong layout reads `привет на английском`).
pub fn parse_any(query: &str, readings: &[QueryReading]) -> Option<Phrase> {
    parse(query).or_else(|| readings.iter().find_map(|r| parse(&r.text)))
}

/// Where ` in `, ` into ` and ` на ` stand in `query` (any case), with the marker. A match
/// spans exactly the marker's bytes: both cases of its letters have the same length.
fn markers(query: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (at, _) in query.char_indices() {
        for marker in [" in ", " into ", " на "] {
            let Some(rest) = query.get(at..at + marker.len()) else {
                continue;
            };
            let same = rest.chars().count() == marker.chars().count()
                && rest
                    .chars()
                    .zip(marker.chars())
                    .all(|(c, m)| c.to_lowercase().eq(m.to_lowercase()));
            if same {
                found.push((at, marker));
            }
        }
    }
    found
}

/// The text before the marker: trimmed, one pair of surrounding quotes dropped, 1 to
/// [`MAX_TEXT`] characters.
fn clean(text: &str) -> Option<String> {
    let mut text = text.trim();
    for (open, close) in [('"', '"'), ('«', '»'), ('\'', '\''), ('“', '”'), ('„', '“')] {
        if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
            text = inner.trim();
            break;
        }
    }
    let n = text.chars().count();
    (1..=MAX_TEXT).contains(&n).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_plugin::QueryReadingKind;

    fn phrase(text: &str, to: &'static str) -> Option<Phrase> {
        Some(Phrase {
            text: text.into(),
            to,
        })
    }

    #[test]
    fn phrases_that_ask() {
        assert_eq!(parse("hello in russian"), phrase("hello", "ru"));
        assert_eq!(
            parse("\"good night\" into French"),
            phrase("good night", "fr")
        );
        assert_eq!(parse("привет на немецком"), phrase("привет", "de"));
        assert_eq!(parse("привет на немецкий"), phrase("привет", "de"));
        assert_eq!(parse("спасибо по-английски"), phrase("спасибо", "en"));
        assert_eq!(parse("«как дела» in english"), phrase("как дела", "en"));
        assert_eq!(parse("bring it in in german"), phrase("bring it in", "de"));
        assert_eq!(
            parse("Hello In Chinese (Simplified)"),
            phrase("Hello", "zh-CN")
        );
        assert_eq!(parse("hola auf in deutsch"), phrase("hola auf", "de"));
    }

    #[test]
    fn queries_that_do_not() {
        let long = format!("{} in german", "x".repeat(MAX_TEXT + 1));
        for query in [
            "go to port",
            "log in",
            "check in gym",
            "in russian",
            "hello in de",
            "hello in port",
            "hello to german",
            "firefox",
            "по-английски",
            "\"\" in german",
            "a\nb in german",
            long.as_str(),
        ] {
            assert_eq!(parse(query), None, "{query:?}");
        }
    }

    /// Review finding 7: a huge query is turned down at once, inside search's deadline.
    #[test]
    fn huge_queries_are_cheap() {
        let huge = format!("{} in german", "word in other ".repeat(20_000));
        let started = std::time::Instant::now();
        assert_eq!(parse(&huge), None);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn many_markers_stay_cheap() {
        let many = format!("{}x in german", "in ".repeat(160));
        let started = std::time::Instant::now();
        assert!(parse(&many).is_some());
        assert!(
            started.elapsed() < std::time::Duration::from_millis(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn readings_count() {
        let readings = [QueryReading {
            kind: QueryReadingKind::Layout,
            text: "привет на английском".into(),
        }];
        assert_eq!(
            parse_any("ghbdtn yf fyukbqcrjv", &readings),
            phrase("привет", "en")
        );
        assert_eq!(parse_any("ghbdtn", &[]), None);
    }
}
