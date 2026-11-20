//! Result formatting.

use super::currency;
use super::units::{self, Dim};
use super::{Base, Value};

/// Significant digits kept in a result.
const SIGNIFICANT: usize = 10;

/// Integers up to this print in full, without an exponent.
const MAX_PLAIN_INTEGER: f64 = 1e15;

/// Formats a value: at most 10 significant digits, trailing zeros trimmed, no grouping, the
/// unit after a space. Non-finite numbers cannot come out of the evaluator; they are named
/// here only so that formatting never prints `inf` or `NaN`.
pub fn format(value: &Value) -> String {
    match *value {
        Value::Num { number: n, unit } => match unit {
            Some(unit) if unit.dim == Dim::Currency => {
                format!("{} {}", money(n, currency::is_crypto(unit.name)), unit.name)
            }
            Some(unit) => format!("{} {}", number(n), units::display(unit, n)),
            None => number(n),
        },
        Value::Based { number, base } => based(number, base),
        Value::Moment(m) => super::time::format(&m),
    }
}

/// The number part of an amount of money. Fiat has 2 decimals, but one below 0.01 (not 0) is
/// shown as crypto is: 8 significant digits with trailing zeros trimmed. The integer part is
/// grouped by 3 with a space. From 1e15 up it is [`number`]'s.
pub fn money(x: f64, crypto: bool) -> String {
    if x.abs() >= MAX_PLAIN_INTEGER || !x.is_finite() {
        return number(x);
    }
    if x == 0.0 {
        return if crypto { "0" } else { "0.00" }.into();
    }
    let text = if crypto || x.abs() < 0.01 {
        // Round first: the exponent of the rounded value says how many decimals are left.
        let sci = format!("{:.*e}", MONEY_DIGITS - 1, x);
        let (mantissa, exp) = sci.split_once('e').expect("`{:e}` always has an exponent");
        let exp: i32 = exp.parse().expect("`{:e}` exponent is an integer");
        if exp < -MONEY_MIN_EXP {
            return format!("{}e{exp}", trim(mantissa));
        }
        let decimals = (MONEY_DIGITS as i32 - 1 - exp).max(0) as usize;
        trim(&format!("{x:.decimals$}"))
    } else {
        format!("{x:.2}")
    };
    group(&text)
}

/// Significant digits of a small or crypto amount.
const MONEY_DIGITS: usize = 8;

/// Amounts below 1e-12 are written in scientific notation, not with a long run of zeros.
const MONEY_MIN_EXP: i32 = 12;

/// `text`, a decimal number, with its integer part grouped by 3 using spaces.
fn group(text: &str) -> String {
    let (sign, text) = text.strip_prefix('-').map_or(("", text), |t| ("-", t));
    let (int, frac) = text.split_once('.').map_or((text, ""), |(i, f)| (i, f));
    let mut out = String::from(sign);
    for (i, digit) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(digit);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// A whole number in base 2, 8 or 16 with its prefix: `0xff`, `-0b101`.
fn based(number: f64, base: Base) -> String {
    let n = number.abs() as u64;
    let sign = if number < 0.0 { "-" } else { "" };
    match base {
        Base::Bin => format!("{sign}0b{n:b}"),
        Base::Oct => format!("{sign}0o{n:o}"),
        Base::Hex => format!("{sign}0x{n:x}"),
    }
}

/// Formats a bare number the way [`format`] does. Between 1e-6 and 1e15 it prints fixed
/// notation with 10 significant digits, but never fewer digits than the integer part:
/// a non-integer from 1e10 up is rounded to a whole number, so it keeps up to 15
/// significant digits (`98765432109.87` prints `98765432110`). Outside that range it prints
/// `1.5e20` / `1.5e-7`.
pub fn number(x: f64) -> String {
    if x.is_nan() {
        return "not a number".into();
    }
    if x.is_infinite() {
        return "too large".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.fract() == 0.0 && x.abs() <= MAX_PLAIN_INTEGER {
        return format!("{}", x as i64);
    }
    // Round to the significant digits first: the exponent of the rounded value decides
    // between fixed and scientific notation (`9.99999999999` rounds up to 10).
    let sci = format!("{:.*e}", SIGNIFICANT - 1, x);
    let (mantissa, exp) = sci.split_once('e').expect("`{:e}` always has an exponent");
    let exp: i32 = exp.parse().expect("`{:e}` exponent is an integer");
    if (-6..15).contains(&exp) {
        let decimals = (SIGNIFICANT as i32 - 1 - exp).max(0) as usize;
        trim(&format!("{x:.decimals$}"))
    } else {
        format!("{}e{exp}", trim(mantissa))
    }
}

/// Trims trailing zeros of a decimal fraction, and the point if nothing is left after it.
fn trim(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_table() {
        for (x, want) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-12.0, "-12"),
            (0.5, "0.5"),
            (1.0 / 3.0, "0.3333333333"),
            (-2.0 / 3.0, "-0.6666666667"),
            (123456.789012345, "123456.789"),
            (0.1 + 0.2, "0.3"),
            (9.99999999999, "10"),
            (999999999.95, "1000000000"),
            (12345678901.4, "12345678901"),
            (98765432109.87, "98765432110"),
            (1e14, "100000000000000"),
            (999_999_999_999_999.0, "999999999999999"),
            (1e15, "1000000000000000"),
            (1e16, "1e16"),
            (1.5e20, "1.5e20"),
            (1.234567890123e15, "1.23456789e15"),
            (1e-6, "0.000001"),
            (1.5e-6, "0.0000015"),
            (1.5e-7, "1.5e-7"),
            (-1.5e-7, "-1.5e-7"),
            (1e308, "1e308"),
            (f64::NAN, "not a number"),
            (f64::INFINITY, "too large"),
            (f64::NEG_INFINITY, "too large"),
        ] {
            assert_eq!(number(x), want, "{x:e}");
        }
    }

    #[test]
    fn money_table() {
        assert_eq!(money(9312.5, false), "9 312.50");
        assert_eq!(money(6_239_375.0, false), "6 239 375.00");
        assert_eq!(money(0.05223880597014925, true), "0.052238806");
        assert_eq!(money(12345.678901, true), "12 345.679");
        assert_eq!(money(-12.3, false), "-12.30");
        assert_eq!(money(0.004, false), "0.004");
        assert_eq!(money(0.0, false), "0.00");
        assert_eq!(money(1e16, false), "1e16");
    }
}
