//! Turn a tree into window frames.

use crate::geometry::{Orientation, Rect, Size};
use crate::tree::{ContainerLayout, Leaf, NodeId, NodeKind, Tree};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutOutput {
    /// Frame for every present leaf, in tree order.
    pub frames: Vec<(NodeId, Rect)>,
    /// Leaves in containers whose minimum sizes do not fit. The reducer
    /// floats the newest of them rather than fighting the app.
    pub overflow: Vec<NodeId>,
}

pub struct LayoutInput<'a> {
    /// Space between tiles.
    pub inner_gap: f64,
    /// Space between the tiles and the edge of the area.
    pub outer_gap: f64,
    /// Drop both gaps when the tree shows a single tile.
    pub smart_gaps: bool,
    /// Leaves for which this returns false take no space (windows of apps
    /// that quit and may come back, slots that are no longer pending).
    pub present: &'a dyn Fn(&Leaf) -> bool,
    /// Known minimum size of a leaf's window.
    pub min_size: &'a dyn Fn(&Leaf) -> Size,
}

/// Lay out the tree at `root` in `area` (a display's visible frame): the
/// outer gap is taken off its edges, the inner gap between tiles. With
/// `smart_gaps`, a tree that shows a single tile fills `area`.
pub fn compute(tree: &Tree, root: NodeId, area: Rect, input: &LayoutInput) -> LayoutOutput {
    let single = input.smart_gaps && tile_count(tree, root, input) <= 1;
    let (inner, outer) = if single {
        (0.0, 0.0)
    } else {
        (input.inner_gap, input.outer_gap)
    };
    let input = LayoutInput {
        inner_gap: inner,
        outer_gap: outer,
        smart_gaps: input.smart_gaps,
        present: input.present,
        min_size: input.min_size,
    };
    let mut out = LayoutOutput::default();
    place(tree, root, area.inset(outer), &input, &mut out);
    out
}

/// How many separate tiles the tree at `id` shows: the windows of an
/// accordion are stacked in one tile.
fn tile_count(tree: &Tree, id: NodeId, input: &LayoutInput) -> usize {
    match &tree.node(id).map(|n| &n.kind) {
        Some(NodeKind::Leaf(l)) => usize::from((input.present)(l)),
        Some(NodeKind::Container(c)) => {
            let kids = c.children.iter().map(|ch| tile_count(tree, *ch, input));
            match c.layout {
                ContainerLayout::Accordion => kids.max().unwrap_or(0),
                ContainerLayout::Tiles => kids.sum(),
            }
        }
        None => 0,
    }
}

fn is_present(tree: &Tree, id: NodeId, input: &LayoutInput) -> bool {
    match &tree.node(id).map(|n| &n.kind) {
        Some(NodeKind::Leaf(l)) => (input.present)(l),
        Some(NodeKind::Container(c)) => c.children.iter().any(|ch| is_present(tree, *ch, input)),
        None => false,
    }
}

fn min_of(tree: &Tree, id: NodeId, input: &LayoutInput) -> Size {
    match &tree.node(id).unwrap().kind {
        NodeKind::Leaf(l) => (input.min_size)(l),
        NodeKind::Container(c) => {
            let kids: Vec<Size> = c
                .children
                .iter()
                .filter(|ch| is_present(tree, **ch, input))
                .map(|ch| min_of(tree, *ch, input))
                .collect();
            if kids.is_empty() {
                return Size::ZERO;
            }
            let gaps = input.inner_gap * (kids.len() as f64 - 1.0);
            match (c.layout, c.orientation) {
                (ContainerLayout::Accordion, _) => kids.iter().fold(Size::ZERO, |a, b| a.max(*b)),
                (ContainerLayout::Tiles, Orientation::Horizontal) => Size::new(
                    kids.iter().map(|k| k.w).sum::<f64>() + gaps,
                    kids.iter().map(|k| k.h).fold(0.0, f64::max),
                ),
                (ContainerLayout::Tiles, Orientation::Vertical) => Size::new(
                    kids.iter().map(|k| k.w).fold(0.0, f64::max),
                    kids.iter().map(|k| k.h).sum::<f64>() + gaps,
                ),
            }
        }
    }
}

