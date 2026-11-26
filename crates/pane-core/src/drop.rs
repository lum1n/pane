//! Where a window dropped by a gesture goes, and what the tiles look like
//! afterwards.

use crate::geometry::{Direction, Point, Rect};
use crate::model::{Model, WindowMode};
use crate::reconcile::layout_root;
use crate::tree::{InsertAt, Leaf, NodeId, Tree};
use crate::{DisplayId, WindowId, WorkspaceId};

/// Share of a tile (per axis, centred) that means "swap".
pub const CENTER_FRACTION: f64 = 0.4;
/// Width (points) of the band along a tiling area's edges that means
/// "full-height column / full-width row on that side".
pub const OUTER_BAND: f64 = 48.0;

#[derive(Clone, Debug, PartialEq)]
pub enum DropTarget {
    /// Trade places with another tiled window.
    Swap { other: WindowId },
    /// Split the tile `leaf` and go on side `dir` of it.
    Split { leaf: NodeId, dir: Direction },
    /// Go along a whole side of the display's tiling area.
    DisplayEdge { display: DisplayId, dir: Direction },
    /// Fill a display whose workspace has no tiles.
    Fill { display: DisplayId },
}

/// The window being dropped: a tile already in the tree, or a leaf that is
/// not in it yet (a floating window).
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Leaf(NodeId),
    New(Leaf),
}

/// Tiled windows of a workspace with their laid-out frames.
fn tiles(m: &Model, tree: &Tree, ws: &str) -> Vec<(NodeId, WindowId, Rect)> {
    let Some(w) = m.ws(ws) else { return vec![] };
    let Some(d) = m.display(&w.display) else {
        return vec![];
    };
    layout_root(m, tree, w.root, m.tiling_area(d), None)
        .frames
        .into_iter()
        .filter_map(|(n, r)| tree.leaf(n)?.window().map(|wid| (n, wid, r)))
        .collect()
}

/// The workspace a drop target lands in.
pub fn target_ws(m: &Model, target: &DropTarget) -> Option<WorkspaceId> {
    match target {
        DropTarget::Swap { other } => m.windows.get(other).map(|w| w.workspace.clone()),
        DropTarget::Split { leaf, .. } => {
            let root = m.tree.root_of(*leaf);
            m.workspaces
                .iter()
                .find(|w| w.root == root)
                .map(|w| w.id.clone())
        }
        DropTarget::DisplayEdge { display, .. } | DropTarget::Fill { display } => {
            m.visible.get(display).cloned()
        }
    }
}

fn nearest_side(dl: f64, dr: f64, du: f64, dd: f64) -> (Direction, f64) {
    [
        (Direction::Left, dl),
        (Direction::Right, dr),
        (Direction::Up, du),
        (Direction::Down, dd),
    ]
    .into_iter()
    .fold((Direction::Left, f64::MAX), |best, c| {
        if c.1 < best.1 {
            c
        } else {
            best
        }
    })
}

/// What dropping `wid` at `at` would do, or `None` for nothing.
pub fn target_at(m: &Model, wid: WindowId, at: Point) -> Option<DropTarget> {
    let d = m.displays.iter().find(|d| d.frame.contains(at))?;
    let ws = m.visible.get(&d.id)?;
    let own = m
        .windows
        .get(&wid)
        .filter(|w| w.mode == WindowMode::Tiled)
        .and_then(|_| m.window_node(wid));
    let all = tiles(m, &m.tree, ws);
    let others: Vec<&(NodeId, WindowId, Rect)> = all.iter().filter(|t| t.1 != wid).collect();
    if others.is_empty() {
        let own_here = all.iter().any(|t| t.1 == wid);
        return (!own_here).then(|| DropTarget::Fill {
            display: d.id.clone(),
        });
    }
    let area = m.tiling_area(d);
    let (dir, gap) = nearest_side(
        at.x - area.x,
        area.max_x() - at.x,
        at.y - area.y,
        area.max_y() - at.y,
    );
    if gap < OUTER_BAND {
        return Some(DropTarget::DisplayEdge {
            display: d.id.clone(),
            dir,
        });
    }
    let (node, other, r) = all.iter().find(|t| t.2.contains(at))?;
    if Some(*node) == own || *other == wid {
        return None;
    }
    let fx = (at.x - r.x) / r.w.max(1.0);
    let fy = (at.y - r.y) / r.h.max(1.0);
    let lo = (1.0 - CENTER_FRACTION) / 2.0;
    let hi = 1.0 - lo;
    if (lo..=hi).contains(&fx) && (lo..=hi).contains(&fy) {
        return Some(DropTarget::Swap { other: *other });
    }
    let (dir, _) = nearest_side(fx, 1.0 - fx, fy, 1.0 - fy);
    Some(DropTarget::Split { leaf: *node, dir })
}

