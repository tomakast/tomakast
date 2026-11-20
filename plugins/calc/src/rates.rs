//! The exchange rates calc holds: read from storage at start, lent to every evaluation, noted
//! as used by the answers that needed them, and fetched again in the background.
//!
//! Two feeds: fiat from open.er-api.com ([`FIAT_URL`]) and crypto from CoinGecko
//! ([`crypto_url`]). `init` and a repeating [`TICK_MS`] timer [`refresh`](Feeds::refresh)
//! them: a feed that is [`due`] is fetched, and its answer is kept in memory and written to
//! storage (one write, under the sheets' budget). A failed fetch of a feed calc has no table
//! of is tried again once after [`RETRY_MS`], not only on the next tick. Search never fetches, never waits for an
//! answer and writes nothing for rates: it reads what is in memory.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;

use tomakast_plugin::{HttpDone, HttpRequest, RequestId, TimerId, http, log, storage, timers};

use crate::engine::Used;
use crate::engine::currency::{CRYPTO, FIAT, Rates, Table};
use crate::json::{self, Json};
use crate::sheets;

pub const FIAT_KEY: &str = "rates:fiat";
pub const CRYPTO_KEY: &str = "rates:crypto";

pub const FIAT_URL: &str = "https://open.er-api.com/v6/latest/USD";

/// CoinGecko's USD prices of every id of [`CRYPTO`], in table order.
pub fn crypto_url() -> String {
    let ids: Vec<&str> = CRYPTO.iter().map(|(_, id)| *id).collect();
    format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
        ids.join(",")
    )
}

/// The refresh timer's interval: 10 minutes.
pub const TICK_MS: u64 = 600_000;
/// The one-shot retry after a failed fetch of a feed calc has no table of: 60 s.
pub const RETRY_MS: u64 = 60_000;
/// How much sooner than its age a feed is due, so that a tick a little early still fetches.
const SLACK: i64 = 60;
/// Fiat is due at this age, crypto at [`CRYPTO_AGE`] (both less [`SLACK`]).
const FIAT_AGE: i64 = 3600;
const CRYPTO_AGE: i64 = 600;
/// Crypto is fetched again (outside `init`) only when an answer needed it this recently.
const CRYPTO_USE: i64 = 1800;

/// One flag per feed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Due {
    pub fiat: bool,
    pub crypto: bool,
}

/// The refresh policy of the Global Constraints; feeds in `in_flight` are never due.
///
/// - Fiat is due when missing or an hour old.
/// - Crypto is due when missing, or when 10 minutes old and either at `init` or used within
///   the last 30 minutes.
///
/// The age of a table is the time calc received it; both ages less [`SLACK`]. A table
/// [from the future](from_the_future) counts as old.
pub fn due(
    now: i64,
    rates: &Rates,
    crypto_used_at: Option<i64>,
    at_init: bool,
    in_flight: Due,
) -> Due {
    let aged = |table: &Option<Table>, age: i64| {
        table
            .as_ref()
            .is_none_or(|t| now - t.fetched >= age - SLACK || from_the_future(t, now))
    };
    let missing = rates.crypto.is_none();
    let wanted = at_init || crypto_used_at.is_some_and(|at| now - at <= CRYPTO_USE);
    Due {
        fiat: !in_flight.fiat && aged(&rates.fiat, FIAT_AGE),
        crypto: !in_flight.crypto && (missing || (wanted && aged(&rates.crypto, CRYPTO_AGE))),
    }
}

/// Whether `table` says it was fetched more than [`SLACK`] after `now`: the clock was ahead
/// when it came, or the stored value is bad. Its age is unknown, so it is due and the answer
/// says so.
pub fn from_the_future(table: &Table, now: i64) -> bool {
    table.fetched > now.saturating_add(SLACK)
}

/// Status 200, `result == "success"`, at least one known code: the table, fetched at `now`.
/// A body over 8 KiB is refused unread.
pub fn parse_fiat(status: u16, body: &[u8], now: i64) -> Option<Table> {
    let body = answer_json(status, body)?;
    if body.get("result") != Some(&Json::Str("success".into())) {
        return None;
    }
    let Some(Json::Obj(rates)) = body.get("rates") else {
        return None;
    };
    let values = rates
        .iter()
        .filter_map(|(code, value)| Some((fiat_code(code)?, price(value)?)));
    table(values, now)
}

