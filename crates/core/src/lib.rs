//! Platform-independent core of tomakast: item model, index and matching.

pub mod core;
pub mod effect;
pub mod engine;
mod host;
pub mod index;
pub mod matcher;
pub mod module;
pub mod paths;
pub mod settings;
pub mod store;
#[cfg(feature = "testing")]
pub mod testing;
pub mod usage;
pub mod view;

pub use core::{Core, CoreUpdate, SearchSnapshot};
pub use effect::{Effect, Toast, ToastKind};
pub use engine::{DYNAMIC_MATCH_SCORE, Hit, SearchEngine};
pub use host::DYNAMIC_SEARCH_DEADLINE;
pub use index::{Action, Icon, Index, Item, ItemId, ItemKind};
pub use matcher::{Matcher, Pattern};
pub use module::{CoreEvent, Module, ModuleContext, RequestId};
pub use paths::Paths;
pub use settings::{Hotkey, HotkeyError, Modifiers, Settings};
pub use usage::{
    LocalNow, RankWeights, UsageModel, UsageRecord, UsageStats, combined_score, usage_value,
};
pub use view::{
    Accessory, DetailView, FieldKind, FieldValue, FormField, FormView, GridTile, GridView,
    LineAnnotation, ListRow, ListView, ScratchpadView, SearchMode, View, ViewEvent, ViewId,
};
