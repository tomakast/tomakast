//! The strip's tiles in order: condition tiles first, then the always tiles
//! in the user's order, then in approval order; `[widgets] hidden` hides any.

use std::sync::Arc;

use super::{Provider, WidgetStore};
use crate::index::Action;
use crate::settings::WidgetSettings;
use crate::widget::{Frame, WidgetKey, WidgetSize};

/// One tile of the strip.
#[derive(Clone, Debug, PartialEq)]
pub struct StripTile {
    pub key: WidgetKey,
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
    pub payload: String,
    pub actions: Vec<Action>,
    /// Answered for an older query than the latest search's: it stays on
    /// screen until its provider answers that search, but takes no user action.
    pub stale: bool,
}

impl WidgetStore {
    /// Returns whether the settings changed.
    pub(crate) fn set_settings(&mut self, settings: WidgetSettings) -> bool {
        if self.settings == settings {
            return false;
        }
        self.settings = settings;
        true
    }

    /// Whether `key` is a tile of the strip: answered, or an always tile, and not hidden.
    pub(crate) fn on_strip(&self, key: &WidgetKey) -> bool {
        !self.settings.hidden.contains(key)
            && self
                .providers
                .get(&key.provider)
                .is_some_and(|p| p.answer(&key.widget).is_some() || p.lists_always(&key.widget))
    }

    /// The tiles of the strip, in order:
    /// - the condition tiles, providers by id, each in its answer order;
    /// - the always tiles not answered, but those the paint guard stopped:
    ///   first those of `[widgets] order`, in that order; then by approval
    ///   time (built-ins first), provider id and the module's declared order.
    pub(crate) fn tiles(&self) -> Vec<StripTile> {
        let mut providers: Vec<(&String, &Provider)> = self.providers.iter().collect();
        providers.sort_by_key(|(id, _)| *id);
        let hidden = |key: &WidgetKey| self.settings.hidden.contains(key);
        let mut tiles = Vec::new();
        for (id, p) in &providers {
            for a in &p.answers {
                let key = WidgetKey::new(*id, a.answer.widget.as_str());
                let Some(decl) = p.decl(&a.answer.widget) else {
                    continue;
                };
                if hidden(&key) {
                    continue;
                }
                tiles.push(StripTile {
                    key,
                    title: decl.title.clone(),
                    size: decl.size,
                    frame: Some(a.frame.clone()),
                    condition: Some(ConditionTile {
                        payload: a.answer.payload.clone(),
                        actions: a.answer.actions.clone(),
                        stale: a.generation != self.generation,
                    }),
                });
            }
        }
        let mut always = Vec::new();
        for (id, p) in &providers {
            for widget in &p.always {
                let key = WidgetKey::new(*id, widget.as_str());
                if p.answer(widget).is_some() || !p.lists_always(widget) || hidden(&key) {
                    continue;
                }
                let Some(declared) = p.decls.iter().position(|d| d.id == *widget) else {
                    continue;
                };
                let listed = self.settings.order.iter().position(|k| *k == key);
                let rank = (listed.unwrap_or(usize::MAX), p.approved_at, *id, declared);
                always.push((rank, key, *p));
            }
        }
        always.sort_by_key(|(rank, ..)| *rank);
        tiles.extend(always.into_iter().map(|(rank, key, p)| {
            let decl = &p.decls[rank.3];
            StripTile {
                frame: p.widgets.get(&key.widget).and_then(|s| s.frame.clone()),
                key,
                title: decl.title.clone(),
                size: decl.size,
                condition: None,
            }
        }));
        tiles
    }
}
