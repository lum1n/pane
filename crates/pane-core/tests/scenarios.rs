//! End-to-end scenarios through `reduce` + `reconcile` with simulated apps.

use std::collections::BTreeMap;

use pane_core::config::Config;
use pane_core::display::{rank_displays, Display};
use pane_core::effect::Effect;
use pane_core::event::{Event, WindowProbe};
use pane_core::geometry::{Direction, Rect, Size};
use pane_core::model::{Model, WindowMode};
use pane_core::proto::{Command, Reply, UiRequest};
use pane_core::reconcile::reconcile;
use pane_core::reducer::{capture, reduce, refresh_placements};
use pane_core::rules::WindowRole;
use pane_core::workspace::Cycle;
use pane_core::{Pid, WindowId};

fn disp(id: &str, x: f64, main: bool) -> Display {
    Display {
        id: id.into(),
        name: format!("Display {id}"),
        frame: Rect::new(x, 0.0, 1920.0, 1080.0),
        visible_frame: Rect::new(x, 30.0, 1920.0, 1050.0),
        is_main: main,
        scale: 2.0,
        position_rank: 0,
    }
}

fn displays(ids: &[&str]) -> Vec<Display> {
    let all = [
        disp("L", -1920.0, false),
        disp("C", 0.0, true),
        disp("R", 1920.0, false),
    ];
    let mut v: Vec<Display> = all
        .into_iter()
        .filter(|d| ids.contains(&d.id.as_str()))
        .collect();
    rank_displays(&mut v);
    v
}

struct Sim {
    m: Model,
    now: u64,
    /// Simulated window frames (what the app really has).
    frames: BTreeMap<WindowId, Rect>,
    min: BTreeMap<WindowId, Size>,
    frozen: Vec<Pid>,
    /// Windows whose app answers but never moves them.
    stuck: Vec<WindowId>,
    notices: Vec<String>,
    launches: Vec<String>,
    sets: usize,
    sets_for: BTreeMap<WindowId, usize>,
}

impl Sim {
    fn new() -> Sim {
        Sim::with(Model::new(Config::default(), 1_000_000))
    }

    fn with(m: Model) -> Sim {
        let mut s = Sim {
            m,
            now: 1_000_000,
            frames: BTreeMap::new(),
            min: BTreeMap::new(),
            frozen: vec![],
            stuck: vec![],
            notices: vec![],
            launches: vec![],
            sets: 0,
            sets_for: BTreeMap::new(),
        };
        s.ev(Event::DisplaysChanged {
            displays: displays(&["L", "C", "R"]),
        });
        s
    }

    /// Feed an event, reconcile, and let the simulated apps apply frames.
    fn ev(&mut self, e: Event) -> Vec<Effect> {
        self.now += 10;
        let mut out = reduce(&mut self.m, e, self.now);
        out.extend(reconcile(&mut self.m));
        let mut all = out.clone();
        for _ in 0..20 {
            let mut follow = Vec::new();
            for f in &out {
                match f {
                    Effect::SetFrame {
                        pid,
                        wid,
                        rect,
                        gen,
                        ..
                    } => {
                        self.sets += 1;
                        *self.sets_for.entry(*wid).or_default() += 1;
                        if self.frozen.contains(pid) {
                            continue;
                        }
                        let min = self.min.get(wid).copied().unwrap_or(Size::ZERO);
                        let actual = if self.stuck.contains(wid) {
                            self.frames[wid]
                        } else {
                            Rect::new(rect.x, rect.y, rect.w.max(min.w), rect.h.max(min.h))
                        };
                        self.frames.insert(*wid, actual);
                        follow.push(Event::FrameApplied {
                            pid: *pid,
                            wid: *wid,
                            gen: *gen,
                            ok: true,
                            actual: Some(actual),
                        });
                    }
                    Effect::UiRequest(UiRequest::Notice { message, .. }) => {
                        self.notices.push(message.clone())
                    }
                    Effect::UiRequest(UiRequest::LaunchApp { bundle_id, .. }) => {
                        self.launches.push(bundle_id.clone())
                    }
                    _ => {}
                }
            }
            if follow.is_empty() {
                break;
            }
            out.clear();
            for e in follow {
                self.now += 1;
                out.extend(reduce(&mut self.m, e, self.now));
            }
            out.extend(reconcile(&mut self.m));
            all.extend(out.clone());
        }
        all
    }

