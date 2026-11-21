//! The strip's keys: where the keyboard focus goes at the root list, as a
//! pure function of the focus, the key and the strip's focusable parts.
//!
//! The keyboard stays in the search field; the selection moves over the
//! screen. It is on a row of the list ([`RootFocus::List`]), on a condition
//! tile that took it ([`RootFocus::TileSelected`]), in the strip
//! ([`RootFocus::Strip`]), on a tile or on one of its parts: a button or a
//! focusable canvas, or in the field itself ([`RootFocus::Field`]), where
//! nothing is highlighted. A focused canvas takes the keys itself.
//!
//! Up moves the selection up the screen, the list's first row to the strip
//! to the field, and Down moves it back.

use tomakast_core::strip::StripTile;
use tomakast_core::{
    DYNAMIC_SEARCH_DEADLINE, Frame, NodeKind, SOFT_RENDER_DEADLINE, Slot, TileKey,
};

/// Where the keyboard is at the root, by tile index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootFocus {
    List,
    /// Nothing is selected: Up there is the history's, and Enter and
    /// Ctrl+K act on the list's first row.
    Field,
    TileSelected(usize),
    Strip {
        tile: usize,
        part: Option<usize>,
    },
}

/// A focusable part of a tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Button,
    Canvas,
}

/// A key the root acts on; `Other` is any key the launcher does not bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    ShiftTab,
    Enter,
    Escape,
    Actions,
    Other,
}

pub(crate) struct KeyCtx<'a> {
    /// The list's first row is selected, or the list is empty: Up leaves it.
    pub on_first_row: bool,
    /// The field shows a recalled query, unedited: Down in the field is
    /// the history's.
    pub recalled: bool,
    /// The parts of every tile of the strip, in strip order.
    pub tiles: &'a [Vec<Part>],
}

/// What a key does besides moving the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Not the strip's: the key does what it does without a strip.
    Unhandled,
    Handled,
    /// The focus went back to the search field.
    ToList,
    RunPrimary(usize),
    /// The button `part` of `tile` was pressed.
    RunPart {
        tile: usize,
        part: usize,
    },
    OpenActions(usize),
    /// The key goes to the canvas `part` of `tile`.
    ToCanvas {
        tile: usize,
        part: usize,
    },
}

/// The focus after `key` at `focus`, and what else the key does.
/// A focus on a tile the strip no longer has is the list's.
pub(crate) fn step(focus: RootFocus, key: Key, ctx: &KeyCtx) -> (RootFocus, Outcome) {
    let tiles = ctx.tiles.len();
    match focus {
        RootFocus::List => match key {
            Key::Up if ctx.on_first_row && tiles > 0 => (entered(0), Outcome::Handled),
            Key::Up if ctx.on_first_row => (RootFocus::Field, Outcome::Handled),
            Key::Tab if tiles > 0 => (entered(0), Outcome::Handled),
            _ => (RootFocus::List, Outcome::Unhandled),
        },
        // Up is the launcher's: the history, or nothing for a typed query.
        RootFocus::Field => match key {
            Key::Down if ctx.recalled => (focus, Outcome::Unhandled),
            Key::Down if tiles > 0 => (entered(0), Outcome::Handled),
            Key::Down => (RootFocus::List, Outcome::ToList),
            Key::Tab if tiles > 0 => (entered(0), Outcome::Handled),
            _ => (focus, Outcome::Unhandled),
        },
        RootFocus::TileSelected(t) if t >= tiles => (RootFocus::List, Outcome::Unhandled),
        RootFocus::TileSelected(t) => match key {
            Key::Up => (RootFocus::Field, Outcome::Handled),
            Key::Enter => (focus, Outcome::RunPrimary(t)),
            Key::Actions => (focus, Outcome::OpenActions(t)),
            Key::Down => (RootFocus::List, Outcome::ToList),
            Key::Tab => (entered(t), Outcome::Handled),
            _ => (focus, Outcome::Unhandled),
        },
        RootFocus::Strip { tile, .. } if tile >= tiles => (RootFocus::List, Outcome::Unhandled),
        RootFocus::Strip { tile, part } => {
            let parts = &ctx.tiles[tile];
            // A part the tile no longer has: the tile itself.
            let part = part.filter(|&p| p < parts.len());
            let at = RootFocus::Strip { tile, part };
            match key {
                Key::Tab => (tab(at, ctx.tiles, true), Outcome::Handled),
                Key::ShiftTab => (tab(at, ctx.tiles, false), Outcome::Handled),
                _ => match part.map(|p| (p, parts[p])) {
                    Some((p, Part::Canvas)) => match key {
                        Key::Escape => (entered(tile), Outcome::Handled),
                        _ => (at, Outcome::ToCanvas { tile, part: p }),
                    },
                    button => match key {
                        Key::Left => (entered(tile.saturating_sub(1)), Outcome::Handled),
                        Key::Right => (entered((tile + 1).min(tiles - 1)), Outcome::Handled),
                        Key::Down | Key::Escape => (RootFocus::List, Outcome::ToList),
                        Key::Up => (RootFocus::Field, Outcome::Handled),
                        Key::Enter => match button {
                            Some((p, _)) => (at, Outcome::RunPart { tile, part: p }),
                            None => (at, Outcome::RunPrimary(tile)),
                        },
                        Key::Actions => (at, Outcome::OpenActions(tile)),
                        Key::Other => (RootFocus::List, Outcome::Unhandled),
                        // Taken above.
                        Key::Tab | Key::ShiftTab => (at, Outcome::Handled),
                    },
                },
            }
        }
    }
}

