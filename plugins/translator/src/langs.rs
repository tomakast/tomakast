//! The languages of Google Translate: codes as Google writes them, English and native names,
//! and, for the languages people name in Russian, the forms a Russian phrase uses
//! (`на немецком`, `на немецкий`, `по-немецки`). Names are matched whole, ignoring case.

/// One language.
#[derive(Debug)]
pub struct Lang {
    /// Google's code: `en`, `zh-CN`, `iw` (Hebrew), `jw` (Javanese), `tl` (Filipino).
    pub code: &'static str,
    pub english: &'static str,
    pub native: &'static str,
    /// The Russian forms, when the language has them in the table.
    pub ru: Option<RuForms>,
    /// Other English names it is known by (`chinese`, `farsi`).
    pub also: &'static [&'static str],
}

/// How a Russian phrase names a language.
#[derive(Debug)]
pub struct RuForms {
    /// `немецкий`: also the accusative of `на немецкий`.
    pub nominative: &'static str,
    /// `немецком`, of `на немецком`.
    pub prepositional: &'static str,
    /// `по-немецки`; empty when the language has none (`хинди`).
    pub adverb: &'static str,
}

const fn ru(
    nominative: &'static str,
    prepositional: &'static str,
    adverb: &'static str,
) -> Option<RuForms> {
    Some(RuForms {
        nominative,
        prepositional,
        adverb,
    })
}

const fn lang(
    code: &'static str,
    english: &'static str,
    native: &'static str,
    ru: Option<RuForms>,
) -> Lang {
    Lang {
        code,
        english,
        native,
        ru,
        also: &[],
    }
}

const fn aka(
    code: &'static str,
    english: &'static str,
    native: &'static str,
    ru: Option<RuForms>,
    also: &'static [&'static str],
) -> Lang {
    Lang {
        code,
        english,
        native,
        ru,
        also,
    }
}

