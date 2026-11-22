//! Google Translate's no-key endpoint, the one browser extensions use:
//! `POST https://translate.googleapis.com/translate_a/single?client=gtx&…` with the text in a
//! form body. One answer (`dj=1`, JSON) carries the translation, the detected language and,
//! for short input with the `dt` parts asked, the dictionary: other translations by part of
//! speech, meanings with examples, examples, synonyms, transliterations and spelling.
//!
//! The endpoint is unofficial: it can be rate-limited or closed. This module is the only
//! place that knows it; the rest of the plugin sees [`Translation`] and [`Failure`].

use tomakast_plugin::HttpRequest;

use crate::json::{self, Json};

/// The one host the plugin asks (its `http` permission).
pub const HOST: &str = "translate.googleapis.com";
/// The longest text sent, in characters; longer text is cut (see [`cut`]).
pub const MAX_CHARS: usize = 30_000;
/// Short input asks for the dictionary too: at most this many words…
const SHORT_WORDS: usize = 3;
/// …and this many characters, on one line.
const SHORT_CHARS: usize = 64;

/// What to translate, and between which languages (`from: None` detects).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Request {
    pub text: String,
    pub from: Option<String>,
    pub to: String,
}

/// A translation and what Google adds to it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Translation {
    /// The translation, Google's sentences joined; line breaks kept.
    pub text: String,
    /// The source language Google detected (or was given), as Google writes it.
    pub src: String,
    /// The translation in Latin letters (`begat'`), when Google gives one.
    pub translit: Option<String>,
    /// The source in Latin letters or a transcription (`rən`).
    pub src_translit: Option<String>,
    /// "Did you mean": the corrected source.
    pub spell: Option<String>,
    pub dict: Vec<PosTerms>,
    pub definitions: Vec<PosDefinitions>,
    pub examples: Vec<String>,
    pub synsets: Vec<PosSynonyms>,
}

/// Other translations of a word for one part of speech, best first.
#[derive(Clone, Debug, PartialEq)]
pub struct PosTerms {
    pub pos: String,
    pub terms: Vec<String>,
}

/// Meanings of a word for one part of speech, in the source language.
#[derive(Clone, Debug, PartialEq)]
pub struct PosDefinitions {
    pub pos: String,
    pub entries: Vec<Definition>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub gloss: String,
    pub example: Option<String>,
    /// Register, subject and other labels (`informal`, `Nautical`).
    pub labels: Vec<String>,
}

/// Groups of synonyms of a word for one part of speech.
#[derive(Clone, Debug, PartialEq)]
pub struct PosSynonyms {
    pub pos: String,
    pub groups: Vec<Vec<String>>,
}

/// Why there is no translation.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// HTTP 429 or 503: too many requests.
    Refused,
    /// Any other status but 200.
    Status(u16),
    /// No answer: the network, a timeout, the host's refusal.
    Offline,
    /// An answer that is not the JSON expected.
    Unexpected,
}

impl Failure {
    /// One line for the pane.
    pub fn message(&self) -> String {
        match self {
            Failure::Refused => "Google refused: too many requests. Try again in a minute.".into(),
            Failure::Status(status) => format!("Google answered {status}."),
            Failure::Offline => "No connection.".into(),
            Failure::Unexpected => "Google answered something unexpected.".into(),
        }
    }
}

/// Whether `text` asks for the dictionary: at most 3 words, one line, at most 64 characters.
pub fn is_short(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !text.contains('\n')
        && text.chars().count() <= SHORT_CHARS
        && text.split_whitespace().count() <= SHORT_WORDS
}

/// `text` cut to at most [`MAX_CHARS`] characters, at the last sentence end (`.`, `!`, `?`,
/// `…`, `。`) or line end before that point, else at the limit; `true` when it was cut.
pub fn cut(text: &str) -> (&str, bool) {
    let Some((limit, _)) = text.char_indices().nth(MAX_CHARS) else {
        return (text, false);
    };
    let head = &text[..limit];
    let end = head
        .char_indices()
        .filter(|(_, c)| matches!(c, '.' | '!' | '?' | '…' | '。' | '\n'))
        .map(|(i, c)| i + c.len_utf8())
        .next_back()
        .unwrap_or(limit);
    (&text[..end], true)
}

/// The POST for `req`; the dictionary parts are asked for short input only.
pub fn http_request(req: &Request) -> HttpRequest {
    let mut url = format!(
        "https://{HOST}/translate_a/single?client=gtx&sl={}&tl={}&hl=en&dj=1&dt=t",
        encode(req.from.as_deref().unwrap_or("auto")),
        encode(&req.to),
    );
    if is_short(&req.text) {
        url.push_str("&dt=bd&dt=md&dt=ex&dt=ss&dt=rm&dt=qca");
    }
    let body = format!("q={}", encode(cut(&req.text).0));
    HttpRequest::new("POST", url)
        .header(
            "content-type",
            "application/x-www-form-urlencoded;charset=utf-8",
        )
        .body(body.into_bytes())
}

