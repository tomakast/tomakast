//! `settings` module: the "Settings" item. It only asks for the settings window (the
//! internal `Effect::OpenSettings`); the launcher hides and the app opens the window.

use anyhow::bail;
use tomakast_core::{Action, Effect, Icon, Item, ItemId, ItemKind, Module, ModuleContext};

/// The provider id; no plugin may take it.
pub const PROVIDER: &str = "settings";

const ITEM: &str = "open";
const ACTION: &str = "open";

#[derive(Default)]
pub struct SettingsModule;

impl SettingsModule {
    pub fn new() -> SettingsModule {
        SettingsModule
    }
}

fn open_item() -> Item {
    Item {
        id: ItemId::new(PROVIDER, ITEM),
        title: "Settings".to_string(),
        subtitle: Some("Launcher, plugins, hotkeys and Home".to_string()),
        keywords: ["settings", "preferences", "config", "options"]
            .map(String::from)
            .to_vec(),
        icon: Some(Icon::Named("settings".to_string())),
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: ACTION.to_string(),
            title: "Open Settings".to_string(),
            shortcut: None,
        }],
    }
}

impl Module for SettingsModule {
    fn id(&self) -> &str {
        PROVIDER
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.replace_items(vec![open_item()]);
    }

    fn run_action(&mut self, _item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        match action_id {
            ACTION => Ok(vec![Effect::OpenSettings(None)]),
            other => bail!("unknown action {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_runs_open_settings() {
        let item = open_item();
        assert_eq!(item.id, ItemId::new("settings", "open"));
        assert!(item.id.is_valid());
        assert_eq!(item.title, "Settings");
        assert_eq!(item.kind, ItemKind::Command);
        assert_eq!(item.actions.len(), 1);
        let mut m = SettingsModule::new();
        let effects = m.run_action(&item, &item.actions[0].id).unwrap();
        assert_eq!(effects, vec![Effect::OpenSettings(None)]);
        assert!(m.run_action(&item, "nope").is_err());
    }
}
