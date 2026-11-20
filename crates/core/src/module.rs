//! Module interface: providers of items and executors of their actions.

use std::sync::mpsc;
use std::time::Duration;

use crate::effect::Effect;
use crate::host::Command;
use crate::index::{Item, ItemId};
use crate::readings::QueryReading;
use crate::view::{ViewEvent, ViewId};
use crate::widget::{Frame, SearchAnswer, WidgetAnswer, WidgetContext, WidgetDecl, WidgetEvent};

/// Identifies one action or view-event request, so its reply can be matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

/// Events sent to the core from module workers and module-owned threads.
#[derive(Clone, Debug)]
pub enum CoreEvent {
    ReplaceItems {
        provider: String,
        items: Vec<Item>,
    },
    ActionFinished {
        request: RequestId,
        item: ItemId,
        /// The query typed when the action was requested.
        query: String,
        result: Result<Vec<Effect>, String>,
    },
    /// A dynamic source's answer to the search of `generation`.
    SearchResults {
        provider: String,
        generation: u64,
        items: Vec<Item>,
    },
    ViewEventFinished {
        request: RequestId,
        /// The module that owns the view and answered.
        provider: String,
        view: ViewId,
        result: Result<Vec<Effect>, String>,
    },
    /// Effects a module sent on its own: from `on_wake`, or through
    /// [`ModuleContext::emit`].
    ModuleEffects {
        provider: String,
        effects: Vec<Effect>,
    },
    /// The frame of render `seq` of `widget`. `took` is the module call alone.
    WidgetRendered {
        provider: String,
        widget: String,
        seq: u64,
        took: Duration,
        result: Result<Frame, String>,
    },
    /// The widget answers of a dynamic source's search of `generation`,
    /// each rendered; sent after its `SearchResults`, and empty when the
    /// search failed.
    WidgetAnswers {
        provider: String,
        generation: u64,
        answers: Vec<RenderedAnswer>,
    },
    /// A module's answer to a widget event; `request` is that of
    /// [`Core::send_widget_event`](crate::Core::send_widget_event).
    WidgetEventFinished {
        request: Option<RequestId>,
        provider: String,
        widget: String,
        result: Result<Vec<Effect>, String>,
    },
    /// Sent by the core itself: the strip changed since the last one was applied.
    StripChanged,
    /// `event`, sent by the worker or a context of `provider`'s module
    /// instance number `instance`. The core applies `event` only while that
    /// instance is the registered one: once it is replaced or unregistered,
    /// its replies fail and everything else it sends is dropped.
    Instance {
        provider: String,
        instance: u64,
        event: Box<CoreEvent>,
    },
}

/// A widget answer and its frame, rendered on the search's worker turn.
#[derive(Clone, Debug)]
pub struct RenderedAnswer {
    pub answer: WidgetAnswer,
    /// The render call alone.
    pub took: Duration,
    pub frame: Result<Frame, String>,
}

/// Handle given to a module in `start`; cheap to clone and `Send`. It
/// belongs to one module instance: once that instance is replaced or
/// unregistered, everything sent through it is dropped.
#[derive(Clone)]
pub struct ModuleContext {
    provider: String,
    instance: u64,
    tx: async_channel::Sender<CoreEvent>,
    /// The instance's worker, for [`wake`](Self::wake).
    worker: mpsc::Sender<Command>,
}

impl ModuleContext {
    pub(crate) fn new(
        provider: &str,
        instance: u64,
        tx: async_channel::Sender<CoreEvent>,
        worker: mpsc::Sender<Command>,
    ) -> ModuleContext {
        ModuleContext {
            provider: provider.to_string(),
            instance,
            tx,
            worker,
        }
    }

    /// Replaces all items of this module's provider. Items whose id belongs to
    /// another provider, or is not a valid id, are dropped with a warning.
    pub fn replace_items(&self, items: Vec<Item>) {
        let ev = CoreEvent::ReplaceItems {
            provider: self.provider.clone(),
            items: own_valid_items(&self.provider, items),
        };
        if !self.send(ev) {
            tracing::debug!(module = %self.provider, "core is gone, items not delivered");
        }
    }

