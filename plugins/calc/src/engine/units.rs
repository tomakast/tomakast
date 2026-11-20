//! Unit table and conversion.

/// A physical dimension; only units of one dimension convert into each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Length,
    Mass,
    Time,
    Temperature,
    Area,
    Volume,
    Speed,
    Data,
    Angle,
    /// Money: converts through the exchange rates, not through `factor`.
    Currency,
}

/// A unit: `base = number * factor + offset`, where the base unit of each dimension is
/// metre, gram, second, kelvin, square metre, litre, metre per second, bit and radian.
/// Only temperatures have an offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    pub name: &'static str,
    pub dim: Dim,
    pub factor: f64,
    pub offset: f64,
}

const fn unit(name: &'static str, dim: Dim, factor: f64) -> Unit {
    Unit {
        name,
        dim,
        factor,
        offset: 0.0,
    }
}

const fn affine(name: &'static str, dim: Dim, factor: f64, offset: f64) -> Unit {
    Unit {
        name,
        dim,
        factor,
        offset,
    }
}

/// A unit with the other names it answers to. Names are case-sensitive.
type Entry = (Unit, &'static [&'static str]);

const KIB: f64 = 8.0 * 1024.0;

const UNITS: &[Entry] = &[
    (
        unit("mm", Dim::Length, 0.001),
        &["millimeter", "millimeters", "millimetre", "millimetres"],
    ),
    (
        unit("cm", Dim::Length, 0.01),
        &["centimeter", "centimeters", "centimetre", "centimetres"],
    ),
    (
        unit("m", Dim::Length, 1.0),
        &["meter", "meters", "metre", "metres"],
    ),
    (
        unit("km", Dim::Length, 1000.0),
        &["kilometer", "kilometers", "kilometre", "kilometres"],
    ),
    (unit("in", Dim::Length, 0.0254), &["inch", "inches"]),
    (unit("ft", Dim::Length, 0.3048), &["foot", "feet"]),
    (unit("yd", Dim::Length, 0.9144), &["yard", "yards"]),
    (unit("mi", Dim::Length, 1609.344), &["mile", "miles"]),
    (unit("mg", Dim::Mass, 0.001), &["milligram", "milligrams"]),
    (unit("g", Dim::Mass, 1.0), &["gram", "grams"]),
    (
        unit("kg", Dim::Mass, 1000.0),
        &["kilogram", "kilograms", "kilo", "kilos"],
    ),
    (
        unit("t", Dim::Mass, 1_000_000.0),
        &["tonne", "tonnes", "ton", "tons"],
    ),
    (unit("oz", Dim::Mass, 28.349_523_125), &["ounce", "ounces"]),
    (
        unit("lb", Dim::Mass, 453.592_37),
        &["pound", "pounds", "lbs"],
    ),
    (
        unit("ms", Dim::Time, 0.001),
        &["millisecond", "milliseconds"],
    ),
    (
        unit("s", Dim::Time, 1.0),
        &["second", "seconds", "sec", "secs"],
    ),
    (unit("min", Dim::Time, 60.0), &["minute", "minutes", "mins"]),
    (
        unit("h", Dim::Time, 3600.0),
        &["hour", "hours", "hr", "hrs"],
    ),
    (unit("day", Dim::Time, 86_400.0), &["days"]),
    (unit("week", Dim::Time, 604_800.0), &["weeks"]),
    (unit("month", Dim::Time, 2_629_746.0), &["months"]),
    (
        unit("year", Dim::Time, 31_556_952.0),
        &["years", "yr", "yrs"],
    ),
    (
        affine("°C", Dim::Temperature, 1.0, 273.15),
        &["C", "c", "celsius", "degC"],
    ),
    (
        affine("°F", Dim::Temperature, 5.0 / 9.0, 459.67 * 5.0 / 9.0),
        &["F", "f", "fahrenheit", "degF"],
    ),
    (unit("K", Dim::Temperature, 1.0), &["kelvin", "kelvins"]),
    (unit("mm2", Dim::Area, 1e-6), &["mm²"]),
    (unit("cm2", Dim::Area, 1e-4), &["cm²"]),
    (unit("m2", Dim::Area, 1.0), &["m²"]),
    (unit("km2", Dim::Area, 1e6), &["km²"]),
    (unit("ha", Dim::Area, 1e4), &["hectare", "hectares"]),
    (unit("acre", Dim::Area, 4_046.856_422_4), &["acres"]),
    (unit("ft2", Dim::Area, 0.092_903_04), &["ft²", "sqft"]),
    (unit("in2", Dim::Area, 0.000_645_16), &["in²"]),
    (unit("mi2", Dim::Area, 2_589_988.110_336), &["mi²"]),
    (
        unit("ml", Dim::Volume, 0.001),
        &[
            "mL",
            "milliliter",
            "milliliters",
            "millilitre",
            "millilitres",
        ],
    ),
    (
        unit("l", Dim::Volume, 1.0),
        &["L", "liter", "liters", "litre", "litres"],
    ),
    (unit("m3", Dim::Volume, 1000.0), &["m³"]),
    (
        unit("tsp", Dim::Volume, 0.004_928_921_593_75),
        &["teaspoon", "teaspoons"],
    ),
    (
        unit("tbsp", Dim::Volume, 0.014_786_764_781_25),
        &["tablespoon", "tablespoons"],
    ),
    (unit("floz", Dim::Volume, 0.029_573_529_562_5), &[]),
    (unit("cup", Dim::Volume, 0.236_588_236_5), &["cups"]),
    (unit("pt", Dim::Volume, 0.473_176_473), &["pint", "pints"]),
    (unit("qt", Dim::Volume, 0.946_352_946), &["quart", "quarts"]),
    (
        unit("gal", Dim::Volume, 3.785_411_784),
        &["gallon", "gallons"],
    ),
    (unit("m/s", Dim::Speed, 1.0), &["mps"]),
    (unit("km/h", Dim::Speed, 1.0 / 3.6), &["kph", "kmh"]),
    (unit("mph", Dim::Speed, 0.447_04), &[]),
    (
        unit("kn", Dim::Speed, 1852.0 / 3600.0),
        &["knot", "knots", "kt"],
    ),
    (unit("bit", Dim::Data, 1.0), &["bits"]),
    (unit("B", Dim::Data, 8.0), &["byte", "bytes"]),
    (unit("KB", Dim::Data, 8e3), &["kB", "kilobyte", "kilobytes"]),
    (unit("MB", Dim::Data, 8e6), &["megabyte", "megabytes"]),
    (unit("GB", Dim::Data, 8e9), &["gigabyte", "gigabytes"]),
    (unit("TB", Dim::Data, 8e12), &["terabyte", "terabytes"]),
    (unit("KiB", Dim::Data, KIB), &["kibibyte", "kibibytes"]),
    (
        unit("MiB", Dim::Data, KIB * 1024.0),
        &["mebibyte", "mebibytes"],
    ),
    (
        unit("GiB", Dim::Data, KIB * 1024.0 * 1024.0),
        &["gibibyte", "gibibytes"],
    ),
    (
        unit("TiB", Dim::Data, KIB * 1024.0 * 1024.0 * 1024.0),
        &["tebibyte", "tebibytes"],
    ),
    (
        unit("deg", Dim::Angle, std::f64::consts::PI / 180.0),
        &["°", "degree", "degrees"],
    ),
    (unit("rad", Dim::Angle, 1.0), &["radian", "radians"]),
];

