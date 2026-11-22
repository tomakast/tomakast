//! The user's language pair and which language a text goes to.
//!
//! The pair is (first, second): the user's own language and the other one. Text in the first
//! goes to the second; text in any other language goes to the first. A picked target or source
//! (a [`Choice`]) overrides that. Google detects the source in the same request that names
//! the target, so the plugin guesses the source first ([`first_target`]) and asks once more
//! when Google's detected source says the rule wanted another target ([`retry_target`]).

use crate::langs;

/// The user's languages, as Google's codes; `second` is unknown for an English-only system.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub first: String,
    pub second: Option<String>,
}

/// Languages fixed by the user (the pickers) or for one pad (swap, a tile, a recent row).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Choice {
    pub from: Option<String>,
    pub to: Option<String>,
}

impl Choice {
    /// `self` over `under`: a side fixed here wins, else `under`'s.
    pub fn over(&self, under: &Choice) -> Choice {
        Choice {
            from: self.from.clone().or_else(|| under.from.clone()),
            to: self.to.clone().or_else(|| under.to.clone()),
        }
    }
}

/// The `languages` preference: two languages, as codes or English names, separated by a
/// comma (`ru, en`, `German, English`); `None` unless both are known and differ.
pub fn from_preference(value: &str) -> Option<Pair> {
    let mut parts = value.split(',').map(str::trim).filter(|p| !p.is_empty());
    let first = lookup(parts.next()?)?;
    let second = lookup(parts.next()?)?;
    (parts.next().is_none() && first != second).then_some(Pair {
        first,
        second: Some(second),
    })
}

fn lookup(word: &str) -> Option<String> {
    langs::by_code(word)
        .or_else(|| langs::by_name(word))
        .map(|l| l.code.to_string())
}

/// The pair from the system's languages: the first that is not English, then English; an
/// English-only (or silent) system gives (English, unknown).
pub fn automatic(system: &[String]) -> Pair {
    let own = system
        .iter()
        .filter_map(|code| langs::by_code(code))
        .map(|l| l.code)
        .find(|code| *code != "en");
    match own {
        Some(own) => Pair {
            first: own.to_string(),
            second: Some("en".into()),
        },
        None => Pair {
            first: "en".into(),
            second: None,
        },
    }
}

/// Where text in `src` goes: the second language for the first, the first for any other.
/// `None` when that is the unknown second.
pub fn target_for(pair: &Pair, src: &str) -> Option<String> {
    if same(src, &pair.first) {
        pair.second.clone()
    } else {
        Some(pair.first.clone())
    }
}

/// The target to ask for first: the chosen one; else by the rule, for the chosen source, the
/// source this pad saw last, or a guess from the text's script. `None` when the text has no
/// target yet (English text on an English-only pair).
pub fn first_target(
    pair: &Pair,
    choice: &Choice,
    text: &str,
    previous_src: Option<&str>,
) -> Option<String> {
    if let Some(to) = &choice.to {
        return Some(to.clone());
    }
    if let Some(from) = &choice.from {
        return target_for(pair, from);
    }
    if let Some(target) = previous_src.and_then(|src| target_for(pair, src)) {
        return Some(target);
    }
    let script = Script::of_text(text);
    let first = Script::of_lang(&pair.first);
    if script != Script::Latin && script == first {
        // The first language's own script: probably the first language.
        target_for(pair, &pair.first)
    } else if script == Script::Latin && first == Script::Latin {
        // Cannot tell (German or English): guess "not the first language".
        Some(pair.first.clone())
    } else if pair
        .second
        .as_deref()
        .is_some_and(|s| Script::of_lang(s) == script)
    {
        Some(pair.first.clone())
    } else {
        target_for(pair, "")
    }
}

