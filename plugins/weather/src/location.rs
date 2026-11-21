//! The computer's location, for the tiles without a city: asked of the launcher, then asked
//! again every [`LOCATE_SECS`], and [`RELOCATE_FAILED_SECS`] after it could not be told or
//! was told only coarsely. Pure, so it is tested natively; `lib.rs` asks only while such a
//! tile is wanted.

use crate::api::Place;

/// How often the location is asked again once known.
pub const LOCATE_SECS: i64 = 3 * 3600;
/// How soon it is asked again when it could not be told, or only coarsely.
pub const RELOCATE_FAILED_SECS: i64 = 30 * 60;
/// A location less accurate than this is coarse: marked on the tile, and asked again sooner.
pub const COARSE_KM: u32 = 50;

/// What a tile says besides the reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Fine, or waiting for the first answer.
    Ok,
    /// The tile shows the computer's location, and it cannot be told.
    NoLocation,
    /// Geocoding knows no such city.
    Unknown,
    /// The last try failed; it is tried again later.
    Failed,
}

/// See the module doc.
#[derive(Debug, Default)]
pub struct Location {
    /// Where the computer is, once told.
    pub place: Option<Place>,
    /// Why the location cannot be told, when it cannot.
    no_location: Option<String>,
    /// When the location was last asked for (unix seconds).
    asked: Option<i64>,
    /// The location known is coarser than [`COARSE_KM`].
    coarse: bool,
    /// The location was asked for and has not been told yet: an answer now is the answer, not
    /// news of a change of the launcher's settings.
    locating: bool,
}

impl Location {
    /// Whether to ask the launcher for the location at `now` (see the module doc); `forced`:
    /// the user asked for fresh weather.
    pub fn wants_location(&self, now: i64, forced: bool) -> bool {
        let again = if self.place.is_some() && !self.coarse {
            LOCATE_SECS
        } else {
            RELOCATE_FAILED_SECS
        };
        forced || self.asked.is_none_or(|at| now - at >= again || at > now)
    }

    /// The location was asked for at `now`.
    pub fn location_asked(&mut self, now: i64) {
        self.asked = Some(now);
        self.locating = true;
    }

    /// The location with its accuracy in km, or why it cannot be told. A failure that answers
    /// the plugin's request keeps a place known before (the service was slow or busy); one
    /// that comes unasked, news that the user changed the launcher's location settings,
    /// forgets it. Returns whether what the tiles show changed (a move does).
    pub fn located(&mut self, answer: Result<(Place, u32), String>) -> bool {
        let asked = std::mem::take(&mut self.locating);
        let before = (self.status(), self.place.clone());
        match answer {
            Ok((place, accuracy_km)) => {
                self.no_location = None;
                self.coarse = accuracy_km > COARSE_KM;
                self.place = Some(place);
            }
            Err(why) => {
                if !asked {
                    self.place = None;
                }
                self.no_location = Some(why);
            }
        }
        before != (self.status(), self.place.clone())
    }

    /// Why the location cannot be told, when it cannot.
    pub fn no_location(&self) -> Option<&str> {
        self.no_location.as_deref()
    }

    /// [`Status::NoLocation`] without a place once it cannot be told, else [`Status::Ok`].
    pub fn status(&self) -> Status {
        if self.place.is_none() && self.no_location.is_some() {
            Status::NoLocation
        } else {
            Status::Ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here(latitude: f64) -> Place {
        Place {
            name: "Here".into(),
            latitude,
            longitude: 13.4,
        }
    }

    #[test]
    fn the_location_is_asked_once_then_taken() {
        let mut l = Location::default();
        assert_eq!(l.status(), Status::Ok);
        assert!(l.wants_location(0, false));
        l.location_asked(0);
        assert!(!l.wants_location(10, false), "asked once");
        assert!(l.located(Ok((here(52.5), 11))));
        assert_eq!(l.place, Some(here(52.5)));
    }

    #[test]
    fn auto_asks_again_in_three_hours_or_half_an_hour_after_a_failure() {
        let mut l = Location::default();
        l.location_asked(0);
        assert!(l.located(Err("location services are off".into())));
        assert_eq!(l.status(), Status::NoLocation);
        assert_eq!(l.no_location(), Some("location services are off"));
        assert!(!l.wants_location(RELOCATE_FAILED_SECS - 1, false));
        assert!(l.wants_location(RELOCATE_FAILED_SECS, false));
        assert!(l.wants_location(1, true), "the user's refresh asks at once");
        l.location_asked(100);
        l.located(Ok((here(52.5), 11)));
        assert_eq!(l.status(), Status::Ok);
        assert!(!l.wants_location(100 + LOCATE_SECS - 1, false));
        assert!(l.wants_location(100 + LOCATE_SECS, false));
        // A failure of a later request keeps the known place.
        l.location_asked(200);
        assert!(!l.located(Err("timed out".into())));
        assert_eq!(l.place.as_ref().unwrap().latitude, 52.5);
        assert_eq!(l.status(), Status::Ok);
    }

    #[test]
    fn a_new_place_is_news() {
        let mut l = Location::default();
        assert!(l.located(Ok((here(52.5), 11))));
        assert!(
            !l.located(Ok((here(52.5), 11))),
            "the same place changes nothing"
        );
        assert!(l.located(Ok((here(48.1), 11))));
        assert_eq!(l.place, Some(here(48.1)));
    }

    #[test]
    fn a_coarse_location_is_asked_again_sooner() {
        let mut l = Location::default();
        l.location_asked(100);
        l.located(Ok((here(55.8), 500)));
        assert_eq!(l.status(), Status::Ok);
        assert!(!l.wants_location(100 + RELOCATE_FAILED_SECS - 1, false));
        assert!(l.wants_location(100 + RELOCATE_FAILED_SECS, false));
        l.location_asked(200);
        l.located(Ok((here(55.8), COARSE_KM)));
        assert!(
            !l.wants_location(200 + RELOCATE_FAILED_SECS, false),
            "fine enough"
        );
    }

    #[test]
    fn a_failure_that_comes_unasked_forgets_the_place() {
        let mut l = Location::default();
        l.location_asked(100);
        l.located(Ok((here(52.5), 11)));
        // Asked, and failed: the place known stays (a slow or busy service).
        l.location_asked(200);
        assert!(!l.located(Err("timed out".into())));
        assert_eq!(l.status(), Status::Ok);
        // Unasked: the user changed the launcher's location settings to a wrong city.
        assert!(l.located(Err("not a city of the built-in list".into())));
        assert_eq!(l.place, None);
        assert_eq!(l.status(), Status::NoLocation);
        // A place that comes unasked is taken as usual.
        assert!(l.located(Ok((here(48.1), 11))));
        assert_eq!(l.status(), Status::Ok);
    }
}
