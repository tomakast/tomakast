//! Module host: one worker thread per module. Every module method runs on
//! its worker; replies come back to the core as [`CoreEvent`]s.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::index::Item;
use crate::module::{CoreEvent, HomeTile, Module, ModuleContext, RenderedAnswer, RequestId};
use crate::readings::QueryReading;
use crate::strip::WidgetEnv;
use crate::thread_time::ThreadTime;
use crate::view::{ViewEvent, ViewId};
use crate::widget::{
    Frame, Params, TileKey, WidgetAnswer, WidgetContext, WidgetDecl, WidgetEvent, check_frame,
};

/// Time a dynamic source has to answer one query. Nothing waits for it: a
/// late answer for the current query is still merged, and an overrun is logged.
pub const DYNAMIC_SEARCH_DEADLINE: Duration = Duration::from_millis(100);

pub(crate) enum Command {
    Action {
        request: RequestId,
        item: Item,
        action_id: String,
        query: String,
    },
    /// With `env`, a module that declared widgets also renders its widget
    /// answers, except those of its `stopped` widgets.
    Search {
        generation: u64,
        query: String,
        readings: Vec<QueryReading>,
        env: Option<WidgetEnv>,
        stopped: Vec<String>,
    },
    /// Draws `tile` (the module's widget `tile.widget.widget`) for `context`, which carries
    /// the tile's parameters.
    RenderWidget {
        tile: TileKey,
        seq: u64,
        context: WidgetContext,
    },
    /// `request` is `None` for an event no one waits on (`Shown`, `Hidden`).
    WidgetEvent {
        request: Option<RequestId>,
        widget: String,
        params: Params,
        event: WidgetEvent,
    },
    /// The module's own Home tiles changed: no reply.
    Home {
        tiles: Vec<HomeTile>,
    },
    ViewEvent {
        request: RequestId,
        view: ViewId,
        event: ViewEvent,
    },
    Wake {
        token: u64,
    },
    /// Wakes a worker blocked on an empty queue so it sees its stop flag.
    Stop,
}

struct Worker {
    /// Host-wide number of this module instance; never reused.
    instance: u64,
    commands: mpsc::Sender<Command>,
    /// Newest search generation sent to this worker; older queued searches are skipped.
    latest_search: Arc<AtomicU64>,
    /// Whether the module answers searches; set once it has started.
    dynamic: Arc<OnceLock<bool>>,
    /// Set when the instance is replaced or unregistered; the worker exits
    /// after its current call and serves nothing queued behind it (queued
    /// requests are answered with an error).
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        // Fails only when the worker has already exited.
        let _ = self.commands.send(Command::Stop);
    }
}

pub(crate) struct ModuleHost {
    /// Registered before [`start`](Self::start), not yet started, with their
    /// widgets. The id is read once, on registration, because it names the
    /// worker thread.
    pending: Vec<(String, Box<dyn Module>, Vec<WidgetDecl>)>,
    workers: HashMap<String, Worker>,
    events: async_channel::Sender<CoreEvent>,
    /// Set by `start`: from then on a registered module starts at once.
    started: bool,
    /// The number of the last instance started.
    last_instance: u64,
}

impl ModuleHost {
    pub(crate) fn new(events: async_channel::Sender<CoreEvent>) -> ModuleHost {
        ModuleHost {
            pending: Vec::new(),
            workers: HashMap::new(),
            events,
            started: false,
            last_instance: 0,
        }
    }

    /// Adds `module` under `id`, with the `widgets` it declared, both read
    /// from it by the caller. Before [`start`](Self::start) it waits; after,
    /// its worker starts at once. Call [`stop`](Self::stop) first to replace
    /// a running module.
    pub(crate) fn register(
        &mut self,
        id: String,
        module: Box<dyn Module>,
        widgets: Vec<WidgetDecl>,
    ) {
        self.pending.retain(|(pending, ..)| *pending != id);
        if self.started {
            self.spawn(id, module, widgets);
        } else {
            self.pending.push((id, module, widgets));
        }
    }

    /// Spawns a worker for every registered module; each worker calls `start` first.
    pub(crate) fn start(&mut self) {
        self.started = true;
        for (id, module, widgets) in std::mem::take(&mut self.pending) {
            self.spawn(id, module, widgets);
        }
    }

    /// Stops `provider`'s worker after its current call, or drops it if not
    /// started yet. Returns whether a worker was running.
    pub(crate) fn stop(&mut self, provider: &str) -> bool {
        self.pending.retain(|(pending, ..)| pending != provider);
        match self.workers.remove(provider) {
            Some(worker) => {
                worker.stop();
                true
            }
            None => false,
        }
    }

