//! The background budget of one plugin (Ruling AK): what the host starts on the plugin's
//! behalf (`on-event` for a timer or an HTTP answer) may use at most [`BACKGROUND_BUDGET`] per
//! [`BUDGET_WINDOW`] of time spent in the call (including host imports). Over budget, the
//! plugin's timers are suspended for [`SUSPENSION`]; the [`MAX_SUSPENSIONS`]th suspension
//! within [`SUSPENSION_WINDOW`] stops the plugin instead.
//!
//! The struct only counts; [`crate::WasmPlugin`] measures the calls and acts on the verdict.
//! Time is passed in, so the rules are tested without waiting.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Time spent in host-started calls (host imports included) per [`BUDGET_WINDOW`].
pub const BACKGROUND_BUDGET: Duration = Duration::from_secs(1);
pub const BUDGET_WINDOW: Duration = Duration::from_secs(10);
/// How long an over-budget plugin's timers are suspended.
pub const SUSPENSION: Duration = Duration::from_secs(60);
/// The suspension that stops the plugin instead, counted within [`SUSPENSION_WINDOW`].
pub const MAX_SUSPENSIONS: usize = 3;
pub const SUSPENSION_WINDOW: Duration = Duration::from_secs(600);

/// What to do after a host-started call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Within budget.
    Ok,
    /// Over budget: suspend the plugin's timers for [`SUSPENSION`].
    Suspend,
    /// Over budget for the [`MAX_SUSPENSIONS`]th time within [`SUSPENSION_WINDOW`]: stop it.
    Stop,
}

#[derive(Debug, Default)]
pub struct Budget {
    /// Host-started calls in the last [`BUDGET_WINDOW`]: when each ended and how long it ran.
    spent: VecDeque<(Instant, Duration)>,
    /// Suspensions in the last [`SUSPENSION_WINDOW`].
    suspensions: VecDeque<Instant>,
}

impl Budget {
    /// Counts a host-started call that ended at `now` after running for `took`.
    pub fn charge(&mut self, now: Instant, took: Duration) -> Verdict {
        self.spent.push_back((now, took));
        while self
            .spent
            .front()
            .is_some_and(|&(at, _)| now.saturating_duration_since(at) >= BUDGET_WINDOW)
        {
            self.spent.pop_front();
        }
        let total: Duration = self.spent.iter().map(|&(_, took)| took).sum();
        if total <= BACKGROUND_BUDGET {
            return Verdict::Ok;
        }
        // The time that led to this suspension is paid for: start counting afresh.
        self.spent.clear();
        while self
            .suspensions
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) >= SUSPENSION_WINDOW)
        {
            self.suspensions.pop_front();
        }
        self.suspensions.push_back(now);
        if self.suspensions.len() >= MAX_SUSPENSIONS {
            Verdict::Stop
        } else {
            Verdict::Suspend
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn within_one_second_per_ten_is_fine() {
        let mut budget = Budget::default();
        let start = Instant::now();
        // 10 calls of 100 ms in 10 s: exactly the budget.
        for i in 0..10 {
            assert_eq!(budget.charge(start + i * 900 * MS, 100 * MS), Verdict::Ok);
        }
        // Old calls leave the window: a steady 100 ms per second runs forever.
        for i in 10..100 {
            assert_eq!(
                budget.charge(start + i * 1000 * MS, 100 * MS),
                Verdict::Ok,
                "{i}"
            );
        }
    }

    #[test]
    fn over_budget_suspends_then_the_third_time_stops() {
        let mut budget = Budget::default();
        let start = Instant::now();
        assert_eq!(budget.charge(start, 600 * MS), Verdict::Ok);
        assert_eq!(budget.charge(start + 100 * MS, 401 * MS), Verdict::Suspend);
        // Counting starts afresh after a suspension.
        let later = start + SUSPENSION;
        assert_eq!(budget.charge(later, 900 * MS), Verdict::Ok);
        assert_eq!(budget.charge(later + MS, 200 * MS), Verdict::Suspend);
        let later = later + SUSPENSION;
        assert_eq!(budget.charge(later, 1001 * MS), Verdict::Stop);
    }

    #[test]
    fn suspensions_older_than_ten_minutes_are_forgotten() {
        let mut budget = Budget::default();
        let start = Instant::now();
        assert_eq!(budget.charge(start, 2000 * MS), Verdict::Suspend);
        assert_eq!(
            budget.charge(start + SUSPENSION, 2000 * MS),
            Verdict::Suspend
        );
        // The first has left the window: this is the second in 10 minutes, not the third.
        let after = start + SUSPENSION_WINDOW;
        assert_eq!(budget.charge(after, 2000 * MS), Verdict::Suspend);
        assert_eq!(
            budget.charge(after + SUSPENSION / 2, 2000 * MS),
            Verdict::Stop
        );
    }
}
