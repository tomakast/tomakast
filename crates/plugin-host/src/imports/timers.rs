//! `timers`: one host thread for all plugins fires their timers into each plugin's [`Inbox`].
//! With no timer due the thread waits on a condvar (parked); with timers it sleeps until the
//! earliest is due.
//!
//! A plugin has at most [`MAX_TIMERS`] active timers. A one-shot timer stays active until its
//! `timer-fired` event is delivered to the plugin (or it is cancelled), so the events waiting
//! for a busy plugin are bounded. A repeating timer has at most one event waiting: while one
//! is queued, its next firings are skipped. A plugin's timers die with its [`Timers`] handle,
//! which the plugin's store owns: a broken or unloaded plugin leaves no timer behind.
//!
//! [`Timers::suspend`] moves every timer of a plugin to the end of a suspension (Ruling AK):
//! nothing of it is due, so the thread does not wake for it, and no event of it is queued.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use super::{HostState, Inbox};
use crate::bindings::tomakast::plugin::timers::Host;
use crate::bindings::tomakast::plugin::types::Event;

/// Active timers one plugin may hold.
pub const MAX_TIMERS: usize = 16;
/// Shortest interval of a repeating timer.
pub const MIN_REPEAT: Duration = Duration::from_millis(250);
/// Shortest delay of a one-shot timer.
pub const MIN_DELAY: Duration = Duration::from_millis(10);

const DENIED: &str = "the timers permission was not granted";

/// A timer of one plugin store: `(owner, timer id)`.
type Key = (u64, u64);

struct Timer {
    inbox: Arc<Inbox>,
    /// The interval of a repeating timer.
    repeat: Option<Duration>,
    /// When it fires next; `None` once a one-shot timer fired, or when the time does not fit
    /// in an `Instant` (it never comes due).
    due: Option<Instant>,
    /// Its `timer-fired` event is queued and not delivered yet.
    pending: bool,
}

#[derive(Default)]
struct State {
    timers: HashMap<Key, Timer>,
    /// Due times; an entry whose time is no longer its timer's `due` is stale and skipped.
    queue: BinaryHeap<Reverse<(Instant, Key)>>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Times the thread woke up; observable for tests.
    wakeups: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The timer thread. The daemon uses [`TimerService::global`]; tests make their own.
pub(crate) struct TimerService {
    shared: Arc<Shared>,
}

impl TimerService {
    pub(crate) fn global() -> &'static TimerService {
        static GLOBAL: OnceLock<TimerService> = OnceLock::new();
        GLOBAL.get_or_init(TimerService::new)
    }

    pub(crate) fn new() -> TimerService {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            wakeups: AtomicU64::new(0),
        });
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("plugin-timers".into())
            .spawn(move || run(&thread_shared))
            .expect("spawn the plugin timer thread");
        TimerService { shared }
    }

    /// The timers of one new plugin store, delivered to `inbox`.
    pub(crate) fn timers(&self, inbox: Arc<Inbox>) -> Timers {
        static OWNERS: AtomicU64 = AtomicU64::new(1);
        Timers {
            shared: Arc::clone(&self.shared),
            owner: OWNERS.fetch_add(1, Ordering::Relaxed),
            next_id: 1,
            inbox,
            suspended_until: None,
        }
    }

    #[cfg(test)]
    fn wakeups(&self) -> u64 {
        self.shared.wakeups.load(Ordering::SeqCst)
    }
}

impl Drop for TimerService {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.wake.notify_all();
    }
}

/// The timers of one plugin store. Dropping it cancels them all.
pub(crate) struct Timers {
    shared: Arc<Shared>,
    owner: u64,
    next_id: u64,
    inbox: Arc<Inbox>,
    /// While suspended, no timer of this store comes due before this time.
    suspended_until: Option<Instant>,
}

