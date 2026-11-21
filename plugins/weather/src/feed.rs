//! One request kind of one place (its forecast, or its air): the value last fetched, the
//! request out, and when to ask again. Pure, so it is tested natively.
//!
//! - A value is fetched when there is none, when it is [`REFRESH_SECS`] old (less half a
//!   minute, so a timer a little early still fetches), or when it is from the future.
//! - One request at a time; the answer goes to [`Feed::answered`].
//! - A failure (no network, a timeout, a bad answer, a request that cannot start) keeps the
//!   last value and waits before the next try: 1, 2, 4, 8 and then 15 minutes. The user's
//!   refresh skips the wait.

use crate::api::{Air, Reading};

/// A value this old is fetched again.
pub const REFRESH_SECS: i64 = 15 * 60;
/// How much sooner than [`REFRESH_SECS`] a value is due.
const SLACK_SECS: i64 = 30;
/// The wait after the first failure in a row; it doubles up to [`MAX_BACKOFF_SECS`].
const FIRST_BACKOFF_SECS: i64 = 60;
const MAX_BACKOFF_SECS: i64 = REFRESH_SECS;

/// A value that knows when it was fetched (unix seconds).
pub trait Fetched {
    fn fetched(&self) -> i64;
}

impl Fetched for Reading {
    fn fetched(&self) -> i64 {
        self.fetched
    }
}

impl Fetched for Air {
    fn fetched(&self) -> i64 {
        self.fetched
    }
}

/// See the module doc.
#[derive(Debug)]
pub struct Feed<T> {
    /// The value last fetched, kept through failures.
    pub value: Option<T>,
    /// The request waiting for its answer.
    pending: Option<u64>,
    /// Failures in a row.
    failures: u32,
    /// No automatic fetch before this time (unix seconds).
    retry_at: Option<i64>,
}

impl<T> Default for Feed<T> {
    fn default() -> Self {
        Feed {
            value: None,
            pending: None,
            failures: 0,
            retry_at: None,
        }
    }
}

impl<T: Fetched + PartialEq> Feed<T> {
    /// Whether to fetch at `now`: no request is out, and `forced` (the user asked), or the
    /// wait after a failure is over and the value is missing, old or from the future.
    pub fn due(&self, now: i64, forced: bool) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let waits = self.retry_at.is_some_and(|at| now < at);
        let stale = self
            .value
            .as_ref()
            .is_none_or(|v| now - v.fetched() >= REFRESH_SECS - SLACK_SECS || v.fetched() > now);
        forced || (!waits && stale)
    }

    /// The fetch was started as request `id`.
    pub fn started(&mut self, id: u64) {
        self.pending = Some(id);
    }

    /// Whether request `id` is the one this waits for.
    pub fn waits_for(&self, id: u64) -> bool {
        self.pending == Some(id)
    }

    /// Whether a request is out.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The answer at `now`: the value read, or `None` for a failure or a bad answer. Returns
    /// whether what the tiles show changed.
    pub fn answered(&mut self, value: Option<T>, now: i64) -> bool {
        self.pending = None;
        match value {
            Some(value) => {
                let changed = self.failed() || self.value.as_ref() != Some(&value);
                self.value = Some(value);
                self.failures = 0;
                self.retry_at = None;
                changed
            }
            None => {
                let was = self.failed();
                self.fail(now);
                !was
            }
        }
    }

    /// Starting the fetch failed at `now`.
    pub fn start_failed(&mut self, now: i64) {
        self.pending = None;
        self.fail(now);
    }

    /// Whether the last try failed.
    pub fn failed(&self) -> bool {
        self.failures > 0
    }

    /// How many failures in a row there were.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    fn fail(&mut self, now: i64) {
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Some(now + backoff(self.failures));
    }
}

/// How long to wait after `failures` failures in a row: 1, 2, 4, 8 and then 15 minutes.
pub fn backoff(failures: u32) -> i64 {
    FIRST_BACKOFF_SECS
        .saturating_mul(1 << failures.saturating_sub(1).min(10))
        .min(MAX_BACKOFF_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Value(i64);

    impl Fetched for Value {
        fn fetched(&self) -> i64 {
            self.0
        }
    }

    /// Starts the fetch due at `now` as request `id`; panics when none is.
    fn start(f: &mut Feed<Value>, now: i64, id: u64) {
        assert!(f.due(now, false), "a fetch is due at {now}");
        f.started(id);
    }

    #[test]
    fn a_value_is_fetched_again_when_old() {
        let mut f = Feed::default();
        start(&mut f, 0, 1);
        assert!(!f.due(0, false), "one request at a time");
        assert!(!f.due(0, true), "even when forced");
        assert!(f.waits_for(1) && !f.waits_for(2));
        assert!(f.answered(Some(Value(2)), 2));
        assert!(!f.waits_for(1));
        assert!(!f.due(2 + 60, false));
        assert!(!f.due(2 + REFRESH_SECS - 31, false));
        assert!(f.due(2 + REFRESH_SECS - 30, false));
        // The user's refresh fetches at once.
        assert!(f.due(3, true));
    }

    #[test]
    fn failures_keep_the_reading_and_back_off() {
        let mut f = Feed::default();
        start(&mut f, 0, 1);
        f.answered(Some(Value(0)), 0);
        let now = REFRESH_SECS;
        start(&mut f, now, 2);
        assert!(f.answered(None, now), "the status changed");
        assert!(f.failed());
        assert_eq!(f.value, Some(Value(0)), "the last value stays");
        // 60 s, then 120, 240, 480, 900 and 900 again.
        let mut at = now;
        for (id, wait) in (3..).zip([60, 120, 240, 480, 900, 900]) {
            assert!(!f.due(at + wait - 1, false), "waits {wait} s");
            assert!(f.due(at + wait - 1, true), "unless forced");
            at += wait;
            start(&mut f, at, id);
            // A failure on a failure changes nothing on screen.
            assert!(!f.answered(None, at));
        }
        assert_eq!(f.failures(), 7);
        // A good answer ends the run.
        start(&mut f, at + 900, 99);
        assert!(f.answered(Some(Value(at + 900)), at + 900));
        assert!(!f.failed());
        assert_eq!(f.failures(), 0);
    }

    #[test]
    fn a_failed_start_backs_off_too() {
        let mut f = Feed::<Value>::default();
        assert!(f.due(0, false));
        f.start_failed(0);
        assert!(f.failed());
        assert!(!f.due(59, false));
        assert!(f.due(60, false));
    }

    #[test]
    fn a_reading_from_the_future_is_stale() {
        let mut f = Feed::default();
        start(&mut f, 1_000_000, 1);
        f.answered(Some(Value(1_000_000)), 1_000_000);
        assert!(f.due(1_000_000 - 3600, false));
    }
}
