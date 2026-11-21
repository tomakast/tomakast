//! The strip's tiles in order: at the empty query the Home tiles, in the
//! user's order; at a typed query the search's condition tiles.

use std::sync::Arc;

use super::{Provider, WidgetStore};
use crate::index::Action;
use crate::widget::{Frame, Slot, TileKey, WidgetKey, WidgetSize};

/// One tile of the strip.
#[derive(Clone, Debug, PartialEq)]
pub struct StripTile {
    pub key: TileKey,
    pub title: String,
    pub size: WidgetSize,
    /// `None` until its first frame arrives: an empty placeholder of its size.
    pub frame: Option<Arc<Frame>>,
    /// `Some` for a condition tile.
    pub condition: Option<ConditionTile>,
}

/// What a search answered for a condition tile.
#[derive(Clone, Debug, PartialEq)]
pub struct ConditionTile {
    pub actions: Vec<Action>,
    /// Answered for an older query than the latest search's: it stays on
    /// screen until its provider answers that search, but takes no user action.
    pub stale: bool,
}

impl WidgetStore {
    /// Whether `key` is a tile of the strip: at a typed query a condition
    /// tile, at the empty query a Home tile that shows.
    pub(crate) fn on_strip(&self, key: &TileKey) -> bool {
        let Some(p) = self.providers.get(&key.widget.provider) else {
            return false;
        };
        match key.slot {
            Slot::Search => self.typed && p.answer_for(key).is_some(),
            Slot::Home(_) => !self.typed && self.home_slot(key).is_some() && p.lists_home(key),
        }
    }

    /// The tiles of the strip, in order:
    /// - at the empty query, the Home tiles that can show, in the user's
    ///   order, but those the paint guard stopped;
    /// - at a typed query, the condition tiles, providers by id, each in its
    ///   answer order.
    pub(crate) fn tiles(&self) -> Vec<StripTile> {
        if !self.typed {
            return self.home_strip();
        }
        let mut providers: Vec<(&String, &Provider)> = self.providers.iter().collect();
        providers.sort_by_key(|(id, _)| *id);
        let mut tiles = Vec::new();
        for (id, p) in &providers {
            for a in &p.answers {
                let Some(decl) = p.decl(&a.answer.widget) else {
                    continue;
                };
                tiles.push(StripTile {
                    key: TileKey {
                        widget: WidgetKey::new(*id, a.answer.widget.as_str()),
                        params: a.answer.params.clone(),
                        slot: Slot::Search,
                    },
                    title: decl.title.clone(),
                    size: decl.size_or_default(a.answer.size),
                    frame: Some(a.frame.clone()),
                    condition: Some(ConditionTile {
                        actions: a.answer.actions.clone(),
                        stale: a.generation != self.generation,
                    }),
                });
            }
        }
        tiles
    }

    fn home_strip(&self) -> Vec<StripTile> {
        let mut tiles = Vec::with_capacity(self.home_tiles.len());
        for slot in &self.home_tiles {
            let Some(p) = self.providers.get(&slot.key.widget.provider) else {
                continue;
            };
            let Some(decl) = p.decl(&slot.key.widget.widget) else {
                continue;
            };
            if !p.lists_home(&slot.key) {
                continue;
            }
            tiles.push(StripTile {
                key: slot.key.clone(),
                title: decl.title.clone(),
                size: slot.size,
                frame: p.home.get(&slot.key).and_then(|s| s.frame.clone()),
                condition: None,
            });
        }
        tiles
    }
}
