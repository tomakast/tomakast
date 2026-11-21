//! Where the strip's tiles sit: pure arithmetic over tile sizes and the panel
//! width, so it is unit-tested apart from the view.
//!
//! The content is the panel less [`EDGE_PADDING`] on each side, `W`. A quarter
//! tile is `(W - 3 * STRIP_GAP) / 4` wide, a single one `(W - STRIP_GAP) / 2`
//! and a double one `W`: four quarters, two singles or one double fill the
//! content width, and two quarters with the gap between them are one single,
//! so tiles of every size line up. The scroll viewport is the whole panel:
//! the tile after them starts in the right margin and peeks in.

use tomakast_core::WidgetSize;

use crate::theme::{EDGE_PADDING, STRIP_GAP};

/// One tile's place in the strip's scrolled content, in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TileSlot {
    /// From the content's left edge.
    pub x: f32,
    pub width: f32,
}

/// The width of a tile of `size` in a panel `panel_width` wide.
pub(crate) fn tile_width(size: WidgetSize, panel_width: f32) -> f32 {
    let content = panel_width - 2. * EDGE_PADDING;
    match size {
        WidgetSize::Quarter => (content - 3. * STRIP_GAP) / 4.,
        WidgetSize::Single => (content - STRIP_GAP) / 2.,
        WidgetSize::Double => content,
    }
}

/// The slots of tiles of `sizes`, in order, for a panel `panel_width` wide:
/// the first at [`EDGE_PADDING`], each next [`STRIP_GAP`] after the last.
pub(crate) fn slots(sizes: &[WidgetSize], panel_width: f32) -> Vec<TileSlot> {
    let mut x = EDGE_PADDING;
    sizes
        .iter()
        .map(|&size| {
            let slot = TileSlot {
                x,
                width: tile_width(size, panel_width),
            };
            x += slot.width + STRIP_GAP;
            slot
        })
        .collect()
}

/// The width of the scrolled content: the slots and [`EDGE_PADDING`] after
/// the last one; 0 for no slots.
pub(crate) fn content_width(slots: &[TileSlot]) -> f32 {
    slots
        .last()
        .map_or(0., |last| last.x + last.width + EDGE_PADDING)
}

/// The indices of the slots that show, even in part, in a viewport
/// `viewport` wide scrolled `offset` (≥ 0) to the right: the tiles on screen
/// and the peeking ones.
pub(crate) fn visible(slots: &[TileSlot], offset: f32, viewport: f32) -> Vec<usize> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.x < offset + viewport && slot.x + slot.width > offset)
        .map(|(ix, _)| ix)
        .collect()
}

/// The scroll offset that puts slot `ix` at the content edge
/// ([`EDGE_PADDING`] from the viewport's left), clamped to the scroll range.
pub(crate) fn offset_for(slots: &[TileSlot], ix: usize, viewport: f32) -> f32 {
    let Some(slot) = slots.get(ix) else {
        return 0.;
    };
    let max = (content_width(slots) - viewport).max(0.);
    (slot.x - EDGE_PADDING).clamp(0., max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use WidgetSize::{Double, Quarter, Single};

    fn three() -> Vec<TileSlot> {
        slots(&[Single, Single, Double], 750.)
    }

    #[test]
    fn slots_fill_the_content_width() {
        let slots = three();
        assert_eq!(
            slots,
            [
                TileSlot {
                    x: 16.,
                    width: 355.
                },
                TileSlot {
                    x: 379.,
                    width: 355.
                },
                TileSlot {
                    x: 742.,
                    width: 718.
                },
            ]
        );
        assert_eq!(content_width(&slots), 1476.);
        assert_eq!(tile_width(Single, 750.), 355.);
        assert_eq!(tile_width(Double, 750.), 718.);
        assert_eq!(content_width(&[]), 0.);
    }

    #[test]
    fn the_next_tile_starts_in_the_right_margin() {
        let x = three()[2].x;
        assert_eq!(x, 742.);
        // Past the content's right edge (750 - 16), inside the panel.
        assert!(734. < x && x < 750., "{x}");
    }

    #[test]
    fn visible_includes_the_peeking_tile() {
        let slots = three();
        assert_eq!(visible(&slots, 0., 750.), [0, 1, 2]);
        assert_eq!(visible(&slots, 800., 750.), [2]);
        assert_eq!(visible(&[], 0., 750.), Vec::<usize>::new());
    }

    #[test]
    fn offset_for_aligns_the_tile_with_the_content_edge() {
        let slots = three();
        // 742 - 16, which is also the end of the range, 1476 - 750.
        assert_eq!(offset_for(&slots, 2, 750.), 726.);
        assert_eq!(offset_for(&slots, 0, 750.), 0.);
        assert_eq!(offset_for(&slots, 1, 750.), 363.);
        // A strip narrower than the viewport does not scroll.
        assert_eq!(offset_for(&super::slots(&[Single], 750.), 0, 750.), 0.);
        assert_eq!(offset_for(&slots, 9, 750.), 0.);
    }

    #[test]
    fn offset_for_clamps_to_the_scroll_range() {
        let slots = slots(&[Single, Single, Single], 750.);
        // Slot 2 at 742: 726 would scroll past 1113 - 750 = 363.
        assert_eq!(content_width(&slots), 1113.);
        assert_eq!(offset_for(&slots, 2, 750.), 363.);
    }

    #[test]
    fn four_quarters_fill_the_content_width_and_two_make_a_single() {
        assert_eq!(tile_width(Quarter, 750.), 173.5);
        let quarters = slots(&[Quarter, Quarter, Quarter, Quarter], 750.);
        let last = quarters[3];
        // The content edge: 750 - 16.
        assert_eq!(last.x + last.width, 734.);
        let (a, b) = (quarters[0], quarters[1]);
        assert_eq!(b.x + b.width - a.x, tile_width(Single, 750.));
    }

    #[test]
    fn mixed_sizes_line_up_with_singles() {
        // Q Q S D Q Q: every tile starts where a single grid would put one.
        let slots = slots(&[Quarter, Quarter, Single, Double, Quarter, Quarter], 750.);
        let xs: Vec<f32> = slots.iter().map(|s| s.x).collect();
        assert_eq!(xs, [16., 197.5, 379., 742., 1468., 1649.5]);
        assert_eq!(content_width(&slots), 1839.);
        // Scrolled to the double: the single before it peeks in at the left,
        // the first quarter after it at the right.
        let offset = offset_for(&slots, 3, 750.);
        assert_eq!(offset, 726.);
        assert_eq!(visible(&slots, offset, 750.), [2, 3, 4]);
        // The last quarter scrolls only as far as the strip's end.
        assert_eq!(offset_for(&slots, 5, 750.), 1839. - 750.);
    }
}
