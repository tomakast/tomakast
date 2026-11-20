//! Keyboard layout tables: what a query typed on one layout reads as on the US layout and back.

mod tables;

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::script::Scripts;

/// One keyboard layout, as the map from its characters to the US layout's characters on the
/// same physical keys (base and Shift levels).
pub struct Layout {
    pub name: &'static str,
    pub script: Scripts,
    /// `(layout char, US char)`, sorted by the first char.
    pub to_us: &'static [(char, char)],
}

/// Layouts in priority order: Cyrillic standards first, then their macOS and Windows
/// variants that differ, then Georgian, then Armenian.
pub static LAYOUTS: &[Layout] = tables::TABLES;

impl Layout {
    fn lookup(&self, ch: char) -> Option<char> {
        self.to_us
            .binary_search_by_key(&ch, |&(from, _)| from)
            .ok()
            .map(|i| self.to_us[i].1)
    }

    /// Reads `s` as typed on this layout, as it would read on the US layout. `None` when a
    /// letter of `s` is not on this layout. Other characters not in the table pass through.
    pub fn to_latin(&self, s: &str) -> Option<String> {
        let mut out = String::with_capacity(s.len());
        for ch in s.chars() {
            match self.lookup(ch) {
                Some(us) => out.push(us),
                None if ch.is_alphabetic() => out.push(self.fold_lookup(ch)?),
                None => out.push(ch),
            }
        }
        Some(out)
    }

    /// A letter missing at its own case: try the other case and carry the case over.
    fn fold_lookup(&self, ch: char) -> Option<char> {
        let mut lower = ch.to_lowercase();
        let l = lower.next().filter(|_| lower.next().is_none())?;
        if l == ch {
            return None;
        }
        self.lookup(l)?.to_uppercase().next()
    }

    /// Reads `s`, typed on the US layout, as it would read on this layout.
    pub fn from_latin(&self, s: &str) -> String {
        let inverse = inverse_of(self);
        s.chars()
            .map(|ch| inverse.get(&ch).copied().unwrap_or(ch))
            .collect()
    }
}

fn inverse_of(layout: &Layout) -> &'static HashMap<char, char> {
    static INVERSES: OnceLock<HashMap<&'static str, HashMap<char, char>>> = OnceLock::new();
    let all = INVERSES.get_or_init(|| {
        LAYOUTS
            .iter()
            .map(|l| {
                let mut inv = HashMap::new();
                for &(from, us) in l.to_us {
                    inv.entry(us).or_insert(from);
                }
                (l.name, inv)
            })
            .collect()
    });
    &all[layout.name]
}