/// Looks a unit up by its canonical name or an alias, both case-sensitive.
pub fn lookup(name: &str) -> Option<Unit> {
    UNITS
        .iter()
        .find(|(u, aliases)| u.name == name || aliases.contains(&name))
        .map(|(u, _)| *u)
}

/// Every unit name and alias.
#[cfg(test)]
pub fn all_names() -> Vec<&'static str> {
    UNITS
        .iter()
        .flat_map(|(u, aliases)| std::iter::once(u.name).chain(aliases.iter().copied()))
        .collect()
}

/// The names that were units before conversions grew, but `t`: they cannot be variables.
/// Every other unit name is soft, and a variable of that name wins wherever a name may stand.
/// `t` is soft so that a sheet can say `t = 2 h` (`3 t` is still three tonnes).
pub fn reserved(name: &str) -> bool {
    matches!(
        name,
        "mm" | "cm"
            | "m"
            | "km"
            | "in"
            | "ft"
            | "yd"
            | "mi"
            | "mg"
            | "g"
            | "kg"
            | "oz"
            | "lb"
            | "ms"
            | "s"
            | "min"
            | "h"
            | "day"
            | "week"
    )
}

/// Converts `number` from one unit to another of the same dimension (a point on a scale:
/// `0 °C` is `32 °F`).
pub fn convert(number: f64, from: Unit, to: Unit) -> Option<f64> {
    (from.dim == to.dim).then(|| (number * from.factor + from.offset - to.offset) / to.factor)
}

