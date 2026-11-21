//! Test doubles. Available with the `testing` feature.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::effect::Effect;
use crate::index::{Item, ItemId};
use crate::module::{CoreEvent, HomeTile, Module, ModuleContext};
use crate::readings::QueryReading;
use crate::thread_time::ThreadTime;
use crate::view::{ViewEvent, ViewId};
use crate::widget::{
    Frame, Params, SearchAnswer, WidgetAnswer, WidgetContext, WidgetDecl, WidgetEvent,
    soft_render_deadline,
};

/// Answers a view event; see [`FakeModule::with_view_events`].
pub type ViewHandler = fn(&ViewId, ViewEvent) -> anyhow::Result<Vec<Effect>>;

/// Answers a wake-up; see [`FakeModule::with_on_wake`].
pub type WakeHandler = fn(u64) -> anyhow::Result<Vec<Effect>>;

/// Draws a widget; see [`FakeModule::with_render`].
pub type RenderHandler = fn(&str, &WidgetContext) -> anyhow::Result<Frame>;

/// Answers a widget event; see [`FakeModule::with_widget_events`].
pub type WidgetEventHandler = fn(&str, &Params, WidgetEvent) -> anyhow::Result<Vec<Effect>>;

/// A widget event as the module saw it: widget, the tile's parameters and event.
pub type SeenWidgetEvent = (String, Params, WidgetEvent);

/// How long [`wait_event`] waits before failing the test.
const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Keeps the calling thread busy until it has used `cpu` of CPU time (of
/// wall time where there is no thread clock).
pub fn burn_cpu(cpu: Duration) {
    let started = ThreadTime::now();
    while started.elapsed() < cpu {
        std::hint::spin_loop();
    }
}

/// Milliseconds of CPU time that make a render late in this build: past
/// [`soft_render_deadline`].
pub fn late_render_millis() -> u64 {
    soft_render_deadline().as_millis() as u64 + 4
}

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
    id: String,
    items: Vec<Item>,
    result: fn() -> anyhow::Result<Vec<Effect>>,
    /// `Some`: a dynamic source answering every query with these items.
    search_results: Option<Vec<Item>>,
    /// `Some`: answers view events; otherwise they get no effects.
    view_handler: Option<ViewHandler>,
    /// `Some`: answers wake-ups; otherwise they get no effects.
    wake_handler: Option<WakeHandler>,
    widgets: Vec<WidgetDecl>,
    /// Widget answers returned with every `search`.
    widget_answers: Vec<WidgetAnswer>,
    /// `Some`: draws widgets; otherwise a render fails.
    render_handler: Option<RenderHandler>,
    /// `Some`: answers widget events; otherwise they get no effects.
    widget_event_handler: Option<WidgetEventHandler>,
    /// Sleep before answering `run_action`, `search`, `on_view_event` and `render_widget`.
    delay: Duration,
    /// Milliseconds of extra sleep before answering `render_widget` only; may change any time.
    render_delay: Arc<AtomicU64>,
    /// Milliseconds of CPU time `render_widget` burns; may change any time.
    render_work: Arc<AtomicU64>,
    /// Calls of `render_widget`, counted as they start.
    pub renders: Arc<AtomicUsize>,
    /// Widget and context of every `render_widget` call, in order.
    pub render_contexts: Arc<Mutex<Vec<(String, WidgetContext)>>>,
    /// Widget, parameters and event of every `on_widget_event` call, in order.
    pub widget_events: Arc<Mutex<Vec<SeenWidgetEvent>>>,
    /// The tiles of every `home` call, in order.
    pub homes: Arc<Mutex<Vec<Vec<HomeTile>>>>,
    /// Executed actions.
    pub calls: Arc<Mutex<Vec<(ItemId, String)>>>,
    /// Queries passed to `search`.
    pub searches: Arc<Mutex<Vec<String>>>,
    /// Readings passed to `search_with`, one entry per call.
    pub readings: Arc<Mutex<Vec<Vec<QueryReading>>>>,
    /// View events received, in order.
    pub view_events: Arc<Mutex<Vec<(ViewId, ViewEvent)>>>,
    /// Name of the thread of every `start` and `run_action` call, in order.
    pub threads: Arc<Mutex<Vec<String>>>,
    /// Token and thread name of every `on_wake` call, in order.
    pub wakes: Arc<Mutex<Vec<(u64, String)>>>,
    /// The context `start` received; `None` until then.
    pub ctx: Arc<Mutex<Option<ModuleContext>>>,
    /// While set, `run_action` panics after recording the call.
    pub panic_on_action: Arc<AtomicBool>,
    /// While set, `search` panics after recording the query.
    pub panic_on_search: Arc<AtomicBool>,
}

