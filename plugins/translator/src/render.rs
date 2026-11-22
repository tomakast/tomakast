//! The pad's output pane, as markdown, and its title.
//!
//! The launcher renders a CommonMark subset (pulldown-cmark): the translation is escaped so it
//! shows exactly as Google returned it, line breaks and all. The dictionary parts are fixed
//! markdown around escaped values, each capped.

use crate::google::{Failure, Translation};

/// The most a compose pad's `output` may hold (the launcher's cap).
pub const OUTPUT_CAP: usize = 256 * 1024;
const TERMS_PER_POS: usize = 8;
const MEANINGS_PER_POS: usize = 3;
const EXAMPLES: usize = 5;
const SYNONYM_GROUPS_PER_POS: usize = 3;
/// Room kept under [`OUTPUT_CAP`] for the notices and the dictionary of a cut translation.
const RESERVE: usize = 1024;

const CUT_INPUT: &str = "Translated the first 30 000 characters.";
const CUT_OUTPUT: &str = "The translation is too long to show whole.";

/// `text` as markdown that shows as written: punctuation that means something is
/// backslash-escaped, a line that would start a list, a heading or a code block is defused,
/// a single line break becomes a hard break, and blank lines keep paragraphs apart.
pub fn escape(text: &str) -> String {
    // One block, every line break a hard break, an empty line an empty line: the text keeps
    // the shape it has in the editor, paragraph gaps included.
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut out = String::new();
            if line.trim().is_empty() {
                out.push('\u{a0}');
            } else {
                escape_line(line, &mut out);
            }
            out
        })
        .collect();
    lines.join("  \n")
}

fn escape_line(line: &str, out: &mut String) {
    // Leading spaces would make an indented code block: keep them as no-break spaces.
    let body = line.trim_start_matches([' ', '\t']);
    for c in line[..line.len() - body.len()].chars() {
        out.push_str(if c == '\t' {
            "\u{a0}\u{a0}\u{a0}\u{a0}"
        } else {
            "\u{a0}"
        });
    }
    // An ordered-list marker: digits then `.` or `)`.
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let ordered = digits > 0 && matches!(body.as_bytes().get(digits), Some(b'.' | b')'));
    for (i, c) in body.char_indices() {
        let at_start = i == 0;
        let special = matches!(
            c,
            '\\' | '`'
                | '*'
                | '_'
                | '['
                | ']'
                | '<'
                | '>'
                | '#'
                | '|'
                | '~'
                | '!'
                | '('
                | ')'
                | '&'
                | '{'
                | '}'
        ) || (at_start && matches!(c, '-' | '+' | '='))
            || (ordered && i == digits);
        if special {
            out.push('\\');
        }
        out.push(c);
    }
}

/// The output and the detail for a translation: the translation itself, as typed; then, at
/// the body size, "Did you mean", the transliterations, the dictionary parts Google gave and
/// the notes on a cut. Together at most [`OUTPUT_CAP`] bytes.
pub fn pane(t: &Translation, input_cut: bool) -> (String, Option<String>) {
    let mut body = escape(&t.text);
    let mut detail = String::new();
    if let Some(spell) = &t.spell {
        detail.push_str(&format!("Did you mean **{}**?", escape(spell)));
    }
    let translits: Vec<String> = [&t.translit, &t.src_translit]
        .into_iter()
        .flatten()
        .map(|s| escape(s))
        .collect();
    if !translits.is_empty() {
        detail.push_str(&format!("\n\n*{}*", translits.join(" · ")));
    }
    detail.push_str(&dictionary(t));
    let mut notes = Vec::new();
    if input_cut {
        notes.push(CUT_INPUT);
    }
    let room = OUTPUT_CAP - RESERVE - detail.len().min(OUTPUT_CAP / 2);
    if body.len() > room {
        body = cut_at_line(&body, room).to_string();
        notes.push(CUT_OUTPUT);
        detail.clear();
    }
    for note in notes {
        detail.push_str(&format!("\n\n*{note}*"));
    }
    let detail = detail.trim_start_matches('\n').to_string();
    (body, (!detail.is_empty()).then_some(detail))
}

