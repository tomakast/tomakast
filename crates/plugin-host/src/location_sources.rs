//! Where the launcher finds the computer's location, in turn (the `[location]` settings choose):
//!
//! 1. the city the user set, from the built-in list; when set, it is the only source;
//! 2. the operating system's location service (`tomakast_location::locate`);
//! 3. on Linux, only when the user turned it on: the Wi-Fi networks around, looked up in
//!    BeaconDB, with the service's IP and cell fallbacks off;
//! 4. the time zone's main place.
//!
//! [`configure`] gives the process's [`Locator`] the sources of the user's settings.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use tomakast_core::LocationSettings;
use tomakast_location::Network;
use tomakast_places::cities::Cities;

pub use crate::imports::location::rounded;
use crate::imports::location::{Fix, LocationSource, Locator, MIN_ACCURACY_KM, Source};

/// How long the operating system's service may take, and how long it is waited for.
pub const SYSTEM_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the Wi-Fi lookup may take.
pub const WIFI_TIMEOUT: Duration = Duration::from_secs(20);
/// What a time zone's place is said to be accurate to: one zone may cover a country.
pub const ZONE_ACCURACY_KM: u32 = 500;
/// BeaconDB's Ichnaea-compatible geolocation API.
pub const BEACONDB: &str = "https://api.beacondb.net/v1/geolocate";
/// The client's name in the lookup, and nothing of the user.
const USER_AGENT: &str = "tomakast";

/// Gives the process's locator the sources of `settings` (its city and Wi-Fi lookup), when
/// they changed since the last call (the first call always does).
pub fn configure(settings: &LocationSettings) {
    // The time zone is not a source of the location: changing it keeps the sources.
    static LAST: Mutex<Option<(String, bool)>> = Mutex::new(None);
    let key = (settings.city.clone(), settings.wifi_lookup);
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    drop(last);
    if !settings.city.is_empty() || settings.wifi_lookup {
        tracing::info!(
            target: "plugin",
            city = %settings.city,
            wifi_lookup = settings.wifi_lookup,
            "location settings"
        );
    }
    Locator::system().set_sources(sources(settings));
}

/// The sources for `settings`, in turn.
pub fn sources(settings: &LocationSettings) -> Vec<Source> {
    if !settings.city.is_empty() {
        return vec![city(&settings.city)];
    }
    let mut sources = vec![system()];
    if settings.wifi_lookup {
        sources.push(wifi());
    }
    sources.push(zone(Arc::new(|| iana_time_zone::get_timezone().ok())));
    sources
}

/// The city `name` of the built-in list, or an error that names it.
pub fn city(name: &str) -> Source {
    let found = Cities::builtin().find(name).map(|place| Fix {
        latitude: place.latitude,
        longitude: place.longitude,
        accuracy_km: MIN_ACCURACY_KM,
    });
    let why = format!("`{name}` is not a city of the built-in list");
    Source::new(
        "the city in the settings",
        Duration::from_secs(5),
        Arc::new(move || found.ok_or_else(|| why.clone())),
    )
}

/// The operating system's location service. One for the process, so that a call that has not
/// returned is never made twice, whatever the settings.
fn system() -> Source {
    static SYSTEM: OnceLock<Source> = OnceLock::new();
    SYSTEM
        .get_or_init(|| {
            Source::new(
                "the location service",
                SYSTEM_TIMEOUT + Duration::from_secs(5),
                Arc::new(|| {
                    let fix = tomakast_location::locate(SYSTEM_TIMEOUT)?;
                    Ok(Fix {
                        latitude: fix.latitude,
                        longitude: fix.longitude,
                        accuracy_km: fix.accuracy_m.map_or(MIN_ACCURACY_KM, km),
                    })
                }),
            )
        })
        .clone()
}

/// The Wi-Fi lookup, one for the process like [`system`].
fn wifi() -> Source {
    static WIFI: OnceLock<Source> = OnceLock::new();
    WIFI.get_or_init(|| {
        Source::new(
            "Wi-Fi",
            WIFI_TIMEOUT + Duration::from_secs(5),
            Arc::new(|| {
                let networks = tomakast_location::wifi_networks()?;
                let body = wifi_request(&networks)?;
                let (status, answer) = post(BEACONDB, &body)?;
                wifi_answer(status, &answer)
            }),
        )
    })
    .clone()
}