    fn tick(&mut self, ms: u64) {
        self.now += ms;
        self.ev(Event::Tick);
    }

    fn cmd(&mut self, c: Command) -> Reply {
        let fx = self.ev(Event::Command {
            cmd: c,
            token: Some(1),
        });
        fx.into_iter()
            .find_map(|f| match f {
                Effect::Reply { reply, .. } => Some(reply),
                _ => None,
            })
            .unwrap()
    }

    fn launch(&mut self, pid: Pid, bundle: &str) {
        self.ev(Event::AppLaunched {
            pid,
            bundle_id: Some(bundle.into()),
            name: bundle.into(),
            regular: true,
        });
    }

    fn window(&mut self, pid: Pid, wid: u32, title: &str, at: Rect) {
        self.frames.insert(WindowId(wid), at);
        self.ev(Event::WindowsDiscovered {
            pid,
            windows: vec![probe(pid, wid, title, at)],
            initial: false,
        });
    }

    fn frame(&self, wid: u32) -> Rect {
        self.frames[&WindowId(wid)]
    }

    fn on_screen(&self, wid: u32) -> bool {
        let f = self.frame(wid);
        self.m.displays.iter().any(|d| {
            d.visible_frame
                .intersection(&f)
                .is_some_and(|i| i.w > 10.0 && i.h > 10.0)
        })
    }

    fn focus(&mut self, pid: Pid, wid: u32) {
        self.ev(Event::FocusChanged {
            frontmost: true,
            pid,
            wid: Some(WindowId(wid)),
        });
    }
}

fn probe(pid: Pid, wid: u32, title: &str, frame: Rect) -> WindowProbe {
    WindowProbe {
        wid: WindowId(wid),
        pid,
        title: title.into(),
        role: WindowRole::Standard,
        resizable: true,
        has_close_button: true,
        modal: false,
        minimized: false,
        frame,
        document: None,
        min_size: None,
    }
}

const CENTER: Rect = Rect::new(100.0, 100.0, 800.0, 600.0);

#[test]
fn tiles_on_focused_display_and_relayouts_on_close() {
    let mut s = Sim::new();
    s.launch(100, "com.apple.Terminal");
    for i in 1..=4 {
        s.window(100, i, &format!("term {i}"), CENTER);
    }
    // Default config: 4-6 on the centre display, workspace 4 visible.
    let area = Rect::new(6.0, 36.0, 1908.0, 1038.0);
    let total: f64 = (1..=4).map(|i| s.frame(i).w).sum::<f64>() + 3.0 * 6.0;
    assert!(
        (total - area.w).abs() <= 2.0,
        "tiles fill the width: {total}"
    );
    assert_eq!(s.frame(1).x, area.x);
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(4),
    });
    let total: f64 = (1..=3).map(|i| s.frame(i).w).sum::<f64>() + 2.0 * 6.0;
    assert!((total - area.w).abs() <= 2.0);
    // Toggle orientation stacks them.
    s.focus(100, 1);
    s.cmd(Command::ToggleOrientation);
    assert_eq!(s.frame(1).w, area.w);
    assert!(s.frame(2).y > s.frame(1).y);
}

#[test]
fn dialogs_float_and_keep_their_frame() {
    let mut s = Sim::new();
    s.launch(100, "com.apple.TextEdit");
    s.window(100, 1, "doc", CENTER);
    let mut p = probe(100, 2, "Save", Rect::new(500.0, 300.0, 400.0, 200.0));
    p.role = WindowRole::Dialog;
    s.frames.insert(WindowId(2), p.frame);
    s.ev(Event::WindowsDiscovered {
        pid: 100,
        windows: vec![p],
        initial: false,
    });
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Floating);
    assert_eq!(s.frame(2), Rect::new(500.0, 300.0, 400.0, 200.0));
    assert_eq!(s.frame(1).w, 1908.0, "dialog takes no tile");
}

