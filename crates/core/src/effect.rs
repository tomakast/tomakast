//! What module calls ask the UI to do.

use crate::index::{Item, ItemId};
use crate::view::View;
use crate::widget::WidgetImage;

pub use crate::view::{ViewEvent, ViewId};

/// Something a module call asks the UI to do after it returns.
///
/// `UpdateView` follows the revision rule: it updates only the non-text parts
/// of the current view, and only when its `revision` matches. `ReplaceItems`
/// replaces the replying module's items; the core applies it, with the same
/// validation as `ModuleContext::replace_items`, and the UI never sees it.
///
/// Unsolicited effects ([`CoreEvent::ModuleEffects`](crate::CoreEvent)), sent
/// with no view event to answer: `UpdateView` targets the topmost stack entry
/// with that view id owned by the module, and is dropped if there is none;
/// `PushView` is honoured only while the window is open and the top view
/// belongs to the module; `PopView` and `CloseWindow` are ignored; `Toast`
/// shows only while the window is open; `CopyText` and `ReplaceItems` always
/// apply, and so do `RecordUsage` (for the module's own items) and `SearchAgain`; `SetRootQuery`,
/// `EditHome` and `OpenSettings` are ignored.
///
/// A plugin's answer to a widget event that is no user action (`shown`,
/// `hidden`, a hover, a press alone) is unsolicited too: the plugin host drops
/// its `PushView`, `PopView` and `CloseWindow` before the core sees them
/// (`Origin::WidgetEvent` in `tomakast-plugin-host`'s caps).
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    PushView(View),
    UpdateView(View),
    PopView,
    CloseWindow,
    Toast(Toast),
    CopyText(String),
    ReplaceItems(Vec<Item>),
    /// Internal, for built-in modules: there is no WIT case, so plugins cannot send it.
    /// It pops every pushed view (each gets `Closed`, as for Esc), puts the text in the
    /// root search field with the caret at its end, and runs the search. It is honoured
    /// in a reply to a request of the open window, and ignored unsolicited or from a
    /// daemon run.
    SetRootQuery(String),
    /// Internal, for built-in modules: there is no WIT case, so plugins cannot send it.
    /// It asks the open window to enter the edit mode of the Home tiles. It is honoured
    /// in a reply to a request of the open window, and ignored unsolicited or from a
    /// daemon run.
    EditHome,
    /// Asks for the settings window, on a page id (`None`: the last one). Internal, like
    /// `EditHome`; the launcher hides first. It is honoured in a reply to a request of the
    /// open window and from a daemon run, and ignored unsolicited.
    OpenSettings(Option<String>),
    /// Internal, for built-in modules: there is no WIT case, so plugins cannot send it.
    /// It counts `item` as opened for `query`, as a finished root-list action does, so
    /// an item opened from a module's own screen ranks by habit. The core applies it
    /// (in any reply or unsolicited) for the replying module's own items only, drops
    /// others with a warning, and the UI never sees it. Do not send it in the reply to a
    /// root-list action on the same item: the core already counts that open.
    RecordUsage {
        item: ItemId,
        query: String,
    },
    /// Internal, for built-in modules: there is no WIT case, so plugins cannot send it.
    /// It asks the module the latest root search again, as when its worker restarts: its
    /// answer to the query on screen changed (it answered [`busy`](crate::SearchAnswer)
    /// and is ready now). The core applies it (in any reply or unsolicited) and the UI
    /// never sees it; it is ignored while the window is closed.
    SearchAgain,
    /// Asks for a new render of every tile of one widget (by widget id), whatever its
    /// parameters.
    RedrawWidget(String),
    /// Makes an image available to the module's frames by its id.
    SetWidgetImage(WidgetImage),
    /// Plugins only: what an unset preference `key` (one the plugin declares) stands for,
    /// for the settings window to show (empty: none, taking back one told before). The plugin
    /// host takes it out of the reply and keeps it in the plugin catalog; the core and the UI never see it from a plugin, and the core
    /// drops one from a built-in module.
    PreferenceDefault {
        key: String,
        value: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: String,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Progress,
}