/// After an answer for `asked` whose source Google detected as `src`: the target to ask
/// again, when the rule wants another one. Never with a chosen target or source.
pub fn retry_target(pair: &Pair, choice: &Choice, asked: &str, src: &str) -> Option<String> {
    if choice.to.is_some() || choice.from.is_some() {
        return None;
    }
    target_for(pair, src).filter(|want| !same(want, asked))
}

/// "Russian ↔ English", or "English" alone.
pub fn label(pair: &Pair) -> String {
    match &pair.second {
        Some(second) => format!("{} ↔ {}", langs::name(&pair.first), langs::name(second)),
        None => langs::name(&pair.first),
    }
}

/// Whether two codes name one language (`he` and `iw`, `zh` and `zh-CN`).
pub fn same(a: &str, b: &str) -> bool {
    match (langs::by_code(a), langs::by_code(b)) {
        (Some(a), Some(b)) => a.code == b.code,
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// The writing systems the guess tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Han,
    Hangul,
    Kana,
    Devanagari,
    Thai,
    Georgian,
    Armenian,
    Other,
}

impl Script {
    fn of_char(c: char) -> Option<Script> {
        if !c.is_alphabetic() {
            return None;
        }
        Some(match c as u32 {
            0x0041..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
            0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
            0x0400..=0x052F => Script::Cyrillic,
            0x0530..=0x058F => Script::Armenian,
            0x0590..=0x05FF => Script::Hebrew,
            0x0600..=0x06FF | 0x0750..=0x077F => Script::Arabic,
            0x0900..=0x097F => Script::Devanagari,
            0x0E00..=0x0E7F => Script::Thai,
            0x10A0..=0x10FF => Script::Georgian,
            0x3040..=0x30FF => Script::Kana,
            0xAC00..=0xD7AF | 0x1100..=0x11FF => Script::Hangul,
            0x4E00..=0x9FFF | 0x3400..=0x4DBF => Script::Han,
            _ => Script::Other,
        })
    }

    /// The most frequent script among the letters of `text`; Latin when it has none.
    fn of_text(text: &str) -> Script {
        let mut counts: Vec<(Script, usize)> = Vec::new();
        for script in text.chars().filter_map(Script::of_char) {
            match counts.iter_mut().find(|(s, _)| *s == script) {
                Some((_, n)) => *n += 1,
                None => counts.push((script, 1)),
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map_or(Script::Latin, |(s, _)| s)
    }

    fn of_lang(code: &str) -> Script {
        let code = langs::by_code(code).map_or(code, |l| l.code);
        match code {
            "ru" | "uk" | "be" | "bg" | "sr" | "mk" | "kk" | "ky" | "tg" | "mn" | "tt" => {
                Script::Cyrillic
            }
            "el" => Script::Greek,
            "ar" | "fa" | "ur" | "ps" | "sd" | "ug" | "ckb" => Script::Arabic,
            "iw" | "yi" => Script::Hebrew,
            "zh-CN" | "zh-TW" => Script::Han,
            "ko" => Script::Hangul,
            "ja" => Script::Kana,
            "hi" | "mr" | "ne" | "sa" | "mai" | "bho" | "doi" | "gom" => Script::Devanagari,
            "th" => Script::Thai,
            "ka" => Script::Georgian,
            "hy" => Script::Armenian,
            "am" | "ti" | "km" | "lo" | "my" | "si" | "ta" | "te" | "kn" | "ml" | "gu" | "pa"
            | "bn" | "as" | "or" | "dv" | "mni-Mtei" => Script::Other,
            _ => Script::Latin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(first: &str, second: Option<&str>) -> Pair {
        Pair {
            first: first.into(),
            second: second.map(String::from),
        }
    }

    fn system(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    fn to(code: &str) -> Choice {
        Choice {
            from: None,
            to: Some(code.into()),
        }
    }

    #[test]
    fn automatic_pairs() {
        assert_eq!(automatic(&system(&["en", "ru"])), pair("ru", Some("en")));
        assert_eq!(automatic(&system(&["de"])), pair("de", Some("en")));
        assert_eq!(automatic(&system(&[])), pair("en", None));
        assert_eq!(automatic(&system(&["en"])), pair("en", None));
        assert_eq!(automatic(&system(&["zz", "he"])), pair("iw", Some("en")));
    }

    #[test]
    fn preference_pairs() {
        assert_eq!(from_preference("ru, en"), Some(pair("ru", Some("en"))));
        assert_eq!(
            from_preference("Russian, English"),
            Some(pair("ru", Some("en")))
        );
        assert_eq!(from_preference("de,fr"), Some(pair("de", Some("fr"))));
        for bad in ["xx", "ru", "ru, xx", "ru, ru", "ru, en, de", ""] {
            assert_eq!(from_preference(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_rule() {
        let ru = pair("ru", Some("en"));
        assert_eq!(target_for(&ru, "ru").as_deref(), Some("en"));
        for src in ["uk", "en", "de"] {
            assert_eq!(target_for(&ru, src).as_deref(), Some("ru"), "{src}");
        }
        assert_eq!(target_for(&pair("en", None), "en"), None);
        assert_eq!(target_for(&pair("en", None), "de").as_deref(), Some("en"));
    }

    #[test]
    fn the_first_guess() {
        let ru = pair("ru", Some("en"));
        let none = Choice::default();
        assert_eq!(
            first_target(&ru, &none, "привет", None).as_deref(),
            Some("en")
        );
        assert_eq!(
            first_target(&ru, &none, "hello", None).as_deref(),
            Some("ru")
        );
        let de = pair("de", Some("en"));
        assert_eq!(
            first_target(&de, &none, "hello", None).as_deref(),
            Some("de")
        );
        assert_eq!(
            first_target(&de, &none, "hallo", Some("en")).as_deref(),
            Some("de")
        );
        assert_eq!(
            first_target(&de, &none, "hallo", Some("de")).as_deref(),
            Some("en")
        );
        assert_eq!(
            first_target(&ru, &to("fr"), "hello", None).as_deref(),
            Some("fr")
        );
        let from_de = Choice {
            from: Some("de".into()),
            to: None,
        };
        assert_eq!(
            first_target(&ru, &from_de, "hallo", None).as_deref(),
            Some("ru")
        );
        // Latin text on an English-only pair may be French: ask for English; an answer
        // that says it was English has no target (the pad shows the hint).
        assert_eq!(
            first_target(&pair("en", None), &none, "hello", None).as_deref(),
            Some("en")
        );
        // A previous English text gives no target: the script guess decides again.
        assert_eq!(
            first_target(&pair("en", None), &none, "привет", Some("en")).as_deref(),
            Some("en")
        );
        assert_eq!(
            first_target(&pair("en", None), &none, "привет", None).as_deref(),
            Some("en")
        );
    }

    #[test]
    fn the_retry() {
        let ru = pair("ru", Some("en"));
        let none = Choice::default();
        assert_eq!(retry_target(&ru, &none, "en", "uk").as_deref(), Some("ru"));
        assert_eq!(retry_target(&ru, &none, "en", "ru"), None);
        let de = pair("de", Some("en"));
        assert_eq!(retry_target(&de, &none, "de", "de").as_deref(), Some("en"));
        assert_eq!(retry_target(&de, &none, "de", "en"), None);
        assert_eq!(retry_target(&ru, &to("fr"), "fr", "fr"), None);
        assert_eq!(
            retry_target(&pair("iw", Some("en")), &none, "en", "he"),
            None
        );
    }

    #[test]
    fn labels_and_layering() {
        assert_eq!(label(&pair("ru", Some("en"))), "Russian ↔ English");
        assert_eq!(label(&pair("en", None)), "English");
        let stored = to("de");
        let pad = Choice {
            from: Some("en".into()),
            to: None,
        };
        assert_eq!(
            pad.over(&stored),
            Choice {
                from: Some("en".into()),
                to: Some("de".into())
            }
        );
    }
}
