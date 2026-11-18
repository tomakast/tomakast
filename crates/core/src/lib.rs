//! Platform-independent core of tomakast: item model, index and matching.

pub mod core;
pub mod engine;
pub mod index;
pub mod matcher;
pub mod module;
pub mod paths;
pub mod settings;
pub mod store;
#[cfg(feature = "testing")]
pub mod testing;
pub mod usage;

pub use core::Core;
pub use engine::{Hit, SearchEngine};
pub use index::{Action, Icon, Index, Item, ItemId, ItemKind};
pub use matcher::{Matcher, Pattern};
pub use module::{ActionOutcome, CoreEvent, Module, ModuleContext};
pub use paths::Paths;
pub use settings::{Hotkey, HotkeyError, Modifiers, Settings};
pub use usage::{
    LocalNow, RankWeights, UsageModel, UsageRecord, UsageStats, combined_score, usage_value,
};
