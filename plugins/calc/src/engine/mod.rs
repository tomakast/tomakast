//! The calculator's pure expression engine: no WASM, no SDK, no I/O.

pub mod color;
pub mod currency;
mod eval;
mod format;
mod lexer;
mod parser;
mod time;
mod units;
pub mod zones;

use std::cell::Cell;

use eval::Env;
use parser::{Expr, Stmt};

pub use color::{Color, Form};
pub use currency::{Rates, Table};
pub use format::format;
pub use time::Moment;
pub use units::{Dim, Unit};
pub use zones::ZoneId;

/// What a line evaluates to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// A number with an optional unit.
    Num { number: f64, unit: Option<Unit> },
    /// A whole number shown in base 2, 8 or 16 (`255 in hex`); a plain number in arithmetic.
    Based { number: f64, base: Base },
    /// A date, or a date and time, in a zone.
    Moment(Moment),
    /// A colour, with the form it is shown in.
    Color(Color),
}

/// What an evaluation may know about the world: the current time, the user's zone and the
/// stored exchange rates. The engine reads no clock and no storage; the caller passes them in.
pub struct Context<'a> {
    /// Unix seconds.
    pub now: i64,
    pub zone: ZoneId,
    pub rates: &'a Rates,
    used: Cell<Used>,
}

impl<'a> Context<'a> {
    pub fn new(now: i64, zone: ZoneId, rates: &'a Rates) -> Context<'a> {
        Context {
            now,
            zone,
            rates,
            used: Cell::new(Used::default()),
        }
    }

    /// What the evaluations so far relied on.
    pub fn used(&self) -> Used {
        self.used.get()
    }

    /// Notes that an evaluation read `zone`.
    fn mark_zone(&self) {
        self.mark(|used| used.zone = true);
    }

    /// Notes that an evaluation needed the feed that prices `code`; USD needs none.
    fn mark_rate(&self, code: &str) {
        if code == "USD" {
            return;
        }
        if currency::is_crypto(code) {
            self.mark(|used| used.crypto = true);
        } else {
            self.mark(|used| used.fiat = true);
        }
    }

    fn mark(&self, set: impl FnOnce(&mut Used)) {
        let mut used = self.used.get();
        set(&mut used);
        self.used.set(used);
    }
}

/// What an evaluation relied on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Used {
    /// It read the user's time zone (`today`, a date, a clock without a zone, `until` or
    /// `to date`; not `now`, the same instant in every zone).
    pub zone: bool,
    /// It needed the fiat rates, whether or not they were there.
    pub fiat: bool,
    /// It needed the crypto rates, whether or not they were there.
    pub crypto: bool,
}

/// A base a whole number can be shown in, 2 to 36.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base(pub u32);

impl Base {
    pub const BIN: Base = Base(2);
    pub const OCT: Base = Base(8);
    pub const HEX: Base = Base(16);
}

impl Value {
    pub fn plain(number: f64) -> Value {
        Value::Num { number, unit: None }
    }

    pub fn with_unit(number: f64, unit: Option<Unit>) -> Value {
        Value::Num { number, unit }
    }

    /// The number and unit an arithmetic operand stands for (`Based` is a plain number); a
    /// moment is none.
    pub fn quantity(&self) -> Option<(f64, Option<Unit>)> {
        match *self {
            Value::Num { number, unit } => Some((number, unit)),
            Value::Based { number, .. } => Some((number, None)),
            Value::Moment(_) | Value::Color(_) => None,
        }
    }
}

/// The outcome of one line of a sheet.
#[derive(Debug, Clone, PartialEq)]
pub enum LineResult {
    Empty,
    Value(Value),
    Assign(String, Value),
    Error(String),
}

/// The longest line that is evaluated, in bytes.
const MAX_LINE_BYTES: usize = 1024;

/// A scratchpad: lines evaluated top to bottom, sharing variables and `prev`.
pub struct Sheet;

impl Sheet {
    /// Evaluates `text` line by line. There is exactly one result per line (`split('\n')`),
    /// so a final newline yields a final `Empty`. `prev` is the last value or assignment;
    /// empty lines and errors leave it alone.
    pub fn eval(text: &str, ctx: &Context) -> Vec<LineResult> {
        let mut env = Env::default();
        text.split('\n')
            .map(|line| eval_line(&mut env, ctx, line.strip_suffix('\r').unwrap_or(line)))
            .collect()
    }
}

/// Evaluates one line of a sheet and keeps what it made in `env`: an assignment's variable,
/// and `prev`, the last value or assignment (empty lines and errors leave it alone). Whether
/// either is a percentage is kept too: a percentage is one wherever it is kept.
fn eval_line(env: &mut Env, ctx: &Context, line: &str) -> LineResult {
    if line.len() > MAX_LINE_BYTES {
        return LineResult::Error("line too long".into());
    }
    // A `#` is a colour only where the whole line then works; otherwise it starts a
    // comment, as it always did. When both readings fail, the colour's error is shown.
    match lexer::lex_colors(line) {
        Ok(toks) if has_color(&toks) => {
            // A colour that does not even parse (`issue #1234`) was meant as a comment.
            let parsed = parser::parse(toks.clone()).is_ok();
            match eval_toks(env, ctx, Ok(toks)) {
                LineResult::Error(colored) => match eval_toks(env, ctx, lexer::lex(line)) {
                    LineResult::Error(plain) => {
                        LineResult::Error(if parsed { colored } else { plain })
                    }
                    result => result,
                },
                result => result,
            }
        }
        _ => eval_toks(env, ctx, lexer::lex(line)),
    }
}

/// Whether the tokens hold a colour written with `#`.
fn has_color(toks: &[lexer::Tok]) -> bool {
    toks.iter().any(|t| matches!(t, lexer::Tok::Hex(_)))
}

/// [`eval_line`] for one reading of the line; `env` changes only when it succeeds.
fn eval_toks(env: &mut Env, ctx: &Context, toks: Result<Vec<lexer::Tok>, String>) -> LineResult {
    let toks = match toks {
        Ok(toks) if toks.is_empty() => return LineResult::Empty,
        Ok(toks) => toks,
        Err(e) => return LineResult::Error(e),
    };
    let (name, e) = match parser::parse(toks) {
        Ok(Stmt::Expr(e)) => (None, e),
        Ok(Stmt::Assign(name, e)) => (Some(name), e),
        Err(e) => return LineResult::Error(e),
    };
    let v = match eval::eval(env, ctx, &e) {
        Ok(v) => v,
        Err(e) => return LineResult::Error(e),
    };
    let percent = eval::is_percent(env, &e);
    env.prev = Some(v);
    env.prev_percent = percent;
    let Some(name) = name else {
        return LineResult::Value(v);
    };
    if percent {
        env.percents.insert(name.clone());
    } else {
        env.percents.remove(&name);
    }
    env.vars.insert(name.clone(), v);
    LineResult::Assign(name, v)
}

/// Evaluates a search query, or returns `None` unless it is clearly math: it parses
/// completely, evaluates without error, and contains an operator, a function, a percentage,
/// a unit conversion, `until`, the constant `pi` or a time word (`now`, `today`). A number
/// alone (`42`), a date (`2026-11-19`, `31 dec`), a clock time (`5pm`), a bare unit value
/// (`10 km`), anything with a `#` and anything that errors give `None`; search never shows
/// an error row. Variables and `prev` do not exist here, so `now` is always the moment.
pub fn eval_query(query: &str, ctx: &Context) -> Option<Value> {
    if query.len() > MAX_LINE_BYTES {
        return None;
    }
    let query = query.trim();
    if query.is_empty() || looks_like_date(query) {
        return None;
    }
    // A `#` that is not a colour starts a comment, and a query with a comment is not math.
    let toks = lexer::lex_colors(query).ok()?;
    let colors = toks
        .iter()
        .filter(|t| matches!(t, lexer::Tok::Hex(_)))
        .count();
    if query.matches('#').count() != colors {
        return None;
    }
    let Stmt::Expr(expr) = parser::parse(toks).ok()? else {
        return None;
    };
    if !is_mathy(&expr) {
        return None;
    }
    eval::eval(&Env::default(), ctx, &expr).ok()
}

/// Whether the expression is more than a number, a unit value, a date, a clock time or a
/// lone word. A literal in another base is a conversion (`0xff` is 255). A bare `e` is not
/// math (it is a letter people type); `pi` alone is, and so are the time words.
fn is_mathy(e: &Expr) -> bool {
    match e {
        // A colour written in hex is a conversion, as `0xff` is.
        Expr::Lit(Value::Color(_)) => true,
        Expr::Lit(_) | Expr::Clock { .. } | Expr::Date { .. } => false,
        Expr::Radix(_) | Expr::Until(..) | Expr::ColorCall { .. } => true,
        Expr::Name(name) => name == "pi" || eval::time_word(name).is_some(),
        Expr::Neg(inner) => is_mathy(inner),
        Expr::Bin(..) | Expr::Percent(_) | Expr::Of(..) | Expr::Call(..) | Expr::Convert(..) => {
            true
        }
    }
}

