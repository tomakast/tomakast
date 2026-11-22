//! Evaluator: an expression tree to a [`Value`], or an error message.

use std::collections::{HashMap, HashSet};

use super::color::{self, Color, Form};
use super::currency;
use super::parser::{self, ColorFn, Expr, Func, Op, Target};
use super::time::{self, Moment};
use super::units::{self, Dim, Unit};
use super::zones::{self, ZoneId};
use super::{Base, Context, Value};

/// What a line can see: earlier assignments and the previous result.
#[derive(Default)]
pub struct Env {
    pub vars: HashMap<String, Value>,
    pub prev: Option<Value>,
    /// The variables that hold a percentage (`tax = 13%`), so that `salary - tax` takes 13%
    /// off, as `salary - 13%` does.
    pub percents: HashSet<String>,
    /// Whether `prev` is a percentage.
    pub prev_percent: bool,
}

/// Rejects NaN and infinity so that no later step quietly turns them into a wrong number.
fn fin(n: f64) -> Result<f64, String> {
    if n.is_nan() {
        Err("not a number".into())
    } else if n.is_infinite() {
        Err("too large".into())
    } else {
        Ok(n)
    }
}

fn plain(number: f64) -> Result<Value, String> {
    Ok(Value::plain(fin(number)?))
}

fn with_unit(number: f64, unit: Option<Unit>) -> Result<Value, String> {
    Ok(Value::with_unit(fin(number)?, unit))
}

/// The number and unit an operand stands for; `moment` is the message for a date.
fn quantity(v: &Value, moment: &str) -> Result<(f64, Option<Unit>), String> {
    v.quantity().ok_or_else(|| moment.to_string())
}

const CANNOT_MULTIPLY: &str = "cannot multiply a date";

/// A unit's name for an error message; a plain number reads "a number".
fn unit_name(unit: Option<Unit>) -> &'static str {
    unit.map_or("a number", |u| u.name)
}

/// `15%` and `-15%`, or a variable or `prev` that holds one: the left side of `of`, and the
/// one case where `+`/`-` of a percentage does not mean "a share of the left side".
pub fn is_percent(env: &Env, e: &Expr) -> bool {
    match e {
        Expr::Percent(_) => true,
        Expr::Neg(inner) => is_percent(env, inner),
        Expr::Name(name) if env.vars.contains_key(name) => env.percents.contains(name),
        Expr::Name(name) if matches!(name.as_str(), "prev" | "ans") => env.prev_percent,
        _ => false,
    }
}

pub fn eval(env: &Env, ctx: &Context, e: &Expr) -> Result<Value, String> {
    match e {
        Expr::Lit(v) => Ok(*v),
        Expr::Radix(n) => plain(*n),
        Expr::Name(name) => name_value(env, ctx, name),
        Expr::Neg(inner) => {
            let v = eval(env, ctx, inner)?;
            let what = if is_color(&v) { "color" } else { "date" };
            let (n, unit) = quantity(&v, &format!("cannot negate a {what}"))?;
            with_unit(-n, unit)
        }
        Expr::Percent(inner) => {
            let (n, unit) = quantity(&eval(env, ctx, inner)?, PERCENT_PLAIN)?;
            if unit.is_some() {
                return Err(PERCENT_PLAIN.into());
            }
            plain(n / 100.0)
        }
        Expr::Of(left, right) => {
            if !is_percent(env, left) {
                return Err("'of' needs a percentage on its left".into());
            }
            let (share, _) = quantity(&eval(env, ctx, left)?, PERCENT_PLAIN)?;
            let whole = eval(env, ctx, right)?;
            if is_color(&whole) {
                return Err(color_binary(Op::Mul));
            }
            let (whole, unit) = quantity(&whole, CANNOT_MULTIPLY)?;
            with_unit(share * whole, unit)
        }
        Expr::Bin(op, left, right) => {
            let (a, b) = (eval(env, ctx, left)?, eval(env, ctx, right)?);
            if is_color(&a) || is_color(&b) {
                return Err(color_binary(*op));
            }
            let (Some(a), Some(b)) = (a.quantity(), b.quantity()) else {
                return moment_binary(*op, a, b);
            };
            // `80 + 15%` is 80 plus 15% of 80 and `80 - -10%` is 80 plus 10% of 80, but
            // `15% + 5%` is plain addition.
            if matches!(op, Op::Add | Op::Sub) && is_percent(env, right) && !is_percent(env, left) {
                let sign = if *op == Op::Add { 1.0 } else { -1.0 };
                return with_unit(a.0 * (1.0 + sign * b.0), a.1);
            }
            binary(ctx, *op, a, b)
        }
        Expr::Call(func, arg) => match eval(env, ctx, arg)?.quantity() {
            Some(q) => call(*func, q),
            None => Err(format!("{} needs a plain number", func.name())),
        },
        Expr::Convert(inner, Target::Zone(zone)) if relative_day(env, inner).is_some() => {
            let offset = relative_day(env, inner).expect("checked by the guard");
            Ok(Value::Moment(time::day_in(ctx, offset, *zone)?))
        }
        Expr::Convert(inner, target) => convert(ctx, eval(env, ctx, inner)?, *target),
        Expr::ColorCall { func, args, alpha } => color_call(env, ctx, *func, args, alpha),
        Expr::Clock { hour, minute, zone } => {
            let zone = zone.unwrap_or_else(|| home(ctx));
            Ok(Value::Moment(time::at_clock(ctx, *hour, *minute, zone)?))
        }
        Expr::Date { day, month, year } => {
            Ok(Value::Moment(time::on_date(ctx, *day, *month, *year)?))
        }
        Expr::Until(unit, target) => {
            ctx.mark_zone();
            let Value::Moment(moment) = eval(env, ctx, target)? else {
                return Err("'until' needs a date".into());
            };
            let roll = matches!(**target, Expr::Date { year: None, .. });
            time::until(ctx, *unit, moment, roll)
        }
    }
}

