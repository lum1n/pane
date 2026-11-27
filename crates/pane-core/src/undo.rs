//! Layout undo: a ring of the last [`UNDO_DEPTH`] workspace-tree states.
//!
//! Tree-changing commands (move, resize, swap, move-to-workspace, toggles,
//! profile apply, edge drags) push the state from before them when they
//! changed something. `undo` puts the newest one back; reconcile then moves
//! the windows. Windows that closed since are left out, and windows that
//! opened since stay where they are, so an undo never loses a window.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Model, WindowMode};
use crate::tree::{AppSlot, Leaf, SerNode};
use crate::workspace::LayoutMode;
use crate::{DisplayId, Rect, WindowId, WorkspaceId};

/// How many layouts `undo` can go back.
pub const UNDO_DEPTH: usize = 20;

/// One workspace's tree and floating windows.
#[derive(Clone, Debug, PartialEq)]
pub struct WsLayout {
    pub id: WorkspaceId,
    pub home: DisplayId,
    pub name: String,
    pub tree: SerNode,
    pub floating: Vec<WindowId>,
    pub focused: Option<WindowId>,
    pub layout: Option<LayoutMode>,
}

/// Where a window was.
#[derive(Clone, Debug, PartialEq)]
pub struct WinLayout {
    pub workspace: WorkspaceId,
    pub mode: WindowMode,
    pub float_frame: Option<Rect>,
    pub slot: Option<AppSlot>,
}

/// Every workspace tree plus what goes with it (window placement,
/// visible workspaces, focus, the active profile).
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutState {
    pub workspaces: Vec<WsLayout>,
    pub windows: BTreeMap<WindowId, WinLayout>,
    pub visible: BTreeMap<DisplayId, WorkspaceId>,
    pub focus_window: Option<WindowId>,
    pub active_profile: Option<String>,
}

pub fn capture(m: &Model) -> LayoutState {
    LayoutState {
        workspaces: m
            .workspaces
            .iter()
            .map(|w| WsLayout {
                id: w.id.clone(),
                home: w.home.clone(),
                name: w.name.clone(),
                tree: m.tree.to_ser(w.root),
                floating: w.floating.clone(),
                focused: w.focused,
                layout: w.layout,
            })
            .collect(),
        windows: m
            .windows
            .values()
            .map(|w| {
                (
                    w.id,
                    WinLayout {
                        workspace: w.workspace.clone(),
                        mode: w.mode,
                        float_frame: w.float_frame,
                        slot: w.slot.clone(),
                    },
                )
            })
            .collect(),
        visible: m.visible.clone(),
        focus_window: m.focus_window,
        active_profile: m.active_profile.clone(),
    }
}

/// Push `before` if the layout is no longer what it was.
pub fn record(m: &mut Model, before: LayoutState) {
    if capture(m) == before {
        return;
    }
    m.undo.push_back(before);
    while m.undo.len() > UNDO_DEPTH {
        m.undo.pop_front();
    }
}

/// `ser` without window leaves `keep` rejects. Containers left empty go
/// too (the top level stays, possibly empty).
fn prune(ser: &SerNode, keep: &dyn Fn(WindowId) -> bool) -> Option<SerNode> {
    match ser {
        SerNode::Window { id } => keep(*id).then(|| ser.clone()),
        SerNode::Slot { .. } => Some(ser.clone()),
        SerNode::Container {
            orientation,
            layout,
            weights,
            children,
        } => {
            let mut w = Vec::new();
            let mut c = Vec::new();
            for (i, ch) in children.iter().enumerate() {
                if let Some(p) = prune(ch, keep) {
                    w.push(weights.get(i).copied().unwrap_or(1.0));
                    c.push(p);
                }
            }
            (!c.is_empty()).then_some(SerNode::Container {
                orientation: *orientation,
                layout: *layout,
                weights: w,
                children: c,
            })
        }
    }
}

fn ser_windows(ser: &SerNode, out: &mut Vec<WindowId>) {
    match ser {
        SerNode::Window { id } => out.push(*id),
        SerNode::Slot { .. } => {}
        SerNode::Container { children, .. } => {
            for c in children {
                ser_windows(c, out);
            }
        }
    }
}