/// Converts a difference or a size, which ignores offsets (`9 °F` apart is `5 °C` apart):
/// the conversion for `+`, `-` and `/`.
pub fn convert_interval(number: f64, from: Unit, to: Unit) -> Option<f64> {
    (from.dim == to.dim).then(|| number * from.factor / to.factor)
}

/// The powers of length and time a dimension is made of, and how many SI units (metres,
/// seconds and their powers) its base unit is: the dimensions that a product or a quotient
/// of two others can make.
fn powers(dim: Dim) -> Option<(i8, i8, f64)> {
    Some(match dim {
        Dim::Length => (1, 0, 1.0),
        Dim::Time => (0, 1, 1.0),
        Dim::Area => (2, 0, 1.0),
        // The litre.
        Dim::Volume => (3, 0, 0.001),
        Dim::Speed => (1, -1, 1.0),
        _ => return None,
    })
}

/// The unit of `a * b`, or of `a / b` when `div`, for units of dimensions that make another
/// (`km/h * h`, `km / h`, `m * m`, `m2 * m`, `km / km/h`), and the factor its number is the
/// product (or quotient) of the operands' numbers times. The unit is the one whose size the
/// result has (`km/h * h` is `km`, `cm2 * cm` is `ml`), else the base unit of its dimension.
pub fn derive(a: Unit, b: Unit, div: bool) -> Option<(f64, Unit)> {
    let ((la, ta, sa), (lb, tb, sb)) = (powers(a.dim)?, powers(b.dim)?);
    let (l, t, size) = if div {
        (la - lb, ta - tb, a.factor * sa / (b.factor * sb))
    } else {
        (la + lb, ta + tb, a.factor * sa * b.factor * sb)
    };
    let (dim, base) = [Dim::Length, Dim::Time, Dim::Area, Dim::Volume, Dim::Speed]
        .into_iter()
        .find_map(|dim| match powers(dim) {
            Some((dl, dt, s)) if (dl, dt) == (l, t) => Some((dim, s)),
            _ => None,
        })?;
    let size = size / base;
    let unit = UNITS
        .iter()
        .map(|(u, _)| *u)
        .filter(|u| u.dim == dim)
        .find(|u| ((u.factor - size) / size).abs() < 1e-9)
        .or_else(|| {
            UNITS
                .iter()
                .map(|(u, _)| *u)
                .find(|u| u.dim == dim && u.factor == 1.0)
        })?;
    Some((size / unit.factor, unit))
}

/// The name to print: the long units are plural unless the number is exactly 1.
pub fn display(unit: Unit, number: f64) -> &'static str {
    if number == 1.0 {
        return unit.name;
    }
    match unit.name {
        "day" => "days",
        "week" => "weeks",
        "month" => "months",
        "year" => "years",
        name => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_exact() {
        assert_eq!(lookup("km").map(|u| u.dim), Some(Dim::Length));
        assert_eq!(lookup("kg").map(|u| u.dim), Some(Dim::Mass));
        assert_eq!(lookup("min").map(|u| u.dim), Some(Dim::Time));
        assert!(lookup("KM").is_none());
        assert!(lookup("parsec").is_none());
        assert_eq!(lookup("kilometers").map(|u| u.name), Some("km"));
        assert!(lookup("Kilometers").is_none());
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = UNITS
            .iter()
            .flat_map(|(u, aliases)| std::iter::once(u.name).chain(aliases.iter().copied()))
            .collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "a name or alias is used twice");
    }

    #[test]
    fn reserved_is_the_old_set() {
        assert!(reserved("km") && reserved("week"));
        assert!(!reserved("hours") && !reserved("C") && !reserved("cup"));
        // `t` is free for a variable (`t = 2 h`); the other 19 are not.
        assert!(!reserved("t"));
        assert_eq!(UNITS.iter().filter(|(u, _)| reserved(u.name)).count(), 19);
    }

    #[test]
    fn convert_only_within_a_dimension() {
        let (km, m, kg) = (
            lookup("km").unwrap(),
            lookup("m").unwrap(),
            lookup("kg").unwrap(),
        );
        assert_eq!(convert(2.0, km, m), Some(2000.0));
        assert_eq!(convert(2.0, km, kg), None);
    }
}