fn place(tree: &Tree, id: NodeId, rect: Rect, input: &LayoutInput, out: &mut LayoutOutput) {
    match &tree.node(id).unwrap().kind {
        NodeKind::Leaf(l) => {
            if (input.present)(l) {
                out.frames.push((id, rect));
            }
        }
        NodeKind::Container(c) => {
            let kids: Vec<(NodeId, f64)> = c
                .children
                .iter()
                .zip(&c.weights)
                .filter(|(ch, _)| is_present(tree, **ch, input))
                .map(|(ch, w)| (*ch, *w))
                .collect();
            if kids.is_empty() {
                return;
            }
            match c.layout {
                ContainerLayout::Accordion => {
                    for (ch, _) in kids {
                        place(tree, ch, rect, input, out);
                    }
                }
                ContainerLayout::Tiles => {
                    let axis = c.orientation;
                    let (start, len) = rect.span(axis);
                    let avail = (len - input.inner_gap * (kids.len() as f64 - 1.0)).max(0.0);
                    let weights: Vec<f64> = kids.iter().map(|k| k.1).collect();
                    let mins: Vec<f64> = kids
                        .iter()
                        .map(|k| min_of(tree, k.0, input).along(axis))
                        .collect();
                    let (sizes, overflow) = distribute(avail, &weights, &mins);
                    if overflow {
                        for (ch, _) in &kids {
                            for leaf in tree.leaves(*ch) {
                                if tree.leaf(leaf).is_some_and(|l| (input.present)(l)) {
                                    out.overflow.push(leaf);
                                }
                            }
                        }
                    }
                    let mut pos = start;
                    for (i, (ch, _)) in kids.iter().enumerate() {
                        let a = pos.round();
                        let b = (pos + sizes[i]).round();
                        place(
                            tree,
                            *ch,
                            rect.with_span(axis, a, (b - a).max(1.0)),
                            input,
                            out,
                        );
                        pos += sizes[i] + input.inner_gap;
                    }
                }
            }
        }
    }
}

