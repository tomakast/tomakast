//! Render bookkeeping and frame judging: which render is the newest, whether
//! one is in flight, and what a frame that came back means for its widget.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    Applied, Provider, RENDER_INTERVAL_JITTER, Stop, StoppedWidget, Target, WidgetState,
    WidgetStore,
};
use crate::widget::{
    Frame, LATE_FRAMES_TO_STOP, MIN_RENDER_INTERVAL, WidgetContext, WidgetKey, soft_render_deadline,
};

/// A render about to be issued.
pub(crate) struct Render {
    pub(crate) seq: u64,
    target: Target,
    pub(crate) context: WidgetContext,
}

/// What [`Provider::judge`] makes of one frame.
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

    /// The next render of `key` at `now`, not yet issued: see
    /// [`issued`](Self::issued). `None` while there is no env, for an unknown
    /// or stopped widget, and within [`MIN_RENDER_INTERVAL`] (less
    /// [`RENDER_INTERVAL_JITTER`]) of the last issue.
    pub(crate) fn next_render(&self, key: &WidgetKey, now: Instant) -> Option<Render> {
        let env = self.env.as_ref()?;
        let p = self.providers.get(&key.provider)?;
        let size = p.decl(&key.widget)?.size;
        let answered = p
            .answer(&key.widget)
            .map(|a| (a.generation, a.query.as_str(), a.answer.payload.as_str()));
        let state = p.widgets.get(&key.widget)?;
        let interval = MIN_RENDER_INTERVAL - RENDER_INTERVAL_JITTER;
        let too_soon = state
            .last_issue
            .is_some_and(|last| now.saturating_duration_since(last) < interval);
        if state.is_stopped() || too_soon {
            return None;
        }
        let (target, context) = match answered {
            Some((generation, query, payload)) => (
                Target::Answer(generation),
                env.context(size, Some(query), Some(payload)),
            ),
            None => (Target::Always, env.context(size, None, None)),
        };
        Some(Render {
            seq: state.seq + 1,
            target,
            context,
        })
    }

    /// Records that `render` of `key` was issued at `now`.
    pub(crate) fn issued(&mut self, key: &WidgetKey, render: &Render, now: Instant) {
        let state = self
            .providers
            .get_mut(&key.provider)
            .and_then(|p| p.widgets.get_mut(&key.widget));
        if let Some(state) = state {
            state.seq = render.seq;
            state.target = render.target;
            state.last_issue = Some(now);
            state.dirty = false;
        }
    }

    /// Applies the frame of render `seq` of `key`.
    pub(crate) fn rendered(
        &mut self,
        key: &WidgetKey,
        seq: u64,
        took: Duration,
        mut result: Result<Frame, String>,
    ) -> Applied {
        let mut applied = Applied::default();
        // Images are sent once: they are kept even when the frame is not.
        if let Ok(frame) = &mut result {
            applied.changed |= self.store_images(&key.provider, std::mem::take(&mut frame.images));
        }
        let deadline = self.soft_deadline();
        let Some(p) = self.providers.get_mut(&key.provider) else {
            return applied;
        };
        let Some(state) = p.widgets.get_mut(&key.widget) else {
            return applied;
        };
        state.received = state.received.max(seq);
        if seq < state.seq || state.is_stopped() {
            tracing::debug!(module = %key.provider, widget = %key.widget, seq, "dropping the frame of an older render");
            return applied;
        }
        let target = state.target;
        match p.judge(&key.widget, took, deadline, result) {
            Judged::OnTime(frame) => {
                let frame = Arc::new(frame);
                match target {
                    Target::Always => {
                        if let Some(state) = p.widgets.get_mut(&key.widget) {
                            state.frame = Some(frame);
                        }
                    }
                    Target::Answer(generation) => {
                        let answered = p
                            .answers
                            .iter_mut()
                            .find(|a| a.generation == generation && a.answer.widget == key.widget);
                        match answered {
                            Some(answered) => answered.frame = frame,
                            None => {
                                tracing::debug!(module = %key.provider, widget = %key.widget, "dropping the frame of a condition tile that is gone")
                            }
                        }
                    }
                }
                applied.changed = true;
            }
            Judged::Dropped => {}
            Judged::Stopped(reason) => {
                applied.changed = true;
                applied
                    .stopped
                    .push(p.stopped(&key.widget, reason, Stop::Frames));
            }
        }
        applied
    }
}

impl WidgetStore {
    /// Stops `key`'s widget for `reason`, unless it is unknown or stopped
    /// already; its always tile leaves the strip, with its frame.
    pub(crate) fn stop(&mut self, key: &WidgetKey, reason: &str) -> Applied {
        let mut applied = Applied::default();
        let Some(p) = self.providers.get_mut(&key.provider) else {
            return applied;
        };
        if p.widgets
            .get(&key.widget)
            .is_none_or(WidgetState::is_stopped)
        {
            return applied;
        }
        applied.changed = true;
        applied
            .stopped
            .push(p.stopped(&key.widget, reason, Stop::Paint));
        if let Some(state) = p.widgets.get_mut(&key.widget) {
            state.frame = None;
        }
        applied
    }
}

impl Provider {
    /// Counts one frame of `widget`, which took `took` against `deadline`,
    /// against its run of late or failed frames.
    pub(super) fn judge(
        &mut self,
        widget: &str,
        took: Duration,
        deadline: Duration,
        result: Result<Frame, String>,
    ) -> Judged {
        let Some(state) = self.widgets.get_mut(widget) else {
            return Judged::Dropped;
        };
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

    /// Stops `widget` for `cause`: its condition tile goes; its always tile
    /// keeps its last frame, unless the paint guard stopped it.
    pub(super) fn stopped(&mut self, widget: &str, reason: &str, cause: Stop) -> StoppedWidget {
        if let Some(state) = self.widgets.get_mut(widget) {
            state.stopped = Some(cause);
        }
        self.answers.retain(|a| a.answer.widget != widget);
        let title = self.decl(widget).map_or(widget, |d| d.title.as_str());
        StoppedWidget {
            title: title.to_string(),
            reason: reason.to_string(),
        }
    }
}