/// Put `src` at `target` in the tree whose workspace root is `root`.
/// Returns the leaf the window ends up in. A `New` source dropped on a
/// `Swap` replaces the other window's leaf; the caller re-homes that window.
pub fn apply(tree: &mut Tree, root: NodeId, src: Source, target: &DropTarget) -> NodeId {
    if let DropTarget::Swap { other } = target {
        let Some(on) = tree.find_window(root, *other) else {
            return insert(tree, root, src, InsertAt::End);
        };
        return match src {
            Source::Leaf(n) => {
                tree.swap_leaves(n, on);
                on
            }
            Source::New(leaf) => {
                *tree.leaf_mut(on).unwrap() = leaf;
                on
            }
        };
    }
    let at = match target {
        DropTarget::Split { leaf, dir } => {
            if src == Source::Leaf(*leaf) {
                return *leaf;
            }
            InsertAt::Split {
                target: *leaf,
                dir: *dir,
            }
        }
        DropTarget::DisplayEdge { dir, .. } => InsertAt::Split {
            target: root,
            dir: *dir,
        },
        _ => InsertAt::End,
    };
    insert(tree, root, src, at)
}

fn insert(tree: &mut Tree, root: NodeId, src: Source, at: InsertAt) -> NodeId {
    let leaf = match src {
        Source::Leaf(n) => {
            let l = tree.leaf(n).cloned().expect("source is a leaf");
            tree.remove(n);
            l
        }
        Source::New(l) => l,
    };
    if let InsertAt::Split { target, dir } = &at {
        // A root with one child just turns: wrapping that child would leave
        // a one-child container.
        if *target == root && tree.children(root).len() <= 1 {
            tree.container_mut(root).unwrap().orientation = dir.orientation();
        }
    }
    let root_ok = tree.contains(root);
    let at = match at {
        InsertAt::Split { target, .. } if !tree.contains(target) && root_ok => InsertAt::End,
        a => a,
    };
    tree.insert_leaf(root, leaf, at)
}

/// The frame `wid` gets if dropped at `target`, and the target tile's frame
/// for splits (to outline which tile is split).
pub fn preview(m: &Model, wid: WindowId, target: &DropTarget) -> Option<(Rect, Option<Rect>)> {
    let ws = target_ws(m, target)?;
    let w = m.ws(&ws)?;
    let area = m.tiling_area(m.display(&w.display)?);
    let split_rect = match target {
        DropTarget::Split { leaf, .. } => tiles(m, &m.tree, &ws)
            .into_iter()
            .find(|t| t.0 == *leaf)
            .map(|t| t.2),
        _ => None,
    };
    let mut tree = m.tree.clone();
    let src = match (m.windows.get(&wid)?.mode, m.window_node(wid)) {
        (WindowMode::Tiled, Some(n)) => Source::Leaf(n),
        _ => Source::New(Leaf::Window(wid)),
    };
    let node = apply(&mut tree, w.root, src, target);
    let out = layout_root(m, &tree, w.root, area, Some(wid));
    let rect = out.frames.iter().find(|f| f.0 == node)?.1;
    Some((rect, split_rect))
}

