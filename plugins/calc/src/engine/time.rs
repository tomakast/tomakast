//! Moments: an instant in a time zone, from the injected `now` (the engine reads no clock).

use jiff::civil::{Date, Time};
use jiff::{Span, Timestamp, ZonedDateTime};

use super::units::{self, Unit};
use super::zones::{self, ZoneId};
use super::{Context, Value};

/// An instant, the zone it is shown in, and whether it is a whole day (`today`, `31 dec`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    pub unix: i64,
    pub zone: ZoneId,
    pub date_only: bool,
}

const OUT_OF_RANGE: &str = "date out of range";
const NOT_A_DATE: &str = "not a date";

/// Month names in English, any case: `jan` … `dec`, `sept` and the full names.
pub(crate) fn month(name: &str) -> Option<u8> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let name = name.to_ascii_lowercase();
    if name == "sept" {
        return Some(9);
    }
    MONTHS
        .iter()
        .position(|full| name == *full || (name.len() == 3 && full.starts_with(&name)))
        .map(|i| i as u8 + 1)
}

/// The moment as jiff sees it. Every [`Moment`] is built from a checked timestamp, so this
/// fails only for a `now` outside jiff's range.
fn zoned(m: &Moment) -> Result<ZonedDateTime, String> {
    let ts = Timestamp::from_second(m.unix).map_err(|_| OUT_OF_RANGE.to_string())?;
    Ok(ts.to_zoned_datetime(zones::tz(m.zone)))
}

fn moment(z: &ZonedDateTime, zone: ZoneId, date_only: bool) -> Moment {
    Moment {
        unix: z.timestamp().as_second(),
        zone,
        date_only,
    }
}

/// The current moment in `ctx.zone`. Not noted as reading the zone: the instant is the same
/// in every zone, and a moment with a time shows its zone.
pub fn now(ctx: &Context) -> Moment {
    Moment {
        unix: ctx.now,
        zone: ctx.zone,
        date_only: false,
    }
}

/// `today` is 0, `tomorrow` 1, `yesterday` -1: midnight in `ctx.zone`, date-only.
pub fn day(ctx: &Context, offset_days: i64) -> Result<Moment, String> {
    ctx.mark_zone();
    day_in(ctx, offset_days, ctx.zone)
}

/// [`day`] in `zone`: `today in Tokyo` is today's date in Tokyo.
pub fn day_in(ctx: &Context, offset_days: i64, zone: ZoneId) -> Result<Moment, String> {
    let now = Moment {
        unix: ctx.now,
        zone,
        date_only: false,
    };
    let today = zoned(&now)?
        .start_of_day()
        .map_err(|_| OUT_OF_RANGE.to_string())?;
    let day = add_span(&today, Span::new().try_days(offset_days))?;
    Ok(moment(&day, zone, true))
}

/// The date `m` shows, as a date in `zone`: `31 dec in Tokyo` is still 31 December.
pub fn date_in(m: Moment, zone: ZoneId) -> Result<Moment, String> {
    let midnight = zoned(&m)?
        .date()
        .to_zoned_datetime(zones::tz(zone))
        .map_err(|_| OUT_OF_RANGE.to_string())?;
    Ok(moment(&midnight, zone, true))
}

/// Today's date in `zone`, at that wall time.
pub fn at_clock(ctx: &Context, hour: u8, minute: u8, zone: ZoneId) -> Result<Moment, String> {
    let today = Timestamp::from_second(ctx.now)
        .map_err(|_| OUT_OF_RANGE.to_string())?
        .to_zoned_datetime(zones::tz(zone))
        .date();
    let time = Time::new(hour as i8, minute as i8, 0, 0).map_err(|_| "not a time".to_string())?;
    let at = today
        .to_datetime(time)
        .to_zoned_datetime(zones::tz(zone))
        .map_err(|_| OUT_OF_RANGE.to_string())?;
    Ok(moment(&at, zone, false))
}

/// Midnight in `ctx.zone`, date-only; a missing year is the current one.
pub fn on_date(ctx: &Context, day: u8, month: u8, year: Option<i32>) -> Result<Moment, String> {
    let year = match year {
        Some(year) => year,
        None => i32::from(zoned(&now(ctx))?.year()),
    };
    ctx.mark_zone();
    let year = i16::try_from(year).map_err(|_| NOT_A_DATE.to_string())?;
    let date = Date::new(year, month as i8, day as i8).map_err(|_| NOT_A_DATE.to_string())?;
    let midnight = date
        .to_zoned_datetime(zones::tz(ctx.zone))
        .map_err(|_| OUT_OF_RANGE.to_string())?;
    Ok(moment(&midnight, ctx.zone, true))
}