const PERCENT_PLAIN: &str = "a percentage needs a plain number";

fn is_color(v: &Value) -> bool {
    matches!(v, Value::Color(_))
}

/// The error of an operator with a colour on either side.
fn color_binary(op: Op) -> String {
    match op {
        Op::Add => "cannot add a color".into(),
        Op::Sub => "cannot subtract a color".into(),
        Op::Mul => "cannot multiply a color".into(),
        Op::Div => "cannot divide a color".into(),
        Op::Mod => "mod needs plain numbers".into(),
        Op::Pow => "cannot raise a color to a power".into(),
    }
}

/// `rgb(…)`, `hsl(…)`, `oklch(…)`. A value written as a percentage is a share of its
/// range; a hue may carry an angle unit; every other value is a plain number.
fn color_call(
    env: &Env,
    ctx: &Context,
    func: ColorFn,
    args: &[Expr],
    alpha: &Option<Box<Expr>>,
) -> Result<Value, String> {
    let plain_only = || format!("{} needs plain numbers", func.name());
    // The number of an argument and whether it was a percentage.
    let read = |e: &Expr, hue: bool| -> Result<(f64, bool), String> {
        match eval(env, ctx, e)? {
            Value::Num {
                number,
                unit: Some(unit),
            } if hue && unit.dim == Dim::Angle => {
                let deg = units::lookup("deg").expect("`deg` is in the table");
                Ok((units::convert(number, unit, deg).unwrap_or(number), false))
            }
            v => match v.quantity() {
                Some((n, None)) => Ok((n, is_percent(env, e))),
                _ => Err(plain_only()),
            },
        }
    };
    let a = match alpha {
        Some(e) => read(e, false)?.0,
        None => 1.0,
    };
    let (x, xp) = read(&args[0], func == ColorFn::Hsl)?;
    let (y, yp) = read(&args[1], false)?;
    let (z, zp) = read(&args[2], func == ColorFn::Oklch)?;
    let color = match func {
        // A percentage of a channel is a share of 255.
        ColorFn::Rgb => {
            let ch = |n: f64, p: bool| if p { n * 255.0 } else { n };
            color::from_rgb(ch(x, xp), ch(y, yp), ch(z, zp), a)
        }
        // Saturation and lightness written as plain numbers are percentages too.
        ColorFn::Hsl => {
            let share = |n: f64, p: bool| if p { n } else { n / 100.0 };
            color::from_hsl(x, share(y, yp), share(z, zp), a)
        }
        ColorFn::Oklch => color::from_oklch(x, y, z, a),
    };
    Ok(Value::Color(Color {
        form: Form::Hex,
        ..color
    }))
}

