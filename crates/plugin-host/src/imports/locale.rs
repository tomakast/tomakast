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
}
