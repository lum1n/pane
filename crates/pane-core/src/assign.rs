//! Deterministic workspace -> display assignment.
//!
//! [`assign`] is total: every workspace lands on a connected display and
//! every connected display shows exactly one workspace (creating one if no
//! workspace wants it).

use std::collections::{BTreeMap, BTreeSet};

use crate::display::{Display, DisplayRef};
use crate::{DisplayId, WorkspaceId};

#[derive(Clone, Debug)]
pub struct WsInput {
    pub id: WorkspaceId,
    pub prefs: Vec<DisplayRef>,
    /// Display it is on now, if any.
    pub current: Option<DisplayId>,
    /// Display it was evicted from by a disconnect.
    pub last_display: Option<DisplayId>,
}

pub struct AssignInput<'a> {
    /// Connected displays, ranked.
    pub displays: &'a [Display],
    /// Displays connected before this change.
    pub prev_displays: &'a BTreeSet<DisplayId>,
    pub workspaces: &'a [WsInput],
    pub prev_visible: &'a BTreeMap<DisplayId, WorkspaceId>,
    /// Last workspace shown on each display, including disconnected ones.
    pub remembered_visible: &'a BTreeMap<DisplayId, WorkspaceId>,
    pub return_on_reconnect: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assignment {
    pub ws_display: BTreeMap<WorkspaceId, DisplayId>,
    pub visible: BTreeMap<DisplayId, WorkspaceId>,
    /// Workspaces that lost their display: (workspace, display it was on).
    pub evicted: Vec<(WorkspaceId, DisplayId)>,
    /// Workspaces created so a display has something to show.
    pub created: Vec<(WorkspaceId, DisplayId)>,
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
    let newly = |d: &str| !input.prev_displays.contains(d);

    for ws in input.workspaces {
        let target = ws
            .prefs
            .iter()
            .find_map(|p| p.resolve(input.displays))
            .unwrap_or(main)
            .id
            .clone();
        let chosen = match &ws.current {
            Some(cur) if present.contains(cur.as_str()) => match &ws.last_display {
                Some(ld)
                    if input.return_on_reconnect && ld != cur && present.contains(ld.as_str()) =>
                {
                    ld.clone()
                }
                _ if input.return_on_reconnect && &target != cur && newly(&target) => target,
                _ => cur.clone(),
            },
            Some(cur) => {
                out.evicted.push((ws.id.clone(), cur.clone()));
                match &ws.last_display {
                    Some(ld) if present.contains(ld.as_str()) => ld.clone(),
                    _ => target,
                }
            }
            None => match &ws.last_display {
                Some(ld) if input.return_on_reconnect && present.contains(ld.as_str()) => {
                    ld.clone()
                }
                _ => target,
            },
        };
        out.ws_display.insert(ws.id.clone(), chosen);
    }

    let mut taken: BTreeSet<WorkspaceId> = input.workspaces.iter().map(|w| w.id.clone()).collect();
    for d in input.displays {
        let on_d: Vec<&WorkspaceId> = input
            .workspaces
            .iter()
            .map(|w| &w.id)
            .filter(|id| out.ws_display.get(*id) == Some(&d.id))
            .collect();
        let pick = [
            input.prev_visible.get(&d.id),
            input.remembered_visible.get(&d.id),
        ]
        .into_iter()
        .flatten()
        .find(|w| on_d.contains(w))
        .cloned()
        .or_else(|| on_d.first().map(|w| (*w).clone()));
        let ws = match pick {
            Some(w) => w,
            None => {
                let name = next_free_name(&taken);
                taken.insert(name.clone());
                out.ws_display.insert(name.clone(), d.id.clone());
                out.created.push((name.clone(), d.id.clone()));
                name
            }
        };
        out.visible.insert(d.id.clone(), ws);
    }
    out
}

