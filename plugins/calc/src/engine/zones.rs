//! Time zones: jiff's bundled database, found by IANA name, city or abbreviation. Every
//! name matches in any case; `_` and a space are the same.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use jiff::tz::TimeZone;

use super::lexer::Tok;

/// An index into jiff's bundled zone names (`tz::db().available()`, 598 of them), kept in a
/// `OnceLock<Vec<String>>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneId(u16);

/// Large cities and capitals whose names are not the last part of an IANA name, lower-case.
const CITIES: &[(&str, &str)] = &[
    // North America
    ("washington", "America/New_York"),
    ("washington dc", "America/New_York"),
    ("new york city", "America/New_York"),
    ("nyc", "America/New_York"),
    ("boston", "America/New_York"),
    ("philadelphia", "America/New_York"),
    ("miami", "America/New_York"),
    ("atlanta", "America/New_York"),
    ("baltimore", "America/New_York"),
    ("pittsburgh", "America/New_York"),
    ("charlotte", "America/New_York"),
    ("orlando", "America/New_York"),
    ("tampa", "America/New_York"),
    ("cleveland", "America/New_York"),
    ("columbus", "America/New_York"),
    ("ottawa", "America/Toronto"),
    ("quebec", "America/Toronto"),
    ("houston", "America/Chicago"),
    ("dallas", "America/Chicago"),
    ("austin", "America/Chicago"),
    ("san antonio", "America/Chicago"),
    ("minneapolis", "America/Chicago"),
    ("new orleans", "America/Chicago"),
    ("kansas city", "America/Chicago"),
    ("saint louis", "America/Chicago"),
    ("st louis", "America/Chicago"),
    ("milwaukee", "America/Chicago"),
    ("nashville", "America/Chicago"),
    ("memphis", "America/Chicago"),
    ("salt lake city", "America/Denver"),
    ("albuquerque", "America/Denver"),
    ("calgary", "America/Edmonton"),
    ("las vegas", "America/Los_Angeles"),
    ("san francisco", "America/Los_Angeles"),
    ("san diego", "America/Los_Angeles"),
    ("san jose", "America/Los_Angeles"),
    ("seattle", "America/Los_Angeles"),
    ("portland", "America/Los_Angeles"),
    ("sacramento", "America/Los_Angeles"),
    ("oakland", "America/Los_Angeles"),
    ("guadalajara", "America/Mexico_City"),
    // South America
    ("rio de janeiro", "America/Sao_Paulo"),
    ("rio", "America/Sao_Paulo"),
    ("brasilia", "America/Sao_Paulo"),
    ("sao paulo", "America/Sao_Paulo"),
    ("são paulo", "America/Sao_Paulo"),
    ("belo horizonte", "America/Sao_Paulo"),
    ("porto alegre", "America/Sao_Paulo"),
    ("curitiba", "America/Sao_Paulo"),
    ("salvador", "America/Bahia"),
    ("quito", "America/Guayaquil"),
    ("medellin", "America/Bogota"),
    ("cali", "America/Bogota"),
    ("rosario", "America/Argentina/Buenos_Aires"),
    ("valparaiso", "America/Santiago"),
    // Europe
    ("manchester", "Europe/London"),
    ("birmingham", "Europe/London"),
    ("liverpool", "Europe/London"),
    ("leeds", "Europe/London"),
    ("glasgow", "Europe/London"),
    ("edinburgh", "Europe/London"),
    ("cardiff", "Europe/London"),
    ("marseille", "Europe/Paris"),
    ("lyon", "Europe/Paris"),
    ("nice", "Europe/Paris"),
    ("toulouse", "Europe/Paris"),
    ("bordeaux", "Europe/Paris"),
    ("strasbourg", "Europe/Paris"),
    ("munich", "Europe/Berlin"),
    ("hamburg", "Europe/Berlin"),
    ("frankfurt", "Europe/Berlin"),
    ("cologne", "Europe/Berlin"),
    ("stuttgart", "Europe/Berlin"),
    ("dusseldorf", "Europe/Berlin"),
    ("düsseldorf", "Europe/Berlin"),
    ("dresden", "Europe/Berlin"),
    ("leipzig", "Europe/Berlin"),
    ("hanover", "Europe/Berlin"),
    ("barcelona", "Europe/Madrid"),
    ("valencia", "Europe/Madrid"),
    ("seville", "Europe/Madrid"),
    ("malaga", "Europe/Madrid"),
    ("bilbao", "Europe/Madrid"),
    ("milan", "Europe/Rome"),
    ("naples", "Europe/Rome"),
    ("turin", "Europe/Rome"),
    ("florence", "Europe/Rome"),
    ("venice", "Europe/Rome"),
    ("bologna", "Europe/Rome"),
    ("rotterdam", "Europe/Amsterdam"),
    ("the hague", "Europe/Amsterdam"),
    ("antwerp", "Europe/Brussels"),
    ("geneva", "Europe/Zurich"),
    ("bern", "Europe/Zurich"),
    ("basel", "Europe/Zurich"),
    ("salzburg", "Europe/Vienna"),
    ("krakow", "Europe/Warsaw"),
    ("gdansk", "Europe/Warsaw"),
    ("wroclaw", "Europe/Warsaw"),
    ("brno", "Europe/Prague"),
    ("gothenburg", "Europe/Stockholm"),
    ("bergen", "Europe/Oslo"),
    ("porto", "Europe/Lisbon"),
    ("thessaloniki", "Europe/Athens"),
    ("ankara", "Europe/Istanbul"),
    ("izmir", "Europe/Istanbul"),
    ("kiev", "Europe/Kyiv"),
    ("kharkiv", "Europe/Kyiv"),
    ("odesa", "Europe/Kyiv"),
    ("odessa", "Europe/Kyiv"),
    ("lviv", "Europe/Kyiv"),
    ("saint petersburg", "Europe/Moscow"),
    ("st petersburg", "Europe/Moscow"),
    ("kazan", "Europe/Moscow"),
    ("nizhny novgorod", "Europe/Moscow"),
    ("rostov", "Europe/Moscow"),
    ("sochi", "Europe/Moscow"),
    ("perm", "Asia/Yekaterinburg"),
    ("ekaterinburg", "Asia/Yekaterinburg"),
    // Middle East and Africa
    ("abu dhabi", "Asia/Dubai"),
    ("doha", "Asia/Qatar"),
    ("kuwait city", "Asia/Kuwait"),
    ("manama", "Asia/Bahrain"),
    ("sanaa", "Asia/Aden"),
    ("sharjah", "Asia/Dubai"),
    ("jeddah", "Asia/Riyadh"),
    ("mecca", "Asia/Riyadh"),
    ("tel aviv", "Asia/Jerusalem"),
    ("alexandria", "Africa/Cairo"),
    ("giza", "Africa/Cairo"),
    ("abuja", "Africa/Lagos"),
    ("cape town", "Africa/Johannesburg"),
    ("pretoria", "Africa/Johannesburg"),
    ("durban", "Africa/Johannesburg"),
    ("rabat", "Africa/Casablanca"),
    ("marrakesh", "Africa/Casablanca"),
    ("mombasa", "Africa/Nairobi"),
    // Asia
    ("beijing", "Asia/Shanghai"),
    ("shenzhen", "Asia/Shanghai"),
    ("guangzhou", "Asia/Shanghai"),
    ("chengdu", "Asia/Shanghai"),
    ("wuhan", "Asia/Shanghai"),
    ("hangzhou", "Asia/Shanghai"),
    ("nanjing", "Asia/Shanghai"),
    ("tianjin", "Asia/Shanghai"),
    ("xian", "Asia/Shanghai"),
    ("osaka", "Asia/Tokyo"),
    ("kyoto", "Asia/Tokyo"),
    ("yokohama", "Asia/Tokyo"),
    ("nagoya", "Asia/Tokyo"),
    ("sapporo", "Asia/Tokyo"),
    ("fukuoka", "Asia/Tokyo"),
    ("kobe", "Asia/Tokyo"),
    ("busan", "Asia/Seoul"),
    ("incheon", "Asia/Seoul"),
    ("delhi", "Asia/Kolkata"),
    ("new delhi", "Asia/Kolkata"),
    ("mumbai", "Asia/Kolkata"),
    ("bombay", "Asia/Kolkata"),
    ("bangalore", "Asia/Kolkata"),
    ("bengaluru", "Asia/Kolkata"),
    ("chennai", "Asia/Kolkata"),
    ("madras", "Asia/Kolkata"),
    ("hyderabad", "Asia/Kolkata"),
    ("ahmedabad", "Asia/Kolkata"),
    ("pune", "Asia/Kolkata"),
    ("jaipur", "Asia/Kolkata"),
    ("lahore", "Asia/Karachi"),
    ("islamabad", "Asia/Karachi"),
    ("chittagong", "Asia/Dhaka"),
    ("hanoi", "Asia/Ho_Chi_Minh"),
    ("ho chi minh city", "Asia/Ho_Chi_Minh"),
    ("bali", "Asia/Makassar"),
    ("denpasar", "Asia/Makassar"),
    ("surabaya", "Asia/Jakarta"),
    ("bandung", "Asia/Jakarta"),
    ("cebu", "Asia/Manila"),
    ("quezon city", "Asia/Manila"),
    ("astana", "Asia/Almaty"),
    ("chiang mai", "Asia/Bangkok"),
    ("penang", "Asia/Kuala_Lumpur"),
    ("male", "Indian/Maldives"),
    // Oceania
    ("canberra", "Australia/Sydney"),
    ("gold coast", "Australia/Brisbane"),
    ("wellington", "Pacific/Auckland"),
    ("christchurch", "Pacific/Auckland"),
    // US regions, which would otherwise be nothing (the `US/` and `Canada/` links are not
    // cities).
    ("pacific", "America/Los_Angeles"),
    ("eastern", "America/New_York"),
    ("central", "America/Chicago"),
    ("mountain", "America/Denver"),
    // Two zones share these last parts at the same depth; the canonical one wins.
    ("istanbul", "Europe/Istanbul"),
    ("nicosia", "Asia/Nicosia"),
];