#[test]
fn workspace_switch_parks_and_restores_exact_frames() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    let before = (s.frame(1), s.frame(2));
    let fx = s.cmd(Command::Workspace { name: "2".into() });
    assert!(matches!(fx, Reply::Ok { .. }));
    assert!(!s.on_screen(1) && !s.on_screen(2));
    // Parked at the bottom-right corner of the centre display, size kept.
    assert_eq!(s.frame(1).x, 1919.0);
    assert_eq!(s.frame(1).y, 1079.0);
    assert_eq!(s.frame(1).w, before.0.w);
    assert_eq!(s.m.parked.len(), 2);
    s.cmd(Command::Workspace { name: "1".into() });
    assert_eq!((s.frame(1), s.frame(2)), before);
    assert!(s.m.parked.is_empty());
}

#[test]
fn write_parked_precedes_parking_frames() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    let fx = s.ev(Event::Command {
        cmd: Command::Workspace { name: "3".into() },
        token: None,
    });
    let wp = fx
        .iter()
        .position(|f| matches!(f, Effect::WriteParked { .. }))
        .expect("journal written");
    let set = fx
        .iter()
        .position(|f| {
            matches!(
                f,
                Effect::SetFrame {
                    restore_to: Some(_),
                    ..
                }
            )
        })
        .expect("parked");
    assert!(wp < set);
}

#[test]
fn shutdown_unparks_everything() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    let orig = s.frame(1);
    s.cmd(Command::Workspace { name: "2".into() });
    assert!(!s.on_screen(1));
    s.ev(Event::Shutdown);
    assert_eq!(s.frame(1), orig);
    assert_eq!(s.m.outstanding_sets(), 0);
}

#[test]
fn min_size_is_learned_without_retry_storm() {
    let mut s = Sim::new();
    s.launch(100, "com.apple.systempreferences");
    s.launch(200, "b");
    s.min.insert(WindowId(1), Size::new(1200.0, 500.0));
    s.window(100, 1, "Settings", CENTER);
    s.window(200, 2, "other", CENTER);
    let sets_before = s.sets;
    assert!(s.frame(1).w >= 1200.0);
    let right = s.frame(2);
    assert!(
        right.x >= s.frame(1).max_x(),
        "neighbour moved over: {right:?}"
    );
    for _ in 0..10 {
        s.tick(3000);
    }
    assert_eq!(s.sets, sets_before, "no further sets once settled");
}

#[test]
fn too_many_min_sizes_float_newest() {
    let mut s = Sim::new();
    s.launch(100, "a");
    for i in 1..=3 {
        s.min.insert(WindowId(i), Size::new(800.0, 400.0));
        s.window(100, i, &format!("w{i}"), CENTER);
    }
    assert_eq!(s.m.windows[&WindowId(3)].mode, WindowMode::Floating);
    assert_eq!(s.m.windows[&WindowId(1)].mode, WindowMode::Tiled);
    assert!(s.notices.iter().any(|n| n.contains("Not enough room")));
}

#[test]
fn frozen_app_is_marked_and_others_keep_tiling() {
    let mut s = Sim::new();
    s.launch(100, "frozen");
    s.launch(200, "ok");
    s.window(100, 1, "f", CENTER);
    s.frozen.push(100);
    s.window(200, 2, "o", CENTER);
    s.tick(2500);
    assert!(s.m.apps[&100].unresponsive);
    s.window(200, 3, "o2", CENTER);
    assert!(s.frame(3).w < 1000.0, "new window still tiled");
    // Any event from the app clears it.
    s.frozen.clear();
    s.ev(Event::TitleChanged {
        pid: 100,
        wid: WindowId(1),
        title: "f!".into(),
    });
    assert!(!s.m.apps[&100].unresponsive);
}