    /// Whether an instance of `provider` is registered (running, or waiting
    /// for [`start`](Self::start)).
    pub(crate) fn is_registered(&self, provider: &str) -> bool {
        self.workers.contains_key(provider) || self.pending.iter().any(|(id, ..)| id == provider)
    }

    /// The number of the running instance of `provider`.
    pub(crate) fn instance(&self, provider: &str) -> Option<u64> {
        self.workers.get(provider).map(|w| w.instance)
    }

    /// Whether `provider` has a worker and answers searches, or has not
    /// said yet whether it does: its `start` is still running.
    pub(crate) fn may_be_dynamic(&self, provider: &str) -> bool {
        self.workers
            .get(provider)
            .is_some_and(|w| w.dynamic.get().copied().unwrap_or(true))
    }

    /// Whether `instance` is the running instance of `provider`.
    pub(crate) fn is_current(&self, provider: &str, instance: u64) -> bool {
        self.workers
            .get(provider)
            .is_some_and(|w| w.instance == instance)
    }

    fn spawn(&mut self, id: String, module: Box<dyn Module>, widgets: Vec<WidgetDecl>) {
        // A running worker under this id would be orphaned, not stopped.
        if let Some(old) = self.workers.remove(&id) {
            old.stop();
        }
        self.last_instance += 1;
        let instance = self.last_instance;
        let (commands, rx) = mpsc::channel();
        let latest_search = Arc::new(AtomicU64::new(0));
        let dynamic = Arc::new(OnceLock::new());
        let stop = Arc::new(AtomicBool::new(false));
        let ctx = ModuleContext::new(&id, instance, self.events.clone(), commands.clone());
        let worker = WorkerState {
            id: id.clone(),
            ctx,
            latest_search: latest_search.clone(),
            dynamic: dynamic.clone(),
            stop: stop.clone(),
            widgets,
        };
        let spawned = std::thread::Builder::new()
            .name(format!("module-{id}"))
            .spawn(move || serve(worker, module, rx));
        match spawned {
            Ok(_) => {
                self.workers.insert(
                    id,
                    Worker {
                        instance,
                        commands,
                        latest_search,
                        dynamic,
                        stop,
                    },
                );
            }
            Err(err) => tracing::error!(module = %id, %err, "cannot start the module worker"),
        }
    }

    pub(crate) fn run_action(
        &self,
        request: RequestId,
        item: Item,
        action_id: &str,
        query: &str,
    ) -> anyhow::Result<()> {
        let provider = item.id.provider().to_string();
        self.send(
            &provider,
            Command::Action {
                request,
                item,
                action_id: action_id.to_string(),
                query: query.to_string(),
            },
        )
    }

    /// Asks every module for `query`; workers of static-only modules ignore
    /// it. `stopped` gives a provider's stopped widgets.
    pub(crate) fn search(
        &self,
        generation: u64,
        query: &str,
        readings: &[QueryReading],
        env: Option<&WidgetEnv>,
        stopped: impl Fn(&str) -> Vec<String>,
    ) {
        for id in self.workers.keys() {
            self.search_one(id, generation, query, readings, env, stopped(id));
        }
    }

    /// Asks `provider` alone for `query`, as [`search`](Self::search) does;
    /// nothing when it has no worker.
    pub(crate) fn search_one(
        &self,
        provider: &str,
        generation: u64,
        query: &str,
        readings: &[QueryReading],
        env: Option<&WidgetEnv>,
        stopped: Vec<String>,
    ) {
        let Some(worker) = self.workers.get(provider) else {
            return;
        };
        worker.latest_search.store(generation, Ordering::Release);
        let cmd = Command::Search {
            generation,
            query: query.to_string(),
            readings: readings.to_vec(),
            env: env.cloned(),
            stopped,
        };
        if worker.commands.send(cmd).is_err() {
            tracing::warn!(module = %provider, "module worker is gone");
        }
    }

    pub(crate) fn view_event(
        &self,
        provider: &str,
        request: RequestId,
        view: &ViewId,
        event: ViewEvent,
    ) -> anyhow::Result<()> {
        self.send(
            provider,
            Command::ViewEvent {
                request,
                view: view.clone(),
                event,
            },
        )
    }

    pub(crate) fn render_widget(
        &self,
        tile: &TileKey,
        seq: u64,
        context: WidgetContext,
    ) -> anyhow::Result<()> {
        self.send(
            &tile.widget.provider,
            Command::RenderWidget {
                tile: tile.clone(),
                seq,
                context,
            },
        )
    }

    /// Sends `event` to the module of `tile`, with the tile's parameters.
    pub(crate) fn widget_event(
        &self,
        tile: &TileKey,
        request: Option<RequestId>,
        event: WidgetEvent,
    ) -> anyhow::Result<()> {
        self.send(
            &tile.widget.provider,
            Command::WidgetEvent {
                request,
                widget: tile.widget.widget.clone(),
                params: tile.params.clone(),
                event,
            },
        )
    }

