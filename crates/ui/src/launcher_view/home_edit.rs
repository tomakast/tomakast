//! The Home edit mode in the launcher view: getting in and out, its keys, its
//! tile menu, and the writes. [`strip::edit`] decides what a key does; this
//! applies it.
//!
//! While the mode lasts it has a focus handle of its own, which the strip's
//! row tracks, and the root's key context is [`KEY_CONTEXT`] alone: no
//! launcher binding fires, and the search field never sees a key. The mode
//! keeps its selection as an index in the core's Home list, mapped to a
//! strip position each time (see [`strip::edit`]). Every change is written to
//! `config.toml` at once.
//!
//! By mouse: a long press on a Home tile enters the mode, and a right-click on
//! a tile opens its panel. In the mode a press on a tile selects it and may
//! become a drag, which moves the tile once, on release; its remove badge
//! removes it, and a right-click opens its menu, whose rows run by click. A
//! press on the menu or on the strip's scrollbar is theirs; a mouse down
//! anywhere else leaves the mode. When the desktop asks for reduced motion the
//! tiles stay still, each with a dashed outline.

use std::time::Instant;

use gpui_kit::{
    Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Window,
};
use tomakast_core::{EDIT_HOME, HomeEdit, HomeWriteError, TileKey, ToastKind};

use super::LauncherView;
use crate::chrome;
use crate::strip::edit::{EditOp, UndoStack, edit_key_of, edit_step, selected_position};
use crate::strip::keys::Focus;
use crate::strip::{self, EditLook, ReducedMotionSource, TilePress};
use crate::theme::{EDIT_DRAG_SLOP, EDIT_LONG_PRESS};
use crate::toast::ToastPart;
use crate::views::action_panel::{ActionPanel, PanelEntry, PanelTarget};

/// Key context of the launcher view while the Home edit mode lasts and no
/// panel is open, in place of the launcher's own.
pub(crate) const KEY_CONTEXT: &str = "HomeEdit";
/// The command's toast when Home has no tile to edit.
const EMPTY_HOME: &str = "Home is empty: add widgets from the search";
/// The footer's hint for Enter in the mode.
const DONE: &str = "Done";

/// The Home edit mode while it lasts.
pub(crate) struct HomeEditState {
    /// The selected tile, as an index in the core's Home list.
    selected: Option<usize>,
    /// The removals of this mode, which Ctrl/Cmd+Z restores, last first.
    undo: UndoStack,
    /// The mode's own focus, which has the keyboard while no panel is open;
    /// never the strip's, which the strip hands back to the field.
    focus: FocusHandle,
    /// When the mode began, on the executor's clock: the tiles jiggle by it.
    started: Instant,
    /// The tile being dragged, if one is.
    drag: Option<Drag>,
    /// The desktop asks for reduced motion: the tiles do not jiggle. Read off
    /// the UI thread as the mode is entered; until the answer comes, the
    /// last one ([`strip::StripState::reduced_motion`]).
    reduced_motion: bool,
}

/// A tile dragged in the mode. Its x values are in the strip's scrolled
/// content, so they hold as the strip scrolls.
pub(crate) struct Drag {
    /// The dragged tile's strip position.
    from: usize,
    /// Where the pointer was when the tile was pressed.
    start_x: f32,
    /// Where the pointer is now: the tile draws `x - start_x` from its place.
    x: f32,
}

impl LauncherView {
    /// Whether the Home edit mode lasts.
    pub fn home_editing(&self) -> bool {
        self.home_edit.is_some()
    }

    /// The mode's focus handle while the mode lasts.
    pub(super) fn home_edit_focus(&self) -> Option<&FocusHandle> {
        self.home_edit.as_ref().map(|state| &state.focus)
    }

