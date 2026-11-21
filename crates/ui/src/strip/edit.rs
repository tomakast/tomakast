//! The Home edit mode's logic as pure functions: what a key means, where the
//! selection and the tiles go, the undo stack, the drop position of a drag and
//! the tiles' jiggle. The launcher view wires them to the screen.
//!
//! Indices in [`EditOp`] are Home indices (positions in `core.home()`), not
//! strip positions: a strip shows only the Home entries that show, so `indices`
//! maps a strip position to its Home index, ascending.

use std::time::Duration;

use tomakast_core::{HomeEdit, Removed};

use super::layout::TileSlot;
use crate::theme::{EDIT_WOBBLE_PERIOD, EDIT_WOBBLE_PX};

/// A key the edit mode acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKey {
    Prev,
    Next,
    MoveLeft,
    MoveRight,
    Remove,
    Undo,
    Menu,
    Done,
}

/// What a key does. Indices are Home indices (`core.home()`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EditOp {
    Select(usize),
    Edit(HomeEdit),
    Undo,
    OpenMenu(usize),
    Leave,
    Nothing,
}

/// The key named `key` (GPUI's keystroke key, e.g. "left", "z") with these modifiers; `None`
/// for a key the mode swallows. `command` is Ctrl, or Cmd on macOS. `held` repeats are `None`
/// but for the arrows and Tab: a held Delete removes one tile and a held Ctrl/Cmd+Z restores
/// one, and Enter, Esc or Ctrl/Cmd+K still held from the press that entered the mode neither
/// leave it nor open the menu.
pub(crate) fn edit_key_of(key: &str, shift: bool, command: bool, held: bool) -> Option<EditKey> {
    let key = match (key, shift, command) {
        ("left", false, false) | ("tab", true, false) => EditKey::Prev,
        ("right", false, false) | ("tab", false, false) => EditKey::Next,
        ("left", true, false) => EditKey::MoveLeft,
        ("right", true, false) => EditKey::MoveRight,
        ("delete" | "backspace", false, false) => EditKey::Remove,
        ("z", false, true) => EditKey::Undo,
        ("k", false, true) => EditKey::Menu,
        ("escape" | "enter", false, false) => EditKey::Done,
        _ => return None,
    };
    let repeats = matches!(
        key,
        EditKey::Prev | EditKey::Next | EditKey::MoveLeft | EditKey::MoveRight
    );
    (!held || repeats).then_some(key)
}

/// `selected` (a Home index) after `key`, over the strip's tiles, whose Home indices are
/// `indices` in strip order.
pub(crate) fn edit_step(selected: Option<usize>, key: EditKey, indices: &[usize]) -> EditOp {
    match key {
        EditKey::Done => return EditOp::Leave,
        EditKey::Undo => return EditOp::Undo,
        _ => {}
    }
    let Some(&first) = indices.first() else {
        return EditOp::Nothing;
    };
    let Some(pos) = selected.and_then(|s| selected_position(s, indices)) else {
        // Nothing is selected: a key that moves picks the first tile.
        return match key {
            EditKey::Prev | EditKey::Next | EditKey::MoveLeft | EditKey::MoveRight => {
                EditOp::Select(first)
            }
            _ => EditOp::Nothing,
        };
    };
    let here = indices[pos];
    match key {
        EditKey::Prev => EditOp::Select(indices[pos.saturating_sub(1)]),
        EditKey::Next => EditOp::Select(indices[(pos + 1).min(indices.len() - 1)]),
        EditKey::MoveLeft => match pos.checked_sub(1) {
            Some(before) => EditOp::Edit(HomeEdit::Move {
                from: here,
                to: indices[before],
            }),
            None => EditOp::Nothing,
        },
        EditKey::MoveRight => match indices.get(pos + 1) {
            Some(&after) => EditOp::Edit(HomeEdit::Move {
                from: here,
                to: after,
            }),
            None => EditOp::Nothing,
        },
        EditKey::Remove => EditOp::Edit(HomeEdit::Remove(here)),
        EditKey::Menu => EditOp::OpenMenu(here),
        EditKey::Done | EditKey::Undo => unreachable!("taken above"),
    }
}

