//! The widget strip in the launcher view: its keys, its clicks, and the
//! Enter and Ctrl+K that wait for a search's tiles. [`keys`] decides where
//! the focus goes; this applies it.

use gpui_kit::component::input::{MoveLeft, MoveRight};
use gpui_kit::{Context, KeyDownEvent, ScrollStrategy, Task, Window};
use tomakast_core::{HomeWriteError, KeyEvent, Modifiers, TileKey, Toast, ToastKind, WidgetEvent};

use super::{LauncherView, StripNext, StripPrev};
use crate::stack::{Issuer, RequestKind};
use crate::strip;
use crate::strip::keys::{self, ENTER_WAIT, Focus, Key, KeyCtx, Outcome};

/// Enter or Ctrl+K pressed at the root while a dynamic source still owed the
/// search its tiles: it acts once they are in, or at [`ENTER_WAIT`].
pub(super) struct PendingKey {
    /// [`Key::Enter`] or [`Key::Actions`].
    key: Key,
    /// The search it waits on.
    generation: u64,
    _timer: Task<()>,
}

/// The widget strip's keys, clicks and waits; [`keys`] decides where the
/// focus goes.
impl LauncherView {
    /// Whether an unedited recalled query shows with its condition tile
    /// selected: Up and Down stay the history's there.
    pub(super) fn recalled_tile(&self) -> bool {
        self.recall.is_some() && matches!(self.strip.focus, Focus::TileSelected(_))
    }

    /// `key` on the root list, with no panel open, as the strip takes it
    /// (see [`keys::step`]): the focus moves, and a tile's action runs, its
    /// panel opens, or the key goes to the focused canvas as `name`.
    /// Elsewhere, and for a key the strip leaves alone, `Unhandled`.
    pub(super) fn strip_key(
        &mut self,
        key: Key,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Outcome {
        if self.stack.top().is_some() || self.panel.is_some() {
            return Outcome::Unhandled;
        }
        // A key reaches a canvas only while the canvas has the keyboard.
        self.follow_keyboard_in(window, cx);
        let kinds = keys::part_kinds(&self.strip.tiles);
        let ctx = KeyCtx {
            on_first_row: self.on_first_row(),
            recalled: self.recall.is_some(),
            tiles: &kinds,
        };
        let (focus, outcome) = keys::step(self.strip.focus.index(&self.strip.tiles), key, &ctx);
        if outcome != Outcome::Unhandled {
            self.strip.moved = true;
        }
        self.set_strip_focus(Focus::named(focus, &self.strip.tiles), window, cx);
        let tile = |this: &Self, ix: usize| this.strip.tiles.get(ix).cloned();
        match outcome {
            Outcome::Unhandled | Outcome::Handled | Outcome::ToList => {}
            Outcome::RunPrimary(ix) => {
                if let Some(tile) = tile(self, ix) {
                    self.run_tile_primary(&tile.key, cx);
                }
            }
            Outcome::RunPart { tile: ix, part } => {
                if let Some(tile) = tile(self, ix)
                    && let Some((node, Some(action))) = keys::part_node(&tile, part)
                {
                    self.run_tile_action(&tile.key, Some(node), action, cx);
                }
            }
            Outcome::OpenActions(_) => self.open_panel(window, cx),
            Outcome::ToCanvas { tile: ix, part } => {
                if let Some(tile) = tile(self, ix)
                    && let Some((node, _)) = keys::part_node(&tile, part)
                {
                    let mods = match key {
                        Key::Actions => command_modifier(),
                        _ => Modifiers::empty(),
                    };
                    self.send_canvas_key(&tile.key, node, name, mods, cx);
                }
            }
        }
        outcome
    }

    /// Tab: into the strip and through its tiles and parts.
    pub(super) fn strip_next(
        &mut self,
        _: &StripNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.strip_key(Key::Tab, "tab", window, cx) == Outcome::Unhandled {
            cx.propagate();
        }
    }

    pub(super) fn strip_prev(
        &mut self,
        _: &StripPrev,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.strip_key(Key::ShiftTab, "shift-tab", window, cx) == Outcome::Unhandled {
            cx.propagate();
        }
    }

    /// Left in the search field, before the field takes it: while the strip
    /// has the selection, it moves between the tiles, and the caret stays.
    /// The field moves its caret first and hands the key on only from the
    /// start of its text (Right: from the end), so without this the strip
    /// got the key only at one end of the caret's way.
    pub(super) fn field_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.strip_key(Key::Left, "left", window, cx) != Outcome::Unhandled {
            cx.stop_propagation();
        }
    }

