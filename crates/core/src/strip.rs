//! The widget store: what every provider declared, the user's Home list and
//! its tiles, the search's condition tiles, the last frame of every tile and
//! each provider's images.
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
use crate::module::{HomeTile, RenderedAnswer};
use crate::settings::HomeEntry;
use crate::widget::{
    Frame, Params, Slot, TileKey, WidgetAnswer, WidgetContext, WidgetDecl, WidgetImage, WidgetKey,
    WidgetSize, WidgetTheme,
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

/// What the UI tells the core about the tiles it draws: the size of a
/// quarter, a single and a double tile, the display scale and the theme.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetEnv {
    pub quarter: (f32, f32),
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
        params: Params,
    ) -> WidgetContext {
        let (width, height) = match size {
            WidgetSize::Quarter => self.quarter,
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
            params,
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
    providers: HashMap<String, Provider>,
    /// The user's Home list as set, with the entries that cannot show now.
    home: Vec<HomeEntry>,
    /// The tiles of the Home entries that can show now (their provider is registered and
    /// declares their widget), in Home order; those the paint guard stopped included.
    home_tiles: Vec<HomeSlot>,
    /// Version of the last image stored, of any provider: a version is never
    /// reused, not even by a reloaded provider.
    image_version: u64,
    /// The latest search generation; a condition tile answered for an older
    /// one is stale.
    generation: u64,
    /// The latest search's query is not empty: the strip shows the search's
    /// tiles, not Home.
    typed: bool,
    /// The launcher hid since the latest search: no widget answer is kept
    /// until the next one.
    closed: bool,
    /// The tiles on screen, with when each came on it.
    shown: HashMap<TileKey, Instant>,
    /// By provider: the widgets that had tiles on the strip when an instance
    /// of it was dropped. A user action on one of them, while the provider's
    /// current instance does not show that tile, would reach an instance that
    /// never drew it. It never shrinks, on purpose: a tile the new instance
    /// shows is on the strip, which lets its events through, so no entry
    /// needs removing. It is bounded by the 16 widgets a plugin may declare,
    /// per provider id.
    gone: HashMap<String, HashSet<String>>,
    /// A test's soft render deadline in place of [`soft_render_deadline`].
    #[cfg(any(test, feature = "testing"))]
    soft_deadline: Option<Duration>,
}

/// A Home tile that can show: its key, the size it shows in and the index in the user's list of
/// the entry it shows.
#[derive(Clone, Debug, PartialEq)]
struct HomeSlot {
    key: TileKey,
    size: WidgetSize,
    entry: usize,
}

struct Provider {
    decls: Vec<WidgetDecl>,
    /// By declared id: the render state of the widget's search tile. A
    /// widget has one search tile at a time, whatever its parameters, so its
    /// run of late frames counts across searches.
    search: HashMap<String, WidgetState>,
    /// The render state and last frame of each of the provider's Home tiles.
    home: HashMap<TileKey, WidgetState>,
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
    /// The last frame drawn as a Home tile.
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
    /// Stopped: it renders no more, for this module instance, or, a search
    /// tile stopped for its frames, until the launcher hides.
    stopped: Option<Stop>,
}

/// Why a tile stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    /// Late or failed frames in a row; it keeps its last frame.
    Frames,
    /// The UI's paint guard: its frame is gone, and its Home tile leaves the strip.
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
    Home,
    /// The condition tile answered for this search generation.
    Answer(u64),
}

impl WidgetStore {
    pub(crate) fn env(&self) -> Option<&WidgetEnv> {
        self.env.as_ref()
    }

    /// Starts `provider`'s widgets afresh with `decls`; a duplicate id keeps
    /// the first. Its Home entries show from now on.
    pub(crate) fn declare(&mut self, provider: &str, decls: Vec<WidgetDecl>) {
        let mut kept: Vec<WidgetDecl> = Vec::with_capacity(decls.len());
        for decl in decls {
            if kept.iter().any(|d| d.id == decl.id) {
                tracing::warn!(module = %provider, widget = %decl.id, "dropping a widget declared twice");
            } else {
                kept.push(decl);
            }
        }
        let search = kept
            .iter()
            .map(|d| (d.id.clone(), WidgetState::default()))
            .collect();
        self.providers.insert(
            provider.to_string(),
            Provider {
                decls: kept,
                search,
                home: HashMap::new(),
                answers: Vec::new(),
                images: HashMap::new(),
                asked: 0,
                answered: 0,
            },
        );
        self.refresh_home();
    }