/// Google Translate's languages, by English name.
pub static LANGS: &[Lang] = &[
    lang(
        "af",
        "Afrikaans",
        "Afrikaans",
        ru("африкаанс", "африкаанс", ""),
    ),
    lang(
        "sq",
        "Albanian",
        "Shqip",
        ru("албанский", "албанском", "по-албански"),
    ),
    lang(
        "am",
        "Amharic",
        "አማርኛ",
        ru("амхарский", "амхарском", "по-амхарски"),
    ),
    lang(
        "ar",
        "Arabic",
        "العربية",
        ru("арабский", "арабском", "по-арабски"),
    ),
    lang(
        "hy",
        "Armenian",
        "Հայերեն",
        ru("армянский", "армянском", "по-армянски"),
    ),
    lang("as", "Assamese", "অসমীয়া", None),
    lang("ay", "Aymara", "Aymar aru", None),
    lang(
        "az",
        "Azerbaijani",
        "Azərbaycan",
        ru("азербайджанский", "азербайджанском", "по-азербайджански"),
    ),
    lang("bm", "Bambara", "Bamanankan", None),
    lang(
        "eu",
        "Basque",
        "Euskara",
        ru("баскский", "баскском", "по-баскски"),
    ),
    lang(
        "be",
        "Belarusian",
        "Беларуская",
        ru("белорусский", "белорусском", "по-белорусски"),
    ),
    lang(
        "bn",
        "Bengali",
        "বাংলা",
        ru("бенгальский", "бенгальском", "по-бенгальски"),
    ),
    lang("bho", "Bhojpuri", "भोजपुरी", None),
    lang(
        "bs",
        "Bosnian",
        "Bosanski",
        ru("боснийский", "боснийском", "по-боснийски"),
    ),
    lang(
        "bg",
        "Bulgarian",
        "Български",
        ru("болгарский", "болгарском", "по-болгарски"),
    ),
    lang(
        "ca",
        "Catalan",
        "Català",
        ru("каталанский", "каталанском", "по-каталански"),
    ),
    lang("ceb", "Cebuano", "Cebuano", None),
    lang("ny", "Chichewa", "Chichewa", None),
    aka(
        "zh-CN",
        "Chinese (Simplified)",
        "简体中文",
        ru("китайский", "китайском", "по-китайски"),
        &["chinese", "simplified chinese", "mandarin", "中文"],
    ),
    aka(
        "zh-TW",
        "Chinese (Traditional)",
        "繁體中文",
        None,
        &["traditional chinese"],
    ),
    lang("co", "Corsican", "Corsu", None),
    lang(
        "hr",
        "Croatian",
        "Hrvatski",
        ru("хорватский", "хорватском", "по-хорватски"),
    ),
    lang(
        "cs",
        "Czech",
        "Čeština",
        ru("чешский", "чешском", "по-чешски"),
    ),
    lang(
        "da",
        "Danish",
        "Dansk",
        ru("датский", "датском", "по-датски"),
    ),
    lang("dv", "Dhivehi", "ދިވެހި", None),
    lang("doi", "Dogri", "डोगरी", None),
    aka(
        "nl",
        "Dutch",
        "Nederlands",
        ru("нидерландский", "нидерландском", "по-нидерландски"),
        &["flemish"],
    ),
    lang(
        "en",
        "English",
        "English",
        ru("английский", "английском", "по-английски"),
    ),
    lang(
        "eo",
        "Esperanto",
        "Esperanto",
        ru("эсперанто", "эсперанто", ""),
    ),
    lang(
        "et",
        "Estonian",
        "Eesti",
        ru("эстонский", "эстонском", "по-эстонски"),
    ),
    lang("ee", "Ewe", "Eʋegbe", None),
    aka("tl", "Filipino", "Wikang Filipino", None, &["tagalog"]),
    lang(
        "fi",
        "Finnish",
        "Suomi",
        ru("финский", "финском", "по-фински"),
    ),
    lang(
        "fr",
        "French",
        "Français",
        ru("французский", "французском", "по-французски"),
    ),
    lang("fy", "Frisian", "Frysk", None),
    lang("gl", "Galician", "Galego", None),
    lang(
        "ka",
        "Georgian",
        "ქართული",
        ru("грузинский", "грузинском", "по-грузински"),
    ),
    lang(
        "de",
        "German",
        "Deutsch",
        ru("немецкий", "немецком", "по-немецки"),
    ),
    lang(
        "el",
        "Greek",
        "Ελληνικά",
        ru("греческий", "греческом", "по-гречески"),
    ),
    lang("gn", "Guarani", "Avañe'ẽ", None),
    lang("gu", "Gujarati", "ગુજરાતી", None),
    lang("ht", "Haitian Creole", "Kreyòl ayisyen", None),
    lang("ha", "Hausa", "Hausa", None),
    lang("haw", "Hawaiian", "ʻŌlelo Hawaiʻi", None),
    lang("iw", "Hebrew", "עברית", ru("иврит", "иврите", "")),
    lang("hi", "Hindi", "हिन्दी", ru("хинди", "хинди", "")),
    lang("hmn", "Hmong", "Hmoob", None),
    lang(
        "hu",
        "Hungarian",
        "Magyar",
        ru("венгерский", "венгерском", "по-венгерски"),
    ),
    lang(
        "is",
        "Icelandic",
        "Íslenska",
        ru("исландский", "исландском", "по-исландски"),
    ),
    lang("ig", "Igbo", "Igbo", None),
    lang("ilo", "Ilocano", "Iloko", None),
    lang(
        "id",
        "Indonesian",
        "Bahasa Indonesia",
        ru("индонезийский", "индонезийском", "по-индонезийски"),
    ),
    lang(
        "ga",
        "Irish",
        "Gaeilge",
        ru("ирландский", "ирландском", "по-ирландски"),
    ),
    lang(
        "it",
        "Italian",
        "Italiano",
        ru("итальянский", "итальянском", "по-итальянски"),
    ),
    lang(
        "ja",
        "Japanese",
        "日本語",
        ru("японский", "японском", "по-японски"),
    ),
    lang("jw", "Javanese", "Basa Jawa", None),
    lang("kn", "Kannada", "ಕನ್ನಡ", None),
    lang(
        "kk",
        "Kazakh",
        "Қазақ тілі",
        ru("казахский", "казахском", "по-казахски"),
    ),
    lang(
        "km",
        "Khmer",
        "ខ្មែរ",
        ru("кхмерский", "кхмерском", "по-кхмерски"),
    ),
    lang("rw", "Kinyarwanda", "Ikinyarwanda", None),
    lang("gom", "Konkani", "कोंकणी", None),
    lang(
        "ko",
        "Korean",
        "한국어",
        ru("корейский", "корейском", "по-корейски"),
    ),
    lang("kri", "Krio", "Krio", None),
    aka(
        "ku",
        "Kurdish (Kurmanji)",
        "Kurdî",
        ru("курдский", "курдском", "по-курдски"),
        &["kurdish", "kurmanji"],
    ),
    aka("ckb", "Kurdish (Sorani)", "کوردی", None, &["sorani"]),
    lang(
        "ky",
        "Kyrgyz",
        "Кыргызча",
        ru("киргизский", "киргизском", "по-киргизски"),
    ),
    lang("lo", "Lao", "ລາວ", ru("лаосский", "лаосском", "по-лаосски")),
    lang(
        "la",
        "Latin",
        "Latina",
        ru("латинский", "латинском", "по-латински"),
    ),
    lang(
        "lv",
        "Latvian",
        "Latviešu",
        ru("латышский", "латышском", "по-латышски"),
    ),
    lang("ln", "Lingala", "Lingála", None),
    lang(
        "lt",
        "Lithuanian",
        "Lietuvių",
        ru("литовский", "литовском", "по-литовски"),
    ),
    lang("lg", "Luganda", "Luganda", None),
    lang("lb", "Luxembourgish", "Lëtzebuergesch", None),
    lang(
        "mk",
        "Macedonian",
        "Македонски",
        ru("македонский", "македонском", "по-македонски"),
    ),
    lang("mai", "Maithili", "मैथिली", None),
    lang("mg", "Malagasy", "Malagasy", None),
    lang(
        "ms",
        "Malay",
        "Bahasa Melayu",
        ru("малайский", "малайском", "по-малайски"),
    ),
    lang("ml", "Malayalam", "മലയാളം", None),
    lang(
        "mt",
        "Maltese",
        "Malti",
        ru("мальтийский", "мальтийском", "по-мальтийски"),
    ),
    lang("mi", "Maori", "Te Reo Māori", None),
    lang("mr", "Marathi", "मराठी", None),
    lang("mni-Mtei", "Meiteilon (Manipuri)", "ꯃꯤꯇꯩꯂꯣꯟ", None),
    lang("lus", "Mizo", "Mizo ṭawng", None),
    lang(
        "mn",
        "Mongolian",
        "Монгол",
        ru("монгольский", "монгольском", "по-монгольски"),
    ),
    aka("my", "Myanmar (Burmese)", "မြန်မာ", None, &["burmese"]),
    lang("ne", "Nepali", "नेपाली", None),
    lang(
        "no",
        "Norwegian",
        "Norsk",
        ru("норвежский", "норвежском", "по-норвежски"),
    ),
    lang("or", "Odia (Oriya)", "ଓଡ଼ିଆ", None),
    lang("om", "Oromo", "Afaan Oromoo", None),
    lang("ps", "Pashto", "پښتو", None),
    aka(
        "fa",
        "Persian",
        "فارسی",
        ru("персидский", "персидском", "по-персидски"),
        &["farsi"],
    ),
    lang(
        "pl",
        "Polish",
        "Polski",
        ru("польский", "польском", "по-польски"),
    ),
    lang(
        "pt",
        "Portuguese",
        "Português",
        ru("португальский", "португальском", "по-португальски"),
    ),
    lang("pa", "Punjabi", "ਪੰਜਾਬੀ", None),
    lang("qu", "Quechua", "Runasimi", None),
    lang(
        "ro",
        "Romanian",
        "Română",
        ru("румынский", "румынском", "по-румынски"),
    ),
    lang(
        "ru",
        "Russian",
        "Русский",
        ru("русский", "русском", "по-русски"),
    ),
    lang("sm", "Samoan", "Gagana Sāmoa", None),
    lang("sa", "Sanskrit", "संस्कृतम्", None),
    lang("gd", "Scots Gaelic", "Gàidhlig", None),
    lang("nso", "Sepedi", "Sesotho sa Leboa", None),
    lang(
        "sr",
        "Serbian",
        "Српски",
        ru("сербский", "сербском", "по-сербски"),
    ),
    lang("st", "Sesotho", "Sesotho", None),
    lang("sn", "Shona", "chiShona", None),
    lang("sd", "Sindhi", "سنڌي", None),
    lang("si", "Sinhala", "සිංහල", None),
    lang(
        "sk",
        "Slovak",
        "Slovenčina",
        ru("словацкий", "словацком", "по-словацки"),
    ),
    lang(
        "sl",
        "Slovenian",
        "Slovenščina",
        ru("словенский", "словенском", "по-словенски"),
    ),
    lang("so", "Somali", "Soomaali", None),
    lang(
        "es",
        "Spanish",
        "Español",
        ru("испанский", "испанском", "по-испански"),
    ),
    lang("su", "Sundanese", "Basa Sunda", None),
    lang("sw", "Swahili", "Kiswahili", ru("суахили", "суахили", "")),
    lang(
        "sv",
        "Swedish",
        "Svenska",
        ru("шведский", "шведском", "по-шведски"),
    ),
    lang(
        "tg",
        "Tajik",
        "Тоҷикӣ",
        ru("таджикский", "таджикском", "по-таджикски"),
    ),
    lang("ta", "Tamil", "தமிழ்", None),
    lang(
        "tt",
        "Tatar",
        "Татар",
        ru("татарский", "татарском", "по-татарски"),
    ),
    lang("te", "Telugu", "తెలుగు", None),
    lang("th", "Thai", "ไทย", ru("тайский", "тайском", "по-тайски")),
    lang("ti", "Tigrinya", "ትግርኛ", None),
    lang("ts", "Tsonga", "Xitsonga", None),
    lang(
        "tr",
        "Turkish",
        "Türkçe",
        ru("турецкий", "турецком", "по-турецки"),
    ),
    lang(
        "tk",
        "Turkmen",
        "Türkmen",
        ru("туркменский", "туркменском", "по-туркменски"),
    ),
    aka("ak", "Twi", "Twi", None, &["akan"]),
    lang(
        "uk",
        "Ukrainian",
        "Українська",
        ru("украинский", "украинском", "по-украински"),
    ),
    lang("ur", "Urdu", "اردو", ru("урду", "урду", "")),
    lang(
        "ug",
        "Uyghur",
        "ئۇيغۇرچە",
        ru("уйгурский", "уйгурском", "по-уйгурски"),
    ),
    lang(
        "uz",
        "Uzbek",
        "Oʻzbek",
        ru("узбекский", "узбекском", "по-узбекски"),
    ),
    lang(
        "vi",
        "Vietnamese",
        "Tiếng Việt",
        ru("вьетнамский", "вьетнамском", "по-вьетнамски"),
    ),
    lang(
        "cy",
        "Welsh",
        "Cymraeg",
        ru("валлийский", "валлийском", "по-валлийски"),
    ),
    lang("xh", "Xhosa", "isiXhosa", None),
    lang("yi", "Yiddish", "ייִדיש", ru("идиш", "идише", "")),
    lang("yo", "Yoruba", "Yorùbá", None),
    lang("zu", "Zulu", "isiZulu", None),
];

