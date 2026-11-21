//! `location`: the computer's approximate location, for plugins with the `location`
//! permission. The answer reaches the plugin as `event.location` through its [`Inbox`].
//!
//! A [`Locator`] stands between the plugins and the [`Source`]s, which it tries in turn (see
//! `crate::location_sources` for the launcher's own: the user's city, the operating system's
//! service, Wi-Fi, the time zone). It finds the location at most once at a time, and keeps the
//! answer: a fix of [`FINE_KM`] or better for [`KEEP_FIX`], a coarser one or a failure for
//! [`KEEP_SHORT`], so plugins that ask often never reach a source more often, nor show its
//! prompt again. A source has its own time: one that outlives it is not waited for, and is not
//! asked again until its call returns. Positions are rounded to [`ROUNDING`] degrees here,
//! before any plugin sees them, and are never said to be closer than [`MIN_ACCURACY_KM`].
//!
//! Each plugin has at most one request waiting: a request while one waits is answered by the
//! same event. A plugin that asked once also hears of a change of the sources (the user's
//! settings): the kept answer is forgotten and the new one comes to it unasked. When the
//! plugin's store goes (it stopped or was unloaded), an answer still to come is discarded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use super::{HostState, Inbox};
use crate::bindings::tomakast::plugin::location::Host;
use crate::bindings::tomakast::plugin::types::{Coordinates, Event};

/// A position a source found: degrees, and how far the real place may be from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_km: u32,
}

/// What a source does: blocks until it has a position, or says why it has none.
pub type LocationSource = Arc<dyn Fn() -> Result<Fix, String> + Send + Sync>;

/// Degrees the coordinates are rounded to: 0.1° is about 11 km of latitude.
pub const ROUNDING: f64 = 0.1;
/// No answer is said to be closer than this, the rounding.
pub const MIN_ACCURACY_KM: u32 = 11;
/// An answer this accurate or better is kept for [`KEEP_FIX`].
pub const FINE_KM: u32 = 50;
/// How long a fine fix is kept.
pub const KEEP_FIX: Duration = Duration::from_secs(10 * 60);
/// How long a coarser fix, or a failure, is kept.
pub const KEEP_SHORT: Duration = Duration::from_secs(60);

const DENIED: &str = "the location permission was not granted";

type Answer = Result<Fix, String>;
/// Hands an answer to a plugin; `false` when the plugin is gone.
pub(crate) type Notify = Arc<dyn Fn(&Answer) -> bool + Send + Sync>;

/// One place to find the location: a name for the reasons, its call, and how long it may take.
#[derive(Clone)]
pub struct Source {
    name: &'static str,
    find: LocationSource,
    wait: Duration,
    /// A call is running (or outlived `wait` and has not returned yet).
    busy: Arc<AtomicBool>,
}

impl Source {
    pub fn new(name: &'static str, wait: Duration, find: LocationSource) -> Source {
        Source {
            name,
            find,
            wait,
            busy: Arc::default(),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Calls the source on its own thread and waits `wait` for it; not rounded.
    pub fn run(&self) -> Answer {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("still busy with an earlier request".into());
        }
        let (tx, rx) = mpsc::channel();
        let find = Arc::clone(&self.find);
        let busy = Arc::clone(&self.busy);
        let spawned = std::thread::Builder::new()
            .name("tomakast-location-source".into())
            .spawn(move || {
                let answer = find();
                busy.store(false, Ordering::Release);
                let _ = tx.send(answer);
            });
        if let Err(e) = spawned {
            self.busy.store(false, Ordering::Release);
            return Err(format!("cannot start: {e}"));
        }
        rx.recv_timeout(self.wait)
            .unwrap_or_else(|_| Err(format!("no answer within {} s", self.wait.as_secs())))
    }
}

/// The sources and what they last answered (see the module doc).
pub struct Locator {
    state: Mutex<State>,
}

struct State {
    sources: Arc<[Source]>,
    /// Bumped when the sources change: an answer of older sources is dropped.
    generation: u64,
    kept: Option<(Instant, Answer)>,
    /// A search is running; these wait for it.
    waiters: Option<Vec<Notify>>,
    /// The plugins that asked once, told when the sources change.
    listeners: Vec<Notify>,
}

impl Locator {
    /// A locator with one source, `find`, which may take up to 65 s.
    pub fn new(find: LocationSource) -> Arc<Locator> {
        Locator::with_sources(vec![Source::new("location", Duration::from_secs(65), find)])
    }

    pub fn with_sources(sources: Vec<Source>) -> Arc<Locator> {
        Arc::new(Locator {
            state: Mutex::new(State {
                sources: sources.into(),
                generation: 0,
                kept: None,
                waiters: None,
                listeners: Vec::new(),
            }),
        })
    }