    /// Right in the search field, before the field takes it; see
    /// [`Self::field_left`].
    pub(super) fn field_right(
        &mut self,
        _: &MoveRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.strip_key(Key::Right, "right", window, cx) != Outcome::Unhandled {
            cx.stop_propagation();
        }
    }

    /// Moves the root focus to `focus`: the strip scrolls to a newly focused
    /// tile, and a focused canvas takes the keyboard from the field.
    /// A key or a press that waits for the tiles goes once the focus is in
    /// the strip: it acts on what the field shows, which the user left.
    /// In the field, the list's selection is its first row, scrolled to the
    /// top: what Enter and Ctrl+K act on there, and where Down comes back to.
    pub(super) fn set_strip_focus(
        &mut self,
        focus: Focus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if focus == self.strip.focus {
            return;
        }
        if focus == Focus::Field {
            self.selected = self.first_row();
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
        if matches!(focus, Focus::Strip { .. }) {
            self.pending = None;
        }
        let tiles = &self.strip.tiles;
        let was_canvas = self.strip.focus.on_canvas(tiles);
        if focus.tile() != self.strip.focus.tile()
            && let Some(ix) = focus
                .tile()
                .and_then(|key| tiles.iter().position(|tile| &tile.key == key))
        {
            strip::scroll_to(&self.strip, tiles, ix);
        }
        let canvas = focus.on_canvas(tiles);
        self.strip.focus = focus;
        if canvas && !was_canvas {
            window.focus(&self.strip_focus, cx);
        } else if !canvas && was_canvas {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// After the strip changed: a focus on a tile or part that left falls
    /// back, a selected condition tile keeps the selection while it waits
    /// for its new answer (answered for an older query, it takes no action),
    /// and a fresh one that comes first takes it unless the user moved it
    /// since the query changed (never at an empty query): from the list, or
    /// from the field while it shows a recalled query, which keeps Up and
    /// Down the history's (see [`Self::recalled_tile`]).
    pub(super) fn sync_strip_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_keyboard_in(window, cx);
        let tiles = &self.strip.tiles;
        let mut focus = Focus::named(self.strip.focus.index(tiles), tiles);
        let condition = |key: &TileKey| {
            tiles
                .iter()
                .find(|tile| &tile.key == key)
                .and_then(|tile| tile.condition.as_ref())
        };
        let fresh = |key: &TileKey| condition(key).is_some_and(|condition| !condition.stale);
        if let Focus::TileSelected(key) = &focus
            && condition(key).is_none()
        {
            focus = Focus::List;
        }
        let recalled_field = focus == Focus::Field && self.recall.is_some();
        if (focus == Focus::List || recalled_field)
            && !self.strip.moved
            && !self.query_empty
            && self.stack.top().is_none()
            && self.panel.is_none()
            && let Some(first) = tiles.first().filter(|tile| fresh(&tile.key))
        {
            focus = Focus::TileSelected(first.key.clone());
        }
        self.set_strip_focus(focus, window, cx);
        // The focus left a canvas without a key (a new query), or the strip
        // took it some other way: the keyboard goes back to the field.
        if !self.strip.focus.on_canvas(&self.strip.tiles) && self.strip_focus.is_focused(window) {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// The focus names a canvas, but the keyboard went elsewhere (a click on
    /// the field): the focus goes back to the list, where the keyboard is.
    fn follow_keyboard_in(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.strip.focus.on_canvas(&self.strip.tiles) && !self.strip_focus.is_focused(window) {
            self.strip.focus = Focus::List;
            cx.notify();
        }
    }

    /// Whether a dynamic source still owes the latest search its tiles.
    fn answers_pending(&self) -> bool {
        self.core
            .try_borrow()
            .is_ok_and(|core| core.widget_answers_pending())
    }

    /// Enter or Ctrl+K (`key`) on the root list. With the keyboard in the
    /// field and a search's tiles in flight, it waits for them, at most
    /// [`ENTER_WAIT`]: a press must not act on the list's row when the tile
    /// it is typed for is about to come. A press while one waits is ignored.
    pub(super) fn root_press(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let in_field = matches!(
            self.strip.focus,
            Focus::List | Focus::Field | Focus::TileSelected(_)
        );
        if in_field && self.answers_pending() {
            let timer = cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(ENTER_WAIT).await;
                let _ = this.update_in(cx, |this, window, cx| this.resolve_pending(window, cx));
            });
            self.pending = Some(PendingKey {
                key,
                generation: self.generation,
                _timer: timer,
            });
            return;
        }
        self.act_at_root(key, window, cx);
    }

    /// Enter or Ctrl+K on what the root shows: the tile with the focus,
    /// else the list's selected row. A selected tile answered for an older
    /// query (its new answer did not come in time) gives the selection to
    /// the list, whose row the press acts on.
    fn act_at_root(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) {
        if let Focus::TileSelected(tile) = &self.strip.focus
            && self.stale_tile(tile)
        {
            self.set_strip_focus(Focus::List, window, cx);
        }
        let name = if key == Key::Enter { "enter" } else { "k" };
        if self.strip_key(key, name, window, cx) == Outcome::Unhandled {
            match key {
                Key::Enter => self.confirm_root(cx),
                _ => self.open_panel(window, cx),
            }
        }
    }

    /// The strip changed (`CoreUpdate::Widgets`): the window draws again,
    /// and the focus follows the new tiles there (GPUI draws a window it was
    /// asked to before it takes a key); a waiting Enter or Ctrl+K acts once
    /// no source owes the search its tiles.
    pub(crate) fn strip_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.generation == self.generation)
            && !self.answers_pending()
        {
            self.resolve_pending(window, cx);
        }
    }