/// Split `avail` by `weights`, raising shares below their minimum to it and
/// taking the difference from the others. Returns the sizes and whether the
/// minimums could not all be met.
pub fn distribute(avail: f64, weights: &[f64], mins: &[f64]) -> (Vec<f64>, bool) {
    let n = weights.len();
    let min_total: f64 = mins.iter().sum();
    let wsum_all: f64 = weights.iter().sum::<f64>().max(1e-9);
    if min_total > avail + 0.5 {
        return (weights.iter().map(|w| avail * w / wsum_all).collect(), true);
    }
    let mut fixed: Vec<Option<f64>> = vec![None; n];
    loop {
        let free = avail - fixed.iter().flatten().sum::<f64>();
        let wsum: f64 = (0..n)
            .filter(|i| fixed[*i].is_none())
            .map(|i| weights[i])
            .sum::<f64>()
            .max(1e-9);
        let mut changed = false;
        for i in 0..n {
            if fixed[i].is_none() && free * weights[i] / wsum < mins[i] {
                fixed[i] = Some(mins[i]);
                changed = true;
            }
        }
        if !changed {
            let sizes = (0..n)
                .map(|i| fixed[i].unwrap_or(free * weights[i] / wsum))
                .collect();
            return (sizes, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Direction;
    use crate::tree::InsertAt;
    use crate::WindowId;

    fn input<'a>(min: &'a dyn Fn(&Leaf) -> Size) -> LayoutInput<'a> {
        LayoutInput {
            inner_gap: 10.0,
            outer_gap: 0.0,
            smart_gaps: false,
            present: &|_| true,
            min_size: min,
        }
    }

    #[test]
    fn two_columns_with_gap() {
        let mut t = Tree::new();
        let root = t.new_root(Orientation::Horizontal, ContainerLayout::Tiles);
        t.insert_leaf(root, Leaf::Window(WindowId(1)), InsertAt::End);
        let b = t.insert_leaf(root, Leaf::Window(WindowId(2)), InsertAt::End);
        t.insert_leaf(
            root,
            Leaf::Window(WindowId(3)),
            InsertAt::Split {
                target: b,
                dir: Direction::Down,
            },
        );
        let min = |_: &Leaf| Size::ZERO;
        let out = compute(&t, root, Rect::new(0.0, 0.0, 1010.0, 610.0), &input(&min));
        let rects: Vec<Rect> = out.frames.iter().map(|f| f.1).collect();
        assert_eq!(rects[0], Rect::new(0.0, 0.0, 500.0, 610.0));
        assert_eq!(rects[1], Rect::new(510.0, 0.0, 500.0, 300.0));
        assert_eq!(rects[2], Rect::new(510.0, 310.0, 500.0, 300.0));
        assert!(out.overflow.is_empty());
    }

    #[test]
    fn min_sizes_take_space_from_neighbours() {
        let (sizes, overflow) = distribute(1000.0, &[1.0, 1.0, 1.0], &[600.0, 0.0, 0.0]);
        assert!(!overflow);
        assert_eq!(sizes, vec![600.0, 200.0, 200.0]);
        let (_, overflow) = distribute(1000.0, &[1.0, 1.0], &[600.0, 600.0]);
        assert!(overflow);
    }

    #[test]
    fn absent_leaves_take_no_space() {
        let mut t = Tree::new();
        let root = t.new_root(Orientation::Horizontal, ContainerLayout::Tiles);
        t.insert_leaf(root, Leaf::Window(WindowId(1)), InsertAt::End);
        t.insert_leaf(root, Leaf::Window(WindowId(2)), InsertAt::End);
        let present = |l: &Leaf| l.window() != Some(WindowId(2));
        let min = |_: &Leaf| Size::ZERO;
        let inp = LayoutInput {
            inner_gap: 0.0,
            outer_gap: 0.0,
            smart_gaps: false,
            present: &present,
            min_size: &min,
        };
        let out = compute(&t, root, Rect::new(0.0, 0.0, 100.0, 100.0), &inp);
        assert_eq!(out.frames.len(), 1);
        assert_eq!(out.frames[0].1, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn accordion_stacks() {
        let mut t = Tree::new();
        let root = t.new_root(Orientation::Horizontal, ContainerLayout::Accordion);
        t.insert_leaf(root, Leaf::Window(WindowId(1)), InsertAt::End);
        t.insert_leaf(root, Leaf::Window(WindowId(2)), InsertAt::End);
        let min = |_: &Leaf| Size::ZERO;
        let out = compute(&t, root, Rect::new(0.0, 0.0, 100.0, 100.0), &input(&min));
        assert_eq!(out.frames[0].1, out.frames[1].1);
    }

    fn gapped<'a>(min: &'a dyn Fn(&Leaf) -> Size, smart: bool) -> LayoutInput<'a> {
        LayoutInput {
            inner_gap: 10.0,
            outer_gap: 5.0,
            smart_gaps: smart,
            present: &|_| true,
            min_size: min,
        }
    }

    fn row(layout: ContainerLayout, n: u32) -> (Tree, NodeId) {
        let mut t = Tree::new();
        let root = t.new_root(Orientation::Horizontal, layout);
        for i in 1..=n {
            t.insert_leaf(root, Leaf::Window(WindowId(i)), InsertAt::End);
        }
        (t, root)
    }

    #[test]
    fn outer_gap_insets_the_area() {
        let (t, root) = row(ContainerLayout::Tiles, 1);
        let min = |_: &Leaf| Size::ZERO;
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        let out = compute(&t, root, area, &gapped(&min, false));
        assert_eq!(out.frames[0].1, Rect::new(5.0, 5.0, 90.0, 90.0));
    }

    #[test]
    fn smart_gaps_drop_gaps_for_a_single_tile() {
        let min = |_: &Leaf| Size::ZERO;
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        let (t, root) = row(ContainerLayout::Tiles, 1);
        let out = compute(&t, root, area, &gapped(&min, true));
        assert_eq!(out.frames[0].1, area);

        // An accordion stacks its windows in one tile.
        let (t, root) = row(ContainerLayout::Accordion, 3);
        let out = compute(&t, root, area, &gapped(&min, true));
        assert!(out.frames.iter().all(|f| f.1 == area));

        // Two tiles keep both gaps.
        let (t, root) = row(ContainerLayout::Tiles, 2);
        let out = compute(
            &t,
            root,
            Rect::new(0.0, 0.0, 120.0, 100.0),
            &gapped(&min, true),
        );
        assert_eq!(out.frames[0].1, Rect::new(5.0, 5.0, 50.0, 90.0));
        assert_eq!(out.frames[1].1, Rect::new(65.0, 5.0, 50.0, 90.0));
    }

    #[test]
    fn smart_gaps_count_only_present_tiles() {
        let (t, root) = row(ContainerLayout::Tiles, 2);
        let present = |l: &Leaf| l.window() != Some(WindowId(2));
        let min = |_: &Leaf| Size::ZERO;
        let inp = LayoutInput {
            present: &present,
            ..gapped(&min, true)
        };
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        let out = compute(&t, root, area, &inp);
        assert_eq!(out.frames, vec![(out.frames[0].0, area)]);
    }
}