/// Codes other sources use for a language Google names otherwise: `he`, `jv`, `fil`, `nb`,
/// a bare `zh`, a regional `pt`.
const SYNONYMS: &[(&str, &str)] = &[
    ("he", "iw"),
    ("jv", "jw"),
    ("fil", "tl"),
    ("nb", "no"),
    ("nn", "no"),
    ("zh", "zh-CN"),
    ("zh-Hans", "zh-CN"),
    ("zh-Hant", "zh-TW"),
    ("zh-HK", "zh-TW"),
    ("pt-BR", "pt"),
    ("pt-PT", "pt"),
    ("mni", "mni-Mtei"),
];

/// The language of `code`, in any case: Google's codes and [`SYNONYMS`].
pub fn by_code(code: &str) -> Option<&'static Lang> {
    let code = SYNONYMS
        .iter()
        .find(|(other, _)| other.eq_ignore_ascii_case(code))
        .map_or(code, |(_, google)| google);
    LANGS.iter().find(|l| l.code.eq_ignore_ascii_case(code))
}

/// The language a name names whole, ignoring case: its English or native name, another
/// English name, or a Russian nominative or prepositional form. Codes are not names.
pub fn by_name(name: &str) -> Option<&'static Lang> {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return None;
    }
    LANGS.iter().find(|l| {
        let ru =
            l.ru.as_ref()
                .is_some_and(|f| f.nominative == name || f.prepositional == name);
        ru || l.english.to_lowercase() == name
            || l.native.to_lowercase() == name
            || l.also.iter().any(|a| a.to_lowercase() == name)
    })
}