/// Percent-encodes everything but `A-Z a-z 0-9 - . _ ~`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Reads an answer.
pub fn parse(status: u16, body: &[u8]) -> Result<Translation, Failure> {
    match status {
        200 => {}
        429 | 503 => return Err(Failure::Refused),
        other => return Err(Failure::Status(other)),
    }
    let text = std::str::from_utf8(body).map_err(|_| Failure::Unexpected)?;
    let root = json::parse(text).ok_or(Failure::Unexpected)?;
    let sentences = array(root.get("sentences")).ok_or(Failure::Unexpected)?;
    let mut out = Translation::default();
    for sentence in sentences {
        if let Some(trans) = string(sentence.get("trans")) {
            out.text.push_str(trans);
        }
        if let Some(t) = string(sentence.get("translit")).filter(|t| !t.is_empty()) {
            out.translit = Some(t.to_string());
        }
        if let Some(t) = string(sentence.get("src_translit")).filter(|t| !t.is_empty()) {
            out.src_translit = Some(t.to_string());
        }
    }
    out.src = string(root.get("src")).unwrap_or_default().to_string();
    out.spell = root
        .get("spell")
        .and_then(|s| string(s.get("spell_res")))
        .filter(|s| !s.is_empty())
        .map(String::from);
    for part in array(root.get("dict")).unwrap_or_default() {
        let terms = strings(part.get("terms"));
        if !terms.is_empty() {
            out.dict.push(PosTerms {
                pos: pos(part),
                terms,
            });
        }
    }
    for part in array(root.get("definitions")).unwrap_or_default() {
        let entries: Vec<Definition> = array(part.get("entry"))
            .unwrap_or_default()
            .iter()
            .filter_map(|e| {
                Some(Definition {
                    gloss: string(e.get("gloss"))?.to_string(),
                    example: string(e.get("example")).map(String::from),
                    labels: labels(e.get("label_info")),
                })
            })
            .collect();
        if !entries.is_empty() {
            out.definitions.push(PosDefinitions {
                pos: pos(part),
                entries,
            });
        }
    }
    out.examples = root
        .get("examples")
        .and_then(|e| array(e.get("example")))
        .unwrap_or_default()
        .iter()
        .filter_map(|e| string(e.get("text")))
        .map(plain)
        .collect();
    for part in array(root.get("synsets")).unwrap_or_default() {
        let groups: Vec<Vec<String>> = array(part.get("entry"))
            .unwrap_or_default()
            .iter()
            .map(|e| strings(e.get("synonym")))
            .filter(|g| !g.is_empty())
            .collect();
        if !groups.is_empty() {
            out.synsets.push(PosSynonyms {
                pos: pos(part),
                groups,
            });
        }
    }
    Ok(out)
}

fn array(value: Option<&Json>) -> Option<&[Json]> {
    match value {
        Some(Json::Arr(items)) => Some(items),
        _ => None,
    }
}

fn string(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Str(s)) => Some(s),
        _ => None,
    }
}

fn strings(value: Option<&Json>) -> Vec<String> {
    array(value)
        .unwrap_or_default()
        .iter()
        .filter_map(|v| string(Some(v)).map(String::from))
        .collect()
}

fn pos(part: &Json) -> String {
    string(part.get("pos")).unwrap_or_default().to_string()
}

/// Every string in a `label_info` object (`register`, `subject`, `labels`, …).
fn labels(info: Option<&Json>) -> Vec<String> {
    match info {
        Some(Json::Obj(members)) => members.iter().flat_map(|(_, v)| strings(Some(v))).collect(),
        _ => Vec::new(),
    }
}