    /// The launcher's own sources, shared by every plugin of the process; the user's settings
    /// replace them (`crate::location_sources::configure`).
    pub fn system() -> Arc<Locator> {
        static SYSTEM: OnceLock<Arc<Locator>> = OnceLock::new();
        Arc::clone(SYSTEM.get_or_init(|| {
            Locator::with_sources(crate::location_sources::sources(
                &tomakast_core::LocationSettings::default(),
            ))
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the sources: the kept answer is forgotten, a search of the old ones is not
    /// waited for, and every plugin that asked before gets the new answer.
    pub fn set_sources(self: &Arc<Self>, sources: Vec<Source>) {
        let mut state = self.lock();
        state.sources = sources.into();
        state.generation += 1;
        state.kept = None;
        // Each waiter is a listener too: they all hear the new answer below.
        state.waiters = None;
        state.listeners.retain(notify_alive);
        if state.listeners.is_empty() {
            return;
        }
        let listeners = state.listeners.clone();
        let fan_out: Notify = Arc::new(move |answer| {
            for notify in &listeners {
                notify(answer);
            }
            true
        });
        self.search(state, fan_out);
    }

    /// `notify` hears every change of the sources from now on, until it answers `false`.
    fn listen(&self, notify: Notify) {
        self.lock().listeners.push(notify);
    }

    /// Answers `notify` with the kept answer, or with the next one of the sources, which are
    /// searched unless a search is running.
    pub(crate) fn ask(self: &Arc<Self>, notify: Notify) {
        let state = self.lock();
        if let Some((at, answer)) = &state.kept
            && at.elapsed() < keep(answer)
        {
            let answer = answer.clone();
            drop(state);
            notify(&answer);
            return;
        }
        self.search(state, notify);
    }

    /// Adds `notify` to the running search, or starts one.
    fn search(self: &Arc<Self>, mut state: std::sync::MutexGuard<'_, State>, notify: Notify) {
        if let Some(waiters) = &mut state.waiters {
            waiters.push(notify);
            return;
        }
        state.waiters = Some(vec![notify]);
        let sources = Arc::clone(&state.sources);
        let generation = state.generation;
        drop(state);
        let locator = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("tomakast-location".into())
            .spawn(move || {
                let answer = find(&sources);
                locator.answered(generation, answer);
            });
        if let Err(e) = spawned {
            let generation = self.lock().generation;
            self.answered(generation, Err(format!("cannot find the location: {e}")));
        }
    }

    fn answered(&self, generation: u64, answer: Answer) {
        let waiters = {
            let mut state = self.lock();
            if state.generation != generation {
                return;
            }
            state.kept = Some((Instant::now(), answer.clone()));
            state.waiters.take().unwrap_or_default()
        };
        for notify in waiters {
            notify(&answer);
        }
        // Forget the listeners of plugins that are gone.
        self.lock().listeners.retain(|notify| notify_alive(notify));
    }
}

/// Whether the plugin of `notify` is still there (it answers an empty probe without pushing).
fn notify_alive(notify: &Notify) -> bool {
    notify(&Err(String::new()))
}

/// How long `answer` is kept.
fn keep(answer: &Answer) -> Duration {
    match answer {
        Ok(fix) if fix.accuracy_km <= FINE_KM => KEEP_FIX,
        _ => KEEP_SHORT,
    }
}

/// The first answer of `sources`, rounded, or the reasons of all.
fn find(sources: &[Source]) -> Answer {
    let mut reasons = Vec::new();
    for source in sources {
        match source.run().and_then(rounded) {
            Ok(fix) => return Ok(fix),
            Err(why) => reasons.push(format!("{}: {why}", source.name)),
        }
    }
    if reasons.is_empty() {
        return Err("there is nowhere to find the location".into());
    }
    Err(reasons.join("; "))
}

/// `fix` rounded to [`ROUNDING`], at least [`MIN_ACCURACY_KM`]; an error for a position that is
/// not one.
pub fn rounded(fix: Fix) -> Answer {
    let Fix {
        latitude,
        longitude,
        accuracy_km,
    } = fix;
    let ok = latitude.is_finite()
        && longitude.is_finite()
        && latitude.abs() <= 90.
        && longitude.abs() <= 180.;
    if !ok {
        return Err("no valid position".into());
    }
    // In tenths, then back: 52.5, not 52.50000000000001.
    let round = |v: f64| (v / ROUNDING).round() / (1. / ROUNDING);
    Ok(Fix {
        latitude: round(latitude),
        longitude: round(longitude),
        accuracy_km: accuracy_km.max(MIN_ACCURACY_KM),
    })
}

/// The location state of one plugin store.
pub(crate) struct Location {
    granted: bool,
    inbox: Arc<Inbox>,
    locator: Arc<Locator>,
    waiting: bool,
    /// Listening to the locator's changes.
    listening: bool,
    /// Cleared when the store goes: an answer still to come is discarded.
    alive: Arc<AtomicBool>,
}

impl Location {
    pub(crate) fn new(granted: bool, inbox: Arc<Inbox>) -> Location {
        Location {
            granted,
            inbox,
            locator: Locator::system(),
            waiting: false,
            listening: false,
            alive: Arc::new(AtomicBool::new(true)),
        }
    }

    pub(crate) fn set_locator(&mut self, locator: Arc<Locator>) {
        self.locator = locator;
    }

    /// Pushes a real answer to the plugin while it is there; an empty error is only a probe.
    fn notify(&self) -> Notify {
        let inbox = Arc::clone(&self.inbox);
        let alive = Arc::clone(&self.alive);
        Arc::new(move |answer: &Answer| {
            if !alive.load(Ordering::Acquire) {
                return false;
            }
            if matches!(answer, Err(why) if why.is_empty()) {
                return true;
            }
            let answer = answer.clone().map(|fix| Coordinates {
                latitude: fix.latitude,
                longitude: fix.longitude,
                accuracy_km: fix.accuracy_km,
            });
            inbox.push(Event::Location(answer));
            true
        })
    }

    fn request(&mut self) -> Result<(), String> {
        if !self.granted {
            return Err(DENIED.into());
        }
        if !self.listening {
            self.listening = true;
            self.locator.listen(self.notify());
        }
        if self.waiting {
            return Ok(());
        }
        self.waiting = true;
        self.locator.ask(self.notify());
        Ok(())
    }

    /// The event of the waiting request is being delivered.
    pub(crate) fn delivered(&mut self) {
        self.waiting = false;
    }
}

impl Drop for Location {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

impl Host for HostState {
    fn request(&mut self) -> Result<(), String> {
        self.record_import("location.request");
        self.location.request()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    fn fix(latitude: f64, longitude: f64, accuracy_km: u32) -> Fix {
        Fix {
            latitude,
            longitude,
            accuracy_km,
        }
    }

    #[test]
    fn positions_are_rounded_to_a_tenth_of_a_degree_and_at_least_11_km() {
        assert_eq!(rounded(fix(52.52437, 13.41053, 1)), Ok(fix(52.5, 13.4, 11)));
        assert_eq!(
            rounded(fix(-33.86785, 151.20732, 30)),
            Ok(fix(-33.9, 151.2, 30))
        );
        assert_eq!(rounded(fix(90., -180., 500)), Ok(fix(90., -180., 500)));
        for (lat, lon) in [(91., 0.), (0., 181.), (f64::NAN, 0.), (0., f64::INFINITY)] {
            assert!(rounded(fix(lat, lon, 11)).is_err(), "{lat} {lon}");
        }
    }

    /// A source that counts its calls, sleeps `delay`, then answers `answer`.
    fn counting(
        name: &'static str,
        wait: Duration,
        delay: Duration,
        answer: Answer,
    ) -> (Source, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let source = Source::new(
            name,
            wait,
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(delay);
                answer.clone()
            }),
        );
        (source, calls)
    }

    /// A notify that sends what it hears (not probes) to the returned receiver.
    fn collector() -> (Notify, mpsc::Receiver<Answer>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let notify: Notify = Arc::new(move |answer: &Answer| {
            if !matches!(answer, Err(why) if why.is_empty()) {
                let _ = tx.lock().unwrap().send(answer.clone());
            }
            true
        });
        (notify, rx)
    }

    /// Answers of `locator` to `n` asks, collected.
    fn ask_n(locator: &Arc<Locator>, n: usize) -> Vec<Answer> {
        let (notify, rx) = collector();
        for _ in 0..n {
            locator.ask(Arc::clone(&notify));
        }
        (0..n)
            .map(|_| rx.recv_timeout(Duration::from_secs(5)).expect("an answer"))
            .collect()
    }

    const SEC: Duration = Duration::from_secs(1);

    #[test]
    fn asks_at_the_same_time_share_one_search_and_its_answer_is_kept() {
        let (source, calls) = counting(
            "os",
            SEC,
            Duration::from_millis(50),
            Ok(fix(52.52437, 13.41053, 3)),
        );
        let locator = Locator::with_sources(vec![source]);
        assert_eq!(ask_n(&locator, 3), vec![Ok(fix(52.5, 13.4, 11)); 3]);
        assert_eq!(ask_n(&locator, 2), vec![Ok(fix(52.5, 13.4, 11)); 2]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn sources_are_tried_in_turn_and_the_first_answer_wins() {
        let (os, _) = counting("os", SEC, Duration::ZERO, Err("off".into()));
        let (zone, zone_calls) = counting("zone", SEC, Duration::ZERO, Ok(fix(55.75, 37.62, 500)));
        let (last, last_calls) = counting("last", SEC, Duration::ZERO, Ok(fix(1., 2., 11)));
        let locator = Locator::with_sources(vec![os, zone, last]);
        assert_eq!(ask_n(&locator, 1), [Ok(fix(55.8, 37.6, 500))]);
        assert_eq!(zone_calls.load(Ordering::SeqCst), 1);
        assert_eq!(last_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn with_no_answer_the_reasons_of_all_are_given_and_kept_briefly() {
        let (os, os_calls) = counting("os", SEC, Duration::ZERO, Err("off".into()));
        let (zone, _) = counting(
            "zone",
            SEC,
            Duration::ZERO,
            Err("UTC names no place".into()),
        );
        let locator = Locator::with_sources(vec![os, zone]);
        let reasons = Err("os: off; zone: UTC names no place".to_string());
        assert_eq!(ask_n(&locator, 1), std::slice::from_ref(&reasons));
        assert_eq!(ask_n(&locator, 1), [reasons]);
        assert_eq!(os_calls.load(Ordering::SeqCst), 1);
        assert_eq!(keep(&Err("x".into())), KEEP_SHORT);
        assert_eq!(keep(&Ok(fix(0., 0., 500))), KEEP_SHORT);
        assert_eq!(keep(&Ok(fix(0., 0., 50))), KEEP_FIX);
    }

    #[test]
    fn a_slow_source_is_not_waited_for_nor_asked_again_while_it_runs() {
        let (slow, slow_calls) = counting(
            "os",
            Duration::from_millis(50),
            Duration::from_millis(400),
            Ok(fix(1., 1., 11)),
        );
        let (zone, _) = counting("zone", SEC, Duration::ZERO, Ok(fix(2., 2., 500)));
        let locator = Locator::with_sources(vec![slow, zone]);
        assert_eq!(ask_n(&locator, 1), [Ok(fix(2., 2., 500))]);
        // The coarse answer is kept a minute; a change of sources forgets it.
        let sources = locator.lock().sources.to_vec();
        locator.set_sources(sources);
        assert_eq!(ask_n(&locator, 1), [Ok(fix(2., 2., 500))]);
        assert_eq!(
            slow_calls.load(Ordering::SeqCst),
            1,
            "still running: skipped"
        );
        std::thread::sleep(Duration::from_millis(500));
        let sources = locator.lock().sources.to_vec();
        locator.set_sources(sources);
        assert_eq!(ask_n(&locator, 1), [Ok(fix(2., 2., 500))]);
        assert_eq!(
            slow_calls.load(Ordering::SeqCst),
            2,
            "returned: asked again"
        );
    }

    #[test]
    fn a_change_of_sources_reaches_those_that_asked_before() {
        let (first, _) = counting("city", SEC, Duration::ZERO, Ok(fix(1., 1., 11)));
        let locator = Locator::with_sources(vec![first]);
        let (notify, rx) = collector();
        locator.listen(Arc::clone(&notify));
        locator.ask(notify);
        assert_eq!(rx.recv_timeout(SEC * 5).unwrap(), Ok(fix(1., 1., 11)));
        let (second, _) = counting("city", SEC, Duration::ZERO, Ok(fix(2., 2., 11)));
        locator.set_sources(vec![second]);
        assert_eq!(rx.recv_timeout(SEC * 5).unwrap(), Ok(fix(2., 2., 11)));
        assert_eq!(ask_n(&locator, 1), [Ok(fix(2., 2., 11))], "kept");
    }

    #[test]
    fn an_answer_of_old_sources_is_dropped() {
        let (old, _) = counting("os", SEC, Duration::from_millis(200), Ok(fix(1., 1., 11)));
        let locator = Locator::with_sources(vec![old]);
        let (notify, rx) = collector();
        locator.listen(Arc::clone(&notify));
        locator.ask(notify);
        let (new, _) = counting("city", SEC, Duration::ZERO, Ok(fix(2., 2., 11)));
        locator.set_sources(vec![new]);
        assert_eq!(rx.recv_timeout(SEC * 5).unwrap(), Ok(fix(2., 2., 11)));
        assert!(
            rx.recv_timeout(Duration::from_millis(400)).is_err(),
            "one answer"
        );
        assert_eq!(ask_n(&locator, 1), [Ok(fix(2., 2., 11))]);
    }
}
