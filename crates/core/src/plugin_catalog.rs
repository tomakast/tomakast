//! The list of every installed plugin that the plugin manager publishes and the settings
//! window shows: what a plugin is, which state it is in, and what it declares.

use crate::settings_schema::ValueKind;
use crate::widget::WidgetDecl;

/// Every installed plugin, sorted by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginCatalog {
    pub plugins: Vec<CatalogPlugin>,
    /// Set when the plugins folder cannot be listed; there are no plugin pages then.
    pub unavailable: Option<String>,
}

/// One plugin folder.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogPlugin {
    /// The folder name when the manifest could not be read.
    pub id: String,
    /// The folder name when the manifest could not be read.
    pub name: String,
    /// Empty when the manifest could not be read.
    pub version: String,
    pub author: Option<String>,
    pub state: PluginState,
    pub preferences: Vec<PluginPreference>,
    pub commands: Vec<CatalogCommand>,
    pub widgets: Vec<WidgetDecl>,
    /// The titles the manifest gives the widgets' params, for the settings window. Kept
    /// beside `widgets`, which hold only the keys a tile is drawn with.
    pub param_titles: Vec<ParamTitle>,
}

impl CatalogPlugin {
    /// The title of param `key` of widget `widget`, when the manifest gives one.
    pub fn param_title(&self, widget: &str, key: &str) -> Option<&str> {
        self.param_titles
            .iter()
            .find(|t| t.widget == widget && t.key == key)
            .map(|t| t.title.as_str())
    }
}

/// The title of a param a widget declares (`[[widgets.params]]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamTitle {
    pub widget: String,
    pub key: String,
    pub title: String,
}

/// What became of a plugin folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginState {
    /// Approved and loaded.
    Running,
    /// Waiting for the user's decision.
    Review,
    /// The user denied it.
    Denied,
    /// The plugin runtime is down, so nothing runs; the text says why.
    NotRunning(String),
    /// Approved, but it did not load or it broke; the text says why.
    Failed(String),
    /// The folder could not be read as a plugin; the text says why.
    ManifestError(String),
    /// The user turned it off (`[enabled_plugins]`): not gated, not loaded. Its approval
    /// stays as it was.
    Off,
}

/// A preference a plugin declares in its manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginPreference {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: ValueKind,
    pub default: Option<String>,
    /// What the running plugin says an unset value stands for (its `preference-default`),
    /// the last it said; `None` until it says.
    pub reported_default: Option<String>,
}

impl PluginPreference {
    /// The value in effect while the file has none, when it is known: what the plugin said,
    /// else the manifest's default.
    pub fn shown_default(&self) -> Option<&str> {
        self.reported_default.as_deref().or(self.default.as_deref())
    }
}

/// A command a plugin declares in its manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogCommand {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
}
