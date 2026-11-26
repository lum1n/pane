//! Profiles: a saved desk ("which app in which pane on which screen").
//!
//! Profiles are trees of [`AppSlot`] leaves per display. Editor operations
//! run on the same [`Tree`] code as live layouts.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::display::Display;
use crate::geometry::{Direction, Orientation};
use crate::tree::{AppSlot, InsertAt, Leaf, NodeId, SerNode, Tree};

pub const PROFILE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default = "version_one")]
    pub version: u32,
    /// When to apply this profile by itself (`[when]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<ProfileCondition>,
    #[serde(default, rename = "display")]
    pub displays: Vec<ProfileDisplay>,
}

/// The desk a profile is for: it applies by itself when the connected
/// displays come to match.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileCondition {
    /// Displays that must all be connected: `uuid:...`, `name:...` or a
    /// `[[display]]` alias. Positions (`left`, `main`) match any display.
    pub displays: Vec<String>,
    /// No other display may be connected (the default). With `false`, more
    /// displays are fine and the profile with the most matching ones wins.
    #[serde(default = "yes", skip_serializing_if = "Clone::clone")]
    pub exact: bool,
}

fn yes() -> bool {
    true
}

impl ProfileCondition {
    /// A condition matching exactly these displays, by UUID.
    pub fn for_displays(displays: &[Display]) -> ProfileCondition {
        let mut ids: Vec<String> = displays.iter().map(|d| format!("uuid:{}", d.id)).collect();
        ids.sort();
        ProfileCondition {
            displays: ids,
            exact: true,
        }
    }

    /// How well the connected (ranked) displays match: `None` if they don't,
    /// else a score where higher is more specific.
    pub fn score(&self, displays: &[Display], config: &Config) -> Option<usize> {
        if self.displays.is_empty() {
            return None;
        }
        let mut used: Vec<&str> = Vec::new();
        for r in &self.displays {
            let d = config.display_ref(r).resolve(displays)?;
            if used.contains(&d.id.as_str()) {
                return None;
            }
            used.push(&d.id);
        }
        if self.exact && used.len() != displays.len() {
            return None;
        }
        // An exact match beats any partial one; then more displays wins.
        Some(used.len() + if self.exact { 1000 } else { 0 })
    }
}

/// Key identifying a set of connected displays (order does not matter).
pub fn topology_key(displays: &[Display]) -> String {
    let mut ids: Vec<&str> = displays.iter().map(|d| d.id.as_str()).collect();
    ids.sort_unstable();
    ids.join(",")
}

/// The profile meant for the connected displays: the best-scoring `[when]`
/// match. The active profile wins a tie, then the first name alphabetically.
pub fn pick_for_displays<'a>(
    profiles: &'a [Profile],
    displays: &[Display],
    config: &Config,
    active: Option<&str>,
) -> Option<&'a Profile> {
    profiles
        .iter()
        .filter_map(|p| Some((p.when.as_ref()?.score(displays, config)?, p)))
        .max_by(|(sa, a), (sb, b)| {
            sa.cmp(sb)
                .then_with(|| (active == Some(&a.name)).cmp(&(active == Some(&b.name))))
                .then_with(|| b.name.cmp(&a.name))
        })
        .map(|(_, p)| p)
}

