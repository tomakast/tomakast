//! The computer's location, from the operating system's location service, at about city
//! accuracy: one blocking call, [`locate`]. And the Wi-Fi networks around, for a lookup:
//! [`wifi_networks`]. The plugin host calls them on threads of their own and rounds what it
//! gets.
//!
//! - Linux: the XDG desktop portal's Location interface (GeoClue behind it).
//! - macOS: Core Location, on the calling thread with its own run loop.
//! - Windows: `Windows.Devices.Geolocation.Geolocator`.
//!
//! This crate depends on nothing of the launcher, so it type-checks for every target from one
//! machine.

use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// A position from the operating system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    /// Degrees.
    pub latitude: f64,
    pub longitude: f64,
    /// How far the real place may be from it, in metres, when the service says.
    pub accuracy_m: Option<f64>,
}

/// A Wi-Fi network the computer last saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    /// The access point's address (BSSID), as `aa:bb:cc:dd:ee:ff`.
    pub bssid: String,
    /// The network's name as bytes (not always UTF-8); empty for a hidden network.
    pub ssid: Vec<u8>,
    /// The signal, 0 to 100 %.
    pub strength: u8,
}

/// Where the computer is, or why that is not known; waits at most about `timeout`.
pub fn locate(timeout: Duration) -> Result<Fix, String> {
    #[cfg(target_os = "linux")]
    return linux::locate(timeout);
    #[cfg(target_os = "macos")]
    return macos::locate(timeout);
    #[cfg(windows)]
    return windows::locate(timeout);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = timeout;
        Err("location is not available on this operating system".into())
    }
}

/// The Wi-Fi networks the computer saw in its last scan (none is started), or why they cannot
/// be listed. Only Linux, through NetworkManager.
pub fn wifi_networks() -> Result<Vec<Network>, String> {
    #[cfg(target_os = "linux")]
    return linux::wifi_networks();
    #[cfg(not(target_os = "linux"))]
    Err("the Wi-Fi lookup is only on Linux".into())
}