/// Put `s` back. Returns the window to focus, if it still exists.
pub fn restore(m: &mut Model, s: &LayoutState) -> Option<WindowId> {
    let live = |m: &Model, w: WindowId| m.windows.contains_key(&w) || m.ghosts.contains_key(&w);
    // Workspaces to restore: those that can exist again under the same id
    // (a display Pane has never heard of would give another id).
    let mut restored: Vec<(&WsLayout, SerNode, Vec<WindowId>)> = Vec::new();
    let mut placed: BTreeSet<WindowId> = BTreeSet::new();
    for wl in &s.workspaces {
        if m.ws(&wl.id).is_none() && m.ensure_workspace(&wl.home, &wl.name) != wl.id {
            continue;
        }
        let tree = prune(&wl.tree, &|w| live(m, w)).unwrap_or_else(SerNode::empty);
        let floating: Vec<WindowId> = wl
            .floating
            .iter()
            .copied()
            .filter(|w| m.windows.contains_key(w))
            .collect();
        let mut wins = Vec::new();
        ser_windows(&tree, &mut wins);
        placed.extend(wins);
        placed.extend(floating.iter().copied());
        restored.push((wl, tree, floating));
    }
    // Trees as they are now: a workspace the undo leaves as it is keeps its
    // zoomed window.
    let trees_now: BTreeMap<WorkspaceId, SerNode> = restored
        .iter()
        .filter_map(|(wl, _, _)| Some((wl.id.clone(), m.tree.to_ser(m.ws(&wl.id)?.root))))
        .collect();
    // Take the restored windows out of wherever they are now. Workspaces
    // made since (by the command undone) also lose their profile slots.
    let known: BTreeSet<WorkspaceId> = s.workspaces.iter().map(|w| w.id.clone()).collect();
    for i in 0..m.workspaces.len() {
        let root = m.workspaces[i].root;
        let new = !known.contains(&m.workspaces[i].id);
        while let Some(n) = m.tree.find_leaf(root, |l| match l {
            Leaf::Window(w) => placed.contains(w),
            Leaf::Slot(_) => new,
        }) {
            m.tree.remove(n);
        }
        m.workspaces[i].floating.retain(|w| !placed.contains(w));
    }
    for (wl, tree, floating) in &restored {
        let ws = m.ws(&wl.id).unwrap();
        let root = ws.root;
        // Windows here that the old layout doesn't know (opened since) stay.
        let extra_tiled: Vec<WindowId> = m
            .tree
            .windows(root)
            .into_iter()
            .filter(|w| m.windows.contains_key(w))
            .collect();
        let extra_floating = ws.floating.clone();
        m.tree.fill_from_ser(root, tree);
        for w in extra_tiled {
            m.tree
                .insert_leaf(root, Leaf::Window(w), crate::tree::InsertAt::End);
        }
        let ws = m.ws_mut(&wl.id).unwrap();
        ws.floating = floating.clone();
        ws.floating.extend(extra_floating);
        ws.focused = wl.focused.filter(|w| placed.contains(w));
        ws.layout = wl.layout;
        let ws = m.ws(&wl.id).unwrap();
        let unchanged = trees_now.get(&wl.id) == Some(&m.tree.to_ser(ws.root));
        if !unchanged {
            m.ws_mut(&wl.id).unwrap().fullscreen = None;
        }
    }
    for (wid, wl) in &s.windows {
        if !placed.contains(wid) {
            continue;
        }
        if let Some(w) = m.windows.get_mut(wid) {
            w.workspace = wl.workspace.clone();
            w.mode = wl.mode;
            w.float_frame = wl.float_frame;
            w.slot = wl.slot.clone();
        }
    }
    for (d, ws) in &s.visible {
        if m.display(d).is_some() && m.ws(ws).is_some_and(|w| w.display == *d) {
            m.visible.insert(d.clone(), ws.clone());
            m.remembered_visible.insert(d.clone(), ws.clone());
        }
    }
    // Launches for slots that are gone place their windows as usual.
    let pending = std::mem::take(&mut m.pending_slots);
    m.pending_slots = pending
        .into_iter()
        .filter(|p| {
            m.ws(&p.workspace).is_some_and(|w| {
                m.tree
                    .find_leaf(
                        w.root,
                        |l| matches!(l, Leaf::Slot(x) if x.bundle_id == p.bundle_id),
                    )
                    .is_some()
            })
        })
        .collect();
    m.active_profile = s.active_profile.clone();
    m.layout_dirty = true;
    s.focus_window
        .filter(|w| m.windows.get(w).is_some_and(|x| m.window_shown(x)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Orientation;
    use crate::tree::ContainerLayout;

    #[test]
    fn prune_drops_windows_and_empty_containers() {
        let ser = SerNode::Container {
            orientation: Orientation::Horizontal,
            layout: ContainerLayout::Tiles,
            weights: vec![2.0, 1.0],
            children: vec![
                SerNode::Window { id: WindowId(1) },
                SerNode::Container {
                    orientation: Orientation::Vertical,
                    layout: ContainerLayout::Tiles,
                    weights: vec![1.0, 1.0],
                    children: vec![
                        SerNode::Window { id: WindowId(2) },
                        SerNode::Window { id: WindowId(3) },
                    ],
                },
            ],
        };
        let p = prune(&ser, &|w| w == WindowId(1)).unwrap();
        let SerNode::Container {
            weights, children, ..
        } = p
        else {
            panic!()
        };
        assert_eq!(weights, vec![2.0]);
        assert_eq!(children, vec![SerNode::Window { id: WindowId(1) }]);
        assert_eq!(prune(&ser, &|_| false), None);
    }
}