/// Tile `tile` itself, none of its parts.
fn entered(tile: usize) -> RootFocus {
    RootFocus::Strip { tile, part: None }
}

/// The next (`forward`) or previous stop of Tab from `at`, a strip focus:
/// each tile, then its parts, then the next tile; it stays at either end.
fn tab(at: RootFocus, tiles: &[Vec<Part>], forward: bool) -> RootFocus {
    let RootFocus::Strip { tile, part } = at else {
        return at;
    };
    let parts = tiles[tile].len();
    let next = if forward {
        match part {
            None if parts > 0 => Some((tile, Some(0))),
            Some(p) if p + 1 < parts => Some((tile, Some(p + 1))),
            _ => (tile + 1 < tiles.len()).then(|| (tile + 1, None)),
        }
    } else {
        match part {
            Some(0) => Some((tile, None)),
            Some(p) => Some((tile, Some(p - 1))),
            None => tile
                .checked_sub(1)
                .map(|prev| (prev, tiles[prev].len().checked_sub(1))),
        }
    };
    match next {
        Some((tile, part)) => RootFocus::Strip { tile, part },
        None => at,
    }
}

/// The root focus by what it names, so it follows its tile when the strip
/// changes around it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Focus {
    #[default]
    List,
    Field,
    TileSelected(TileKey),
    /// `part` is the node id of a button or a focusable canvas.
    Strip {
        tile: TileKey,
        part: Option<String>,
    },
}

impl Focus {
    /// The tile this focus is on, if any.
    pub(crate) fn tile(&self) -> Option<&TileKey> {
        match self {
            Focus::List | Focus::Field => None,
            Focus::TileSelected(key) | Focus::Strip { tile: key, .. } => Some(key),
        }
    }

    /// The node id of the focused part, if any.
    pub(crate) fn part(&self) -> Option<&str> {
        match self {
            Focus::Strip {
                part: Some(part), ..
            } => Some(part),
            _ => None,
        }
    }

    /// This focus by index in `tiles`: the list when its tile is not there,
    /// the tile itself when its part is not. A search has one tile per
    /// widget, so a search tile is found again when its new answer has other
    /// parameters.
    pub(crate) fn index(&self, tiles: &[StripTile]) -> RootFocus {
        let at = |key: &TileKey| {
            tiles.iter().position(|tile| &tile.key == key).or_else(|| {
                let search = |k: &TileKey| k.slot == Slot::Search && k.widget == key.widget;
                tiles
                    .iter()
                    .position(|tile| search(key) && search(&tile.key))
            })
        };
        match self {
            Focus::List => RootFocus::List,
            Focus::Field => RootFocus::Field,
            Focus::TileSelected(key) => at(key).map_or(RootFocus::List, RootFocus::TileSelected),
            Focus::Strip { tile, part } => match at(tile) {
                None => RootFocus::List,
                Some(ix) => RootFocus::Strip {
                    tile: ix,
                    part: part.as_deref().and_then(|id| {
                        let frame = tiles[ix].frame.as_deref()?;
                        frame_parts(frame)
                            .iter()
                            .position(|&(_, node)| frame.nodes[node].id == id)
                    }),
                },
            },
        }
    }

