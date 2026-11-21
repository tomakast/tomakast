//! The view stack above the root search list, the ledger of requests the
//! launcher windows issued, and the one place where module effects are applied.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{App, ClipboardItem, Context, Window, WindowHandle};
use tomakast_core::{CoreUpdate, Effect, RequestId, View, ViewId};

use crate::launcher_view::LauncherView;
use crate::views::ViewState;

/// A module-owned view pushed above the root list.
pub(crate) struct StackEntry {
    /// Number of this push, unique within the window. View ids are the
    /// module's own and usually constant, so a view pushed again has the
    /// same id; replies and the Enter gate are matched by instance.
    pub instance: u64,
    /// The module that owns the view; it receives the view's events.
    pub provider: String,
    /// The instance of `provider` that pushed the view. Its events go to that instance only:
    /// once the module is replaced (a reloaded plugin) they fail with "plugin reloaded".
    pub module_instance: u64,
    pub view: View,
    /// Revision of the view's typed text: the pushed view's revision, then
    /// incremented for every text event sent to the module.
    pub revision: u64,
    pub state: ViewState,
}

/// Pushed views a stack holds at most (Ruling AB); a push beyond is dropped.
pub(crate) const MAX_PUSHED_VIEWS: usize = 32;

/// Pushed views, bottom to top. The root search list is below them and
/// counts as depth 1.
#[derive(Default)]
pub(crate) struct ViewStack {
    entries: Vec<StackEntry>,
    /// Instances handed out so far.
    instances: u64,
}

/// What [`ViewStack::update`] did with an `UpdateView`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Updated {
    Applied,
    /// The update's revision is not the view's current one.
    Stale,
    /// The view is not on the stack.
    Unknown,
    /// The update is a different kind of view than the one on the stack.
    KindChanged,
    /// A form update with other fields than the form on the stack.
    FieldsChanged,
}

impl ViewStack {
    pub(crate) fn depth(&self) -> usize {
        self.entries.len() + 1
    }

    /// The instance number for the next push.
    pub(crate) fn new_instance(&mut self) -> u64 {
        self.instances += 1;
        self.instances
    }

    /// Whether another push would exceed [`MAX_PUSHED_VIEWS`].
    pub(crate) fn is_full(&self) -> bool {
        self.entries.len() >= MAX_PUSHED_VIEWS
    }

    pub(crate) fn push(&mut self, entry: StackEntry) {
        self.entries.push(entry);
    }

    pub(crate) fn pop(&mut self) -> Option<StackEntry> {
        self.entries.pop()
    }

    /// The view ids, bottom to top.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &ViewId> {
        self.entries.iter().map(|e| e.view.id())
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The index, bottom up, of the topmost entry with view `id` that `provider` pushed
    /// (any instance of it: a reloaded plugin takes its old view's place).
    pub(crate) fn position(&self, provider: &str, id: &ViewId) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|e| e.provider == provider && e.view.id() == id)
    }

    /// Whether every entry above `provider`'s view `id` (if it has one on the stack) is
    /// `provider`'s too: an unsolicited push may return to that view only then (Ruling 9b).
    pub(crate) fn owns_above(&self, provider: &str, id: &ViewId) -> bool {
        self.position(provider, id).is_none_or(|at| {
            self.entries[at + 1..]
                .iter()
                .all(|e| e.provider == provider)
        })
    }

    pub(crate) fn top(&self) -> Option<&StackEntry> {
        self.entries.last()
    }

    pub(crate) fn top_mut(&mut self) -> Option<&mut StackEntry> {
        self.entries.last_mut()
    }

    pub(crate) fn get(&self, instance: u64) -> Option<&StackEntry> {
        self.entries.iter().find(|e| e.instance == instance)
    }

    pub(crate) fn get_mut(&mut self, instance: u64) -> Option<&mut StackEntry> {
        self.entries.iter_mut().find(|e| e.instance == instance)
    }

    pub(crate) fn contains(&self, instance: u64) -> bool {
        self.get(instance).is_some()
    }

    /// The topmost view `id` pushed by instance `module_instance` of
    /// `provider`. View ids are the module's own, so two modules, or two
    /// instances of one, may use the same one.
    pub(crate) fn find(
        &self,
        provider: &str,
        module_instance: u64,
        id: &ViewId,
    ) -> Option<&StackEntry> {
        self.entries.iter().rev().find(|e| {
            e.provider == provider && e.module_instance == module_instance && e.view.id() == id
        })
    }

    /// Applies an `UpdateView` to view `instance`, following the revision
    /// rule: a view with typed text takes the update only at its current
    /// revision, and keeps its text. The caller has checked that the update's
    /// id and provider are the instance's.
    pub(crate) fn update(&mut self, instance: u64, mut update: View) -> Updated {
        let Some(entry) = self.get_mut(instance) else {
            return Updated::Unknown;
        };
        if std::mem::discriminant(&entry.view) != std::mem::discriminant(&update) {
            return Updated::KindChanged;
        }
        match (&entry.view, &mut update) {
            (View::List(current), View::List(new)) => {
                if new.revision != entry.revision {
                    return Updated::Stale;
                }
                new.query.clone_from(&current.query);
            }
            (View::Scratchpad(current), View::Scratchpad(new)) => {
                if new.revision != entry.revision {
                    return Updated::Stale;
                }
                new.text.clone_from(&current.text);
            }
            (View::Form(current), View::Form(new)) => {
                let same = current.fields.len() == new.fields.len()
                    && current.fields.iter().zip(&new.fields).all(|(a, b)| {
                        a.id == b.id
                            && std::mem::discriminant(&a.kind) == std::mem::discriminant(&b.kind)
                    });
                if !same {
                    return Updated::FieldsChanged;
                }
            }
            _ => {}
        }
        entry.view = update;
        Updated::Applied
    }
}