/// The user's zone, noted as used.
fn home(ctx: &Context) -> ZoneId {
    ctx.mark_zone();
    ctx.zone
}

/// `in` / `to` a target.
fn convert(ctx: &Context, value: Value, target: Target) -> Result<Value, String> {
    let quantity = value.quantity();
    let from = |to: &str| match quantity {
        Some((_, Some(unit))) => format!("cannot convert {} to {to}", unit.name),
        _ => format!("cannot convert a plain number to {to}"),
    };
    let of_color = |to: &str| Err(format!("cannot convert a color to {to}"));
    match (target, value) {
        (Target::Color(form), Value::Color(c)) => Ok(Value::Color(Color { form, ..c })),
        (Target::Color(form), _) => Err(format!("{} needs a color", parser::form_word(form))),
        (Target::Base(Some(Base::HEX)), Value::Color(c)) => Ok(Value::Color(Color {
            form: Form::Hex,
            ..c
        })),
        (Target::Unit(to), Value::Color(_)) => of_color(to.name),
        (Target::Zone(zone), Value::Color(_)) => of_color(zones::name(zone)),
        (Target::Date, Value::Color(_)) => of_color("a date"),
        (Target::Unix, Value::Color(_)) => of_color("unix"),
        (Target::Base(base), _) => to_base(value, base),
        (Target::Unit(to), Value::Moment(_)) => {
            Err(format!("cannot convert a date to {}", to.name))
        }
        (Target::Unit(to), _) => {
            let Some((number, Some(unit))) = quantity else {
                return Err(from(to.name));
            };
            let converted = if both_currencies(unit, to) {
                rate_convert(ctx, number, unit, to).map(Some)?
            } else {
                units::convert(number, unit, to)
            };
            let n =
                converted.ok_or_else(|| format!("cannot convert {} to {}", unit.name, to.name))?;
            with_unit(n, Some(to))
        }
        // A date stays that date; a moment is the same instant, shown with the time there.
        (Target::Zone(zone), Value::Moment(m)) if m.date_only => {
            Ok(Value::Moment(time::date_in(m, zone)?))
        }
        (Target::Zone(zone), Value::Moment(m)) => Ok(Value::Moment(Moment { zone, ..m })),
        (Target::Zone(zone), _) => Err(from(zones::name(zone))),
        (Target::Date, Value::Moment(_)) => Ok(value),
        (Target::Date, _) => match quantity {
            Some((seconds, None)) => {
                // Unix seconds from the epoch, range-checked like any added time.
                let epoch = Moment {
                    unix: 0,
                    zone: home(ctx),
                    date_only: false,
                };
                let second = units::lookup("s").expect("`s` is a unit");
                Ok(Value::Moment(time::add(epoch, seconds.floor(), second)?))
            }
            _ => Err(from("a date")),
        },
        (Target::Unix, Value::Moment(m)) => plain(m.unix as f64),
        (Target::Unix, _) => Err(from("unix")),
    }
}

fn both_currencies(a: Unit, b: Unit) -> bool {
    a.dim == Dim::Currency && b.dim == Dim::Currency
}

/// `number` of currency `from` in currency `to`, through USD. A conversion that needs a feed
/// marks it as used, found or not; one between a currency and itself or USD needs none.
fn rate_convert(ctx: &Context, number: f64, from: Unit, to: Unit) -> Result<f64, String> {
    if from.name == to.name {
        return Ok(number);
    }
    ctx.mark_rate(from.name);
    ctx.mark_rate(to.name);
    let (from, to) = (ctx.rates.usd_worth(from.name), ctx.rates.usd_worth(to.name));
    Ok(number * from? / to?)
}

/// A difference or a size `number` of `from` counted in `to`, which is `None` when the
/// dimensions differ: the conversion for `+`, `-` and `/`. Currencies go through the rates.
fn interval(ctx: &Context, number: f64, from: Unit, to: Unit) -> Result<Option<f64>, String> {
    if both_currencies(from, to) {
        rate_convert(ctx, number, from, to).map(Some)
    } else {
        Ok(units::convert_interval(number, from, to))
    }
}

