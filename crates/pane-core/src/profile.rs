//! Profiles: a saved desk ("which app in which pane on which screen").
//!
//! Profiles are trees of [`AppSlot`] leaves per display. Editor operations
//! run on the same [`Tree`] code as live layouts.

use serde::{Deserialize, Serialize};

use crate::geometry::{Direction, Orientation};
use crate::tree::{AppSlot, InsertAt, Leaf, NodeId, SerNode, Tree};

pub const PROFILE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default = "version_one")]
    pub version: u32,
    #[serde(default, rename = "display")]
    pub displays: Vec<ProfileDisplay>,
}

fn version_one() -> u32 {
    PROFILE_VERSION
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileDisplay {
    /// Display reference in short form (`left`, `main`, `uuid:...`, a config alias).
    pub display: String,
    pub workspace: String,
    pub tree: SerNode,
}

impl Profile {
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn from_toml(s: &str) -> Result<Profile, String> {
        let p: Profile = toml::from_str(s).map_err(|e| e.to_string())?;
        if p.version != PROFILE_VERSION {
            return Err(format!("profile version {} is not supported", p.version));
        }
        Ok(p)
    }

    /// File-name-safe form of the name.
    pub fn file_stem(name: &str) -> String {
        let s: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let s = s.trim().to_string();
        if s.is_empty() {
            "profile".into()
        } else {
            s
        }
    }

    /// Every slot with (display index, path).
    pub fn slots(&self) -> Vec<(usize, Vec<usize>, AppSlot)> {
        let mut out = Vec::new();
        for (i, d) in self.displays.iter().enumerate() {
            d.tree.for_each_leaf(&mut |path, leaf| {
                if let SerNode::Slot {
                    bundle_id,
                    title,
                    launch_args,
                } = leaf
                {
                    out.push((
                        i,
                        path.to_vec(),
                        AppSlot {
                            bundle_id: bundle_id.clone(),
                            title: title.clone(),
                            launch_args: launch_args.clone(),
                        },
                    ));
                }
            });
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DropZone {
    Center,
    Left,
    Right,
    Up,
    Down,
}

impl DropZone {
    fn dir(self) -> Option<Direction> {
        match self {
            DropZone::Center => None,
            DropZone::Left => Some(Direction::Left),
            DropZone::Right => Some(Direction::Right),
            DropZone::Up => Some(Direction::Up),
            DropZone::Down => Some(Direction::Down),
        }
    }
}

/// A tile address: display index plus index path (empty = the root).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileRef {
    pub display: usize,
    #[serde(default)]
    pub path: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProfileEdit {
    /// Drop an app from the palette onto a tile (or an empty display).
    Drop {
        at: TileRef,
        zone: DropZone,
        slot: AppSlot,
    },
    /// Drag an existing tile somewhere else.
    MoveTile {
        from: TileRef,
        to: TileRef,
        zone: DropZone,
    },
    Swap {
        a: TileRef,
        b: TileRef,
    },
    Remove {
        at: TileRef,
    },
    SetWeight {
        at: TileRef,
        weight: f64,
    },
    SetOrientation {
        at: TileRef,
        orientation: Orientation,
    },
    ToggleLayout {
        at: TileRef,
    },
    SetSlot {
        at: TileRef,
        slot: AppSlot,
    },
    SetWorkspace {
        display: usize,
        workspace: String,
    },
    SetDisplay {
        display: usize,
        display_ref: String,
    },
    AddDisplay {
        display_ref: String,
        workspace: String,
    },
    RemoveDisplay {
        display: usize,
    },
    Rename {
        name: String,
    },
}

struct Work {
    tree: Tree,
    roots: Vec<NodeId>,
}

impl Work {
    fn new(p: &Profile) -> Work {
        let mut tree = Tree::new();
        let roots = p
            .displays
            .iter()
            .map(|d| tree.root_from_ser(&d.tree))
            .collect();
        Work { tree, roots }
    }

    fn node(&self, at: &TileRef) -> Result<NodeId, String> {
        let root = *self
            .roots
            .get(at.display)
            .ok_or("no such display in profile")?;
        self.tree
            .node_at_path(root, &at.path)
            .ok_or_else(|| "no such tile".to_string())
    }

    fn drop_leaf(&mut self, target: NodeId, zone: DropZone, leaf: Leaf) {
        let root = self.tree.root_of(target);
        match zone.dir() {
            None => {
                if self.tree.leaf(target).is_some() {
                    *self.tree.leaf_mut(target).unwrap() = leaf;
                } else {
                    let n = self.tree.children(target).len();
                    self.tree.insert_leaf(
                        root,
                        leaf,
                        if n == 0 {
                            InsertAt::End
                        } else {
                            InsertAt::After(*self.tree.children(target).last().unwrap())
                        },
                    );
                }
            }
            Some(dir) => {
                self.tree
                    .insert_leaf(root, leaf, InsertAt::Split { target, dir });
            }
        }
    }

    fn write_back(&self, p: &mut Profile) {
        for (i, r) in self.roots.iter().enumerate() {
            p.displays[i].tree = self.tree.to_ser(*r);
        }
    }
}

/// Apply an editor operation, returning the new profile.
pub fn edit(profile: &Profile, op: &ProfileEdit) -> Result<Profile, String> {
    let mut p = profile.clone();
    match op {
        ProfileEdit::SetWorkspace { display, workspace } => {
            p.displays
                .get_mut(*display)
                .ok_or("no such display")?
                .workspace = workspace.clone();
            return Ok(p);
        }
        ProfileEdit::SetDisplay {
            display,
            display_ref,
        } => {
            p.displays
                .get_mut(*display)
                .ok_or("no such display")?
                .display = display_ref.clone();
            return Ok(p);
        }
        ProfileEdit::AddDisplay {
            display_ref,
            workspace,
        } => {
            p.displays.push(ProfileDisplay {
                display: display_ref.clone(),
                workspace: workspace.clone(),
                tree: SerNode::empty(),
            });
            return Ok(p);
        }
        ProfileEdit::RemoveDisplay { display } => {
            if *display >= p.displays.len() {
                return Err("no such display".into());
            }
            p.displays.remove(*display);
            return Ok(p);
        }
        ProfileEdit::Rename { name } => {
            p.name = name.clone();
            return Ok(p);
        }
        _ => {}
    }
    let mut w = Work::new(&p);
    match op {
        ProfileEdit::Drop { at, zone, slot } => {
            let t = w.node(at)?;
            w.drop_leaf(t, *zone, Leaf::Slot(slot.clone()));
        }
        ProfileEdit::MoveTile { from, to, zone } => {
            let src = w.node(from)?;
            let dst = w.node(to)?;
            if src == dst {
                return Ok(p);
            }
            let leaf = w.tree.leaf(src).cloned().ok_or("can only move tiles")?;
            let dst_root = w.roots[to.display];
            w.tree.remove(src);
            let dst = if w.tree.contains(dst) { dst } else { dst_root };
            w.drop_leaf(dst, *zone, leaf);
        }
        ProfileEdit::Swap { a, b } => {
            let (na, nb) = (w.node(a)?, w.node(b)?);
            w.tree.swap_leaves(na, nb);
        }
        ProfileEdit::Remove { at } => {
            let n = w.node(at)?;
            if w.roots.contains(&n) {
                w.tree.clear(n);
            } else {
                w.tree.remove(n);
            }
        }
        ProfileEdit::SetWeight { at, weight } => {
            let n = w.node(at)?;
            w.tree.set_weight(n, *weight);
        }
        ProfileEdit::SetOrientation { at, orientation } => {
            let n = w.node(at)?;
            let c = w.tree.owning_container(n).ok_or("no container")?;
            w.tree.container_mut(c).unwrap().orientation = *orientation;
        }
        ProfileEdit::ToggleLayout { at } => {
            let n = w.node(at)?;
            w.tree.toggle_layout(n);
        }
        ProfileEdit::SetSlot { at, slot } => {
            if let Some(t) = &slot.title {
                regex::Regex::new(t).map_err(|e| format!("bad title regex: {e}"))?;
            }
            let n = w.node(at)?;
            *w.tree.leaf_mut(n).ok_or("not a tile")? = Leaf::Slot(slot.clone());
        }
        _ => unreachable!(),
    }
    w.write_back(&mut p);
    Ok(p)
}

/// Does a window title satisfy a slot's title regex?
pub fn title_matches(slot: &AppSlot, title: &str) -> bool {
    match &slot.title {
        None => true,
        Some(re) => regex::Regex::new(re)
            .map(|r| r.is_match(title))
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(b: &str) -> AppSlot {
        AppSlot {
            bundle_id: b.into(),
            title: None,
            launch_args: vec![],
        }
    }

    fn base() -> Profile {
        Profile {
            name: "work".into(),
            version: 1,
            displays: vec![ProfileDisplay {
                display: "left".into(),
                workspace: "1".into(),
                tree: SerNode::empty(),
            }],
        }
    }

    #[test]
    fn drop_zones_build_tree() {
        let root = TileRef {
            display: 0,
            path: vec![],
        };
        let p = edit(
            &base(),
            &ProfileEdit::Drop {
                at: root.clone(),
                zone: DropZone::Center,
                slot: slot("mail"),
            },
        )
        .unwrap();
        let p = edit(
            &p,
            &ProfileEdit::Drop {
                at: TileRef {
                    display: 0,
                    path: vec![0],
                },
                zone: DropZone::Right,
                slot: slot("safari"),
            },
        )
        .unwrap();
        let p = edit(
            &p,
            &ProfileEdit::Drop {
                at: TileRef {
                    display: 0,
                    path: vec![1],
                },
                zone: DropZone::Down,
                slot: slot("term"),
            },
        )
        .unwrap();
        let slots: Vec<(Vec<usize>, String)> = p
            .slots()
            .into_iter()
            .map(|(_, path, s)| (path, s.bundle_id))
            .collect();
        assert_eq!(
            slots,
            vec![
                (vec![0], "mail".into()),
                (vec![1, 0], "safari".into()),
                (vec![1, 1], "term".into())
            ]
        );
        // Replace via centre drop, then move a tile.
        let p = edit(
            &p,
            &ProfileEdit::Drop {
                at: TileRef {
                    display: 0,
                    path: vec![0],
                },
                zone: DropZone::Center,
                slot: slot("notes"),
            },
        )
        .unwrap();
        let p = edit(
            &p,
            &ProfileEdit::MoveTile {
                from: TileRef {
                    display: 0,
                    path: vec![1, 1],
                },
                to: TileRef {
                    display: 0,
                    path: vec![0],
                },
                zone: DropZone::Left,
            },
        )
        .unwrap();
        let order: Vec<String> = p.slots().into_iter().map(|s| s.2.bundle_id).collect();
        assert_eq!(order, vec!["term", "notes", "safari"]);
    }

    #[test]
    fn toml_roundtrip() {
        let root = TileRef {
            display: 0,
            path: vec![],
        };
        let p = edit(
            &base(),
            &ProfileEdit::Drop {
                at: root.clone(),
                zone: DropZone::Center,
                slot: slot("com.apple.mail"),
            },
        )
        .unwrap();
        let p = edit(
            &p,
            &ProfileEdit::Drop {
                at: TileRef {
                    display: 0,
                    path: vec![0],
                },
                zone: DropZone::Down,
                slot: AppSlot {
                    bundle_id: "com.apple.Safari".into(),
                    title: Some("^Docs".into()),
                    launch_args: vec![],
                },
            },
        )
        .unwrap();
        let text = p.to_toml().unwrap();
        let back = Profile::from_toml(&text).unwrap();
        assert_eq!(back, p, "{text}");
    }

    #[test]
    fn bad_regex_rejected() {
        let p = edit(
            &base(),
            &ProfileEdit::Drop {
                at: TileRef {
                    display: 0,
                    path: vec![],
                },
                zone: DropZone::Center,
                slot: slot("a"),
            },
        )
        .unwrap();
        let bad = AppSlot {
            bundle_id: "a".into(),
            title: Some("(".into()),
            launch_args: vec![],
        };
        assert!(edit(
            &p,
            &ProfileEdit::SetSlot {
                at: TileRef {
                    display: 0,
                    path: vec![0]
                },
                slot: bad
            }
        )
        .is_err());
    }
}