/// How long a pending action blocks Enter on its provider's rows. A module
/// that hangs longer stops blocking; its late reply is still applied.
pub(crate) const ACTION_GATE: Duration = Duration::from_secs(5);

/// Whether a request is an action (Enter, which is gated while it is
/// pending) or a text event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestKind {
    Action,
    Query,
}

/// A request a launcher window issued and whose reply has not been applied.
#[derive(Clone, Debug)]
struct Pending {
    /// [`RequestLedger::open_window`] number of the issuing window.
    window: u64,
    provider: String,
    /// Instance of the pushed view that issued it; `None` for the root list.
    view: Option<u64>,
    kind: RequestKind,
    /// Executor time, so tests can drive it with the simulated clock.
    issued_at: Instant,
}

/// Requests issued by launcher windows, shared by the controller and the
/// open window's view. A reply to a request not in the ledger was not issued
/// by a window: the daemon ran an item (hotkey, `tomakast run`).
#[derive(Default)]
pub(crate) struct RequestLedger {
    windows: u64,
    requests: HashMap<RequestId, Pending>,
}

pub(crate) type SharedLedger = Rc<RefCell<RequestLedger>>;

/// Who issued a request, for [`RequestLedger::issue`].
pub(crate) struct Issuer<'a> {
    pub window: u64,
    pub provider: &'a str,
    /// Instance of the issuing view; `None`: the root list.
    pub view: Option<u64>,
}

impl RequestLedger {
    /// Numbers a newly opened window.
    pub(crate) fn open_window(&mut self) -> u64 {
        self.windows += 1;
        self.windows
    }

    pub(crate) fn issue(
        &mut self,
        request: RequestId,
        issuer: Issuer<'_>,
        kind: RequestKind,
        now: Instant,
    ) {
        let pending = Pending {
            window: issuer.window,
            provider: issuer.provider.to_string(),
            view: issuer.view,
            kind,
            issued_at: now,
        };
        self.requests.insert(request, pending);
    }

    /// Whether Enter on a row of `issuer` must wait: the same view instance
    /// of the same window has an action of the same provider pending, issued less
    /// than [`ACTION_GATE`] ago.
    pub(crate) fn action_pending(&self, issuer: Issuer<'_>, now: Instant) -> bool {
        self.requests.values().any(|p| {
            p.window == issuer.window
                && p.kind == RequestKind::Action
                && p.provider == issuer.provider
                && p.view == issuer.view
                && now.saturating_duration_since(p.issued_at) < ACTION_GATE
        })
    }

    fn claim(&mut self, request: RequestId) -> Option<Pending> {
        self.requests.remove(&request)
    }
}

