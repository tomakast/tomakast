//! The epoch ticker that enforces plugin call deadlines.
//!
//! Every engine registered with an [`Epoch`] gets `increment_epoch` once per [`TICK`] while at
//! least one plugin call is in flight. With no call in flight the ticker thread is parked on a
//! condvar, so it costs nothing in the background.
//!
//! Ticks are at least one [`TICK`] apart: the thread sleeps a full tick before each increment
//! and never catches up on missed ticks (not after a park, not after being descheduled). A
//! deadline can therefore run late under load, but never ends before the time it was given.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use wasmtime::{AsContextMut, Engine, EngineWeak};

/// The ticker period.
pub const TICK: Duration = Duration::from_millis(5);

/// Ticks a call with `deadline` may run for: `ceil(deadline / TICK) + 1`. The extra tick covers
/// the tick that may land right after the call starts.
pub fn deadline_ticks(deadline: Duration) -> u64 {
    let ticks = deadline.as_nanos().div_ceil(TICK.as_nanos()) + 1;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

pub struct Epoch {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
    /// Increments issued so far; observable for tests.
    ticks: AtomicU64,
}

struct State {
    in_flight: usize,
    engines: Vec<EngineWeak>,
    shutdown: bool,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Epoch {
    /// The daemon's ticker. [`crate::HostEngine::new`] registers its engine here.
    pub fn global() -> &'static Epoch {
        static GLOBAL: OnceLock<Epoch> = OnceLock::new();
        GLOBAL.get_or_init(Epoch::new)
    }

    /// A ticker with its own thread, parked until the first [`Epoch::enter_store`]. The daemon uses
    /// [`Epoch::global`]; tests make their own so that other tests cannot keep it running.
    pub fn new() -> Epoch {
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                in_flight: 0,
                engines: Vec::new(),
                shutdown: false,
            }),
            wake: Condvar::new(),
            ticks: AtomicU64::new(0),
        });
        let thread_inner = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || run(&thread_inner))
            .expect("spawn the plugin epoch ticker");
        Epoch { inner }
    }

    /// Adds `engine` to the engines this ticker increments, once: registering the same engine
    /// again does nothing (twice would halve every deadline). Dropped engines are forgotten.
    pub fn register(&self, engine: &Engine) {
        let mut state = self.inner.lock();
        let known = state
            .engines
            .iter()
            .any(|weak| weak.upgrade().is_some_and(|e| Engine::same(&e, engine)));
        if !known {
            state.engines.push(engine.weak());
        }
    }

    /// Starts a guest call on `store` with `deadline`: marks the call as in flight (waking the
    /// ticker) and sets the store's epoch deadline to `ceil(deadline / TICK) + 1` ticks from
    /// the current epoch. Keep the guard alive for the whole call; dropping it lets the ticker
    /// park once no call is left. This is the only way to arm a store, so a call cannot run
    /// with a stale deadline.
    #[must_use = "the deadline is enforced only while the guard is alive"]
    pub fn enter_store(&self, mut store: impl AsContextMut, deadline: Duration) -> EpochGuard<'_> {
        let guard = self.enter(deadline);
        store.as_context_mut().set_epoch_deadline(guard.ticks);
        guard
    }

    fn enter(&self, deadline: Duration) -> EpochGuard<'_> {
        let mut state = self.inner.lock();
        state.in_flight += 1;
        if state.in_flight == 1 {
            self.inner.wake.notify_all();
        }
        EpochGuard {
            epoch: self,
            ticks: deadline_ticks(deadline),
        }
    }

    /// The number of ticks issued since this ticker started.
    pub fn ticks(&self) -> u64 {
        self.inner.ticks.load(Ordering::SeqCst)
    }
}

impl Default for Epoch {
    fn default() -> Self {
        Epoch::new()
    }
}

impl Drop for Epoch {
    fn drop(&mut self) {
        self.inner.lock().shutdown = true;
        self.inner.wake.notify_all();
    }
}

/// One call in flight, from [`Epoch::enter_store`]. Dropping it lets the ticker park once no
/// call is left.
#[must_use = "the deadline is enforced only while the guard is alive"]
pub struct EpochGuard<'a> {
    epoch: &'a Epoch,
    ticks: u64,
}

impl EpochGuard<'_> {
    /// The store deadline, in ticks from the current epoch.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
}

impl Drop for EpochGuard<'_> {
    fn drop(&mut self) {
        self.epoch.inner.lock().in_flight -= 1;
    }
}

fn run(inner: &Inner) {
    loop {
        {
            let mut state = inner.lock();
            while state.in_flight == 0 && !state.shutdown {
                state = inner
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.shutdown {
                return;
            }
        }
        // A fresh period starts at every wake: the first tick comes a full TICK later.
        loop {
            std::thread::sleep(TICK);
            let mut state = inner.lock();
            if state.shutdown {
                return;
            }
            state.engines.retain(|weak| match weak.upgrade() {
                Some(engine) => {
                    engine.increment_epoch();
                    true
                }
                None => false,
            });
            inner.ticks.fetch_add(1, Ordering::SeqCst);
            if state.in_flight == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn deadline_ticks_round_up_plus_one() {
        assert_eq!(deadline_ticks(Duration::from_millis(100)), 21);
        assert_eq!(deadline_ticks(Duration::from_millis(101)), 22);
        assert_eq!(deadline_ticks(Duration::from_millis(2000)), 401);
        assert_eq!(deadline_ticks(Duration::from_millis(1)), 2);
        assert_eq!(deadline_ticks(Duration::ZERO), 1);
        assert_eq!(deadline_ticks(Duration::MAX), u64::MAX);
    }

    #[test]
    fn epoch_ticker_parks_when_idle() {
        let epoch = Epoch::new();
        let engine = Engine::default();
        epoch.register(&engine);

        let guard = epoch.enter(Duration::from_secs(1));
        sleep(Duration::from_millis(50));
        let running = epoch.ticks();
        assert!(
            running >= 5,
            "ticker ran while a call was in flight: {running} ticks"
        );
        drop(guard);

        // The ticker notices within one tick that nothing is in flight, then parks: wait (at
        // most ~500 ms) until the count has been still for four ticks.
        let mut parked = epoch.ticks();
        let mut settled = false;
        for _ in 0..25 {
            sleep(TICK * 4);
            let now = epoch.ticks();
            if now == parked {
                settled = true;
                break;
            }
            parked = now;
        }
        assert!(settled, "ticker never parked: {parked} ticks and counting");
        sleep(Duration::from_millis(200));
        assert_eq!(epoch.ticks(), parked, "no tick while idle");

        // And it wakes for the next call.
        let _guard = epoch.enter(Duration::from_secs(1));
        sleep(Duration::from_millis(50));
        assert!(epoch.ticks() > parked);
    }

    #[test]
    fn register_is_idempotent_per_engine() {
        let epoch = Epoch::new();
        let a = Engine::default();
        epoch.register(&a);
        epoch.register(&a.clone());
        assert_eq!(epoch.inner.lock().engines.len(), 1);
        let b = Engine::default();
        epoch.register(&b);
        assert_eq!(epoch.inner.lock().engines.len(), 2);
    }
}