impl FakeModule {
    pub fn new(items: Vec<Item>, result: fn() -> anyhow::Result<Vec<Effect>>) -> FakeModule {
        FakeModule {
            id: "fake".to_string(),
            items,
            result,
            search_results: None,
            view_handler: None,
            wake_handler: None,
            widgets: Vec::new(),
            widget_answers: Vec::new(),
            render_handler: None,
            widget_event_handler: None,
            delay: Duration::ZERO,
            render_delay: Arc::default(),
            render_work: Arc::default(),
            renders: Arc::default(),
            render_contexts: Arc::default(),
            widget_events: Arc::default(),
            homes: Arc::default(),
            calls: Arc::default(),
            searches: Arc::default(),
            readings: Arc::default(),
            view_events: Arc::default(),
            threads: Arc::default(),
            wakes: Arc::default(),
            ctx: Arc::default(),
            panic_on_action: Arc::default(),
            panic_on_search: Arc::default(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> FakeModule {
        self.id = id.into();
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

    /// Answers every wake-up with `handler`.
    pub fn with_on_wake(mut self, handler: WakeHandler) -> FakeModule {
        self.wake_handler = Some(handler);
        self
    }

    pub fn with_delay(mut self, delay: Duration) -> FakeModule {
        self.delay = delay;
        self
    }

    /// Declares `widgets`.
    pub fn with_widgets(mut self, widgets: Vec<WidgetDecl>) -> FakeModule {
        self.widgets = widgets;
        self
    }

    /// Draws every widget with `handler`.
    pub fn with_render(mut self, handler: RenderHandler) -> FakeModule {
        self.render_handler = Some(handler);
        self
    }

    /// Answers every search with `answers` too, besides the
    /// [`with_search`](Self::with_search) items.
    pub fn with_widget_answers(mut self, answers: Vec<WidgetAnswer>) -> FakeModule {
        self.widget_answers = answers;
        self
    }

    /// Answers every widget event with `handler`.
    pub fn with_widget_events(mut self, handler: WidgetEventHandler) -> FakeModule {
        self.widget_event_handler = Some(handler);
        self
    }

    /// Sleeps `millis` more before every render, read at each call, so a
    /// test can slow renders down and speed them up; other calls are not delayed.
    pub fn with_render_delay(mut self, millis: Arc<AtomicU64>) -> FakeModule {
        self.render_delay = millis;
        self
    }

    /// Burns `millis` of CPU time in every render, read at each call: a
    /// render that is late however loaded the machine is (see
    /// [`late_render_millis`]). A sleep uses no CPU time, so it is never late.
    pub fn with_render_work(mut self, millis: Arc<AtomicU64>) -> FakeModule {
        self.render_work = millis;
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
    fn id(&self) -> &str {
        &self.id
    }

    fn start(&mut self, ctx: ModuleContext) {
        self.record_thread();
        ctx.replace_items(self.items.clone());
        *self.ctx.lock().unwrap() = Some(ctx);
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

    fn search(&mut self, query: &str) -> anyhow::Result<SearchAnswer> {
        std::thread::sleep(self.delay);
        let panic = self.panic_on_search.load(Ordering::SeqCst);
        self.searches.lock().unwrap().push(query.to_string());
        if panic {
            panic!("FakeModule: panic_on_search is set");
        }
        Ok(SearchAnswer {
            items: self.search_results.clone().unwrap_or_default(),
            widgets: self.widget_answers.clone(),
        })
    }

    fn widgets(&self) -> Vec<WidgetDecl> {
        self.widgets.clone()
    }

    fn render_widget(&mut self, widget: &str, ctx: &WidgetContext) -> anyhow::Result<Frame> {
        self.renders.fetch_add(1, Ordering::SeqCst);
        self.render_contexts
            .lock()
            .unwrap()
            .push((widget.to_string(), ctx.clone()));
        let extra = Duration::from_millis(self.render_delay.load(Ordering::SeqCst));
        std::thread::sleep(self.delay + extra);
        burn_cpu(Duration::from_millis(
            self.render_work.load(Ordering::SeqCst),
        ));
        match self.render_handler {
            Some(handler) => handler(widget, ctx),
            None => anyhow::bail!("no widget {widget}"),
        }
    }

    fn on_widget_event(
        &mut self,
        widget: &str,
        params: &Params,
        event: WidgetEvent,
    ) -> anyhow::Result<Vec<Effect>> {
        self.widget_events.lock().unwrap().push((
            widget.to_string(),
            params.clone(),
            event.clone(),
        ));
        match self.widget_event_handler {
            Some(handler) => handler(widget, params, event),
            None => Ok(vec![]),
        }
    }

    fn home(&mut self, tiles: Vec<HomeTile>) {
        self.homes.lock().unwrap().push(tiles);
    }

    fn search_with(
        &mut self,
        query: &str,
        readings: &[QueryReading],
    ) -> anyhow::Result<SearchAnswer> {
        self.readings.lock().unwrap().push(readings.to_vec());
        self.search(query)
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

    fn on_wake(&mut self, token: u64) -> anyhow::Result<Vec<Effect>> {
        let name = std::thread::current()
            .name()
            .unwrap_or_default()
            .to_string();
        self.wakes.lock().unwrap().push((token, name));
        match self.wake_handler {
            Some(handler) => handler(token),
            None => Ok(vec![]),
        }
    }
}