#[test]
fn stubborn_window_floats_after_backoff() {
    let mut s = Sim::new();
    s.tick(2000); // past the post-display-change resync pass
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.stuck.push(WindowId(2));
    s.window(100, 2, "two", Rect::new(5.0, 5.0, 300.0, 300.0));
    let sets = s.sets_for[&WindowId(2)];
    for _ in 0..10 {
        s.tick(3000);
    }
    assert!(s.m.windows[&WindowId(2)].stubborn);
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Floating);
    assert_eq!(
        s.sets_for[&WindowId(2)] - sets,
        5,
        "one retry per backoff step"
    );
    assert_eq!(s.frame(1).w, 1908.0);
    assert!(s.notices.iter().any(|n| n.contains("refuses")));
}

#[test]
fn focus_crosses_displays_including_empty_ones() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "left", CENTER);
    s.window(100, 2, "right", CENTER);
    s.focus(100, 2);
    let r = s.cmd(Command::Focus {
        dir: Direction::Right,
    });
    assert!(matches!(r, Reply::Ok { .. }));
    assert_eq!(s.m.focus_display.as_deref(), Some("R"));
    assert_eq!(s.m.focus_window, None);
    // Focusing left from the empty right display lands on the rightmost tile.
    let fx = s.ev(Event::Command {
        cmd: Command::Focus {
            dir: Direction::Left,
        },
        token: None,
    });
    assert!(fx.iter().any(|f| matches!(
        f,
        Effect::Focus {
            wid: WindowId(2),
            ..
        }
    )));
}

#[test]
fn move_window_across_display_edge() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    s.focus(100, 2);
    s.cmd(Command::Move {
        dir: Direction::Right,
    });
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "R/1");
    assert!(s.frame(2).x >= 1920.0, "now on the right display");
    assert_eq!(s.frame(1).w, 1908.0);
}

#[test]
fn unplug_and_replug_keeps_everything_reachable() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "centre", CENTER);
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    assert_eq!(s.m.focus_display.as_deref(), Some("R"));
    s.window(100, 2, "right", CENTER);
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(100, 3, "right2", CENTER);
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "R/2");
    s.ev(Event::DisplaysChanged {
        displays: displays(&["L", "C"]),
    });
    // R's workspaces are guests on C (its neighbour), hidden; C keeps C/1.
    assert_eq!(s.m.visible["C"], "C/1");
    assert_eq!(s.m.ws("R/2").unwrap().display, "C");
    assert!(s.on_screen(1));
    assert!(!s.on_screen(2) && !s.on_screen(3));
    // Reachable by name.
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    let r = s.cmd(Command::Workspace {
        name: "uuid:R:2".into(),
    });
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    assert!(s.on_screen(3));
    // mod-1 on C (still focused) means C's own workspace 1.
    assert_eq!(s.m.focus_display.as_deref(), Some("C"));
    s.ev(Event::Command {
        cmd: Command::Workspace { name: "1".into() },
        token: None,
    });
    assert_eq!(s.m.visible["C"], "C/1");
    s.ev(Event::DisplaysChanged {
        displays: displays(&["L", "C", "R"]),
    });
    assert_eq!(s.m.visible["R"], "R/2", "R shows what it showed before");
    assert_eq!(s.m.visible["C"], "C/1");
    assert!(s.on_screen(3) && s.frame(3).x >= 1920.0);
    assert!(!s.on_screen(2));
}

#[test]
fn move_and_swap_workspaces_between_displays() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "centre", CENTER);
    s.focus(100, 1);
    s.cmd(Command::SwapWorkspaces);
    // Names stay with displays; the window moved to R's visible workspace.
    assert_eq!(s.m.visible["R"], "R/1");
    assert_eq!(s.m.visible["C"], "C/1");
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "R/1");
    assert!(s.frame(1).x >= 1920.0);
    // Move R's workspace to C: C already has a "1", so it becomes C/2.
    s.cmd(Command::MoveWorkspaceToDisplay { which: Cycle::Prev });
    assert_eq!(s.m.visible["C"], "C/2");
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "C/2");
    assert!(s.frame(1).x >= 0.0 && s.frame(1).x < 1920.0);
    assert!(s.m.visible.contains_key("R"));
}

