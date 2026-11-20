//! Script detection: which writing systems the letters of a text belong to.

/// A set of scripts, as bit flags. Mixed text sets several bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Scripts(u8);

impl Scripts {
    pub const LATIN: Scripts = Scripts(1);
    pub const CYRILLIC: Scripts = Scripts(1 << 1);
    pub const GEORGIAN: Scripts = Scripts(1 << 2);
    pub const ARMENIAN: Scripts = Scripts(1 << 3);
    /// Any other alphabetic character (Greek, CJK, Arabic, ...).
    pub const OTHER: Scripts = Scripts(1 << 4);

    /// The scripts of the letters of `text`. Digits, punctuation and spaces count for nothing.
    pub fn of(text: &str) -> Scripts {
        let mut set = Scripts::default();
        for ch in text.chars().filter(|c| c.is_alphabetic()) {
            set = set.union(Scripts::of_letter(ch));
        }
        set
    }

    fn of_letter(ch: char) -> Scripts {
        match ch {
            'a'..='z' | 'A'..='Z' | '\u{C0}'..='\u{24F}' => Scripts::LATIN,
            // U+02BC MODIFIER LETTER APOSTROPHE: the Ukrainian layout types it as a letter.
            '\u{400}'..='\u{4FF}' | '\u{2BC}' => Scripts::CYRILLIC,
            '\u{10A0}'..='\u{10FF}' | '\u{1C90}'..='\u{1CBF}' => Scripts::GEORGIAN,
            '\u{530}'..='\u{58F}' => Scripts::ARMENIAN,
            _ => Scripts::OTHER,
        }
    }

    /// True when the two sets have a script in common.
    pub const fn contains(self, other: Scripts) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Scripts) -> Scripts {
        Scripts(self.0 | other.0)
    }

    /// The set itself when it holds exactly one script.
    pub const fn single(self) -> Option<Scripts> {
        if self.0.count_ones() == 1 {
            Some(self)
        } else {
            None
        }
    }
}

/// Number of alphabetic characters in `text`.
pub fn letter_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}