/// "Swap with Safari", "Left of Mail", "Left edge", "Fill Studio Display".
pub fn label(m: &Model, target: &DropTarget) -> String {
    let app_of = |wid: WindowId| {
        m.windows
            .get(&wid)
            .map(|w| {
                m.apps
                    .get(&w.pid)
                    .map(|a| a.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| w.bundle_id.clone())
            })
            .unwrap_or_default()
    };
    match target {
        DropTarget::Swap { other } => format!("Swap with {}", app_of(*other)),
        DropTarget::Split { leaf, dir } => {
            let name = m
                .tree
                .leaf(*leaf)
                .and_then(|l| l.window())
                .map(app_of)
                .unwrap_or_default();
            let side = match dir {
                Direction::Left => "Left of",
                Direction::Right => "Right of",
                Direction::Up => "Above",
                Direction::Down => "Below",
            };
            format!("{side} {name}")
        }
        DropTarget::DisplayEdge { dir, .. } => match dir {
            Direction::Left => "Left edge",
            Direction::Right => "Right edge",
            Direction::Up => "Top edge",
            Direction::Down => "Bottom edge",
        }
        .to_string(),
        DropTarget::Fill { display } => {
            let name = m
                .display(display)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| display.clone());
            format!("Fill {name}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::display::Display;
    use crate::geometry::Orientation;
    use crate::model::WindowInfo;
    use crate::rules::WindowRole;
    use crate::tree::ContainerLayout;

    fn disp(id: &str, x: f64) -> Display {
        Display {
            id: id.into(),
            name: format!("Display {id}"),
            frame: Rect::new(x, 0.0, 1000.0, 1000.0),
            visible_frame: Rect::new(x, 0.0, 1000.0, 1000.0),
            is_main: x == 0.0,
            scale: 1.0,
            position_rank: 0,
        }
    }

    fn win(m: &mut Model, id: u32, ws: &str) {
        m.windows.insert(
            WindowId(id),
            WindowInfo {
                id: WindowId(id),
                pid: 1,
                bundle_id: format!("app{id}"),
                title: String::new(),
                role: WindowRole::Standard,
                resizable: true,
                document: None,
                min_size: None,
                actual: Rect::default(),
                ordinal: 0,
                mode: WindowMode::Tiled,
                workspace: ws.into(),
                minimized: false,
                stubborn: false,
                double_set: false,
                float_frame: None,
                seq: id as u64,
                reason: String::new(),
                slot: None,
            },
        );
    }

    /// Display A (0..1000) shows windows 1 | 2 side by side; display B is empty.
    fn model() -> Model {
        let mut c = Config::default();
        c.settings.inner_gap = 0.0;
        c.settings.outer_gap = 0.0;
        let mut m = Model::new(c, 0);
        m.displays = vec![disp("A", 0.0), disp("B", 1000.0)];
        let a = m.ensure_workspace("A", "1");
        let b = m.ensure_workspace("B", "1");
        m.visible.insert("A".into(), a.clone());
        m.visible.insert("B".into(), b);
        let root = m.ws(&a).unwrap().root;
        for i in [1, 2] {
            win(&mut m, i, &a);
            m.tree
                .insert_leaf(root, Leaf::Window(WindowId(i)), InsertAt::End);
        }
        m
    }

    fn pt(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn centre_swaps_and_edges_split() {
        let m = model();
        let w = WindowId(1);
        let two = m.window_node(WindowId(2)).unwrap();
        assert_eq!(
            target_at(&m, w, pt(750.0, 500.0)),
            Some(DropTarget::Swap { other: WindowId(2) })
        );
        for (p, dir) in [
            (pt(560.0, 500.0), Direction::Left),
            (pt(940.0, 500.0), Direction::Right),
            (pt(750.0, 100.0), Direction::Up),
            (pt(750.0, 900.0), Direction::Down),
        ] {
            assert_eq!(
                target_at(&m, w, p),
                Some(DropTarget::Split { leaf: two, dir }),
                "{p:?}"
            );
        }
    }

    #[test]
    fn outer_band_own_tile_and_empty_display() {
        let m = model();
        let w = WindowId(1);
        assert_eq!(
            target_at(&m, w, pt(990.0, 500.0)),
            Some(DropTarget::DisplayEdge {
                display: "A".into(),
                dir: Direction::Right
            })
        );
        assert_eq!(target_at(&m, w, pt(250.0, 500.0)), None, "own tile");
        assert_eq!(
            target_at(&m, w, pt(1500.0, 500.0)),
            Some(DropTarget::Fill {
                display: "B".into()
            })
        );
        assert_eq!(target_at(&m, w, pt(5000.0, 500.0)), None, "no display");
    }

    #[test]
    fn only_tile_on_its_display_has_nowhere_to_go_there() {
        let mut m = model();
        let n = m.window_node(WindowId(2)).unwrap();
        m.tree.remove(n);
        m.windows.remove(&WindowId(2));
        assert_eq!(target_at(&m, WindowId(1), pt(10.0, 500.0)), None);
    }

    fn rects(m: &Model, tree: &Tree, ws: &str) -> Vec<(WindowId, Rect)> {
        tiles(m, tree, ws).into_iter().map(|t| (t.1, t.2)).collect()
    }

    #[test]
    fn apply_each_target() {
        let m = model();
        let root = m.ws("A/1").unwrap().root;
        let one = m.window_node(WindowId(1)).unwrap();
        let two = m.window_node(WindowId(2)).unwrap();

        let mut t = m.tree.clone();
        apply(
            &mut t,
            root,
            Source::Leaf(one),
            &DropTarget::Swap { other: WindowId(2) },
        );
        assert_eq!(t.windows(root), vec![WindowId(2), WindowId(1)]);

        let mut t = m.tree.clone();
        apply(
            &mut t,
            root,
            Source::Leaf(one),
            &DropTarget::Split {
                leaf: two,
                dir: Direction::Down,
            },
        );
        assert_eq!(
            rects(&m, &t, "A/1"),
            vec![
                (WindowId(2), Rect::new(0.0, 0.0, 1000.0, 500.0)),
                (WindowId(1), Rect::new(0.0, 500.0, 1000.0, 500.0)),
            ]
        );

        let mut t = m.tree.clone();
        apply(
            &mut t,
            root,
            Source::New(Leaf::Window(WindowId(3))),
            &DropTarget::DisplayEdge {
                display: "A".into(),
                dir: Direction::Down,
            },
        );
        // A horizontal root with a bottom drop: the old row is wrapped.
        let c = t.container(root).unwrap();
        assert_eq!(c.orientation, Orientation::Vertical);
        assert_eq!(t.windows(root), vec![WindowId(1), WindowId(2), WindowId(3)]);

        let mut t = m.tree.clone();
        let b = m.ws("B/1").unwrap().root;
        apply(
            &mut t,
            b,
            Source::Leaf(two),
            &DropTarget::Fill {
                display: "B".into(),
            },
        );
        assert_eq!(t.windows(b), vec![WindowId(2)]);
        assert_eq!(t.windows(root), vec![WindowId(1)]);
    }

    #[test]
    fn display_edge_on_a_one_tile_root_turns_it() {
        let mut t = Tree::new();
        let root = t.new_root(Orientation::Horizontal, ContainerLayout::Tiles);
        t.insert_leaf(root, Leaf::Window(WindowId(1)), InsertAt::End);
        apply(
            &mut t,
            root,
            Source::New(Leaf::Window(WindowId(2))),
            &DropTarget::DisplayEdge {
                display: "A".into(),
                dir: Direction::Up,
            },
        );
        let c = t.container(root).unwrap();
        assert_eq!(c.orientation, Orientation::Vertical);
        assert_eq!(t.windows(root), vec![WindowId(2), WindowId(1)]);
        assert!(c.children.iter().all(|ch| t.leaf(*ch).is_some()));
    }

    #[test]
    fn preview_matches_the_layout_after_apply() {
        let m = model();
        let two = m.window_node(WindowId(2)).unwrap();
        let (r, split) = preview(
            &m,
            WindowId(1),
            &DropTarget::Split {
                leaf: two,
                dir: Direction::Up,
            },
        )
        .unwrap();
        assert_eq!(r, Rect::new(0.0, 0.0, 1000.0, 500.0));
        assert_eq!(split, Some(Rect::new(500.0, 0.0, 500.0, 1000.0)));
        assert_eq!(
            label(
                &m,
                &DropTarget::Split {
                    leaf: two,
                    dir: Direction::Up
                }
            ),
            "Above app2"
        );
    }
}