#[test]
fn workspaces_are_per_display() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "c1", CENTER);
    // mod-2 on the centre display switches the centre display only.
    s.cmd(Command::Workspace { name: "2".into() });
    assert_eq!(s.m.visible["C"], "C/2");
    assert_eq!(s.m.visible["L"], "L/1");
    assert_eq!(s.m.visible["R"], "R/1");
    s.window(100, 2, "c2", CENTER);
    // Focus the left display: its "2" is a different workspace.
    s.cmd(Command::FocusDisplay { which: Cycle::Prev });
    assert_eq!(s.m.focus_display.as_deref(), Some("L"));
    s.cmd(Command::Workspace { name: "2".into() });
    assert_eq!(s.m.visible["L"], "L/2");
    assert_eq!(s.m.visible["C"], "C/2");
    assert!(s.on_screen(2));
    // Cycling on the centre display goes through workspaces with windows.
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    s.cmd(Command::CycleWorkspace { which: Cycle::Next });
    assert_eq!(s.m.visible["C"], "C/1");
    s.cmd(Command::CycleWorkspace { which: Cycle::Next });
    assert_eq!(s.m.visible["C"], "C/2");
    // Move the window to centre workspace 3 and back.
    s.focus(100, 2);
    s.cmd(Command::MoveToWorkspace { name: "3".into() });
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "C/3");
    assert!(!s.on_screen(2));
}

#[test]
fn app_relaunch_returns_windows_to_their_tiles() {
    let mut s = Sim::new();
    s.launch(100, "com.apple.Terminal");
    s.launch(200, "com.apple.Safari");
    s.window(100, 1, "term", CENTER);
    s.window(200, 2, "GitHub", CENTER);
    s.focus(200, 2);
    s.window(200, 3, "Docs", CENTER);
    s.focus(200, 3);
    s.cmd(Command::Resize {
        axis: pane_core::Orientation::Horizontal,
        delta: 0.1,
    });
    let before = (s.frame(2), s.frame(3));
    s.ev(Event::AppTerminated { pid: 200 });
    assert_eq!(s.frame(1).w, 1908.0, "terminal takes the space meanwhile");
    // Safari comes back with new ids, windows in the opposite order.
    s.launch(201, "com.apple.Safari");
    s.frames.insert(WindowId(31), CENTER);
    s.frames.insert(WindowId(32), CENTER);
    s.ev(Event::WindowsDiscovered {
        pid: 201,
        windows: vec![
            probe(201, 31, "Docs", CENTER),
            probe(201, 32, "GitHub", CENTER),
        ],
        initial: true,
    });
    assert_eq!(s.frame(32), before.0);
    assert_eq!(s.frame(31), before.1);
}

#[test]
fn pane_restart_restores_layout_from_state() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    s.focus(100, 2);
    s.cmd(Command::Move {
        dir: Direction::Down,
    });
    s.window(100, 3, "three", CENTER);
    let before: Vec<Rect> = (1..=3).map(|i| s.frame(i)).collect();
    refresh_placements(&mut s.m);
    let state = s.m.to_state();
    let json = serde_json::to_string(&state).unwrap();
    let state = serde_json::from_str(&json).unwrap();

    let mut s2 = Sim::with(Model::from_state(Config::default(), &state, s.now));
    s2.frames = s.frames.clone();
    s2.launch(100, "a");
    s2.ev(Event::WindowsDiscovered {
        pid: 100,
        windows: (1..=3)
            .map(|i| probe(100, i, ["one", "two", "three"][i as usize - 1], CENTER))
            .collect(),
        initial: true,
    });
    let after: Vec<Rect> = (1..=3).map(|i| s2.frame(i)).collect();
    assert_eq!(before, after);
}

