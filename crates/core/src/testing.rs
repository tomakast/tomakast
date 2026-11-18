//! Test doubles. Available with the `testing` feature.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::effect::Effect;
use crate::index::{Item, ItemId};
use crate::module::{CoreEvent, Module, ModuleContext};
use crate::view::{ViewEvent, ViewId};

/// Answers a view event; see [`FakeModule::with_view_events`].
pub type ViewHandler = fn(&ViewId, ViewEvent) -> anyhow::Result<Vec<Effect>>;

/// How long [`wait_event`] waits before failing the test.
const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Blocks until the next core event arrives; panics after 5 s.
pub fn wait_event(rx: &async_channel::Receiver<CoreEvent>) -> CoreEvent {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        match rx.try_recv() {
            Ok(ev) => return ev,
            Err(async_channel::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "no core event within {EVENT_TIMEOUT:?}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("core event channel closed"),
        }
    }
}

/// Module that publishes fixed items and records its calls. Its id is
/// `fake` unless set with [`with_id`](Self::with_id).
pub struct FakeModule {
    id: &'static str,
    items: Vec<Item>,
    result: fn() -> anyhow::Result<Vec<Effect>>,
    /// `Some`: a dynamic source answering every query with these items.
    search_results: Option<Vec<Item>>,
    /// `Some`: answers view events; otherwise they get no effects.
    view_handler: Option<ViewHandler>,
    /// Sleep before answering `run_action`, `search` and `on_view_event`.
    delay: Duration,
    /// Executed actions.
    pub calls: Arc<Mutex<Vec<(ItemId, String)>>>,
    /// Queries passed to `search`.
    pub searches: Arc<Mutex<Vec<String>>>,
    /// View events received, in order.
    pub view_events: Arc<Mutex<Vec<(ViewId, ViewEvent)>>>,
    /// Name of the thread of every `start` and `run_action` call, in order.
    pub threads: Arc<Mutex<Vec<String>>>,
    /// While set, `run_action` panics after recording the call.
    pub panic_on_action: Arc<AtomicBool>,
    /// While set, `search` panics after recording the query.
    pub panic_on_search: Arc<AtomicBool>,
}

impl FakeModule {
    pub fn new(items: Vec<Item>, result: fn() -> anyhow::Result<Vec<Effect>>) -> FakeModule {
        FakeModule {
            id: "fake",
            items,
            result,
            search_results: None,
            view_handler: None,
            delay: Duration::ZERO,
            calls: Arc::default(),
            searches: Arc::default(),
            view_events: Arc::default(),
            threads: Arc::default(),
            panic_on_action: Arc::default(),
            panic_on_search: Arc::default(),
        }
    }

    pub fn with_id(mut self, id: &'static str) -> FakeModule {
        self.id = id;
        self
    }

    /// Makes the module a dynamic source that answers every query with `items`.
    pub fn with_search(mut self, items: Vec<Item>) -> FakeModule {
        self.search_results = Some(items);
        self
    }

    /// Answers every view event with `handler`.
    pub fn with_view_events(mut self, handler: ViewHandler) -> FakeModule {
        self.view_handler = Some(handler);
        self
    }

    pub fn with_delay(mut self, delay: Duration) -> FakeModule {
        self.delay = delay;
        self
    }

    fn record_thread(&self) {
        let name = std::thread::current()
            .name()
            .unwrap_or_default()
            .to_string();
        self.threads.lock().unwrap().push(name);
    }
}

impl Module for FakeModule {
    fn id(&self) -> &'static str {
        self.id
    }

    fn start(&mut self, ctx: ModuleContext) {
        self.record_thread();
        ctx.replace_items(self.items.clone());
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        self.record_thread();
        std::thread::sleep(self.delay);
        // Read before the call is recorded, so a test that sees the call knows its outcome.
        let panic = self.panic_on_action.load(Ordering::SeqCst);
        self.calls
            .lock()
            .unwrap()
            .push((item.id.clone(), action_id.to_string()));
        if panic {
            panic!("FakeModule: panic_on_action is set");
        }
        (self.result)()
    }

    fn is_dynamic_source(&self) -> bool {
        self.search_results.is_some()
    }

    fn search(&mut self, query: &str) -> anyhow::Result<Vec<Item>> {
        std::thread::sleep(self.delay);
        let panic = self.panic_on_search.load(Ordering::SeqCst);
        self.searches.lock().unwrap().push(query.to_string());
        if panic {
            panic!("FakeModule: panic_on_search is set");
        }
        Ok(self.search_results.clone().unwrap_or_default())
    }

    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        std::thread::sleep(self.delay);
        self.view_events
            .lock()
            .unwrap()
            .push((view.clone(), event.clone()));
        match self.view_handler {
            Some(handler) => handler(view, event),
            None => Ok(vec![]),
        }
    }
}
