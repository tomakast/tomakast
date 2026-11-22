//! `locale`: what the user's computer is set to. Needs no permission.

use std::sync::Arc;

use super::HostState;
use crate::bindings::tomakast::plugin::locale::Host;

/// Longest time zone name the import answers.
pub const MAX_ZONE_BYTES: usize = 64;

/// Where the time zone comes from: the computer's IANA name, or `None` when it cannot be
/// told. Called on every `local-time-zone` call.
pub type ZoneSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The computer's time zone, as the operating system names it.
pub(crate) fn system_zone() -> ZoneSource {
    Arc::new(|| iana_time_zone::get_timezone().ok())
}

/// Most languages `locale.languages` answers.
pub const MAX_LANGUAGES: usize = 16;

/// Where the computer's languages come from, best first, as the OS writes them
/// (`en_US.UTF-8`, `zh-Hans-CN`, `ru`); [`language_codes`] cleans them. Called on every
/// `languages` call.
pub type LanguagesSource = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// The OS's preferred languages, then (on Linux) the languages of KDE's keyboard layouts.
pub(crate) fn system_languages() -> LanguagesSource {
    Arc::new(|| {
        let mut all: Vec<String> = sys_locale::get_locales().collect();
        #[cfg(target_os = "linux")]
        all.extend(kde_layout_languages());
        all
    })
}

/// The languages of the layouts in `kxkbrc`; a missing or unreadable file names none.
#[cfg(target_os = "linux")]
fn kde_layout_languages() -> Vec<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
        });
    config
        .and_then(|dir| std::fs::read_to_string(dir.join("kxkbrc")).ok())
        .map(|rc| kxkb_languages(&rc))
        .unwrap_or_default()
}

/// xkb layout → ISO 639 language, for the layouts KDE lists.
const XKB_LANGUAGES: &[(&str, &str)] = &[
    ("al", "sq"),
    ("am", "hy"),
    ("ara", "ar"),
    ("at", "de"),
    ("az", "az"),
    ("ba", "bs"),
    ("bd", "bn"),
    ("be", "fr"),
    ("bg", "bg"),
    ("br", "pt"),
    ("by", "be"),
    ("ca", "fr"),
    ("ch", "de"),
    ("cn", "zh"),
    ("cz", "cs"),
    ("de", "de"),
    ("dk", "da"),
    ("ee", "et"),
    ("es", "es"),
    ("fi", "fi"),
    ("fo", "fo"),
    ("fr", "fr"),
    ("gb", "en"),
    ("ge", "ka"),
    ("gr", "el"),
    ("hr", "hr"),
    ("hu", "hu"),
    ("ie", "ga"),
    ("il", "he"),
    ("in", "hi"),
    ("iq", "ar"),
    ("ir", "fa"),
    ("is", "is"),
    ("it", "it"),
    ("jp", "ja"),
    ("ke", "sw"),
    ("kg", "ky"),
    ("kh", "km"),
    ("kr", "ko"),
    ("kz", "kk"),
    ("la", "lo"),
    ("latam", "es"),
    ("lk", "si"),
    ("lt", "lt"),
    ("lv", "lv"),
    ("md", "ro"),
    ("me", "sr"),
    ("mk", "mk"),
    ("mn", "mn"),
    ("mt", "mt"),
    ("my", "my"),
    ("ng", "en"),
    ("nl", "nl"),
    ("no", "nb"),
    ("np", "ne"),
    ("ph", "fil"),
    ("pk", "ur"),
    ("pl", "pl"),
    ("pt", "pt"),
    ("ro", "ro"),
    ("rs", "sr"),
    ("ru", "ru"),
    ("se", "sv"),
    ("si", "sl"),
    ("sk", "sk"),
    ("sy", "ar"),
    ("th", "th"),
    ("tj", "tg"),
    ("tm", "tk"),
    ("tr", "tr"),
    ("tw", "zh"),
    ("ua", "uk"),
    ("us", "en"),
    ("uz", "uz"),
    ("vn", "vi"),
    ("za", "en"),
];