/// A date or a phone number, not a subtraction or a division (Ruling 6d): only these shapes,
/// with no spaces, give no answer:
/// - a phone number `^\d{3}-\d{4}$` (`555-1234`) or `^\d{3}-\d{3}-\d{4}$` (`555-123-4567`);
/// - a month `^\d{4}-\d{2}$` (`2026-11`);
/// - a date `^\d{4}[-/]\d{1,2}[-/]\d{1,2}$` (`2026-11-19`, `2026/1/5`);
/// - a date `^\d{1,2}/\d{1,2}/\d{4}$` (`11/19/2026`).
///
/// `1000-5`, `1500-300`, `100-200`, `5/1000`, `3/4` and `12/25` are still math.
fn looks_like_date(q: &str) -> bool {
    let digits = |g: &str, min: usize, max: usize| {
        (min..=max).contains(&g.len()) && g.bytes().all(|b| b.is_ascii_digit())
    };
    let dashes: Vec<&str> = q.split('-').collect();
    let slashes: Vec<&str> = q.split('/').collect();
    let groups: Vec<&str> = q.split(['-', '/']).collect();
    let phone = match dashes[..] {
        [a, b] => digits(a, 3, 3) && digits(b, 4, 4),
        [a, b, c] => digits(a, 3, 3) && digits(b, 3, 3) && digits(c, 4, 4),
        _ => false,
    };
    let month = matches!(dashes[..], [y, m] if digits(y, 4, 4) && digits(m, 2, 2));
    let year_first = matches!(groups[..], [y, m, d]
        if digits(y, 4, 4) && digits(m, 1, 2) && digits(d, 1, 2));
    let year_last = matches!(slashes[..], [m, d, y]
        if digits(m, 1, 2) && digits(d, 1, 2) && digits(y, 4, 4));
    phone || month || year_first || year_last
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::time::Instant;

    /// Renders one line's result the way a test table reads it.
    fn show(r: &LineResult) -> String {
        match r {
            LineResult::Empty => "empty".into(),
            LineResult::Value(v) => format(v),
            LineResult::Assign(n, v) => format!("{n} = {}", format(v)),
            LineResult::Error(e) => format!("error: {e}"),
        }
    }

    /// Both tables fetched a minute before [`ctx`]'s `now`.
    fn rates() -> &'static Rates {
        static RATES: OnceLock<Rates> = OnceLock::new();
        RATES.get_or_init(|| {
            let table = |pairs: &[(&'static str, f64)]| Table {
                fetched: 1_795_099_320 - 60,
                values: pairs.iter().copied().collect(),
            };
            Rates {
                fiat: Some(table(&[
                    ("USD", 1.0),
                    ("EUR", 0.92),
                    ("RUB", 93.125),
                    ("GBP", 0.79),
                    ("JPY", 150.0),
                ])),
                crypto: Some(table(&[("BTC", 67000.0), ("ETH", 3500.0)])),
            }
        })
    }

    /// 2026-11-19T14:42:00Z, in Moscow, with exchange rates.
    fn ctx() -> Context<'static> {
        Context::new(
            1_795_099_320,
            zones::find("Europe/Moscow").unwrap(),
            rates(),
        )
    }

    /// [`ctx`] without any rates.
    fn bare() -> Context<'static> {
        static NONE: OnceLock<Rates> = OnceLock::new();
        Context::new(
            1_795_099_320,
            zones::find("Europe/Moscow").unwrap(),
            NONE.get_or_init(Rates::default),
        )
    }

    /// Evaluates `text` as a sheet and renders every line.
    fn run(text: &str) -> Vec<String> {
        Sheet::eval(text, &ctx()).iter().map(show).collect()
    }

    /// [`run`] at the same instant in another zone.
    fn run_in(zone: &str, text: &str) -> Vec<String> {
        run_at(1_795_099_320, zone, text)
    }

    /// [`run`] at another instant and zone.
    fn run_at(now: i64, zone: &str, text: &str) -> Vec<String> {
        let ctx = Context::new(now, zones::find(zone).unwrap(), rates());
        Sheet::eval(text, &ctx).iter().map(show).collect()
    }

    #[test]
    fn dates_in_another_zone_stay_dates() {
        table(&[
            // Today, tomorrow and yesterday are those of the target zone.
            ("today in Tokyo", "Thu, 19 Nov 2026"),
            ("tomorrow in New York", "Fri, 20 Nov 2026"),
            ("yesterday in Tokyo", "Wed, 18 Nov 2026"),
            // A date keeps its civil date.
            ("31 dec in Tokyo", "Thu, 31 Dec 2026"),
            ("31 dec in New York", "Thu, 31 Dec 2026"),
            ("31 dec in Tokyo to unix", "1798642800"),
        ]);
        assert_eq!(
            run("x = today\nx in Tokyo"),
            ["x = Thu, 19 Nov 2026", "Thu, 19 Nov 2026"]
        );
        // 23:00 in Moscow is already tomorrow in Tokyo.
        let late = 1_795_118_400;
        assert_eq!(
            run_at(late, "Europe/Moscow", "today\ntoday in Tokyo"),
            ["Thu, 19 Nov 2026", "Fri, 20 Nov 2026"]
        );
        assert_eq!(
            run_at(late, "Europe/Moscow", "now in Tokyo"),
            ["Fri, 20 Nov 2026, 05:00 JST"]
        );
    }

    /// Evaluates a single line.
    fn one(line: &str) -> String {
        run(line).remove(0)
    }

    fn table(cases: &[(&str, &str)]) {
        for (input, want) in cases {
            assert_eq!(one(input), *want, "input: {input:?}");
        }
    }

    #[test]
    fn arithmetic_and_precedence() {
        table(&[
            ("1 + 2", "3"),
            ("2 + 3 * 4", "14"),
            ("(2 + 3) * 4", "20"),
            ("10 / 4", "2.5"),
            ("10 - 2 - 3", "5"),
            ("100 / 10 / 5", "2"),
            ("-3 + 5", "2"),
            ("-2 ^ 2", "-4"),
            ("2 * -3", "-6"),
            ("2 ^ -1", "0.5"),
            ("7 mod 3", "1"),
            ("-7 mod 3", "-1"),
            ("2 + 7 mod 4 * 2", "8"),
            ("1_000 + 1", "1001"),
            ("1e3", "1000"),
            ("2.5e-1 * 4", "1"),
            (".5 + .5", "1"),
            ("0.1 + 0.2", "0.3"),
            ("pi", "3.141592654"),
            ("e", "2.718281828"),
            ("sqrt(16) + abs(-2)", "6"),
            ("round(2.5)", "3"),
            ("floor(-1.5)", "-2"),
            ("ceil(1.2)", "2"),
            ("ln(e)", "1"),
            ("log(1000)", "3"),
            ("sin(0) + cos(0)", "1"),
            ("tan(0)", "0"),
            ("sqrt(2) * sqrt(2)", "2"),
        ]);
    }

    #[test]
    fn power_is_right_associative() {
        table(&[
            ("2 ^ 3 ^ 2", "512"),
            ("(2 ^ 3) ^ 2", "64"),
            ("2 ^ 2 ^ 3", "256"),
            ("2 * 3 ^ 2", "18"),
        ]);
    }

    #[test]
    fn variables_and_prev() {
        assert_eq!(run("x = 5\nx * 2"), ["x = 5", "10"]);
        assert_eq!(run("a = 2\nb = a + 3\na * b"), ["a = 2", "b = 5", "10"]);
        assert_eq!(run("x = 1\nx = x + 1\nx"), ["x = 1", "x = 2", "2"]);
        assert_eq!(run("3 * 4\nprev + 1\nans * 2"), ["12", "13", "26"]);
        // Empty lines, comments and errors do not change `prev`.
        assert_eq!(
            run("2 + 2\n\n# note\n1 / 0\nprev"),
            ["4", "empty", "empty", "error: division by zero", "4"]
        );
        // An assignment is a result, so `prev` sees it.
        assert_eq!(run("x = 7\nprev * 2"), ["x = 7", "14"]);
        // A variable keeps its unit.
        assert_eq!(run("d = 5 km\nd + 500 m"), ["d = 5 km", "5.5 km"]);
        table(&[
            ("prev", "error: no previous result"),
            ("nope + 1", "error: unknown name 'nope'"),
            (
                "km = 5",
                "error: 'km' is a unit name and cannot be a variable",
            ),
            (
                "sqrt = 5",
                "error: 'sqrt' is a function name and cannot be a variable",
            ),
            ("pi = 3", "error: 'pi' is reserved and cannot be a variable"),
            (
                "prev = 3",
                "error: 'prev' is reserved and cannot be a variable",
            ),
            ("x =", "error: expected a value at the end of the line"),
        ]);
    }

    #[test]
    fn percentages() {
        table(&[
            ("15% of 80", "12"),
            ("80 + 15%", "92"),
            ("80 - 15%", "68"),
            ("15%", "0.15"),
            ("200 * 10%", "20"),
            ("50 + 10% + 10%", "60.5"),
            ("15% + 5%", "0.2"),
            ("80 + -10%", "72"),
            ("80 - -10%", "88"),
            ("80 - 10%", "72"),
            ("-10% + 5%", "-0.05"),
            ("-10% of 50", "-5"),
            ("10% of 5 km", "0.5 km"),
            ("80 km + 25%", "100 km"),
            ("5 km%", "error: a percentage needs a plain number"),
            ("5% of", "error: expected a value at the end of the line"),
            ("5 of 3", "error: 'of' needs a percentage on its left"),
        ]);
    }

    #[test]
    fn unit_conversion_and_arithmetic() {
        table(&[
            ("1 mi in km", "1.609344 km"),
            ("3 ft + 20 cm", "3.656167979 ft"),
            ("5 km in mi", "3.106855961 mi"),
            ("5 km to mi", "3.106855961 mi"),
            ("5 km", "5 km"),
            ("1 km in m", "1000 m"),
            ("2 h in min", "120 min"),
            ("1 week in day", "7 days"),
            ("500 g to kg", "0.5 kg"),
            ("1 lb in oz", "16 oz"),
            ("12 in in cm", "30.48 cm"),
            ("12 in cm", "30.48 cm"),
            ("12 in", "12 in"),
            ("2 * 5 km", "10 km"),
            ("5 km * 2", "10 km"),
            ("10 km / 4", "2.5 km"),
            ("10 km / 500 m", "20"),
            ("5 km - 500 m", "4.5 km"),
            ("-5 km", "-5 km"),
            ("abs(-5 km)", "5 km"),
            ("5 km + 2 mi in m", "8218.688 m"),
            ("5 km in mi in km", "5 km"),
        ]);
        assert_eq!(run("x = 3 km\nx in m"), ["x = 3 km", "3000 m"]);
        assert_eq!(run("3 km\nprev in m"), ["3 km", "3000 m"]);
    }

    #[test]
    fn more_units() {
        table(&[
            ("1 ha in m2", "10000 m2"),
            ("1 acre in m2", "4046.856422 m2"),
            ("1 mi2 in km2", "2.58998811 km2"),
            ("1 m² in cm2", "10000 cm2"),
            ("1 gal in l", "3.785411784 l"),
            ("1 cup in ml", "236.5882365 ml"),
            ("3 tsp in tbsp", "1 tbsp"),
            ("2 pt in qt", "1 qt"),
            ("1 m3 in l", "1000 l"),
            ("8 floz in cup", "1 cup"),
            ("100 km/h in mph", "62.13711922 mph"),
            ("10 m/s in km/h", "36 km/h"),
            ("1 kn in km/h", "1.852 km/h"),
            ("10 km / h", "10 km/h"),
            ("10 km / 500 m", "20"),
            ("1 GB in MB", "1000 MB"),
            ("1 GiB in MiB", "1024 MiB"),
            ("1 GB in GiB", "0.9313225746 GiB"),
            ("1 B in bit", "8 bit"),
            ("1 TiB in GiB", "1024 GiB"),
            ("180 deg in rad", "3.141592654 rad"),
            ("1 rad in deg", "57.29577951 deg"),
            ("sin(90 deg)", "1"),
            ("cos(180°)", "-1"),
            ("sin(5 km)", "error: sin needs a plain number"),
            ("1 year in day", "365.2425 days"),
            ("1 month in day", "30.436875 days"),
            ("1 week in day", "7 days"),
            ("1 day in h", "24 h"),
            ("1 week in days", "7 days"),
            ("5 km in GB", "error: cannot convert km to GB"),
        ]);
    }

    #[test]
    fn units_of_one_dimension_make_another() {
        table(&[
            ("10 km / 2 h", "5 km/h"),
            ("100 m / 10 s", "10 m/s"),
            ("1 mi / 1 h", "1 mph"),
            ("1 km / 1 min", "16.66666667 m/s"),
            ("90 km/h * 2 h", "180 km"),
            ("2 h * 90 km/h", "180 km"),
            ("10 m/s * 1 min", "600 m"),
            ("60 mph * 30 min in km", "48.28032 km"),
            ("100 km / 50 km/h", "2 h"),
            ("100 m / 10 m/s", "10 s"),
            ("2 m * 3 m", "6 m2"),
            ("1 km * 1 km", "1 km2"),
            ("1 cm * 1 m", "0.01 m2"),
            ("2 m2 * 3 m", "6 m3"),
            ("1 cm2 * 1 cm", "1 ml"),
            ("2 m * 3 m * 4 m in l", "24000 l"),
            ("6 m2 / 2 m", "3 m"),
            // A litre over a square metre is a millimetre (of rain).
            ("1000 l / 2 m2", "500 mm"),
            ("1000 l / 10 m", "0.1 m2"),
            ("10 km / 2 h / 0", "error: division by zero"),
            ("5 km * 3 kg", "error: cannot multiply km and kg"),
            ("2 m2 * 3 m2", "error: cannot multiply m2 and m2"),
            ("2 h * 3 h", "error: cannot multiply h and h"),
            ("1 GB / 1 s", "1 GB/s"),
            ("20 C * 2 m", "error: cannot multiply °C and m"),
            ("10 km/h / 2 h", "error: cannot divide km/h by h"),
            ("100 usd / 2 h", "error: cannot divide USD by h"),
            ("5 / 2 h", "error: cannot divide a number by h"),
        ]);
    }

    #[test]
    fn temperature_is_affine() {
        table(&[
            ("0 C in F", "32 °F"),
            ("100 °C to °F", "212 °F"),
            ("100 F to C", "37.77777778 °C"),
            ("-40 C in F", "-40 °F"),
            ("300 K in C", "26.85 °C"),
            ("0 K in F", "-459.67 °F"),
            ("20 C + 5 C", "25 °C"),
            ("20 C + 9 F", "25 °C"),
            ("20 C - 5 K", "15 °C"),
            ("100 f to c", "37.77777778 °C"),
            ("20 celsius in fahrenheit", "68 °F"),
            ("5 C in km", "error: cannot convert °C to km"),
        ]);
    }

    #[test]
    fn aliases_and_plurals() {
        table(&[
            ("5 kilometers in miles", "3.106855961 mi"),
            ("2 hours in minutes", "120 min"),
            ("3 feet in inches", "36 in"),
            ("1 kilogram in pounds", "2.204622622 lb"),
            ("90 seconds in min", "1.5 min"),
            ("2 litres in ml", "2000 ml"),
            ("1 gigabyte in megabytes", "1000 MB"),
            ("1 kph in m/s", "0.2777777778 m/s"),
            ("100 km/h", "100 km/h"),
            ("10 km / 500 m", "20"),
            ("1 day", "1 day"),
            ("2 day", "2 days"),
            ("0.5 week", "0.5 weeks"),
        ]);
    }

    #[test]
    fn number_bases() {
        table(&[
            ("0xff", "255"),
            ("0b1010 + 1", "11"),
            ("0o17", "15"),
            ("255 in hex", "0xff"),
            ("255 in bin", "0b11111111"),
            ("255 in oct", "0o377"),
            ("0xff in decimal", "255"),
            ("-255 in hex", "-0xff"),
            ("12 in hex", "0xc"),
            ("0xff in hex + 1", "256"),
            ("12 in", "12 in"),
            ("12 in cm", "30.48 cm"),
            ("2.5 in hex", "error: hex needs a whole number"),
            ("5 km in hex", "error: hex needs a plain number"),
            ("2 ^ 60 in hex", "error: too large for hex"),
            ("0x", "error: unexpected 'x'"),
        ]);
    }

    #[test]
    fn new_unit_names_stay_free_for_variables() {
        assert_eq!(
            run("hours = 40\nrate = 50\nhours * rate"),
            ["hours = 40", "rate = 50", "2000"]
        );
        assert_eq!(run("year = 2026\nyear + 1"), ["year = 2026", "2027"]);
        assert_eq!(run("c = 3\nc * 2\n10 c in f"), ["c = 3", "6", "50 °F"]);
        table(&[
            ("hours", "error: 'hours' is a unit; put a number before it"),
            (
                "day = 5",
                "error: 'day' is a unit name and cannot be a variable",
            ),
        ]);
    }

    #[test]
    fn dimension_mismatch_is_an_error() {
        table(&[
            ("5 km + 3 kg", "error: cannot add km and kg"),
            ("5 km - 3 kg", "error: cannot subtract km and kg"),
            ("5 km + 3", "error: cannot add km and a number"),
            ("3 + 5 km", "error: cannot add a number and km"),
            ("5 km in kg", "error: cannot convert km to kg"),
            ("5 in 3 kg", "error: unexpected '3'"),
            ("5 in 7", "error: unexpected '7'"),
            ("5 to km", "error: cannot convert a plain number to km"),
            ("5 km * 3 kg", "error: cannot multiply km and kg"),
            ("5 km / 3 kg", "error: cannot divide km by kg"),
            ("5 / 3 kg", "error: cannot divide a number by kg"),
            ("5 km ^ 2", "error: cannot raise a unit value to a power"),
            ("2 ^ 5 km", "error: cannot raise to a unit value"),
            ("sqrt(4 km)", "error: sqrt needs a plain number"),
            ("5 km mod 2", "error: mod needs plain numbers"),
            ("5 apples", "error: unexpected 'apples'"),
            ("km", "error: 'km' is a unit; put a number before it"),
            ("5 km in", "error: expected a unit after 'in'"),
            ("5 km in 3", "error: expected a unit after 'in'"),
            ("5 km in banana", "error: unknown unit 'banana'"),
        ]);
    }

    #[test]
    fn comments_and_empty_lines() {
        assert_eq!(run(""), ["empty"]);
        assert_eq!(run("   "), ["empty"]);
        assert_eq!(run("# just a note"), ["empty"]);
        assert_eq!(run("  # indented note"), ["empty"]);
        assert_eq!(run("1 + 1 # two"), ["2"]);
        assert_eq!(run("1\n\n2"), ["1", "empty", "2"]);
        // One result per line, including the empty last line after a final newline.
        assert_eq!(run("1\n"), ["1", "empty"]);
        assert_eq!(run("1\r\n2\r\n"), ["1", "2", "empty"]);
        assert_eq!(run("x = 4 # four\nx"), ["x = 4", "4"]);
    }

    #[test]
    fn syntax_errors_are_messages_not_panics() {
        table(&[
            ("1 +", "error: expected a value at the end of the line"),
            ("(1 + 2", "error: expected ')'"),
            ("1 + 2)", "error: unexpected ')'"),
            ("1 2", "error: unexpected '2'"),
            ("2pi", "error: unexpected 'pi'"),
            ("2 pi", "error: unexpected 'pi'"),
            ("sqrt 4", "error: 'sqrt' needs parentheses"),
            ("sqrt()", "error: unexpected ')'"),
            ("1.2.3", "error: unexpected '.3'"),
            ("3 @ 4", "error: unexpected character '@'"),
            ("1 = 2", "error: unexpected '='"),
            ("sqrt(-1)", "error: not a number"),
            ("ln(0)", "error: not a number"),
            ("0 ^ -1", "error: division by zero"),
            ("(-8) ^ 0.5", "error: not a number"),
            ("10 mod 0", "error: division by zero"),
        ]);
    }

    #[test]
    fn formatting() {
        table(&[
            ("1 / 3", "0.3333333333"),
            ("2 / 3", "0.6666666667"),
            ("1000000 * 1000000", "1000000000000"),
            ("123456789012345", "123456789012345"),
            ("1e15", "1000000000000000"),
            ("1e16", "1e16"),
            ("1234567890123456", "1.23456789e15"),
            ("123456.789012345", "123456.789"),
            ("12345678901.4", "12345678901"),
            ("98765432109.87", "98765432110"),
            ("12345.6789012345", "12345.6789"),
            ("0.000001", "0.000001"),
            ("0.0000001234", "1.234e-7"),
            ("9.99999999999", "10"),
            ("0 * -1", "0"),
            ("-0.5", "-0.5"),
            ("1e300 * 1e300", "error: too large"),
            ("5 km in mi", "3.106855961 mi"),
        ]);
    }

    #[test]
    fn limits() {
        table(&[
            ("2 ^ 1024", "error: too large"),
            ("10 ^ 308", "1e308"),
            ("10 ^ 309", "error: too large"),
            ("1 / 0", "error: division by zero"),
            ("0 / 0", "error: division by zero"),
            ("5 km / 0", "error: division by zero"),
            ("1e999", "error: too large"),
        ]);
        let nested = |n: usize| format!("{}1{}", "(".repeat(n), ")".repeat(n));
        assert_eq!(one(&nested(60)), "1");
        assert_eq!(one(&nested(64)), "1");
        assert_eq!(one(&nested(65)), "error: too deeply nested");
        assert_eq!(
            one(&format!("{}1", "-".repeat(100))),
            "error: too deeply nested"
        );
        let long = format!("1{}", "+1".repeat(600));
        assert_eq!(one(&long), "error: line too long");
        // The expression tree is at most 64 deep, whatever its shape.
        assert_eq!(one(&format!("1{}", "+1".repeat(63))), "64");
        assert_eq!(
            one(&format!("1{}", "+1".repeat(64))),
            "error: too deeply nested"
        );
        assert_eq!(
            one(&format!("1{}", "+1".repeat(500))),
            "error: too deeply nested"
        );
        assert_eq!(one(&format!("1{}", "%".repeat(63))), "1e-126");
        assert_eq!(
            one(&format!("1{}", "%".repeat(64))),
            "error: too deeply nested"
        );
        assert_eq!(
            one(&format!("1{}", "%".repeat(1000))),
            "error: too deeply nested"
        );
        assert_eq!(one(&format!("{}1", "-".repeat(63))), "-1");
        assert_eq!(
            one(&format!("{}1", "-".repeat(64))),
            "error: too deeply nested"
        );
        assert_eq!(
            one(&format!("{}1", "-".repeat(1000))),
            "error: too deeply nested"
        );
        assert_eq!(one(&format!("5 km{}", " in m".repeat(62))), "5000 m");
        assert_eq!(
            one(&format!("5 km{}", " in m".repeat(200))),
            "error: too deeply nested"
        );
        assert_eq!(
            one(&format!("1{}", " mod 7".repeat(160))),
            "error: too deeply nested"
        );
        // The limit counts bytes, and a long line does not poison its neighbours.
        assert_eq!(
            run(&format!("{}\n2", "9".repeat(1025))),
            ["error: line too long", "2"]
        );
    }

    #[test]
    fn pathological_inputs_do_not_panic() {
        // A guest wasm stack is small: run on 256 KiB so any recursion past the depth limit
        // overflows here instead of in the plugin. Unoptimised debug frames are several
        // times larger (a depth-64 tree needs 384-512 KiB), so debug gets 1 MiB; unbounded
        // recursion still overflows either.
        let stack = if cfg!(debug_assertions) {
            1024 * 1024
        } else {
            256 * 1024
        };
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(pathological_inputs)
            .unwrap()
            .join()
            .unwrap();
    }

    fn pathological_inputs() {
        let inputs = [
            "9^9^9^9".to_string(),
            "(".repeat(10_000),
            ")".repeat(10_000),
            "-".repeat(10_000),
            format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000)),
            "9".repeat(1024),
            "9".repeat(1025),
            "1/0".to_string(),
            "0/0".to_string(),
            "+-*/^%=".to_string(),
            "* / ^ + - mod of in to".to_string(),
            "= = = =".to_string(),
            "% % %".to_string(),
            "1e".to_string(),
            "1e+".to_string(),
            "1e99999999999999999999".to_string(),
            "_".to_string(),
            "1__0".to_string(),
            "\u{0}\u{7f}é日本語🙂".to_string(),
            "9^9^9^9 + 1/0".to_string(),
            "sqrt(sqrt(sqrt(".to_string(),
            format!("1{}", "%".repeat(1000)),
            format!("1{}", "+1".repeat(511)),
            format!("{}1", "-".repeat(1000)),
            format!("5 km{}", " in m".repeat(200)),
            format!("1{}", " mod 7".repeat(170)),
            format!("2{}", "^2".repeat(500)),
            format!("{}1{}", "sqrt(".repeat(63), ")".repeat(63)),
            format!("0x{}", "f".repeat(1000)),
            "°".repeat(500),
            "x/".repeat(511) + "x",
            "km/".repeat(340) + "h",
            format!("now{}", " in Tokyo".repeat(200)),
            format!("{}31 dec", "days until ".repeat(100)),
            "New York ".repeat(100),
            format!("now{}", " + 1 day".repeat(500)),
            "15:00 ".repeat(200),
            "31 dec ".repeat(100),
        ];
        let ctx = ctx();
        for input in &inputs {
            let started = Instant::now();
            let results = Sheet::eval(input, &ctx);
            assert_eq!(results.len(), 1, "input: {:.40?}", input);
            assert!(started.elapsed().as_millis() < 100, "slow: {:.40?}", input);
            // None of these is a valid value: each is an error, an empty line or a number
            // that fits; the point is that they return at all.
            let _ = eval_query(input, &ctx);
        }
        assert_eq!(one("9^9^9^9"), "error: too large");
        assert_eq!(one(&format!("0x{}", "f".repeat(1000))), "error: too large");
        assert_eq!(one("1/0"), "error: division by zero");
        assert_eq!(one("0/0"), "error: division by zero");
        assert_eq!(one("+-*/^%="), "error: unexpected '+'");
        assert_eq!(one(&"9".repeat(1024)), "error: too large");
        // 10 000 bytes is over the line limit; 1000 fits the line and trips the depth limit.
        assert_eq!(one(&"(".repeat(10_000)), "error: line too long");
        assert_eq!(one(&"(".repeat(1000)), "error: too deeply nested");
        assert_eq!(one(&"-(".repeat(500)), "error: too deeply nested");
        assert_eq!(one(&"sqrt(".repeat(200)), "error: too deeply nested");
        assert_eq!(one(&"2^".repeat(500)), "error: too deeply nested");
        // Those two inputs above are over 1 KiB (`line too long`); the same shapes that fit
        // the line trip the depth limit.
        assert_eq!(
            one(&format!("now{}", " in Tokyo".repeat(200))),
            "error: line too long"
        );
        assert_eq!(
            one(&format!("now{}", " in Tokyo".repeat(100))),
            "error: too deeply nested"
        );
        let until = format!("{}31 dec", "days until ".repeat(100));
        assert_eq!(until.len(), 1106);
        assert_eq!(one(&until), "error: line too long");
        assert_eq!(
            one(&format!("{}31 dec", "days until ".repeat(90))),
            "error: too deeply nested"
        );
    }

    #[test]
    fn dates_and_durations() {
        table(&[
            ("now", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("time", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("today", "Thu, 19 Nov 2026"),
            ("tomorrow", "Fri, 20 Nov 2026"),
            ("yesterday", "Wed, 18 Nov 2026"),
            ("Now", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("TODAY", "Thu, 19 Nov 2026"),
            ("Now + 1 h", "Thu, 19 Nov 2026, 18:42 MSK"),
            ("now + 2 weeks", "Thu, 3 Dec 2026, 17:42 MSK"),
            ("2 weeks + now", "Thu, 3 Dec 2026, 17:42 MSK"),
            ("today - 3 days", "Mon, 16 Nov 2026"),
            ("today + 1 month", "Sat, 19 Dec 2026"),
            ("today + 1 year", "Fri, 19 Nov 2027"),
            ("today + 1.5 days", "Fri, 20 Nov 2026, 12:00 MSK"),
            ("now + 90 min", "Thu, 19 Nov 2026, 19:12 MSK"),
            ("days until 31 dec", "42 days"),
            ("dec 31 - today", "42 days"),
            ("31 december - today", "42 days"),
            ("days until 5 jan", "47 days"),
            ("weeks until 31 dec", "6 weeks"),
            ("(dec 31 - today) in weeks", "6 weeks"),
            ("now - today", "17.7 h"),
            ("31 dec", "Thu, 31 Dec 2026"),
            ("dec 31 2027", "Fri, 31 Dec 2027"),
            ("now + now", "error: cannot add two dates"),
            ("now * 2", "error: cannot multiply a date"),
            ("today + 3", "error: cannot add a date and a number"),
            ("5 - today", "error: cannot subtract a date from a number"),
            ("31 feb", "error: not a date"),
            ("25:00", "error: not a time"),
            ("13pm", "error: not a time"),
            ("now + 1e300 years", "error: date out of range"),
            ("today in km", "error: cannot convert a date to km"),
        ]);
    }

    #[test]
    fn time_zones() {
        table(&[
            ("now in Tokyo", "Thu, 19 Nov 2026, 23:42 JST"),
            ("time in London", "Thu, 19 Nov 2026, 14:42 GMT"),
            ("now in New York", "Thu, 19 Nov 2026, 09:42 EST"),
            ("now in america/los_angeles", "Thu, 19 Nov 2026, 06:42 PST"),
            ("now to UTC", "Thu, 19 Nov 2026, 14:42 UTC"),
            ("5pm London in Moscow", "Thu, 19 Nov 2026, 20:00 MSK"),
            ("15:00 MSK in PST", "Thu, 19 Nov 2026, 04:00 PST"),
            ("5:30 pm in Tokyo", "Thu, 19 Nov 2026, 23:30 JST"),
            ("12am", "Thu, 19 Nov 2026, 00:00 MSK"),
            ("now in Dubai", "Thu, 19 Nov 2026, 18:42 +04"),
            ("now in São Paulo", "Thu, 19 Nov 2026, 11:42 -03"),
            ("now in Narnia", "error: unknown unit 'Narnia'"),
            ("now in pacific", "Thu, 19 Nov 2026, 06:42 PST"),
            ("now in central", "Thu, 19 Nov 2026, 08:42 CST"),
            ("now in mountain", "Thu, 19 Nov 2026, 07:42 MST"),
            ("now in singapore", "Thu, 19 Nov 2026, 22:42 +08"),
            ("now in east", "error: unknown unit 'east'"),
            ("now in north", "error: unknown unit 'north'"),
            ("now in general", "error: unknown unit 'general'"),
            ("5 km in Tokyo", "error: cannot convert km to Asia/Tokyo"),
        ]);
    }

    #[test]
    fn unix_time() {
        table(&[
            ("1700000000 to date", "Wed, 15 Nov 2023, 01:13 MSK"),
            ("now to unix", "1795099320"),
            ("today to unix", "1795035600"),
            ("1700000000 to date in UTC", "Tue, 14 Nov 2023, 22:13 UTC"),
            ("5 km to date", "error: cannot convert km to a date"),
            ("now to hex", "error: hex needs a plain number"),
        ]);
    }

    #[test]
    fn calendar_months_and_dst() {
        table(&[("31 jan 2026 + 1 month", "Sat, 28 Feb 2026")]);
        assert_eq!(
            run_in("Europe/London", "1774745940 to date"),
            ["Sun, 29 Mar 2026, 00:59 GMT"]
        );
        assert_eq!(
            run_in("Europe/London", "1774746000 to date"),
            ["Sun, 29 Mar 2026, 02:00 BST"]
        );
        assert_eq!(
            run_in("Europe/London", "1774699200 to date + 1 day"),
            ["Sun, 29 Mar 2026, 12:00 BST"]
        );
        assert_eq!(
            run_in("Europe/London", "1774699200 to date + 24 h"),
            ["Sun, 29 Mar 2026, 13:00 BST"]
        );
        assert_eq!(
            run_in("Europe/London", "29 mar 2026 - 28 mar 2026"),
            ["1 day"]
        );
    }

    #[test]
    fn date_edges_and_messages() {
        table(&[
            ("1e300 to date", "error: date out of range"),
            ("-1e20 to date", "error: date out of range"),
            // Another zone shows the instant, with its time.
            ("hours until 31 dec", "990.3 h"),
            // A date without a year that is past means next year's; with a year it does not.
            ("days until 18 nov", "364 days"),
            ("days until 18 nov 2026", "-1 days"),
            ("days until today", "0 days"),
            ("12pm", "Thu, 19 Nov 2026, 12:00 MSK"),
            ("12:30 am", "Thu, 19 Nov 2026, 00:30 MSK"),
            ("29 feb", "error: not a date"),
            ("29 feb 2028", "Tue, 29 Feb 2028"),
            ("0am", "error: not a time"),
            ("15:60", "error: not a time"),
            ("today - 3", "error: cannot add a date and a number"),
            ("today + 5 km", "error: cannot add a date and km"),
            ("2 h - now", "error: cannot subtract a date from h"),
            ("-today", "error: cannot negate a date"),
            ("abs(today)", "error: abs needs a plain number"),
            ("10% of today", "error: cannot multiply a date"),
            ("days until 5", "error: 'until' needs a date"),
            ("5 to unix", "error: cannot convert a plain number to unix"),
            (
                "12 in Tokyo",
                "error: cannot convert a plain number to Asia/Tokyo",
            ),
        ]);
        assert_eq!(
            run("x = today\nx + 1 day"),
            ["x = Thu, 19 Nov 2026", "Fri, 20 Nov 2026"]
        );
    }

    #[test]
    fn currencies() {
        table(&[
            ("100 usd in rub", "9 312.50 RUB"),
            ("100 USD to RUB", "9 312.50 RUB"),
            ("$100 to €", "92.00 EUR"),
            ("100$ in eur", "92.00 EUR"),
            ("50 eur + 20 usd in gbp", "58.73 GBP"),
            ("¥1000 in usd", "6.67 USD"),
            ("100 rub", "100.00 RUB"),
            ("0.5 btc in usd", "33 500.00 USD"),
            ("1 eth in btc", "0.052238806 BTC"),
            ("1 btc in rub", "6 239 375.00 RUB"),
            ("100 usd * 2", "200.00 USD"),
            ("100 usd / 4 usd", "25"),
            ("1 usd in jpy in usd", "1.00 USD"),
            ("0.001 usd", "0.001 USD"),
            ("-12.3 eur", "-12.30 EUR"),
            ("100 usd + 5 km", "error: cannot add USD and km"),
            ("100 usd in km", "error: cannot convert USD to km"),
            ("100 usd * 2 eur", "error: cannot multiply USD and EUR"),
            ("100 xyz in usd", "error: unexpected 'xyz'"),
            ("100 usd in kzt", "error: no rate for KZT"),
            ("3 $ 4", "error: unexpected '4'"),
        ]);
    }

    #[test]
    fn currency_names_and_physical_units_meet_in_order() {
        table(&[
            ("12 in usd", "error: cannot convert a plain number to USD"),
            ("1 cup in ml", "236.5882365 ml"),
            ("5 CUP in usd", "error: no rate for CUP"),
            ("usd", "error: 'usd' is a currency; put a number before it"),
        ]);
    }

    #[test]
    fn ton_in_lower_case_is_the_tonne() {
        // As `cup` beats the Cuban peso, lower-case `ton` is the metric tonne; `TON` is the coin.
        table(&[
            ("1 ton in kg", "1000 kg"),
            ("2 tons in kg", "2000 kg"),
            ("1 TON in usd", "error: no rate for TON"),
        ]);
        let rates = Rates {
            crypto: Some(Table {
                fetched: 1_795_099_320 - 60,
                values: [("TON", 5.5)].into_iter().collect(),
            }),
            ..Rates::default()
        };
        let ctx = Context::new(1_795_099_320, zones::utc(), &rates);
        let line = |text: &str| show(&Sheet::eval(text, &ctx)[0]);
        assert_eq!(line("1 TON in usd"), "5.50 USD");
        assert_eq!(line("2 TON to usd"), "11.00 USD");
        // A soft name: a variable of that name still works.
        assert_eq!(run("ton = 1\nton + 2"), ["ton = 1", "3"]);
    }

    #[test]
    fn currencies_without_rates() {
        let bare = bare();
        let line = |text: &str| show(&Sheet::eval(text, &bare)[0]);
        assert_eq!(line("100 usd in rub"), "error: no exchange rates yet");
        assert_eq!(line("100 usd"), "100.00 USD");
        assert_eq!(line("1 btc in usd"), "error: no exchange rates yet");
        assert_eq!(eval_query("100 usd in rub", &bare), None);
    }

    #[test]
    fn conversions_report_the_feeds_they_used() {
        let used = |ctx: Context, query: &str| {
            let _ = eval_query(query, &ctx);
            let used = ctx.used();
            (used.fiat, used.crypto)
        };
        assert_eq!(used(ctx(), "100 usd in rub"), (true, false));
        assert_eq!(used(ctx(), "1 eth in btc"), (false, true));
        assert_eq!(used(ctx(), "50 eur + 1 btc in usd"), (true, true));
        assert_eq!(used(ctx(), "2 + 2"), (false, false));
        assert_eq!(used(ctx(), "100 usd in usd"), (false, false));
        assert_eq!(used(bare(), "1 btc in usd"), (false, true));
    }

    #[test]
    fn currency_codes_stay_free_for_variables() {
        assert_eq!(
            run("try = 2\nall = 3\ntry * all"),
            ["try = 2", "all = 3", "6"]
        );
    }

    #[test]
    fn time_words_stay_free_for_variables() {
        assert_eq!(run("time = 5\ntime * 2"), ["time = 5", "10"]);
        assert_eq!(run("dec = 3\ndec + 1"), ["dec = 3", "4"]);
        table(&[(
            "until = 1",
            "error: 'until' is reserved and cannot be a variable",
        )]);
    }

    #[test]
    fn query_heuristic() {
        let answer = |q: &str| eval_query(q, &ctx()).map(|v| format(&v));
        // Clearly math.
        for (q, want) in [
            ("2+2*3", "8"),
            ("2 + 2 * 3", "8"),
            ("15% of 80", "12"),
            ("80 + 15%", "92"),
            ("5 km in mi", "3.106855961 mi"),
            ("5 km to mi", "3.106855961 mi"),
            ("12 in cm", "30.48 cm"),
            ("sqrt(2)", "1.414213562"),
            ("abs(-5)", "5"),
            ("pi", "3.141592654"),
            ("pi * 2", "6.283185307"),
            ("2 ^ 10", "1024"),
            ("1/2", "0.5"),
            ("3/4", "0.75"),
            ("5-3", "2"),
            ("12/25", "0.48"),
            ("1000/5", "200"),
            ("10-100", "-90"),
            ("10/100", "0.1"),
            ("100/20/3", "1.666666667"),
            // Ruling 6d: arithmetic that only resembles a date or a phone number.
            ("1000-5", "995"),
            ("1500-300", "1200"),
            ("100-200", "-100"),
            ("5/1000", "0.005"),
            ("12/2025", "0.005925925926"),
            ("1/2/26", "0.01923076923"),
            ("555-123", "432"),
            ("2026 - 11 - 19", "1996"),
            ("-3 + 5", "2"),
            ("10 mod 3", "1"),
            ("3 ft + 20 cm", "3.656167979 ft"),
            ("100 F to C", "37.77777778 °C"),
            ("100 km/h in mph", "62.13711922 mph"),
            ("1 GiB in MiB", "1024 MiB"),
            ("255 in hex", "0xff"),
            ("0xff", "255"),
            ("0b1010", "10"),
            ("now in Tokyo", "Thu, 19 Nov 2026, 23:42 JST"),
            ("time in London", "Thu, 19 Nov 2026, 14:42 GMT"),
            ("days until 31 dec", "42 days"),
            ("dec 31 - today", "42 days"),
            ("now + 2 weeks", "Thu, 3 Dec 2026, 17:42 MSK"),
            ("5pm London in Moscow", "Thu, 19 Nov 2026, 20:00 MSK"),
            ("1700000000 to date", "Wed, 15 Nov 2023, 01:13 MSK"),
            ("now to unix", "1795099320"),
            ("now", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("time", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("today", "Thu, 19 Nov 2026"),
            ("tomorrow", "Fri, 20 Nov 2026"),
            ("yesterday", "Wed, 18 Nov 2026"),
            ("Now", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("NOW", "Thu, 19 Nov 2026, 17:42 MSK"),
            ("Today", "Thu, 19 Nov 2026"),
            ("100 usd in rub", "9 312.50 RUB"),
            ("$100 to €", "92.00 EUR"),
            ("50 eur + 20 usd in gbp", "58.73 GBP"),
            ("0.5 btc in usd", "33 500.00 USD"),
            ("1 eth in btc", "0.052238806 BTC"),
            ("10 km / 2 h", "5 km/h"),
            ("90 km/h * 2 h in km", "180 km"),
            ("2 m * 3 m", "6 m2"),
        ] {
            assert_eq!(answer(q).as_deref(), Some(want), "query: {q:?}");
        }
        // Only looks like math: no answer, never an error row.
        for q in [
            "42",
            "-42",
            "3.14",
            "1_000",
            "2024 report",
            "10 km run",
            "1/2 price",
            "2026-11-19",
            "2026/11/19",
            "2026-1-5",
            "2026/1/5",
            "2026-11/19",
            "11/19/2026",
            "19.11.2026",
            "2026-11",
            "555-1234",
            "555-123-4567",
            "hello",
            "e",
            "x",
            "km",
            "10 km",
            "5 in",
            "",
            "   ",
            "+",
            "2 +",
            "(2",
            "2)",
            "x = 5",
            "prev + 1",
            "ans",
            "1/0",
            "0/0",
            "sqrt(-1)",
            "9^9^9^9",
            "5 km + 3 kg",
            "2pi",
            "# 2+2",
            "2+2 # four",
            "C++ 2",
            "price 5 + 5",
            "10 C",
            "1 GB",
            "5 kilometers",
            "C",
            "hex",
            "0x",
            "in hex",
            "15:00",
            "5pm",
            "5pm London",
            "15:00 MSK",
            "31 dec",
            "dec 31",
            "today in history",
            "now in Narnia",
            "5pm meeting",
            "jan 2026",
            "may 5 - 3",
            "time to go",
            "100 usd",
            "$100",
            "100$",
            "btc",
            "usd",
        ] {
            assert_eq!(answer(q), None, "query: {q:?}");
        }
        // A query over 1 KiB is never evaluated.
        assert_eq!(answer(&format!("1{}", "+1".repeat(600))), None);
    }

    /// A tiny xorshift generator: no dependencies, repeatable.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
    fn search_is_fast() {
        const PIECES: &[&str] = &[
            "1",
            "2",
            "9",
            "42",
            "3.5",
            "1e3",
            "9^9",
            "((",
            "))",
            "(",
            ")",
            "+",
            "-",
            "*",
            "/",
            "^",
            "%",
            " ",
            " ",
            "mod",
            "of",
            "in",
            "to",
            "km",
            "mi",
            "kg",
            "usd",
            "$",
            "€",
            "btc",
            "in rub",
            "sqrt(",
            "ln(",
            "pi",
            "e",
            "x",
            "=",
            "#",
            "_",
            ".",
            "é",
            "0",
            "10 ^ 308",
            "9^9^9^9",
            "C",
            "°F",
            "km/h",
            "GiB",
            "0xff",
            "0b1",
            "hex",
            "deg",
            "hours",
            "m²",
            "now",
            "today",
            "Tokyo",
            "New York",
            "15:00",
            "5pm",
            "dec",
            "31",
            "until",
            "days",
            "to date",
            "to unix",
            "1700000000",
            " + 1 month",
        ];
        let ctx = ctx();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut worst = 0;
        for _ in 0..1000 {
            let mut query = String::new();
            let target = (rng.next() % 1024) as usize;
            while query.len() < target {
                let piece = PIECES[(rng.next() % PIECES.len() as u64) as usize];
                if query.len() + piece.len() > 1024 {
                    break;
                }
                query.push_str(piece);
            }
            let started = Instant::now();
            let _ = eval_query(&query, &ctx);
            let took = started.elapsed().as_micros();
            worst = worst.max(took);
            assert!(took < 10_000, "query took {took} us: {query:.60?}");
        }
        eprintln!("worst random query: {worst} us");
        // Worst realistic shapes, run explicitly.
        for query in [
            "9^9^9^9".to_string(),
            format!("1{}", "+1".repeat(511)),
            format!("{}1{}", "(".repeat(64), ")".repeat(64)),
            "9".repeat(1024),
            "x/".repeat(511) + "x",
        ] {
            let started = Instant::now();
            let _ = eval_query(&query, &ctx);
            assert!(started.elapsed().as_micros() < 10_000, "slow: {query:.40?}");
        }
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
    fn sheet_of_1000_lines_is_fast() {
        let mut text = String::new();
        for i in 0..1000 {
            match i % 6 {
                0 => text.push_str(&format!("v{i} = {i} * 2 + 1\n")),
                1 => text.push_str("prev + 15% of 80\n"),
                2 => text.push_str("5 km + 300 m in mi\n"),
                3 => text.push_str("sqrt(prev ^ 2) # comment\n"),
                4 => text.push_str("now in Tokyo\n"),
                _ => text.push('\n'),
            }
        }
        let started = Instant::now();
        let results = Sheet::eval(&text, &ctx());
        let took = started.elapsed();
        assert_eq!(results.len(), 1001);
        assert!(took.as_millis() < 50, "1000 lines took {took:?}");
        eprintln!("1000-line sheet: {took:?}");
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
    fn zone_sheet_of_10000_lines_is_fast() {
        let text = "5pm London in New York\n".repeat(10_000);
        let started = Instant::now();
        let results = Sheet::eval(&text, &ctx());
        let took = started.elapsed();
        assert_eq!(results.len(), 10_001);
        assert_eq!(
            show(&results[0]),
            "Thu, 19 Nov 2026, 12:00 EST",
            "every line is a moment"
        );
        assert!(took.as_millis() < 200, "10 000 zone lines took {took:?}");
        eprintln!("10 000-line zone sheet: {took:?}");
    }

    #[test]
    fn colours() {
        table(&[
            ("#ff8800", "rgb(255, 136, 0)"),
            ("#f80 in hex", "#ff8800"),
            ("#ff8800 in hsl", "hsl(32, 100%, 50%)"),
            ("#FF8800 to oklch", "oklch(0.744 0.181 56.5)"),
            ("rgb(255, 136, 0)", "#ff8800"),
            ("rgb(255 136 0 / 50%) in hex", "#ff880080"),
            ("rgba(255, 136, 0, 0.5) in rgb", "rgba(255, 136, 0, 0.5)"),
            ("rgb(100%, 50%, 0%) in rgb", "rgb(255, 128, 0)"),
            ("hsl(32, 100%, 50%) to hex", "#ff8800"),
            ("hsl(32deg 100% 50%)", "#ff8800"),
            ("hsl(180 deg, 100%, 50%) in hex", "#00ffff"),
            ("hsla(32, 100%, 50%, 0.5)", "#ff880080"),
            ("oklch(0.628 0.258 29.2) in hex", "#ff0000"),
            ("oklch(62.8% 0.258 29.2) in hex", "#ff0000"),
            (
                "oklch(0.744 0.181 56.5 / 0.5) in rgb",
                "rgba(255, 136, 0, 0.5)",
            ),
            ("orange in rgb", "rgb(255, 165, 0)"),
            ("Orange", "#ffa500"),
            ("tan in hex", "#d2b48c"),
            ("tan(0)", "0"),
            ("rgb(300, 0, 0)", "#ff0000"),
            (
                "rgb(255, 136, 0) in hsl in oklch",
                "oklch(0.744 0.181 56.5)",
            ),
            ("rgb(1, 2)", "error: rgb needs 3 or 4 values"),
            ("rgb(1, 2, 3, 4, 5)", "error: rgb needs 3 or 4 values"),
            ("rgb(1 km, 2, 3)", "error: rgb needs plain numbers"),
            ("255 in rgb", "error: rgb needs a color"),
            ("now in hsl", "error: hsl needs a color"),
            ("orange in bin", "error: bin needs a plain number"),
            ("x = #fff in bin", "error: bin needs a plain number"),
            ("orange + 1", "error: cannot add a color"),
            ("1 - orange", "error: cannot subtract a color"),
            ("orange * 2", "error: cannot multiply a color"),
            ("orange / 2", "error: cannot divide a color"),
            ("-orange", "error: cannot negate a color"),
            ("sqrt(orange)", "error: sqrt needs a plain number"),
            ("orange%", "error: a percentage needs a plain number"),
            ("10% of orange", "error: cannot multiply a color"),
            ("orange in km", "error: cannot convert a color to km"),
            (
                "orange in Tokyo",
                "error: cannot convert a color to Asia/Tokyo",
            ),
            ("255 in hex", "0xff"),
        ]);
    }

    #[test]
    fn colours_in_sheets() {
        assert_eq!(
            run("brand = #ff8800\nbrand in hsl\nb = #ff8800 in hsl\nb\nprev in hex"),
            [
                "brand = rgb(255, 136, 0)",
                "hsl(32, 100%, 50%)",
                "b = hsl(32, 100%, 50%)",
                "hsl(32, 100%, 50%)",
                "#ff8800"
            ]
        );
        assert_eq!(run("r = 10\nrgb(r, 0, 0) in hex")[1], "#0a0000");
        assert_eq!(run("red = 5\nred + 1"), ["red = 5", "6"]);
        assert_eq!(
            one("tan = 1"),
            "error: 'tan' is a function name and cannot be a variable"
        );
    }

    #[test]
    fn hash_comments_stay_comments() {
        assert_eq!(
            run("# note\n#todo\n#add later\n#add + more\nx = 5 #fee\n5 + 3 #fee\n#fee"),
            [
                "empty",
                "empty",
                "empty",
                "empty",
                "x = 5",
                "8",
                "rgb(255, 238, 238)"
            ]
        );
        assert_eq!(one("#fff + 1"), "empty");
        assert_eq!(one("#ff8800 in hls"), "empty");
        assert_eq!(one("c = #fff + 1"), "error: cannot add a color");
        assert_eq!(one("#fffff"), "empty");
        assert_eq!(one("#fff_"), "empty");
        assert_eq!(run("2 + 2\n#fee\nprev")[2], "rgb(255, 238, 238)");
        assert_eq!(run("2 + 2\n#add later\nprev")[2], "4");
    }

    #[test]
    fn colour_search_rules() {
        let answer = |q: &str| eval_query(q, &ctx()).map(|v| format(&v));
        assert_eq!(answer("#ff8800").as_deref(), Some("rgb(255, 136, 0)"));
        assert_eq!(
            answer("#ff8800 in hsl").as_deref(),
            Some("hsl(32, 100%, 50%)")
        );
        assert_eq!(answer("rgb(255, 136, 0)").as_deref(), Some("#ff8800"));
        assert_eq!(answer("orange in hex").as_deref(), Some("#ffa500"));
        for q in [
            "orange",
            "Orange",
            "red apple",
            "#hashtag",
            "meeting #42",
            "5 + 3 #note",
            "#fff + 1",
            "#",
            "rgb",
        ] {
            assert_eq!(answer(q), None, "{q}");
        }
    }

    #[test]
    fn bases_any() {
        table(&[
            ("255 in base 36", "73"),
            ("255 in base 2", "0b11111111"),
            ("255 in base 8", "0o377"),
            ("255 in base 16", "0xff"),
            ("255 in base 10", "255"),
            ("35 in base 36", "z"),
            ("-255 in base 36", "-73"),
            ("0 in base 3", "0"),
            ("255 in hexadecimal", "0xff"),
            ("5 in binary", "0b101"),
            ("8 in octal", "0o10"),
            ("255 in base 36 + 1", "256"),
            ("255 in base 1", "error: base must be 2 to 36"),
            ("255 in base 37", "error: base must be 2 to 36"),
            ("255 in base 2.5", "error: base must be 2 to 36"),
            ("255 in base", "error: expected a number after 'base'"),
            ("2.5 in base 3", "error: base 3 needs a whole number"),
            ("5 km in base 3", "error: base 3 needs a plain number"),
            ("orange in base 16", "#ffa500"),
            ("31 dec in Tokyo", "Thu, 31 Dec 2026"),
        ]);
    }

    #[test]
    fn even_more_units() {
        table(&[
            ("1 kWh in J", "3600000 J"),
            ("1 kcal in kJ", "4.184 kJ"),
            ("1 BTU in J", "1055.055853 J"),
            ("1 eV in J", "1.602176634e-19 J"),
            ("1 MJ in kWh", "0.2777777778 kWh"),
            ("1 Wh in J", "3600 J"),
            ("1000 cal in kcal", "1 kcal"),
            ("1 hp in W", "745.6998716 W"),
            ("1 GW in MW", "1000 MW"),
            ("1 atm in hPa", "1013.25 hPa"),
            ("30 psi in bar", "2.068427188 bar"),
            ("760 mmHg in atm", "1.000000142 atm"),
            ("760 torr in atm", "1 atm"),
            ("1 bar in mbar", "1000 mbar"),
            ("1 MPa in kPa", "1000 kPa"),
            ("1 Pa in Pa", "1 Pa"),
            ("2.4 GHz in MHz", "2400 MHz"),
            ("1 kHz in Hz", "1000 Hz"),
            ("1 PB in TB", "1000 TB"),
            ("1 PiB in TiB", "1024 TiB"),
            ("100 Mbps in MB/s", "12.5 MB/s"),
            ("1 Gbps in Mbps", "1000 Mbps"),
            ("1 MiB/s in KiB/s", "1024 KiB/s"),
            ("8 bps in B/s", "1 B/s"),
            ("1 Gbit in MB", "125 MB"),
            ("1 Gb in Mb", "1000 Mbit"),
            ("1 kb in bit", "1000 bit"),
            ("1 Tbit in GB", "125 GB"),
            ("10 L/100km in mpg", "23.52145833 mpg"),
            ("30 mpg in L/100km", "7.840486111 L/100km"),
            ("20 km/L in L/100km", "5 L/100km"),
            ("10 l/100km in km/l", "10 km/L"),
            ("5 L/100km + 1 L/100km", "6 L/100km"),
            ("5 L/100km + 20 km/L", "10 L/100km"),
            ("0 L/100km in mpg", "error: too large"),
            ("10 l / 100 km", "0.1 mm2"),
            ("1 nmi in km", "1.852 km"),
            ("500 nm in µm", "0.5 µm"),
            ("1 um in nm", "1000 nm"),
            ("1 ms in µs", "1000 µs"),
            ("1 µs in ns", "1000 ns"),
            ("1 us in ns", "1000 ns"),
            ("10 st in kg", "63.5029318 kg"),
            ("5 ct in g", "1 g"),
            ("1 l in dl", "10 dl"),
            ("1 cl in ml", "10 ml"),
            ("1 cc in ml", "1 ml"),
            ("1 cm3 in ml", "1 ml"),
            ("100 grad in deg", "90 deg"),
            ("1 turn in deg", "360 deg"),
            ("60 arcmin in deg", "1 deg"),
            ("3600 arcsec in deg", "1 deg"),
            ("hsl(0.5 turn, 100%, 50%) in hex", "#00ffff"),
        ]);
    }

    #[test]
    fn new_units_make_each_other() {
        table(&[
            ("2 kW * 3 h", "6 kWh"),
            ("3 h * 2 kW", "6 kWh"),
            ("1 kWh / 100 W", "36000 s"),
            ("1 kWh / 100 W in h", "10 h"),
            ("1 kWh / 1 h", "1 kW"),
            ("1 GB / 100 Mbps", "80 s"),
            ("10 MB/s * 1 min", "600 MB"),
            ("1 GB / 1 s", "1 GB/s"),
            ("1 GB / 1 GB", "1"),
            ("1 kWh / 1 J", "3600000"),
            ("1 kWh * 1 h", "error: cannot multiply kWh and h"),
            ("1 Pa * 1 m", "error: cannot multiply Pa and m"),
        ]);
    }

    #[test]
    fn multiword_zones_beat_units() {
        assert!(
            one("5pm in St Louis").ends_with("CST"),
            "{}",
            one("5pm in St Louis")
        );
        assert!(one("now in St Petersburg").ends_with("MSK"));
        assert!(one("5pm St Louis in London").ends_with("GMT"));
        assert_eq!(one("10 st in kg"), "63.5029318 kg");
    }

    #[test]
    fn new_names_are_soft() {
        assert_eq!(run("W = 3\nx = 5\nW + x"), ["W = 3", "x = 5", "8"]);
        assert_eq!(run("bar = 2\nbar * 3"), ["bar = 2", "6"]);
        assert_eq!(one("2 bar in psi"), "29.00754755 psi");
    }

    /// No unit name or alias is also a currency, a month or a time zone, but those that
    /// were already: the soft-names ruling keeps the unit after a number, and these stay
    /// as they were.
    #[test]
    fn unit_names_collide_with_nothing_new() {
        // `GB` is also the IANA link for Britain; `Gb` (gigabit) joins it, and `now in gb`
        // in lower case is still Britain.
        const OLD: &[&str] = &["cup", "t", "ton", "tons", "min", "in", "c", "C", "GB", "Gb"];
        let clashes: Vec<_> = units::all_names()
            .into_iter()
            .filter(|n| !OLD.contains(n))
            .filter(|n| {
                currency::lookup(n).is_some()
                    || time::month(&n.to_lowercase()).is_some()
                    || zones::find(n).is_some()
            })
            .collect();
        assert!(clashes.is_empty(), "{clashes:?}");
    }

    #[test]
    fn short_city_names_in_expressions() {
        table(&[
            ("5pm bcn in msk", "Thu, 19 Nov 2026, 19:00 MSK"),
            ("15:00 nyc in ekb", "Fri, 20 Nov 2026, 01:00 +05"),
            ("now in spb", "Thu, 19 Nov 2026, 17:42 MSK"),
            // MAD stays the dirham (the test rates have none), not Madrid.
            ("100 usd in mad", "error: no rate for MAD"),
        ]);
    }

    /// Final review: lines that worked before this branch keep their answers, units and all.
    #[test]
    fn old_unit_arithmetic_keeps_its_answers() {
        table(&[
            // Fuel: a trip's fuel and a tank's range, as before `L/100km` was one unit.
            ("8 l/100km * 250 km", "20 l"),
            ("6.5 l/100km * 400 km in l", "26 l"),
            ("50 l / (8 l/100km)", "625 km"),
            // Only the new kinds take an operand's unit.
            ("1 l / 1 cm2", "10 m"),
            ("2 l / 50 cm2", "0.4 m"),
            ("1 cm * 1 m2", "10 l"),
            ("1 km * 1 cm2", "100 l"),
            ("1 ml / 1 km", "1e-9 m2"),
            ("1 cm * 1 mm2", "0.00001 l"),
            ("1 ha * 1 m", "10000000 l"),
            ("1 km2 * 1 m", "1000000000 l"),
            // Units added by this branch are not chosen for old kinds.
            ("1 kn * 1 h", "1852 m"),
            ("1 m * 1 cm2", "0.1 l"),
            ("1 mm2 / 1 m", "0.000001 m"),
            ("1 m3 / 1 km2", "0.000001 m"),
            // The new kinds still do.
            ("10 MB/s * 1 min", "600 MB"),
            ("2 kW * 3 h", "6 kWh"),
        ]);
        assert_eq!(run("x = 8 l/100km\nx * 300 km")[1], "24 l");
    }

    #[test]
    fn colour_review_minors() {
        // Only a hue takes an angle; hsl's lightness does not.
        assert_eq!(one("hsl(0, 100%, 1 rad)"), "error: hsl needs plain numbers");
        assert_eq!(one("hsl(1 rad, 100%, 50%) in hex"), "#fff400");
        // A colour followed by a comment is no search answer.
        assert_eq!(eval_query("#ff8800 #note", &ctx()), None);
        assert_eq!(eval_query("#ff8800 in hsl #note", &ctx()), None);
        assert!(eval_query("#ff8800 in hsl", &ctx()).is_some());
        // When both readings fail, a colour that did not even parse gives the comment
        // reading's error.
        assert_eq!(one("issue #1234"), "error: unknown name 'issue'");
        assert_eq!(one("c = #fff + 1"), "error: cannot add a color");
    }
}
