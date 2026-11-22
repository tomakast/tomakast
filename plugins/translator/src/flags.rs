//! A flag for each language: its main country's (or region's), drawn from the PNGs in the
//! plugin's `flags/` folder, made from flag-icons (MIT, lipis/flag-icons) at 64x48. A
//! language with no country of its own (Esperanto, Latin, Yiddish, Kurdish, Hawaiian, Hmong,
//! Uyghur) shows a neutral globe. Emoji flags are not used: Windows draws them as letters.

use tomakast_plugin::Icon;

use crate::langs;

/// Google's language code → the flag-icons name of its flag.
pub const FLAGS: &[(&str, &str)] = &[
    ("af", "za"),
    ("sq", "al"),
    ("am", "et"),
    ("ar", "sa"),
    ("hy", "am"),
    ("as", "in"),
    ("ay", "bo"),
    ("az", "az"),
    ("bm", "ml"),
    ("eu", "es-pv"),
    ("be", "by"),
    ("bn", "bd"),
    ("bho", "in"),
    ("bs", "ba"),
    ("bg", "bg"),
    ("ca", "es-ct"),
    ("ceb", "ph"),
    ("ny", "mw"),
    ("zh-CN", "cn"),
    ("zh-TW", "tw"),
    ("co", "fr"),
    ("hr", "hr"),
    ("cs", "cz"),
    ("da", "dk"),
    ("dv", "mv"),
    ("doi", "in"),
    ("nl", "nl"),
    ("en", "gb"),
    ("et", "ee"),
    ("ee", "gh"),
    ("tl", "ph"),
    ("fi", "fi"),
    ("fr", "fr"),
    ("fy", "nl"),
    ("gl", "es-ga"),
    ("ka", "ge"),
    ("de", "de"),
    ("el", "gr"),
    ("gn", "py"),
    ("gu", "in"),
    ("ht", "ht"),
    ("ha", "ng"),
    ("iw", "il"),
    ("hi", "in"),
    ("hu", "hu"),
    ("is", "is"),
    ("ig", "ng"),
    ("ilo", "ph"),
    ("id", "id"),
    ("ga", "ie"),
    ("it", "it"),
    ("ja", "jp"),
    ("jw", "id"),
    ("kn", "in"),
    ("kk", "kz"),
    ("km", "kh"),
    ("rw", "rw"),
    ("gom", "in"),
    ("ko", "kr"),
    ("kri", "sl"),
    ("ky", "kg"),
    ("lo", "la"),
    ("lv", "lv"),
    ("ln", "cd"),
    ("lt", "lt"),
    ("lg", "ug"),
    ("lb", "lu"),
    ("mk", "mk"),
    ("mai", "in"),
    ("mg", "mg"),
    ("ms", "my"),
    ("ml", "in"),
    ("mt", "mt"),
    ("mi", "nz"),
    ("mr", "in"),
    ("mni-Mtei", "in"),
    ("lus", "in"),
    ("mn", "mn"),
    ("my", "mm"),
    ("ne", "np"),
    ("no", "no"),
    ("or", "in"),
    ("om", "et"),
    ("ps", "af"),
    ("fa", "ir"),
    ("pl", "pl"),
    ("pt", "pt"),
    ("pa", "in"),
    ("qu", "pe"),
    ("ro", "ro"),
    ("ru", "ru"),
    ("sm", "ws"),
    ("sa", "in"),
    ("gd", "gb-sct"),
    ("nso", "za"),
    ("sr", "rs"),
    ("st", "ls"),
    ("sn", "zw"),
    ("sd", "pk"),
    ("si", "lk"),
    ("sk", "sk"),
    ("sl", "si"),
    ("so", "so"),
    ("es", "es"),
    ("su", "id"),
    ("sw", "tz"),
    ("sv", "se"),
    ("tg", "tj"),
    ("ta", "in"),
    ("tt", "ru"),
    ("te", "in"),
    ("th", "th"),
    ("ti", "er"),
    ("ts", "za"),
    ("tr", "tr"),
    ("tk", "tm"),
    ("ak", "gh"),
    ("uk", "ua"),
    ("ur", "pk"),
    ("uz", "uz"),
    ("vi", "vn"),
    ("cy", "gb-wls"),
    ("xh", "za"),
    ("yo", "ng"),
    ("zu", "za"),
];

/// The Lucide icon of a language with no flag.
pub const NO_FLAG: &str = "globe";

/// The flag of language `code` (any code [`langs::by_code`] knows): its PNG, or the globe.
pub fn icon(code: &str) -> Icon {
    let code = langs::by_code(code).map_or(code, |l| l.code);
    match FLAGS.iter().find(|(lang, _)| *lang == code) {
        Some((_, flag)) => Icon::PluginFile(format!("flags/{flag}.png")),
        None => Icon::Named(NO_FLAG.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn folder() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/flags"))
    }

    fn shown(icon: Icon) -> String {
        match icon {
            Icon::PluginFile(path) => path,
            Icon::Named(name) => format!("named:{name}"),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn the_main_ones() {
        assert_eq!(shown(icon("en")), "flags/gb.png");
        assert_eq!(shown(icon("ru")), "flags/ru.png");
        assert_eq!(shown(icon("he")), "flags/il.png");
        assert_eq!(shown(icon("zh")), "flags/cn.png");
        for none in ["eo", "la", "yi", "ku", "haw", "hmn", "ug", "xx"] {
            assert_eq!(shown(icon(none)), "named:globe", "{none}");
        }
    }

    #[test]
    fn every_flag_is_shipped_and_every_png_used() {
        for (lang, flag) in FLAGS {
            assert!(langs::by_code(lang).is_some(), "{lang} is a language");
            let png = std::fs::read(folder().join(format!("{flag}.png")))
                .unwrap_or_else(|_| panic!("flags/{flag}.png for {lang}"));
            assert_eq!(&png[1..4], b"PNG", "{flag}");
            // The IHDR width and height, big-endian, at bytes 16..24.
            let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
            assert_eq!((width, height), (64, 48), "{flag}");
        }
        for file in std::fs::read_dir(folder()).unwrap() {
            let name = file.unwrap().file_name().into_string().unwrap();
            if let Some(flag) = name.strip_suffix(".png") {
                assert!(FLAGS.iter().any(|(_, f)| *f == flag), "{name} is not used");
            }
        }
        assert!(folder().join("LICENSE-flag-icons").is_file());
    }
}