    /// The "Edit Home" command's `EditHome`: clears the query, so the strip
    /// shows Home, then enters the mode with the first tile selected, or
    /// says Home is empty when the strip has no tile.
    pub(crate) fn edit_home_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_root_query(String::new(), window, cx);
        self.read_strip(window, cx);
        match self.home_tiles().first() {
            Some(&(_, first)) => self.enter_home_edit(Some(first), window, cx),
            None => self.show_error_toast(EMPTY_HOME.to_string(), cx),
        }
    }

    /// Enters the mode with Home entry `selected` selected; the mode's focus
    /// takes the keyboard. Entered again, the mode keeps its undo stack.
    pub(crate) fn enter_home_edit(
        &mut self,
        selected: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel = None;
        self.pending = None;
        // The mode's selection takes the strip focus's place.
        self.strip.focus = Focus::List;
        self.strip.editing = true;
        self.strip.press = None;
        let started = cx.background_executor().now();
        if self.home_edit.is_none() {
            self.read_reduced_motion(cx);
        }
        let state = self.home_edit.get_or_insert_with(|| HomeEditState {
            selected,
            undo: UndoStack::default(),
            focus: cx.focus_handle(),
            started,
            drag: None,
            reduced_motion: self.strip.reduced_motion,
        });
        state.selected = selected;
        window.focus(&state.focus, cx);
        self.scroll_to_home_selection();
        cx.notify();
    }

    /// Leaves the mode: the undo stack goes, and with it the toast that
    /// offers it, and the tile menu if it is open. The strip focus goes to the
    /// selected tile, or to the list when Home is empty, and the field takes
    /// the keyboard back.
    pub(crate) fn leave_home_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.home_edit.take() else {
            return;
        };
        // The menu's entries act on the mode, which is gone.
        if self
            .panel
            .as_ref()
            .is_some_and(|panel| matches!(panel.target, PanelTarget::HomeEdit(_)))
        {
            self.panel = None;
        }
        self.clear_undo_toast(cx);
        self.strip.editing = false;
        self.strip.press = None;
        let focus = match self.home_tile_position(state.selected) {
            Some(pos) => Focus::Strip {
                tile: self.strip.tiles[pos].key.clone(),
                part: None,
            },
            None => Focus::List,
        };
        self.set_strip_focus(focus, window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Ends the mode without a window to give the keyboard to: the launcher
    /// hides. A long press in progress goes too.
    pub(super) fn forget_home_edit(&mut self) {
        self.home_edit = None;
        self.strip.editing = false;
        self.strip.press = None;
        self.strip.suppress_click = false;
    }

    /// Asks, on the background executor, whether the desktop wants reduced
    /// motion ([`ReducedMotionSource`]); the mode's tiles stop jiggling when
    /// it says so. The answer is kept for the next entry; one that comes
    /// after the mode ended changes no mode.
    fn read_reduced_motion(&self, cx: &mut Context<Self>) {
        let read = cx
            .try_global::<ReducedMotionSource>()
            .copied()
            .unwrap_or_default()
            .0;
        let answer = cx.background_executor().spawn(async move { read() });
        cx.spawn(async move |this, cx| {
            let reduced = answer.await == Some(true);
            let _ = this.update(cx, |this, cx| {
                this.strip.reduced_motion = reduced;
                if let Some(state) = &mut this.home_edit {
                    state.reduced_motion = reduced;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A press on `key`'s tile outside the mode, seen before its canvases
    /// take it: it ends the hold a long press put on clicks, and the left
    /// button on a Home tile starts a long press, which enters the mode once
    /// it lasts [`EDIT_LONG_PRESS`]. A release or a move cancels it (see
    /// [`Self::strip_pointer_moved`]).
    pub(crate) fn tile_pressed(
        &mut self,
        key: &TileKey,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.home_editing() {
            return;
        }
        self.strip.suppress_click = false;
        let home = self
            .strip
            .tiles
            .iter()
            .any(|tile| &tile.key == key && strip::takes_edit_home(tile));
        if !home {
            self.strip.press = None;
            return;
        }
        let timer = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(EDIT_LONG_PRESS).await;
            let _ = this.update_in(cx, |this, window, cx| this.long_pressed(window, cx));
        });
        self.strip.press = Some(TilePress {
            key: key.clone(),
            at: event.position,
            _timer: Some(timer),
        });
    }

    /// The long press lasted: the mode is entered with its tile selected.
    /// The release that follows is the press's own: no tile or button takes
    /// it as a click.
    fn long_pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.strip.press.take() else {
            return;
        };
        // A view pushed over the root in the meantime keeps the root hidden.
        if self.stack.top().is_some() {
            return;
        }
        let Some(ix) = self.home_index(&press.key) else {
            return;
        };
        self.strip.suppress_click = true;
        self.enter_home_edit(Some(ix), window, cx);
    }

    /// A right-click on `key`'s tile outside the mode: the strip focus goes
    /// to the tile and its panel opens, as Ctrl/Cmd+K would there.
    pub(crate) fn tile_context_menu(
        &mut self,
        key: &TileKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home_editing() || self.stack.top().is_some() {
            return;
        }
        // The panel's filter takes the keyboard, not the strip the tile is in.
        window.prevent_default();
        self.strip.press = None;
        self.strip.moved = true;
        let focus = Focus::Strip {
            tile: key.clone(),
            part: None,
        };
        self.set_strip_focus(focus, window, cx);
        self.open_panel(window, cx);
    }

    /// A press on `key`'s tile in the mode, which goes no further: the left
    /// button selects the tile, and a move then starts a drag; the right one
    /// selects it and opens its menu. The strip does not scroll to it, so the
    /// tile stays under the pointer.
    pub(crate) fn edit_tile_pressed(
        &mut self,
        key: &TileKey,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.prevent_default();
        let Some(ix) = self.home_index(key) else {
            return;
        };
        if self.panel.is_some() {
            self.close_panel(window, cx);
        }
        if let Some(state) = &mut self.home_edit {
            state.selected = Some(ix);
            state.drag = None;
        }
        match event.button {
            MouseButton::Left => {
                self.strip.press = Some(TilePress {
                    key: key.clone(),
                    at: event.position,
                    _timer: None,
                })
            }
            MouseButton::Right => self.open_home_edit_menu(ix, window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// A press on the remove badge of `key`'s tile: the left button removes
    /// the tile, as Delete does; another is a press on the tile.
    pub(crate) fn edit_badge_pressed(
        &mut self,
        key: &TileKey,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            self.edit_tile_pressed(key, event, window, cx);
            return;
        }
        cx.stop_propagation();
        window.prevent_default();
        let Some(ix) = self.home_index(key) else {
            return;
        };
        if self.panel.is_some() {
            self.close_panel(window, cx);
        }
        self.apply_home_edit(HomeEdit::Remove(ix), cx);
    }

    /// The pointer moved to `position`, anywhere in the window, while a tile
    /// is pressed or dragged, with `pressed` down. A press that moves farther
    /// than [`EDIT_DRAG_SLOP`] is no long press; in the mode it starts a drag.
    /// A dragged tile follows the pointer. A move without the left button
    /// down means its release was lost (it went up outside the window): the
    /// press ends, and a dragged tile goes back to its place, with nothing
    /// written.
    pub(crate) fn strip_pointer_moved(
        &mut self,
        position: Point<Pixels>,
        pressed: Option<MouseButton>,
        cx: &mut Context<Self>,
    ) {
        let dragging = self
            .home_edit
            .as_ref()
            .is_some_and(|state| state.drag.is_some());
        if self.strip.press.is_none() && !dragging {
            return;
        }
        if pressed != Some(MouseButton::Left) {
            self.strip.press = None;
            if let Some(state) = &mut self.home_edit {
                state.drag = None;
            }
            cx.notify();
            return;
        }
        let x = strip::content_x(&self.strip, position.x);
        if let Some(drag) = self
            .home_edit
            .as_mut()
            .and_then(|state| state.drag.as_mut())
        {
            drag.x = x;
            cx.notify();
            return;
        }
        let Some(press) = &self.strip.press else {
            return;
        };
        let (dx, dy) = (
            f32::from(position.x - press.at.x),
            f32::from(position.y - press.at.y),
        );
        if dx.hypot(dy) <= EDIT_DRAG_SLOP {
            return;
        }
        let Some(press) = self.strip.press.take() else {
            return;
        };
        let from = self
            .strip
            .tiles
            .iter()
            .position(|tile| tile.key == press.key);
        if let (Some(state), Some(from)) = (&mut self.home_edit, from) {
            state.drag = Some(Drag {
                from,
                start_x: strip::content_x(&self.strip, press.at.x),
                x,
            });
        }
        cx.notify();
    }

    /// The left button went up, anywhere in the window: a press ends, and a
    /// dragged tile lands where it was let go. The landing writes one move,
    /// only when the tile lands somewhere new.
    pub(crate) fn strip_pointer_released(&mut self, cx: &mut Context<Self>) {
        if self.strip.press.take().is_some() {
            cx.notify();
        }
        let Some(drag) = self.home_edit.as_mut().and_then(|state| state.drag.take()) else {
            return;
        };
        cx.notify();
        let to = strip::drag_landing(&self.strip.tiles, drag.from, drag.x - drag.start_x);
        if to == drag.from {
            return;
        }
        let tiles = self.home_tiles();
        let index = |pos: usize| tiles.iter().find(|&&(p, _)| p == pos).map(|&(_, ix)| ix);
        if let (Some(from), Some(to)) = (index(drag.from), index(to)) {
            self.apply_home_edit(HomeEdit::Move { from, to }, cx);
        }
    }

    /// A mouse down that reaches the launcher's root while the mode lasts:
    /// one outside the tiles and the action panel, as each tile's overlay and
    /// the panel hide the root from the pointer. It leaves the mode, closing
    /// the tile menu with it, and goes no further: no row, field or button
    /// under it takes it. A press on the strip's scrollbar is the bar's: it
    /// scrolls the strip, and the mode stays.
    pub(super) fn home_edit_outside_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.home_editing() || strip::over_scrollbar(&self.strip, event.position) {
            return;
        }
        cx.stop_propagation();
        window.prevent_default();
        self.panel = None;
        self.leave_home_edit(window, cx);
    }

    /// The Home index of `key`'s tile, if it is a Home tile.
    fn home_index(&self, key: &TileKey) -> Option<usize> {
        self.core.try_borrow().ok()?.home_index(key)
    }

    /// A key while the mode has the keyboard: the mode's, as [`strip::edit`] says,
    /// or nothing. It goes no further either way.
    pub(crate) fn home_edit_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(state) = &self.home_edit else {
            return;
        };
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let command = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        let Some(key) = edit_key_of(&keystroke.key, m.shift, command, event.is_held) else {
            return;
        };
        let indices: Vec<usize> = self.home_tiles().iter().map(|&(_, ix)| ix).collect();
        match edit_step(state.selected, key, &indices) {
            EditOp::Select(ix) => {
                self.set_home_selection(ix);
                cx.notify();
            }
            EditOp::Edit(edit) => {
                self.apply_home_edit(edit, cx);
            }
            EditOp::Undo => self.undo_home_edit(cx),
            EditOp::OpenMenu(ix) => self.open_home_edit_menu(ix, window, cx),
            EditOp::Leave => self.leave_home_edit(window, cx),
            EditOp::Nothing => {}
        }
    }

    /// Writes `edit` to the config file and shows the result; returns
    /// whether it was written. A removal goes on the undo stack, and a toast
    /// says how to restore it. The selection follows: a moved tile to its new
    /// place, a restored one to its own, after a removal the tile that comes
    /// next, else the last. A failed write shows Part 1's toasts; one refused
    /// because the file changed re-reads it first.
    pub(crate) fn apply_home_edit(&mut self, edit: HomeEdit, cx: &mut Context<Self>) -> bool {
        let written = self
            .core
            .try_borrow_mut()
            .map(|mut core| core.edit_home(edit.clone()));
        let removed = match written {
            Ok(Ok(removed)) => removed,
            Ok(Err(e)) => {
                // The file changed since Home was read: the strip shows it as
                // it is now, so the next try acts on that list, not on a stale
                // one until the watcher reports the change.
                if matches!(e, HomeWriteError::Changed)
                    && let Ok(mut core) = self.core.try_borrow_mut()
                {
                    core.home_file_changed(Vec::new());
                    self.strip.tiles = core.strip();
                    drop(core);
                    self.scroll_to_home_selection();
                    cx.notify();
                }
                self.show_home_write_error(e, cx);
                return false;
            }
            Err(_) => {
                self.show_error_toast("the core is busy".to_string(), cx);
                return false;
            }
        };
        let selected = match edit {
            HomeEdit::Remove(at) | HomeEdit::Resize(at, _) | HomeEdit::Insert { at, .. } => at,
            HomeEdit::Move { to, .. } => to,
        };
        // The tiles of the next tick, now: `Slot::Home` renumbers after a move.
        if let Ok(core) = self.core.try_borrow() {
            self.strip.tiles = core.strip();
        }
        self.set_home_selection(selected);
        if let Some(removed) = removed {
            if let Some(state) = &mut self.home_edit {
                state.undo.push(removed);
            }
            self.show_undo_toast(cx);
        }
        cx.notify();
        true
    }

    /// Ctrl/Cmd+Z: restores the last removed tile at its old place. A
    /// restore that fails stays on the stack, for the next try.
    fn undo_home_edit(&mut self, cx: &mut Context<Self>) {
        let Some(removed) = self.home_edit.as_mut().and_then(|state| state.undo.pop()) else {
            return;
        };
        let len = self
            .core
            .try_borrow()
            .map_or(removed.at, |core| core.home().len());
        if !self.apply_home_edit(removed.undo(len), cx) {
            if let Some(state) = &mut self.home_edit {
                state.undo.push(removed);
            }
            return;
        }
        if self
            .home_edit
            .as_ref()
            .is_some_and(|state| state.undo.is_empty())
        {
            self.clear_undo_toast(cx);
        }
    }

    /// "Press [Ctrl] [Z] to restore widget", the keys as chips.
    fn show_undo_toast(&mut self, cx: &mut Context<Self>) {
        let parts = vec![
            ToastPart::Text("Press".to_string()),
            ToastPart::Key(chrome::shortcut_label()[0].to_string()),
            ToastPart::Key("Z".to_string()),
            ToastPart::Text("to restore widget".to_string()),
        ];
        self.toasts.update(cx, |toasts, cx| {
            toasts.show_keys(ToastKind::Success, parts, cx)
        });
    }

    /// Clears the undo toast if it still shows; another toast stays.
    fn clear_undo_toast(&mut self, cx: &mut Context<Self>) {
        self.toasts.update(cx, |toasts, cx| toasts.clear_keys(cx));
    }

    /// Ctrl/Cmd+K on Home entry `ix`: the menu of its tile, its sizes when it
    /// has more than one (its own marked), then Remove.
    fn open_home_edit_menu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tile) = self
            .home_tiles()
            .into_iter()
            .find(|&(_, home)| home == ix)
            .map(|(pos, _)| self.strip.tiles[pos].clone())
        else {
            return;
        };
        let sizes = self
            .core
            .try_borrow()
            .map(|core| core.widget_sizes(&tile.key.widget))
            .unwrap_or_default();
        let mut entries: Vec<PanelEntry> = Vec::new();
        if sizes.len() > 1 {
            entries.extend(sizes.into_iter().map(|size| PanelEntry::Size {
                size,
                current: size == tile.size,
            }));
        }
        entries.push(PanelEntry::Remove);
        self.panel = Some(ActionPanel::new(
            PanelTarget::HomeEdit(ix),
            entries,
            window,
            cx,
        ));
        cx.notify();
    }

    /// Enter on one of the launcher's Home entries of the panel: "Edit Home"
    /// enters the mode on the panel's tile; in the mode's menu a size
    /// resizes the tile (its own size changes nothing) and Remove removes
    /// it. The panel closes first, back into the mode while it lasts.
    pub(super) fn confirm_home_entry(
        &mut self,
        entry: PanelEntry,
        target: PanelTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_panel(window, cx);
        match (entry, target) {
            (PanelEntry::EditHome, PanelTarget::Tile(key)) => {
                let ix = self
                    .core
                    .try_borrow()
                    .ok()
                    .and_then(|core| core.home_index(&key));
                self.enter_home_edit(ix, window, cx);
            }
            (PanelEntry::Size { size, current }, PanelTarget::HomeEdit(ix)) => {
                if !current {
                    self.apply_home_edit(HomeEdit::Resize(ix, size), cx);
                }
            }
            (PanelEntry::Remove, PanelTarget::HomeEdit(ix)) => {
                self.apply_home_edit(HomeEdit::Remove(ix), cx);
            }
            _ => {}
        }
    }

    /// How the mode draws the strip at `now`, and the next frame it asks for:
    /// for the jiggle, unless the desktop asks for reduced motion, and while
    /// a dragged tile scrolls the strip at its edge. `None` outside the mode.
    pub(super) fn home_edit_frame(
        &mut self,
        now: Instant,
        window: &mut Window,
    ) -> Option<EditLook> {
        let drag = self.home_edit.as_mut()?.drag.as_mut();
        if let Some(drag) = drag {
            let step = strip::edge_scroll(&self.strip, drag.x);
            if step != 0. {
                drag.x += step;
                window.request_animation_frame();
            }
        }
        let state = self.home_edit.as_ref()?;
        if !state.reduced_motion {
            window.request_animation_frame();
        }
        let dragged = state.drag.as_ref().and_then(|drag| {
            let tile = self.strip.tiles.get(drag.from)?;
            Some((tile.key.clone(), drag.x - drag.start_x))
        });
        Some(EditLook {
            selected: self
                .home_tile_position(state.selected)
                .map(|pos| self.strip.tiles[pos].key.clone()),
            elapsed: now.saturating_duration_since(state.started),
            reduced: state.reduced_motion,
            dragged,
        })
    }

    /// The footer in the mode: its context, the Enter hint, and whether the
    /// Ctrl/Cmd+K hint shows (a tile is selected); `None` outside the mode.
    pub(super) fn home_edit_footer(&self) -> Option<(String, Option<String>, bool)> {
        let state = self.home_edit.as_ref()?;
        let menu = self.home_tile_position(state.selected).is_some();
        Some((EDIT_HOME.to_string(), Some(DONE.to_string()), menu))
    }

    /// The strip's Home tiles: their strip positions and Home indices, in
    /// strip order.
    fn home_tiles(&self) -> Vec<(usize, usize)> {
        let Ok(core) = self.core.try_borrow() else {
            return Vec::new();
        };
        self.strip
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(pos, tile)| Some((pos, core.home_index(&tile.key)?)))
            .collect()
    }

    /// The strip position of Home entry `selected`: its tile, else the next
    /// tile, else the last; `None` with no tile.
    fn home_tile_position(&self, selected: Option<usize>) -> Option<usize> {
        let tiles = self.home_tiles();
        let indices: Vec<usize> = tiles.iter().map(|&(_, ix)| ix).collect();
        let at = selected_position(selected?, &indices)?;
        Some(tiles[at].0)
    }

    /// Selects Home entry `ix`; the strip scrolls to its tile.
    fn set_home_selection(&mut self, ix: usize) {
        if let Some(state) = &mut self.home_edit {
            state.selected = Some(ix);
        }
        self.scroll_to_home_selection();
    }

    fn scroll_to_home_selection(&self) {
        let selected = self.home_edit.as_ref().and_then(|state| state.selected);
        if let Some(pos) = self.home_tile_position(selected) {
            strip::scroll_to(&self.strip, &self.strip.tiles, pos);
        }
    }
}