/// Status 200, at least one known id with a `usd` price: the table by ticker. A body over
/// 8 KiB is refused unread.
pub fn parse_crypto(status: u16, body: &[u8], now: i64) -> Option<Table> {
    let Json::Obj(members) = answer_json(status, body)? else {
        return None;
    };
    let values = members.iter().filter_map(|(id, value)| {
        let (ticker, _) = CRYPTO.iter().find(|(_, known)| known == id)?;
        Some((*ticker, price(value.get("usd")?)?))
    });
    table(values, now)
}

/// The longest answer body calc reads (the background budget); longer ones are refused
/// unread. [`json::parse`] has its own, larger ceiling.
const MAX_ANSWER_BYTES: usize = 8 * 1024;

/// The JSON of an answer with status 200 and at most [`MAX_ANSWER_BYTES`] of body.
fn answer_json(status: u16, body: &[u8]) -> Option<Json> {
    if status != 200 || body.len() > MAX_ANSWER_BYTES {
        return None;
    }
    json::parse(std::str::from_utf8(body).ok()?)
}

/// The static spelling of fiat code `code`.
fn fiat_code(code: &str) -> Option<&'static str> {
    FIAT.iter().copied().find(|known| *known == code)
}

/// A rate: a finite number above 0, as [`decode`] keeps.
fn price(value: &Json) -> Option<f64> {
    match value {
        Json::Num(n) if n.is_finite() && *n > 0.0 => Some(*n),
        _ => None,
    }
}

fn table(values: impl Iterator<Item = (&'static str, f64)>, now: i64) -> Option<Table> {
    let values: HashMap<_, _> = values.collect();
    (!values.is_empty()).then_some(Table {
        fetched: now,
        values,
    })
}

/// "<fetched unix> CODE=value CODE=value …", values in Rust's `{}` for f64 (an exact round
/// trip), in code order.
pub fn encode(table: &Table) -> String {
    let mut pairs: Vec<_> = table.values.iter().collect();
    pairs.sort_unstable_by_key(|(code, _)| **code);
    let mut text = table.fetched.to_string();
    for (code, value) in pairs {
        let _ = write!(text, " {code}={value}");
    }
    text
}

/// The table `text` holds. `None` for garbage or a table with no valid pair; codes calc does
/// not know and values that are not finite and above 0 are skipped.
pub fn decode(text: &str) -> Option<Table> {
    let mut words = text.split_ascii_whitespace();
    let fetched = words.next()?.parse().ok()?;
    let values: HashMap<_, _> = words
        .filter_map(|pair| {
            let (code, value) = pair.split_once('=')?;
            let code = known(code)?;
            let value: f64 = value.parse().ok()?;
            (value.is_finite() && value > 0.0).then_some((code, value))
        })
        .collect();
    (!values.is_empty()).then_some(Table { fetched, values })
}

/// The static spelling of `code`, if it is a fiat code or a crypto ticker.
fn known(code: &str) -> Option<&'static str> {
    FIAT.iter()
        .copied()
        .chain(CRYPTO.iter().map(|(ticker, _)| *ticker))
        .find(|known| *known == code)
}

/// A rate feed; it indexes the per-feed arrays of [`Feeds`].
#[derive(Clone, Copy)]
enum Feed {
    Fiat,
    Crypto,
}

const FEEDS: [Feed; 2] = [Feed::Fiat, Feed::Crypto];

impl Feed {
    fn key(self) -> &'static str {
        match self {
            Feed::Fiat => FIAT_KEY,
            Feed::Crypto => CRYPTO_KEY,
        }
    }

    fn url(self) -> String {
        match self {
            Feed::Fiat => FIAT_URL.to_owned(),
            Feed::Crypto => crypto_url(),
        }
    }

    fn parse(self, status: u16, body: &[u8], now: i64) -> Option<Table> {
        match self {
            Feed::Fiat => parse_fiat(status, body, now),
            Feed::Crypto => parse_crypto(status, body, now),
        }
    }

    fn of(self, due: Due) -> bool {
        match self {
            Feed::Fiat => due.fiat,
            Feed::Crypto => due.crypto,
        }
    }
}

