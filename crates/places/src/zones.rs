//! Where each time zone of the tz database is (`zones.tsv`, public domain; see
//! `tools/zones.py`): the zone's main place, `Europe/Moscow` at Moscow. Old and other names of
//! a zone (`Europe/Kiev`) are there too; zones without a place (`UTC`) are not.

const DATA: &str = include_str!("zones.tsv");

/// The `(latitude, longitude)` of the main place of `zone`, an IANA name.
pub fn place(zone: &str) -> Option<(f64, f64)> {
    DATA.lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|line| {
            let mut cols = line.split('\t');
            if cols.next()? != zone {
                return None;
            }
            Some((cols.next()?.parse().ok()?, cols.next()?.parse().ok()?))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_are_at_their_main_place() {
        assert_eq!(place("Europe/Moscow"), Some((55.7558, 37.6178)));
        assert_eq!(place("America/New_York"), Some((40.7142, -74.0064)));
        // Its own country's place, though a link to Europe/Berlin.
        assert_eq!(place("Europe/Oslo"), Some((59.9167, 10.75)));
        // An old name.
        assert_eq!(place("Europe/Kiev"), place("Europe/Kyiv"));
        assert!(place("Europe/Kyiv").is_some());
        for none in ["UTC", "Etc/UTC", "Etc/GMT+3", "", "Mars/Olympus"] {
            assert_eq!(place(none), None, "{none:?}");
        }
    }
}