/// Lowest positive integer not used as a workspace name.
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
    use crate::display::Position;

    struct Desk {
        ws: Vec<WsInput>,
        visible: BTreeMap<DisplayId, WorkspaceId>,
        remembered: BTreeMap<DisplayId, WorkspaceId>,
        displays: BTreeSet<DisplayId>,
    }

    impl Desk {
        fn new() -> Desk {
            let ws = (1..=9)
                .map(|i| WsInput {
                    id: i.to_string().into(),
                    prefs: vec![
                        DisplayRef::Position(match i {
                            1..=3 => Position::Left,
                            4..=6 => Position::Center,
                            _ => Position::Right,
                        }),
                        DisplayRef::Main,
                    ],
                    current: None,
                    last_display: None,
                })
                .collect();
            Desk {
                ws,
                visible: BTreeMap::new(),
                remembered: BTreeMap::new(),
                displays: BTreeSet::new(),
            }
        }

        /// Run an assignment the way the reducer does and apply the result.
        fn connect(&mut self, ids: &[&str]) -> Assignment {
            let displays = subset(ids);
            let a = assign(&AssignInput {
                displays: &displays,
                prev_displays: &self.displays,
                workspaces: &self.ws,
                prev_visible: &self.visible,
                remembered_visible: &self.remembered,
                return_on_reconnect: true,
            });
            for (w, d) in &a.evicted {
                let x = self.ws.iter_mut().find(|x| &x.id == w).unwrap();
                // Keep the original home through cascaded disconnects.
                x.last_display.get_or_insert(d.clone());
            }
            for w in &mut self.ws {
                w.current = a.ws_display.get(&w.id).cloned();
                if w.current == w.last_display {
                    w.last_display = None;
                }
            }
            for (w, d) in &a.created {
                self.ws.push(WsInput {
                    id: w.clone(),
                    prefs: vec![],
                    current: Some(d.clone()),
                    last_display: None,
                });
            }
            self.visible = a.visible.clone();
            for (d, w) in &a.visible {
                self.remembered.insert(d.clone(), w.clone());
            }
            self.displays = displays.iter().map(|d| d.id.clone()).collect();
            a
        }

        fn on(&self, ws: &str) -> String {
            self.ws
                .iter()
                .find(|w| w.id == ws)
                .unwrap()
                .current
                .clone()
                .unwrap()
        }

        fn show(&mut self, ws: &str) {
            let d = self.on(ws);
            self.visible.insert(d.clone(), ws.into());
            self.remembered.insert(d, ws.into());
        }
    }

    #[test]
    fn three_displays_initial() {
        let mut desk = Desk::new();
        let a = desk.connect(&["L", "C", "R"]);
        assert_eq!(desk.on("1"), "L");
        assert_eq!(desk.on("5"), "C");
        assert_eq!(desk.on("9"), "R");
        assert_eq!(a.visible["L"], "1");
        assert_eq!(a.visible["C"], "4");
        assert_eq!(a.visible["R"], "7");
        assert!(a.created.is_empty());
    }

    #[test]
    fn unplug_and_replug_right() {
        let mut desk = Desk::new();
        desk.connect(&["L", "C", "R"]);
        desk.show("5");
        desk.show("8");
        let a = desk.connect(&["L", "C"]);
        // 7-9 fall back to "right" == C (rightmost of two); centre keeps showing 5.
        assert_eq!(desk.on("8"), "C");
        assert_eq!(a.visible["C"], "5");
        assert_eq!(a.visible["L"], "1");
        let a = desk.connect(&["L", "C", "R"]);
        assert_eq!(desk.on("7"), "R");
        assert_eq!(desk.on("8"), "R");
        assert_eq!(a.visible["R"], "8", "previously visible workspace returns");
        assert_eq!(a.visible["C"], "5");
    }

    #[test]
    fn unplug_left_then_two() {
        let mut desk = Desk::new();
        desk.connect(&["L", "C", "R"]);
        desk.show("2");
        let a = desk.connect(&["C", "R"]);
        assert_eq!(desk.on("2"), "C");
        assert_eq!(a.visible["C"], "4");
        let a = desk.connect(&["C"]);
        for i in 1..=9 {
            assert_eq!(desk.on(&i.to_string()), "C");
        }
        assert_eq!(a.visible["C"], "4");
        let a = desk.connect(&["L", "C", "R"]);
        assert_eq!(desk.on("2"), "L");
        assert_eq!(desk.on("7"), "R");
        assert_eq!(a.visible["L"], "2");
        assert_eq!(a.visible["R"], "7");
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
                    assert!(
                        r.visible.contains_key(*d),
                        "{a:?} -> {b:?}: {d} shows nothing"
                    );
                    let ws = &r.visible[*d];
                    assert_eq!(desk.on(ws), *d);
                }
                for w in &desk.ws {
                    assert!(b.contains(&w.current.as_deref().unwrap()));
                }
                // Back to all three: canonical home displays again.
                desk.connect(&all);
                assert_eq!(desk.on("1"), "L");
                assert_eq!(desk.on("5"), "C");
                assert_eq!(desk.on("9"), "R");
            }
        }
    }

    #[test]
    fn replaced_display_in_same_position() {
        let mut desk = Desk::new();
        desk.connect(&["L", "C", "R"]);
        // R is swapped for a new monitor with another UUID at the same spot.
        let mut displays = three();
        displays[2].id = "R2".into();
        let a = assign(&AssignInput {
            displays: &displays,
            prev_displays: &desk.displays,
            workspaces: &desk.ws,
            prev_visible: &desk.visible,
            remembered_visible: &desk.remembered,
            return_on_reconnect: true,
        });
        assert_eq!(a.ws_display[&WorkspaceId::from("8")], "R2");
        assert_eq!(a.visible["R2"], "7");
    }

    #[test]
    fn extra_display_gets_new_workspace() {
        let mut ws = Desk::new().ws;
        ws.truncate(1);
        ws[0].prefs = vec![DisplayRef::Main];
        let displays = three();
        let a = assign(&AssignInput {
            displays: &displays,
            prev_displays: &BTreeSet::new(),
            workspaces: &ws,
            prev_visible: &BTreeMap::new(),
            remembered_visible: &BTreeMap::new(),
            return_on_reconnect: true,
        });
        assert_eq!(a.visible["C"], "1");
        assert_eq!(a.created.len(), 2);
        assert_eq!(a.visible["L"], "2");
        assert_eq!(a.visible["R"], "3");
    }
}