/// An example as text: `<b>` and `</b>` dropped, the common entities decoded.
fn plain(html: &str) -> String {
    html.replace("<b>", "")
        .replace("</b>", "")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const RUN: &str = include_str!("../tests/fixtures/word-run-en-ru.json");
    pub const BEZHAT: &str = include_str!("../tests/fixtures/word-bezhat-ru-en.json");
    pub const SENTENCE: &str = include_str!("../tests/fixtures/sentence-en-ru.json");
    pub const TEXT: &str = include_str!("../tests/fixtures/text-en-ru.json");
    pub const TEXT_SOURCE: &str = include_str!("../tests/fixtures/text-en.txt");
    pub const SPELL: &str = include_str!("../tests/fixtures/spell-en-ru.json");
    pub const NAME: &str = include_str!("../tests/fixtures/name-en-ru.json");

    pub fn answer(fixture: &str) -> Translation {
        parse(200, fixture.as_bytes()).expect("a fixture parses")
    }

    #[test]
    fn a_word_brings_its_dictionary() {
        let t = answer(RUN);
        assert_eq!(t.text, "бегать");
        assert_eq!(t.src, "en");
        assert_eq!(t.translit.as_deref(), Some("begat'"));
        assert_eq!(t.src_translit.as_deref(), Some("rən"));
        assert_eq!(t.dict[0].pos, "verb");
        assert!(t.dict[0].terms.iter().any(|w| w == "бежать"));
        assert!(t.definitions[0].entries[0].example.is_some());
        assert!(
            t.examples.iter().all(|e| !e.contains('<')),
            "{:?}",
            t.examples
        );
        assert!(t.examples.iter().any(|e| e == "a cross-country run"));
        assert!(
            t.synsets[0]
                .groups
                .iter()
                .any(|g| g.iter().any(|w| w == "sprint"))
        );
        let labelled = t.definitions[1]
            .entries
            .iter()
            .find(|d| d.gloss.starts_with("diarrhea"))
            .unwrap();
        assert_eq!(labelled.labels, ["informal"]);
    }

    #[test]
    fn a_russian_word_has_russian_meanings() {
        let t = answer(BEZHAT);
        assert_eq!(t.src, "ru");
        assert_eq!(t.text, "run");
        let gloss = &t.definitions[0].entries[0].gloss;
        assert!(gloss.chars().any(|c| ('а'..='я').contains(&c)), "{gloss}");
    }

    #[test]
    fn a_text_keeps_its_paragraphs() {
        let t = answer(TEXT);
        assert_eq!(t.text.matches("\n\n").count(), 2, "{:?}", t.text);
        let (ours, theirs) = (TEXT_SOURCE.chars().count(), t.text.chars().count());
        assert!(
            theirs * 10 > ours * 7 && theirs * 10 < ours * 13,
            "{ours} vs {theirs}"
        );
        assert!(t.dict.is_empty() && t.definitions.is_empty());
    }

    #[test]
    fn spelling_and_names() {
        assert_eq!(answer(SPELL).spell.as_deref(), Some("receive"));
        let name = answer(NAME);
        assert!(name.dict.is_empty() && name.definitions.is_empty() && name.synsets.is_empty());
        assert_eq!(answer(SENTENCE).spell, None);
    }

    #[test]
    fn failures() {
        assert_eq!(parse(429, b""), Err(Failure::Refused));
        assert_eq!(parse(503, b""), Err(Failure::Refused));
        assert_eq!(parse(500, b""), Err(Failure::Status(500)));
        assert_eq!(parse(200, b"<html>"), Err(Failure::Unexpected));
        assert_eq!(parse(200, b"{}"), Err(Failure::Unexpected));
        assert_eq!(parse(200, &[0xff, 0xfe]), Err(Failure::Unexpected));
    }

    #[test]
    fn short_input() {
        for short in ["run", "take off", "бежать быстро очень", " run "] {
            assert!(is_short(short), "{short:?}");
        }
        for long in ["a b c d", "a\nb", "", &"x".repeat(65)] {
            assert!(!is_short(long), "{long:?}");
        }
    }

    #[test]
    fn long_text_is_cut_at_a_sentence() {
        let text = "Sentence number one is here. ".repeat(1500);
        let (head, was_cut) = cut(&text);
        assert!(was_cut);
        assert!(head.chars().count() <= MAX_CHARS);
        assert!(head.ends_with('.'), "{:?}", &head[head.len() - 10..]);
        assert_eq!(cut("short"), ("short", false));
        let unbroken = "x".repeat(MAX_CHARS + 5);
        assert_eq!(cut(&unbroken).0.len(), MAX_CHARS);
    }

    fn decode(form: &str) -> String {
        let bytes = form.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&form[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_request() {
        let req = Request {
            text: "a&b=c ё and more".into(),
            from: None,
            to: "ru".into(),
        };
        let http = http_request(&req);
        assert_eq!(http.method, "POST");
        assert!(
            http.url
                .starts_with("https://translate.googleapis.com/translate_a/single?")
        );
        assert!(http.url.contains("&sl=auto&tl=ru&"), "{}", http.url);
        assert!(
            !http.url.contains("dt=bd"),
            "four words are not short: {}",
            http.url
        );
        let body = String::from_utf8(http.body.unwrap()).unwrap();
        assert_eq!(decode(body.strip_prefix("q=").unwrap()), "a&b=c ё and more");

        let word = Request {
            text: "run".into(),
            from: Some("en".into()),
            to: "zh-CN".into(),
        };
        let http = http_request(&word);
        assert!(http.url.contains("&sl=en&tl=zh-CN&"), "{}", http.url);
        assert!(http.url.contains("&dt=bd&") && http.url.ends_with("&dt=qca"));
    }
}