    /// Forgets everything of `provider`, and that its tiles were on screen;
    /// returns whether it had any tile to show. Its tiles are remembered as
    /// gone (see [`is_gone`](Self::is_gone)); its Home entries stay in the
    /// list, and show again once it is declared again.
    pub(crate) fn forget(&mut self, provider: &str) -> bool {
        self.shown.retain(|key, _| key.widget.provider != provider);
        let Some(p) = self.providers.remove(provider) else {
            return false;
        };
        let mut tiles: Vec<String> = p.answers.into_iter().map(|a| a.answer.widget).collect();
        tiles.extend(
            self.home_tiles
                .iter()
                .filter(|slot| slot.key.widget.provider == provider)
                .map(|slot| slot.key.widget.widget.clone()),
        );
        self.refresh_home();
        if tiles.is_empty() {
            return false;
        }
        self.gone
            .entry(provider.to_string())
            .or_default()
            .extend(tiles);
        true
    }

    /// The ids of the providers that declared widgets.
    pub(crate) fn providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    /// Whether `provider` declared widgets.
    pub(crate) fn has_provider(&self, provider: &str) -> bool {
        self.providers.contains_key(provider)
    }

    pub(crate) fn is_declared(&self, key: &WidgetKey) -> bool {
        self.providers
            .get(&key.provider)
            .is_some_and(|p| p.decl(&key.widget).is_some())
    }

    /// Whether `key`'s widget had a tile on the strip when its provider's
    /// instance was dropped, and the current instance does not show `key`.
    pub(crate) fn is_gone(&self, key: &TileKey) -> bool {
        self.gone
            .get(&key.widget.provider)
            .is_some_and(|gone| gone.contains(&key.widget.widget))
            && !self.on_strip(key)
    }

    /// The user's Home list, with the entries that cannot show now.
    pub(crate) fn home(&self) -> &[HomeEntry] {
        &self.home
    }

    /// Replaces the Home list; returns whether it differs from the last.
    pub(crate) fn set_home(&mut self, home: Vec<HomeEntry>) -> bool {
        if self.home == home {
            return false;
        }
        self.home = home;
        self.refresh_home();
        true
    }

    /// `provider`'s own Home tiles: each widget and parameters once, in Home
    /// order. Empty for a provider that declared no widgets.
    pub(crate) fn home_of(&self, provider: &str) -> Vec<HomeTile> {
        let mut tiles: Vec<HomeTile> = Vec::new();
        for slot in &self.home_tiles {
            let key = &slot.key;
            if key.widget.provider != provider
                || tiles
                    .iter()
                    .any(|t| t.widget == key.widget.widget && t.params == key.params)
            {
                continue;
            }
            tiles.push(HomeTile {
                widget: key.widget.widget.clone(),
                params: key.params.clone(),
            });
        }
        tiles
    }

    /// Resolves the Home list against what the providers declare: an entry
    /// shows when its provider declares its widget, in its size if the
    /// widget offers it (else the default), with the parameters the widget
    /// declares. Tiles that stay keep their state and frame; one whose size
    /// changed draws again.
    fn refresh_home(&mut self) {
        let mut tiles: Vec<HomeSlot> = Vec::new();
        let mut counts: HashMap<(&WidgetKey, Params), u32> = HashMap::new();
        for (index, entry) in self.home.iter().enumerate() {
            let Some(decl) = self
                .providers
                .get(&entry.widget.provider)
                .and_then(|p| p.decl(&entry.widget.widget))
            else {
                continue;
            };
            let params = decl.declared_params(&entry.params);
            let count = counts.entry((&entry.widget, params.clone())).or_insert(0);
            let slot = Slot::Home(*count);
            *count += 1;
            tiles.push(HomeSlot {
                key: TileKey {
                    widget: entry.widget.clone(),
                    params,
                    slot,
                },
                size: decl.size_or_default(entry.size),
                entry: index,
            });
        }
        let old = std::mem::replace(&mut self.home_tiles, tiles);
        for (id, p) in &mut self.providers {
            let mine: Vec<&HomeSlot> = self
                .home_tiles
                .iter()
                .filter(|slot| slot.key.widget.provider == *id)
                .collect();
            p.home
                .retain(|key, _| mine.iter().any(|slot| slot.key == *key));
            for slot in mine {
                let state = p.home.entry(slot.key.clone()).or_default();
                let resized = old.iter().any(|o| o.key == slot.key && o.size != slot.size);
                if resized {
                    state.dirty = true;
                }
            }
        }
    }

    /// The condition tile's frame for a search tile, the last frame for a Home tile.
    pub(crate) fn frame(&self, key: &TileKey) -> Option<Arc<Frame>> {
        let p = self.providers.get(&key.widget.provider)?;
        match key.slot {
            Slot::Search => p.answer_for(key).map(|a| a.frame.clone()),
            Slot::Home(_) => p.home.get(key)?.frame.clone(),
        }
    }

    pub(crate) fn image(&self, provider: &str, id: &str) -> Option<(u64, WidgetImage)> {
        self.providers.get(provider)?.images.get(id).cloned()
    }