/// The languages of the `LayoutList` of `[Layout]` in a `kxkbrc`, in its order. A variant
/// (`us(intl)`) counts as its layout; a layout not in the table is skipped.
pub fn kxkb_languages(kxkbrc: &str) -> Vec<String> {
    let mut in_layout = false;
    for line in kxkbrc.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_layout = line == "[Layout]";
        } else if in_layout && let Some(list) = line.strip_prefix("LayoutList=") {
            return list
                .split(',')
                .filter_map(|layout| {
                    let layout = layout.split('(').next().unwrap_or("").trim();
                    XKB_LANGUAGES
                        .iter()
                        .find(|(xkb, _)| *xkb == layout)
                        .map(|(_, lang)| lang.to_string())
                })
                .collect();
        }
    }
    Vec::new()
}

/// `raw` cut to languages: the part before `_`, `-`, `.` or `@`, lowercased, kept when it is
/// 2–3 ASCII letters (`C` and `POSIX` are not), without repeats, at most [`MAX_LANGUAGES`].
pub fn language_codes(raw: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for entry in raw {
        let code = entry
            .split(['_', '-', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let ok = (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase());
        if ok && code != "c" && code != "posix" && !codes.contains(&code) {
            codes.push(code);
            if codes.len() == MAX_LANGUAGES {
                break;
            }
        }
    }
    codes
}

/// `detected` when it is 1–[`MAX_ZONE_BYTES`] bytes of `A-Z a-z 0-9 / _ + -`, else `UTC`.
pub fn zone_name(detected: Option<String>) -> String {
    match detected {
        Some(name)
            if (1..=MAX_ZONE_BYTES).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b)) =>
        {
            name
        }
        _ => "UTC".into(),
    }
}

impl Host for HostState {
    fn local_time_zone(&mut self) -> String {
        self.record_import("locale.local-time-zone");
        zone_name((self.time_zone)())
    }

    fn languages(&mut self) -> Vec<String> {
        self.record_import("locale.languages");
        language_codes((self.languages)())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_names_are_kept_or_utc() {
        assert_eq!(zone_name(Some("Europe/Moscow".into())), "Europe/Moscow");
        assert_eq!(
            zone_name(Some("America/Argentina/Buenos_Aires".into())),
            "America/Argentina/Buenos_Aires"
        );
        assert_eq!(zone_name(Some("Etc/GMT+3".into())), "Etc/GMT+3");
        assert_eq!(zone_name(Some("Etc/GMT-14".into())), "Etc/GMT-14");
        let longest = "a".repeat(MAX_ZONE_BYTES);
        assert_eq!(zone_name(Some(longest.clone())), longest);
        for bad in [
            None,
            Some(String::new()),
            Some("a\nb".into()),
            Some("../x y".into()),
            Some("a".repeat(MAX_ZONE_BYTES + 1)),
            Some("Europe/Zürich".into()),
        ] {
            assert_eq!(zone_name(bad.clone()), "UTC", "{bad:?}");
        }
    }

    #[test]
    fn language_codes_keep_the_language_only() {
        let raw = [
            "en_US.UTF-8",
            "ru_RU",
            "C",
            "POSIX",
            "zh-Hans-CN",
            "en",
            "pt_BR",
            "x",
            "toolong",
        ];
        assert_eq!(
            language_codes(raw.map(String::from)),
            ["en", "ru", "zh", "pt"].map(String::from)
        );
    }

    #[test]
    fn language_codes_are_at_most_sixteen() {
        let many: Vec<String> = (0..40u8)
            .map(|i| format!("{}{}", (b'a' + i / 26) as char, (b'a' + i % 26) as char))
            .collect();
        assert_eq!(language_codes(many).len(), MAX_LANGUAGES);
    }

    #[test]
    fn kde_layouts_name_languages() {
        let rc = "[Layout]\nDisplayNames=,\nLayoutList=us,ru,ua\nUse=true\n";
        assert_eq!(kxkb_languages(rc), ["en", "ru", "uk"].map(String::from));
        let rc = "[Layout]\nLayoutList=us(intl),de(nodeadkeys),zz\n";
        assert_eq!(kxkb_languages(rc), ["en", "de"].map(String::from));
        assert!(kxkb_languages("[Other]\nLayoutList=us\n").is_empty());
        assert!(kxkb_languages("[Layout]\nUse=true\n").is_empty());
        assert!(kxkb_languages("").is_empty());
    }
}
