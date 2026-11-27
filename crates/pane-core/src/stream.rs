//! Events for `pane subscribe`, found by diffing consecutive [`Snapshot`]s.
//!
//! Wire format: one JSON object per line, tagged by `event`. Diffing against
//! `None` gives the current state as one event of each kind, so a new
//! subscriber can draw at once.

use serde::{Deserialize, Serialize};

use crate::snapshot::{SnapWorkspace, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Focus,
    WorkspaceShown,
    Workspaces,
    Displays,
    Profile,
    /// The whole snapshot whenever anything in it changes. Only sent when
    /// asked for by name.
    Snapshot,
}

impl EventKind {
    pub const ALL: [EventKind; 6] = [
        EventKind::Focus,
        EventKind::WorkspaceShown,
        EventKind::Workspaces,
        EventKind::Displays,
        EventKind::Profile,
        EventKind::Snapshot,
    ];

    /// Whether a subscriber asking for `kinds` gets `k`. An empty list means
    /// every kind except `snapshot`.
    pub fn wants(kinds: &[EventKind], k: EventKind) -> bool {
        if kinds.is_empty() {
            k != EventKind::Snapshot
        } else {
            kinds.contains(&k)
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            EventKind::Focus => "focus",
            EventKind::WorkspaceShown => "workspace-shown",
            EventKind::Workspaces => "workspaces",
            EventKind::Displays => "displays",
            EventKind::Profile => "profile",
            EventKind::Snapshot => "snapshot",
        }
    }

    pub fn parse(s: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// A display as `displays` events describe it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamDisplay {
    pub id: String,
    pub name: String,
    pub is_main: bool,
    pub rank: usize,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum StreamEvent {
    /// The focused window, its title, or its display/workspace changed.
    /// `window` is null when the desktop has focus.
    Focus {
        window: Option<u32>,
        app: Option<String>,
        title: Option<String>,
        display: Option<String>,
        workspace: Option<String>,
    },
    /// A display shows another workspace (or appeared).
    WorkspaceShown {
        display: String,
        display_name: String,
        workspace: Option<String>,
        /// The workspace's `[workspace_names]` name, if it has one.
        #[serde(default)]
        workspace_title: Option<String>,
        previous: Option<String>,
    },
    Workspaces {
        workspaces: Vec<SnapWorkspace>,
    },
    Displays {
        displays: Vec<StreamDisplay>,
        focused: Option<String>,
    },
    Profile {
        active: Option<String>,
    },
    Snapshot {
        snapshot: Box<Snapshot>,
    },
}

impl StreamEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            StreamEvent::Focus { .. } => EventKind::Focus,
            StreamEvent::WorkspaceShown { .. } => EventKind::WorkspaceShown,
            StreamEvent::Workspaces { .. } => EventKind::Workspaces,
            StreamEvent::Displays { .. } => EventKind::Displays,
            StreamEvent::Profile { .. } => EventKind::Profile,
            StreamEvent::Snapshot { .. } => EventKind::Snapshot,
        }
    }
}

/// What changed from `prev` to `next`, limited to `kinds`
/// ([`EventKind::wants`]). `prev: None` gives the current state.
pub fn diff(prev: Option<&Snapshot>, next: &Snapshot, kinds: &[EventKind]) -> Vec<StreamEvent> {
    let want = |k| EventKind::wants(kinds, k);
    let mut out = Vec::new();

    if want(EventKind::Displays) {
        let ev = displays(next);
        if prev.map(displays).as_ref() != Some(&ev) {
            out.push(ev);
        }
    }
    if want(EventKind::WorkspaceShown) {
        for d in &next.displays {
            let before = prev.and_then(|p| p.displays.iter().find(|x| x.id == d.id));
            match before {
                Some(b)
                    if b.visible_workspace == d.visible_workspace
                        && b.visible_workspace_title == d.visible_workspace_title => {}
                _ => out.push(StreamEvent::WorkspaceShown {
                    display: d.id.clone(),
                    display_name: d.name.clone(),
                    workspace: d.visible_workspace.clone(),
                    workspace_title: d.visible_workspace_title.clone(),
                    previous: before.and_then(|b| b.visible_workspace.clone()),
                }),
            }
        }
    }
    if want(EventKind::Workspaces) && prev.map(|p| &p.workspaces) != Some(&next.workspaces) {
        out.push(StreamEvent::Workspaces {
            workspaces: next.workspaces.clone(),
        });
    }
    if want(EventKind::Focus) {
        let ev = focus(next);
        if prev.map(focus).as_ref() != Some(&ev) {
            out.push(ev);
        }
    }
    if want(EventKind::Profile) && prev.map(|p| &p.active_profile) != Some(&next.active_profile) {
        out.push(StreamEvent::Profile {
            active: next.active_profile.clone(),
        });
    }
    if want(EventKind::Snapshot) && prev != Some(next) {
        out.push(StreamEvent::Snapshot {
            snapshot: Box::new(next.clone()),
        });
    }
    out
}

