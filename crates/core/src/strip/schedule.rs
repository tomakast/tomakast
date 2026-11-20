//! What is on screen, and what a tick of the UI's frame clock renders.

use std::time::Instant;

use super::{Render, Stop, WidgetEnv, WidgetStore};
use crate::widget::{PointerAt, WidgetKey};

/// A tile the UI draws on screen this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleTile {
    pub key: WidgetKey,
    /// The focused component of the tile, if any.
    pub focused: Option<String>,
    /// Where the pointer is over the tile, if it is.
    pub pointer: Option<PointerAt>,
}

/// What one tick does: the events to send, then the renders to issue.
#[derive(Default)]
pub(crate) struct Tick {
    /// Tiles that left the screen.
    pub(crate) hidden: Vec<WidgetKey>,
    /// Tiles that came on it.
    pub(crate) shown: Vec<WidgetKey>,
    pub(crate) renders: Vec<(WidgetKey, Render)>,
}

impl WidgetStore {
    /// Sets the env; one that differs from the last asks every tile on
    /// screen to draw again. Returns whether any was asked.
    pub(crate) fn set_env(&mut self, env: WidgetEnv) -> bool {
        if self.env.as_ref() == Some(&env) {
            return false;
        }
        self.env = Some(env);
        let shown: Vec<WidgetKey> = self.shown.keys().cloned().collect();
        let mut dirty = false;
        for key in shown {
            dirty |= self.mark_dirty(&key.provider, &key.widget);
        }
        dirty
    }

    /// Takes `visible` as what is on screen at `now`: tiles that came on it
    /// are recorded and asked to draw, and those that left are forgotten. A
    /// visible tile that is not on the strip (yet, or any more) is ignored.
    /// Then each visible tile renders when all of these hold:
    /// - it is asked to draw again, or its last frame animates;
    /// - it has no render in flight;
    /// - its last issue is at least [`MIN_RENDER_INTERVAL`](crate::MIN_RENDER_INTERVAL)
    ///   (less [`RENDER_INTERVAL_JITTER`](super::RENDER_INTERVAL_JITTER)) ago;
    /// - it is not stopped.
    ///
    /// Record each issued render with [`issued`](Self::issued).
    pub(crate) fn tick(&mut self, now: Instant, visible: &[VisibleTile]) -> Tick {
        let mut on_screen: Vec<&VisibleTile> = Vec::with_capacity(visible.len());
        for tile in visible {
            if self.on_strip(&tile.key) && !on_screen.iter().any(|t| t.key == tile.key) {
                on_screen.push(tile);
            }
        }
        let mut tick = Tick::default();
        self.shown.retain(|key, _| {
            let stays = on_screen.iter().any(|t| t.key == *key);
            if !stays {
                tick.hidden.push(key.clone());
            }
            stays
        });
        tick.hidden.sort();
        for tile in &on_screen {
            if !self.shown.contains_key(&tile.key) {
                self.shown.insert(tile.key.clone(), now);
                self.mark_dirty(&tile.key.provider, &tile.key.widget);
                tick.shown.push(tile.key.clone());
            }
        }
        for tile in on_screen {
            if let Some(render) = self.tick_render(tile, now) {
                tick.renders.push((tile.key.clone(), render));
            }
        }
        tick
    }

    /// The render a tick issues for `tile` at `now`, if it renders.
    fn tick_render(&self, tile: &VisibleTile, now: Instant) -> Option<Render> {
        let state = self.state(&tile.key)?;
        let animates = self.frame(&tile.key).is_some_and(|f| f.animate);
        if !(state.dirty || animates) || state.in_flight() {
            return None;
        }
        let mut render = self.next_render(&tile.key, now)?;
        let shown_at = self.shown.get(&tile.key).copied().unwrap_or(now);
        let shown_for = now.saturating_duration_since(shown_at).as_millis();
        render.context.millis_shown = u64::try_from(shown_for).unwrap_or(u64::MAX);
        render.context.focused = tile.focused.clone();
        render.context.pointer = tile.pointer.clone();
        Some(render)
    }

    /// Whether a tile on screen still wants frames: asked to draw again,
    /// animating, or waiting for a render. A stopped tile wants none.
    pub(crate) fn busy(&self) -> bool {
        self.env.is_some()
            && self.shown.keys().any(|key| {
                self.state(key).is_some_and(|state| {
                    !state.is_stopped()
                        && (state.dirty
                            || state.in_flight()
                            || self.frame(key).is_some_and(|f| f.animate))
                })
            })
    }

    /// Takes every tile off screen; returns them, to be told.
    pub(crate) fn hide_all(&mut self) -> Vec<WidgetKey> {
        let mut hidden: Vec<WidgetKey> = self.shown.drain().map(|(key, _)| key).collect();
        hidden.sort();
        hidden
    }

    /// The launcher hid (after [`hide_all`](Self::hide_all)): drops every
    /// condition tile, and keeps none answered until the next search, so the
    /// next showing never flashes the last query's. A widget stopped for its
    /// frames as a condition tile (not an always tile, and not by the paint
    /// guard) may draw again: a load spike while typing does not stop it for
    /// the session. Returns whether the strip changed.
    pub(crate) fn close(&mut self) -> bool {
        self.closed = true;
        let mut changed = false;
        for p in self.providers.values_mut() {
            changed |= !p.answers.is_empty();
            p.drop_answers();
            for (id, state) in &mut p.widgets {
                if state.stopped == Some(Stop::Frames) && !p.always.contains(id) {
                    state.stopped = None;
                    state.late = 0;
                }
            }
        }
        changed
    }
}
