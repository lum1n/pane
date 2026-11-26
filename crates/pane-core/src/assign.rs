//! Which display each workspace is on, and what each display shows.
//!
//! Workspaces belong to a home display. [`assign`] is total and
//! deterministic:
//! - a workspace is on its home display when that is connected;
//! - otherwise it is a guest on the connected display nearest to where its
//!   home last was, hidden, without changing what that display shows;
//! - a newly connected display that owns nothing and sits where a missing
//!   display used to be adopts that display's workspaces (a replaced
//!   monitor, or the same one with a new UUID);
//! - every connected display shows exactly one workspace: the one it showed
//!   before, else the one it showed last time it was connected, else its
//!   first own workspace, else a new "1".

use std::collections::{BTreeMap, BTreeSet};

use crate::display::Display;
use crate::geometry::Rect;
use crate::workspace::{name_order, ws_id};
use crate::{DisplayId, WorkspaceId};

#[derive(Clone, Debug)]
pub struct WsInput {
    pub id: WorkspaceId,
    pub home: DisplayId,
    pub name: String,
}

pub struct AssignInput<'a> {
    /// Connected displays.
    pub displays: &'a [Display],
    pub workspaces: &'a [WsInput],
    pub prev_visible: &'a BTreeMap<DisplayId, WorkspaceId>,
    /// Last workspace shown on each display, including disconnected ones.
    pub remembered_visible: &'a BTreeMap<DisplayId, WorkspaceId>,
    /// Last known frame of every display ever seen.
    pub last_frames: &'a BTreeMap<DisplayId, Rect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assignment {
    pub ws_display: BTreeMap<WorkspaceId, DisplayId>,
    pub visible: BTreeMap<DisplayId, WorkspaceId>,
    /// Workspaces whose home moves to another display: (old home, new home).
    pub rehome: Vec<(DisplayId, DisplayId)>,
    /// Displays that need a workspace "1" created (id, display).
    pub created: Vec<(WorkspaceId, DisplayId)>,
}

fn dist2(a: &Rect, b: &Rect) -> f64 {
    let (ca, cb) = (a.center(), b.center());
    (ca.x - cb.x).powi(2) + (ca.y - cb.y).powi(2)
}

