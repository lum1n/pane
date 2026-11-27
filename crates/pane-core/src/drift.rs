//! Profile drift: how the live desk differs from a saved profile, and the
//! profile that "Update Profile from Current Desk" would write.
//!
//! Entries are matched by (resolved display, workspace name). Slots match
//! by bundle id only: titles change all the time, and a captured slot has
//! no title unless its app has several windows on that workspace.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::display::Display;
use crate::profile::{Profile, ProfileDisplay};
use crate::tree::SerNode;

/// Shares (weight / sum of weights) closer than this count as equal, so a
/// one-pixel nudge is not drift.
const SHARE_TOLERANCE: f64 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryChange {
    Unchanged,
    /// Same workspace, different apps, arrangement or sizes.
    Changed,
    /// Holds windows on the desk but is not in the profile. Not drift (the
    /// profile does not claim it), but the update adds it.
    Added,
    /// In the profile, but its apps are no longer on the desk.
    Removed,
    /// Its display is not connected, so it cannot be compared. Kept as is.
    NotConnected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryDiff {
    /// Display reference (the profile's wording when it has the entry).
    pub display: String,
    pub workspace: String,
    /// Index into `ProfileDiff.saved.displays`.
    pub saved: Option<usize>,
    /// Index into `ProfileDiff.updated.displays` (none when the update
    /// drops the entry).
    pub updated: Option<usize>,
    /// The workspace as it is on the desk now, if it holds anything.
    pub live: Option<SerNode>,
    pub change: EntryChange,
    /// Bundle ids on the desk but not in the profile (one per extra window).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_apps: Vec<String>,
    /// Bundle ids in the profile but not on the desk.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_apps: Vec<String>,
    /// Same apps, different tree (order, splits, orientation, layout).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub arrangement: bool,
    /// Same tree, different tile sizes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sizes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileDiff {
    pub name: String,
    pub saved: Profile,
    /// The saved profile updated from the desk: what saving would write.
    pub updated: Profile,
    pub entries: Vec<EntryDiff>,
    /// A workspace the profile describes no longer matches the desk.
    pub drifted: bool,
}

/// Compare `saved` with `live` (a capture of the desk, see
/// `reducer::capture`) on the connected `displays`.
pub fn diff(saved: &Profile, live: &Profile, displays: &[Display], config: &Config) -> ProfileDiff {
    let resolve = |r: &str| {
        config
            .display_ref(r)
            .resolve(displays)
            .map(|d| d.id.clone())
    };
    let live_keys: Vec<Option<(String, String)>> = live
        .displays
        .iter()
        .map(|pd| resolve(&pd.display).map(|id| (id, pd.workspace.clone())))
        .collect();
    let mut live_used = vec![false; live.displays.len()];

    let mut updated = Profile {
        displays: Vec::new(),
        ..saved.clone()
    };
    let mut entries = Vec::new();
    for (si, sd) in saved.displays.iter().enumerate() {
        let mut e = EntryDiff {
            display: sd.display.clone(),
            workspace: sd.workspace.clone(),
            saved: Some(si),
            updated: None,
            live: None,
            change: EntryChange::Unchanged,
            added_apps: vec![],
            removed_apps: vec![],
            arrangement: false,
            sizes: false,
        };
        let Some(id) = resolve(&sd.display) else {
            e.change = EntryChange::NotConnected;
            e.updated = Some(push(&mut updated, sd.clone()));
            entries.push(e);
            continue;
        };
        let key = Some((id, sd.workspace.clone()));
        let li = (0..live.displays.len()).find(|&i| !live_used[i] && live_keys[i] == key);
        if let Some(i) = li {
            live_used[i] = true;
        }
        let live_tree = li.map(|i| live.displays[i].tree.clone()).filter(has_slots);
        compare(&sd.tree, live_tree.as_ref(), &mut e);
        e.live = live_tree.clone();
        e.change = match (&live_tree, has_slots(&sd.tree)) {
            _ if !e.differs() => EntryChange::Unchanged,
            (None, true) => EntryChange::Removed,
            _ => EntryChange::Changed,
        };
        match (e.change, live_tree) {
            (EntryChange::Unchanged, _) => e.updated = Some(push(&mut updated, sd.clone())),
            (_, Some(t)) => {
                let tree = keep_slot_details(&t, &sd.tree);
                e.updated = Some(push(&mut updated, ProfileDisplay { tree, ..sd.clone() }));
            }
            // Gone from the desk: keep the entry only if it chooses what
            // its display shows.
            (_, None) if sd.visible => {
                let pd = ProfileDisplay {
                    tree: SerNode::empty(),
                    ..sd.clone()
                };
                e.updated = Some(push(&mut updated, pd));
            }
            (_, None) => {}
        }
        entries.push(e);
    }
    // Workspaces with windows the profile does not mention.
    for (i, ld) in live.displays.iter().enumerate() {
        if live_used[i] || live_keys[i].is_none() || !has_slots(&ld.tree) {
            continue;
        }
        let mut e = EntryDiff {
            display: ld.display.clone(),
            workspace: ld.workspace.clone(),
            saved: None,
            updated: None,
            live: Some(ld.tree.clone()),
            change: EntryChange::Added,
            added_apps: apps(&ld.tree),
            removed_apps: vec![],
            arrangement: false,
            sizes: false,
        };
        let pd = ProfileDisplay {
            visible: false,
            ..ld.clone()
        };
        e.updated = Some(push(&mut updated, pd));
        entries.push(e);
    }
    let drifted = entries
        .iter()
        .any(|e| matches!(e.change, EntryChange::Changed | EntryChange::Removed));
    ProfileDiff {
        name: saved.name.clone(),
        saved: saved.clone(),
        updated,
        entries,
        drifted,
    }
}

impl EntryDiff {
    fn differs(&self) -> bool {
        !self.added_apps.is_empty()
            || !self.removed_apps.is_empty()
            || self.arrangement
            || self.sizes
    }
}

fn push(p: &mut Profile, pd: ProfileDisplay) -> usize {
    p.displays.push(pd);
    p.displays.len() - 1
}

fn compare(saved: &SerNode, live: Option<&SerNode>, e: &mut EntryDiff) {
    let empty = SerNode::empty();
    let live = live.unwrap_or(&empty);
    let mut have = apps(live);
    for b in apps(saved) {
        match have.iter().position(|x| *x == b) {
            Some(i) => {
                have.remove(i);
            }
            None => e.removed_apps.push(b),
        }
    }
    e.added_apps = have;
    if e.added_apps.is_empty() && e.removed_apps.is_empty() {
        let (a, b) = (normalize(saved), normalize(live));
        if !same_shape(&a, &b) {
            e.arrangement = true;
        } else if !same_sizes(&a, &b) {
            e.sizes = true;
        }
    }
}

/// Bundle ids of every slot, in tree order.
fn apps(n: &SerNode) -> Vec<String> {
    let mut out = Vec::new();
    n.for_each_leaf(&mut |_, l| {
        if let SerNode::Slot { bundle_id, .. } = l {
            out.push(bundle_id.clone());
        }
    });
    out
}

fn has_slots(n: &SerNode) -> bool {
    !apps(n).is_empty()
}

/// Drop empty containers and collapse a container whose only child is a
/// container, so equivalent trees compare equal.
fn normalize(n: &SerNode) -> SerNode {
    match n {
        SerNode::Container {
            orientation,
            layout,
            weights,
            children,
        } => {
            let mut ws = Vec::new();
            let mut cs = Vec::new();
            for (i, c) in children.iter().enumerate() {
                let c = normalize(c);
                if matches!(&c, SerNode::Container { children, .. } if children.is_empty()) {
                    continue;
                }
                cs.push(c);
                ws.push(weights.get(i).copied().unwrap_or(1.0));
            }
            if cs.len() == 1 && matches!(cs[0], SerNode::Container { .. }) {
                return cs.pop().unwrap();
            }
            SerNode::Container {
                orientation: *orientation,
                layout: *layout,
                weights: ws,
                children: cs,
            }
        }
        other => other.clone(),
    }
}

fn same_shape(a: &SerNode, b: &SerNode) -> bool {
    match (a, b) {
        (
            SerNode::Container {
                orientation: oa,
                layout: la,
                children: ca,
                ..
            },
            SerNode::Container {
                orientation: ob,
                layout: lb,
                children: cb,
                ..
            },
        ) => {
            // One child: orientation does not show.
            (ca.len() < 2 || oa == ob)
                && la == lb
                && ca.len() == cb.len()
                && ca.iter().zip(cb).all(|(x, y)| same_shape(x, y))
        }
        (SerNode::Slot { bundle_id: x, .. }, SerNode::Slot { bundle_id: y, .. }) => x == y,
        (SerNode::Window { id: x }, SerNode::Window { id: y }) => x == y,
        _ => false,
    }
}

/// Call only on trees with the same shape.
fn same_sizes(a: &SerNode, b: &SerNode) -> bool {
    match (a, b) {
        (
            SerNode::Container {
                weights: wa,
                children: ca,
                ..
            },
            SerNode::Container {
                weights: wb,
                children: cb,
                ..
            },
        ) => {
            let (sa, sb) = (shares(wa, ca.len()), shares(wb, cb.len()));
            sa.iter()
                .zip(&sb)
                .all(|(x, y)| (x - y).abs() <= SHARE_TOLERANCE)
                && ca.iter().zip(cb).all(|(x, y)| same_sizes(x, y))
        }
        _ => true,
    }
}

fn shares(weights: &[f64], n: usize) -> Vec<f64> {
    let w: Vec<f64> = (0..n)
        .map(|i| weights.get(i).copied().unwrap_or(1.0).max(0.0))
        .collect();
    let sum: f64 = w.iter().sum();
    if sum <= 0.0 {
        return vec![1.0 / n.max(1) as f64; n];
    }
    w.into_iter().map(|x| x / sum).collect()
}

/// The live tree, with each slot taking the title regex and launch
/// arguments of a saved slot for the same app (in order), so updating a
/// profile keeps what was set in the editor.
fn keep_slot_details(live: &SerNode, saved: &SerNode) -> SerNode {
    let mut pool: Vec<SerNode> = Vec::new();
    saved.for_each_leaf(&mut |_, l| {
        if matches!(l, SerNode::Slot { .. }) {
            pool.push(l.clone());
        }
    });
    fn walk(n: &SerNode, pool: &mut Vec<SerNode>) -> SerNode {
        match n {
            SerNode::Container {
                orientation,
                layout,
                weights,
                children,
            } => SerNode::Container {
                orientation: *orientation,
                layout: *layout,
                weights: weights.clone(),
                children: children.iter().map(|c| walk(c, pool)).collect(),
            },
            // A captured title (the app has several windows here) only
            // takes a saved slot with that same title: any other could be
            // a sibling window's.
            SerNode::Slot {
                bundle_id, title, ..
            } => {
                match pool.iter().position(
                    |s| matches!(s, SerNode::Slot { bundle_id: b, title: t, .. } if b == bundle_id && (title.is_none() || t == title)),
                ) {
                    Some(i) => pool.remove(i),
                    None => n.clone(),
                }
            }
            other => other.clone(),
        }
    }
    walk(live, &mut pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Orientation, Rect};
    use crate::tree::ContainerLayout;

    fn slot(b: &str) -> SerNode {
        SerNode::Slot {
            bundle_id: b.into(),
            title: None,
            launch_args: vec![],
        }
    }

    fn row(weights: &[f64], children: Vec<SerNode>) -> SerNode {
        SerNode::Container {
            orientation: Orientation::Horizontal,
            layout: ContainerLayout::Tiles,
            weights: weights.to_vec(),
            children,
        }
    }

    fn col(children: Vec<SerNode>) -> SerNode {
        SerNode::Container {
            orientation: Orientation::Vertical,
            layout: ContainerLayout::Tiles,
            weights: vec![1.0; children.len()],
            children,
        }
    }

    fn entry(display: &str, ws: &str, tree: SerNode) -> ProfileDisplay {
        ProfileDisplay {
            display: display.into(),
            workspace: ws.into(),
            tree,
            visible: false,
        }
    }

    fn profile(entries: Vec<ProfileDisplay>) -> Profile {
        Profile {
            name: "work".into(),
            version: 1,
            when: None,
            displays: entries,
        }
    }

    fn displays() -> Vec<Display> {
        let mut ds = vec![
            Display {
                id: "A".into(),
                name: "Left".into(),
                frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
                visible_frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
                is_main: true,
                scale: 1.0,
                position_rank: 0,
            },
            Display {
                id: "B".into(),
                name: "Right".into(),
                frame: Rect::new(1000.0, 0.0, 1000.0, 800.0),
                visible_frame: Rect::new(1000.0, 0.0, 1000.0, 800.0),
                is_main: false,
                scale: 1.0,
                position_rank: 1,
            },
        ];
        crate::display::rank_displays(&mut ds);
        ds
    }

    fn run(saved: &Profile, live: &Profile) -> ProfileDiff {
        diff(saved, live, &displays(), &Config::default())
    }

    #[test]
    fn same_desk_is_not_drift() {
        let saved = profile(vec![entry(
            "uuid:A",
            "1",
            row(&[2.0, 1.0], vec![slot("mail"), slot("safari")]),
        )]);
        // Captured with another wording, a wrapping container and a
        // slightly different split.
        let live = profile(vec![entry(
            "left",
            "1",
            row(
                &[1.0],
                vec![row(&[0.67, 0.33], vec![slot("mail"), slot("safari")])],
            ),
        )]);
        let d = run(&saved, &live);
        assert!(!d.drifted, "{d:#?}");
        assert_eq!(d.entries[0].change, EntryChange::Unchanged);
        assert_eq!(d.updated, saved, "unchanged entries are kept verbatim");
    }

    #[test]
    fn changes_are_classified() {
        let titled = SerNode::Slot {
            bundle_id: "safari".into(),
            title: Some("^Docs".into()),
            launch_args: vec!["--x".into()],
        };
        let saved = profile(vec![
            entry(
                "left",
                "1",
                row(&[1.0, 1.0], vec![slot("mail"), titled.clone()]),
            ),
            ProfileDisplay {
                visible: true,
                ..entry("left", "2", row(&[1.0], vec![slot("notes")]))
            },
            entry(
                "right",
                "1",
                row(&[1.0, 1.0], vec![slot("term"), slot("code")]),
            ),
            entry("right", "3", row(&[1.0, 1.0], vec![slot("a"), slot("b")])),
            entry("uuid:GONE", "1", row(&[1.0], vec![slot("music")])),
        ]);
        let live = profile(vec![
            // Mail replaced by Slack, Safari kept.
            entry(
                "left",
                "1",
                row(&[1.0, 1.0], vec![slot("slack"), slot("safari")]),
            ),
            // Notes quit: left 2 missing entirely.
            // Same apps, now stacked.
            entry("right", "1", col(vec![slot("term"), slot("code")])),
            // Same tree, resized.
            entry("right", "3", row(&[3.0, 1.0], vec![slot("a"), slot("b")])),
            // New workspace with windows; and a visible empty one.
            entry("right", "4", row(&[1.0], vec![slot("zoom")])),
            entry("right", "5", SerNode::empty()),
        ]);
        let d = run(&saved, &live);
        assert!(d.drifted);
        let by = |ws: &str, disp: &str| {
            d.entries
                .iter()
                .find(|e| e.workspace == ws && e.display == disp)
                .unwrap_or_else(|| panic!("no entry {disp} {ws}: {d:#?}"))
        };
        let e = by("1", "left");
        assert_eq!(e.change, EntryChange::Changed);
        assert_eq!(e.added_apps, vec!["slack"]);
        assert_eq!(e.removed_apps, vec!["mail"]);
        // The update keeps Safari's title regex and arguments.
        let tree = &d.updated.displays[e.updated.unwrap()].tree;
        assert_eq!(tree, &row(&[1.0, 1.0], vec![slot("slack"), titled]));

        let e = by("2", "left");
        assert_eq!(e.change, EntryChange::Removed);
        assert_eq!(e.removed_apps, vec!["notes"]);
        // Kept (empty) because it chose what the display shows.
        let pd = &d.updated.displays[e.updated.unwrap()];
        assert!(pd.visible && !has_slots(&pd.tree));

        let e = by("1", "right");
        assert_eq!(
            (e.change, e.arrangement, e.sizes),
            (EntryChange::Changed, true, false)
        );
        let e = by("3", "right");
        assert_eq!(
            (e.change, e.arrangement, e.sizes),
            (EntryChange::Changed, false, true)
        );
        assert_eq!(by("1", "uuid:GONE").change, EntryChange::NotConnected);
        let e = by("4", "right");
        assert_eq!((e.change, e.saved), (EntryChange::Added, None));
        assert!(d.entries.iter().all(|e| e.workspace != "5"));

        // The update: saved order, then additions; the unconnected display kept.
        let names: Vec<(&str, &str)> = d
            .updated
            .displays
            .iter()
            .map(|p| (p.display.as_str(), p.workspace.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("left", "1"),
                ("left", "2"),
                ("right", "1"),
                ("right", "3"),
                ("uuid:GONE", "1"),
                ("right", "4")
            ]
        );
        assert_eq!(d.updated.name, "work");
    }

    #[test]
    fn added_workspace_alone_is_not_drift() {
        let saved = profile(vec![entry("left", "1", row(&[1.0], vec![slot("mail")]))]);
        let live = profile(vec![
            entry("left", "1", row(&[1.0], vec![slot("mail")])),
            entry("left", "2", row(&[1.0], vec![slot("music")])),
        ]);
        let d = run(&saved, &live);
        assert!(!d.drifted);
        assert_eq!(d.entries[1].change, EntryChange::Added);
        assert_eq!(d.updated.displays.len(), 2);
    }

    #[test]
    fn removed_hidden_entry_is_dropped() {
        let saved = profile(vec![
            entry("left", "1", row(&[1.0], vec![slot("mail")])),
            entry("left", "2", row(&[1.0], vec![slot("notes")])),
        ]);
        let live = profile(vec![entry("left", "1", row(&[1.0], vec![slot("mail")]))]);
        let d = run(&saved, &live);
        assert!(d.drifted);
        assert_eq!(d.entries[1].updated, None);
        assert_eq!(d.updated.displays.len(), 1);
    }

    fn one(saved: SerNode, live: SerNode) -> EntryDiff {
        let d = run(
            &profile(vec![entry("left", "1", saved)]),
            &profile(vec![entry("left", "1", live)]),
        );
        d.entries.into_iter().next().unwrap()
    }

    #[test]
    fn duplicate_apps_are_counted_per_window() {
        let e = one(
            row(&[1.0, 1.0], vec![slot("term"), slot("term")]),
            row(&[1.0], vec![slot("term")]),
        );
        assert_eq!(e.change, EntryChange::Changed);
        assert_eq!(
            (e.added_apps, e.removed_apps),
            (vec![], vec!["term".into()])
        );
        let e = one(
            row(&[1.0], vec![slot("term")]),
            row(&[1.0, 1.0], vec![slot("term"), slot("term")]),
        );
        assert_eq!(
            (e.added_apps, e.removed_apps),
            (vec!["term".into()], vec![])
        );
    }

    #[test]
    fn order_and_layout_are_arrangement_but_a_lone_tile_has_no_orientation() {
        let e = one(
            row(&[1.0, 1.0], vec![slot("a"), slot("b")]),
            row(&[1.0, 1.0], vec![slot("b"), slot("a")]),
        );
        assert_eq!((e.change, e.arrangement), (EntryChange::Changed, true));
        let accordion = SerNode::Container {
            orientation: Orientation::Horizontal,
            layout: ContainerLayout::Accordion,
            weights: vec![1.0, 1.0],
            children: vec![slot("a"), slot("b")],
        };
        let e = one(row(&[1.0, 1.0], vec![slot("a"), slot("b")]), accordion);
        assert_eq!((e.change, e.arrangement), (EntryChange::Changed, true));
        // One tile: row or column looks the same.
        let e = one(row(&[1.0], vec![slot("a")]), col(vec![slot("a")]));
        assert_eq!(e.change, EntryChange::Unchanged, "{e:#?}");
        // Empty containers left in the tree do not count either.
        let e = one(
            row(&[1.0, 1.0], vec![slot("a"), slot("b")]),
            row(
                &[1.0, 1.0, 1.0],
                vec![slot("a"), SerNode::empty(), slot("b")],
            ),
        );
        assert_eq!(e.change, EntryChange::Unchanged, "{e:#?}");
    }

    #[test]
    fn small_size_nudges_are_not_drift() {
        let saved = || row(&[1.0, 1.0], vec![slot("a"), slot("b")]);
        let e = one(saved(), row(&[1.02, 0.98], vec![slot("a"), slot("b")]));
        assert_eq!(e.change, EntryChange::Unchanged);
        let e = one(saved(), row(&[1.1, 0.9], vec![slot("a"), slot("b")]));
        assert_eq!((e.change, e.sizes), (EntryChange::Changed, true));
    }

    #[test]
    fn shares_handle_missing_and_bad_weights() {
        assert_eq!(shares(&[3.0, 1.0], 2), vec![0.75, 0.25]);
        assert_eq!(shares(&[2.0], 2), vec![2.0 / 3.0, 1.0 / 3.0]);
        assert_eq!(shares(&[-1.0, 1.0], 2), vec![0.0, 1.0]);
        assert_eq!(shares(&[0.0, 0.0], 2), vec![0.5, 0.5]);
        assert!(shares(&[], 0).is_empty());
    }

    #[test]
    fn update_keeps_slot_details_only_for_matching_titles() {
        let saved_docs = SerNode::Slot {
            bundle_id: "safari".into(),
            title: Some("^Docs".into()),
            launch_args: vec!["--x".into()],
        };
        let inbox = SerNode::Slot {
            bundle_id: "safari".into(),
            title: Some("Inbox".into()),
            launch_args: vec![],
        };
        let saved = row(&[1.0], vec![saved_docs.clone()]);
        // A captured title that differs could be another Safari window.
        let live = row(&[1.0, 1.0], vec![inbox.clone(), slot("safari")]);
        assert_eq!(
            keep_slot_details(&live, &saved),
            row(&[1.0, 1.0], vec![inbox, saved_docs.clone()])
        );
        // Each saved slot is handed out once.
        let live = row(&[1.0, 1.0], vec![slot("safari"), slot("safari")]);
        assert_eq!(
            keep_slot_details(&live, &saved),
            row(&[1.0, 1.0], vec![saved_docs, slot("safari")])
        );
    }
}