fn displays(s: &Snapshot) -> StreamEvent {
    StreamEvent::Displays {
        displays: s
            .displays
            .iter()
            .map(|d| StreamDisplay {
                id: d.id.clone(),
                name: d.name.clone(),
                is_main: d.is_main,
                rank: d.rank,
                focused: d.focused,
            })
            .collect(),
        focused: s.focused_display.clone(),
    }
}

fn focus(s: &Snapshot) -> StreamEvent {
    let tile = s.displays.iter().find_map(|d| {
        d.tiles
            .iter()
            .find(|t| t.focused && t.wid.is_some())
            .map(|t| (d, t))
    });
    match tile {
        Some((d, t)) => StreamEvent::Focus {
            window: t.wid,
            app: Some(t.bundle_id.clone()),
            title: Some(t.title.clone()),
            display: Some(d.id.clone()),
            workspace: d.visible_workspace.clone(),
        },
        None => {
            let d = s
                .focused_display
                .as_ref()
                .and_then(|id| s.displays.iter().find(|d| &d.id == id));
            StreamEvent::Focus {
                window: None,
                app: None,
                title: None,
                display: s.focused_display.clone(),
                workspace: d.and_then(|d| d.visible_workspace.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::snapshot::{SnapDisplay, SnapTile};

    fn tile(wid: u32, app: &str, title: &str, focused: bool) -> SnapTile {
        SnapTile {
            wid: Some(wid),
            bundle_id: app.into(),
            title: title.into(),
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            focused,
            floating: false,
            pending: false,
        }
    }

    fn display(id: &str, ws: &str, focused: bool, tiles: Vec<SnapTile>) -> SnapDisplay {
        SnapDisplay {
            id: id.into(),
            name: format!("Screen {id}"),
            frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
            visible_frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
            is_main: id == "A",
            rank: 0,
            visible_workspace: Some(ws.into()),
            visible_workspace_title: None,
            focused,
            tiles,
        }
    }

    fn workspace(display: &str, name: &str, visible: bool) -> SnapWorkspace {
        SnapWorkspace {
            id: format!("{display}:{name}"),
            name: name.into(),
            display: display.into(),
            home: display.into(),
            guest: false,
            label: name.into(),
            title: None,
            target: format!("uuid:{display}:{name}"),
            visible,
            windows: 1,
        }
    }

    /// Two displays; window 1 (Safari) focused on A's workspace 1.
    fn base() -> Snapshot {
        Snapshot {
            displays: vec![
                display(
                    "A",
                    "1",
                    true,
                    vec![
                        tile(1, "com.apple.Safari", "Home", true),
                        tile(2, "com.apple.Terminal", "zsh", false),
                    ],
                ),
                display(
                    "B",
                    "1",
                    false,
                    vec![tile(3, "com.apple.Mail", "Inbox", false)],
                ),
            ],
            workspaces: vec![workspace("A", "1", true), workspace("B", "1", true)],
            focused_display: Some("A".into()),
            focused_window: Some(1),
            active_profile: Some("Office".into()),
            ..Snapshot::default()
        }
    }

    fn kinds(evs: &[StreamEvent]) -> Vec<EventKind> {
        evs.iter().map(StreamEvent::kind).collect()
    }

    #[test]
    fn initial_state_emits_each_kind() {
        let s = base();
        let evs = diff(None, &s, &[]);
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::Displays,
                EventKind::WorkspaceShown,
                EventKind::WorkspaceShown,
                EventKind::Workspaces,
                EventKind::Focus,
                EventKind::Profile,
            ]
        );
        assert_eq!(
            evs[4],
            StreamEvent::Focus {
                window: Some(1),
                app: Some("com.apple.Safari".into()),
                title: Some("Home".into()),
                display: Some("A".into()),
                workspace: Some("1".into()),
            }
        );
        assert_eq!(
            evs[1],
            StreamEvent::WorkspaceShown {
                display: "A".into(),
                display_name: "Screen A".into(),
                workspace: Some("1".into()),
                workspace_title: None,
                previous: None,
            }
        );
        // Asking for the snapshot gets it too.
        let evs = diff(None, &s, &[EventKind::Snapshot]);
        assert_eq!(kinds(&evs), vec![EventKind::Snapshot]);
    }

    #[test]
    fn unchanged_snapshot_emits_nothing() {
        let s = base();
        assert!(diff(Some(&s), &s, &EventKind::ALL).is_empty());
    }

    #[test]
    fn only_focus_changed() {
        let a = base();
        let mut b = base();
        b.displays[0].tiles[0].focused = false;
        b.displays[0].tiles[1].focused = true;
        b.focused_window = Some(2);
        let evs = diff(Some(&a), &b, &[]);
        assert_eq!(
            evs,
            vec![StreamEvent::Focus {
                window: Some(2),
                app: Some("com.apple.Terminal".into()),
                title: Some("zsh".into()),
                display: Some("A".into()),
                workspace: Some("1".into()),
            }]
        );
        // The desktop takes focus: window is null, display stays.
        let mut c = b.clone();
        c.displays[0].tiles[1].focused = false;
        c.focused_window = None;
        let evs = diff(Some(&b), &c, &[]);
        assert_eq!(
            evs,
            vec![StreamEvent::Focus {
                window: None,
                app: None,
                title: None,
                display: Some("A".into()),
                workspace: Some("1".into()),
            }]
        );
    }

    #[test]
    fn title_change_is_a_focus_event() {
        let a = base();
        let mut b = base();
        b.displays[0].tiles[0].title = "News".into();
        let evs = diff(Some(&a), &b, &[]);
        assert_eq!(kinds(&evs), vec![EventKind::Focus]);
        // An unfocused window's title is not.
        let mut c = base();
        c.displays[1].tiles[0].title = "Sent".into();
        assert!(diff(Some(&a), &c, &[]).is_empty());
    }

    #[test]
    fn workspace_switch_on_one_display() {
        let a = base();
        let mut b = base();
        b.displays[1].visible_workspace = Some("3".into());
        b.displays[1].tiles.clear();
        b.workspaces[1].visible = false;
        b.workspaces.push(workspace("B", "3", true));
        let evs = diff(Some(&a), &b, &[]);
        assert_eq!(
            evs,
            vec![
                StreamEvent::WorkspaceShown {
                    display: "B".into(),
                    display_name: "Screen B".into(),
                    workspace: Some("3".into()),
                    workspace_title: None,
                    previous: Some("1".into()),
                },
                StreamEvent::Workspaces {
                    workspaces: b.workspaces.clone()
                },
            ]
        );
    }

    #[test]
    fn display_unplugged() {
        let a = base();
        let mut b = base();
        b.displays.pop();
        b.workspaces[1].display = "A".into();
        b.workspaces[1].guest = true;
        b.workspaces[1].visible = false;
        let evs = diff(Some(&a), &b, &[]);
        assert_eq!(
            kinds(&evs),
            vec![EventKind::Displays, EventKind::Workspaces]
        );
        let StreamEvent::Displays { displays, focused } = &evs[0] else {
            panic!()
        };
        assert_eq!(displays.len(), 1);
        assert_eq!(focused.as_deref(), Some("A"));
    }

    #[test]
    fn workspace_renamed() {
        let a = base();
        let mut b = base();
        b.displays[0].visible_workspace_title = Some("web".into());
        b.workspaces[0].title = Some("web".into());
        let evs = diff(Some(&a), &b, &[]);
        assert_eq!(
            kinds(&evs),
            vec![EventKind::WorkspaceShown, EventKind::Workspaces]
        );
        let StreamEvent::WorkspaceShown {
            workspace_title, ..
        } = &evs[0]
        else {
            panic!()
        };
        assert_eq!(workspace_title.as_deref(), Some("web"));
    }

    #[test]
    fn profile_changed() {
        let a = base();
        let mut b = base();
        b.active_profile = Some("Home".into());
        assert_eq!(
            diff(Some(&a), &b, &[]),
            vec![StreamEvent::Profile {
                active: Some("Home".into())
            }]
        );
    }

    #[test]
    fn filtering_and_wire_format() {
        let a = base();
        let mut b = base();
        b.active_profile = None;
        b.displays[0].tiles[0].title = "News".into();
        assert_eq!(
            kinds(&diff(Some(&a), &b, &[EventKind::Profile])),
            vec![EventKind::Profile]
        );
        assert_eq!(
            kinds(&diff(
                Some(&a),
                &b,
                &[EventKind::Focus, EventKind::Snapshot]
            )),
            vec![EventKind::Focus, EventKind::Snapshot]
        );
        assert!(diff(Some(&a), &b, &[EventKind::Displays]).is_empty());

        let j = serde_json::to_string(&StreamEvent::Profile { active: None }).unwrap();
        assert_eq!(j, r#"{"event":"profile","active":null}"#);
        let k: Vec<EventKind> = serde_json::from_str(r#"["workspace-shown","focus"]"#).unwrap();
        assert_eq!(k, vec![EventKind::WorkspaceShown, EventKind::Focus]);
        for k in EventKind::ALL {
            assert_eq!(EventKind::parse(k.name()), Some(k));
            assert_eq!(
                serde_json::to_string(&k).unwrap(),
                format!("\"{}\"", k.name())
            );
        }
    }
}
