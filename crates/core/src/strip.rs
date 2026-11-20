//! The widget store: what every provider declared, its always tiles, its
//! condition tiles, the last frame of every widget and the provider's images.
//!
//! Its parts: [`render`] judges frames and keeps the render bookkeeping,
//! [`order`] lays the tiles out in strip order, and [`schedule`] follows what
//! is on screen and decides what a tick renders.

mod order;
mod render;
mod schedule;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::index::Action;
use crate::module::RenderedAnswer;
use crate::settings::WidgetSettings;
use crate::widget::{
    Frame, WidgetAnswer, WidgetContext, WidgetDecl, WidgetImage, WidgetKey, WidgetSize, WidgetTheme,
};

pub use order::{ConditionTile, StripTile};
use render::Judged;
pub(crate) use render::Render;
pub(crate) use schedule::Tick;
pub use schedule::VisibleTile;

/// How much earlier than [`MIN_RENDER_INTERVAL`](crate::MIN_RENDER_INTERVAL)
/// after the last issue a render may still issue, so a frame clock's jitter
/// does not skip a frame.
pub const RENDER_INTERVAL_JITTER: Duration = Duration::from_micros(1_500);

/// What the UI tells the core about the tiles it draws: the size of a single
/// and of a double tile, the display scale and the theme.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetEnv {
    pub single: (f32, f32),
    pub double: (f32, f32),
    pub scale: f32,
    pub theme: WidgetTheme,
}

impl WidgetEnv {
    /// The context of a tile of `size`, before it is shown, focused or pointed at.
    pub(crate) fn context(
        &self,
        size: WidgetSize,
        query: Option<&str>,
        payload: Option<&str>,
    ) -> WidgetContext {
        let (width, height) = match size {
            WidgetSize::Single => self.single,
            WidgetSize::Double => self.double,
        };
        WidgetContext {
            width,
            height,
            scale: self.scale,
            theme: self.theme.clone(),
            millis_shown: 0,
            focused: None,
            pointer: None,
            query: query.map(str::to_string),
            payload: payload.map(str::to_string),
        }
    }
}

/// A widget that stopped rendering: its title and why.
#[derive(Clone, Debug, PartialEq)]
pub struct StoppedWidget {
    pub title: String,
    pub reason: String,
}

/// What applying a reply changed.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    /// The strip changed: the UI should re-read it.
    pub(crate) changed: bool,
    pub(crate) stopped: Vec<StoppedWidget>,
}

#[derive(Default)]
pub(crate) struct WidgetStore {
    env: Option<WidgetEnv>,
    settings: WidgetSettings,
    providers: HashMap<String, Provider>,
    /// Version of the last image stored, of any provider: a version is never
    /// reused, not even by a reloaded provider.
    image_version: u64,
    /// The latest search generation; a condition tile answered for an older
    /// one is stale.
    generation: u64,
    /// The launcher hid since the latest search: no widget answer is kept
    /// until the next one.
    closed: bool,
    /// The tiles on screen, with when each came on it.
    shown: HashMap<WidgetKey, Instant>,
    /// By provider: the widgets that were on the strip when an instance of it
    /// was dropped. A user action on one of them, while the provider's
    /// current instance does not show it, would reach an instance that never
    /// drew it. It never shrinks, on purpose: a tile the new instance
    /// shows again is on the strip, which lets its events through, so no
    /// entry needs removing. It is bounded by the 16 widgets a plugin may
    /// declare, per provider id.
    gone: HashMap<String, HashSet<String>>,
    /// A test's soft render deadline in place of [`soft_render_deadline`].
    #[cfg(any(test, feature = "testing"))]
    soft_deadline: Option<Duration>,
}

struct Provider {
    decls: Vec<WidgetDecl>,
    /// Orders the provider's always tiles; `None` for a built-in module.
    approved_at: Option<i64>,
    /// Ids of the always tiles, in the module's order; declared ids only.
    always: Vec<String>,
    /// By declared id.
    widgets: HashMap<String, WidgetState>,
    /// Condition tiles of the latest applied answer, one per widget.
    answers: Vec<Answered>,
    /// By image id, with its version.
    images: HashMap<String, (u64, WidgetImage)>,
    /// The latest search generation asked of this instance with an env, so
    /// one it answers with tiles; 0 for none.
    asked: u64,
    /// The latest search generation whose widget answers were applied.
    answered: u64,
}

