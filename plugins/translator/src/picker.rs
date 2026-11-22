//! The language menus of the pad's toolbar: every Google language with its flag, the pair
//! first. The launcher filters them by the English name, the native name and the keywords
//! (the Russian name and other English names), so `german`, `deutsch` and `немецкий` all find
//! German.

use tomakast_plugin::{Icon, InvalidId, MenuOption};

use crate::flags;
use crate::langs::{self, LANGS, Lang};
use crate::pair::{self, Pair};

/// Which side a menu sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Target,
    Source,
}

/// The option that clears a picked target: the pair decides again.
pub const AUTO: &str = "auto";
/// The option that clears a picked source: Google detects it again.
pub const DETECT: &str = "detect";

impl Side {
    /// The toolbar button's id, which is also the `action-id` of a choice.
    pub fn button_id(self) -> &'static str {
        match self {
            Side::Target => "to-lang",
            Side::Source => "from-lang",
        }
    }

    pub fn of_button(id: &str) -> Option<Side> {
        [Side::Target, Side::Source]
            .into_iter()
            .find(|s| s.button_id() == id)
    }
}

/// The menu of `side`: "Automatic (…)" or "Detect language" first, then the pair, then every
/// other language A–Z.
pub fn options(side: Side, pair: &Pair) -> Result<Vec<MenuOption>, InvalidId> {
    let first = match side {
        Side::Target => MenuOption::new(AUTO, format!("Automatic ({})", pair::label(pair)))?
            .icon(Icon::Named("languages".into()))
            .keyword("auto"),
        Side::Source => MenuOption::new(DETECT, "Detect language")?
            .icon(Icon::Named("languages".into()))
            .keyword("auto"),
    };
    let in_pair = |l: &Lang| {
        pair::same(l.code, &pair.first)
            || pair
                .second
                .as_deref()
                .is_some_and(|s| pair::same(l.code, s))
    };
    let ordered = LANGS
        .iter()
        .filter(|l| in_pair(l))
        .chain(LANGS.iter().filter(|l| !in_pair(l)));
    let mut options = vec![first];
    for l in ordered {
        let mut option = MenuOption::new(l.code, l.english)?.icon(flags::icon(l.code));
        if !l.native.eq_ignore_ascii_case(l.english) {
            option = option.subtitle(l.native);
        }
        if let Some(ru) = &l.ru {
            option = option.keyword(ru.nominative);
        }
        for also in l.also.iter().take(7) {
            option = option.keyword(*also);
        }
        options.push(option);
    }
    Ok(options)
}

/// The code a chosen option stands for: `None` for "Automatic" and "Detect language".
pub fn chosen(option_id: &str) -> Option<Option<String>> {
    match option_id {
        AUTO | DETECT => Some(None),
        code => langs::by_code(code).map(|l| Some(l.code.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(side: Side) -> Vec<String> {
        let pair = Pair {
            first: "ru".into(),
            second: Some("en".into()),
        };
        options(side, &pair)
            .unwrap()
            .into_iter()
            .map(|o| o.id)
            .collect()
    }

    #[test]
    fn the_first_options() {
        let all = ids(Side::Target);
        assert_eq!(&all[..3], ["auto", "en", "ru"]);
        assert_eq!(all.len(), LANGS.len() + 1);
        assert_eq!(ids(Side::Source)[0], "detect");
    }

    #[test]
    fn options_carry_flags_and_names() {
        let pair = Pair {
            first: "ru".into(),
            second: Some("en".into()),
        };
        let all = options(Side::Target, &pair).unwrap();
        let german = all.iter().find(|o| o.id == "de").unwrap();
        assert_eq!(german.title, "German");
        assert_eq!(german.subtitle.as_deref(), Some("Deutsch"));
        assert_eq!(german.keywords, ["немецкий"]);
        assert!(matches!(&german.icon, Some(Icon::PluginFile(p)) if p == "flags/de.png"));
        let english = all.iter().find(|o| o.id == "en").unwrap();
        assert_eq!(english.subtitle, None, "the native name is the English one");
        assert!(all.iter().all(|o| o.keywords.len() <= 8));
    }

    #[test]
    fn choices() {
        assert_eq!(chosen("auto"), Some(None));
        assert_eq!(chosen("detect"), Some(None));
        assert_eq!(chosen("zh-CN"), Some(Some("zh-CN".into())));
        assert_eq!(chosen("nope"), None);
        assert_eq!(Side::of_button("from-lang"), Some(Side::Source));
    }
}