    /// `focus`, an index in `tiles`, by what it names there.
    pub(crate) fn named(focus: RootFocus, tiles: &[StripTile]) -> Focus {
        let key = |ix: usize| tiles.get(ix).map(|tile| tile.key.clone());
        match focus {
            RootFocus::List => Focus::List,
            RootFocus::Field => Focus::Field,
            RootFocus::TileSelected(ix) => key(ix).map_or(Focus::List, Focus::TileSelected),
            RootFocus::Strip { tile, part } => match key(tile) {
                None => Focus::List,
                Some(key) => Focus::Strip {
                    tile: key,
                    part: part.and_then(|part| {
                        let frame = tiles[tile].frame.as_deref()?;
                        let (_, node) = *frame_parts(frame).get(part)?;
                        Some(frame.nodes[node].id.clone())
                    }),
                },
            },
        }
    }

    /// Whether this focus is on a canvas of `tiles`, which takes the keys.
    pub(crate) fn on_canvas(&self, tiles: &[StripTile]) -> bool {
        match self.index(tiles) {
            RootFocus::Strip {
                tile,
                part: Some(part),
            } => tile_parts(&tiles[tile])[part].0 == Part::Canvas,
            _ => false,
        }
    }
}

/// The kinds of the focusable parts of every tile of `tiles`.
pub(crate) fn part_kinds(tiles: &[StripTile]) -> Vec<Vec<Part>> {
    tiles
        .iter()
        .map(|tile| tile_parts(tile).into_iter().map(|(kind, _)| kind).collect())
        .collect()
}

/// The id of the node of part `part` of `tile`, and the action it presses
/// when it is a button.
pub(crate) fn part_node(tile: &StripTile, part: usize) -> Option<(String, Option<String>)> {
    let frame = tile.frame.as_deref()?;
    let (_, ix) = *frame_parts(frame).get(part)?;
    let node = &frame.nodes[ix];
    let action = match &node.kind {
        NodeKind::Button(button) => Some(button.action.clone()),
        _ => None,
    };
    Some((node.id.clone(), action))
}

/// Enter and Ctrl+K at the root wait at most this long for the tiles of a
/// search in flight: a dynamic source's deadline, then the first frame's.
pub(crate) const ENTER_WAIT: std::time::Duration =
    DYNAMIC_SEARCH_DEADLINE.saturating_add(SOFT_RENDER_DEADLINE);

/// The focusable parts of `frame`, in node order, with their node index:
/// its buttons and focusable canvases. Both have ids, unique in the frame.
pub(crate) fn frame_parts(frame: &Frame) -> Vec<(Part, usize)> {
    frame
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(ix, node)| match &node.kind {
            NodeKind::Button(_) => Some((Part::Button, ix)),
            NodeKind::Canvas(canvas) if canvas.focusable => Some((Part::Canvas, ix)),
            _ => None,
        })
        .collect()
}