#[test]
fn profile_capture_and_apply_launches_missing_apps() {
    let mut s = Sim::new();
    s.launch(100, "com.apple.mail");
    s.launch(200, "com.apple.Safari");
    s.window(100, 1, "Inbox", CENTER);
    s.window(200, 2, "Web", CENTER);
    let profile = capture(&s.m, "work");
    assert_eq!(profile.displays.len(), 3);
    assert_eq!(profile.displays[1].display, "center");
    let before = (s.frame(1), s.frame(2));

    // Everything running: nothing launched, nothing closed.
    let r = s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(profile.clone()),
    });
    assert!(matches!(r, Reply::Ok { .. }));
    assert!(s.launches.is_empty());
    assert_eq!((s.frame(1), s.frame(2)), before);

    // Mail quits; applying again launches it into its slot.
    s.ev(Event::AppTerminated { pid: 100 });
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(profile),
    });
    assert_eq!(s.launches, vec!["com.apple.mail".to_string()]);
    assert_eq!(
        s.frame(2),
        before.1,
        "slot keeps its space while Mail launches"
    );
    s.launch(101, "com.apple.mail");
    s.window(101, 11, "Inbox", CENTER);
    assert_eq!(s.frame(11), before.0);
    assert!(s.m.pending_slots.is_empty());
}

#[test]
fn pending_slot_expires() {
    let mut s = Sim::new();
    let mut p = capture(&s.m, "x");
    p.displays[1].tree = pane_core::tree::SerNode::Container {
        orientation: pane_core::Orientation::Horizontal,
        layout: Default::default(),
        weights: vec![1.0],
        children: vec![pane_core::tree::SerNode::Slot {
            bundle_id: "com.nope".into(),
            title: None,
            launch_args: vec![],
        }],
    };
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(p),
    });
    assert_eq!(s.m.pending_slots.len(), 1);
    s.tick(31_000);
    assert!(s.m.pending_slots.is_empty());
    assert!(s.m.slot_frames.is_empty());
}

#[test]
fn crash_leftovers_are_restored_for_ignored_and_counted() {
    let mut s = Sim::new();
    s.ev(Event::RestoreParked {
        entries: vec![pane_core::state::ParkedEntry {
            pid: 100,
            wid: WindowId(1),
            bundle_id: "a".into(),
            title: "one".into(),
            original: Rect::new(200.0, 200.0, 500.0, 400.0),
        }],
    });
    s.launch(100, "a");
    let parked_at = Rect::new(1919.0, 1079.0, 500.0, 400.0);
    s.frames.insert(WindowId(1), parked_at);
    s.ev(Event::WindowsDiscovered {
        pid: 100,
        windows: vec![probe(100, 1, "one", parked_at)],
        initial: true,
    });
    s.ev(Event::InitialScanDone);
    assert!(s.on_screen(1));
    assert!(s.notices.iter().any(|n| n.contains("Restored 1")));
}

#[test]
fn focus_on_parked_window_switches_workspace_unless_just_closed() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.launch(200, "b");
    s.window(100, 1, "one", CENTER);
    s.focus(100, 1);
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(200, 2, "two", CENTER);
    s.focus(200, 2);
    s.now += 1000;
    s.focus(100, 1);
    assert_eq!(
        s.m.visible["C"], "C/1",
        "cmd-tab to a hidden window shows its workspace"
    );
    s.now += 1000;
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(100, 3, "three", CENTER);
    s.focus(100, 3);
    s.now += 1000;
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(3),
    });
    s.focus(100, 1);
    assert_eq!(
        s.m.visible["C"], "C/2",
        "macOS picking a hidden window after close does not switch"
    );
}