pub fn assign(input: &AssignInput) -> Assignment {
    let mut out = Assignment::default();
    if input.displays.is_empty() {
        return out;
    }
    let present: BTreeSet<&str> = input.displays.iter().map(|d| d.id.as_str()).collect();
    let main = input
        .displays
        .iter()
        .find(|d| d.is_main)
        .unwrap_or(&input.displays[0]);

    // Adoption: a connected display without workspaces where a missing one was.
    let mut homes: BTreeSet<String> = input.workspaces.iter().map(|w| w.home.clone()).collect();
    let missing: Vec<String> = homes
        .iter()
        .filter(|h| !present.contains(h.as_str()))
        .cloned()
        .collect();
    let mut rehome: BTreeMap<String, String> = BTreeMap::new();
    for h in &missing {
        let Some(old) = input.last_frames.get(h) else {
            continue;
        };
        let adopter = input
            .displays
            .iter()
            .filter(|d| !homes.contains(&d.id) && d.frame.contains(old.center()))
            .min_by(|a, b| dist2(&a.frame, old).total_cmp(&dist2(&b.frame, old)));
        if let Some(d) = adopter {
            rehome.insert(h.clone(), d.id.clone());
            homes.insert(d.id.clone());
            out.rehome.push((h.clone(), d.id.clone()));
        }
    }
    let home_of = |w: &WsInput| {
        rehome
            .get(&w.home)
            .cloned()
            .unwrap_or_else(|| w.home.clone())
    };
    // Output is keyed by ids after adoption.
    let new_id = |w: &WsInput| ws_id(&home_of(w), &w.name);

    for w in input.workspaces {
        let home = home_of(w);
        let on = if present.contains(home.as_str()) {
            home
        } else {
            match input.last_frames.get(&home) {
                Some(old) => input
                    .displays
                    .iter()
                    .min_by(|a, b| dist2(&a.frame, old).total_cmp(&dist2(&b.frame, old)))
                    .unwrap_or(main)
                    .id
                    .clone(),
                None => main.id.clone(),
            }
        };
        out.ws_display.insert(new_id(w), on);
    }

    for d in input.displays {
        let renamed = |id: &WorkspaceId| -> WorkspaceId {
            // A remembered id from before adoption refers to the old home.
            let (h, n) = crate::workspace::split_id(id);
            match rehome.get(h) {
                Some(new) if *new == d.id => ws_id(new, n),
                _ => id.clone(),
            }
        };
        let here = |id: &WorkspaceId| out.ws_display.get(id) == Some(&d.id);
        let own = |id: &WorkspaceId| {
            input
                .workspaces
                .iter()
                .any(|w| new_id(w) == *id && home_of(w) == d.id)
        };
        let mut own_ws: Vec<&WsInput> = input
            .workspaces
            .iter()
            .filter(|w| home_of(w) == d.id)
            .collect();
        own_ws.sort_by_key(|w| name_order(&w.name));
        let pick = input
            .prev_visible
            .get(&d.id)
            .map(renamed)
            .filter(|w| here(w))
            .or_else(|| {
                input
                    .remembered_visible
                    .get(&d.id)
                    .map(renamed)
                    .filter(|w| here(w) && own(w))
            })
            .or_else(|| {
                rehome
                    .iter()
                    .find(|(_, new)| **new == d.id)
                    .and_then(|(old, _)| input.remembered_visible.get(old))
                    .map(renamed)
                    .filter(|w| here(w))
            })
            .or_else(|| own_ws.first().map(|w| new_id(w)));
        let ws = match pick {
            Some(w) => w,
            None => {
                let id = ws_id(&d.id, "1");
                out.ws_display.insert(id.clone(), d.id.clone());
                out.created.push((id.clone(), d.id.clone()));
                id
            }
        };
        out.visible.insert(d.id.clone(), ws);
    }
    out
}