    /// Reads the strip's tiles from the core, and the focus follows them.
    pub(super) fn read_strip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.top().is_none()
            && let Ok(core) = self.core.try_borrow()
        {
            self.strip.tiles = core.strip();
        }
        self.sync_strip_focus(window, cx);
    }

    /// The waiting Enter or Ctrl+K acts on what is on screen now: a fresh
    /// condition tile that came first, else the list's row. Not once a view
    /// was pushed or the panel opened since: the root no longer shows.
    fn resolve_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if pending.generation != self.generation
            || self.stack.top().is_some()
            || self.panel.is_some()
        {
            return;
        }
        self.read_strip(window, cx);
        self.act_at_root(pending.key, window, cx);
    }

    /// The first action of `key`'s tile, if it has one.
    fn run_tile_primary(&mut self, key: &TileKey, cx: &mut Context<Self>) {
        let action = self
            .strip
            .tiles
            .iter()
            .find(|tile| &tile.key == key)
            .and_then(|tile| strip::tile_actions(tile).first())
            .map(|action| action.id.clone());
        if let Some(action) = action {
            self.run_tile_action(key, None, action, cx);
        }
    }

    /// Sends `action` of `key`'s tile, from its button `node` or the tile
    /// itself, under the Enter gate; returns `false` while that holds it back.
    pub(super) fn run_tile_action(
        &mut self,
        key: &TileKey,
        node: Option<String>,
        action: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.gated(&key.widget.provider, None, cx) {
            return false;
        }
        let event = WidgetEvent::Action { node, action };
        if let Err(e) = self.send_tile_event(key, event, RequestKind::Action, cx) {
            tracing::warn!(widget = %key, error = %e, "widget action failed");
            self.show_error_toast(e, cx);
        }
        true
    }

    /// Sends key `key` with `modifiers` to canvas `node` of `key`'s tile.
    fn send_canvas_key(
        &mut self,
        tile: &TileKey,
        node: String,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let event = WidgetEvent::Key(KeyEvent {
            node,
            key: key.to_string(),
            modifiers,
        });
        // Not gated, as typing is not.
        if let Err(e) = self.send_tile_event(tile, event, RequestKind::Query, cx) {
            tracing::debug!(widget = %tile, error = %e, "key not sent to the canvas");
        }
    }

    /// Sends `event` to `key`'s module and records its request, if it gets
    /// one, in the ledger as `kind`, so that its reply reaches this window.
    fn send_tile_event(
        &mut self,
        key: &TileKey,
        event: WidgetEvent,
        kind: RequestKind,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        let request = self
            .core
            .try_borrow_mut()
            .map_err(|_| "the core is busy".to_string())?
            .send_widget_event(key, event)
            .map_err(|e| e.to_string())?;
        if let Some(request) = request {
            let issuer = Issuer {
                window: self.window_number,
                provider: &key.widget.provider,
                view: None,
            };
            let now = cx.background_executor().now();
            self.ledger.borrow_mut().issue(request, issuer, kind, now);
        }
        Ok(())
    }

    /// The launcher's "Add to Home" on search tile `key`: appends it to Home
    /// and says so. A tile no longer the search's (answered for an older
    /// query in the meantime) adds nothing, and says nothing.
    pub(super) fn add_to_home(&mut self, key: &TileKey, cx: &mut Context<Self>) {
        let Some(added) = self
            .core
            .try_borrow_mut()
            .ok()
            .map(|mut core| core.add_to_home(key))
        else {
            self.show_error_toast("the core is busy".to_string(), cx);
            return;
        };
        match added {
            Ok(()) => {
                let toast = Toast {
                    kind: ToastKind::Success,
                    title: "Added to Home".to_string(),
                    message: None,
                };
                self.show_toast(toast, cx);
            }
            Err(HomeWriteError::NotASearchTile) => {}
            Err(e) => self.show_home_write_error(e, cx),
        }
    }

    /// The toast of a Home write that failed: a config file that does not
    /// parse says how to fix it; no config file, a write that failed, or a
    /// file changed since it was read ("config.toml changed: try again") each
    /// say so in their own words.
    pub(super) fn show_home_write_error(&mut self, err: HomeWriteError, cx: &mut Context<Self>) {
        let message = match err {
            HomeWriteError::Broken(e) => {
                tracing::warn!(error = %e, "Home not written: config.toml does not parse");
                "config.toml has an error: fix it to change Home".to_string()
            }
            e => {
                tracing::warn!(error = %e, "Home not written");
                e.to_string()
            }
        };
        self.show_error_toast(message, cx);
    }

    /// Whether `key`'s tile is answered for an older query: it takes no click.
    fn stale_tile(&self, key: &TileKey) -> bool {
        self.strip
            .tiles
            .iter()
            .find(|tile| &tile.key == key)
            .is_none_or(|tile| tile.condition.as_ref().is_some_and(|c| c.stale))
    }

    /// Whether a click on a tile acts: the root list shows with no panel
    /// over it and outside the Home edit mode, the click is not the release
    /// of a long press, and the tile is not answered for an older query.
    fn takes_clicks(&self, key: &TileKey) -> bool {
        self.stack.top().is_none()
            && self.panel.is_none()
            && !self.home_editing()
            && !self.strip.suppress_click
            && !self.stale_tile(key)
    }

    /// A click on `key`'s tile outside its buttons and the canvases that
    /// take the pointer: its first action. A canvas without an id takes no
    /// pointer, so a click on it is the tile's.
    pub(crate) fn click_tile(&mut self, key: &TileKey, cx: &mut Context<Self>) {
        if self.takes_clicks(key) {
            self.run_tile_primary(key, cx);
        }
    }

    /// A click on button `node` of `key`'s tile, which presses `action`.
    pub(crate) fn press_button(
        &mut self,
        key: &TileKey,
        node: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        if self.takes_clicks(key) {
            self.run_tile_action(key, Some(node.to_string()), action.to_string(), cx);
        }
    }

    /// A key the launcher does not bind, while a canvas has the focus: it
    /// goes to the canvas, and nowhere else.
    pub(crate) fn canvas_key(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tiles = &self.strip.tiles;
        if !self.strip.focus.on_canvas(tiles) {
            return;
        }
        let keys::RootFocus::Strip {
            tile: ix,
            part: Some(part),
        } = self.strip.focus.index(tiles)
        else {
            return;
        };
        let Some((node, _)) = keys::part_node(&tiles[ix], part) else {
            return;
        };
        let tile = tiles[ix].key.clone();
        cx.stop_propagation();
        // A held key's repeats go too, on purpose: a game moves while an
        // arrow is held.
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let modifiers = [
            (m.control, Modifiers::CTRL),
            (m.alt, Modifiers::ALT),
            (m.shift, Modifiers::SHIFT),
            (m.platform, Modifiers::SUPER),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(Modifiers::empty(), |all, (_, flag)| all | flag);
        let key = keystroke.key.clone();
        self.send_canvas_key(&tile, node, &key, modifiers, cx);
    }
}

/// The modifier of the action panel's key: Cmd on macOS, Ctrl elsewhere.
fn command_modifier() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::SUPER
    } else {
        Modifiers::CTRL
    }
}