/// The strip position of `selected`: the first tile whose Home index is ≥ `selected`, else
/// the last tile; `None` for an empty strip.
pub(crate) fn selected_position(selected: usize, indices: &[usize]) -> Option<usize> {
    let last = indices.len().checked_sub(1)?;
    Some(
        indices
            .iter()
            .position(|&ix| ix >= selected)
            .unwrap_or(last),
    )
}

/// The removals of this mode, last first.
#[derive(Default)]
pub(crate) struct UndoStack(Vec<Removed>);

impl UndoStack {
    pub(crate) fn push(&mut self, removed: Removed) {
        self.0.push(removed);
    }

    pub(crate) fn pop(&mut self) -> Option<Removed> {
        self.0.pop()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The strip position a dragged tile lands on with its centre at content x `x`: the number of
/// the other tiles whose centre it reached. `others` are the slots of the strip's other tiles,
/// at their places, so a tile past them all lands at the end of the strip.
pub(crate) fn drop_position(others: &[TileSlot], x: f32) -> usize {
    others
        .iter()
        .take_while(|slot| slot.x + slot.width / 2. <= x)
        .count()
}

/// Tile `ix`'s jiggle offset in px at `t` since the mode began.
pub(crate) fn wobble(ix: usize, t: Duration) -> (f32, f32) {
    let turns = t.as_secs_f32() / EDIT_WOBBLE_PERIOD.as_secs_f32();
    let phase = ix as f32 * 0.37;
    let at = |shift: f32| EDIT_WOBBLE_PX * (std::f32::consts::TAU * (turns + phase + shift)).sin();
    (at(0.), at(0.25))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strip::layout::slots;
    use EditKey::*;
    use tomakast_core::WidgetSize::{Double, Single};

    #[test]
    fn arrows_and_tab_select_and_stop_at_the_ends() {
        let ix = [0, 1, 2];
        assert_eq!(edit_step(Some(2), Next, &ix), EditOp::Select(2));
        assert_eq!(edit_step(Some(0), Prev, &ix), EditOp::Select(0));
        assert_eq!(edit_step(Some(0), Next, &ix), EditOp::Select(1));
        assert_eq!(edit_step(Some(2), Prev, &ix), EditOp::Select(1));
    }

    #[test]
    fn move_skips_entries_that_do_not_show() {
        let ix = [0, 2, 3];
        assert_eq!(
            edit_step(Some(0), MoveRight, &ix),
            EditOp::Edit(HomeEdit::Move { from: 0, to: 2 })
        );
        assert_eq!(
            edit_step(Some(2), MoveLeft, &ix),
            EditOp::Edit(HomeEdit::Move { from: 2, to: 0 })
        );
        assert_eq!(edit_step(Some(0), MoveLeft, &ix), EditOp::Nothing);
        assert_eq!(edit_step(Some(3), MoveRight, &ix), EditOp::Nothing);
    }

    #[test]
    fn remove_names_the_selected_entry() {
        assert_eq!(
            edit_step(Some(2), Remove, &[0, 2, 3]),
            EditOp::Edit(HomeEdit::Remove(2))
        );
        assert_eq!(edit_step(Some(2), Menu, &[0, 2, 3]), EditOp::OpenMenu(2));
    }

    #[test]
    fn no_selection_selects_the_first_tile_on_any_move_key() {
        for key in [Prev, Next, MoveLeft, MoveRight] {
            assert_eq!(edit_step(None, key, &[3, 5]), EditOp::Select(3), "{key:?}");
        }
        assert_eq!(edit_step(None, Remove, &[3, 5]), EditOp::Nothing);
        assert_eq!(edit_step(None, Menu, &[3, 5]), EditOp::Nothing);
    }

    #[test]
    fn an_empty_strip_takes_only_done_and_undo() {
        for key in [Prev, Next, MoveLeft, MoveRight, Remove, Menu] {
            assert_eq!(edit_step(Some(0), key, &[]), EditOp::Nothing, "{key:?}");
            assert_eq!(edit_step(None, key, &[]), EditOp::Nothing, "{key:?}");
        }
        assert_eq!(edit_step(None, Done, &[]), EditOp::Leave);
        assert_eq!(edit_step(None, Undo, &[]), EditOp::Undo);
    }

    #[test]
    fn selection_after_removal_is_the_next_tile_else_the_last() {
        assert_eq!(selected_position(1, &[0, 2]), Some(1));
        assert_eq!(selected_position(3, &[0, 2]), Some(1));
        assert_eq!(selected_position(0, &[]), None);
        assert_eq!(selected_position(0, &[0, 2]), Some(0));
    }

    #[test]
    fn keys_map_with_modifiers_and_held_repeats() {
        assert_eq!(edit_key_of("backspace", false, false, true), None);
        assert_eq!(edit_key_of("backspace", false, false, false), Some(Remove));
        assert_eq!(edit_key_of("delete", false, false, false), Some(Remove));
        assert_eq!(edit_key_of("right", false, false, true), Some(Next));
        assert_eq!(edit_key_of("z", false, true, false), Some(Undo));
        assert_eq!(edit_key_of("z", false, true, true), None);
        assert_eq!(edit_key_of("k", false, true, false), Some(Menu));
        assert_eq!(edit_key_of("a", false, false, false), None);
        assert_eq!(edit_key_of("z", false, false, false), None);
        assert_eq!(edit_key_of("left", false, false, false), Some(Prev));
        assert_eq!(edit_key_of("left", true, false, false), Some(MoveLeft));
        assert_eq!(edit_key_of("right", true, false, false), Some(MoveRight));
        assert_eq!(edit_key_of("tab", false, false, false), Some(Next));
        assert_eq!(edit_key_of("tab", true, false, false), Some(Prev));
        assert_eq!(edit_key_of("escape", false, false, false), Some(Done));
        assert_eq!(edit_key_of("enter", false, false, false), Some(Done));
    }

    /// A key held since before the mode began repeats into it: Enter or Esc held from the
    /// "Edit Home" that entered it must not leave at once, nor Ctrl/Cmd+K open the menu.
    #[test]
    fn held_done_and_menu_are_ignored() {
        assert_eq!(edit_key_of("enter", false, false, true), None);
        assert_eq!(edit_key_of("escape", false, false, true), None);
        assert_eq!(edit_key_of("k", false, true, true), None);
    }

    #[test]
    fn drop_position_follows_the_tile_centres() {
        // The other tiles' centres at 193.5, 556.5 and 1101.
        let others = slots(&[Single, Single, Double], 750.);
        assert_eq!(drop_position(&others, 0.), 0);
        assert_eq!(drop_position(&others, 193.), 0);
        assert_eq!(drop_position(&others, 193.5), 1, "a centre reached counts");
        assert_eq!(drop_position(&others, 300.), 1);
        assert_eq!(drop_position(&others, 560.), 2);
        assert_eq!(drop_position(&others, 5000.), 3, "past them all: the end");
        assert_eq!(drop_position(&[], 10.), 0);
    }

    #[test]
    fn wobble_is_small_and_out_of_phase() {
        let mut apart = false;
        for ms in 0..1000 {
            let t = Duration::from_millis(ms);
            for ix in 0..4 {
                let (dx, dy) = wobble(ix, t);
                assert!(dx.abs() <= 1.5 && dy.abs() <= 1.5, "{ix} {ms}: {dx} {dy}");
            }
            apart |= wobble(0, t) != wobble(1, t);
        }
        assert!(apart);
    }

    #[test]
    fn undo_stack_is_last_first() {
        use tomakast_core::{HomeEntry, Params, WidgetKey};
        let removed = |at| Removed {
            at,
            entry: HomeEntry {
                widget: WidgetKey::new("calc", "result"),
                size: None,
                params: Params::default(),
            },
            comments: String::new(),
        };
        let mut stack = UndoStack::default();
        assert!(stack.is_empty());
        stack.push(removed(1));
        stack.push(removed(2));
        assert!(!stack.is_empty());
        assert_eq!(stack.pop().map(|r| r.at), Some(2));
        assert_eq!(stack.pop().map(|r| r.at), Some(1));
        assert!(stack.pop().is_none());
    }
}
