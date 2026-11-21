//! The XDG desktop portal's Location interface. The portal asks GeoClue and may show the
//! desktop's own permission dialog; without GeoClue, or with location off in the desktop's
//! settings, it answers with an error. And the Wi-Fi networks NetworkManager last saw.

use std::time::Duration;

use ashpd::desktop::location::{Accuracy, CreateSessionOptions, LocationProxy};
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

use crate::{Fix, Network};

/// The launcher window's app id, under which the portal keeps the user's decision (the hotkey
/// code registers the same one; a second registration fails harmlessly).
const APP_ID: &str = "dev.tomakast.Tomakast";

pub(crate) fn locate(timeout: Duration) -> Result<Fix, String> {
    async_io::block_on(async {
        let work = std::pin::pin!(ask());
        let timer = std::pin::pin!(async_io::Timer::after(timeout));
        match select(work, timer).await {
            Either::Left((answer, _)) => answer,
            Either::Right(_) => Err(format!("no location within {} s", timeout.as_secs())),
        }
    })
}

async fn ask() -> Result<Fix, String> {
    if let Ok(id) = ashpd::AppID::try_from(APP_ID) {
        let _ = ashpd::register_host_app(id).await;
    }
    let proxy = LocationProxy::new().await.map_err(why)?;
    let session = proxy
        .create_session(CreateSessionOptions::default().set_accuracy(Accuracy::City))
        .await
        .map_err(why)?;
    let mut updates = proxy.receive_location_updated().await.map_err(why)?;
    let started = proxy
        .start(&session, None, Default::default())
        .await
        .and_then(|request| request.response());
    let answer = match started {
        Ok(()) => match updates.next().await {
            Some(location) => Ok(Fix {
                latitude: location.latitude(),
                longitude: location.longitude(),
                accuracy_m: Some(location.accuracy()).filter(|m| m.is_finite() && *m > 0.),
            }),
            None => Err("the location portal stopped without a location".into()),
        },
        Err(e) => Err(why(e)),
    };
    let _ = session.close().await;
    answer
}

/// What a portal error means for the user.
fn why(e: ashpd::Error) -> String {
    match e {
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => {
            "location was not allowed".into()
        }
        ashpd::Error::PortalNotFound(_) => {
            "the desktop has no location portal (install xdg-desktop-portal)".into()
        }
        other => format!("the location portal failed: {other}"),
    }
}

const NM: &str = "org.freedesktop.NetworkManager";
/// `NM_DEVICE_TYPE_WIFI`.
const WIFI: u32 = 2;

pub(crate) fn wifi_networks() -> Result<Vec<Network>, String> {
    let failed = |e: zbus::Error| format!("NetworkManager did not answer: {e}");
    let bus = Connection::system().map_err(|e| format!("no system bus: {e}"))?;
    let proxy = |path: &OwnedObjectPath, interface: &'static str| {
        Proxy::new(&bus, NM, path.clone(), interface)
    };
    let manager = Proxy::new(&bus, NM, "/org/freedesktop/NetworkManager", NM).map_err(failed)?;
    let devices: Vec<OwnedObjectPath> = manager.call("GetDevices", &()).map_err(failed)?;
    let mut networks = Vec::new();
    for device in &devices {
        let kind: u32 = proxy(device, "org.freedesktop.NetworkManager.Device")
            .and_then(|p| p.get_property("DeviceType"))
            .map_err(failed)?;
        if kind != WIFI {
            continue;
        }
        let points: Vec<OwnedObjectPath> =
            proxy(device, "org.freedesktop.NetworkManager.Device.Wireless")
                .and_then(|p| p.call("GetAllAccessPoints", &()))
                .map_err(failed)?;
        for point in &points {
            // An access point may go between the list and its properties: skip it.
            let Ok(ap) = proxy(point, "org.freedesktop.NetworkManager.AccessPoint") else {
                continue;
            };
            let (Ok(bssid), Ok(ssid), Ok(strength)) = (
                ap.get_property::<String>("HwAddress"),
                ap.get_property::<Vec<u8>>("Ssid"),
                ap.get_property::<u8>("Strength"),
            ) else {
                continue;
            };
            networks.push(Network {
                bssid,
                ssid,
                strength,
            });
        }
    }
    if devices.is_empty() {
        return Err("NetworkManager has no devices".into());
    }
    Ok(networks)
}