/// A condition tile: a search's widget answer and its frame.
struct Answered {
    generation: u64,
    query: String,
    answer: WidgetAnswer,
    frame: Arc<Frame>,
}

#[derive(Default)]
struct WidgetState {
    /// The last frame drawn as an always tile.
    frame: Option<Arc<Frame>>,
    /// The newest render issued; an older one's frame is dropped.
    seq: u64,
    /// The newest render whose reply came, of any outcome.
    received: u64,
    /// What the newest render draws.
    target: Target,
    last_issue: Option<Instant>,
    /// Asked to draw again.
    dirty: bool,
    /// Late or failed frames in a row.
    late: u32,
    /// Stopped: it renders no more, for this module instance, or, stopped
    /// for its frames as a condition tile only, until the launcher hides.
    stopped: Option<Stop>,
}

/// Why a widget stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    /// Late or failed frames in a row; it keeps its last frame.
    Frames,
    /// The UI's paint guard: its frame is gone, and its always tile leaves the strip.
    Paint,
}

impl WidgetState {
    fn in_flight(&self) -> bool {
        self.seq > self.received
    }

    fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Target {
    #[default]
    Always,
    /// The condition tile answered for this search generation.
    Answer(u64),
}

impl WidgetStore {
    pub(crate) fn env(&self) -> Option<&WidgetEnv> {
        self.env.as_ref()
    }

    /// Starts `provider`'s widgets afresh with `decls`; a duplicate id keeps the first.
    pub(crate) fn declare(
        &mut self,
        provider: &str,
        decls: Vec<WidgetDecl>,
        approved_at: Option<i64>,
    ) {
        let mut kept: Vec<WidgetDecl> = Vec::with_capacity(decls.len());
        for decl in decls {
            if kept.iter().any(|d| d.id == decl.id) {
                tracing::warn!(module = %provider, widget = %decl.id, "dropping a widget declared twice");
            } else {
                kept.push(decl);
            }
        }
        let widgets = kept
            .iter()
            .map(|d| (d.id.clone(), WidgetState::default()))
            .collect();
        self.providers.insert(
            provider.to_string(),
            Provider {
                decls: kept,
                approved_at,
                always: Vec::new(),
                widgets,
                answers: Vec::new(),
                images: HashMap::new(),
                asked: 0,
                answered: 0,
            },
        );
    }

    /// Forgets everything of `provider`, and that its tiles were on screen;
    /// returns whether it had any tile to show. Its tiles on the strip are
    /// remembered as gone (see [`is_gone`](Self::is_gone)).
    pub(crate) fn forget(&mut self, provider: &str) -> bool {
        self.shown.retain(|key, _| key.provider != provider);
        let Some(p) = self.providers.remove(provider) else {
            return false;
        };
        let tiles: Vec<String> = p
            .always
            .into_iter()
            .chain(p.answers.into_iter().map(|a| a.answer.widget))
            .collect();
        if tiles.is_empty() {
            return false;
        }
        self.gone
            .entry(provider.to_string())
            .or_default()
            .extend(tiles);
        true
    }

    pub(crate) fn is_declared(&self, key: &WidgetKey) -> bool {
        self.providers
            .get(&key.provider)
            .is_some_and(|p| p.widgets.contains_key(&key.widget))
    }

    /// Whether `key` was on the strip when its provider's instance was
    /// dropped, and the current instance does not show it.
    pub(crate) fn is_gone(&self, key: &WidgetKey) -> bool {
        self.gone
            .get(&key.provider)
            .is_some_and(|gone| gone.contains(&key.widget))
            && !self.on_strip(key)
    }

    pub(crate) fn always(&self, provider: &str) -> Vec<String> {
        self.providers
            .get(provider)
            .map(|p| p.always.clone())
            .unwrap_or_default()
    }

    /// The condition tile's frame if `key` is answered, else its last always frame.
    pub(crate) fn frame(&self, key: &WidgetKey) -> Option<Arc<Frame>> {
        let p = self.providers.get(&key.provider)?;
        if let Some(answered) = p.answer(&key.widget) {
            return Some(answered.frame.clone());
        }
        p.widgets.get(&key.widget)?.frame.clone()
    }

    pub(crate) fn image(&self, provider: &str, id: &str) -> Option<(u64, WidgetImage)> {
        self.providers.get(provider)?.images.get(id).cloned()
    }

