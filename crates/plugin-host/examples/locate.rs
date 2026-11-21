//! Asks each of the launcher's location sources on this computer once, in turn, and prints
//! every answer and what plugins would get: `just locate`. The desktop may show its
//! permission dialog for the operating system's service.
//!
//! With `--wifi` (`just locate-wifi`) the Wi-Fi lookup runs too: the networks around and the
//! computer's IP address go to BeaconDB.

use tomakast_core::LocationSettings;
use tomakast_plugin_host::location_sources::{rounded, sources};

fn main() {
    let settings = LocationSettings {
        city: String::new(),
        wifi_lookup: std::env::args().any(|a| a == "--wifi"),
    };
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|e| format!("unknown ({e})"));
    println!("time zone: {zone}");
    let mut found = false;
    for source in sources(&settings) {
        match source.run() {
            Ok(fix) => {
                println!(
                    "{}: {}, {} within {} km",
                    source.name(),
                    fix.latitude,
                    fix.longitude,
                    fix.accuracy_km
                );
                if let Ok(fix) = rounded(fix) {
                    println!(
                        "  plugins would get: {}, {} within {} km",
                        fix.latitude, fix.longitude, fix.accuracy_km
                    );
                }
                found = true;
            }
            Err(why) => println!("{}: no location: {why}", source.name()),
        }
    }
    if !found {
        std::process::exit(1);
    }
}