/// The language of a Russian adverb such as `по-немецки`, ignoring case.
pub fn by_adverb(word: &str) -> Option<&'static Lang> {
    let word = word.trim().to_lowercase();
    LANGS.iter().find(|l| {
        l.ru.as_ref()
            .is_some_and(|f| !f.adverb.is_empty() && f.adverb == word)
    })
}

/// The English name of `code`, else the code as given.
pub fn name(code: &str) -> String {
    by_code(code).map_or_else(|| code.to_string(), |l| l.english.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(l: Option<&Lang>) -> Option<&'static str> {
        l.map(|l| l.code)
    }

    #[test]
    fn names_find_languages() {
        for name in ["German", "deutsch", "немецкий", "Немецком", "GERMAN"] {
            assert_eq!(code(by_name(name)), Some("de"), "{name}");
        }
        assert_eq!(code(by_adverb("по-английски")), Some("en"));
        assert_eq!(code(by_adverb("По-Русски")), Some("ru"));
        assert_eq!(code(by_name("chinese")), Some("zh-CN"));
        assert_eq!(code(by_name("иврите")), Some("iw"));
        assert_eq!(code(by_code("zh")), Some("zh-CN"));
        assert_eq!(code(by_code("he")), Some("iw"));
        assert_eq!(code(by_code("EN")), Some("en"));
        assert_eq!(code(by_code("pt-BR")), Some("pt"));
    }

    #[test]
    fn codes_and_fragments_are_not_names() {
        for name in ["port", "de", "en", "germ", "", "  "] {
            assert!(by_name(name).is_none(), "{name:?}");
        }
        assert!(by_adverb("по-").is_none());
        assert!(by_adverb("хинди").is_none());
    }

    #[test]
    fn every_name_names_one_language() {
        let mut seen: Vec<(String, &str)> = Vec::new();
        for l in LANGS {
            let mut forms = vec![
                l.code.to_lowercase(),
                l.english.to_lowercase(),
                l.native.to_lowercase(),
            ];
            forms.extend(l.also.iter().map(|a| a.to_lowercase()));
            if let Some(f) = &l.ru {
                forms.extend([f.nominative, f.prepositional, f.adverb].map(String::from));
            }
            forms.sort();
            forms.dedup();
            for form in forms.into_iter().filter(|f| !f.is_empty()) {
                if let Some((_, other)) = seen.iter().find(|(f, _)| *f == form) {
                    panic!("{form:?} names {other} and {}", l.code);
                }
                seen.push((form, l.code));
            }
        }
    }

    #[test]
    fn the_table_is_complete_enough() {
        assert!(LANGS.len() >= 130, "{}", LANGS.len());
        assert!(LANGS.iter().filter(|l| l.ru.is_some()).count() >= 40);
        for l in LANGS {
            if let Some(f) = &l.ru {
                assert!(
                    f.adverb.is_empty() || f.adverb.starts_with("по-"),
                    "{}",
                    l.code
                );
            }
        }
        assert_eq!(name("de"), "German");
        // The phrase parser looks at most 64 bytes past a marker.
        for l in LANGS {
            let mut forms = vec![l.english, l.native];
            forms.extend(l.also.iter().copied());
            if let Some(f) = &l.ru {
                forms.extend([f.nominative, f.prepositional]);
            }
            assert!(forms.iter().all(|f| f.len() <= 64), "{}", l.code);
        }
        assert_eq!(name("xx"), "xx");
    }
}