/// The main place of the zone `zone_name` gives.
pub fn zone(zone_name: Arc<dyn Fn() -> Option<String> + Send + Sync>) -> Source {
    let find: LocationSource = Arc::new(move || {
        let zone = zone_name().ok_or("the computer's time zone is not known")?;
        let (latitude, longitude) =
            tomakast_places::zones::place(&zone).ok_or_else(|| format!("{zone} names no place"))?;
        Ok(Fix {
            latitude,
            longitude,
            accuracy_km: ZONE_ACCURACY_KM,
        })
    });
    Source::new("the time zone", Duration::from_secs(5), find)
}

/// Metres as whole km, rounded up.
fn km(metres: f64) -> u32 {
    (metres / 1000.).ceil().clamp(0., u32::MAX as f64) as u32
}

/// The body of a BeaconDB lookup for `networks`: each usable one's address and signal, with
/// the IP and cell fallbacks off; an error when fewer than two are usable (the service needs
/// two). Hidden networks and those whose name ends in `_nomap` are not used: their owners'
/// opt-out.
pub fn wifi_request(networks: &[Network]) -> Result<String, String> {
    let points: Vec<String> = networks
        .iter()
        .filter(|n| !n.ssid.is_empty() && !n.ssid.ends_with(b"_nomap"))
        .filter(|n| is_bssid(&n.bssid))
        .map(|n| {
            // NetworkManager's percent back to dBm: its own mapping, 2 × (dBm + 100).
            let dbm = i32::from(n.strength.min(100)) / 2 - 100;
            format!(
                r#"{{"macAddress":"{}","signalStrength":{dbm}}}"#,
                n.bssid.to_ascii_lowercase()
            )
        })
        .collect();
    if points.len() < 2 {
        return Err(format!(
            "{} usable Wi-Fi network{} around; the lookup needs two",
            points.len(),
            if points.len() == 1 { "" } else { "s" }
        ));
    }
    Ok(format!(
        r#"{{"considerIp":false,"fallbacks":{{"ipf":false,"lacf":false}},"wifiAccessPoints":[{}]}}"#,
        points.join(",")
    ))
}