/// Short names people use for cities, lower-case: everyday forms (`spb`, `la`, `ekb`) and
/// the IATA codes of large cities and their main airports (`bcn`, `lon`, `jfk`). Codes that
/// mean something else in calc are left out: `mad` (the dirham), `ist` (India time), `sin`
/// (the sine), `mil` and `was`.
const SHORT_NAMES: &[(&str, &str)] = &[
    // Russian everyday forms.
    ("spb", "Europe/Moscow"),
    ("piter", "Europe/Moscow"),
    ("ekb", "Asia/Yekaterinburg"),
    ("nsk", "Asia/Novosibirsk"),
    ("kzn", "Europe/Moscow"),
    ("vvo", "Asia/Vladivostok"),
    ("krr", "Europe/Moscow"),
    // English everyday forms.
    ("la", "America/Los_Angeles"),
    ("sf", "America/Los_Angeles"),
    ("dc", "America/New_York"),
    ("philly", "America/New_York"),
    ("vegas", "America/Los_Angeles"),
    ("nola", "America/Chicago"),
    ("hk", "Asia/Hong_Kong"),
    ("kl", "Asia/Kuala_Lumpur"),
    ("hcmc", "Asia/Ho_Chi_Minh"),
    ("joburg", "Africa/Johannesburg"),
    // IATA city and airport codes.
    ("lon", "Europe/London"),
    ("par", "Europe/Paris"),
    ("ber", "Europe/Berlin"),
    ("bcn", "Europe/Madrid"),
    ("rom", "Europe/Rome"),
    ("ams", "Europe/Amsterdam"),
    ("bru", "Europe/Brussels"),
    ("vie", "Europe/Vienna"),
    ("prg", "Europe/Prague"),
    ("waw", "Europe/Warsaw"),
    ("bud", "Europe/Budapest"),
    ("lis", "Europe/Lisbon"),
    ("ath", "Europe/Athens"),
    ("mow", "Europe/Moscow"),
    ("dxb", "Asia/Dubai"),
    ("tlv", "Asia/Jerusalem"),
    ("cai", "Africa/Cairo"),
    ("jnb", "Africa/Johannesburg"),
    ("nbo", "Africa/Nairobi"),
    ("del", "Asia/Kolkata"),
    ("bom", "Asia/Kolkata"),
    ("blr", "Asia/Kolkata"),
    ("bkk", "Asia/Bangkok"),
    ("hkg", "Asia/Hong_Kong"),
    ("tyo", "Asia/Tokyo"),
    ("sel", "Asia/Seoul"),
    ("bjs", "Asia/Shanghai"),
    ("pek", "Asia/Shanghai"),
    ("sha", "Asia/Shanghai"),
    ("syd", "Australia/Sydney"),
    ("mel", "Australia/Melbourne"),
    ("akl", "Pacific/Auckland"),
    ("yto", "America/Toronto"),
    ("yyz", "America/Toronto"),
    ("yvr", "America/Vancouver"),
    ("yul", "America/Toronto"),
    ("mex", "America/Mexico_City"),
    ("sao", "America/Sao_Paulo"),
    ("gru", "America/Sao_Paulo"),
    ("bue", "America/Argentina/Buenos_Aires"),
    ("scl", "America/Santiago"),
    ("lim", "America/Lima"),
    ("bog", "America/Bogota"),
    ("chi", "America/Chicago"),
    ("lax", "America/Los_Angeles"),
    ("sfo", "America/Los_Angeles"),
    ("sea", "America/Los_Angeles"),
    ("bos", "America/New_York"),
    ("mia", "America/New_York"),
    ("atl", "America/New_York"),
    ("den", "America/Denver"),
    ("phx", "America/Phoenix"),
    ("las", "America/Los_Angeles"),
    ("dfw", "America/Chicago"),
    ("jfk", "America/New_York"),
];