    /// The payload of `key`'s condition tile, if it is answered.
    pub(crate) fn payload(&self, key: &WidgetKey) -> Option<String> {
        let p = self.providers.get(&key.provider)?;
        p.answer(&key.widget).map(|a| a.answer.payload.clone())
    }

    /// The query `key`'s condition tile was answered for, if it is answered.
    pub(crate) fn answered_query(&self, key: &WidgetKey) -> Option<String> {
        let p = self.providers.get(&key.provider)?;
        p.answer(&key.widget).map(|a| a.query.clone())
    }

    /// Whether `key` is a condition tile answered for an older search.
    pub(crate) fn is_stale(&self, key: &WidgetKey) -> bool {
        self.providers
            .get(&key.provider)
            .and_then(|p| p.answer(&key.widget))
            .is_some_and(|a| a.generation != self.generation)
    }

    /// The actions of `key`: its condition tile's (none while stale), else
    /// those of its last always frame.
    pub(crate) fn actions(&self, key: &WidgetKey) -> Vec<Action> {
        let Some(p) = self.providers.get(&key.provider) else {
            return Vec::new();
        };
        match p.answer(&key.widget) {
            Some(a) if a.generation != self.generation => Vec::new(),
            Some(a) => a.answer.actions.clone(),
            None => p
                .widgets
                .get(&key.widget)
                .and_then(|state| state.frame.as_ref())
                .map(|frame| frame.actions.clone())
                .unwrap_or_default(),
        }
    }

    /// Starts search `generation`: every condition tile turns stale, and
    /// each provider is expected to answer with tiles when there is an env.
    /// Returns whether the strip changed.
    pub(crate) fn searched(&mut self, generation: u64) -> bool {
        self.generation = generation;
        self.closed = false;
        let with_env = self.env.is_some();
        let mut stale = false;
        for p in self.providers.values_mut() {
            if with_env {
                p.asked = generation;
            }
            stale |= !p.answers.is_empty();
        }
        stale
    }

    /// Whether the launcher hid since the latest search.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    /// `provider` alone is asked the latest search again: it answers with
    /// tiles when there is an env.
    pub(crate) fn asked_again(&mut self, provider: &str) {
        if self.env.is_some()
            && let Some(p) = self.providers.get_mut(provider)
        {
            p.asked = self.generation;
        }
    }

    /// Whether a provider `dynamic` says answers searches was asked the
    /// latest search with an env and has not answered it with tiles yet.
    pub(crate) fn answers_pending(&self, dynamic: impl Fn(&str) -> bool) -> bool {
        self.providers.iter().any(|(id, p)| {
            p.asked == self.generation && p.answered != self.generation && dynamic(id)
        })
    }

    /// Replaces `provider`'s always tiles with the declared ones of `ids`.
    pub(crate) fn show(&mut self, provider: &str, ids: Vec<String>) -> bool {
        let Some(p) = self.providers.get_mut(provider) else {
            return false;
        };
        let mut always: Vec<String> = Vec::with_capacity(ids.len());
        for id in ids {
            if !p.widgets.contains_key(&id) {
                tracing::debug!(module = %provider, widget = %id, "not showing an undeclared widget");
            } else if !always.contains(&id) {
                always.push(id);
            }
        }
        if p.always == always {
            return false;
        }
        p.always = always;
        true
    }

    /// Asks for a new render of a declared widget; returns whether it is one.
    pub(crate) fn mark_dirty(&mut self, provider: &str, widget: &str) -> bool {
        let state = self
            .providers
            .get_mut(provider)
            .and_then(|p| p.widgets.get_mut(widget));
        match state {
            Some(state) => {
                state.dirty = true;
                true
            }
            None => {
                tracing::debug!(module = %provider, %widget, "not redrawing an undeclared widget");
                false
            }
        }
    }

    /// Stores `image`, replacing the one with its id, under a new version. The same bytes
    /// under the same id keep the version and change nothing: `false`.
    pub(crate) fn set_image(&mut self, provider: &str, image: WidgetImage) -> bool {
        let Some(p) = self.providers.get_mut(provider) else {
            return false;
        };
        if p.images
            .get(&image.id)
            .is_some_and(|(_, old)| *old == image)
        {
            return false;
        }
        self.image_version += 1;
        p.images
            .insert(image.id.clone(), (self.image_version, image));
        true
    }