/// `aa:bb:cc:dd:ee:ff`.
fn is_bssid(text: &str) -> bool {
    let parts: Vec<&str> = text.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

#[derive(Deserialize)]
struct Geolocated {
    location: LatLng,
    /// Metres.
    accuracy: Option<f64>,
    /// Set when the service fell back (to the IP); such an answer is not taken.
    fallback: Option<String>,
}

#[derive(Deserialize)]
struct LatLng {
    lat: f64,
    lng: f64,
}

/// What BeaconDB's answer, `status` and `body`, says.
pub fn wifi_answer(status: u16, body: &str) -> Result<Fix, String> {
    match status {
        200 => {}
        404 => return Err("the networks around are not in BeaconDB".into()),
        other => return Err(format!("BeaconDB answered {other}")),
    }
    let answer: Geolocated = serde_json::from_str(body)
        .map_err(|e| format!("BeaconDB's answer is not readable: {e}"))?;
    if let Some(fallback) = answer.fallback {
        return Err(format!(
            "BeaconDB guessed by {fallback}, not by the networks"
        ));
    }
    Ok(Fix {
        latitude: answer.location.lat,
        longitude: answer.location.lng,
        accuracy_km: answer.accuracy.map_or(MIN_ACCURACY_KM, km),
    })
}

/// POSTs `body` to `url`: the status and the answer's text (up to 64 KiB).
fn post(url: &str, body: &str) -> Result<(u16, String), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(WIFI_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into();
    let mut response = agent
        .post(url)
        .header("content-type", "application/json")
        .send(body.as_bytes())
        .map_err(|e| format!("the lookup failed: {e}"))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|e| format!("reading BeaconDB's answer failed: {e}"))?;
    Ok((status, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(bssid: &str, ssid: &str, strength: u8) -> Network {
        Network {
            bssid: bssid.into(),
            ssid: ssid.as_bytes().to_vec(),
            strength,
        }
    }

    #[test]
    fn the_lookup_sends_two_or_more_networks_without_hidden_or_nomap_ones_and_no_ip_fallback() {
        let body = wifi_request(&[
            network("AA:BB:CC:DD:EE:01", "home", 100),
            network("aa:bb:cc:dd:ee:02", "", 80),
            network("aa:bb:cc:dd:ee:03", "cafe_nomap", 80),
            network("not a bssid", "x", 80),
            network("aa:bb:cc:dd:ee:04", "neighbour", 40),
        ])
        .unwrap();
        assert_eq!(
            body,
            r#"{"considerIp":false,"fallbacks":{"ipf":false,"lacf":false},"wifiAccessPoints":[{"macAddress":"aa:bb:cc:dd:ee:01","signalStrength":-50},{"macAddress":"aa:bb:cc:dd:ee:04","signalStrength":-80}]}"#
        );
        let one = wifi_request(&[
            network("aa:bb:cc:dd:ee:01", "home", 100),
            network("aa:bb:cc:dd:ee:03", "x_nomap", 80),
        ]);
        assert_eq!(
            one,
            Err("1 usable Wi-Fi network around; the lookup needs two".into())
        );
        assert!(wifi_request(&[]).is_err());
    }

    #[test]
    fn the_answer_gives_the_place_and_its_accuracy_but_never_an_ip_guess() {
        let fix = wifi_answer(
            200,
            r#"{"location":{"lat":52.52,"lng":13.41},"accuracy":1200.5}"#,
        )
        .unwrap();
        assert_eq!(
            fix,
            Fix {
                latitude: 52.52,
                longitude: 13.41,
                accuracy_km: 2
            }
        );
        assert!(
            wifi_answer(
                200,
                r#"{"location":{"lat":1,"lng":2},"accuracy":25000,"fallback":"ipf"}"#
            )
            .unwrap_err()
            .contains("ipf")
        );
        assert_eq!(
            wifi_answer(404, r#"{"error":{"code":404}}"#),
            Err("the networks around are not in BeaconDB".into())
        );
        assert!(wifi_answer(500, "").is_err());
        assert!(wifi_answer(200, "nonsense").is_err());
    }

    fn run(source: &Source) -> Result<Fix, String> {
        let locator = Locator::with_sources(vec![source.clone()]);
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        locator.ask(Arc::new(move |answer| {
            let _ = tx.lock().unwrap().send(answer.clone());
            true
        }));
        rx.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn a_city_of_the_list_is_its_place_and_another_is_an_error() {
        assert_eq!(
            run(&city("Москва")),
            Ok(Fix {
                latitude: 55.8,
                longitude: 37.6,
                accuracy_km: 11
            })
        );
        assert_eq!(
            run(&city("Xyzzy")),
            Err("the city in the settings: `Xyzzy` is not a city of the built-in list".into())
        );
    }

    #[test]
    fn a_time_zone_is_its_main_place_coarsely() {
        let moscow = zone(Arc::new(|| Some("Europe/Moscow".into())));
        assert_eq!(
            run(&moscow),
            Ok(Fix {
                latitude: 55.8,
                longitude: 37.6,
                accuracy_km: 500
            })
        );
        let utc = zone(Arc::new(|| Some("Etc/UTC".into())));
        assert_eq!(
            run(&utc),
            Err("the time zone: Etc/UTC names no place".into())
        );
        let unknown = zone(Arc::new(|| None));
        assert!(run(&unknown).unwrap_err().contains("not known"));
    }

    #[test]
    fn a_set_city_is_the_only_source_and_wifi_only_when_turned_on() {
        let names = |settings: LocationSettings| -> Vec<&'static str> {
            sources(&settings).iter().map(Source::name).collect()
        };
        assert_eq!(
            names(LocationSettings::default()),
            ["the location service", "the time zone"]
        );
        assert_eq!(
            names(LocationSettings {
                city: String::new(),
                wifi_lookup: true,
                ..LocationSettings::default()
            }),
            ["the location service", "Wi-Fi", "the time zone"]
        );
        assert_eq!(
            names(LocationSettings {
                city: "Berlin".into(),
                wifi_lookup: true,
                ..LocationSettings::default()
            }),
            ["the city in the settings"]
        );
    }
}