fn version_one() -> u32 {
    PROFILE_VERSION
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileDisplay {
    /// Display reference in short form (`left`, `main`, `uuid:...`, a config alias).
    pub display: String,
    /// Workspace name on that display ("1", "2", "web").
    pub workspace: String,
    pub tree: SerNode,
    /// Shown on its display after applying. Without any, a display shows
    /// its first workspace in the profile.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub visible: bool,
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
    /// Make entry `display` the one its display shows (others for the same
    /// display stop being visible).
    SetVisible {
        display: usize,
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
            if !crate::workspace::valid_name(workspace) {
                return Err(format!("`{workspace}` is not a valid workspace name"));
            }
            p.displays
                .get_mut(*display)
                .ok_or("no such display")?
                .workspace = workspace.trim().to_string();
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
            if !crate::workspace::valid_name(workspace) {
                return Err(format!("`{workspace}` is not a valid workspace name"));
            }
            if p.displays
                .iter()
                .any(|d| d.display == *display_ref && d.workspace == *workspace)
            {
                return Err(format!(
                    "workspace {workspace} is already in this profile for that display"
                ));
            }
            p.displays.push(ProfileDisplay {
                display: display_ref.clone(),
                workspace: workspace.clone(),
                tree: SerNode::empty(),
                visible: false,
            });
            return Ok(p);
        }
        ProfileEdit::SetVisible { display } => {
            let r = p
                .displays
                .get(*display)
                .ok_or("no such display")?
                .display
                .clone();
            for (i, d) in p.displays.iter_mut().enumerate() {
                if d.display == r {
                    d.visible = i == *display;
                }
            }
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
            when: None,
            displays: vec![ProfileDisplay {
                display: "left".into(),
                workspace: "1".into(),
                tree: SerNode::empty(),
                visible: false,
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

    fn display(id: &str, name: &str, x: f64, main: bool) -> Display {
        Display {
            id: id.into(),
            name: name.into(),
            frame: crate::geometry::Rect::new(x, 0.0, 1000.0, 800.0),
            visible_frame: crate::geometry::Rect::new(x, 0.0, 1000.0, 800.0),
            is_main: main,
            scale: 1.0,
            position_rank: 0,
        }
    }

    fn desk(ds: Vec<Display>) -> Vec<Display> {
        let mut ds = ds;
        crate::display::rank_displays(&mut ds);
        ds
    }

    fn with_when(name: &str, displays: &[&str], exact: bool) -> Profile {
        Profile {
            name: name.into(),
            when: Some(ProfileCondition {
                displays: displays.iter().map(|s| s.to_string()).collect(),
                exact,
            }),
            ..base()
        }
    }

    #[test]
    fn when_matches_displays() {
        let cfg = Config::parse(
            "[[display]]\nname = \"office-left\"\nuuid = \"AAAAAAAA-0000-0000-0000-000000000001\"\n",
        )
        .unwrap();
        let a = "AAAAAAAA-0000-0000-0000-000000000001";
        let b = "BBBBBBBB-0000-0000-0000-000000000002";
        let laptop = display("LAPTOP", "Built-in Retina Display", 0.0, true);
        let office = desk(vec![
            display(a, "Dell U2720Q", -1000.0, false),
            laptop.clone(),
            display(b, "Dell U2720Q", 1000.0, false),
        ]);
        let alone = desk(vec![laptop.clone()]);

        let train = with_when("train", &["name:built-in"], true);
        let work = with_when(
            "work",
            &["office-left", &format!("uuid:{b}"), "uuid:laptop"],
            true,
        );
        let any_dell = with_when("dell", &[&format!("uuid:{a}")], false);
        let all = vec![any_dell.clone(), train.clone(), work.clone(), base()];

        assert!(train.when.as_ref().unwrap().score(&alone, &cfg).is_some());
        assert!(
            train.when.as_ref().unwrap().score(&office, &cfg).is_none(),
            "exact: other displays connected"
        );
        assert!(work.when.as_ref().unwrap().score(&alone, &cfg).is_none());
        let pick = |ds: &[Display], active| {
            pick_for_displays(&all, ds, &cfg, active).map(|p| p.name.as_str())
        };
        assert_eq!(pick(&office, None), Some("work"), "exact beats partial");
        assert_eq!(pick(&alone, None), Some("train"));
        assert_eq!(pick(&desk(vec![laptop.clone()])[..0], None), None);
        // Only the partial match applies with one office monitor unplugged.
        let half = desk(vec![display(a, "Dell", -1000.0, false), laptop.clone()]);
        assert_eq!(pick(&half, None), Some("dell"));

        // Two refs resolving to the same display never match.
        let twice = with_when("twice", &["name:dell", "name:dell"], false);
        assert!(twice.when.as_ref().unwrap().score(&office, &cfg).is_none());

        // Ties: the active profile, then the first name.
        let t1 = with_when("b-train", &["uuid:LAPTOP"], true);
        let t2 = with_when("a-train", &["uuid:LAPTOP"], true);
        let tied = vec![t1, t2];
        assert_eq!(
            pick_for_displays(&tied, &alone, &cfg, None).map(|p| p.name.as_str()),
            Some("a-train")
        );
        assert_eq!(
            pick_for_displays(&tied, &alone, &cfg, Some("b-train")).map(|p| p.name.as_str()),
            Some("b-train")
        );

        // Written for the connected displays; round-trips through TOML.
        let here = Profile {
            when: Some(ProfileCondition::for_displays(&office)),
            ..base()
        };
        assert_eq!(
            pick_for_displays(std::slice::from_ref(&here), &office, &cfg, None),
            Some(&here)
        );
        assert!(here.when.as_ref().unwrap().score(&half, &cfg).is_none());
        let text = here.to_toml().unwrap();
        assert!(text.contains("[when]") && !text.contains("exact"), "{text}");
        assert_eq!(Profile::from_toml(&text).unwrap(), here);
        assert_eq!(
            topology_key(&office),
            topology_key(&office.iter().rev().cloned().collect::<Vec<_>>())
        );
    }
}