    /// Tells `provider`'s module its own Home tiles.
    pub(crate) fn home(&self, provider: &str, tiles: Vec<HomeTile>) -> anyhow::Result<()> {
        self.send(provider, Command::Home { tiles })
    }

    fn send(&self, provider: &str, cmd: Command) -> anyhow::Result<()> {
        let worker = self
            .workers
            .get(provider)
            .ok_or_else(|| anyhow!("no module for provider {provider}"))?;
        worker
            .commands
            .send(cmd)
            .map_err(|_| anyhow!("module {provider} is not running"))
    }
}

impl Drop for ModuleHost {
    /// The contexts modules keep hold command senders, so a worker's queue
    /// never closes on its own: stop every worker explicitly.
    fn drop(&mut self) {
        for worker in self.workers.values() {
            worker.stop();
        }
    }
}

/// What a worker thread owns besides its module and its queue.
struct WorkerState {
    id: String,
    /// Stamps every event the worker sends with its instance.
    ctx: ModuleContext,
    latest_search: Arc<AtomicU64>,
    /// Set once `start` returned.
    dynamic: Arc<OnceLock<bool>>,
    stop: Arc<AtomicBool>,
    /// The widgets the module declared.
    widgets: Vec<WidgetDecl>,
}

/// The worker loop: starts the module, then serves commands until stopped
/// or until the core is gone.
fn serve(worker: WorkerState, mut module: Box<dyn Module>, commands: mpsc::Receiver<Command>) {
    let WorkerState {
        id,
        ctx,
        latest_search,
        dynamic: dynamic_flag,
        stop,
        widgets,
    } = worker;
    let id = id.as_str();
    let start_ctx = ctx.clone();
    if let Err(message) = guarded(id, || {
        module.start(start_ctx);
        Ok(())
    }) {
        tracing::error!(module = id, "start failed: {message}");
    }
    let dynamic = guarded(id, || Ok(module.is_dynamic_source())).unwrap_or_else(|message| {
        tracing::error!(module = id, "{message}");
        false
    });
    // Set here only, once: it cannot fail.
    let _ = dynamic_flag.set(dynamic);
    for cmd in commands.iter() {
        if stop.load(Ordering::Acquire) {
            refuse(&ctx, id, cmd);
            break;
        }
        let ev = match cmd {
            Command::Stop => break,
            Command::Action {
                request,
                item,
                action_id,
                query,
            } => CoreEvent::ActionFinished {
                request,
                result: guarded(id, || module.run_action(&item, &action_id)),
                item: item.id,
                query,
            },
            Command::Search {
                generation,
                query,
                readings,
                env,
                stopped,
            } => {
                let superseded = || generation < latest_search.load(Ordering::Acquire);
                if !dynamic || superseded() {
                    continue;
                }
                // Only a module that declared widgets answers with tiles.
                let env = env.filter(|_| !widgets.is_empty());
                let started = Instant::now();
                let result = guarded(id, || module.search_with(&query, &readings));
                let elapsed = started.elapsed();
                if elapsed > DYNAMIC_SEARCH_DEADLINE {
                    tracing::warn!(
                        module = id,
                        ms = elapsed.as_secs_f64() * 1000.0,
                        "search missed its deadline"
                    );
                }
                if superseded() {
                    continue;
                }
                match (result, env) {
                    (Ok(answer), None) => CoreEvent::SearchResults {
                        provider: id.to_string(),
                        generation,
                        items: answer.items,
                        busy: answer.busy,
                    },
                    (Ok(answer), Some(env)) => {
                        let items = CoreEvent::SearchResults {
                            provider: id.to_string(),
                            generation,
                            items: answer.items,
                            busy: answer.busy,
                        };
                        if !ctx.send(items) {
                            return;
                        }
                        let module = module.as_mut();
                        let answers = render_answers(
                            id,
                            module,
                            &widgets,
                            &stopped,
                            &env,
                            &query,
                            answer.widgets,
                        );
                        CoreEvent::WidgetAnswers {
                            provider: id.to_string(),
                            generation,
                            answers,
                        }
                    }
                    (Err(message), env) => {
                        tracing::warn!(module = id, "search failed: {message}");
                        if env.is_none() {
                            continue;
                        }
                        // No tiles: the provider's condition tiles go with the failed answer.
                        CoreEvent::WidgetAnswers {
                            provider: id.to_string(),
                            generation,
                            answers: Vec::new(),
                        }
                    }
                }
            }
            Command::RenderWidget { tile, seq, context } => {
                let widget = tile.widget.widget.as_str();
                let (took, result) = render(id, module.as_mut(), widget, &context);
                CoreEvent::WidgetRendered {
                    provider: id.to_string(),
                    tile,
                    seq,
                    took,
                    result,
                }
            }
            Command::WidgetEvent {
                request,
                widget,
                params,
                event,
            } => CoreEvent::WidgetEventFinished {
                request,
                provider: id.to_string(),
                result: guarded(id, || module.on_widget_event(&widget, &params, event)),
                widget,
            },
            Command::Home { tiles } => {
                if let Err(message) = guarded(id, || {
                    module.home(tiles);
                    Ok(())
                }) {
                    tracing::warn!(module = id, "home failed: {message}");
                }
                continue;
            }
            Command::ViewEvent {
                request,
                view,
                event,
            } => CoreEvent::ViewEventFinished {
                request,
                provider: id.to_string(),
                result: guarded(id, || module.on_view_event(&view, event)),
                view,
            },
            Command::Wake { token } => match guarded(id, || module.on_wake(token)) {
                Ok(effects) if effects.is_empty() => continue,
                Ok(effects) => CoreEvent::ModuleEffects {
                    provider: id.to_string(),
                    effects,
                },
                Err(message) => {
                    tracing::warn!(module = id, "on_wake failed: {message}");
                    continue;
                }
            },
        };
        if !ctx.send(ev) {
            return;
        }
    }
    if stop.load(Ordering::Acquire) {
        for cmd in commands.try_iter() {
            refuse(&ctx, id, cmd);
        }
    }
}