    /// The ids of `provider`'s stopped widgets.
    pub(crate) fn stopped(&self, provider: &str) -> Vec<String> {
        let Some(p) = self.providers.get(provider) else {
            return Vec::new();
        };
        p.decls
            .iter()
            .filter(|d| p.widgets.get(&d.id).is_some_and(WidgetState::is_stopped))
            .map(|d| d.id.clone())
            .collect()
    }

    /// Replaces `provider`'s condition tiles with the answers of search
    /// `generation` for `query` whose frame is on time; a widget no longer
    /// answered is asked to draw again. The provider has
    /// answered that search, so the strip changes even when no tile does:
    /// the UI may be waiting for the answer. Once the launcher hid, until
    /// the next search, no answer is kept (their images are).
    pub(crate) fn answered(
        &mut self,
        provider: &str,
        generation: u64,
        query: &str,
        mut answers: Vec<RenderedAnswer>,
    ) -> Applied {
        let mut applied = Applied::default();
        self.answer_images(provider, &mut answers);
        if self.closed {
            answers.clear();
        }
        let deadline = self.soft_deadline();
        let Some(p) = self.providers.get_mut(provider) else {
            return applied;
        };
        let mut kept: Vec<Answered> = Vec::with_capacity(answers.len());
        for RenderedAnswer {
            answer,
            took,
            frame,
        } in answers
        {
            let usable = p
                .widgets
                .get(&answer.widget)
                .is_some_and(|state| !state.is_stopped());
            if !usable || kept.iter().any(|a| a.answer.widget == answer.widget) {
                tracing::debug!(module = %provider, widget = %answer.widget, "dropping a widget answer");
                continue;
            }
            match p.judge(&answer.widget, took, deadline, frame) {
                Judged::OnTime(frame) => {
                    kept.push(Answered {
                        generation,
                        query: query.to_string(),
                        answer,
                        frame: Arc::new(frame),
                    });
                }
                Judged::Dropped => {}
                Judged::Stopped(reason) => {
                    applied
                        .stopped
                        .push(p.stopped(&answer.widget, reason, Stop::Frames))
                }
            }
        }
        let old = std::mem::replace(&mut p.answers, kept);
        p.redraw_unanswered(old);
        p.answered = generation;
        applied.changed = true;
        applied
    }

    /// Stores the images of every answer's frame, kept or not (images are
    /// sent once), and takes them out of the frames; returns whether any was.
    pub(crate) fn answer_images(&mut self, provider: &str, answers: &mut [RenderedAnswer]) -> bool {
        let mut stored = false;
        for answer in answers {
            if let Ok(frame) = &mut answer.frame {
                stored |= self.store_images(provider, std::mem::take(&mut frame.images));
            }
        }
        stored
    }

    fn store_images(&mut self, provider: &str, images: Vec<WidgetImage>) -> bool {
        let mut stored = false;
        for image in images {
            stored |= self.set_image(provider, image);
        }
        stored
    }

    /// The state of `key`'s widget.
    fn state(&self, key: &WidgetKey) -> Option<&WidgetState> {
        self.providers.get(&key.provider)?.widgets.get(&key.widget)
    }
}

impl Provider {
    fn answer(&self, widget: &str) -> Option<&Answered> {
        self.answers.iter().find(|a| a.answer.widget == widget)
    }

    /// Drops every condition tile.
    fn drop_answers(&mut self) {
        let old = std::mem::take(&mut self.answers);
        self.redraw_unanswered(old);
    }

    /// Asks every widget of `old` answers that is no longer answered to draw
    /// again: it may stay as an always tile, whose frame was not drawn while
    /// it was answered.
    fn redraw_unanswered(&mut self, old: Vec<Answered>) {
        for old in old {
            if self.answer(&old.answer.widget).is_none()
                && let Some(state) = self.widgets.get_mut(&old.answer.widget)
            {
                state.dirty = true;
            }
        }
    }

    /// Whether `widget` is one of the always tiles the strip lists: one the
    /// paint guard stopped is not.
    fn lists_always(&self, widget: &str) -> bool {
        self.always.iter().any(|id| id == widget)
            && self
                .widgets
                .get(widget)
                .is_none_or(|state| state.stopped != Some(Stop::Paint))
    }

    fn decl(&self, widget: &str) -> Option<&WidgetDecl> {
        self.decls.iter().find(|d| d.id == widget)
    }
}