/// Who a reply belongs to.
#[derive(Clone, Copy)]
enum Origin {
    /// Window `number` issued the request, from view `instance` (`None`:
    /// the root list).
    Window { number: u64, instance: Option<u64> },
    /// The daemon issued it.
    Daemon,
}

/// A launcher window, as the controller sees it.
#[derive(Clone, Copy)]
pub(crate) struct OpenTarget {
    pub number: u64,
    pub handle: WindowHandle<LauncherView>,
}

/// What the applier needs from the controller.
pub(crate) trait Launcher {
    fn ledger(&self) -> SharedLedger;
    /// The open window, if any.
    fn window(&self) -> Option<OpenTarget>;
    /// Opens the window if it is hidden, and returns it.
    fn show_window(&mut self, cx: &mut App) -> Option<OpenTarget>;
    fn hide_window(&mut self, cx: &mut App);
}

/// Applies a [`CoreUpdate`]: the one place where module effects are applied.
/// It runs in the controller, never inside a window update.
pub(crate) fn apply_update(launcher: &mut impl Launcher, update: CoreUpdate, cx: &mut App) {
    // `None` for a reply to a window closed since; it is dropped, except
    // that it may still copy text.
    let origin = |launcher: &dyn Launcher, request: RequestId| match launcher
        .ledger()
        .borrow_mut()
        .claim(request)
    {
        None => Some(Origin::Daemon),
        Some(p) if launcher.window().is_some_and(|w| w.number == p.window) => {
            Some(Origin::Window {
                number: p.window,
                instance: p.view,
            })
        }
        Some(_) => None,
    };
    match update {
        CoreUpdate::None => {}
        CoreUpdate::ItemsChanged | CoreUpdate::Results(_) => {
            if let Some(window) = launcher.window() {
                update_window(window, cx, |host, _, cx| host.root_update(update, cx));
            }
        }
        CoreUpdate::ActionDone {
            request,
            provider,
            module_instance,
            effects,
            items_changed,
        } => {
            let origin = origin(launcher, request);
            refresh_root(launcher, items_changed, cx);
            let owner = (provider.as_str(), module_instance);
            apply_effects(launcher, origin, owner, None, effects, cx);
        }
        CoreUpdate::ViewEffects {
            request,
            provider,
            module_instance,
            view,
            effects,
            items_changed,
        } => {
            let origin = origin(launcher, request);
            refresh_root(launcher, items_changed, cx);
            let owner = (provider.as_str(), module_instance);
            apply_effects(launcher, origin, owner, Some(&view), effects, cx);
        }
        CoreUpdate::ActionFailed { request, message } => match origin(launcher, request) {
            Some(Origin::Window { number, .. }) => {
                tracing::warn!(error = %message, "action failed");
                if let Some(window) = current(launcher, number) {
                    update_window(window, cx, |host, _, cx| host.show_error_toast(message, cx));
                }
            }
            Some(Origin::Daemon) => tracing::error!("action failed: {message}"),
            None => tracing::warn!(error = %message, "action of a closed window failed"),
        },
        CoreUpdate::ModuleEffects {
            provider,
            module_instance,
            effects,
            items_changed,
        } => {
            refresh_root(launcher, items_changed, cx);
            apply_unsolicited(launcher, (&provider, module_instance), effects, cx);
        }
        CoreUpdate::ViewFailed {
            request, message, ..
        } => {
            tracing::warn!(error = %message, "view event failed");
            if let Some(Origin::Window {
                number,
                instance: Some(instance),
            }) = origin(launcher, request)
                && let Some(window) = current(launcher, number)
            {
                update_window(window, cx, |host, _, cx| {
                    if host.has_view(instance) {
                        host.show_error_toast(message, cx);
                    }
                });
            }
        }
        // The strip changed: the open window draws it again, and a waiting
        // Enter may act on it.
        CoreUpdate::Widgets => {
            if let Some(window) = launcher.window() {
                update_window(window, cx, |host, window, cx| {
                    host.strip_changed(window, cx)
                });
            }
        }
        CoreUpdate::WidgetEffects {
            request,
            provider,
            module_instance,
            effects,
            items_changed,
        } => {
            refresh_root(launcher, items_changed, cx);
            let owner = (provider.as_str(), module_instance);
            match request {
                // A user action on a tile: its effects act as an action's on the root list.
                Some(request) => {
                    let origin = origin(launcher, request);
                    apply_effects(launcher, origin, owner, None, effects, cx);
                }
                None => apply_unsolicited(launcher, owner, effects, cx),
            }
        }
        CoreUpdate::WidgetStopped { widgets } => match launcher.window() {
            Some(window) => {
                for stopped in widgets {
                    let message = format!("Widget {} stopped: {}", stopped.title, stopped.reason);
                    update_window(window, cx, |host, _, cx| host.show_error_toast(message, cx));
                }
            }
            // The core logged them already.
            None => {
                for stopped in widgets {
                    tracing::debug!(widget = %stopped.title, "widget stopped with no window: {}", stopped.reason);
                }
            }
        },
    }
}