impl Timers {
    /// Starts a timer that fires after `millis`, every `millis` when `repeat`.
    pub(crate) fn start(&mut self, millis: u64, repeat: bool) -> Result<u64, String> {
        let delay = Duration::from_millis(millis);
        if repeat && delay < MIN_REPEAT {
            return Err(format!(
                "a repeating timer's interval is at least {} ms",
                MIN_REPEAT.as_millis()
            ));
        }
        if !repeat && delay < MIN_DELAY {
            return Err(format!(
                "a one-shot timer's delay is at least {} ms",
                MIN_DELAY.as_millis()
            ));
        }
        let mut state = self.shared.lock();
        let active = state.timers.keys().filter(|k| k.0 == self.owner).count();
        if active >= MAX_TIMERS {
            return Err(format!("at most {MAX_TIMERS} timers may be active"));
        }
        let id = self.next_id;
        self.next_id += 1;
        let key = (self.owner, id);
        let now = Instant::now();
        let due = now
            .checked_add(delay)
            .map(|due| due.max(self.suspended_until.unwrap_or(due)));
        state.timers.insert(
            key,
            Timer {
                inbox: Arc::clone(&self.inbox),
                repeat: repeat.then_some(delay),
                due,
                pending: false,
            },
        );
        if let Some(due) = due {
            state.queue.push(Reverse((due, key)));
            self.shared.wake.notify_all();
        }
        Ok(id)
    }

    /// Suspends every timer of this store until `until` (Ruling AK): none comes due before,
    /// a timer started meanwhile included, and the queued `timer-fired` events are dropped (a
    /// dropped one-shot fires at `until` instead). The thread re-plans and does not wake for
    /// this store until then.
    pub(crate) fn suspend(&mut self, until: Instant) {
        self.suspended_until = Some(until);
        let mut dropped = Vec::new();
        {
            let mut state = self.shared.lock();
            for (key, timer) in state.timers.iter_mut().filter(|(k, _)| k.0 == self.owner) {
                let was_pending = std::mem::take(&mut timer.pending);
                if was_pending {
                    dropped.push(key.1);
                }
                timer.due = match timer.due {
                    Some(due) => Some(due.max(until)),
                    // A fired one-shot whose event is dropped fires again, at `until`.
                    None if was_pending => Some(until),
                    None => None,
                };
            }
            // Without the old entries: a stale one would still wake the thread.
            rebuild(&mut state);
        }
        for id in dropped {
            self.inbox.remove_timer(id);
        }
        self.shared.wake.notify_all();
    }

    /// Cancels timer `id` of this store and drops its queued event; unknown ids are ignored.
    pub(crate) fn cancel(&mut self, id: u64) {
        let removed = {
            let mut state = self.shared.lock();
            let removed = state.timers.remove(&(self.owner, id)).is_some();
            compact(&mut state);
            removed
        };
        if removed {
            self.inbox.remove_timer(id);
        }
    }

    /// Timer `id`'s queued event is about to be delivered: a one-shot timer ends, a repeating
    /// one may queue its next event. Returns whether to deliver it: not when the timer was
    /// cancelled after its event was taken from the inbox (by an earlier event of the same
    /// wake-up).
    pub(crate) fn delivered(&mut self, id: u64) -> bool {
        let mut state = self.shared.lock();
        let key = (self.owner, id);
        let Some(timer) = state.timers.get_mut(&key) else {
            return false;
        };
        if !timer.pending {
            return false;
        }
        timer.pending = false;
        if timer.repeat.is_none() {
            state.timers.remove(&key);
        }
        true
    }
}

impl Drop for Timers {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.timers.retain(|key, _| key.0 != self.owner);
        rebuild(&mut state);
        // The thread may sleep towards one of the removed timers: let it re-plan (and park).
        self.shared.wake.notify_all();
    }
}

/// Rebuilds the queue once stale entries outnumber live ones, so that starting and cancelling
/// timers in a loop cannot grow it without bound; empties it with the last timer, so that the
/// thread parks without waking for a cancelled one.
fn compact(state: &mut State) {
    if state.timers.is_empty() {
        state.queue.clear();
    } else if state.queue.len() > 2 * state.timers.len() + 64 {
        rebuild(state);
    }
}