/// `m` plus `amount` of the time unit `unit`:
/// - a whole number of days or weeks adds civil days, which keeps the wall time across a
///   DST change;
/// - a whole number of months or years adds calendar months, clamped to the month end;
/// - anything else adds exact seconds (month and year at their average length) and
///   clears `date_only`.
pub fn add(m: Moment, amount: f64, unit: Unit) -> Result<Moment, String> {
    let civil = match unit.name {
        "day" => Some((1.0, false)),
        "week" => Some((7.0, false)),
        "month" => Some((1.0, true)),
        "year" => Some((12.0, true)),
        _ => None,
    };
    if let Some((per, months)) = civil.filter(|_| amount.fract() == 0.0) {
        let n = whole(amount * per)?;
        let span = if months {
            Span::new().try_months(n)
        } else {
            Span::new().try_days(n)
        };
        let z = add_span(&zoned(&m)?, span)?;
        return Ok(moment(&z, m.zone, m.date_only));
    }
    let seconds = whole((amount * unit.factor).round())?;
    let unix = m
        .unix
        .checked_add(seconds)
        .filter(|&unix| Timestamp::from_second(unix).is_ok())
        .ok_or(OUT_OF_RANGE)?;
    Ok(Moment {
        unix,
        zone: m.zone,
        date_only: false,
    })
}

/// A whole `f64` as an `i64`, or out of range.
fn whole(n: f64) -> Result<i64, String> {
    // Far past jiff's ±10 000 years in any unit, and well inside `i64`.
    if n.is_finite() && n.abs() < 1e15 {
        Ok(n as i64)
    } else {
        Err(OUT_OF_RANGE.into())
    }
}

fn add_span(z: &ZonedDateTime, span: Result<Span, jiff::Error>) -> Result<ZonedDateTime, String> {
    span.and_then(|span| z.checked_add(span))
        .map_err(|_| OUT_OF_RANGE.to_string())
}

/// `a - b` as a duration value: whole civil days between two dates (across a DST change
/// too); otherwise exact, in days from one day up, else hours from one hour, else minutes.
pub fn between(a: Moment, b: Moment) -> Value {
    if a.date_only && b.date_only {
        return Value::with_unit(civil_days(&b, &a) as f64, units::lookup("day"));
    }
    let seconds = (a.unix - b.unix) as f64;
    let unit = match seconds.abs() {
        s if s >= 86_400.0 => "day",
        s if s >= 3_600.0 => "h",
        _ => "min",
    };
    let unit = units::lookup(unit).expect("`day`, `h` and `min` are units");
    Value::with_unit(seconds / unit.factor, Some(unit))
}

/// Calendar days from the date `a` shows to the date `b` shows.
pub(crate) fn civil_days(a: &Moment, b: &Moment) -> i64 {
    day_number(a).zip(day_number(b)).map_or(0, |(a, b)| b - a)
}

/// Days since 1970-01-01 of the date `m` shows in its zone.
fn day_number(m: &Moment) -> Option<i64> {
    let date = zoned(m).ok()?.date();
    let (y, mo, d) = (
        i64::from(date.year()),
        i64::from(date.month()),
        i64::from(date.day()),
    );
    // Howard Hinnant's days_from_civil.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `days until 31 dec`: `day` and `week` count civil days from today; the other units count
/// exact seconds from now. A date written without a year that is already past means next
/// year's (`roll`).
pub fn until(ctx: &Context, unit: Unit, target: Moment, roll: bool) -> Result<Value, String> {
    let today = day(ctx, 0)?;
    let year = units::lookup("year").expect("`year` is a unit");
    let target = if roll && civil_days(&today, &target) < 0 {
        add(target, 1.0, year)?
    } else {
        target
    };
    let number = match unit.name {
        "day" => civil_days(&today, &target) as f64,
        "week" => civil_days(&today, &target) as f64 / 7.0,
        _ => (target.unix - ctx.now) as f64 / unit.factor,
    };
    Ok(Value::with_unit(number, Some(unit)))
}

/// `Thu, 19 Nov 2026, 17:42 MSK` with the zone's abbreviation (which can be numeric, `+04`),
/// or `Thu, 19 Nov 2026` for a date-only moment. Written by hand: jiff's `strftime` costs
/// about 32 KB of wasm.
pub fn format(m: &Moment) -> String {
    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let Ok(z) = zoned(m) else {
        return OUT_OF_RANGE.into();
    };
    let weekday = WEEKDAYS[z.weekday().to_monday_zero_offset() as usize];
    let month = MONTHS[(z.month() - 1) as usize];
    let year = match z.year() {
        y if y < 0 => format!("-{:04}", -i32::from(y)),
        y => format!("{y:04}"),
    };
    let date = format!("{weekday}, {} {month} {year}", z.day());
    if m.date_only {
        return date;
    }
    let info = z.time_zone().to_offset_info(z.timestamp());
    format!(
        "{date}, {:02}:{:02} {}",
        z.hour(),
        z.minute(),
        info.abbreviation()
    )
}
