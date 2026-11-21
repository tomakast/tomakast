//! `Windows.Devices.Geolocation.Geolocator`: asks for access (refused when location is off in
//! Settings, or for desktop apps there), then for one position, with its own timeout.

use std::time::Duration;

use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator, PositionAccuracy};
use windows::Foundation::TimeSpan;

use crate::Fix;

pub(crate) fn locate(timeout: Duration) -> Result<Fix, String> {
    let failed = |e: windows::core::Error| format!("Windows location failed: {}", e.message());
    let access = Geolocator::RequestAccessAsync()
        .and_then(|op| op.join())
        .map_err(failed)?;
    if access != GeolocationAccessStatus::Allowed {
        return Err("location is off or not allowed for the launcher in Windows Settings".into());
    }
    let locator = Geolocator::new().map_err(failed)?;
    locator
        .SetDesiredAccuracy(PositionAccuracy::Default)
        .map_err(failed)?;
    // A position up to 10 minutes old is good enough for a city.
    let coordinate = locator
        .GetGeopositionAsyncWithAgeAndTimeout(
            TimeSpan::from(Duration::from_secs(600)),
            TimeSpan::from(timeout),
        )
        .and_then(|op| op.join())
        .and_then(|p| p.Coordinate())
        .map_err(failed)?;
    let position = coordinate
        .Point()
        .and_then(|p| p.Position())
        .map_err(failed)?;
    Ok(Fix {
        latitude: position.Latitude,
        longitude: position.Longitude,
        accuracy_m: coordinate.Accuracy().ok().filter(|m| *m > 0.),
    })
}