/// The open window, if it is window `number`.
fn current(launcher: &dyn Launcher, number: u64) -> Option<OpenTarget> {
    launcher.window().filter(|w| w.number == number)
}

/// The core replaced a module's items: re-rank the root list.
fn refresh_root(launcher: &dyn Launcher, items_changed: bool, cx: &mut App) {
    if items_changed && let Some(window) = launcher.window() {
        update_window(window, cx, |host, _, cx| {
            host.root_update(CoreUpdate::ItemsChanged, cx)
        });
    }
}

/// Applies the effects of a reply of `owner` (provider, module instance), in
/// order. `view` is the pushed view the reply is for (`None`: an action of the
/// root list or of the daemon); `origin` `None` is a reply to a window closed
/// since. A view reply acts on the view instance that sent the event.
fn apply_effects(
    launcher: &mut impl Launcher,
    origin: Option<Origin>,
    owner: (&str, u64),
    view: Option<&ViewId>,
    effects: Vec<Effect>,
    cx: &mut App,
) {
    // A reply for a view instance that has been popped is dropped as a
    // whole, even when the same view was pushed again since.
    if let (Some(_), Some(Origin::Window { number, instance })) = (view, origin) {
        let on_stack = instance
            .zip(current(launcher, number))
            .and_then(|(i, w)| update_window(w, cx, |host, _, _| host.has_view(i)));
        if on_stack != Some(true) {
            tracing::debug!("reply for a popped view ignored");
            return;
        }
    }
    let (provider, module_instance) = owner;
    // A daemon run that pushes a view opens the launcher on it.
    let daemon = matches!(origin, Some(Origin::Daemon)) && view.is_none();
    let instance = match origin {
        Some(Origin::Window { instance, .. }) => instance,
        Some(Origin::Daemon) | None => None,
    };
    for effect in effects {
        // The requesting window, if it is still open.
        let window = match origin {
            Some(Origin::Window { number, .. }) => current(launcher, number),
            Some(Origin::Daemon) | None => None,
        };
        match effect {
            Effect::CopyText(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Effect::CloseWindow if window.is_some() || daemon => launcher.hide_window(cx),
            Effect::PushView(new) => {
                let target = if daemon {
                    launcher.show_window(cx)
                } else {
                    window
                };
                if let Some(target) = target {
                    update_window(target, cx, |host, window, cx| {
                        host.push_view(provider, module_instance, new, window, cx)
                    });
                } else {
                    tracing::debug!(effect = "PushView", "effect without a window ignored");
                }
            }
            Effect::UpdateView(new) if window.is_some() => {
                on(window, cx, |host, window, cx| {
                    host.update_view(owner, instance, new, window, cx)
                });
            }
            Effect::PopView if window.is_some() => {
                on(window, cx, |host, window, cx| {
                    host.pop_for_reply(provider, instance, window, cx)
                });
            }
            Effect::SetRootQuery(text) if window.is_some() => {
                on(window, cx, |host, window, cx| {
                    host.set_root_query(text, window, cx)
                });
            }
            Effect::EditHome if window.is_some() => {
                on(window, cx, |host, window, cx| {
                    host.edit_home_command(window, cx)
                });
            }
            Effect::Toast(toast) => {
                let target = if daemon { launcher.window() } else { window };
                match target {
                    Some(target) => {
                        update_window(target, cx, |host, _, cx| host.show_toast(toast, cx));
                    }
                    None if daemon => tracing::info!(title = %toast.title, "toast with no window"),
                    None => tracing::debug!(effect = "Toast", "effect without a window ignored"),
                }
            }
            // The core applies these and strips them from replies.
            Effect::ReplaceItems(_) => {
                tracing::debug!(effect = "ReplaceItems", "effect reached the UI; ignored");
            }
            // The core applies these and strips them from replies, as `ReplaceItems`.
            other @ (Effect::RedrawWidget(_) | Effect::SetWidgetImage(_)) => {
                tracing::debug!(effect = name(&other), "effect reached the UI; ignored");
            }
            other => tracing::debug!(effect = name(&other), "effect without a window ignored"),
        }
    }
}

/// Applies effects `owner` (provider, module instance) sent with no view
/// event to answer (a timer, an HTTP answer, `init`), in order. They never
/// open, hide or pop: `UpdateView` updates the topmost view with that id that
/// this instance pushed; `PushView` needs the window open with one of this
/// instance's views on top; `Toast` needs the window open; `CopyText` always
/// applies; `PopView`, `CloseWindow`, `SetRootQuery` and `EditHome` are ignored.
fn apply_unsolicited(
    launcher: &mut impl Launcher,
    owner: (&str, u64),
    effects: Vec<Effect>,
    cx: &mut App,
) {
    let (provider, module_instance) = owner;
    for effect in effects {
        let window = launcher.window();
        match effect {
            Effect::CopyText(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Effect::UpdateView(new) if window.is_some() => {
                on(window, cx, |host, window, cx| {
                    host.update_view(owner, None, new, window, cx)
                });
            }
            Effect::PushView(new) if window.is_some() => {
                on(window, cx, |host, window, cx| {
                    if host.top_owner() != Some(owner) {
                        tracing::debug!("unsolicited PushView over another view ignored");
                    } else if !host.stack().owns_above(provider, new.id()) {
                        // Ruling 9b: returning to the view would close another module's
                        // views, which nobody asked for.
                        tracing::debug!("unsolicited PushView over another module's views ignored");
                    } else {
                        host.push_view(provider, module_instance, new, window, cx);
                    }
                });
            }
            Effect::Toast(toast) if window.is_some() => {
                on(window, cx, |host, _, cx| host.show_toast(toast, cx));
            }
            Effect::PopView | Effect::CloseWindow | Effect::SetRootQuery(_) | Effect::EditHome => {
                tracing::debug!(effect = name(&effect), "unsolicited effect ignored");
            }
            Effect::ReplaceItems(_) => {
                tracing::debug!(effect = "ReplaceItems", "effect reached the UI; ignored");
            }
            // The core applies these and strips them from replies, as `ReplaceItems`.
            other @ (Effect::RedrawWidget(_) | Effect::SetWidgetImage(_)) => {
                tracing::debug!(effect = name(&other), "effect reached the UI; ignored");
            }
            other => tracing::debug!(effect = name(&other), "effect without a window ignored"),
        }
    }
}

/// Like [`update_window`], for a window that may be gone already.
fn on<R>(
    window: Option<OpenTarget>,
    cx: &mut App,
    f: impl FnOnce(&mut LauncherView, &mut Window, &mut Context<LauncherView>) -> R,
) -> Option<R> {
    update_window(window?, cx, f)
}

/// Runs `f` on the window's view; `None` when the window is gone.
fn update_window<R>(
    target: OpenTarget,
    cx: &mut App,
    f: impl FnOnce(&mut LauncherView, &mut Window, &mut Context<LauncherView>) -> R,
) -> Option<R> {
    match target.handle.update(cx, f) {
        Ok(result) => Some(result),
        Err(e) => {
            tracing::debug!(error = %e, "launcher window is gone");
            None
        }
    }
}

/// Variant name of an effect, for logs: effects carry module data.
fn name(effect: &Effect) -> &'static str {
    match effect {
        Effect::PushView(_) => "PushView",
        Effect::UpdateView(_) => "UpdateView",
        Effect::PopView => "PopView",
        Effect::CloseWindow => "CloseWindow",
        Effect::Toast(_) => "Toast",
        Effect::CopyText(_) => "CopyText",
        Effect::ReplaceItems(_) => "ReplaceItems",
        Effect::SetRootQuery(_) => "SetRootQuery",
        Effect::EditHome => "EditHome",
        Effect::RedrawWidget(_) => "RedrawWidget",
        Effect::SetWidgetImage(_) => "SetWidgetImage",
    }
}
