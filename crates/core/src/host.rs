//! Module host: one worker thread per module. Every module method runs on
//! its worker; replies come back to the core as [`CoreEvent`]s.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::index::Item;
use crate::module::{CoreEvent, Module, ModuleContext, RequestId};
use crate::view::{ViewEvent, ViewId};

/// Time a dynamic source has to answer one query. Nothing waits for it: a
/// late answer for the current query is still merged, and an overrun is logged.
pub const DYNAMIC_SEARCH_DEADLINE: Duration = Duration::from_millis(100);

enum Command {
    Action {
        request: RequestId,
        item: Item,
        action_id: String,
        query: String,
    },
    Search {
        generation: u64,
        query: String,
    },
    ViewEvent {
        request: RequestId,
        view: ViewId,
        event: ViewEvent,
    },
}

struct Worker {
    commands: mpsc::Sender<Command>,
    /// Newest search generation sent to this worker; older queued searches are skipped.
    latest_search: Arc<AtomicU64>,
}

pub(crate) struct ModuleHost {
    /// Registered, not yet started. The id is read once, on registration,
    /// because it names the worker thread.
    pending: Vec<(&'static str, Box<dyn Module>)>,
    workers: HashMap<&'static str, Worker>,
    events: async_channel::Sender<CoreEvent>,
}

impl ModuleHost {
    pub(crate) fn new(events: async_channel::Sender<CoreEvent>) -> ModuleHost {
        ModuleHost {
            pending: Vec::new(),
            workers: HashMap::new(),
            events,
        }
    }

    pub(crate) fn register(&mut self, module: Box<dyn Module>) {
        let id = module.id();
        self.pending.retain(|(pending, _)| *pending != id);
        self.pending.push((id, module));
    }

    /// Spawns a worker for every registered module; each worker calls `start` first.
    pub(crate) fn start(&mut self) {
        for (id, module) in std::mem::take(&mut self.pending) {
            let (commands, rx) = mpsc::channel();
            let latest_search = Arc::new(AtomicU64::new(0));
            let ctx = ModuleContext::new(id, self.events.clone());
            let events = self.events.clone();
            let latest = latest_search.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("module-{id}"))
                .spawn(move || serve(id, module, ctx, rx, &latest, &events));
            match spawned {
                Ok(_) => {
                    // A replaced worker exits once its command sender is dropped here.
                    self.workers.insert(
                        id,
                        Worker {
                            commands,
                            latest_search,
                        },
                    );
                }
                Err(err) => tracing::error!(module = id, %err, "cannot start the module worker"),
            }
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

    /// Asks every module for `query`; workers of static-only modules ignore it.
    pub(crate) fn search(&self, generation: u64, query: &str) {
        for (id, worker) in &self.workers {
            worker.latest_search.store(generation, Ordering::Release);
            let cmd = Command::Search {
                generation,
                query: query.to_string(),
            };
            if worker.commands.send(cmd).is_err() {
                tracing::warn!(module = id, "module worker is gone");
            }
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

/// The worker loop: starts the module, then serves commands until the core is gone.
fn serve(
    id: &'static str,
    mut module: Box<dyn Module>,
    ctx: ModuleContext,
    commands: mpsc::Receiver<Command>,
    latest_search: &AtomicU64,
    events: &async_channel::Sender<CoreEvent>,
) {
    if let Err(message) = guarded(id, || {
        module.start(ctx);
        Ok(())
    }) {
        tracing::error!(module = id, "start failed: {message}");
    }
    let dynamic = guarded(id, || Ok(module.is_dynamic_source())).unwrap_or_else(|message| {
        tracing::error!(module = id, "{message}");
        false
    });
    for cmd in commands {
        let ev = match cmd {
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
            Command::Search { generation, query } => {
                let superseded = || generation < latest_search.load(Ordering::Acquire);
                if !dynamic || superseded() {
                    continue;
                }
                let started = Instant::now();
                let result = guarded(id, || module.search(&query));
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
                match result {
                    Ok(items) => CoreEvent::SearchResults {
                        provider: id.to_string(),
                        generation,
                        items,
                    },
                    Err(message) => {
                        tracing::warn!(module = id, "search failed: {message}");
                        continue;
                    }
                }
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
        };
        if events.try_send(ev).is_err() {
            break;
        }
    }
}

/// Runs one module call; a panic becomes `Err("module <id> panicked")`.
fn guarded<T>(id: &str, call: impl FnOnce() -> anyhow::Result<T>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(format!("{err:#}")),
        Err(_) => Err(format!("module {id} panicked")),
    }
}