    /// The query `key`'s condition tile was answered for, if it is one.
    pub(crate) fn answered_query(&self, key: &TileKey) -> Option<String> {
        let p = self.providers.get(&key.widget.provider)?;
        p.answer_for(key).map(|a| a.query.clone())
    }

    /// Whether `key` is a condition tile answered for an older search.
    pub(crate) fn is_stale(&self, key: &TileKey) -> bool {
        self.providers
            .get(&key.widget.provider)
            .and_then(|p| p.answer_for(key))
            .is_some_and(|a| a.generation != self.generation)
    }

    /// The actions of `key`: its condition tile's (none while stale), or those
    /// of a Home tile's last frame.
    pub(crate) fn actions(&self, key: &TileKey) -> Vec<Action> {
        let Some(p) = self.providers.get(&key.widget.provider) else {
            return Vec::new();
        };
        match key.slot {
            Slot::Search => match p.answer_for(key) {
                Some(a) if a.generation == self.generation => a.answer.actions.clone(),
                _ => Vec::new(),
            },
            Slot::Home(_) => p
                .home
                .get(key)
                .and_then(|state| state.frame.as_ref())
                .map(|frame| frame.actions.clone())
                .unwrap_or_default(),
        }
    }

    /// What adding search tile `key` to Home appends: its widget, the size it
    /// shows in and its parameters. `None` unless `key` is a condition tile
    /// of the strip answered for the latest search.
    pub(crate) fn home_entry(&self, key: &TileKey) -> Option<HomeEntry> {
        if key.slot != Slot::Search || !self.on_strip(key) || self.is_stale(key) {
            return None;
        }
        let p = self.providers.get(&key.widget.provider)?;
        let answered = p.answer_for(key)?;
        let decl = p.decl(&key.widget.widget)?;
        Some(HomeEntry {
            widget: key.widget.clone(),
            size: Some(decl.size_or_default(answered.answer.size)),
            params: decl.declared_params(&key.params),
        })
    }

    /// Gives the last Home tile, added from search tile `search`, the
    /// search tile's frame while it has none, so it draws at once. Only a
    /// tile of the same widget, parameters and size takes it.
    pub(crate) fn seed_last_home_tile(&mut self, search: &TileKey) {
        let Some(slot) = self.home_tiles.last() else {
            return;
        };
        let (home, size) = (slot.key.clone(), slot.size);
        if home.widget != search.widget || home.params != search.params {
            return;
        }
        let Some(p) = self.providers.get_mut(&search.widget.provider) else {
            return;
        };
        let Some(answered) = p.answer_for(search) else {
            return;
        };
        let shown = p
            .decl(&search.widget.widget)
            .map(|d| d.size_or_default(answered.answer.size));
        if shown != Some(size) {
            return;
        }
        let frame = answered.frame.clone();
        if let Some(state) = p.home.get_mut(&home)
            && state.frame.is_none()
        {
            state.frame = Some(frame);
        }
    }

    /// Starts search `generation`, whose query is `typed` (not empty) or not.
    /// At a typed query every condition tile turns stale, and each provider
    /// is expected to answer with tiles when there is an env. At the empty
    /// query the strip is Home: the condition tiles go, and no provider is
    /// asked for tiles. Returns whether the strip changed.
    pub(crate) fn searched(&mut self, generation: u64, typed: bool) -> bool {
        // Home leaves or comes back.
        let mut changed = self.typed != typed && !self.home_tiles.is_empty();
        self.generation = generation;
        self.typed = typed;
        self.closed = false;
        let asked = self.env.is_some() && typed;
        for p in self.providers.values_mut() {
            if asked {
                p.asked = generation;
            }
            changed |= !p.answers.is_empty();
            if !typed {
                p.answers.clear();
            }
        }
        changed
    }

    /// Whether the launcher hid since the latest search.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether the latest search's query is not empty: the strip shows its tiles.
    pub(crate) fn is_typed(&self) -> bool {
        self.typed
    }