/// Time zone abbreviations, upper-case. Each names one zone; the answer shows that zone's
/// real abbreviation at the instant (`PST` in winter is `PDT` in summer).
const ABBREVIATIONS: &[(&str, &str)] = &[
    ("UTC", "UTC"),
    ("GMT", "GMT"),
    ("BST", "Europe/London"),
    ("WET", "Europe/Lisbon"),
    ("CET", "Europe/Berlin"),
    ("CEST", "Europe/Berlin"),
    ("EET", "Europe/Athens"),
    ("EEST", "Europe/Athens"),
    ("MSK", "Europe/Moscow"),
    ("IST", "Asia/Kolkata"),
    ("PKT", "Asia/Karachi"),
    ("ICT", "Asia/Bangkok"),
    ("WIB", "Asia/Jakarta"),
    ("SGT", "Asia/Singapore"),
    ("HKT", "Asia/Hong_Kong"),
    ("JST", "Asia/Tokyo"),
    ("KST", "Asia/Seoul"),
    ("AEST", "Australia/Sydney"),
    ("AEDT", "Australia/Sydney"),
    ("ACST", "Australia/Adelaide"),
    ("AWST", "Australia/Perth"),
    ("NZST", "Pacific/Auckland"),
    ("NZDT", "Pacific/Auckland"),
    ("EST", "America/New_York"),
    ("EDT", "America/New_York"),
    ("CST", "America/Chicago"),
    ("CDT", "America/Chicago"),
    ("MST", "America/Denver"),
    ("MDT", "America/Denver"),
    ("PST", "America/Los_Angeles"),
    ("PDT", "America/Los_Angeles"),
    ("AKST", "America/Anchorage"),
    ("HST", "Pacific/Honolulu"),
    ("BRT", "America/Sao_Paulo"),
    ("ART", "America/Argentina/Buenos_Aires"),
];

