//! `settings` module: the "Settings" and "Set Leader Key…" items. They only ask for the
//! settings window (the internal `Effect::OpenSettings`), the second on its Hotkeys page; the
//! launcher hides and the app opens the window.

use anyhow::bail;
use tomakast_core::{Action, Effect, Icon, Item, ItemId, ItemKind, Module, ModuleContext};

/// The provider id; no plugin may take it.
pub const PROVIDER: &str = "settings";

const ITEM: &str = "open";
const LEADER_ITEM: &str = "leader";
const ACTION: &str = "open";
/// The settings window's Hotkeys page, which holds the leader key.
const HOTKEYS_PAGE: &str = "hotkeys";

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

fn leader_item() -> Item {
    Item {
        id: ItemId::new(PROVIDER, LEADER_ITEM),
        title: "Set Leader Key…".to_string(),
        subtitle: Some("Press it with another key to run that key's hotkey".to_string()),
        keywords: ["leader", "hotkey", "shortcut", "keyboard"]
            .map(String::from)
            .to_vec(),
        icon: Some(Icon::Named("keyboard".to_string())),
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: ACTION.to_string(),
            title: "Open Hotkey Settings".to_string(),
            shortcut: None,
        }],
    }
}

impl Module for SettingsModule {
    fn id(&self) -> &str {
        PROVIDER
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.replace_items(vec![open_item(), leader_item()]);
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        let page =
            (item.id == ItemId::new(PROVIDER, LEADER_ITEM)).then(|| HOTKEYS_PAGE.to_string());
        match action_id {
            ACTION => Ok(vec![Effect::OpenSettings(page)]),
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

    #[test]
    fn leader_item_opens_the_hotkeys_page() {
        let item = leader_item();
        assert_eq!(item.id, ItemId::new("settings", "leader"));
        assert!(item.id.is_valid());
        assert_eq!(item.title, "Set Leader Key…");
        assert_eq!(
            item.subtitle.as_deref(),
            Some("Press it with another key to run that key's hotkey")
        );
        assert_eq!(
            item.keywords,
            ["leader", "hotkey", "shortcut", "keyboard"].map(String::from)
        );
        assert_eq!(item.kind, ItemKind::Command);
        let mut m = SettingsModule::new();
        let effects = m.run_action(&item, "open").unwrap();
        assert_eq!(effects, vec![Effect::OpenSettings(Some("hotkeys".into()))]);
    }
}