fn rebuild(state: &mut State) {
    state.queue = state
        .timers
        .iter()
        .filter_map(|(key, timer)| Some(Reverse((timer.due?, *key))))
        .collect();
}

fn run(shared: &Shared) {
    let mut state = shared.lock();
    loop {
        if state.shutdown {
            return;
        }
        let now = Instant::now();
        while let Some(&Reverse((at, key))) = state.queue.peek() {
            if at > now {
                break;
            }
            state.queue.pop();
            fire(&mut state, key, at, now);
        }
        state = match state.queue.peek() {
            None => shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
            Some(&Reverse((at, _))) => {
                shared
                    .wake
                    .wait_timeout(state, at.saturating_duration_since(now))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        };
        shared.wakeups.fetch_add(1, Ordering::SeqCst);
    }
}

/// Fires the timer `key` due at `at`, unless that queue entry is stale.
fn fire(state: &mut State, key: Key, at: Instant, now: Instant) {
    let Some(timer) = state.timers.get_mut(&key) else {
        return;
    };
    if timer.due != Some(at) {
        return;
    }
    if !timer.pending {
        timer.pending = true;
        timer.inbox.push(Event::TimerFired(key.1));
    }
    timer.due = timer.repeat.and_then(|interval| {
        let next = at.checked_add(interval)?;
        // Behind schedule (a busy machine): skip the missed firings instead of bursting.
        if next > now {
            Some(next)
        } else {
            now.checked_add(interval)
        }
    });
    if let Some(due) = timer.due {
        state.queue.push(Reverse((due, key)));
    }
}

impl Host for HostState {
    fn start(&mut self, millis: u64, repeat: bool) -> Result<u64, String> {
        self.record_import("timers.start");
        if !self.permissions.timers {
            return Err(DENIED.into());
        }
        self.timers.start(millis, repeat)
    }

    fn cancel(&mut self, timer: u64) {
        self.record_import("timers.cancel");
        if self.permissions.timers {
            self.timers.cancel(timer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn fired(inbox: &Inbox) -> Vec<u64> {
        inbox
            .take()
            .into_iter()
            .filter_map(|e| match e {
                Event::TimerFired(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// Waits up to 10 s (a loaded machine) until `inbox` holds an event.
    fn wait_for_event(inbox: &Inbox) {
        for _ in 0..2000 {
            if inbox.len() > 0 {
                return;
            }
            sleep(Duration::from_millis(5));
        }
        panic!("no timer fired");
    }

    #[test]
    fn timer_thread_parks_when_no_timers() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        // Idle from the start: no wake-up at all.
        sleep(Duration::from_millis(200));
        assert_eq!(service.wakeups(), 0, "the thread ran with no timer");

        let mut timers = service.timers(Arc::clone(&inbox));
        let id = timers.start(20, false).unwrap();
        wait_for_event(&inbox);
        assert_eq!(fired(&inbox), [id]);
        assert!(timers.delivered(id));

        // Nothing is due any more: the thread parks.
        sleep(Duration::from_millis(50));
        let parked = service.wakeups();
        sleep(Duration::from_millis(300));
        assert_eq!(
            service.wakeups(),
            parked,
            "the thread ran after the last timer"
        );

        // A repeating timer keeps it running; dropping the plugin's timers parks it again.
        timers.start(250, true).unwrap();
        sleep(Duration::from_millis(600));
        assert!(service.wakeups() > parked);
        drop(timers);
        sleep(Duration::from_millis(50));
        let parked = service.wakeups();
        sleep(Duration::from_millis(600));
        assert_eq!(
            service.wakeups(),
            parked,
            "a dropped plugin's timer kept running"
        );
    }

    #[test]
    fn timers_die_with_their_owner() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut a = service.timers(Arc::clone(&inbox));
        let mut b = service.timers(Arc::clone(&inbox));
        a.start(60_000, true).unwrap();
        a.start(60_000, false).unwrap();
        let kept = b.start(60_000, false).unwrap();
        drop(a);
        let state = service.shared.lock();
        let left: Vec<Key> = state.timers.keys().copied().collect();
        assert_eq!(left, [(b.owner, kept)]);
    }

    #[test]
    fn owners_cannot_cancel_each_other() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut a = service.timers(Arc::clone(&inbox));
        let mut b = service.timers(Arc::clone(&inbox));
        let id = a.start(60_000, false).unwrap();
        b.cancel(id);
        assert_eq!(service.shared.lock().timers.len(), 1);
        a.cancel(id);
        assert_eq!(service.shared.lock().timers.len(), 0);
    }

    #[test]
    fn a_repeating_timer_queues_one_event_until_delivered() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut timers = service.timers(Arc::clone(&inbox));
        let id = timers.start(250, true).unwrap();
        // Five intervals pass while the plugin is busy: one event waits.
        sleep(Duration::from_millis(1300));
        assert_eq!(fired(&inbox), [id]);
        sleep(Duration::from_millis(300));
        assert!(fired(&inbox).is_empty(), "queued again before delivery");
        assert!(timers.delivered(id));
        wait_for_event(&inbox);
        assert_eq!(fired(&inbox), [id]);
    }

    /// An event already taken from the inbox when its timer is cancelled is not delivered.
    #[test]
    fn a_cancelled_timer_is_not_delivered() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut timers = service.timers(Arc::clone(&inbox));
        let id = timers.start(10, false).unwrap();
        wait_for_event(&inbox);
        assert_eq!(fired(&inbox), [id]);
        timers.cancel(id);
        assert!(!timers.delivered(id));
        assert!(!timers.delivered(99), "an unknown id");
    }

    #[test]
    fn cancel_drops_the_queued_event() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut timers = service.timers(Arc::clone(&inbox));
        let id = timers.start(10, false).unwrap();
        wait_for_event(&inbox);
        timers.cancel(id);
        assert_eq!(inbox.len(), 0);
    }

    /// Ruling AK: a suspended plugin's timers are not due before the end of the suspension,
    /// its queued events are dropped, and the thread does not wake for it; another plugin's
    /// timers run on.
    #[test]
    fn suspended_timers_do_not_wake_the_thread() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut timers = service.timers(Arc::clone(&inbox));
        let repeating = timers.start(250, true).unwrap();
        let once = timers.start(10, false).unwrap();
        wait_for_event(&inbox);
        sleep(Duration::from_millis(300));
        timers.suspend(Instant::now() + Duration::from_secs(3600));
        assert_eq!(inbox.len(), 0, "queued events are dropped");
        // A timer started while suspended waits too.
        let started = timers.start(10, false).unwrap();
        sleep(Duration::from_millis(50));
        let parked = service.wakeups();
        sleep(Duration::from_millis(700));
        assert_eq!(
            service.wakeups(),
            parked,
            "the thread woke for a suspended plugin"
        );
        assert_eq!(inbox.len(), 0);
        {
            let state = service.shared.lock();
            let horizon = Instant::now() + Duration::from_secs(3000);
            for id in [repeating, once, started] {
                let due = state.timers[&(timers.owner, id)].due;
                assert!(due.is_some_and(|due| due > horizon), "timer {id}: {due:?}");
            }
        }
        // Another plugin is not affected.
        let other_inbox = Arc::new(Inbox::default());
        let mut other = service.timers(Arc::clone(&other_inbox));
        let id = other.start(10, false).unwrap();
        wait_for_event(&other_inbox);
        assert_eq!(fired(&other_inbox), [id]);
    }

    #[test]
    fn start_and_cancel_in_a_loop_keeps_the_queue_small() {
        let service = TimerService::new();
        let inbox = Arc::new(Inbox::default());
        let mut timers = service.timers(inbox);
        for _ in 0..10_000 {
            let id = timers.start(3_600_000, false).unwrap();
            timers.cancel(id);
        }
        assert!(service.shared.lock().queue.len() <= 64 + 1);
    }
}