/// Answers a request a stopped worker will not serve, so nobody waits for it.
fn refuse(ctx: &ModuleContext, id: &str, cmd: Command) {
    let message = format!("module {id} was stopped");
    let ev = match cmd {
        Command::Action {
            request,
            item,
            query,
            ..
        } => CoreEvent::ActionFinished {
            request,
            item: item.id,
            query,
            result: Err(message),
        },
        Command::ViewEvent { request, view, .. } => CoreEvent::ViewEventFinished {
            request,
            provider: id.to_string(),
            view,
            result: Err(message),
        },
        Command::WidgetEvent {
            request: Some(request),
            widget,
            ..
        } => CoreEvent::WidgetEventFinished {
            request: Some(request),
            provider: id.to_string(),
            widget,
            result: Err(message),
        },
        Command::Search { .. }
        | Command::RenderWidget { .. }
        | Command::Home { .. }
        | Command::WidgetEvent { request: None, .. }
        | Command::Wake { .. }
        | Command::Stop => return,
    };
    ctx.send(ev);
}

/// Renders `widget` for `context`. The time is the worker thread's CPU time
/// during the module call alone (wall time where there is no thread clock):
/// a thread the machine's load keeps waiting does not make a render late.
/// A frame that fails [`check_frame`] is an `Err`.
fn render(
    id: &str,
    module: &mut dyn Module,
    widget: &str,
    context: &WidgetContext,
) -> (Duration, Result<Frame, String>) {
    let started = ThreadTime::now();
    let result = guarded(id, || module.render_widget(widget, context));
    let took = started.elapsed();
    let result = result.and_then(|frame| match check_frame(&frame) {
        Ok(()) => Ok(frame),
        Err(err) => Err(format!("invalid frame: {err}")),
    });
    (took, result)
}

/// Renders every answer of a declared widget that is not `stopped` for
/// `query`, in order, in the size it asks for if the widget offers it, else
/// the widget's default; the rest are dropped, as is a second answer of the
/// same widget.
fn render_answers(
    id: &str,
    module: &mut dyn Module,
    widgets: &[WidgetDecl],
    stopped: &[String],
    env: &WidgetEnv,
    query: &str,
    answers: Vec<WidgetAnswer>,
) -> Vec<RenderedAnswer> {
    let mut seen = HashSet::new();
    let mut rendered = Vec::with_capacity(answers.len());
    for answer in answers {
        let Some(decl) = widgets.iter().find(|d| d.id == answer.widget) else {
            tracing::debug!(module = id, widget = %answer.widget, "dropping an answer of an undeclared widget");
            continue;
        };
        if stopped.contains(&decl.id) {
            tracing::debug!(module = id, widget = %answer.widget, "dropping an answer of a stopped widget");
            continue;
        }
        if !seen.insert(decl.id.as_str()) {
            tracing::debug!(module = id, widget = %answer.widget, "dropping a second answer of a widget");
            continue;
        }
        let size = decl.size_or_default(answer.size);
        let context = env.context(size, Some(query), answer.params.clone());
        let (took, frame) = render(id, module, &answer.widget, &context);
        rendered.push(RenderedAnswer {
            answer,
            took,
            frame,
        });
    }
    rendered
}

/// Runs one module call; a panic becomes `Err("module <id> panicked")`.
fn guarded<T>(id: &str, call: impl FnOnce() -> anyhow::Result<T>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(format!("{err:#}")),
        Err(_) => Err(format!("module {id} panicked")),
    }
}