/// Lowest positive integer not in `taken`.
pub fn next_free_name(taken: &BTreeSet<WorkspaceId>) -> WorkspaceId {
    (1..)
        .map(|i: u32| WorkspaceId::from(i.to_string()))
        .find(|n| !taken.contains(n))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::testutil::{subset, three};

    struct Desk {
        ws: Vec<WsInput>,
        visible: BTreeMap<DisplayId, WorkspaceId>,
        remembered: BTreeMap<DisplayId, WorkspaceId>,
        frames: BTreeMap<DisplayId, Rect>,
    }

    impl Desk {
        fn new() -> Desk {
            let mut ws = Vec::new();
            for d in ["L", "C", "R"] {
                for n in ["1", "2", "3"] {
                    ws.push(WsInput {
                        id: ws_id(d, n),
                        home: d.into(),
                        name: n.into(),
                    });
                }
            }
            Desk {
                ws,
                visible: BTreeMap::new(),
                remembered: BTreeMap::new(),
                frames: BTreeMap::new(),
            }
        }

        fn connect_displays(&mut self, displays: Vec<Display>) -> Assignment {
            let a = assign(&AssignInput {
                displays: &displays,
                workspaces: &self.ws,
                prev_visible: &self.visible,
                remembered_visible: &self.remembered,
                last_frames: &self.frames,
            });
            for (old, new) in &a.rehome {
                for w in self.ws.iter_mut().filter(|w| &w.home == old) {
                    w.home = new.clone();
                    w.id = ws_id(new, &w.name);
                }
            }
            for (id, d) in &a.created {
                self.ws.push(WsInput {
                    id: id.clone(),
                    home: d.clone(),
                    name: "1".into(),
                });
            }
            self.visible = a.visible.clone();
            for (d, w) in &a.visible {
                self.remembered.insert(d.clone(), w.clone());
            }
            for d in &displays {
                self.frames.insert(d.id.clone(), d.frame);
            }
            a
        }

        fn connect(&mut self, ids: &[&str]) -> Assignment {
            self.connect_displays(subset(ids))
        }

        fn show(&mut self, d: &str, n: &str) {
            self.visible.insert(d.into(), ws_id(d, n));
            self.remembered.insert(d.into(), ws_id(d, n));
        }
    }

    #[test]
    fn each_display_shows_its_own() {
        let mut desk = Desk::new();
        let a = desk.connect(&["L", "C", "R"]);
        assert_eq!(a.visible["L"], ws_id("L", "1"));
        assert_eq!(a.visible["C"], ws_id("C", "1"));
        assert_eq!(a.visible["R"], ws_id("R", "1"));
        assert_eq!(a.ws_display[&ws_id("R", "3")], "R");
    }

    #[test]
    fn unplugged_display_is_hosted_by_its_neighbour_and_returns() {
        let mut desk = Desk::new();
        desk.connect(&["L", "C", "R"]);
        desk.show("C", "2");
        desk.show("R", "3");
        let a = desk.connect(&["L", "C"]);
        assert_eq!(
            a.ws_display[&ws_id("R", "3")],
            "C",
            "nearest display hosts R's workspaces"
        );
        assert_eq!(a.visible["C"], ws_id("C", "2"), "host keeps what it showed");
        let a = desk.connect(&["L", "C", "R"]);
        assert_eq!(a.ws_display[&ws_id("R", "1")], "R");
        assert_eq!(
            a.visible["R"],
            ws_id("R", "3"),
            "shows what it showed before"
        );
        assert_eq!(a.visible["C"], ws_id("C", "2"));
    }

    #[test]
    fn left_goes_to_its_neighbour_not_the_main() {
        let mut desk = Desk::new();
        desk.connect(&["L", "C", "R"]);
        let a = desk.connect(&["C", "R"]);
        assert_eq!(a.ws_display[&ws_id("L", "1")], "C");
        let a = desk.connect(&["R"]);
        assert_eq!(a.ws_display[&ws_id("L", "1")], "R");
        assert_eq!(a.visible["R"], ws_id("R", "1"));
    }

    #[test]
    fn every_reconnect_order_is_total() {
        let all = ["L", "C", "R"];
        let subsets: Vec<Vec<&str>> = vec![
            vec!["L"],
            vec!["C"],
            vec!["R"],
            vec!["L", "C"],
            vec!["C", "R"],
            vec!["L", "R"],
            all.to_vec(),
        ];
        for a in &subsets {
            for b in &subsets {
                let mut desk = Desk::new();
                desk.connect(&all);
                desk.connect(a);
                let r = desk.connect(b);
                for d in b {
                    let ws = &r.visible[*d];
                    assert_eq!(r.ws_display[ws], *d, "{a:?} -> {b:?}: {d}");
                }
                for w in &desk.ws {
                    assert!(
                        b.contains(&r.ws_display[&w.id].as_str()),
                        "{a:?} -> {b:?}: {} lost",
                        w.id
                    );
                }
                let r = desk.connect(&all);
                for d in all {
                    assert_eq!(r.ws_display[&ws_id(d, "2")], d, "{a:?} -> {b:?} -> all");
                }
            }
        }
    }

    #[test]
    fn replaced_display_in_same_position_adopts() {
        let mut desk = Desk::new();
        desk.connect(&["L", "C", "R"]);
        desk.show("R", "2");
        desk.connect(&["L", "C"]);
        let mut displays = three();
        displays[2].id = "R2".into();
        let a = desk.connect_displays(displays);
        assert_eq!(a.rehome, vec![("R".to_string(), "R2".to_string())]);
        assert_eq!(a.visible["R2"], ws_id("R2", "2"));
        assert!(desk.ws.iter().any(|w| w.id == ws_id("R2", "3")));
    }

    #[test]
    fn new_display_gets_a_workspace() {
        let mut desk = Desk::new();
        desk.ws.retain(|w| w.home == "C");
        let a = desk.connect(&["L", "C", "R"]);
        assert_eq!(a.created.len(), 2);
        assert_eq!(a.visible["L"], ws_id("L", "1"));
    }
}