    /// `provider` alone is asked the latest search again: it answers with
    /// tiles when there is an env and the query is typed.
    pub(crate) fn asked_again(&mut self, provider: &str) {
        if self.env.is_some()
            && self.typed
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

    /// Asks every tile of a declared widget, whatever its parameters, for a
    /// new render; returns whether it is one.
    pub(crate) fn mark_dirty(&mut self, provider: &str, widget: &str) -> bool {
        let Some(p) = self.providers.get_mut(provider) else {
            tracing::debug!(module = %provider, %widget, "not redrawing an undeclared widget");
            return false;
        };
        let Some(state) = p.search.get_mut(widget) else {
            tracing::debug!(module = %provider, %widget, "not redrawing an undeclared widget");
            return false;
        };
        state.dirty = true;
        for (key, state) in &mut p.home {
            if key.widget.widget == widget {
                state.dirty = true;
            }
        }
        true
    }

    /// Asks the tile `key` alone for a new render.
    fn mark_tile_dirty(&mut self, key: &TileKey) {
        if let Some(state) = self.state_mut(key) {
            state.dirty = true;
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

    /// The ids of `provider`'s widgets whose search tile is stopped: a
    /// search answers no tile of them. A stopped Home tile does not count.
    pub(crate) fn stopped(&self, provider: &str) -> Vec<String> {
        let Some(p) = self.providers.get(provider) else {
            return Vec::new();
        };
        p.decls
            .iter()
            .filter(|d| p.search_stopped(&d.id))
            .map(|d| d.id.clone())
            .collect()
    }

    /// Replaces `provider`'s condition tiles with the answers of search
    /// `generation` for `query` whose frame is on time. The provider has
    /// answered that search, so the strip changes even when no tile does:
    /// the UI may be waiting for the answer. Once the launcher hid, until
    /// the next search, and at the empty query, no answer is kept (their
    /// images are).
    pub(crate) fn answered(
        &mut self,
        provider: &str,
        generation: u64,
        query: &str,
        mut answers: Vec<RenderedAnswer>,
    ) -> Applied {
        let mut applied = Applied::default();
        self.answer_images(provider, &mut answers);
        if self.closed || !self.typed {
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
            let usable = p.decl(&answer.widget).is_some() && !p.search_stopped(&answer.widget);
            if !usable || kept.iter().any(|a| a.answer.widget == answer.widget) {
                tracing::debug!(module = %provider, widget = %answer.widget, "dropping a widget answer");
                continue;
            }
            let Some(state) = p.search.get_mut(&answer.widget) else {
                continue;
            };
            match render::judge(state, &answer.widget, took, deadline, frame) {
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
                    let key = TileKey {
                        widget: WidgetKey::new(provider, answer.widget.as_str()),
                        params: answer.params.clone(),
                        slot: Slot::Search,
                    };
                    applied
                        .stopped
                        .push(p.stop_tile(&key, reason, Stop::Frames));
                }
            }
        }
        p.answers = kept;
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

    /// The render state of tile `key`, while it is one: a Home tile that can
    /// show, or the condition tile answered with `key`'s parameters.
    fn state(&self, key: &TileKey) -> Option<&WidgetState> {
        let p = self.providers.get(&key.widget.provider)?;
        match key.slot {
            Slot::Search => {
                p.answer_for(key)?;
                p.search.get(&key.widget.widget)
            }
            Slot::Home(_) => p.home.get(key),
        }
    }

    /// The render state `key`'s renders go to: for a search tile, that of its
    /// widget's search tile, whatever the parameters answered now, so a reply
    /// for an older answer still counts as received.
    fn state_mut(&mut self, key: &TileKey) -> Option<&mut WidgetState> {
        let p = self.providers.get_mut(&key.widget.provider)?;
        match key.slot {
            Slot::Search => p.search.get_mut(&key.widget.widget),
            Slot::Home(_) => p.home.get_mut(key),
        }
    }

    /// The index in the user's Home list of the entry Home tile `key` shows.
    pub(crate) fn home_index(&self, key: &TileKey) -> Option<usize> {
        self.home_slot(key).map(|slot| slot.entry)
    }

    /// The sizes `widget` offers, its default first; empty when no provider declares it.
    pub(crate) fn widget_sizes(&self, widget: &WidgetKey) -> Vec<WidgetSize> {
        self.providers
            .get(&widget.provider)
            .and_then(|p| p.decl(&widget.widget))
            .map(|decl| decl.sizes.clone())
            .unwrap_or_default()
    }

    /// The Home tile `key`, while it can show.
    fn home_slot(&self, key: &TileKey) -> Option<&HomeSlot> {
        self.home_tiles.iter().find(|slot| slot.key == *key)
    }
}

impl Provider {
    /// The condition tile of `key`'s widget, if it is answered with `key`'s parameters.
    fn answer_for(&self, key: &TileKey) -> Option<&Answered> {
        if key.slot != Slot::Search {
            return None;
        }
        self.answers
            .iter()
            .find(|a| a.answer.widget == key.widget.widget && a.answer.params == key.params)
    }

    /// Whether the search tile of `widget` is stopped.
    fn search_stopped(&self, widget: &str) -> bool {
        self.search.get(widget).is_some_and(WidgetState::is_stopped)
    }

    /// Whether Home tile `key` shows: one the paint guard stopped does not.
    fn lists_home(&self, key: &TileKey) -> bool {
        self.home
            .get(key)
            .is_some_and(|state| state.stopped != Some(Stop::Paint))
    }

    fn decl(&self, widget: &str) -> Option<&WidgetDecl> {
        self.decls.iter().find(|d| d.id == widget)
    }
}