/// The most tokens a zone name spans in [`at`].
const MAX_TOKENS: usize = 5;

/// The bundled zone names, sorted.
fn names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: Vec<String> = jiff::tz::db()
            .available()
            .map(|n| n.as_str().to_owned())
            .collect();
        names.sort_unstable();
        names
    })
}

/// A name as it is looked up: trimmed, lower-case, `_` read as a space.
fn key(name: &str) -> String {
    name.trim().to_lowercase().replace('_', " ")
}

/// Backward links kept for old names (`Canada/Pacific`, `Singapore`, `Australia/North`).
/// They resolve when typed in full, but their last part is no city: `pacific` would be an
/// hour off for a US user, and `north` is not a place.
fn is_link(name: &str) -> bool {
    const PREFIXES: [&str; 7] = [
        "Etc/", "US/", "SystemV/", "Canada/", "Brazil/", "Mexico/", "Chile/",
    ];
    const AUSTRALIA: [&str; 11] = [
        "ACT",
        "Canberra",
        "LHI",
        "North",
        "NSW",
        "Queensland",
        "South",
        "Tasmania",
        "Victoria",
        "West",
        "Yancowinna",
    ];
    !name.contains('/')
        || PREFIXES.iter().any(|p| name.starts_with(p))
        || name
            .strip_prefix("Australia/")
            .is_some_and(|rest| AUSTRALIA.contains(&rest))
}