/// `+`, `-`, `*`, `/`, `^` and `mod` where one side is a moment.
fn moment_binary(op: Op, a: Value, b: Value) -> Result<Value, String> {
    let time_unit = |v: &Value| match v.quantity() {
        Some((n, Some(unit))) if unit.dim == Dim::Time => Some((n, unit)),
        _ => None,
    };
    let what = |v: &Value| unit_name(v.quantity().and_then(|(_, unit)| unit));
    match (op, a, b) {
        (Op::Add, Value::Moment(_), Value::Moment(_)) => Err("cannot add two dates".into()),
        (Op::Sub, Value::Moment(x), Value::Moment(y)) => Ok(time::between(x, y)),
        (Op::Add | Op::Sub, Value::Moment(m), other) => match time_unit(&other) {
            Some((n, unit)) => {
                let n = if op == Op::Add { n } else { -n };
                Ok(Value::Moment(time::add(m, n, unit)?))
            }
            None => Err(format!("cannot add a date and {}", what(&other))),
        },
        (Op::Add, other, Value::Moment(m)) => match time_unit(&other) {
            Some((n, unit)) => Ok(Value::Moment(time::add(m, n, unit)?)),
            None => Err(format!("cannot add a date and {}", what(&other))),
        },
        (Op::Sub, other, _) => Err(format!("cannot subtract a date from {}", what(&other))),
        _ => Err(CANNOT_MULTIPLY.into()),
    }
}

/// The largest whole number shown in another base: 2^53, where `f64` stops being exact.
const MAX_BASED: f64 = 9_007_199_254_740_992.0;

/// `255 in hex`: a plain whole number in a base, or (for `decimal`) a plain number as is.
fn to_base(value: Value, base: Option<Base>) -> Result<Value, String> {
    let word = parser::base_word(base);
    let Some((number, None)) = value.quantity() else {
        return Err(format!("{word} needs a plain number"));
    };
    let Some(base) = base else {
        return plain(number);
    };
    if number.fract() != 0.0 {
        return Err(format!("{word} needs a whole number"));
    }
    if number.abs() > MAX_BASED {
        return Err(format!("too large for {word}"));
    }
    Ok(Value::Based { number, base })
}

/// `now` and `time` (`None`), or `today`, `tomorrow` and `yesterday` (the day offset), in
/// any case.
pub fn time_word(name: &str) -> Option<Option<i64>> {
    [
        ("now", None),
        ("time", None),
        ("today", Some(0)),
        ("tomorrow", Some(1)),
        ("yesterday", Some(-1)),
    ]
    .into_iter()
    .find(|(word, _)| name.eq_ignore_ascii_case(word))
    .map(|(_, offset)| offset)
}

/// The day offset of `today`, `tomorrow` or `yesterday` written alone (not a variable of
/// that name).
fn relative_day(env: &Env, e: &Expr) -> Option<i64> {
    match e {
        Expr::Name(name) if !env.vars.contains_key(name) => time_word(name).flatten(),
        _ => None,
    }
}

fn name_value(env: &Env, ctx: &Context, name: &str) -> Result<Value, String> {
    match name {
        "pi" => plain(std::f64::consts::PI),
        "e" => plain(std::f64::consts::E),
        "prev" | "ans" => env.prev.ok_or_else(|| "no previous result".to_string()),
        _ => match env.vars.get(name) {
            Some(v) => Ok(*v),
            None => match time_word(name) {
                Some(None) => Ok(Value::Moment(time::now(ctx))),
                Some(Some(offset)) => Ok(Value::Moment(time::day(ctx, offset)?)),
                None if units::lookup(name).is_some() => {
                    Err(format!("'{name}' is a unit; put a number before it"))
                }
                None if currency::lookup(name).is_some() => {
                    Err(format!("'{name}' is a currency; put a number before it"))
                }
                None if color::named(name).is_some() => Ok(Value::Color(
                    color::named(name).expect("checked by the guard"),
                )),
                None => Err(format!("unknown name '{name}'")),
            },
        },
    }
}

type Quantity = (f64, Option<Unit>);