/// What calc holds about rates.
#[derive(Default)]
pub struct Feeds {
    pub rates: Rates,
    /// When an answer last needed the crypto rates, in unix seconds.
    crypto_used_at: Option<i64>,
    /// The request of each feed that waits for its answer.
    requests: [Option<RequestId>; 2],
    /// The feeds held in memory whose last write found no room or failed.
    unsaved: [bool; 2],
    /// The repeating [`TICK_MS`] timer.
    pub tick: Option<TimerId>,
    /// The one-shot [`RETRY_MS`] timer that waits to fire, if any: at most one at a time.
    retry: Option<TimerId>,
}

thread_local! {
    static STATE: RefCell<Feeds> = RefCell::default();
}

/// Runs `f` on the plugin's feeds. `f` must not call `with` again.
pub fn with<R>(f: impl FnOnce(&mut Feeds) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

impl Feeds {
    /// Reads both stored feeds. A read error is logged and leaves that feed empty, as does a
    /// stored value that is not a table.
    pub fn load(&mut self) {
        self.rates.fiat = read(FIAT_KEY);
        self.rates.crypto = read(CRYPTO_KEY);
    }

    /// Notes which feeds an answer needed at `now`. Memory only: nothing is stored.
    pub fn mark_used(&mut self, used: Used, now: i64) {
        if used.crypto {
            self.crypto_used_at = Some(now);
        }
    }

    /// The feeds [`refresh`](Self::refresh) would fetch at `now`.
    pub fn due_at(&self, now: i64, at_init: bool) -> Due {
        let in_flight = Due {
            fiat: self.requests[Feed::Fiat as usize].is_some(),
            crypto: self.requests[Feed::Crypto as usize].is_some(),
        };
        due(now, &self.rates, self.crypto_used_at, at_init, in_flight)
    }

    /// Starts the due fetches (`http::fetch`, `accept: application/json`), remembering the
    /// request ids; retries an unsaved write first. A failed `fetch` is logged.
    ///
    /// A feed about to be fetched is not written again here: its answer writes it, so a feed
    /// makes at most one write per refresh.
    pub fn refresh(&mut self, now: i64, at_init: bool) {
        let due = self.due_at(now, at_init);
        for feed in FEEDS {
            if self.unsaved[feed as usize] && !feed.of(due) {
                self.save(feed);
            }
        }
        for feed in FEEDS.into_iter().filter(|feed| feed.of(due)) {
            let request = HttpRequest::get(feed.url()).header("accept", "application/json");
            match http::fetch(&request) {
                Ok(id) => self.requests[feed as usize] = Some(id),
                Err(e) => {
                    log::warn(&format!("fetching {} failed: {e}", feed.key()));
                    self.retry_soon(feed);
                }
            }
        }
    }

    /// A timer fired: the tick or the retry refreshes, anything else is ignored.
    pub fn timer_fired(&mut self, timer: TimerId, now: i64) {
        if self.retry_fired(timer) || self.tick == Some(timer) {
            self.refresh(now, false);
        }
    }

    /// Whether `timer` is the waiting retry; if so it no longer waits.
    fn retry_fired(&mut self, timer: TimerId) -> bool {
        let fired = self.retry == Some(timer);
        if fired {
            self.retry = None;
        }
        fired
    }

    /// Whether a failed fetch of `feed` asks for the retry: calc holds no table of it (the
    /// launcher may have started before the network), and no retry waits already.
    fn wants_retry(&self, feed: Feed) -> bool {
        self.retry.is_none()
            && match feed {
                Feed::Fiat => self.rates.fiat.is_none(),
                Feed::Crypto => self.rates.crypto.is_none(),
            }
    }

    /// A fetch of `feed` failed: starts the one-shot retry when [`wants_retry`](Self::wants_retry).
    fn retry_soon(&mut self, feed: Feed) {
        if !self.wants_retry(feed) {
            return;
        }
        match timers::start(RETRY_MS, false) {
            Ok(timer) => self.retry = Some(timer),
            Err(e) => log::warn(&format!("starting the rates retry failed: {e}")),
        }
    }

    /// An answer to one of its requests: parses it, keeps the table in memory and writes it
    /// with [`Sheets::set_background`](sheets::Sheets::set_background). Anything else is
    /// ignored. A bad answer is logged and changes nothing, except that it may start the
    /// retry ([`retry_soon`](Self::retry_soon)). Returns whether a new table is in memory.
    pub fn answered(&mut self, done: HttpDone, now: i64) -> bool {
        let Some(feed) = FEEDS
            .into_iter()
            .find(|feed| self.requests[*feed as usize] == Some(done.request))
        else {
            return false;
        };
        self.requests[feed as usize] = None;
        let table = match done.response {
            Ok(response) => {
                let table = feed.parse(response.status, &response.body, now);
                if table.is_none() {
                    log::warn(&format!(
                        "{} answered status {} ({} bytes) with no rates calc can read",
                        feed.key(),
                        response.status,
                        response.body.len()
                    ));
                }
                table
            }
            Err(e) => {
                log::warn(&format!("fetching {} failed: {e}", feed.key()));
                None
            }
        };
        match table {
            Some(table) => {
                *self.table(feed) = Some(table);
                self.save(feed);
                true
            }
            None => {
                self.retry_soon(feed);
                false
            }
        }
    }

    fn table(&mut self, feed: Feed) -> &mut Option<Table> {
        match feed {
            Feed::Fiat => &mut self.rates.fiat,
            Feed::Crypto => &mut self.rates.crypto,
        }
    }

    /// Writes the table of `feed`; when there is no room or the write fails, it stays in
    /// memory, marked unsaved for the next refresh.
    fn save(&mut self, feed: Feed) {
        let Some(table) = self.table(feed).as_ref() else {
            return;
        };
        let text = encode(table);
        let saved = match sheets::with(|s| s.set_background(feed.key(), &text)) {
            Ok(saved) => saved,
            Err(e) => {
                log::warn(&format!("writing {} failed: {e}", feed.key()));
                false
            }
        };
        self.unsaved[feed as usize] = !saved;
    }
}

fn read(key: &str) -> Option<Table> {
    match storage::get(key) {
        Ok(text) => text.as_deref().and_then(decode),
        Err(e) => {
            log::warn(&format!("reading {key} failed: {e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::json::{self, Json};

    fn table(fetched: i64, pairs: &[(&'static str, f64)]) -> Table {
        Table {
            fetched,
            values: pairs.iter().copied().collect(),
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let t = table(
            1_795_099_320,
            &[
                ("USD", 1.0),
                ("EUR", 0.92),
                ("RUB", 93.125),
                ("BTC", 0.1 + 0.2),
            ],
        );
        let text = encode(&t);
        assert_eq!(
            text,
            "1795099320 BTC=0.30000000000000004 EUR=0.92 RUB=93.125 USD=1"
        );
        assert_eq!(decode(&text), Some(t));
    }

    #[test]
    fn decode_rejects_garbage() {
        for text in ["", "abc", "123", "x USD=1", "1795099320 USD=abc"] {
            assert_eq!(decode(text), None, "{text:?}");
        }
    }

    #[test]
    fn decode_skips_unknown_codes_and_bad_values() {
        assert_eq!(
            decode("1795099320 USD=1 XXX=2 EUR=-1 RUB=93.125"),
            Some(table(1_795_099_320, &[("USD", 1.0), ("RUB", 93.125)]))
        );
        assert_eq!(
            decode("1 USD=1 EUR=inf GBP=NaN JPY=0 BTC"),
            Some(table(1, &[("USD", 1.0)]))
        );
    }

    const FIAT_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/er-api-latest-usd.json");
    const CRYPTO_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/coingecko-simple-price.json");
    const NOW: i64 = 1_000_000;

    fn rates(fiat: Option<i64>, crypto: Option<i64>) -> Rates {
        Rates {
            fiat: fiat.map(|at| table(at, &[("USD", 1.0)])),
            crypto: crypto.map(|at| table(at, &[("BTC", 67_000.0)])),
        }
    }

    #[test]
    fn due_follows_the_policy() {
        let none = Rates::default();
        let idle = Due::default();
        let both = Due {
            fiat: true,
            crypto: true,
        };
        for at_init in [false, true] {
            assert_eq!(due(NOW, &none, None, at_init, idle), both, "{at_init}");
        }
        let fiat = |fetched| due(NOW, &rates(Some(fetched), Some(NOW)), None, false, idle);
        assert!(fiat(NOW - 3540).fiat);
        assert!(!fiat(NOW - 3539).fiat);
        let crypto = |fetched, used, at_init| {
            due(NOW, &rates(Some(NOW), fetched), used, at_init, idle).crypto
        };
        assert!(crypto(Some(NOW - 540), Some(NOW - 60), false));
        assert!(!crypto(Some(NOW - 539), Some(NOW - 60), false));
        assert!(!crypto(Some(NOW - 900), Some(NOW - 1801), false));
        assert!(crypto(Some(NOW - 900), Some(NOW - 1799), false));
        assert!(!crypto(Some(NOW - 900), None, false));
        assert!(crypto(None, Some(NOW - 10), false));
        assert!(crypto(Some(NOW - 900), None, true));
        assert!(!crypto(Some(NOW - 100), None, true));
        let in_flight = Due {
            fiat: true,
            crypto: false,
        };
        assert_eq!(
            due(NOW, &none, None, false, in_flight),
            Due {
                fiat: false,
                crypto: true
            }
        );
        assert_eq!(due(NOW, &none, None, true, both), idle);
        // A table fetched after `now` (the clock was ahead, or the stored value is bad) is
        // due as if old; crypto still only when wanted. Up to the slack ahead it is fresh.
        assert!(fiat(NOW + 61).fiat);
        assert!(!fiat(NOW + 60).fiat);
        assert!(crypto(Some(NOW + 3600), Some(NOW - 60), false));
        assert!(crypto(Some(NOW + 3600), None, true));
        assert!(!crypto(Some(NOW + 3600), None, false));
        assert!(!crypto(Some(NOW + 60), Some(NOW - 60), false));
    }

    #[test]
    fn the_crypto_url_names_every_id_in_table_order() {
        assert_eq!(
            crypto_url(),
            format!(
                "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
                CRYPTO
                    .iter()
                    .map(|(_, id)| *id)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        );
        assert!(crypto_url().contains("?ids=bitcoin,ethereum,tether,"));
    }

    #[test]
    fn parses_the_er_api_fixture() {
        let table = parse_fiat(200, FIAT_FIXTURE, NOW).expect("the fixture parses");
        assert_eq!(table.values["USD"], 1.0);
        assert_eq!(table.values.len(), FIAT.len());
        assert_eq!(table.fetched, NOW);
        assert!(table.values.values().all(|v| v.is_finite() && *v > 0.0));
    }

    #[test]
    fn fiat_codes_match_the_fixture() {
        let body = std::str::from_utf8(FIAT_FIXTURE).unwrap();
        let Some(Json::Obj(members)) = json::parse(body).and_then(|j| j.get("rates").cloned())
        else {
            panic!("the fixture holds rates");
        };
        let served: HashSet<&str> = members.iter().map(|(code, _)| code.as_str()).collect();
        let known: HashSet<&str> = FIAT.iter().copied().collect();
        assert_eq!(served, known);
    }

    #[test]
    fn rejects_bad_fiat_answers() {
        assert_eq!(parse_fiat(500, FIAT_FIXTURE, NOW), None);
        for body in [
            "<html>",
            r#"{"result":"error","error-type":"invalid-key"}"#,
            r#"{"result":"success","rates":{}}"#,
            r#"{"result":"success","rates":[1]}"#,
            r#"{"rates":{"USD":1}}"#,
        ] {
            assert_eq!(parse_fiat(200, body.as_bytes(), NOW), None, "{body}");
        }
        assert_eq!(parse_fiat(200, b"\xff{}", NOW), None);
        let body = br#"{"result":"success","rates":{"USD":1,"EUR":-1,"XXX":2,"GBP":"x","JPY":0}}"#;
        assert_eq!(
            parse_fiat(200, body, NOW),
            Some(table(NOW, &[("USD", 1.0)]))
        );
    }

    #[test]
    fn parses_the_coingecko_fixture() {
        let table = parse_crypto(200, CRYPTO_FIXTURE, NOW).expect("the fixture parses");
        for (ticker, _) in CRYPTO {
            assert!(
                table.values.get(ticker).is_some_and(|v| *v > 0.0),
                "{ticker}"
            );
        }
        assert_eq!(table.values.len(), CRYPTO.len());
        assert_eq!(table.fetched, NOW);
    }

    #[test]
    fn rejects_bad_crypto_answers() {
        assert_eq!(parse_crypto(429, CRYPTO_FIXTURE, NOW), None);
        for body in [
            "{}",
            r#"{"bitcoin":{"eur":1}}"#,
            r#"{"result":"error"}"#,
            "<html>",
        ] {
            assert_eq!(parse_crypto(200, body.as_bytes(), NOW), None, "{body}");
        }
        let body = br#"{"bitcoin":{"usd":67000},"dogecoin":{"usd":-1},"nope":{"usd":2}}"#;
        assert_eq!(
            parse_crypto(200, body, NOW),
            Some(table(NOW, &[("BTC", 67_000.0)]))
        );
    }

    /// `body` with spaces after it, `len` bytes in all: still valid JSON.
    fn padded(body: &[u8], len: usize) -> Vec<u8> {
        let mut out = body.to_vec();
        out.resize(len, b' ');
        out
    }

    #[test]
    fn answers_over_8_kib_are_refused() {
        assert!(FIAT_FIXTURE.len() <= MAX_ANSWER_BYTES && CRYPTO_FIXTURE.len() <= MAX_ANSWER_BYTES);
        for len in [MAX_ANSWER_BYTES, MAX_ANSWER_BYTES + 1] {
            let fits = len <= MAX_ANSWER_BYTES;
            let fiat = padded(FIAT_FIXTURE, len);
            let crypto = padded(CRYPTO_FIXTURE, len);
            assert!(json::parse(std::str::from_utf8(&fiat).unwrap()).is_some());
            assert_eq!(
                parse_fiat(200, &fiat, NOW).is_some(),
                fits,
                "fiat, {len} bytes"
            );
            assert_eq!(
                parse_crypto(200, &crypto, NOW).is_some(),
                fits,
                "crypto, {len} bytes"
            );
        }
        assert_eq!(MAX_ANSWER_BYTES, 8 * 1024);
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "a timing test: run with --release")]
    fn parsing_rates_is_fast() {
        for (name, parse) in [
            ("er-api", parse_fiat as fn(u16, &[u8], i64) -> Option<Table>),
            ("coingecko", parse_crypto),
        ] {
            let body = if name == "er-api" {
                FIAT_FIXTURE
            } else {
                CRYPTO_FIXTURE
            };
            let best = (0..5)
                .map(|_| {
                    let start = std::time::Instant::now();
                    assert!(parse(200, body, NOW).is_some());
                    start.elapsed()
                })
                .min()
                .unwrap();
            eprintln!("parsing the {name} fixture: {best:?}");
            assert!(
                best < std::time::Duration::from_millis(5),
                "{name}: {best:?}"
            );
        }
    }

    #[test]
    fn a_failed_fetch_of_a_missing_feed_asks_for_one_retry() {
        let mut feeds = Feeds::default();
        // No table: the first failure asks for a retry, a second one while it waits does not.
        assert!(feeds.wants_retry(Feed::Fiat));
        assert!(feeds.wants_retry(Feed::Crypto));
        feeds.retry = Some(7);
        assert!(!feeds.wants_retry(Feed::Fiat));
        assert!(!feeds.wants_retry(Feed::Crypto));
        // Its firing clears it, and any other timer (the tick) does not.
        assert!(!feeds.retry_fired(3));
        assert_eq!(feeds.retry, Some(7));
        assert!(feeds.retry_fired(7));
        assert_eq!(feeds.retry, None);
        // A feed calc holds a table of waits for the tick instead.
        feeds.rates = rates(Some(NOW), None);
        assert!(!feeds.wants_retry(Feed::Fiat));
        assert!(feeds.wants_retry(Feed::Crypto));
        assert_eq!(RETRY_MS, 60_000);
        assert_eq!(TICK_MS, 600_000);
    }

    #[test]
    fn marking_use_is_memory_only() {
        let mut feeds = Feeds::default();
        feeds.mark_used(
            Used {
                crypto: true,
                ..Used::default()
            },
            100,
        );
        assert_eq!(feeds.crypto_used_at, Some(100));
        feeds.mark_used(
            Used {
                fiat: true,
                ..Used::default()
            },
            200,
        );
        assert_eq!(feeds.crypto_used_at, Some(100));
    }
}
