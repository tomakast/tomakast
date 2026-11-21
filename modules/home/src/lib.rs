//! `home` module: the "Edit Home" command. It only asks the launcher window to enter the
//! edit mode of the Home tiles (the internal `Effect::EditHome`); the window does the rest.

use anyhow::bail;
use tomakast_core::{Action, Effect, Item, ItemId, ItemKind, Module, ModuleContext};

/// The provider id; no plugin may take it.
pub const PROVIDER: &str = "home";

const COMMAND: &str = "edit-home";
const ACTION: &str = "edit";

#[derive(Default)]
pub struct HomeModule;

impl HomeModule {
    pub fn new() -> HomeModule {
        HomeModule
    }
}

fn edit_home_item() -> Item {
    Item {
        id: ItemId::new(PROVIDER, COMMAND),
        title: tomakast_core::EDIT_HOME.to_string(),
        subtitle: Some("Rearrange or remove the tiles on Home".to_string()),
        keywords: ["home", "tiles", "edit", "rearrange"]
            .map(String::from)
            .to_vec(),
        icon: None,
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: ACTION.to_string(),
            title: "Edit Home".to_string(),
            shortcut: None,
        }],
    }
}

impl Module for HomeModule {
    fn id(&self) -> &str {
        PROVIDER
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.replace_items(vec![edit_home_item()]);
    }

    fn run_action(&mut self, _item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        match action_id {
            ACTION => Ok(vec![Effect::EditHome]),
            other => bail!("unknown action {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_publishes_edit_home_and_its_action_asks_for_the_mode() {
        let item = edit_home_item();
        assert_eq!(item.id, ItemId::new("home", "edit-home"));
        assert!(item.id.is_valid());
        assert_eq!(item.title, "Edit Home");
        assert_eq!(item.kind, ItemKind::Command);
        assert_eq!(item.actions.len(), 1);
        let mut m = HomeModule::new();
        let effects = m.run_action(&item, &item.actions[0].id).unwrap();
        assert_eq!(effects, vec![Effect::EditHome]);
        assert!(m.run_action(&item, "nope").is_err());
    }
}
