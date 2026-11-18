//! What module calls ask the UI to do.

use crate::index::Item;
use crate::view::View;

pub use crate::view::{ViewEvent, ViewId};

/// Something a module call asks the UI to do after it returns.
///
/// `UpdateView` follows the revision rule: it updates only the non-text parts
/// of the current view, and only when its `revision` matches. `ReplaceItems`
/// replaces the replying module's items; the core applies it, with the same
/// validation as `ModuleContext::replace_items`, and the UI never sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    PushView(View),
    UpdateView(View),
    PopView,
    CloseWindow,
    Toast(Toast),
    CopyText(String),
    ReplaceItems(Vec<Item>),
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