/// The focusable parts of `tile`'s frame; none without one.
pub(crate) fn tile_parts(tile: &StripTile) -> Vec<(Part, usize)> {
    tile.frame.as_deref().map(frame_parts).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Key::*;
    use Part::{Button, Canvas};
    use RootFocus::{Field, List, Strip, TileSelected};

    fn strip(tile: usize, part: Option<usize>) -> RootFocus {
        Strip { tile, part }
    }

    fn strip_tile(widget: &str, expression: &str, slot: tomakast_core::Slot) -> StripTile {
        StripTile {
            key: TileKey {
                widget: tomakast_core::WidgetKey::new("calc", widget),
                params: tomakast_core::widget::params([("expression".into(), expression.into())]),
                slot,
            },
            title: widget.into(),
            size: tomakast_core::WidgetSize::Single,
            frame: None,
            condition: None,
        }
    }

    /// A search has one tile per widget: a selected one stays selected when its new answer
    /// has other parameters. A Home tile is followed by its whole key only.
    #[test]
    fn a_focused_search_tile_follows_its_new_answer() {
        use tomakast_core::Slot;
        let old = strip_tile("result", "2+2", Slot::Search);
        let new = [
            strip_tile("other", "1", Slot::Search),
            strip_tile("result", "2+23", Slot::Search),
        ];
        let focus = Focus::TileSelected(old.key.clone());
        assert_eq!(focus.index(&new), TileSelected(1));
        assert_eq!(
            Focus::named(focus.index(&new), &new),
            Focus::TileSelected(new[1].key.clone())
        );

        let home = strip_tile("result", "2+2", Slot::Home(0));
        let moved = [strip_tile("result", "2+23", Slot::Home(0))];
        assert_eq!(Focus::TileSelected(home.key).index(&moved), List);
    }

    /// On the first row, no recalled query.
    fn ctx(tiles: &[Vec<Part>]) -> KeyCtx<'_> {
        KeyCtx {
            on_first_row: true,
            recalled: false,
            tiles,
        }
    }

    #[test]
    fn up_on_the_first_row_enters_the_strip_else_the_field() {
        let tiles = [vec![], vec![]];
        assert_eq!(
            step(List, Up, &ctx(&tiles)),
            (strip(0, None), Outcome::Handled)
        );
        // No tiles: the field.
        assert_eq!(step(List, Up, &ctx(&[])), (Field, Outcome::Handled));
        // A recalled query changes nothing on the list.
        let recalled = KeyCtx {
            recalled: true,
            ..ctx(&tiles)
        };
        assert_eq!(
            step(List, Up, &recalled),
            (strip(0, None), Outcome::Handled)
        );
    }

    #[test]
    fn up_below_the_first_row_is_the_lists() {
        let tiles = [vec![]];
        let lower = KeyCtx {
            on_first_row: false,
            ..ctx(&tiles)
        };
        assert_eq!(step(List, Up, &lower), (List, Outcome::Unhandled));
        assert_eq!(step(List, Down, &lower), (List, Outcome::Unhandled));
    }

    #[test]
    fn up_in_the_field_is_the_launchers() {
        // It recalls the history, or does nothing for a typed query.
        for recalled in [false, true] {
            let tiles = [vec![]];
            let ctx = KeyCtx {
                recalled,
                ..ctx(&tiles)
            };
            assert_eq!(step(Field, Up, &ctx), (Field, Outcome::Unhandled));
        }
    }

    #[test]
    fn down_in_the_field_enters_the_strip_else_the_list() {
        let tiles = [vec![Button], vec![]];
        assert_eq!(
            step(Field, Down, &ctx(&tiles)),
            (strip(0, None), Outcome::Handled)
        );
        assert_eq!(step(Field, Down, &ctx(&[])), (List, Outcome::ToList));
        // A recalled query: Down is the history's.
        let recalled = KeyCtx {
            recalled: true,
            ..ctx(&tiles)
        };
        assert_eq!(step(Field, Down, &recalled), (Field, Outcome::Unhandled));
    }

    #[test]
    fn the_field_leaves_other_keys_alone_but_tab() {
        let tiles = [vec![]];
        let ctx = ctx(&tiles);
        for key in [Left, Right, ShiftTab, Enter, Escape, Actions, Other] {
            assert_eq!(
                step(Field, key, &ctx),
                (Field, Outcome::Unhandled),
                "{key:?}"
            );
        }
        assert_eq!(step(Field, Tab, &ctx), (strip(0, None), Outcome::Handled));
        assert_eq!(
            step(Field, Tab, &super::tests::ctx(&[])),
            (Field, Outcome::Unhandled)
        );
    }

    #[test]
    fn tab_enters_the_strip_even_when_up_would_not() {
        let tiles = [vec![]];
        let lower = KeyCtx {
            on_first_row: false,
            ..ctx(&tiles)
        };
        assert_eq!(step(List, Tab, &lower), (strip(0, None), Outcome::Handled));
        assert_eq!(step(List, Tab, &ctx(&[])), (List, Outcome::Unhandled));
        for key in [Down, Left, Right, ShiftTab, Enter, Escape, Actions, Other] {
            assert_eq!(
                step(List, key, &ctx(&tiles)),
                (List, Outcome::Unhandled),
                "{key:?}"
            );
        }
    }

    #[test]
    fn selected_tile_takes_only_enter_and_actions() {
        let tiles = [vec![], vec![]];
        let ctx = ctx(&tiles);
        let at = TileSelected(1);
        assert_eq!(step(at, Enter, &ctx), (at, Outcome::RunPrimary(1)));
        assert_eq!(step(at, Actions, &ctx), (at, Outcome::OpenActions(1)));
        // Up goes on up the screen, to the field.
        assert_eq!(step(at, Up, &ctx), (Field, Outcome::Handled));
        for key in [Left, Right, Escape, Other] {
            assert_eq!(step(at, key, &ctx), (at, Outcome::Unhandled), "{key:?}");
        }
        assert_eq!(step(at, Tab, &ctx), (strip(1, None), Outcome::Handled));
    }

    #[test]
    fn down_leaves_the_selected_tile_for_the_list() {
        let tiles = [vec![]];
        assert_eq!(
            step(TileSelected(0), Down, &ctx(&tiles)),
            (List, Outcome::ToList)
        );
    }

    #[test]
    fn arrows_move_between_tiles_and_stop_at_the_ends() {
        let tiles = [vec![Button], vec![], vec![]];
        let ctx = ctx(&tiles);
        assert_eq!(
            step(strip(0, Some(0)), Right, &ctx),
            (strip(1, None), Outcome::Handled)
        );
        assert_eq!(
            step(strip(1, None), Right, &ctx),
            (strip(2, None), Outcome::Handled)
        );
        assert_eq!(
            step(strip(2, None), Right, &ctx),
            (strip(2, None), Outcome::Handled)
        );
        assert_eq!(
            step(strip(1, None), Left, &ctx),
            (strip(0, None), Outcome::Handled)
        );
        assert_eq!(
            step(strip(0, None), Left, &ctx),
            (strip(0, None), Outcome::Handled)
        );
        // Up goes to the field, from the tile or its button; Down and
        // Escape go back to the list.
        assert_eq!(step(strip(1, None), Up, &ctx), (Field, Outcome::Handled));
        assert_eq!(step(strip(0, Some(0)), Up, &ctx), (Field, Outcome::Handled));
        assert_eq!(step(strip(1, None), Down, &ctx), (List, Outcome::ToList));
        assert_eq!(
            step(strip(0, Some(0)), Escape, &ctx),
            (List, Outcome::ToList)
        );
    }

    #[test]
    fn enter_runs_the_tile_or_its_button_and_actions_open_its_panel() {
        let tiles = [vec![Button, Button]];
        let ctx = ctx(&tiles);
        assert_eq!(
            step(strip(0, None), Enter, &ctx),
            (strip(0, None), Outcome::RunPrimary(0))
        );
        assert_eq!(
            step(strip(0, Some(1)), Enter, &ctx),
            (strip(0, Some(1)), Outcome::RunPart { tile: 0, part: 1 })
        );
        assert_eq!(
            step(strip(0, Some(1)), Actions, &ctx),
            (strip(0, Some(1)), Outcome::OpenActions(0))
        );
    }

    #[test]
    fn tab_walks_parts_then_the_next_tile_shift_tab_walks_back() {
        let tiles = [vec![Button, Canvas], vec![], vec![Button]];
        let ctx = ctx(&tiles);
        let walk = [
            strip(0, None),
            strip(0, Some(0)),
            strip(0, Some(1)),
            strip(1, None),
            strip(2, None),
            strip(2, Some(0)),
        ];
        for pair in walk.windows(2) {
            assert_eq!(step(pair[0], Tab, &ctx), (pair[1], Outcome::Handled));
            assert_eq!(step(pair[1], ShiftTab, &ctx), (pair[0], Outcome::Handled));
        }
        // Both stop at the ends.
        let last = strip(2, Some(0));
        assert_eq!(step(last, Tab, &ctx), (last, Outcome::Handled));
        let first = strip(0, None);
        assert_eq!(step(first, ShiftTab, &ctx), (first, Outcome::Handled));
    }

    #[test]
    fn a_focused_canvas_takes_every_key_but_tab_and_escape() {
        let tiles = [vec![Canvas], vec![]];
        let ctx = ctx(&tiles);
        let at = strip(0, Some(0));
        for key in [Up, Down, Left, Right, Enter, Actions, Other] {
            assert_eq!(
                step(at, key, &ctx),
                (at, Outcome::ToCanvas { tile: 0, part: 0 }),
                "{key:?}"
            );
        }
        assert_eq!(step(at, Escape, &ctx), (strip(0, None), Outcome::Handled));
        assert_eq!(step(at, Tab, &ctx), (strip(1, None), Outcome::Handled));
        assert_eq!(step(at, ShiftTab, &ctx), (strip(0, None), Outcome::Handled));
    }

    #[test]
    fn typing_in_the_strip_returns_to_the_list() {
        let tiles = [vec![Button]];
        let ctx = ctx(&tiles);
        for at in [strip(0, None), strip(0, Some(0))] {
            assert_eq!(step(at, Other, &ctx), (List, Outcome::Unhandled));
        }
    }

    #[test]
    fn a_focus_past_the_strip_is_the_list() {
        // The strip shrank since the focus was set.
        let tiles = [vec![]];
        let ctx = ctx(&tiles);
        assert_eq!(step(strip(3, None), Left, &ctx), (List, Outcome::Unhandled));
        assert_eq!(
            step(TileSelected(3), Enter, &ctx),
            (List, Outcome::Unhandled)
        );
    }
}