/// Fuel per distance next to another unit in `*` or `/` is what it was before it became one
/// unit: litres per 100 km, an area (`8 l/100km * 250 km` is `20 l`).
fn fuel_as_area((n, unit): Quantity, other: Option<Unit>) -> Quantity {
    match (unit, other) {
        (Some(u), Some(o)) if u.inverse && o.dim != u.dim => {
            let mm2 = units::lookup("mm2").expect("`mm2` is in the table");
            (n / 100.0, Some(mm2))
        }
        _ => (n, unit),
    }
}

fn binary(ctx: &Context, op: Op, a: Quantity, b: Quantity) -> Result<Value, String> {
    let ((an, au), (bn, bu)) = if matches!(op, Op::Mul | Op::Div) {
        (fuel_as_area(a, b.1), fuel_as_area(b, a.1))
    } else {
        (a, b)
    };
    match op {
        Op::Add | Op::Sub => {
            let (word, sign) = if op == Op::Add {
                ("add", 1.0)
            } else {
                ("subtract", -1.0)
            };
            match (au, bu) {
                (None, None) => plain(an + sign * bn),
                (Some(x), Some(y)) => match interval(ctx, bn, y, x)? {
                    Some(n) => with_unit(an + sign * n, Some(x)),
                    None => Err(format!("cannot {word} {} and {}", x.name, y.name)),
                },
                (x, y) => Err(format!(
                    "cannot {word} {} and {}",
                    unit_name(x),
                    unit_name(y)
                )),
            }
        }
        Op::Mul => match (au, bu) {
            (Some(x), Some(y)) => match units::derive(x, y, false) {
                Some((k, unit)) => with_unit(an * bn * k, Some(unit)),
                None => Err(format!("cannot multiply {} and {}", x.name, y.name)),
            },
            (x, y) => with_unit(an * bn, x.or(y)),
        },
        Op::Div => {
            if bn == 0.0 {
                return Err("division by zero".into());
            }
            match (au, bu) {
                (x, None) => with_unit(an / bn, x),
                (None, Some(y)) => Err(format!("cannot divide a number by {}", y.name)),
                (Some(x), Some(y)) => match interval(ctx, bn, y, x)? {
                    Some(b) => plain(an / b),
                    None => match units::derive(x, y, true) {
                        Some((k, unit)) => with_unit(an / bn * k, Some(unit)),
                        None => Err(format!("cannot divide {} by {}", x.name, y.name)),
                    },
                },
            }
        }
        Op::Mod => {
            if au.is_some() || bu.is_some() {
                return Err("mod needs plain numbers".into());
            }
            if bn == 0.0 {
                return Err("division by zero".into());
            }
            plain(an % bn)
        }
        Op::Pow => {
            if au.is_some() {
                return Err("cannot raise a unit value to a power".into());
            }
            if bu.is_some() {
                return Err("cannot raise to a unit value".into());
            }
            if an == 0.0 && bn < 0.0 {
                return Err("division by zero".into());
            }
            plain(an.powf(bn))
        }
    }
}

fn call(func: Func, (n, unit): Quantity) -> Result<Value, String> {
    // These keep the unit; the rest need a plain number.
    match func {
        Func::Abs => return with_unit(n.abs(), unit),
        Func::Round => return with_unit(n.round(), unit),
        Func::Floor => return with_unit(n.floor(), unit),
        Func::Ceil => return with_unit(n.ceil(), unit),
        _ => {}
    }
    // The trigonometric functions take radians, or an angle.
    let n = match unit {
        None => n,
        Some(u) if u.dim == Dim::Angle && matches!(func, Func::Sin | Func::Cos | Func::Tan) => {
            let rad = units::lookup("rad").expect("`rad` is in the table");
            units::convert(n, u, rad).ok_or_else(|| "not a number".to_string())?
        }
        Some(_) => return Err(format!("{} needs a plain number", func.name())),
    };
    match func {
        Func::Sqrt => plain(n.sqrt()),
        // ln(0) is -inf and a negative argument is NaN; neither is a number to show.
        Func::Ln | Func::Log if n <= 0.0 => Err("not a number".into()),
        Func::Ln => plain(n.ln()),
        Func::Log => plain(n.log10()),
        Func::Sin => plain(n.sin()),
        Func::Cos => plain(n.cos()),
        Func::Tan => plain(n.tan()),
        Func::Abs | Func::Round | Func::Floor | Func::Ceil => unreachable!("returned above"),
    }
}