/// Every name [`find`] knows, by [`key`]. Later inserts win:
/// 1. top-level names (`Singapore`, `Japan`, `UTC`);
/// 2. city names from the last part of the other zones, outside backward links; a deeper
///    name wins (`America/Argentina/Buenos_Aires` over the link `America/Buenos_Aires`);
/// 3. the other full IANA names (`Canada/Pacific`);
/// 4. [`CITIES`], then [`ABBREVIATIONS`], then [`SHORT_NAMES`].
fn index() -> &'static HashMap<String, ZoneId> {
    static INDEX: OnceLock<HashMap<String, ZoneId>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let names = names();
        let id = |name: &str| {
            names
                .binary_search_by(|n| n.as_str().cmp(name))
                .ok()
                .map(|i| ZoneId(i as u16))
        };
        let zones = || names.iter().enumerate().map(|(i, n)| (n, ZoneId(i as u16)));
        let mut index = HashMap::new();
        for (name, id) in zones().filter(|(n, _)| !n.contains('/')) {
            index.insert(key(name), id);
        }
        let mut cities: Vec<_> = zones()
            .filter(|(n, _)| !is_link(n))
            .filter_map(|(n, id)| n.rsplit_once('/').map(|(_, city)| (n, city, id)))
            .collect();
        cities.sort_by_key(|(n, _, _)| n.matches('/').count());
        for (_, city, id) in cities {
            index.insert(key(city), id);
        }
        for (name, id) in zones().filter(|(n, _)| n.contains('/')) {
            index.insert(key(name), id);
        }
        for (name, zone) in CITIES.iter().chain(ABBREVIATIONS).chain(SHORT_NAMES) {
            if let Some(id) = id(zone) {
                index.insert(key(name), id);
            }
        }
        index
    })
}

/// UTC: the zone named exactly `UTC`.
pub fn utc() -> ZoneId {
    find("UTC").expect("the bundled database has UTC")
}

/// An IANA name (any case, `_` or space), a city, or an abbreviation; trimmed. `None` for
/// anything else.
pub fn find(name: &str) -> Option<ZoneId> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    index().get(&key(name)).copied()
}

/// The IANA name.
pub fn name(id: ZoneId) -> &'static str {
    &names()[usize::from(id.0)]
}

/// Cached per id in a `thread_local`.
pub(crate) fn tz(id: ZoneId) -> TimeZone {
    thread_local! {
        static CACHE: RefCell<HashMap<u16, TimeZone>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(id.0)
            .or_insert_with(|| jiff::tz::db().get(name(id)).unwrap_or(TimeZone::UTC))
            .clone()
    })
}