#[test]
fn switching_is_not_undone_by_stale_focus_reports() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.launch(200, "b");
    s.window(100, 1, "one", CENTER);
    s.focus(100, 1);
    // Switch to an empty workspace: the desktop gets keyboard focus.
    let fx = s.ev(Event::Command {
        cmd: Command::Workspace { name: "2".into() },
        token: None,
    });
    assert!(fx.iter().any(|f| matches!(f, Effect::FocusDesktop)));
    // Long after, macOS re-reports the window it had focused: no jump back.
    s.now += 5000;
    s.focus(100, 1);
    assert_eq!(s.m.visible["C"], "C/2");
    // A background app reporting its own focus never switches.
    s.window(200, 2, "bg", CENTER);
    s.cmd(Command::Workspace { name: "3".into() });
    s.now += 5000;
    s.ev(Event::FocusChanged {
        pid: 200,
        wid: Some(WindowId(2)),
        frontmost: false,
    });
    assert_eq!(s.m.visible["C"], "C/3");
    // The user Cmd-Tabs to a different hidden window: follow it.
    s.focus(200, 2);
    assert_eq!(s.m.visible["C"], "C/2");
}

#[test]
fn old_configs_and_state_still_load() {
    let mut s = Sim::new();
    // [[workspace]] is ignored now, but accepted.
    let cfg = Config::parse("[[workspace]]\nname = \"web\"\ndisplay = \"left\"\n").unwrap();
    s.ev(Event::ConfigLoaded {
        config: Box::new(cfg),
    });
    assert_eq!(s.m.visible["L"], "L/1");
    // A state.json from before per-display workspaces.
    let old = r#"{"version":1,"workspaces":[{"id":"4","prefs":[],"display":"C","tree":{"type":"container","children":[{"type":"window","id":1}]}}],"visible":{"C":"4"}}"#;
    let state: pane_core::state::State = serde_json::from_str(old).unwrap();
    let mut s2 = Sim::with(Model::from_state(Config::default(), &state, s.now));
    assert_eq!(s2.m.visible["C"], "C/4");
    s2.launch(100, "a");
    s2.ev(Event::WindowsDiscovered {
        pid: 100,
        windows: vec![probe(100, 1, "x", CENTER)],
        initial: true,
    });
    assert_eq!(s2.m.windows[&WindowId(1)].workspace, "C/4");
}

#[test]
fn dragged_tiled_window_snaps_back_after_the_drag() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    let home = s.frame(1);
    let sets = s.sets_for[&WindowId(1)];
    for i in 0..5 {
        let r = Rect::new(300.0 + i as f64 * 10.0, 200.0, home.w, home.h);
        s.frames.insert(WindowId(1), r);
        s.now += 50;
        s.ev(Event::WindowFrameChanged {
            pid: 100,
            wid: WindowId(1),
            frame: r,
        });
    }
    assert_eq!(
        s.sets_for[&WindowId(1)],
        sets,
        "no fighting during the drag"
    );
    s.tick(400);
    assert_eq!(s.frame(1), home);
}

#[test]
fn unbundled_apps_tile_and_float_apps_float() {
    let mut s = Sim::new();
    // `cargo run`: no bundle id, known by its executable name.
    s.ev(Event::AppLaunched {
        pid: 300,
        bundle_id: None,
        name: "my-app".into(),
        regular: true,
    });
    s.window(300, 30, "my app", CENTER);
    assert_eq!(s.m.windows[&WindowId(30)].bundle_id, "my-app");
    assert_eq!(s.m.windows[&WindowId(30)].mode, WindowMode::Tiled);
    s.launch(100, "com.apple.calculator");
    s.window(100, 1, "Calculator", CENTER);
    assert_eq!(s.m.windows[&WindowId(1)].mode, WindowMode::Tiled);

    // Adding the app to float_apps floats its open window and new ones.
    let cfg = Config::parse("[settings]\nfloat_apps = [\"com.apple.calculator\"]\n").unwrap();
    s.ev(Event::ConfigLoaded {
        config: Box::new(cfg),
    });
    assert_eq!(s.m.windows[&WindowId(1)].mode, WindowMode::Floating);
    assert_eq!(s.frame(30).w, 1908.0, "my-app takes the whole width again");
    s.window(100, 2, "Calculator 2", CENTER);
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Floating);
}
