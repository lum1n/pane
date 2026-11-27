//! The split tree: containers with an orientation and weighted children,
//! windows (or profile app slots) as leaves.
//!
//! All workspaces share one [`Tree`] arena; each workspace owns a root
//! container. Containers other than a root never have fewer than two
//! children: [`Tree::remove`] collapses them.

use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SlotMap};

use crate::geometry::{Direction, Orientation};
use crate::WindowId;

new_key_type! {
    pub struct NodeId;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerLayout {
    #[default]
    Tiles,
    Accordion,
}

impl ContainerLayout {
    pub fn toggle(self) -> Self {
        match self {
            ContainerLayout::Tiles => ContainerLayout::Accordion,
            ContainerLayout::Accordion => ContainerLayout::Tiles,
        }
    }
}

/// A placeholder for an application window, used by profiles.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppSlot {
    pub bundle_id: String,
    /// Regex the window title must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch_args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Leaf {
    Window(WindowId),
    Slot(AppSlot),
}

impl Leaf {
    pub fn window(&self) -> Option<WindowId> {
        match self {
            Leaf::Window(w) => Some(*w),
            Leaf::Slot(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub orientation: Orientation,
    pub layout: ContainerLayout,
    pub children: Vec<NodeId>,
    pub weights: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Container(Container),
    Leaf(Leaf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub kind: NodeKind,
}

/// Where [`Tree::insert_leaf`] puts a new leaf.
#[derive(Clone, Debug, PartialEq)]
pub enum InsertAt {
    /// Last child of the root.
    End,
    /// Right after `NodeId` inside its parent.
    After(NodeId),
    /// Next to `target` on side `dir`, wrapping `target` in a new container
    /// when its parent has the other orientation (i3 drop zones).
    Split { target: NodeId, dir: Direction },
    /// Index path from the root; the parent path must name a container.
    Path(Vec<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveResult {
    /// The leaf moved; its new node id.
    Moved(NodeId),
    /// Nothing to move past in that direction.
    AtEdge,
}

#[derive(Clone, Debug, Default)]
pub struct Tree {
    nodes: SlotMap<NodeId, Node>,
}

const MIN_SHARE: f64 = 0.05;

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_root(&mut self, orientation: Orientation, layout: ContainerLayout) -> NodeId {
        self.nodes.insert(Node {
            parent: None,
            kind: NodeKind::Container(Container {
                orientation,
                layout,
                children: vec![],
                weights: vec![],
            }),
        })
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id).and_then(|n| n.parent)
    }

    pub fn leaf(&self, id: NodeId) -> Option<&Leaf> {
        match &self.nodes.get(id)?.kind {
            NodeKind::Leaf(l) => Some(l),
            NodeKind::Container(_) => None,
        }
    }

    pub fn leaf_mut(&mut self, id: NodeId) -> Option<&mut Leaf> {
        match &mut self.nodes.get_mut(id)?.kind {
            NodeKind::Leaf(l) => Some(l),
            NodeKind::Container(_) => None,
        }
    }

    pub fn container(&self, id: NodeId) -> Option<&Container> {
        match &self.nodes.get(id)?.kind {
            NodeKind::Container(c) => Some(c),
            NodeKind::Leaf(_) => None,
        }
    }

    pub fn container_mut(&mut self, id: NodeId) -> Option<&mut Container> {
        match &mut self.nodes.get_mut(id)?.kind {
            NodeKind::Container(c) => Some(c),
            NodeKind::Leaf(_) => None,
        }
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.container(id)
            .map(|c| c.children.as_slice())
            .unwrap_or(&[])
    }

    pub fn root_of(&self, mut id: NodeId) -> NodeId {
        while let Some(p) = self.parent(id) {
            id = p;
        }
        id
    }

    pub fn index_in_parent(&self, id: NodeId) -> Option<usize> {
        let p = self.parent(id)?;
        self.children(p).iter().position(|c| *c == id)
    }

    pub fn weight_of(&self, id: NodeId) -> f64 {
        match (self.parent(id), self.index_in_parent(id)) {
            (Some(p), Some(i)) => self.container(p).map(|c| c.weights[i]).unwrap_or(1.0),
            _ => 1.0,
        }
    }

    pub fn set_weight(&mut self, id: NodeId, w: f64) {
        if let (Some(p), Some(i)) = (self.parent(id), self.index_in_parent(id)) {
            if let Some(c) = self.container_mut(p) {
                c.weights[i] = w.max(0.01);
            }
        }
    }

    /// Leaves under `root` in depth-first (visual) order.
    pub fn leaves(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_leaves(root, &mut out);
        out
    }

    fn collect_leaves(&self, id: NodeId, out: &mut Vec<NodeId>) {
        match self.nodes.get(id).map(|n| &n.kind) {
            Some(NodeKind::Leaf(_)) => out.push(id),
            Some(NodeKind::Container(c)) => {
                for ch in &c.children {
                    self.collect_leaves(*ch, out);
                }
            }
            None => {}
        }
    }

    pub fn windows(&self, root: NodeId) -> Vec<WindowId> {
        self.leaves(root)
            .into_iter()
            .filter_map(|l| self.leaf(l)?.window())
            .collect()
    }

    pub fn find_window(&self, root: NodeId, wid: WindowId) -> Option<NodeId> {
        self.find_leaf(root, |l| l.window() == Some(wid))
    }

    pub fn find_leaf(&self, root: NodeId, pred: impl Fn(&Leaf) -> bool) -> Option<NodeId> {
        self.leaves(root)
            .into_iter()
            .find(|id| self.leaf(*id).is_some_and(&pred))
    }

    pub fn path_of(&self, mut id: NodeId) -> Vec<usize> {
        let mut path = Vec::new();
        while let Some(i) = self.index_in_parent(id) {
            path.push(i);
            id = self.parent(id).expect("index implies parent");
        }
        path.reverse();
        path
    }

    pub fn node_at_path(&self, root: NodeId, path: &[usize]) -> Option<NodeId> {
        let mut cur = root;
        for i in path {
            cur = *self.children(cur).get(*i)?;
        }
        Some(cur)
    }

    fn mean_weight(c: &Container) -> f64 {
        if c.weights.is_empty() {
            1.0
        } else {
            c.weights.iter().sum::<f64>() / c.weights.len() as f64
        }
    }

    /// Low-level insert of a new node at `index` (clamped) of `container`.
    pub fn insert_child(
        &mut self,
        container: NodeId,
        index: usize,
        kind: NodeKind,
        weight: Option<f64>,
    ) -> NodeId {
        let id = self.nodes.insert(Node {
            parent: Some(container),
            kind,
        });
        let c = self
            .container_mut(container)
            .expect("insert_child into a container");
        let w = weight.unwrap_or_else(|| Self::mean_weight(c));
        let index = index.min(c.children.len());
        c.children.insert(index, id);
        c.weights.insert(index, w);
        id
    }

    pub fn insert_leaf(&mut self, root: NodeId, leaf: Leaf, at: InsertAt) -> NodeId {
        match at {
            InsertAt::End => {
                let n = self.children(root).len();
                self.insert_child(root, n, NodeKind::Leaf(leaf), None)
            }
            InsertAt::After(target) => match (self.parent(target), self.index_in_parent(target)) {
                (Some(p), Some(i)) => self.insert_child(p, i + 1, NodeKind::Leaf(leaf), None),
                _ if self.container(target).is_some() => {
                    let n = self.children(target).len();
                    self.insert_child(target, n, NodeKind::Leaf(leaf), None)
                }
                _ => self.insert_leaf(root, leaf, InsertAt::End),
            },
            InsertAt::Split { target, dir } => self.insert_split(root, target, leaf, dir),
            InsertAt::Path(path) => {
                let Some((last, parent_path)) = path.split_last() else {
                    return self.insert_leaf(root, leaf, InsertAt::End);
                };
                match self.node_at_path(root, parent_path) {
                    Some(p) if self.container(p).is_some() => {
                        self.insert_child(p, *last, NodeKind::Leaf(leaf), None)
                    }
                    _ => self.insert_leaf(root, leaf, InsertAt::End),
                }
            }
        }
    }

    fn insert_split(&mut self, root: NodeId, target: NodeId, leaf: Leaf, dir: Direction) -> NodeId {
        let axis = dir.orientation();
        let fwd = dir.is_forward();
        // Dropping on an empty root, or on a container itself.
        if let Some(c) = self.container(target) {
            if c.children.is_empty() || c.orientation == axis {
                if c.children.is_empty() {
                    self.container_mut(target).unwrap().orientation = axis;
                }
                let idx = if fwd { self.children(target).len() } else { 0 };
                return self.insert_child(target, idx, NodeKind::Leaf(leaf), None);
            }
            if self.parent(target).is_none() {
                self.wrap_root_children(target, axis);
                let idx = if fwd { 1 } else { 0 };
                return self.insert_child(target, idx, NodeKind::Leaf(leaf), Some(1.0));
            }
        }
        let Some(parent) = self.parent(target) else {
            return self.insert_leaf(root, leaf, InsertAt::End);
        };
        let idx = self.index_in_parent(target).unwrap();
        let pc = self.container(parent).unwrap();
        if pc.orientation == axis || pc.children.len() == 1 {
            if pc.children.len() == 1 {
                self.container_mut(parent).unwrap().orientation = axis;
            }
            let w = self.weight_of(target);
            return self.insert_child(
                parent,
                if fwd { idx + 1 } else { idx },
                NodeKind::Leaf(leaf),
                Some(w),
            );
        }
        // Wrap target in a new container with the drop axis.
        let w = self.weight_of(target);
        let wrapper = self.nodes.insert(Node {
            parent: Some(parent),
            kind: NodeKind::Container(Container {
                orientation: axis,
                layout: ContainerLayout::Tiles,
                children: vec![target],
                weights: vec![1.0],
            }),
        });
        self.container_mut(parent).unwrap().children[idx] = wrapper;
        self.container_mut(parent).unwrap().weights[idx] = w;
        self.nodes[target].parent = Some(wrapper);
        self.insert_child(
            wrapper,
            if fwd { 1 } else { 0 },
            NodeKind::Leaf(leaf),
            Some(1.0),
        )
    }

    /// Move all children of `root` into one new child container and give the
    /// root the orientation `axis`.
    fn wrap_root_children(&mut self, root: NodeId, axis: Orientation) {
        let c = self.container(root).unwrap().clone();
        let wrapper = self.nodes.insert(Node {
            parent: Some(root),
            kind: NodeKind::Container(c.clone()),
        });
        for ch in &c.children {
            self.nodes[*ch].parent = Some(wrapper);
        }
        let rc = self.container_mut(root).unwrap();
        rc.orientation = axis;
        rc.layout = ContainerLayout::Tiles;
        rc.children = vec![wrapper];
        rc.weights = vec![1.0];
    }

    /// Delete `id` and its subtree, then collapse containers left with
    /// fewer than two children. Returns the removed leaves.
    pub fn remove(&mut self, id: NodeId) -> Vec<Leaf> {
        let parent = self.parent(id);
        if let (Some(p), Some(i)) = (parent, self.index_in_parent(id)) {
            let c = self.container_mut(p).unwrap();
            c.children.remove(i);
            c.weights.remove(i);
        }
        let mut removed = Vec::new();
        self.delete_subtree(id, &mut removed);
        if let Some(p) = parent {
            self.normalize(p);
        }
        removed
    }

    fn delete_subtree(&mut self, id: NodeId, removed: &mut Vec<Leaf>) {
        if let Some(node) = self.nodes.remove(id) {
            match node.kind {
                NodeKind::Leaf(l) => removed.push(l),
                NodeKind::Container(c) => {
                    for ch in c.children {
                        self.delete_subtree(ch, removed);
                    }
                }
            }
        }
    }

    /// Remove every child of `root`, keeping the root.
    pub fn clear(&mut self, root: NodeId) -> Vec<Leaf> {
        let mut removed = Vec::new();
        let children = std::mem::take(&mut self.container_mut(root).unwrap().children);
        self.container_mut(root).unwrap().weights.clear();
        for ch in children {
            self.delete_subtree(ch, &mut removed);
        }
        removed
    }

    fn normalize(&mut self, id: NodeId) {
        let Some(c) = self.container(id) else { return };
        let parent = self.nodes[id].parent;
        match (parent, c.children.len()) {
            (Some(gp), 0) => {
                let i = self.index_in_parent(id).unwrap();
                let gc = self.container_mut(gp).unwrap();
                gc.children.remove(i);
                gc.weights.remove(i);
                self.nodes.remove(id);
                self.normalize(gp);
            }
            (Some(gp), 1) => {
                let child = c.children[0];
                let i = self.index_in_parent(id).unwrap();
                self.container_mut(gp).unwrap().children[i] = child;
                self.nodes[child].parent = Some(gp);
                self.nodes.remove(id);
                self.flatten_if_same_orientation(child);
            }
            (None, 1) => {
                let child = c.children[0];
                if let Some(cc) = self.container(child).cloned() {
                    for ch in &cc.children {
                        self.nodes[*ch].parent = Some(id);
                    }
                    *self.container_mut(id).unwrap() = cc;
                    self.nodes.remove(child);
                }
            }
            _ => {}
        }
    }

    /// When container `id` has the same orientation as its parent, splice its
    /// children into the parent, keeping their relative sizes.
    fn flatten_if_same_orientation(&mut self, id: NodeId) {
        let (Some(parent), Some(c)) = (self.parent(id), self.container(id).cloned()) else {
            return;
        };
        let pc = self.container(parent).unwrap();
        if pc.orientation != c.orientation || c.layout != pc.layout {
            return;
        }
        let i = self.index_in_parent(id).unwrap();
        let w = pc.weights[i];
        let total: f64 = c.weights.iter().sum::<f64>().max(1e-9);
        let new_weights: Vec<f64> = c.weights.iter().map(|x| x / total * w).collect();
        let pc = self.container_mut(parent).unwrap();
        pc.children.splice(i..=i, c.children.iter().copied());
        pc.weights.splice(i..=i, new_weights);
        for ch in &c.children {
            self.nodes[*ch].parent = Some(parent);
        }
        self.nodes.remove(id);
    }

    /// i3-style move of a leaf one step in `dir`.
    pub fn move_leaf(&mut self, leaf: NodeId, dir: Direction) -> MoveResult {
        let axis = dir.orientation();
        let fwd = dir.is_forward();
        let Some(payload) = self.leaf(leaf).cloned() else {
            return MoveResult::AtEdge;
        };
        let root = self.root_of(leaf);
        let mut child = leaf;
        let mut cur = match self.parent(leaf) {
            Some(p) => p,
            None => return MoveResult::AtEdge,
        };
        loop {
            let c = self.container(cur).unwrap();
            if c.orientation == axis {
                let idx = self.index_in_parent(child).unwrap();
                if child != leaf {
                    // Pop out of the nested container next to it (i3).
                    let at = if fwd { idx + 1 } else { idx };
                    let new = self.insert_child(cur, at, NodeKind::Leaf(payload), None);
                    self.remove(leaf);
                    return MoveResult::Moved(new);
                }
                let nidx = if fwd {
                    idx.checked_add(1)
                } else {
                    idx.checked_sub(1)
                };
                if let Some(nidx) = nidx.filter(|n| *n < c.children.len()) {
                    let sib = c.children[nidx];
                    if self.leaf(sib).is_some() {
                        let c = self.container_mut(cur).unwrap();
                        c.children.swap(idx, nidx);
                        c.weights.swap(idx, nidx);
                        return MoveResult::Moved(leaf);
                    }
                    // Enter the sibling container from its near side.
                    let sc = self.container(sib).unwrap();
                    let at = if sc.orientation == axis && fwd {
                        0
                    } else {
                        sc.children.len()
                    };
                    let new = self.insert_child(sib, at, NodeKind::Leaf(payload), None);
                    self.remove(leaf);
                    return MoveResult::Moved(new);
                }
            }
            if cur == root {
                break;
            }
            child = cur;
            cur = self.parent(cur).unwrap();
        }
        // No ancestor can take the leaf further in that direction.
        let rc = self.container(root).unwrap();
        if rc.orientation == axis || (self.parent(leaf) == Some(root) && rc.children.len() == 1) {
            return MoveResult::AtEdge;
        }
        self.remove(leaf);
        let rc = self.container(root).unwrap();
        if rc.orientation != axis {
            if rc.children.len() <= 1 {
                self.container_mut(root).unwrap().orientation = axis;
            } else {
                self.wrap_root_children(root, axis);
            }
        }
        let n = self.children(root).len();
        let w = if n == 1 { Some(1.0) } else { None };
        MoveResult::Moved(self.insert_child(
            root,
            if fwd { n } else { 0 },
            NodeKind::Leaf(payload),
            w,
        ))
    }

    /// Swap the contents of two leaves; sizes stay with positions.
    pub fn swap_leaves(&mut self, a: NodeId, b: NodeId) {
        if a == b {
            return;
        }
        let (Some(la), Some(lb)) = (self.leaf(a).cloned(), self.leaf(b).cloned()) else {
            return;
        };
        *self.leaf_mut(a).unwrap() = lb;
        *self.leaf_mut(b).unwrap() = la;
    }

    /// The container whose orientation/layout a command on `id` changes.
    pub fn owning_container(&self, id: NodeId) -> Option<NodeId> {
        if self.container(id).is_some() {
            Some(id)
        } else {
            self.parent(id)
        }
    }

    pub fn toggle_orientation(&mut self, id: NodeId) {
        if let Some(c) = self.owning_container(id) {
            let cc = self.container_mut(c).unwrap();
            cc.orientation = cc.orientation.flip();
            // Children that now share the orientation would be redundant nesting.
            for k in self.children(c).to_vec() {
                self.flatten_if_same_orientation(k);
            }
        }
    }

    pub fn toggle_layout(&mut self, id: NodeId) {
        if let Some(c) = self.owning_container(id) {
            let cc = self.container_mut(c).unwrap();
            cc.layout = cc.layout.toggle();
        }
    }

    /// Give every child of the container a command on `id` changes
    /// ([`Tree::owning_container`]) the same size. Returns false when they
    /// already had it.
    pub fn balance(&mut self, id: NodeId) -> bool {
        let Some(c) = self.owning_container(id) else {
            return false;
        };
        let cc = self.container_mut(c).unwrap();
        if cc.weights.iter().all(|w| (*w - 1.0).abs() < 1e-9) {
            return false;
        }
        cc.weights.iter_mut().for_each(|w| *w = 1.0);
        true
    }

    /// Rebuild `root` as one master tile on the left (share `ratio` of the
    /// width) and the other leaves stacked in a column on the right, in
    /// visual order.
    pub fn arrange_master_stack(&mut self, root: NodeId, ratio: f64) {
        let mut leaves = self.leaf_sers(root);
        let ser = if leaves.len() <= 1 {
            row(leaves, vec![])
        } else {
            let master = leaves.remove(0);
            let stack = if leaves.len() == 1 {
                leaves.pop().unwrap()
            } else {
                column(leaves)
            };
            row(vec![master, stack], vec![ratio, 1.0 - ratio])
        };
        self.fill_from_ser(root, &ser);
    }

    /// Rebuild `root` as `n` columns of equal width (fewer if there are
    /// fewer leaves), each stacking its leaves in visual order. Earlier
    /// columns take the leftovers.
    pub fn arrange_columns(&mut self, root: NodeId, n: usize) {
        let leaves = self.leaf_sers(root);
        let cols = n.clamp(1, leaves.len().max(1));
        let (base, extra) = (leaves.len() / cols, leaves.len() % cols);
        let mut it = leaves.into_iter();
        let columns: Vec<SerNode> = (0..cols)
            .map(|i| {
                let take = base + usize::from(i < extra);
                let mut col: Vec<SerNode> = it.by_ref().take(take).collect();
                if col.len() == 1 {
                    col.pop().unwrap()
                } else {
                    column(col)
                }
            })
            .filter(|c| !matches!(c, SerNode::Container { children, .. } if children.is_empty()))
            .collect();
        self.fill_from_ser(root, &row(columns, vec![]));
    }

    /// The master tile's share of the width when `root` has the shape
    /// [`Tree::arrange_master_stack`] makes (two side-by-side tiles, the
    /// first a leaf).
    pub fn master_share(&self, root: NodeId) -> Option<f64> {
        let c = self.container(root)?;
        if c.orientation != Orientation::Horizontal
            || c.layout != ContainerLayout::Tiles
            || c.children.len() != 2
            || self.leaf(c.children[0]).is_none()
        {
            return None;
        }
        let total = c.weights[0] + c.weights[1];
        (total > 0.0).then(|| c.weights[0] / total)
    }

    fn leaf_sers(&self, root: NodeId) -> Vec<SerNode> {
        self.leaves(root)
            .into_iter()
            .map(|l| self.to_ser(l))
            .collect()
    }

    /// Grow (`delta > 0`) or shrink the share of `id` along `axis` by `delta`
    /// (a fraction of the parent's length). Uses the nearest ancestor split
    /// along `axis`. Returns false when there is none.
    pub fn resize(&mut self, id: NodeId, axis: Orientation, delta: f64) -> bool {
        let mut child = id;
        while let Some(p) = self.parent(child) {
            let c = self.container(p).unwrap();
            if c.orientation == axis && c.layout == ContainerLayout::Tiles && c.children.len() > 1 {
                let i = self.index_in_parent(child).unwrap();
                let c = self.container_mut(p).unwrap();
                let total: f64 = c.weights.iter().sum();
                let share = c.weights[i] / total;
                let max_share = 1.0 - MIN_SHARE * (c.children.len() - 1) as f64;
                let new_share = (share + delta).clamp(MIN_SHARE, max_share);
                if (new_share - share).abs() < 1e-9 {
                    return false;
                }
                let scale = (1.0 - new_share) / (1.0 - share);
                for (j, w) in c.weights.iter_mut().enumerate() {
                    *w = if j == i {
                        new_share * total
                    } else {
                        *w * scale
                    };
                }
                return true;
            }
            child = p;
        }
        false
    }

    /// Move one edge of `id` along `axis` by `delta_len` points (positive
    /// grows the tile), as when the user drags that edge. `cur_len` is the
    /// tile's current length along `axis`; `toward_next` picks the right or
    /// bottom edge, otherwise the left or top one. Only the neighbour across
    /// that edge gives or takes space. If the tile's edge is also the edge
    /// of its container, the nearest ancestor split with a neighbour on
    /// that side is used. `present` holds the leaves that take space.
    /// Returns false when nothing could change.
    pub fn resize_edge(
        &mut self,
        id: NodeId,
        axis: Orientation,
        delta_len: f64,
        cur_len: f64,
        toward_next: bool,
        present: &std::collections::HashSet<NodeId>,
    ) -> bool {
        let is_present = |t: &Tree, n: NodeId| t.leaves(n).iter().any(|l| present.contains(l));
        let mut child = id;
        while let Some(p) = self.parent(child) {
            let c = self.container(p).unwrap();
            if c.orientation == axis && c.layout == ContainerLayout::Tiles {
                let i = self.index_in_parent(child).unwrap();
                let neighbour = if toward_next {
                    (i + 1..c.children.len()).find(|j| is_present(self, c.children[*j]))
                } else {
                    (0..i).rev().find(|j| is_present(self, c.children[*j]))
                };
                if let Some(j) = neighbour {
                    let sum_w: f64 = c
                        .children
                        .iter()
                        .zip(&c.weights)
                        .filter(|(ch, _)| is_present(self, **ch))
                        .map(|(_, w)| *w)
                        .sum();
                    let (wi, wj) = (c.weights[i], c.weights[j]);
                    let per_point = wi / cur_len.max(1.0);
                    let min_w = MIN_SHARE * sum_w;
                    let dw = (delta_len * per_point)
                        .clamp(-(wi - min_w).max(0.0), (wj - min_w).max(0.0));
                    if dw.abs() < 1e-9 {
                        return false;
                    }
                    let c = self.container_mut(p).unwrap();
                    c.weights[i] = wi + dw;
                    c.weights[j] = wj - dw;
                    return true;
                }
            }
            child = p;
        }
        false
    }

    pub fn to_ser(&self, id: NodeId) -> SerNode {
        match &self.nodes[id].kind {
            NodeKind::Leaf(Leaf::Window(w)) => SerNode::Window { id: *w },
            NodeKind::Leaf(Leaf::Slot(s)) => SerNode::Slot {
                bundle_id: s.bundle_id.clone(),
                title: s.title.clone(),
                launch_args: s.launch_args.clone(),
            },
            NodeKind::Container(c) => SerNode::Container {
                orientation: c.orientation,
                layout: c.layout,
                weights: c.weights.clone(),
                children: c.children.iter().map(|ch| self.to_ser(*ch)).collect(),
            },
        }
    }

    /// Build a new root from a serialized tree. A leaf at the top level is
    /// wrapped in a horizontal root.
    pub fn root_from_ser(&mut self, ser: &SerNode) -> NodeId {
        match ser {
            SerNode::Container {
                orientation,
                layout,
                ..
            } => {
                let root = self.new_root(*orientation, *layout);
                self.fill_from_ser(root, ser);
                root
            }
            leaf => {
                let root = self.new_root(Orientation::Horizontal, ContainerLayout::Tiles);
                self.add_ser_child(root, leaf, 1.0);
                root
            }
        }
    }

    /// Replace the children of `root` with the contents of `ser`.
    pub fn fill_from_ser(&mut self, root: NodeId, ser: &SerNode) {
        self.clear(root);
        match ser {
            SerNode::Container {
                orientation,
                layout,
                weights,
                children,
            } => {
                let rc = self.container_mut(root).unwrap();
                rc.orientation = *orientation;
                rc.layout = *layout;
                for (i, ch) in children.iter().enumerate() {
                    self.add_ser_child(root, ch, weights.get(i).copied().unwrap_or(1.0));
                }
            }
            leaf => {
                self.add_ser_child(root, leaf, 1.0);
            }
        }
        // Enforce the "no single-child inner container" invariant.
        for ch in self.children(root).to_vec() {
            self.normalize_deep(ch);
        }
        self.normalize(root);
    }

    fn normalize_deep(&mut self, id: NodeId) {
        for ch in self.children(id).to_vec() {
            self.normalize_deep(ch);
        }
        if self.contains(id) && self.container(id).is_some() {
            self.normalize(id);
        }
    }

    fn add_ser_child(&mut self, parent: NodeId, ser: &SerNode, weight: f64) -> NodeId {
        let weight = if weight.is_finite() && weight > 0.0 {
            weight
        } else {
            1.0
        };
        let n = self.children(parent).len();
        match ser {
            SerNode::Window { id } => {
                self.insert_child(parent, n, NodeKind::Leaf(Leaf::Window(*id)), Some(weight))
            }
            SerNode::Slot {
                bundle_id,
                title,
                launch_args,
            } => self.insert_child(
                parent,
                n,
                NodeKind::Leaf(Leaf::Slot(AppSlot {
                    bundle_id: bundle_id.clone(),
                    title: title.clone(),
                    launch_args: launch_args.clone(),
                })),
                Some(weight),
            ),
            SerNode::Container {
                orientation,
                layout,
                weights,
                children,
            } => {
                let id = self.insert_child(
                    parent,
                    n,
                    NodeKind::Container(Container {
                        orientation: *orientation,
                        layout: *layout,
                        children: vec![],
                        weights: vec![],
                    }),
                    Some(weight),
                );
                for (i, ch) in children.iter().enumerate() {
                    self.add_ser_child(id, ch, weights.get(i).copied().unwrap_or(1.0));
                }
                id
            }
        }
    }

    /// Delete a root and everything below it.
    pub fn remove_root(&mut self, root: NodeId) -> Vec<Leaf> {
        let mut removed = Vec::new();
        self.delete_subtree(root, &mut removed);
        removed
    }
}

/// Serializable form of a (sub)tree, shared by `state.json` and profiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerNode {
    Container {
        #[serde(default)]
        orientation: Orientation,
        #[serde(default)]
        layout: ContainerLayout,
        #[serde(default)]
        weights: Vec<f64>,
        #[serde(default)]
        children: Vec<SerNode>,
    },
    Window {
        id: WindowId,
    },
    Slot {
        bundle_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        launch_args: Vec<String>,
    },
}

/// A horizontal tiles container; empty `weights` means equal sizes.
fn row(children: Vec<SerNode>, weights: Vec<f64>) -> SerNode {
    let weights = if weights.is_empty() {
        vec![1.0; children.len()]
    } else {
        weights
    };
    SerNode::Container {
        orientation: Orientation::Horizontal,
        layout: ContainerLayout::Tiles,
        weights,
        children,
    }
}

/// A vertical tiles container with equal sizes.
fn column(children: Vec<SerNode>) -> SerNode {
    SerNode::Container {
        orientation: Orientation::Vertical,
        layout: ContainerLayout::Tiles,
        weights: vec![1.0; children.len()],
        children,
    }
}

impl SerNode {
    pub fn empty() -> Self {
        SerNode::Container {
            orientation: Orientation::Horizontal,
            layout: ContainerLayout::Tiles,
            weights: vec![],
            children: vec![],
        }
    }

    /// Visit every leaf with its index path.
    pub fn for_each_leaf(&self, f: &mut impl FnMut(&[usize], &SerNode)) {
        fn walk(n: &SerNode, path: &mut Vec<usize>, f: &mut impl FnMut(&[usize], &SerNode)) {
            match n {
                SerNode::Container { children, .. } => {
                    for (i, c) in children.iter().enumerate() {
                        path.push(i);
                        walk(c, path, f);
                        path.pop();
                    }
                }
                leaf => f(path, leaf),
            }
        }
        walk(self, &mut Vec::new(), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(n: u32) -> Leaf {
        Leaf::Window(WindowId(n))
    }

    fn shape(t: &Tree, id: NodeId) -> String {
        match &t.node(id).unwrap().kind {
            NodeKind::Leaf(Leaf::Window(w)) => w.0.to_string(),
            NodeKind::Leaf(Leaf::Slot(s)) => s.bundle_id.clone(),
            NodeKind::Container(c) => {
                let o = match c.orientation {
                    Orientation::Horizontal => "H",
                    Orientation::Vertical => "V",
                };
                let inner: Vec<String> = c.children.iter().map(|ch| shape(t, *ch)).collect();
                format!("{o}[{}]", inner.join(" "))
            }
        }
    }

    fn setup(n: u32) -> (Tree, NodeId, Vec<NodeId>) {
        let mut t = Tree::new();
        let root = t.new_root(Orientation::Horizontal, ContainerLayout::Tiles);
        let ids = (1..=n)
            .map(|i| t.insert_leaf(root, w(i), InsertAt::End))
            .collect();
        (t, root, ids)
    }

    #[test]
    fn insert_and_remove_collapses() {
        let (mut t, root, ids) = setup(2);
        let c = t.insert_leaf(
            root,
            w(3),
            InsertAt::Split {
                target: ids[1],
                dir: Direction::Down,
            },
        );
        assert_eq!(shape(&t, root), "H[1 V[2 3]]");
        t.remove(c);
        assert_eq!(shape(&t, root), "H[1 2]");
        t.remove(ids[0]);
        assert_eq!(shape(&t, root), "H[2]");
        t.remove(ids[1]);
        assert_eq!(shape(&t, root), "H[]");
    }

    #[test]
    fn root_absorbs_single_container() {
        let (mut t, root, ids) = setup(2);
        t.insert_leaf(
            root,
            w(3),
            InsertAt::Split {
                target: ids[1],
                dir: Direction::Down,
            },
        );
        t.remove(ids[0]);
        assert_eq!(shape(&t, root), "V[2 3]");
    }

    #[test]
    fn move_swaps_siblings_and_enters_containers() {
        let (mut t, root, ids) = setup(3);
        assert_eq!(
            t.move_leaf(ids[0], Direction::Right),
            MoveResult::Moved(ids[0])
        );
        assert_eq!(shape(&t, root), "H[2 1 3]");
        assert_eq!(t.move_leaf(ids[2], Direction::Right), MoveResult::AtEdge);
        // 3 moves down: root is horizontal, so the root gets wrapped.
        let r = t.move_leaf(ids[2], Direction::Down);
        assert!(matches!(r, MoveResult::Moved(_)));
        assert_eq!(shape(&t, root), "V[H[2 1] 3]");
        // Moving 3 back up enters the H container from below (appended).
        let MoveResult::Moved(n3) =
            t.move_leaf(t.find_window(root, WindowId(3)).unwrap(), Direction::Up)
        else {
            panic!()
        };
        assert_eq!(shape(&t, root), "H[2 1 3]");
        assert_eq!(t.leaf(n3), Some(&w(3)));
    }

    #[test]
    fn move_out_of_nested_container() {
        let (mut t, root, ids) = setup(2);
        let n3 = t.insert_leaf(
            root,
            w(3),
            InsertAt::Split {
                target: ids[1],
                dir: Direction::Down,
            },
        );
        assert_eq!(shape(&t, root), "H[1 V[2 3]]");
        let r = t.move_leaf(n3, Direction::Right);
        assert!(matches!(r, MoveResult::Moved(_)));
        assert_eq!(shape(&t, root), "H[1 2 3]");
        let n3 = t.find_window(root, WindowId(3)).unwrap();
        t.insert_leaf(
            root,
            w(4),
            InsertAt::Split {
                target: ids[1],
                dir: Direction::Down,
            },
        );
        assert_eq!(shape(&t, root), "H[1 V[2 4] 3]");
        let n4 = t.find_window(root, WindowId(4)).unwrap();
        t.move_leaf(n4, Direction::Left);
        assert_eq!(shape(&t, root), "H[1 4 2 3]");
        assert_eq!(t.move_leaf(n3, Direction::Right), MoveResult::AtEdge);
    }

    #[test]
    fn resize_changes_weights_and_clamps() {
        let (mut t, root, ids) = setup(2);
        assert!(t.resize(ids[0], Orientation::Horizontal, 0.1));
        let c = t.container(root).unwrap();
        let total: f64 = c.weights.iter().sum();
        assert!((c.weights[0] / total - 0.6).abs() < 1e-9);
        assert!(!t.resize(ids[0], Orientation::Vertical, 0.1));
        for _ in 0..20 {
            t.resize(ids[0], Orientation::Horizontal, 0.1);
        }
        let c = t.container(root).unwrap();
        let total: f64 = c.weights.iter().sum();
        assert!((c.weights[0] / total - 0.95).abs() < 1e-9);
    }

    #[test]
    fn paths_and_ser_roundtrip() {
        let (mut t, root, ids) = setup(2);
        let n3 = t.insert_leaf(
            root,
            w(3),
            InsertAt::Split {
                target: ids[1],
                dir: Direction::Down,
            },
        );
        assert_eq!(t.path_of(n3), vec![1, 1]);
        assert_eq!(t.node_at_path(root, &[1, 1]), Some(n3));
        let ser = t.to_ser(root);
        let mut t2 = Tree::new();
        let r2 = t2.root_from_ser(&ser);
        assert_eq!(shape(&t2, r2), shape(&t, root));
        let json = serde_json::to_string(&ser).unwrap();
        let back: SerNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ser);
    }

    #[test]
    fn toggle_orientation_flattens() {
        let (mut t, root, ids) = setup(2);
        t.insert_leaf(
            root,
            w(3),
            InsertAt::Split {
                target: ids[1],
                dir: Direction::Down,
            },
        );
        t.toggle_orientation(ids[0]);
        assert_eq!(shape(&t, root), "V[1 2 3]");
    }

    #[test]
    fn balance_equalizes_the_focused_container() {
        let (mut t, root, ids) = setup(3);
        t.resize(ids[0], Orientation::Horizontal, 0.2);
        assert!(t.balance(ids[1]));
        assert_eq!(t.container(root).unwrap().weights, vec![1.0; 3]);
        assert!(!t.balance(ids[1]));
    }

    #[test]
    fn master_stack_and_columns() {
        let (mut t, root, _) = setup(1);
        t.arrange_master_stack(root, 0.6);
        assert_eq!(shape(&t, root), "H[1]");
        t.insert_leaf(root, w(2), InsertAt::End);
        t.arrange_master_stack(root, 0.6);
        assert_eq!(shape(&t, root), "H[1 2]");
        assert!((t.master_share(root).unwrap() - 0.6).abs() < 1e-9);
        t.insert_leaf(root, w(3), InsertAt::End);
        t.insert_leaf(root, w(4), InsertAt::End);
        t.arrange_master_stack(root, 0.7);
        assert_eq!(shape(&t, root), "H[1 V[2 3 4]]");
        assert!((t.master_share(root).unwrap() - 0.7).abs() < 1e-9);

        t.insert_leaf(root, w(5), InsertAt::End);
        t.arrange_columns(root, 3);
        assert_eq!(shape(&t, root), "H[V[1 2] V[3 4] 5]");
        assert_eq!(t.container(root).unwrap().weights, vec![1.0; 3]);
        assert_eq!(t.master_share(root), None);
        t.arrange_columns(root, 8);
        assert_eq!(shape(&t, root), "H[1 2 3 4 5]");
        t.arrange_columns(root, 1);
        assert_eq!(shape(&t, root), "V[1 2 3 4 5]");

        let (mut t, root, _) = setup(0);
        t.arrange_columns(root, 2);
        t.arrange_master_stack(root, 0.5);
        assert_eq!(shape(&t, root), "H[]");
    }

    #[test]
    fn insert_at_path() {
        let (mut t, root, _) = setup(2);
        t.insert_leaf(root, w(9), InsertAt::Path(vec![1]));
        assert_eq!(shape(&t, root), "H[1 9 2]");
        t.insert_leaf(root, w(8), InsertAt::Path(vec![7, 0]));
        assert_eq!(shape(&t, root), "H[1 9 2 8]");
    }
}
