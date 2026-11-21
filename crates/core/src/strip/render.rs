//! Render bookkeeping and frame judging: which render is the newest, whether
//! one is in flight, and what a frame that came back means for its tile.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    Applied, Provider, RENDER_INTERVAL_JITTER, Stop, StoppedWidget, Target, WidgetState,
    WidgetStore,
};
use crate::widget::{
    Frame, LATE_FRAMES_TO_STOP, MIN_RENDER_INTERVAL, Slot, TileKey, WidgetContext,
    soft_render_deadline,
};

/// A render about to be issued.
pub(crate) struct Render {
    pub(crate) seq: u64,
    target: Target,
    pub(crate) context: WidgetContext,
}

/// What [`judge`] makes of one frame.
pub(super) enum Judged {
    OnTime(Frame),
    Dropped,
    Stopped(&'static str),
}

impl WidgetStore {
    /// The soft deadline a frame is held to: [`soft_render_deadline`], or
    /// the one a test set.
    pub(super) fn soft_deadline(&self) -> Duration {
        #[cfg(any(test, feature = "testing"))]
        if let Some(deadline) = self.soft_deadline {
            return deadline;
        }
        soft_render_deadline()
    }

    /// Holds frames to `deadline` in place of [`soft_render_deadline`].
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set_soft_deadline(&mut self, deadline: Duration) {
        self.soft_deadline = Some(deadline);
    }

    /// The next render of tile `key` at `now`, not yet issued: see
    /// [`issued`](Self::issued). `None` while there is no env, for a tile
    /// that is not one (a Home tile that cannot show, a search tile not
    /// answered with `key`'s parameters), for a stopped one, and within
    /// [`MIN_RENDER_INTERVAL`] (less [`RENDER_INTERVAL_JITTER`]) of its last
    /// issue.
    pub(crate) fn next_render(&self, key: &TileKey, now: Instant) -> Option<Render> {
        let env = self.env.as_ref()?;
        let p = self.providers.get(&key.widget.provider)?;
        let (target, context) = match key.slot {
            Slot::Search => {
                let answered = p.answer_for(key)?;
                let size = p
                    .decl(&key.widget.widget)?
                    .size_or_default(answered.answer.size);
                (
                    Target::Answer(answered.generation),
                    env.context(size, Some(&answered.query), key.params.clone()),
                )
            }
            Slot::Home(_) => {
                let size = self.home_slot(key)?.size;
                (Target::Home, env.context(size, None, key.params.clone()))
            }
        };
        let state = self.state(key)?;
        let interval = MIN_RENDER_INTERVAL - RENDER_INTERVAL_JITTER;
        let too_soon = state
            .last_issue
            .is_some_and(|last| now.saturating_duration_since(last) < interval);
        if state.is_stopped() || too_soon {
            return None;
        }
        Some(Render {
            seq: state.seq + 1,
            target,
            context,
        })
    }

    /// Records that `render` of `key` was issued at `now`.
    pub(crate) fn issued(&mut self, key: &TileKey, render: &Render, now: Instant) {
        if let Some(state) = self.state_mut(key) {
            state.seq = render.seq;
            state.target = render.target;
            state.last_issue = Some(now);
            state.dirty = false;
        }
    }

    /// Applies the frame of render `seq` of `key`.
    pub(crate) fn rendered(
        &mut self,
        key: &TileKey,
        seq: u64,
        took: Duration,
        mut result: Result<Frame, String>,
    ) -> Applied {
        let mut applied = Applied::default();
        let provider = key.widget.provider.as_str();
        // Images are sent once: they are kept even when the frame is not.
        if let Ok(frame) = &mut result {
            applied.changed |= self.store_images(provider, std::mem::take(&mut frame.images));
        }
        let deadline = self.soft_deadline();
        let Some(state) = self.state_mut(key) else {
            return applied;
        };
        state.received = state.received.max(seq);
        if seq < state.seq || state.is_stopped() {
            tracing::debug!(tile = %key, seq, "dropping the frame of an older render");
            return applied;
        }
        let target = state.target;
        let judged = judge(state, &key.widget.widget, took, deadline, result);
        let Some(p) = self.providers.get_mut(provider) else {
            return applied;
        };
        match judged {
            Judged::OnTime(frame) => {
                let frame = Arc::new(frame);
                match target {
                    Target::Home => {
                        if let Some(state) = p.home.get_mut(key) {
                            state.frame = Some(frame);
                        }
                    }
                    Target::Answer(generation) => {
                        let answered = p.answers.iter_mut().find(|a| {
                            a.generation == generation
                                && a.answer.widget == key.widget.widget
                                && a.answer.params == key.params
                        });
                        match answered {
                            Some(answered) => answered.frame = frame,
                            None => {
                                tracing::debug!(tile = %key, "dropping the frame of a condition tile that is gone")
                            }
                        }
                    }
                }
                applied.changed = true;
            }
            Judged::Dropped => {}
            Judged::Stopped(reason) => {
                applied.changed = true;
                applied.stopped.push(p.stop_tile(key, reason, Stop::Frames));
            }
        }
        applied
    }

    /// Stops tile `key` for `reason`, unless it is not one or is stopped
    /// already; a Home tile leaves the strip, with its frame.
    pub(crate) fn stop(&mut self, key: &TileKey, reason: &str) -> Applied {
        let mut applied = Applied::default();
        if self.state(key).is_none_or(WidgetState::is_stopped) {
            return applied;
        }
        let Some(p) = self.providers.get_mut(&key.widget.provider) else {
            return applied;
        };
        applied.changed = true;
        applied.stopped.push(p.stop_tile(key, reason, Stop::Paint));
        if let Some(state) = p.home.get_mut(key) {
            state.frame = None;
        }
        applied
    }
}

/// Counts one frame of `widget`, which took `took` against `deadline`,
/// against `state`'s run of late or failed frames.
pub(super) fn judge(
    state: &mut WidgetState,
    widget: &str,
    took: Duration,
    deadline: Duration,
    result: Result<Frame, String>,
) -> Judged {
    let reason = match result {
        Ok(frame) if took <= deadline => {
            state.late = 0;
            return Judged::OnTime(frame);
        }
        Ok(_) => {
            tracing::debug!(%widget, ms = took.as_secs_f64() * 1000.0, "dropping a late frame");
            "it took longer than 16 ms to draw 5 times in a row"
        }
        Err(message) => {
            tracing::debug!(%widget, "dropping a failed frame: {message}");
            "it failed to draw 5 times in a row"
        }
    };
    state.late += 1;
    if state.late >= LATE_FRAMES_TO_STOP {
        Judged::Stopped(reason)
    } else {
        Judged::Dropped
    }
}

impl Provider {
    /// Stops tile `key` for `cause`: a condition tile goes; a Home tile keeps
    /// its last frame, unless the paint guard stopped it.
    pub(super) fn stop_tile(&mut self, key: &TileKey, reason: &str, cause: Stop) -> StoppedWidget {
        let widget = key.widget.widget.as_str();
        let state = match key.slot {
            Slot::Search => self.search.get_mut(widget),
            Slot::Home(_) => self.home.get_mut(key),
        };
        if let Some(state) = state {
            state.stopped = Some(cause);
        }
        if key.slot == Slot::Search {
            self.answers.retain(|a| a.answer.widget != widget);
        }
        let title = self.decl(widget).map_or(widget, |d| d.title.as_str());
        StoppedWidget {
            title: title.to_string(),
            reason: reason.to_string(),
        }
    }
}