/// The longest start of `markdown` within `max` bytes that ends at a line end (else at a
/// character boundary).
fn cut_at_line(markdown: &str, max: usize) -> &str {
    let mut end = max.min(markdown.len());
    while !markdown.is_char_boundary(end) {
        end -= 1;
    }
    match markdown[..end].rfind('\n') {
        Some(nl) if nl > 0 => &markdown[..nl],
        _ => &markdown[..end],
    }
}

fn dictionary(t: &Translation) -> String {
    let mut out = String::new();
    if !t.dict.is_empty() {
        out.push_str("\n\n### Translations\n");
        for part in &t.dict {
            let terms: Vec<String> = part
                .terms
                .iter()
                .take(TERMS_PER_POS)
                .map(|w| escape(w))
                .collect();
            out.push_str(&format!("\n{}{}", pos_label(&part.pos), terms.join(", ")));
            out.push_str("  ");
        }
    }
    if !t.definitions.is_empty() {
        out.push_str("\n\n### Meanings\n");
        for part in &t.definitions {
            if !part.pos.is_empty() {
                out.push_str(&format!("\n*{}*\n", escape(&part.pos)));
            }
            for (n, d) in part.entries.iter().take(MEANINGS_PER_POS).enumerate() {
                out.push_str(&format!("\n{}. {}", n + 1, escape(&d.gloss)));
                if !d.labels.is_empty() {
                    let labels: Vec<String> = d.labels.iter().map(|l| escape(l)).collect();
                    out.push_str(&format!(" *({})*", labels.join(", ")));
                }
                if let Some(example) = &d.example {
                    out.push_str(&format!("  \n   *“{}”*", escape(example)));
                }
            }
            out.push('\n');
        }
    }
    if !t.examples.is_empty() {
        out.push_str("\n\n### Examples\n");
        for example in t.examples.iter().take(EXAMPLES) {
            out.push_str(&format!("\n- {}", escape(example)));
        }
    }
    if !t.synsets.is_empty() {
        out.push_str("\n\n### Synonyms\n");
        for part in &t.synsets {
            let groups: Vec<String> = part
                .groups
                .iter()
                .take(SYNONYM_GROUPS_PER_POS)
                .map(|g| {
                    g.iter()
                        .take(TERMS_PER_POS)
                        .map(|w| escape(w))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .collect();
            out.push_str(&format!(
                "\n{}{}  ",
                pos_label(&part.pos),
                groups.join("; ")
            ));
        }
    }
    out
}

fn pos_label(pos: &str) -> String {
    if pos.is_empty() {
        String::new()
    } else {
        format!("*{}*: ", escape(pos))
    }
}

/// The empty editor's placeholder; the languages are on the bar, the actions in Ctrl+K.
pub const PLACEHOLDER: &str = "Type or paste text to translate.";

/// The hint for English text when no second language is known.
pub fn pick_target_hint() -> String {
    "This text is in English and no second language is set: pick a target language.".into()
}

/// A failure, as one line.
pub fn failure(f: &Failure) -> String {
    escape(&f.message())
}

/// The pad's title: what it does, never the languages (the bar shows them) nor its state
/// (the launcher's busy line does).
pub const TITLE: &str = "Translate";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::tests::{NAME, RUN, SENTENCE, SPELL, answer};

    #[test]
    fn escaping_defuses_markdown() {
        let text = "# T\n*x* _y_ [a](b) <b> & `c`\n1. z\n- item\n    code?";
        let md = escape(text);
        assert_eq!(
            md,
            "\\# T  \n\\*x\\* \\_y\\_ \\[a\\]\\(b\\) \\<b\\> \\& \\`c\\`  \n1\\. z  \n\\- item  \n\u{a0}\u{a0}\u{a0}\u{a0}code?"
        );
    }

    /// The text the launcher's parser (pulldown-cmark, as `crates/ui/src/markdown.rs` uses
    /// it) shows for `md`: text and code as written, a hard break as a line break, and a
    /// paragraph end as a blank line. Any other construct is a failure.
    fn rendered(md: &str) -> String {
        use pulldown_cmark::{Event, Parser, Tag, TagEnd};
        let mut out = String::new();
        for event in Parser::new(md) {
            match event {
                Event::Text(t) => out.push_str(&t),
                Event::HardBreak => out.push('\n'),
                Event::SoftBreak => out.push(' '),
                Event::Start(Tag::Paragraph) => {}
                Event::End(TagEnd::Paragraph) => out.push_str("\n\n"),
                other => panic!("{other:?} in {md:?}"),
            }
        }
        // An empty line is kept as a no-break space: it shows as an empty line.
        out.trim_end_matches('\n')
            .replace('\u{a0}', " ")
            .split('\n')
            .map(|line| if line.trim().is_empty() { "" } else { line })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn escaped_text_renders_as_written() {
        for text in [
            "# Title\n*x* _y_ [a](b) <b>bold</b> &amp; `c` ~~s~~ | t |",
            "1. one\n2) two\n- dash\n+ plus\n> quote\n===\n---",
            "    indented four\n\tand a tab",
            "first paragraph\nsame paragraph\n\nsecond paragraph",
            "![img](http://x) <https://example.com> http://example.com",
            "back\\slash \\* and {braces} and #hash at end #",
            "Привет, *мир*! 1. не список",
        ] {
            assert_eq!(
                rendered(&escape(text)),
                text.replace('\t', "    "),
                "{text:?}"
            );
        }
    }

    /// One block: an empty line between paragraphs is an empty line, as in the editor, not
    /// a paragraph gap of another height.
    #[test]
    fn paragraphs_keep_their_empty_lines() {
        assert_eq!(escape("one\n\ntwo"), "one  \n\u{a0}  \ntwo");
        assert_eq!(escape("one\ntwo"), "one  \ntwo");
        assert_eq!(escape("a\r\nb"), "a  \nb");
        assert_eq!(rendered(&escape("one\n\n\ntwo")), "one\n\n\ntwo");
        assert_eq!(
            escape("a\n   \nb"),
            "a  \n\u{a0}  \nb",
            "a blank line is empty"
        );
    }

    #[test]
    fn a_word_has_its_dictionary() {
        let (output, detail) = pane(&answer(RUN), false);
        assert_eq!(output, "бегать");
        let md = detail.expect("the dictionary");
        let at = |h: &str| md.find(h).unwrap_or_else(|| panic!("{h} in {md}"));
        assert!(at("### Translations") < at("### Meanings"));
        assert!(at("### Meanings") < at("### Examples"));
        assert!(at("### Examples") < at("### Synonyms"));
        let translations = &md[at("### Translations")..at("### Meanings")];
        let verbs = translations
            .lines()
            .find(|l| l.starts_with("*verb*"))
            .unwrap();
        assert_eq!(verbs.matches(", ").count(), TERMS_PER_POS - 1, "{verbs}");
        let meanings = &md[at("### Meanings")..at("### Examples")];
        assert!(!meanings.contains("\n4. "), "{meanings}");
        let examples = &md[at("### Examples")..at("### Synonyms")];
        assert_eq!(examples.matches("\n- ").count(), EXAMPLES);
        assert!(md.starts_with("*begat'"), "the transliteration first: {md}");
    }

    #[test]
    fn no_dictionary_no_detail() {
        for fixture in [NAME, SENTENCE] {
            let (output, detail) = pane(&answer(fixture), false);
            assert!(!output.contains("###"), "{output}");
            assert!(
                detail.as_deref().is_none_or(|d| !d.contains("###")),
                "{detail:?}"
            );
        }
        assert_eq!(pane(&answer(SENTENCE), false).1, None);
    }

    #[test]
    fn spelling_comes_first() {
        let detail = pane(&answer(SPELL), false).1.unwrap();
        assert!(detail.starts_with("Did you mean **receive**?"), "{detail}");
    }

    #[test]
    fn long_translations_fit() {
        let t = Translation {
            text: "Строка перевода.\n".repeat(20_000),
            src: "en".into(),
            ..Translation::default()
        };
        let (output, detail) = pane(&t, true);
        let detail = detail.unwrap();
        assert!(
            output.len() + detail.len() <= OUTPUT_CAP,
            "{}",
            output.len()
        );
        assert!(detail.ends_with(&format!("*{CUT_OUTPUT}*")));
        assert!(detail.contains(CUT_INPUT));
        let short = pane(&answer(SENTENCE), true).1.unwrap();
        assert!(short.ends_with(&format!("*{CUT_INPUT}*")));
    }
}