    /// Queues a wake-up on this module's worker, which then calls
    /// [`Module::on_wake`] with `token`. Does nothing once the worker stopped.
    pub fn wake(&self, token: u64) {
        if self.worker.send(Command::Wake { token }).is_err() {
            tracing::debug!(module = %self.provider, "worker is gone, wake-up dropped");
        }
    }

    /// Sends `effects` to the core from any thread, as a module's own update
    /// ([`CoreEvent::ModuleEffects`]). An empty list sends nothing.
    pub fn emit(&self, effects: Vec<Effect>) {
        if effects.is_empty() {
            return;
        }
        let ev = CoreEvent::ModuleEffects {
            provider: self.provider.clone(),
            effects,
        };
        if !self.send(ev) {
            tracing::debug!(module = %self.provider, "core is gone, effects not delivered");
        }
    }

    /// Sends `event` stamped with this instance; `false` when the core is gone.
    pub(crate) fn send(&self, event: CoreEvent) -> bool {
        let ev = CoreEvent::Instance {
            provider: self.provider.clone(),
            instance: self.instance,
            event: Box::new(event),
        };
        // The channel is unbounded; it only fails when the core is gone.
        self.tx.try_send(ev).is_ok()
    }
}

/// Keeps the items whose id is valid and belongs to `provider`; warns about the rest.
pub(crate) fn own_valid_items(provider: &str, items: Vec<Item>) -> Vec<Item> {
    items
        .into_iter()
        .filter(|item| {
            let problem = if !item.id.is_valid() {
                Some("invalid id")
            } else if item.id.provider() != provider {
                Some("foreign provider")
            } else {
                None
            };
            if let Some(problem) = problem {
                tracing::warn!(module = %provider, item = ?item.id.as_str(), "dropping item: {problem}");
            }
            problem.is_none()
        })
        .collect()
}

/// A provider of items. Every method but `id`, `widgets` and `approved_at`
/// (read once, on registration) is called on the module's own worker thread
/// (`module-<id>`), never on the UI thread; a call may block it.
pub trait Module: Send + 'static {
    /// Provider id: the prefix of every item id this module publishes.
    fn id(&self) -> &str;
    fn start(&mut self, ctx: ModuleContext);
    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>>;
    /// Whether [`search`](Self::search) answers queries; asked once, after `start`.
    fn is_dynamic_source(&self) -> bool {
        false
    }
    /// Items computed for `query` (calculator, converter). Only called when
    /// [`is_dynamic_source`](Self::is_dynamic_source) is true.
    fn search(&mut self, query: &str) -> anyhow::Result<SearchAnswer> {
        let _ = query;
        Ok(SearchAnswer::from(vec![]))
    }
    /// The widgets this module declares. Read once, with
    /// [`approved_at`](Self::approved_at), when the module is registered, on
    /// the caller's thread, like [`id`](Self::id).
    fn widgets(&self) -> Vec<WidgetDecl> {
        vec![]
    }
    /// When the user approved this module's current permissions and widgets, in
    /// seconds since the epoch; `None` for built-in modules.
    fn approved_at(&self) -> Option<i64> {
        None
    }
    /// Draws `widget` for `ctx`.
    fn render_widget(&mut self, widget: &str, ctx: &WidgetContext) -> anyhow::Result<Frame> {
        let _ = ctx;
        anyhow::bail!("no widget {widget}")
    }
    /// Answers an event of a tile of `widget`; `payload` is that of a condition tile.
    fn on_widget_event(
        &mut self,
        widget: &str,
        payload: Option<&str>,
        event: WidgetEvent,
    ) -> anyhow::Result<Vec<Effect>> {
        let _ = (widget, payload, event);
        Ok(vec![])
    }
    /// [`search`](Self::search) with the readings of `query` on other keyboard
    /// layouts and scripts. Defaults to `search`, ignoring the readings.
    fn search_with(
        &mut self,
        query: &str,
        readings: &[QueryReading],
    ) -> anyhow::Result<SearchAnswer> {
        let _ = readings;
        self.search(query)
    }
    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        let _ = (view, event);
        Ok(vec![])
    }
    /// Answers a [`ModuleContext::wake`]; its effects reach the core as
    /// [`CoreEvent::ModuleEffects`].
    fn on_wake(&mut self, token: u64) -> anyhow::Result<Vec<Effect>> {
        let _ = token;
        Ok(vec![])
    }
}
