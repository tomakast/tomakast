//! What the search answers, a sheet line answers too, with the same text: every input goes
//! through the search answer ([`answer_parts`]) and through a sheet's annotations
//! ([`annotations`]), with one fixed context (now, a zone, test rates).
//!
//! A sheet line is not a search: the bare values search hides (`10 km`, `100 usd`, `31 dec`)
//! show their value in a sheet. Rows whose sheet text has several lines build the value with
//! variables or `prev`; their query is the same computation written on one line.

use std::sync::OnceLock;

use crate::engine::{Context, Rates, Table, zones};
use crate::views::{annotations, answer_parts};

/// 2026-11-19T14:42:00Z.
const NOW: i64 = 1_795_099_320;

/// Both tables fetched a minute before [`NOW`].
fn rates() -> &'static Rates {
    static RATES: OnceLock<Rates> = OnceLock::new();
    RATES.get_or_init(|| {
        let table = |pairs: &[(&'static str, f64)]| Table {
            fetched: NOW - 60,
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

/// [`NOW`] in Moscow, with [`rates`].
fn ctx() -> Context<'static> {
    ctx_at(NOW)
}

fn ctx_at(now: i64) -> Context<'static> {
    Context::new(now, zones::find("Europe/Moscow").unwrap(), rates())
}

/// What search does with a row's query.
#[derive(Clone, Copy, PartialEq)]
enum Search {
    /// It answers what the sheet shows.
    Same,
    /// It gives no answer (a bare value), but the sheet shows the value.
    Hidden,
}

use Search::{Hidden, Same};

/// (sheet text, the query of the same computation, or "" for the sheet's text, what the
/// sheet's last line shows, what search does).
const ROWS: &[(&str, &str, &str, Search)] = &[
    // Length, mass, time.
    ("5 km in mi", "", "3.106855961 mi", Same),
    ("1 mi to km", "", "1.609344 km", Same),
    ("3 ft + 20 cm", "", "3.656167979 ft", Same),
    ("12 in cm", "", "30.48 cm", Same),
    ("5 kilometers in miles", "", "3.106855961 mi", Same),
    ("500 g to kg", "", "0.5 kg", Same),
    ("1 lb in oz", "", "16 oz", Same),
    ("1 ton in kg", "", "1000 kg", Same),
    ("2 h in min", "", "120 min", Same),
    ("90 seconds in min", "", "1.5 min", Same),
    ("1 week in days", "", "7 days", Same),
    // Temperature.
    ("0 C in F", "", "32 °F", Same),
    ("100 °F to °C", "", "37.77777778 °C", Same),
    ("300 K in C", "", "26.85 °C", Same),
    ("20 celsius in fahrenheit", "", "68 °F", Same),
    // Area and volume.
    ("1 ha in m2", "", "10000 m2", Same),
    ("1 acre in m2", "", "4046.856422 m2", Same),
    ("1 m² in cm2", "", "10000 cm2", Same),
    ("1 gal in l", "", "3.785411784 l", Same),
    ("1 cup in ml", "", "236.5882365 ml", Same),
    ("3 tsp in tbsp", "", "1 tbsp", Same),
    // Speed, data, angle.
    ("100 km/h in mph", "", "62.13711922 mph", Same),
    ("10 m/s in km/h", "", "36 km/h", Same),
    ("1 kn in km/h", "", "1.852 km/h", Same),
    ("1 GB in MB", "", "1000 MB", Same),
    ("1 GiB in MiB", "", "1024 MiB", Same),
    ("1 B in bit", "", "8 bit", Same),
    ("180 deg in rad", "", "3.141592654 rad", Same),
    ("sin(90 deg)", "", "1", Same),
    // Number bases.
    ("255 in hex", "", "0xff", Same),
    ("255 in bin", "", "0b11111111", Same),
    ("0xff", "", "255", Same),
    ("0b1010 + 1", "", "11", Same),
    ("0o17", "", "15", Same),
    ("0xff in decimal", "", "255", Same),
    // Dates, times and zones.
    ("now", "", "Thu, 19 Nov 2026, 17:42 MSK", Same),
    ("today", "", "Thu, 19 Nov 2026", Same),
    ("tomorrow", "", "Fri, 20 Nov 2026", Same),
    ("now in Tokyo", "", "Thu, 19 Nov 2026, 23:42 JST", Same),
    ("time in London", "", "Thu, 19 Nov 2026, 14:42 GMT", Same),
    ("now + 2 weeks", "", "Thu, 3 Dec 2026, 17:42 MSK", Same),
    ("today - 3 days", "", "Mon, 16 Nov 2026", Same),
    ("days until 31 dec", "", "42 days", Same),
    ("weeks until 31 dec", "", "6 weeks", Same),
    ("dec 31 - today", "", "42 days", Same),
    (
        "5pm London in Moscow",
        "",
        "Thu, 19 Nov 2026, 20:00 MSK",
        Same,
    ),
    ("15:00 MSK in PST", "", "Thu, 19 Nov 2026, 04:00 PST", Same),
    (
        "1700000000 to date",
        "",
        "Wed, 15 Nov 2023, 01:13 MSK",
        Same,
    ),
    ("now to unix", "", "1795099320", Same),
    ("today to unix", "", "1795035600", Same),
    ("31 dec in Tokyo", "", "Thu, 31 Dec 2026", Same),
    ("today in Tokyo", "", "Thu, 19 Nov 2026", Same),
    ("3pm in London", "", "Thu, 19 Nov 2026, 12:00 GMT", Same),
    // Fiat and crypto.
    ("100 usd in rub", "", "9 312.50 RUB", Same),
    ("100 USD to RUB", "", "9 312.50 RUB", Same),
    ("$100 to €", "", "92.00 EUR", Same),
    ("100$ in eur", "", "92.00 EUR", Same),
    ("50 eur + 20 usd in gbp", "", "58.73 GBP", Same),
    ("¥1000 in usd", "", "6.67 USD", Same),
    ("£50 in eur", "", "58.23 EUR", Same),
    ("0.5 btc in usd", "", "33 500.00 USD", Same),
    ("1 eth in btc", "", "0.052238806 BTC", Same),
    ("1 btc in rub", "", "6 239 375.00 RUB", Same),
    ("100 usd * 2", "", "200.00 USD", Same),
    ("100 usd / 4", "", "25.00 USD", Same),
    ("100 usd / 4 usd", "", "25", Same),
    // Percentages.
    ("15% of 80", "", "12", Same),
    ("80 + 15%", "", "92", Same),
    ("80 - 15%", "", "68", Same),
    ("10% of 5 km", "", "0.5 km", Same),
    ("100 usd + 10%", "", "110.00 USD", Same),
    ("5000 usd - 13%", "", "4 350.00 USD", Same),
    // Functions and constants.
    ("sqrt(16) + abs(-2)", "", "6", Same),
    ("round(2.5)", "", "3", Same),
    ("ln(e)", "", "1", Same),
    ("log(1000)", "", "3", Same),
    ("pi * 2", "", "6.283185307", Same),
    ("2 ^ 10", "", "1024", Same),
    ("10 mod 3", "", "1", Same),
    ("(2 + 3) * 4", "", "20", Same),
    // Arithmetic with units, within a dimension and across dimensions.
    ("5 km + 300 m", "", "5.3 km", Same),
    ("2 h * 3", "", "6 h", Same),
    ("1 GiB / 2 MiB", "", "512", Same),
    ("10 km / 2 h", "", "5 km/h", Same),
    ("10 km / 2 h in km/h", "", "5 km/h", Same),
    ("100 m / 10 s", "", "10 m/s", Same),
    ("90 km/h * 2 h", "", "180 km", Same),
    ("90 km/h * 2 h in km", "", "180 km", Same),
    ("2 h * 90 km/h", "", "180 km", Same),
    ("60 mph * 30 min in km", "", "48.28032 km", Same),
    ("100 km / 50 km/h", "", "2 h", Same),
    ("2 m * 3 m", "", "6 m2", Same),
    ("2 m * 3 m * 4 m in l", "", "24000 l", Same),
    ("6 m2 / 2 m", "", "3 m", Same),
    // Bare values: search hides them, a sheet shows them.
    ("10 km", "", "10 km", Hidden),
    ("100 usd", "", "100.00 USD", Hidden),
    ("$100", "", "100.00 USD", Hidden),
    ("31 dec", "", "Thu, 31 Dec 2026", Hidden),
    ("5pm London", "", "Thu, 19 Nov 2026, 17:00 GMT", Hidden),
    ("10 C", "", "10 °C", Hidden),
    ("15:00", "", "Thu, 19 Nov 2026, 15:00 MSK", Hidden),
    ("42", "", "42", Hidden),
    // Variables that hold money, units, dates and percentages; `prev` and `ans`.
    (
        "rent = 1200 eur\nfood = 300 usd\nrent + food in rub",
        "1200 eur + 300 usd in rub",
        "149 404.89 RUB",
        Same,
    ),
    (
        "rent = 1200 eur\nrent in usd",
        "1200 eur in usd",
        "1 304.35 USD",
        Same,
    ),
    (
        "a = 1200 eur\nb = 300 eur\nc = 50 eur\na + b + c",
        "1200 eur + 300 eur + 50 eur",
        "1 550.00 EUR",
        Same,
    ),
    (
        "speed = 90 km/h\nt = 2 h\nspeed * t in km",
        "90 km/h * 2 h in km",
        "180 km",
        Same,
    ),
    (
        "dist = 10 km\ntime = 2 h\ndist / time",
        "10 km / 2 h",
        "5 km/h",
        Same,
    ),
    (
        "price = $100\nprice * 1.2 in eur",
        "$100 * 1.2 in eur",
        "110.40 EUR",
        Same,
    ),
    (
        "meeting = 5pm London\nmeeting in Tokyo",
        "5pm London in Tokyo",
        "Fri, 20 Nov 2026, 02:00 JST",
        Same,
    ),
    (
        "d = 5 km\nd + 300 m in mi",
        "5 km + 300 m in mi",
        "3.293267319 mi",
        Same,
    ),
    ("x = 3 km\nprev in m", "3 km in m", "3000 m", Same),
    ("2 h * 3\nans in min", "(2 h * 3) in min", "360 min", Same),
    (
        "100 usd in rub\nprev / 2",
        "(100 usd in rub) / 2",
        "4 656.25 RUB",
        Same,
    ),
    ("w = 1 btc\nw in usd", "1 btc in usd", "67 000.00 USD", Same),
    (
        "rate = 93 rub\n100 * rate in usd",
        "100 * 93 rub in usd",
        "99.87 USD",
        Same,
    ),
    (
        "deadline = 31 dec\ndays until deadline",
        "days until 31 dec",
        "42 days",
        Same,
    ),
    (
        "start = today\nstart + 1 month",
        "today + 1 month",
        "Sat, 19 Dec 2026",
        Same,
    ),
    (
        "now\nprev + 1 h",
        "now + 1 h",
        "Thu, 19 Nov 2026, 18:42 MSK",
        Same,
    ),
    (
        "today\nprev in Tokyo",
        "today in Tokyo",
        "Thu, 19 Nov 2026",
        Same,
    ),
    ("n = 255\nn in hex", "255 in hex", "0xff", Same),
    ("temp = 20 C\ntemp in F", "20 C in F", "68 °F", Same),
    (
        "big = 1 GiB\nsmall = 2 MiB\nbig / small",
        "1 GiB / 2 MiB",
        "512",
        Same,
    ),
    (
        "salary = 5000 usd\ntax = 13%\nsalary - tax",
        "5000 usd - 13%",
        "4 350.00 USD",
        Same,
    ),
    (
        "tax = 13%\ntax of 5000 usd",
        "13% of 5000 usd",
        "650.00 USD",
        Same,
    ),
    ("tip = 15%\np = tip\n80 + p", "80 + 15%", "92", Same),
    ("15%\n80 + prev", "80 + 15%", "92", Same),
    ("x = 5 km\nx", "5 km", "5 km", Hidden),
    ("t = 2 h", "2 h", "2 h", Hidden),
];

/// What a sheet of `text` shows on each line: its value, `error: …`, or `None` for an empty
/// line.
fn sheet(text: &str, ctx: &Context) -> Vec<Option<String>> {
    let notes = annotations(text, ctx);
    (0..text.split('\n').count() as u32)
        .map(|line| {
            notes.iter().find(|a| a.line == line).map(|a| {
                if a.is_error {
                    format!("error: {}", a.text)
                } else {
                    a.text.clone()
                }
            })
        })
        .collect()
}

#[test]
fn every_search_answer_is_a_sheet_answer() {
    assert!(ROWS.len() >= 80, "{} rows", ROWS.len());
    let mut wrong = Vec::new();
    for &(text, query, want, search) in ROWS {
        let query = if query.is_empty() { text } else { query };
        let lines = sheet(text, &ctx());
        if lines.last().cloned().flatten().as_deref() != Some(want)
            || lines
                .iter()
                .any(|l| l.as_deref().is_none_or(|l| l.starts_with("error")))
        {
            wrong.push(format!("sheet {text:?}: {lines:?}, want {want:?}"));
        }
        let answered = answer_parts(query, &ctx()).map(|a| a.result);
        let expected = (search == Same).then(|| want.to_owned());
        if answered != expected {
            wrong.push(format!("search {query:?}: {answered:?}, want {expected:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} rows differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Where a sheet and the one-line query differ by design: a variable holds the value its line
/// shows.
#[test]
fn a_variable_holds_the_value_its_line_shows() {
    // A date written without a year that is past means next year's in `until`; stored in a
    // variable it is the date the line shows, this year's.
    assert_eq!(
        sheet("d = 18 nov\ndays until d", &ctx()),
        [Some("Wed, 18 Nov 2026".into()), Some("-1 days".into())]
    );
    assert_eq!(
        answer_parts("days until 18 nov", &ctx()).map(|a| a.result),
        Some("364 days".into())
    );
    // `today in Tokyo` is Tokyo's date; a date already shown keeps its civil date. At 23:00
    // in Moscow, Tokyo is a day ahead.
    let late = ctx_at(1_795_118_400);
    assert_eq!(
        sheet("today\nprev in Tokyo\ntoday in Tokyo", &late),
        [
            Some("Thu, 19 Nov 2026".into()),
            Some("Thu, 19 Nov 2026".into()),
            Some("Fri, 20 Nov 2026".into()),
        ]
    );
}

#[test]
fn a_percentage_is_one_wherever_it_is_kept() {
    let shown = |text: &str| sheet(text, &ctx()).pop().flatten();
    // Reassigned to a plain number, the variable is a plain number again.
    assert_eq!(shown("p = 10%\np = 2\n80 + p").as_deref(), Some("82"));
    assert_eq!(shown("p = -10%\n80 + p").as_deref(), Some("72"));
    assert_eq!(shown("p = 10%\n80 - -p").as_deref(), Some("88"));
    // A sum of percentages is a plain number, as written on one line.
    assert_eq!(shown("p = 10% + 5%\n80 + p").as_deref(), Some("80.15"));
    // `prev` after an empty line or an error is still the last value.
    assert_eq!(shown("10%\n\nnope\n80 + prev").as_deref(), Some("88"));
    assert_eq!(shown("10%\n5\n80 + prev").as_deref(), Some("85"));
}

#[test]
fn the_tonne_stays_after_a_number() {
    let shown = |text: &str| sheet(text, &ctx());
    assert_eq!(
        shown("t = 2 h\n3 t in kg\nt in min"),
        [
            Some("2 h".into()),
            Some("3000 kg".into()),
            Some("120 min".into())
        ]
    );
    assert_eq!(
        shown("t"),
        [Some("error: 't' is a unit; put a number before it".into())]
    );
}
