//! Places the launcher and its plugins know without asking anyone: the built-in list of
//! cities ([`cities`]) and where each time zone is ([`zones`]).

pub mod cities;
pub mod zones;

/// A place on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// What it is called: a city as `Name, CC`, the geocoded name, or the coordinates as typed.
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}