/// The longest zone name over at most 5 tokens at `toks[0..]` (words joined by a space,
/// or words joined by `/`), and how many tokens it took.
pub(crate) fn at(toks: &[Tok]) -> Option<(ZoneId, usize)> {
    let Some(Tok::Ident(first)) = toks.first() else {
        return None;
    };
    let mut best = find(first).map(|id| (id, 1));
    // `New York`, `Rio de Janeiro`.
    let mut spaced = first.clone();
    for (i, tok) in toks.iter().enumerate().take(MAX_TOKENS).skip(1) {
        let Tok::Ident(word) = tok else { break };
        spaced.push(' ');
        spaced.push_str(word);
        if let Some(id) = find(&spaced) {
            best = Some((id, i + 1));
        }
    }
    // `America/Los_Angeles`, `America/Argentina/Buenos_Aires`.
    let mut slashed = first.clone();
    let mut n = 1;
    while n + 2 <= MAX_TOKENS {
        let Some([Tok::Slash, Tok::Ident(word)]) = toks.get(n..n + 2) else {
            break;
        };
        slashed.push('/');
        slashed.push_str(word);
        n += 2;
        if let Some(id) = find(&slashed)
            && best.is_none_or(|(_, taken)| taken < n)
        {
            best = Some((id, n));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{lexer, time, units};

    #[test]
    fn the_database_is_bundled() {
        assert!(jiff::tz::db().get("Asia/Tokyo").is_ok());
        assert!(jiff::tz::db().available().count() >= 590);
    }

    #[test]
    fn find_accepts_names_cities_and_abbreviations() {
        for text in ["Europe/Moscow", "europe/moscow", " Moscow ", "MSK", "msk"] {
            assert_eq!(find(text).map(name), Some("Europe/Moscow"), "{text:?}");
        }
        for text in ["new york", "New_York"] {
            assert_eq!(find(text).map(name), Some("America/New_York"), "{text:?}");
        }
        for text in ["Mars/Base", "", "in"] {
            assert_eq!(find(text), None, "{text:?}");
        }
        // Backward links are no cities, and a city is its canonical zone.
        for text in ["east", "north", "general"] {
            assert_eq!(find(text), None, "{text:?}");
        }
        for (text, zone) in [
            ("pacific", "America/Los_Angeles"),
            ("central", "America/Chicago"),
            ("mountain", "America/Denver"),
            ("eastern", "America/New_York"),
            ("singapore", "Asia/Singapore"),
            ("buenos aires", "America/Argentina/Buenos_Aires"),
            ("indianapolis", "America/Indiana/Indianapolis"),
            ("Canada/Pacific", "Canada/Pacific"),
            ("Japan", "Japan"),
            // Hub cities whose zone is named after another city or the country.
            ("doha", "Asia/Qatar"),
            ("kuwait city", "Asia/Kuwait"),
            ("manama", "Asia/Bahrain"),
            ("sanaa", "Asia/Aden"),
            ("male", "Indian/Maldives"),
            ("Ekaterinburg", "Asia/Yekaterinburg"),
        ] {
            assert_eq!(find(text).map(name), Some(zone), "{text:?}");
        }
        // UTC is the zone named exactly that, which the answer then shows.
        assert_eq!(name(utc()), "UTC");
        assert_eq!(find("UTC"), Some(utc()));
        assert_eq!(find("utc"), Some(utc()));
    }

    #[test]
    fn every_city_and_abbreviation_resolves() {
        assert!(CITIES.len() >= 150, "{} cities", CITIES.len());
        for (key, zone) in CITIES.iter().chain(ABBREVIATIONS) {
            assert!(jiff::tz::db().get(zone).is_ok(), "{key} -> {zone}");
            assert_eq!(find(key).map(name), Some(*zone), "{key}");
        }
        for (key, _) in CITIES {
            assert_eq!(*key, key.to_lowercase(), "city keys are lower-case");
        }
        for (key, _) in ABBREVIATIONS {
            assert_eq!(*key, key.to_uppercase(), "abbreviation keys are upper-case");
        }
    }

    #[test]
    fn tables_do_not_collide() {
        const KEYWORDS: &[&str] = &[
            "in", "to", "of", "mod", "until", "hex", "bin", "oct", "decimal", "date", "unix", "am",
            "pm",
        ];
        for (key, _) in ABBREVIATIONS {
            let lower = key.to_lowercase();
            assert!(
                CITIES.iter().all(|(city, _)| *city != lower),
                "{key} is a city"
            );
            assert!(
                units::lookup(key).is_none() && units::lookup(&lower).is_none(),
                "{key} is a unit"
            );
            assert!(time::month(&lower).is_none(), "{key} is a month");
            assert!(!KEYWORDS.contains(&lower.as_str()), "{key} is a keyword");
        }
    }

    #[test]
    fn at_takes_the_longest_name() {
        let toks = |s: &str| lexer::lex(s).unwrap();
        let at_name = |s: &str| at(&toks(s)).map(|(id, n)| (name(id), n));
        assert_eq!(at_name("New York in"), Some(("America/New_York", 2)));
        assert_eq!(at_name("rio de janeiro"), Some(("America/Sao_Paulo", 3)));
        assert_eq!(
            at_name("america/los_angeles"),
            Some(("America/Los_Angeles", 3))
        );
        assert_eq!(
            at_name("America/Argentina/Buenos_Aires x"),
            Some(("America/Argentina/Buenos_Aires", 5))
        );
        assert_eq!(at_name("London to unix"), Some(("Europe/London", 1)));
        assert_eq!(at_name("in Tokyo"), None);
        assert_eq!(at_name("5 Tokyo"), None);
        assert_eq!(at_name(""), None);
    }

    #[test]
    fn short_names() {
        for (text, zone) in [
            ("spb", "Europe/Moscow"),
            ("SPB", "Europe/Moscow"),
            ("piter", "Europe/Moscow"),
            ("bcn", "Europe/Madrid"),
            ("ekb", "Asia/Yekaterinburg"),
            ("nsk", "Asia/Novosibirsk"),
            ("vvo", "Asia/Vladivostok"),
            ("la", "America/Los_Angeles"),
            ("sf", "America/Los_Angeles"),
            ("nyc", "America/New_York"),
            ("lon", "Europe/London"),
            ("tyo", "Asia/Tokyo"),
            ("jfk", "America/New_York"),
            ("yyz", "America/Toronto"),
        ] {
            assert_eq!(find(text).map(name), Some(zone), "{text:?}");
        }
        assert!(SHORT_NAMES.len() >= 70, "{}", SHORT_NAMES.len());
        // These mean something else in calc.
        for kept in ["mad", "ist", "sin", "mil", "was"] {
            assert!(SHORT_NAMES.iter().all(|(n, _)| *n != kept), "{kept}");
        }
    }

    #[test]
    fn short_names_collide_with_nothing() {
        const WORDS: &[&str] = &[
            "in",
            "to",
            "of",
            "mod",
            "until",
            "hex",
            "bin",
            "oct",
            "decimal",
            "date",
            "unix",
            "am",
            "pm",
            "rgb",
            "hsl",
            "oklch",
            "base",
            "binary",
            "octal",
            "hexadecimal",
            "now",
            "time",
            "today",
            "tomorrow",
            "yesterday",
            "pi",
            "e",
            "prev",
            "ans",
        ];
        let top_level: Vec<String> = names()
            .iter()
            .filter(|n| !n.contains('/'))
            .map(|n| n.to_lowercase())
            .collect();
        for (short, zone) in SHORT_NAMES {
            assert!(jiff::tz::db().get(zone).is_ok(), "{short} -> {zone}");
            assert_eq!(*short, short.to_lowercase(), "short names are lower-case");
            let upper = short.to_uppercase();
            assert!(
                crate::engine::currency::lookup(short).is_none(),
                "{short} is a currency"
            );
            assert!(
                crate::engine::currency::lookup(&upper).is_none(),
                "{short} is a currency"
            );
            assert!(units::lookup(short).is_none(), "{short} is a unit");
            assert!(units::lookup(&upper).is_none(), "{short} is a unit");
            assert!(time::month(short).is_none(), "{short} is a month");
            assert!(!WORDS.contains(short), "{short} is a keyword");
            assert!(
                ABBREVIATIONS
                    .iter()
                    .all(|(a, _)| a.to_lowercase() != *short),
                "{short} is an abbreviation"
            );
            assert!(
                CITIES.iter().all(|(c, _)| c != short),
                "{short} is a city row"
            );
            assert!(!top_level.contains(&short.to_string()), "{short} is a zone");
            assert!(
                crate::engine::color::named(short).is_none(),
                "{short} is a colour"
            );
        }
    }
}
