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
use pane_core::swipe::Swipe;
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
    /// Like macOS: a window pushed below a display's bottom edge is nudged
    /// back up so a 30pt strip stays visible.
    clamp_offscreen: bool,
    /// Apps that apply a move late: the read-back right after a SetFrame
    /// still shows the previous frame.
    lagging: Vec<WindowId>,
    /// Report every SetFrame as failed (AX error) even though it moved.
    report_errors: bool,
    /// Report no read-back frame.
    no_readback: bool,
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
            clamp_offscreen: false,
            lagging: vec![],
            report_errors: false,
            no_readback: false,
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
                        let mut actual = if self.stuck.contains(wid) {
                            self.frames[wid]
                        } else {
                            Rect::new(rect.x, rect.y, rect.w.max(min.w), rect.h.max(min.h))
                        };
                        let reported = if self.lagging.contains(wid) {
                            self.frames.get(wid).copied()
                        } else {
                            None
                        };
                        if self.clamp_offscreen {
                            let bottom = self
                                .m
                                .displays
                                .iter()
                                .map(|d| d.frame.max_y())
                                .fold(f64::MIN, f64::max);
                            if actual.y > bottom - 30.0 {
                                actual.y = bottom - 30.0;
                            }
                        }
                        self.frames.insert(*wid, actual);
                        let actual = reported.unwrap_or(actual);
                        follow.push(Event::FrameApplied {
                            pid: *pid,
                            wid: *wid,
                            gen: *gen,
                            ok: !self.report_errors,
                            actual: if self.no_readback { None } else { Some(actual) },
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
        // A user Cmd-Tab / click: the app is activated.
        self.ev(Event::FocusChanged {
            frontmost: true,
            activated: true,
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
fn smart_and_per_display_gaps() {
    let c = Config::parse(
        "[settings]\ninner_gap = 6\nouter_gap = 6\nsmart_gaps = true\n\
         [[display]]\nname = \"middle\"\nposition = \"center\"\ninner_gap = 20\nouter_gap = 10\n",
    )
    .unwrap();
    let mut s = Sim::with(Model::new(c, 1_000_000));
    s.launch(100, "com.apple.Terminal");
    // One tile on the centre display fills its visible frame.
    s.window(100, 1, "term 1", CENTER);
    assert_eq!(s.frame(1), Rect::new(0.0, 30.0, 1920.0, 1050.0));
    // A second tile brings back the centre display's own gaps.
    s.window(100, 2, "term 2", CENTER);
    assert_eq!(s.frame(1).x, 10.0);
    assert_eq!(s.frame(1).y, 40.0);
    assert_eq!(s.frame(1).h, 1030.0);
    assert_eq!(s.frame(2).x - s.frame(1).max_x(), 20.0);
    assert_eq!(s.frame(2).max_x(), 1910.0);
    // Closing one drops the gaps again.
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(2),
    });
    assert_eq!(s.frame(1), Rect::new(0.0, 30.0, 1920.0, 1050.0));
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
    s.tick(600); // The retry reads the same size: it is a minimum.
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
fn late_resize_on_retile_is_not_a_min_size() {
    let mut s = Sim::new();
    s.launch(100, "a");
    for i in 1..=3 {
        s.window(100, i, &format!("w{i}"), CENTER);
    }
    s.focus(100, 3);
    assert_eq!(s.cmd(Command::ToggleFloat), Reply::ok());
    let floating = s.frame(3);
    // Retiling: the first read-back still shows the floating size.
    s.lagging.push(WindowId(3));
    assert_eq!(s.cmd(Command::ToggleFloat), Reply::ok());
    assert!(s.frame(3).w < floating.w);
    assert_eq!(s.m.windows[&WindowId(3)].min_size, None);
    s.tick(600);
    assert_eq!(s.m.windows[&WindowId(3)].min_size, None);
    assert_eq!(s.m.windows[&WindowId(3)].mode, WindowMode::Tiled);
    assert!(!s.notices.iter().any(|n| n.contains("Not enough room")));
}

#[test]
fn missing_read_back_is_not_a_min_size() {
    let mut s = Sim::new();
    s.launch(100, "a");
    for i in 1..=3 {
        s.window(100, i, &format!("w{i}"), CENTER);
    }
    s.focus(100, 3);
    assert_eq!(s.cmd(Command::ToggleFloat), Reply::ok());
    // Retiling while the window can't be read back: the only frame Pane
    // knows is the old floating one, which is no evidence of a minimum.
    s.no_readback = true;
    s.report_errors = true;
    assert_eq!(s.cmd(Command::ToggleFloat), Reply::ok());
    s.tick(600);
    s.tick(600);
    assert_eq!(s.m.windows[&WindowId(3)].min_size, None);
    assert_eq!(s.m.windows[&WindowId(3)].mode, WindowMode::Tiled);
    assert!(!s.notices.iter().any(|n| n.contains("Not enough room")));
}

#[test]
fn retiling_by_hand_forgets_the_learned_min_size() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "w1", CENTER);
    s.window(100, 2, "w2", CENTER);
    s.focus(100, 2);
    assert_eq!(s.cmd(Command::ToggleFloat), Reply::ok());
    s.m.windows.get_mut(&WindowId(2)).unwrap().min_size = Some(Size::new(1900.0, 0.0));
    assert_eq!(s.cmd(Command::ToggleFloat), Reply::ok());
    assert_eq!(s.m.windows[&WindowId(2)].min_size, None);
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Tiled);
}

#[test]
fn too_many_min_sizes_float_newest() {
    let mut s = Sim::new();
    s.launch(100, "a");
    for i in 1..=3 {
        s.min.insert(WindowId(i), Size::new(800.0, 400.0));
        s.window(100, i, &format!("w{i}"), CENTER);
    }
    s.tick(600);
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
fn empty_guests_are_hidden_from_snapshot_and_cycling() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "centre", CENTER);
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    assert_eq!(s.m.focus_display.as_deref(), Some("R"));
    s.window(100, 2, "right", CENTER);
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "R/1");
    s.ev(Event::DisplaysChanged {
        displays: displays(&["L", "C"]),
    });
    let listed = |s: &Sim, id: &str| {
        pane_core::snapshot::build(&s.m)
            .workspaces
            .iter()
            .any(|w| w.id == id)
    };
    assert!(listed(&s, "R/1"), "a guest with windows is listed");
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(2),
    });
    assert!(s.m.ws("R/1").unwrap().is_guest(), "still kept as a guest");
    assert!(!listed(&s, "R/1"), "an empty guest is not listed");
    // Cycling on C skips it (C/1 is the only stop, so nothing changes).
    s.focus(100, 1);
    assert_eq!(s.m.focus_display.as_deref(), Some("C"));
    s.cmd(Command::CycleWorkspace { which: Cycle::Next });
    assert_eq!(s.m.visible["C"], "C/1");
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
    // Long after, macOS re-reports the window it had focused (a plain
    // focus notice, not an activation): no jump back.
    s.now += 5000;
    s.ev(Event::FocusChanged {
        pid: 100,
        wid: Some(WindowId(1)),
        frontmost: true,
        activated: false,
    });
    assert_eq!(s.m.visible["C"], "C/2");
    // A background app reporting its own focus never switches.
    s.window(200, 2, "bg", CENTER);
    s.cmd(Command::Workspace { name: "3".into() });
    s.now += 5000;
    s.ev(Event::FocusChanged {
        pid: 200,
        wid: Some(WindowId(2)),
        frontmost: false,
        activated: false,
    });
    assert_eq!(s.m.visible["C"], "C/3");
    // A late, non-activation focus notice from the frontmost app for a
    // window we hid does not switch back either.
    s.ev(Event::FocusChanged {
        pid: 200,
        wid: Some(WindowId(2)),
        frontmost: true,
        activated: false,
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

fn drag_to(s: &mut Sim, wid: u32, target: Rect) {
    // A drag is a stream of frames; the app follows the mouse.
    let from = s.frame(wid);
    for i in 1..=5 {
        let t = i as f64 / 5.0;
        let r = Rect::new(
            from.x + (target.x - from.x) * t,
            from.y + (target.y - from.y) * t,
            from.w + (target.w - from.w) * t,
            from.h + (target.h - from.h) * t,
        );
        s.frames.insert(WindowId(wid), r);
        s.now += 30;
        s.ev(Event::WindowFrameChanged {
            pid: 100,
            wid: WindowId(wid),
            frame: r,
        });
    }
    s.tick(400);
}

#[test]
fn dragging_a_tile_edge_resizes_against_its_neighbour_only() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    for i in 1..=3 {
        s.window(100, i, "w", CENTER);
    }
    let (a, b, c) = (s.frame(1), s.frame(2), s.frame(3));
    assert!(a.x < b.x && b.x < c.x, "side by side: {a:?} {b:?} {c:?}");
    // Drag the border between 1 and 2 right by 100 (window 1's right edge).
    drag_to(&mut s, 1, Rect::new(a.x, a.y, a.w + 100.0, a.h));
    let (a2, b2, c2) = (s.frame(1), s.frame(2), s.frame(3));
    assert!((a2.w - (a.w + 100.0)).abs() <= 2.0, "grew: {a2:?}");
    assert!(
        (b2.w - (b.w - 100.0)).abs() <= 2.0,
        "neighbour gave: {b2:?}"
    );
    assert_eq!(c2, c, "third tile untouched");
    // Same border from the other side: window 2's left edge back left.
    drag_to(&mut s, 2, Rect::new(b2.x - 100.0, b2.y, b2.w + 100.0, b2.h));
    assert!((s.frame(1).w - a.w).abs() <= 2.0);
    assert_eq!(s.frame(3), c);
}

#[test]
fn left_edge_drag_reported_as_a_move_first_still_resizes() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    for i in 1..=3 {
        s.window(100, i, "w", CENTER);
    }
    s.now += 500;
    let (a, b) = (s.frame(1), s.frame(2));
    // Window 2's left edge, dragged left with the button held. macOS
    // reports each step's move before its resize, so early on the window
    // looks dragged by its title bar.
    for i in 1..=10 {
        let dx = 10.0 * i as f64;
        let at = pt(b.x - dx, b.y + b.h / 2.0);
        s.drag_fx(DragPhase::Move { at });
        let prev = s.frame(2);
        for r in [
            Rect::new(b.x - dx, b.y, prev.w, b.h),
            Rect::new(b.x - dx, b.y, b.w + dx, b.h),
        ] {
            s.frames.insert(WindowId(2), r);
            s.now += 8;
            s.ev(Event::WindowFrameChanged {
                pid: 100,
                wid: WindowId(2),
                frame: r,
            });
        }
    }
    let fx = s.drag_fx(DragPhase::Release {
        at: pt(b.x - 100.0, b.y + b.h / 2.0),
    });
    assert!(s.m.gesture.is_none());
    assert!(overlay(&fx).is_none());
    s.tick(400);
    assert!(
        (s.frame(2).w - (b.w + 100.0)).abs() <= 2.0,
        "grew: {:?}",
        s.frame(2)
    );
    assert!(
        (s.frame(1).w - (a.w - 100.0)).abs() <= 2.0,
        "gave: {:?}",
        s.frame(1)
    );
    assert!(s.frame(1).x < s.frame(2).x, "order kept");
}

#[test]
fn edge_drag_below_a_wrongly_learned_min_size_sticks() {
    // Seen on macOS: an app still applying an earlier resize read back the
    // same in-between size twice, so Pane learned it as a minimum. Every
    // drag that made the window smaller then snapped back to it.
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "w1", CENTER);
    s.window(100, 2, "w2", CENTER);
    let wrong = s.frame(2).w + 400.0;
    s.m.windows.get_mut(&WindowId(2)).unwrap().min_size = Some(Size::new(wrong, 0.0));
    s.m.layout_dirty = true;
    s.tick(600);
    let (a, b) = (s.frame(1), s.frame(2));
    assert!((b.w - wrong).abs() <= 2.0, "pinned at the minimum: {b:?}");
    s.now += 500;
    // Drag window 2's left edge right by 600: smaller than its "minimum".
    drag_to(&mut s, 2, Rect::new(b.x + 600.0, b.y, b.w - 600.0, b.h));
    assert!(
        (s.frame(2).w - (b.w - 600.0)).abs() <= 2.0,
        "kept: {:?}",
        s.frame(2)
    );
    assert!(
        (s.frame(1).w - (a.w + 600.0)).abs() <= 2.0,
        "neighbour took it: {:?}",
        s.frame(1)
    );
    // And it stays there.
    s.tick(3000);
    assert!(
        (s.frame(2).w - (b.w - 600.0)).abs() <= 2.0,
        "stayed: {:?}",
        s.frame(2)
    );
    // The weights hold the new split, not just the (lowered) minimum:
    // without any minimum the tile keeps its size.
    s.m.windows.get_mut(&WindowId(2)).unwrap().min_size = None;
    s.m.layout_dirty = true;
    s.tick(600);
    assert!(
        (s.frame(2).w - (b.w - 600.0)).abs() <= 2.0,
        "weights: {:?}",
        s.frame(2)
    );
}

#[test]
fn dragging_an_outer_edge_snaps_back() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    let (a, b) = (s.frame(1), s.frame(2));
    // Window 1's left edge is the screen edge: nothing to trade with.
    drag_to(&mut s, 1, Rect::new(a.x + 80.0, a.y, a.w - 80.0, a.h));
    assert_eq!(s.frame(1), a);
    assert_eq!(s.frame(2), b);
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

fn slot(bundle: &str) -> pane_core::tree::SerNode {
    pane_core::tree::SerNode::Slot {
        bundle_id: bundle.into(),
        title: None,
        launch_args: vec![],
    }
}

fn two_up(a: &str, b: &str) -> pane_core::tree::SerNode {
    pane_core::tree::SerNode::Container {
        orientation: pane_core::Orientation::Horizontal,
        layout: Default::default(),
        weights: vec![1.0, 1.0],
        children: vec![slot(a), slot(b)],
    }
}

#[test]
fn profile_stays_until_another_is_applied() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(200, "com.apple.Safari");
    s.window(200, 2, "Web", CENTER);
    let p = pane_core::profile::Profile {
        name: "work".into(),
        version: 1,
        when: None,
        displays: vec![
            pane_core::profile::ProfileDisplay {
                display: "center".into(),
                workspace: "1".into(),
                tree: two_up("com.apple.mail", "com.apple.Safari"),
                visible: true,
            },
            pane_core::profile::ProfileDisplay {
                display: "center".into(),
                workspace: "2".into(),
                tree: slot("com.apple.Terminal"),
                visible: false,
            },
        ],
    };
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(p),
    });
    assert_eq!(s.m.active_profile.as_deref(), Some("work"));
    assert_eq!(s.launches.len(), 2, "Mail and Terminal are launched");
    let half = s.frame(2);
    assert!(half.w < 1000.0, "Mail's slot is reserved while it launches");

    // Mail never shows up: the slot stops taking space but is not deleted.
    s.tick(31_000);
    assert_eq!(s.frame(2).w, 1908.0);
    // Switching workspaces and opening other windows does not lose it.
    s.cmd(Command::Workspace { name: "2".into() });
    s.cmd(Command::Workspace { name: "1".into() });
    s.launch(300, "com.other");
    s.window(300, 3, "Other", CENTER);
    assert!(
        s.m.ws("C/2").is_some(),
        "the profile's workspace 2 is kept while empty"
    );
    // Mail launches much later: it goes into its slot, left of Safari.
    s.launch(100, "com.apple.mail");
    s.window(100, 1, "Inbox", CENTER);
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "C/1");
    assert!(
        s.frame(1).x < s.frame(2).x,
        "Mail is left of Safari as in the profile"
    );
    // Closing Mail gives the slot back; reopening returns to it.
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(1),
    });
    s.window(100, 11, "Inbox", CENTER);
    assert!(s.frame(11).x < s.frame(2).x);
    // Terminal, when it opens, goes to workspace 2 of the profile.
    s.launch(400, "com.apple.Terminal");
    s.window(400, 4, "zsh", CENTER);
    assert_eq!(s.m.windows[&WindowId(4)].workspace, "C/2");

    // Restart Pane: still active, and slots survive new window ids.
    pane_core::reducer::refresh_placements(&mut s.m);
    let state: pane_core::state::State =
        serde_json::from_str(&serde_json::to_string(&s.m.to_state()).unwrap()).unwrap();
    let mut s2 = Sim::with(Model::from_state(Config::default(), &state, s.now));
    assert_eq!(s2.m.active_profile.as_deref(), Some("work"));
    s2.launch(101, "com.apple.mail");
    s2.launch(201, "com.apple.Safari");
    s2.window(201, 21, "Web", CENTER);
    s2.window(101, 31, "Inbox", CENTER);
    assert!(
        s2.frame(31).x < s2.frame(21).x,
        "after a reboot Mail is back left of Safari"
    );
}

#[test]
fn stale_placement_never_hides_a_window() {
    let mut s = Sim::new();
    s.tick(2000);
    // A record from an older layout naming a display Pane does not know.
    s.m.placements.push(pane_core::identity::PlacementRecord {
        fp: pane_core::identity::WindowFingerprint::new(
            "com.google.Chrome",
            "Docs",
            None,
            0,
            Size::new(800.0, 600.0),
        ),
        workspace: "GONE-DISPLAY/2".into(),
        anchor: vec![0],
        weight: 1.0,
        floating: false,
        float_frame: None,
        seen_at: s.now,
        last_wid: None,
        live: None,
    });
    s.launch(500, "com.google.Chrome");
    s.window(500, 5, "Docs", CENTER);
    let ws = s.m.windows[&WindowId(5)].workspace.clone();
    assert!(ws.starts_with("C/"), "{ws}");
    // The record said workspace 2: it is that workspace of the centre display.
    s.cmd(Command::Workspace { name: "2".into() });
    assert!(s.on_screen(5), "visible when its workspace is shown");
    assert!(!s
        .m
        .workspaces
        .iter()
        .any(|w| w.home.is_empty() || w.home == "GONE-DISPLAY"));
}

#[test]
fn repair_brings_back_windows_in_missing_workspaces() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    // Corrupt the model the way a bug might.
    s.m.windows.get_mut(&WindowId(1)).unwrap().workspace = "nowhere/9".into();
    s.m.layout_dirty = true;
    s.tick(10);
    assert!(s.on_screen(1));
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "C/1");
}

#[test]
fn moved_window_stays_on_its_workspace() {
    let mut s = Sim::new();
    s.clamp_offscreen = true;
    s.tick(2000);
    s.launch(100, "com.google.Chrome");
    s.launch(150, "com.apple.Terminal");
    s.window(100, 1, "browser", CENTER);
    s.window(150, 5, "term", CENTER);
    s.focus(100, 1);
    // mod-shift-2: move the browser to workspace 2.
    s.cmd(Command::MoveToWorkspace { name: "2".into() });
    s.now += 600;
    s.cmd(Command::Workspace { name: "2".into() });
    assert!(s.on_screen(1), "visible on w2 at first");
    s.now += 600;
    s.cmd(Command::Workspace { name: "1".into() });
    // Time passes on w1 (park retries run their course).
    for _ in 0..10 {
        s.tick(1000);
    }
    // A new app opens on w1 and takes focus.
    s.launch(200, "com.apple.Notes");
    s.window(200, 2, "notes", CENTER);
    s.focus(200, 2);
    s.now += 600;
    s.cmd(Command::Workspace { name: "2".into() });
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "C/2");
    let f = s.frame(1);
    assert!(
        f.y < 100.0 && f.h > 900.0,
        "browser tiled on w2, not in the corner: {f:?}"
    );
    assert!(!s.m.windows[&WindowId(1)].stubborn);
}

#[test]
fn stubborn_windows_still_follow_their_workspace() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.stuck.push(WindowId(2));
    s.window(100, 2, "two", Rect::new(5.0, 5.0, 300.0, 300.0));
    for _ in 0..10 {
        s.tick(3000);
    }
    assert!(s.m.windows[&WindowId(2)].stubborn);
    // It moves again when hidden and shown (the app accepts those moves).
    s.stuck.clear();
    s.cmd(Command::Workspace { name: "2".into() });
    assert!(!s.on_screen(2), "hidden with its workspace");
    s.cmd(Command::Workspace { name: "1".into() });
    assert!(
        s.frame(2).x < 1900.0 && s.frame(2).y < 1000.0,
        "back on screen: {:?}",
        s.frame(2)
    );
}

#[test]
fn fullscreen_zooms_the_focused_tile() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    s.focus(100, 2);
    let tiles = (s.frame(1), s.frame(2));
    s.cmd(Command::ToggleFullscreen);
    // Fills the display's visible frame; the other tile stays behind.
    assert_eq!(s.frame(2), Rect::new(0.0, 30.0, 1920.0, 1050.0));
    assert_eq!(s.frame(1), tiles.0);
    let snap = pane_core::snapshot::build(&s.m);
    let c = snap.displays.iter().find(|d| d.id == "C").unwrap();
    assert_eq!(c.tiles.len(), 1, "icon shows only the zoomed tile");
    // Survives switching away and back.
    s.cmd(Command::Workspace { name: "2".into() });
    s.cmd(Command::Workspace { name: "1".into() });
    assert_eq!(s.frame(2).w, 1920.0);
    // Toggle again restores the tiles.
    s.focus(100, 2);
    s.cmd(Command::ToggleFullscreen);
    assert_eq!((s.frame(1), s.frame(2)), tiles);
    // Focusing a neighbour or a new window ends the zoom.
    s.cmd(Command::ToggleFullscreen);
    s.cmd(Command::Focus {
        dir: Direction::Left,
    });
    assert_eq!((s.frame(1), s.frame(2)), tiles);
    s.focus(100, 2);
    s.cmd(Command::ToggleFullscreen);
    s.window(100, 3, "three", CENTER);
    assert!(s.frame(2).w < 1000.0, "a new tile ends the zoom");
    // Closing the zoomed window clears it.
    s.focus(100, 3);
    s.cmd(Command::ToggleFullscreen);
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(3),
    });
    assert!(s.m.ws("C/1").unwrap().fullscreen.is_none());
}

#[test]
fn new_apps_open_on_the_display_you_work_on() {
    let mut s = Sim::new();
    s.clamp_offscreen = true;
    s.tick(2000);
    s.ev(Event::InitialScanDone);
    // Work on the right display (not the main one).
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    s.launch(100, "com.google.Chrome");
    s.window(100, 1, "browser", CENTER);
    s.focus(100, 1);
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "R/1");
    s.cmd(Command::MoveToWorkspace { name: "2".into() });
    s.now += 600;
    s.cmd(Command::Workspace { name: "2".into() });
    s.now += 600;
    s.cmd(Command::Workspace { name: "1".into() });
    for _ in 0..5 {
        s.tick(1000);
    }
    // A new app: macOS opens its window on the main (centre) display, and
    // the worker reports it from its first enumeration.
    s.ev(Event::AppLaunched {
        pid: 200,
        bundle_id: Some("com.apple.Notes".into()),
        name: "Notes".into(),
        regular: true,
    });
    let on_main = Rect::new(300.0, 200.0, 800.0, 600.0);
    s.frames.insert(WindowId(2), on_main);
    s.ev(Event::WindowsDiscovered {
        pid: 200,
        windows: vec![probe(200, 2, "notes", on_main)],
        initial: true,
    });
    s.focus(200, 2);
    assert_eq!(
        s.m.windows[&WindowId(2)].workspace,
        "R/1",
        "opens where you work"
    );
    s.now += 600;
    s.cmd(Command::Workspace { name: "2".into() });
    assert_eq!(s.m.visible["R"], "R/2");
    let f = s.frame(1);
    assert!(
        f.x >= 1920.0 && f.y < 100.0 && f.h > 900.0,
        "browser tiled on the right display's w2: {f:?}"
    );
}

#[test]
fn minimized_windows_stay_minimized_across_switches() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    s.focus(100, 2);
    s.ev(Event::WindowMinimized {
        pid: 100,
        wid: WindowId(2),
        minimized: true,
    });
    assert_eq!(s.frame(1).w, 1908.0, "the other window takes the space");
    s.cmd(Command::Workspace { name: "2".into() });
    let fx = s.ev(Event::Command {
        cmd: Command::Workspace { name: "1".into() },
        token: None,
    });
    assert!(
        !fx.iter().any(|f| matches!(
            f,
            Effect::Focus {
                wid: WindowId(2),
                ..
            } | Effect::Raise {
                wid: WindowId(2),
                ..
            } | Effect::SetFrame {
                wid: WindowId(2),
                ..
            }
        )),
        "a minimized window is not focused, raised or moved: {fx:?}"
    );
    assert!(s.m.windows[&WindowId(2)].minimized);
}

#[test]
fn late_applying_apps_still_come_back() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "com.google.Chrome");
    s.window(100, 1, "browser", CENTER);
    s.lagging.push(WindowId(1));
    s.cmd(Command::Workspace { name: "2".into() });
    s.tick(3000);
    s.cmd(Command::Workspace { name: "1".into() });
    for _ in 0..5 {
        s.tick(1000);
    }
    let f = s.frame(1);
    assert!(
        f.x < 100.0 && f.y < 100.0 && f.w > 1800.0,
        "back in its tile: {f:?}"
    );
    assert!(!s.m.windows[&WindowId(1)].stubborn);
    assert_eq!(s.m.windows[&WindowId(1)].mode, WindowMode::Tiled);
}

#[test]
fn watchdog_resends_windows_found_hidden() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    // Something moved the window off screen behind our back and we missed it.
    let hidden = Rect::new(1919.0, 1079.0, 900.0, 900.0);
    s.frames.insert(WindowId(1), hidden);
    s.m.windows.get_mut(&WindowId(1)).unwrap().actual = hidden;
    s.tick(2500);
    assert!(
        s.frame(1).x < 100.0,
        "watchdog put it back: {:?}",
        s.frame(1)
    );
}

fn hide_and_return(configure: impl Fn(&mut Sim)) -> (Sim, Vec<String>) {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    configure(&mut s);
    s.cmd(Command::Workspace { name: "2".into() });
    for _ in 0..8 {
        s.tick(1000);
    }
    s.cmd(Command::Workspace { name: "1".into() });
    for _ in 0..8 {
        s.tick(1000);
    }
    let notes = s.notices.clone();
    (s, notes)
}

#[test]
fn hiding_never_marks_windows_stubborn() {
    for (name, cfg) in [
        (
            "errors",
            (|s: &mut Sim| s.report_errors = true) as fn(&mut Sim),
        ),
        ("no readback", |s: &mut Sim| s.no_readback = true),
        ("clamped", |s: &mut Sim| s.clamp_offscreen = true),
        ("lagging", |s: &mut Sim| {
            s.lagging = vec![WindowId(1), WindowId(2)]
        }),
    ] {
        let (s, notes) = hide_and_return(cfg);
        assert!(
            !notes.iter().any(|n| n.contains("refuses")),
            "{name}: {notes:?}"
        );
        for w in [1, 2] {
            let info = &s.m.windows[&WindowId(w)];
            assert!(
                !info.stubborn && info.mode == WindowMode::Tiled,
                "{name}: window {w} {info:?}"
            );
        }
    }
}

// ---------------------------------------------------------------- gestures

use pane_core::geometry::Point;
use pane_core::gesture::{DragPhase, GesturePhase};

fn pt(x: f64, y: f64) -> Point {
    Point { x, y }
}

impl Sim {
    fn gesture_fx(&mut self, phase: GesturePhase) -> Vec<Effect> {
        self.ev(Event::Command {
            cmd: Command::Gesture { phase },
            token: None,
        })
    }
}

fn overlay(fx: &[Effect]) -> Option<(Rect, String)> {
    fx.iter().rev().find_map(|f| match f {
        Effect::UiRequest(UiRequest::GestureOverlay { frame, label, .. }) => {
            Some((*frame, label.clone()))
        }
        _ => None,
    })
}

/// Point at `from`, move to `to`, release. Returns the overlay shown
/// before the drop; checks the window then has exactly that frame.
impl Sim {
    fn gesture(&mut self, wid: u32, from: Point, to: Point) -> Option<(Rect, String)> {
        let s = self;
        let fx = s.gesture_fx(GesturePhase::Begin {
            at: from,
            pointer: to,
        });
        let shown = overlay(&fx);
        let fx = s.gesture_fx(GesturePhase::Drop { at: to });
        assert!(fx
            .iter()
            .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide))));
        assert!(s.m.gesture.is_none());
        if let Some((frame, _)) = &shown {
            assert_eq!(s.frame(wid), *frame, "preview equals the final frame");
        }
        shown
    }
}

fn two_tiles() -> Sim {
    let mut s = Sim::new();
    s.launch(100, "Term");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    s
}

#[test]
fn gesture_swaps_tiles() {
    let mut s = two_tiles();
    let (a, b) = (s.frame(1), s.frame(2));
    let shown = s.gesture(1, a.center(), b.center());
    assert_eq!(shown.unwrap().1, "Swap with Term");
    assert_eq!((s.frame(1), s.frame(2)), (b, a));
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
}

#[test]
fn gesture_splits_a_tile_on_each_side() {
    for (fx, fy, dir) in [
        (0.1, 0.5, Direction::Left),
        (0.9, 0.5, Direction::Right),
        (0.5, 0.1, Direction::Up),
        (0.5, 0.9, Direction::Down),
    ] {
        let mut s = two_tiles();
        let b = s.frame(2);
        let at = pt(b.x + b.w * fx, b.y + b.h * fy);
        let shown = s.gesture(1, s.frame(1).center(), at).expect("overlay");
        let (one, two) = (s.frame(1), s.frame(2));
        match dir {
            Direction::Left => {
                assert_eq!(shown.1, "Left of Term");
                assert!(one.x < two.x && one.w == two.w);
            }
            Direction::Right => assert!(one.x > two.x),
            Direction::Up => {
                assert_eq!(shown.1, "Above Term");
                assert!(one.y < two.y && one.w == two.w && one.w > 1800.0);
            }
            Direction::Down => assert!(one.y > two.y && one.x == two.x),
        }
    }
}

#[test]
fn gesture_to_a_display_edge_makes_a_full_column_or_row() {
    let mut s = two_tiles();
    s.window(100, 3, "three", CENTER);
    let from = s.frame(3).center();
    let shown = s.gesture(3, from, pt(20.0, 500.0)).unwrap();
    assert_eq!(shown.1, "Left edge");
    assert_eq!(s.frame(3).x, 6.0);
    assert_eq!(s.frame(3).h, 1038.0);
    let from = s.frame(3).center();
    let shown = s.gesture(3, from, pt(900.0, 50.0)).unwrap();
    assert_eq!(shown.1, "Top edge");
    assert_eq!((s.frame(3).y, s.frame(3).w), (36.0, 1908.0));
    assert!(s.frame(1).y > s.frame(3).max_y());
}

#[test]
fn gesture_moves_to_another_display() {
    let mut s = two_tiles();
    let from = s.frame(2).center();
    let shown = s.gesture(2, from, pt(2900.0, 500.0)).unwrap();
    assert_eq!(shown.1, "Fill Display R");
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "R/1");
    assert_eq!(s.frame(2), Rect::new(1926.0, 36.0, 1908.0, 1038.0));
    assert_eq!(s.frame(1).w, 1908.0);
    assert_eq!(s.m.focus_display.as_deref(), Some("R"));
    // Back onto C with tiles there: split C's tile.
    let one = s.frame(1);
    s.gesture(2, s.frame(2).center(), pt(one.x + one.w * 0.9, 500.0))
        .unwrap();
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "C/1");
    assert!(s.frame(2).x > s.frame(1).x);
    // Swapping across displays trades workspaces.
    s.window(100, 3, "three", CENTER);
    let three = s.frame(3);
    s.cmd(Command::Workspace { name: "1".into() });
    let r = s.m.windows[&WindowId(3)].workspace.clone();
    s.gesture(3, three.center(), pt(2900.0, 500.0));
    assert_ne!(s.m.windows[&WindowId(3)].workspace, r);
    s.gesture(1, s.frame(1).center(), s.frame(3).center())
        .unwrap();
    assert_eq!(s.m.windows[&WindowId(1)].workspace, "R/1");
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "C/1");
}

#[test]
fn gesture_tiles_a_floating_window() {
    let mut s = two_tiles();
    s.focus(100, 2);
    s.cmd(Command::ToggleFloat);
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Floating);
    let from = s.frame(2).center();
    let one = s.frame(1);
    s.gesture(2, from, pt(one.x + one.w * 0.9, 500.0)).unwrap();
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Tiled);
    assert!(s.m.ws("C/1").unwrap().floating.is_empty());
    assert!(s.frame(2).x > s.frame(1).x);
}

#[test]
fn floating_window_swapped_with_a_tile_floats_the_tile() {
    let mut s = two_tiles();
    s.focus(100, 2);
    s.cmd(Command::ToggleFloat);
    let float = s.frame(2);
    // Move the float off tile 1's centre so the centre hits the tile.
    s.window(100, 3, "three", CENTER);
    let target = s.frame(3).center();
    s.gesture(2, float.center(), target);
    assert_eq!(s.m.windows[&WindowId(2)].mode, WindowMode::Tiled);
    assert_eq!(s.m.windows[&WindowId(3)].mode, WindowMode::Floating);
    assert_eq!(s.frame(3), float);
}

#[test]
fn gesture_cancel_and_nothing_under_the_pointer_change_nothing() {
    let mut s = two_tiles();
    let before = (s.frame(1), s.frame(2));
    let fx = s.gesture_fx(GesturePhase::Begin {
        at: s.frame(1).center(),
        pointer: s.frame(2).center(),
    });
    assert!(overlay(&fx).is_some());
    // Back over its own tile: the overlay goes away.
    let fx = s.gesture_fx(GesturePhase::Move {
        at: s.frame(1).center(),
    });
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide))));
    s.gesture_fx(GesturePhase::Cancel);
    assert!(s.m.gesture.is_none());
    s.gesture_fx(GesturePhase::Drop {
        at: s.frame(2).center(),
    });
    assert_eq!((s.frame(1), s.frame(2)), before);
    // The menu bar has no window.
    s.gesture_fx(GesturePhase::Begin {
        at: pt(500.0, 10.0),
        pointer: s.frame(2).center(),
    });
    assert!(s.m.gesture.is_none());
    // Dropping on the own tile does nothing.
    s.gesture(1, s.frame(1).center(), pt(400.0, 600.0));
    assert_eq!((s.frame(1), s.frame(2)), before);
}

#[test]
fn gesture_drop_unzooms_and_closing_the_window_cancels() {
    let mut s = two_tiles();
    s.focus(100, 2);
    s.cmd(Command::ToggleFullscreen);
    assert_eq!(s.frame(2).w, 1920.0);
    // The zoomed window is the one under the pointer.
    s.gesture(2, pt(500.0, 500.0), pt(2900.0, 500.0)).unwrap();
    assert!(s.m.ws("C/1").unwrap().fullscreen.is_none());
    assert_eq!(s.frame(1).w, 1908.0);
    // A drop onto a zoomed workspace un-zooms it.
    s.focus(100, 2);
    s.cmd(Command::ToggleFullscreen);
    s.gesture(1, s.frame(1).center(), pt(1930.0 + 20.0, 500.0))
        .unwrap();
    assert!(s.m.ws("R/1").unwrap().fullscreen.is_none());
    assert_eq!(s.frame(1).x, 1926.0);
    // The window goes away mid-gesture.
    s.gesture_fx(GesturePhase::Begin {
        at: s.frame(1).center(),
        pointer: pt(500.0, 500.0),
    });
    assert!(s.m.gesture.is_some());
    let fx = s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(1),
    });
    assert!(s.m.gesture.is_none());
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide))));
}

// ------------------------------------------------- plain title-bar drag

impl Sim {
    fn drag_fx(&mut self, phase: DragPhase) -> Vec<Effect> {
        self.ev(Event::Command {
            cmd: Command::Drag { phase },
            token: None,
        })
    }

    /// Grab `wid` by its title bar and drag the pointer to `to` in five
    /// steps; the window follows the pointer at its size, as macOS moves
    /// it. Returns the last overlay shown. Does not let go.
    fn title_drag_to(&mut self, wid: u32, to: Point) -> Option<(Rect, String)> {
        self.now += 500; // Well past Pane's own last move of the window.
        let from = self.frame(wid);
        let grab = pt(from.x + from.w / 2.0, from.y + 10.0);
        let mut shown = None;
        for i in 1..=5 {
            let t = i as f64 / 5.0;
            let p = pt(grab.x + (to.x - grab.x) * t, grab.y + (to.y - grab.y) * t);
            let mut fx = self.drag_fx(DragPhase::Move { at: p });
            let r = Rect::new(p.x - from.w / 2.0, p.y - 10.0, from.w, from.h);
            self.frames.insert(WindowId(wid), r);
            self.now += 30;
            fx.extend(self.ev(Event::WindowFrameChanged {
                pid: 100,
                wid: WindowId(wid),
                frame: r,
            }));
            if fx
                .iter()
                .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide)))
            {
                shown = None;
            }
            shown = overlay(&fx).or(shown);
        }
        shown
    }

    /// Drag `wid` by its title bar to `to` and let go there. Checks the
    /// overlay went away and that the window took the previewed frame.
    fn title_drag(&mut self, wid: u32, to: Point) -> Option<(Rect, String)> {
        let shown = self.title_drag_to(wid, to);
        let fx = self.drag_fx(DragPhase::Release { at: to });
        assert!(self.m.gesture.is_none());
        assert!(self.m.pointer.is_none());
        if self.m.windows.contains_key(&WindowId(wid)) {
            assert!(fx
                .iter()
                .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide))));
        }
        if let Some((frame, _)) = &shown {
            assert_eq!(self.frame(wid), *frame, "preview equals the final frame");
        }
        shown
    }
}

#[test]
fn title_drag_swaps_tiles() {
    let mut s = two_tiles();
    let (a, b) = (s.frame(1), s.frame(2));
    let shown = s.title_drag(1, b.center());
    assert_eq!(shown.unwrap().1, "Swap with Term");
    assert_eq!((s.frame(1), s.frame(2)), (b, a));
    assert_eq!(s.m.windows[&WindowId(1)].mode, WindowMode::Tiled);
}

#[test]
fn title_drag_splits_and_moves_to_another_display() {
    let mut s = two_tiles();
    s.window(100, 3, "three", CENTER);
    let b = s.frame(2);
    let shown = s
        .title_drag(3, pt(b.x + b.w * 0.5, b.y + b.h * 0.9))
        .unwrap();
    assert_eq!(shown.1, "Below Term");
    assert!(s.frame(3).y > s.frame(2).y && s.frame(3).x == s.frame(2).x);
    let shown = s.title_drag(3, pt(2900.0, 500.0)).unwrap();
    assert_eq!(shown.1, "Fill Display R");
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "R/1");
    assert_eq!(s.frame(3), Rect::new(1926.0, 36.0, 1908.0, 1038.0));
}

#[test]
fn title_drag_released_over_own_tile_snaps_back() {
    let mut s = two_tiles();
    let before = (s.frame(1), s.frame(2));
    // Out over tile 2 and back onto its own tile.
    s.title_drag_to(1, before.1.center());
    let shown = s.title_drag(1, pt(before.0.x + 200.0, before.0.y + 300.0));
    assert!(shown.is_none());
    assert_eq!((s.frame(1), s.frame(2)), before);
}

#[test]
fn title_drag_pause_does_not_snap_back() {
    let mut s = two_tiles();
    let b = s.frame(2);
    let sets = s.sets_for[&WindowId(1)];
    s.title_drag_to(1, b.center());
    s.tick(400);
    s.tick(400);
    assert_eq!(s.sets_for[&WindowId(1)], sets, "no snap-back while held");
    assert!(s.m.gesture.is_some());
    s.drag_fx(DragPhase::Release { at: b.center() });
    assert_eq!(s.frame(1), b);
}

#[test]
fn title_drag_is_undoable() {
    let mut s = two_tiles();
    let (a, b) = (s.frame(1), s.frame(2));
    s.title_drag(1, b.center()).unwrap();
    assert_eq!(s.frame(1), b);
    s.cmd(Command::Undo);
    assert_eq!((s.frame(1), s.frame(2)), (a, b));
}

#[test]
fn title_drag_lost_mouse_up_snaps_back() {
    let mut s = two_tiles();
    let a = s.frame(1);
    s.title_drag_to(1, s.frame(2).center());
    assert!(s.m.gesture.is_some());
    let fx = {
        s.now += 6_000;
        s.ev(Event::Tick)
    };
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide))));
    assert!(s.m.gesture.is_none());
    assert_eq!(s.frame(1), a);
}

#[test]
fn title_drag_off_or_without_pointer_snaps_back() {
    // Setting off.
    let mut s = two_tiles();
    s.m.config.settings.title_drag = false;
    let (a, b) = (s.frame(1), s.frame(2));
    assert!(s.title_drag_to(1, b.center()).is_none());
    assert!(s.m.gesture.is_none());
    s.tick(400);
    assert_eq!((s.frame(1), s.frame(2)), (a, b));
    // No pointer news (no event tap): the old snap-back.
    let mut s = two_tiles();
    s.now += 500;
    let r = Rect::new(a.x + 300.0, a.y + 50.0, a.w, a.h);
    s.frames.insert(WindowId(1), r);
    s.ev(Event::WindowFrameChanged {
        pid: 100,
        wid: WindowId(1),
        frame: r,
    });
    assert!(s.m.gesture.is_none());
    s.tick(400);
    assert_eq!(s.frame(1), a);
}

#[test]
fn title_drag_window_closed_mid_drag_cancels() {
    let mut s = two_tiles();
    s.title_drag_to(1, s.frame(2).center());
    let fx = s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(1),
    });
    assert!(s.m.gesture.is_none());
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::UiRequest(UiRequest::GestureOverlayHide))));
    // The release that follows does nothing.
    s.drag_fx(DragPhase::Release {
        at: pt(500.0, 500.0),
    });
    assert_eq!(s.frame(2).w, 1908.0);
}

fn search(s: &mut Sim, q: &str) -> Vec<pane_core::palette::PaletteItem> {
    match s.cmd(Command::Search {
        query: q.into(),
        profiles: vec!["Work".into()],
    }) {
        Reply::Ok { data } => serde_json::from_value(data).unwrap(),
        Reply::Error { message } => panic!("{message}"),
    }
}

#[test]
fn palette_finds_windows_and_jumps_to_them() {
    use pane_core::palette::ItemKind;
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "com.apple.Safari");
    s.launch(200, "com.apple.Terminal");
    s.window(100, 1, "Rust docs", CENTER);
    s.focus(100, 1);
    s.now += 1000;
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(200, 2, "zsh", CENTER);
    s.focus(200, 2);
    s.now += 1000;

    // Empty query: the previous window first, the focused one after it.
    let items = search(&mut s, "");
    assert_eq!(items[0].command, Command::FocusWindow { wid: 1 });
    assert_eq!(items[1].command, Command::FocusWindow { wid: 2 });
    assert!(items[1].current && !items[0].current);
    assert!(
        items[0].subtitle.contains("workspace 1"),
        "{}",
        items[0].subtitle
    );
    assert!(items
        .iter()
        .all(|i| matches!(i.kind, ItemKind::Window | ItemKind::Workspace)));

    // Typing matches app names and titles.
    let items = search(&mut s, "saf");
    assert_eq!(items[0].command, Command::FocusWindow { wid: 1 });
    let items = search(&mut s, "rust");
    assert_eq!(items[0].command, Command::FocusWindow { wid: 1 });
    assert!(search(&mut s, "qqqq").is_empty());

    // `>` searches commands and profiles only.
    let items = search(&mut s, ">float");
    assert!(!items.is_empty());
    assert!(items
        .iter()
        .all(|i| matches!(i.kind, ItemKind::Command | ItemKind::Profile)));
    assert_eq!(items[0].command, Command::ToggleFloat);
    assert!(items[0].shortcut.is_some());
    let items = search(&mut s, ">work");
    assert_eq!(
        items[0].command,
        Command::ProfileApply {
            name: Some("Work".into()),
            profile: None
        }
    );
    assert!(!search(&mut s, ">")
        .iter()
        .any(|i| i.command == Command::Palette));

    // Picking a window on a hidden workspace shows it and focuses it.
    let fx = s.ev(Event::Command {
        cmd: Command::FocusWindow { wid: 1 },
        token: Some(1),
    });
    assert_eq!(s.m.visible["C"], "C/1");
    assert_eq!(s.m.focused_window(), Some(WindowId(1)));
    assert!(fx.contains(&Effect::Focus {
        pid: 100,
        wid: WindowId(1)
    }));
    assert!(s.on_screen(1) && !s.on_screen(2));

    // A minimized window is restored when picked.
    s.ev(Event::WindowMinimized {
        pid: 200,
        wid: WindowId(2),
        minimized: true,
    });
    s.now += 1000;
    let fx = s.ev(Event::Command {
        cmd: Command::FocusWindow { wid: 2 },
        token: Some(1),
    });
    assert_eq!(s.m.visible["C"], "C/2");
    assert!(fx.contains(&Effect::Restore {
        pid: 200,
        wid: WindowId(2)
    }));
    assert!(matches!(
        s.cmd(Command::FocusWindow { wid: 999 }),
        Reply::Error { .. }
    ));
}

#[test]
fn palette_jump_ends_a_zoom_that_hides_the_window() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.window(100, 2, "two", CENTER);
    s.focus(100, 2);
    s.cmd(Command::ToggleFullscreen);
    assert_eq!(s.frame(2).w, 1920.0);
    s.cmd(Command::FocusWindow { wid: 1 });
    assert!(s.m.ws("C/1").unwrap().fullscreen.is_none());
    assert!(s.frame(2).w < 1000.0);
    assert_eq!(s.m.focused_window(), Some(WindowId(1)));
}

fn scratch_sim() -> Sim {
    let cfg = Config::parse(
        "[[scratchpad]]\nname = \"t\"\napp = \"term\"\nkey = \"mod-grave\"\nsize = \"50%x50%\"\n",
    )
    .unwrap();
    let mut s = Sim::with(Model::new(cfg, 1_000_000));
    s.tick(2000);
    s
}

fn toggle(s: &mut Sim) {
    assert_eq!(s.cmd(Command::Scratchpad { name: "t".into() }), Reply::ok());
}

#[test]
fn scratchpad_opens_toggles_and_follows_the_focused_display() {
    let mut s = scratch_sim();
    s.launch(100, "com.apple.Safari");
    s.window(100, 1, "web", CENTER);
    s.focus(100, 1);
    let full = s.frame(1);

    // Not running: Pane opens it; its first window becomes the scratchpad,
    // a centered float on the focused display.
    toggle(&mut s);
    assert_eq!(s.launches, vec!["term".to_string()]);
    toggle(&mut s); // a second press while opening does not open it twice
    assert_eq!(s.launches.len(), 1);
    s.launch(200, "term");
    s.window(200, 5, "zsh", CENTER);
    assert_eq!(s.m.scratch.get("t"), Some(&WindowId(5)));
    assert_eq!(s.m.windows[&WindowId(5)].mode, WindowMode::Floating);
    assert_eq!(s.frame(5), Rect::new(480.0, 292.5, 960.0, 525.0));
    assert_eq!(s.m.focused_window(), Some(WindowId(5)));
    assert_eq!(s.frame(1), full, "a scratchpad takes no tile");
    s.focus(200, 5);

    // Shown and focused: the key hides it and focus goes back.
    toggle(&mut s);
    assert!(!s.on_screen(5));
    assert_eq!(s.m.focused_window(), Some(WindowId(1)));
    // A late report for the hidden window changes nothing.
    s.ev(Event::FocusChanged {
        frontmost: true,
        activated: false,
        pid: 200,
        wid: Some(WindowId(5)),
    });
    assert!(!s.on_screen(5));
    assert_eq!(s.m.focused_window(), Some(WindowId(1)));
    // A hidden scratchpad is not a window of its workspace.
    let snap = pane_core::snapshot::build(&s.m);
    assert!(snap
        .displays
        .iter()
        .all(|d| d.tiles.iter().all(|t| t.wid != Some(5))));

    // On another display it comes up there, centered.
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    toggle(&mut s);
    assert!(s.on_screen(5));
    assert_eq!(s.m.windows[&WindowId(5)].workspace, "R/1");
    assert_eq!(s.frame(5), Rect::new(2400.0, 292.5, 960.0, 525.0));
    s.focus(200, 5);

    // Shown but not focused: the key focuses it rather than hiding it.
    s.cmd(Command::FocusDisplay { which: Cycle::Prev });
    s.focus(100, 1);
    toggle(&mut s);
    assert!(s.on_screen(5));
    assert_eq!(s.m.focused_window(), Some(WindowId(5)));
    assert_eq!(s.m.windows[&WindowId(5)].workspace, "C/1");

    // Closing it: the next press opens the app again.
    s.ev(Event::WindowDestroyed {
        pid: 200,
        wid: WindowId(5),
    });
    assert!(s.m.scratch.is_empty());
    toggle(&mut s);
    assert_eq!(s.launches.len(), 2);
}

#[test]
fn scratchpad_adopts_a_running_window_and_answers_cmd_tab() {
    let mut s = scratch_sim();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.launch(200, "term");
    s.window(200, 5, "zsh", CENTER);
    s.focus(100, 1);
    assert!(s.frame(1).w < 1000.0, "both tiled");

    // The app's tiled window becomes the scratchpad; its tile closes.
    toggle(&mut s);
    assert!(s.launches.is_empty());
    assert_eq!(s.m.windows[&WindowId(5)].mode, WindowMode::Floating);
    assert_eq!(s.frame(1).w, 1920.0 - 12.0);
    s.focus(200, 5);
    toggle(&mut s);
    assert!(!s.on_screen(5));

    // Workspace 1 still has the hidden float, but cycling skips it.
    s.cmd(Command::Workspace { name: "2".into() });
    s.cmd(Command::CycleWorkspace { which: Cycle::Next });
    assert_eq!(s.m.visible["C"], "C/1");
    assert!(!s.on_screen(5), "showing its workspace keeps it hidden");

    // Cmd-Tab to the app shows the scratchpad.
    s.now += 2000;
    s.focus(200, 5);
    assert!(s.on_screen(5));
    assert!(!s.m.scratch_hidden.contains(&WindowId(5)));

    // Tiled meanwhile, then hidden: its tile closes again.
    s.cmd(Command::ToggleFloat);
    assert_eq!(s.m.windows[&WindowId(5)].mode, WindowMode::Tiled);
    toggle(&mut s);
    assert!(!s.on_screen(5));
    assert_eq!(s.frame(1).w, 1920.0 - 12.0);

    // The palette lists the scratchpad and jumping to it shows it.
    s.cmd(Command::FocusWindow { wid: 5 });
    assert!(s.on_screen(5));
    let items = match s.cmd(Command::Search {
        query: ">scratch".into(),
        profiles: vec![],
    }) {
        Reply::Ok { data } => data,
        r => panic!("{r:?}"),
    };
    assert_eq!(items[0]["title"], "Toggle scratchpad t");

    // Dropped from the config: it lets go of the window.
    s.ev(Event::ConfigLoaded {
        config: Box::new(Config::default()),
    });
    assert!(s.m.scratch.is_empty());
    assert!(s.m.scratch_hidden.is_empty());
}

#[test]
fn summon_brings_a_window_here() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    s.launch(200, "com.example.b");
    s.window(100, 1, "one", CENTER);
    s.focus(100, 1);
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(200, 2, "two", CENTER);
    s.focus(200, 2);
    s.cmd(Command::Workspace { name: "1".into() });
    assert!(!s.on_screen(2));

    // By bundle id, ignoring case: it tiles next to the focused window.
    assert_eq!(
        s.cmd(Command::Summon {
            app: "COM.EXAMPLE.B".into()
        }),
        Reply::ok()
    );
    assert_eq!(s.m.visible["C"], "C/1", "no workspace switch");
    assert!(s.on_screen(2) && s.on_screen(1));
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "C/1");
    assert_eq!(s.m.focused_window(), Some(WindowId(2)));
    assert!(s.frame(1).w < 1000.0);

    // From another display, floating windows keep their size.
    s.cmd(Command::ToggleFloat);
    let size = s.m.windows[&WindowId(2)].float_frame.unwrap().size();
    s.cmd(Command::FocusDisplay { which: Cycle::Next });
    s.cmd(Command::Summon {
        app: "com.example.b".into(),
    });
    let f = s.frame(2);
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "R/1");
    assert_eq!(f.size(), size);
    assert!(f.x >= 1920.0);

    // No window: open the app.
    s.cmd(Command::Summon {
        app: "com.example.c".into(),
    });
    assert_eq!(s.launches, vec!["com.example.c".to_string()]);
}

fn warps(fx: &[Effect]) -> Vec<Rect> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::WarpPointer { rect } => Some(*rect),
            _ => None,
        })
        .collect()
}

fn keyboard(s: &mut Sim, cmd: Command) -> Vec<Effect> {
    s.ev(Event::Command { cmd, token: None })
}

#[test]
fn mouse_follows_keyboard_focus_across_displays() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "left", CENTER);
    s.window(100, 2, "right", CENTER);
    s.focus(100, 2);
    // Off by default.
    let fx = keyboard(
        &mut s,
        Command::Focus {
            dir: Direction::Left,
        },
    );
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
    assert!(warps(&fx).is_empty());

    s.m.config.settings.mouse_follows_focus = true;
    let fx = keyboard(
        &mut s,
        Command::Focus {
            dir: Direction::Right,
        },
    );
    assert_eq!(warps(&fx), vec![s.m.desired[&WindowId(2)]]);
    // Onto the empty display on the right: its middle.
    let fx = keyboard(
        &mut s,
        Command::Focus {
            dir: Direction::Right,
        },
    );
    assert_eq!(s.m.focus_display.as_deref(), Some("R"));
    assert_eq!(warps(&fx), vec![s.m.display("R").unwrap().visible_frame]);
    // Another display through focus-display.
    let fx = keyboard(&mut s, Command::FocusDisplay { which: Cycle::Prev });
    assert_eq!(s.m.focus_window, Some(WindowId(2)));
    assert_eq!(warps(&fx), vec![s.m.desired[&WindowId(2)]]);
    // No focus change, no warp.
    let fx = keyboard(&mut s, Command::Focus { dir: Direction::Up });
    assert!(warps(&fx).is_empty());
    // Focus the OS reports (a click, Cmd-Tab) does not move the pointer.
    let fx = s.ev(Event::FocusChanged {
        pid: 100,
        wid: Some(WindowId(1)),
        frontmost: true,
        activated: true,
    });
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
    assert!(warps(&fx).is_empty());
    // A workspace shown from the keyboard: the frame its window gets there.
    keyboard(&mut s, Command::MoveToWorkspace { name: "2".into() });
    let fx = keyboard(&mut s, Command::Workspace { name: "2".into() });
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
    let w = warps(&fx);
    assert_eq!(w, vec![s.m.desired[&WindowId(1)]]);
    assert!(s
        .m
        .display("C")
        .unwrap()
        .visible_frame
        .contains(w[0].center()));
}

#[test]
fn focus_follows_mouse_after_the_pointer_rests() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "left", CENTER);
    s.window(100, 2, "right", CENTER);
    s.focus(100, 2);
    let over_1 = s.m.desired[&WindowId(1)].center();
    let over_2 = s.m.desired[&WindowId(2)].center();
    let hover = |s: &mut Sim, at: Point| keyboard(s, Command::Hover { at });
    // Off by default.
    hover(&mut s, over_1);
    s.tick(500);
    assert_eq!(s.m.focus_window, Some(WindowId(2)));

    s.m.config.settings.focus_follows_mouse = true;
    hover(&mut s, over_1);
    s.tick(50);
    assert_eq!(s.m.focus_window, Some(WindowId(2)), "not yet");
    // Moving on restarts the wait.
    hover(
        &mut s,
        Point {
            x: over_1.x + 10.0,
            y: over_1.y,
        },
    );
    s.tick(100);
    assert_eq!(s.m.focus_window, Some(WindowId(2)), "still not");
    s.now += 100;
    let fx = s.ev(Event::Tick);
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
    assert!(fx.iter().any(|f| matches!(
        f,
        Effect::Focus {
            wid: WindowId(1),
            ..
        }
    )));
    // A key pressed while the pointer waits over a window wins.
    hover(&mut s, over_2);
    keyboard(&mut s, Command::FocusWindow { wid: 1 });
    s.tick(500);
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
    // Over no window (the empty display): nothing happens.
    hover(
        &mut s,
        Point {
            x: 2500.0,
            y: 500.0,
        },
    );
    s.tick(500);
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
    // Pointer moves during a gesture never focus.
    hover(&mut s, over_2);
    s.m.gesture = Some(pane_core::model::Gesture {
        wid: WindowId(1),
        target: None,
        plain: false,
    });
    s.tick(500);
    assert_eq!(s.m.focus_window, Some(WindowId(1)));
}

#[test]
fn workspace_names_show_in_snapshot_and_palette() {
    let cfg = Config::parse(
        "[workspace_names]\n\"1\" = \"web\"\n\"right:1\" = \"chat\"\n\"uuid:L:2\" = \"mail\"\n",
    )
    .unwrap();
    let mut s = Sim::with(Model::new(cfg, 1_000_000));
    s.launch(100, "com.apple.Safari");
    s.window(100, 1, "News", CENTER);
    s.focus(100, 1);
    let snap = pane_core::snapshot::build(&s.m);
    let title = |d: &str| {
        snap.displays
            .iter()
            .find(|x| x.id == d)
            .and_then(|x| x.visible_workspace_title.clone())
    };
    assert_eq!(title("C").as_deref(), Some("web"));
    assert_eq!(title("L").as_deref(), Some("web"));
    assert_eq!(title("R").as_deref(), Some("chat"), "DISPLAY:NAME wins");
    let ws = |id: &str| snap.workspaces.iter().find(|w| w.id == id).unwrap();
    assert_eq!(ws("R/1").title.as_deref(), Some("chat"));
    assert!(snap.appearance.app_icons);

    // L's workspace 2 is named by UUID.
    s.cmd(Command::Workspace {
        name: "left:2".into(),
    });
    let snap = pane_core::snapshot::build(&s.m);
    let l = snap.displays.iter().find(|x| x.id == "L").unwrap();
    assert_eq!(l.visible_workspace.as_deref(), Some("2"));
    assert_eq!(l.visible_workspace_title.as_deref(), Some("mail"));

    let items = pane_core::palette::search(&s.m, "chat", &[]);
    assert_eq!(items[0].title, "Workspace 1 · chat");
    let items = pane_core::palette::search(&s.m, "news", &[]);
    assert!(
        items[0].subtitle.contains("workspace 1 (web)"),
        "{}",
        items[0].subtitle
    );
}

fn layout(s: &mut Sim, text: &str) {
    let c = Command::parse(text).unwrap();
    assert!(matches!(s.cmd(c), Reply::Ok { .. }), "{text}");
}

fn share(s: &Sim, a: u32, b: u32) -> f64 {
    s.frame(a).w / (s.frame(a).w + s.frame(b).w)
}

#[test]
fn balance_evens_out_the_focused_container() {
    let mut s = Sim::new();
    s.launch(100, "a");
    for i in 1..=3 {
        s.window(100, i, "w", CENTER);
    }
    s.focus(100, 1);
    s.cmd(Command::Resize {
        axis: pane_core::geometry::Orientation::Horizontal,
        delta: 0.2,
    });
    assert!(s.frame(1).w > s.frame(2).w + 100.0);
    s.cmd(Command::Balance);
    let w: Vec<f64> = (1..=3).map(|i| s.frame(i).w).collect();
    assert!(w.iter().all(|x| (x - w[0]).abs() <= 1.0), "{w:?}");
    // Only the focused container: a nested split stays as it is.
    s.focus(100, 3);
    s.cmd(Command::Move {
        dir: Direction::Down,
    });
    assert_eq!(s.frame(3).w, 1908.0, "3 is a full-width row now");
    s.cmd(Command::Resize {
        axis: pane_core::geometry::Orientation::Vertical,
        delta: 0.2,
    });
    s.focus(100, 1);
    s.cmd(Command::Resize {
        axis: pane_core::geometry::Orientation::Horizontal,
        delta: 0.1,
    });
    let tall = s.frame(3).h;
    s.cmd(Command::Balance);
    assert!((s.frame(1).w - s.frame(2).w).abs() <= 1.0);
    assert_eq!(s.frame(3).h, tall, "the outer split keeps its sizes");
}

#[test]
fn master_stack_reshapes_as_windows_come_and_go() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "one", CENTER);
    s.focus(100, 1);
    layout(&mut s, "layout master-stack");
    assert_eq!(s.frame(1).w, 1908.0, "alone it fills the display");
    s.window(100, 2, "two", CENTER);
    assert!((share(&s, 1, 2) - 0.6).abs() < 0.01, "default master_ratio");
    s.window(100, 3, "three", CENTER);
    s.window(100, 4, "four", CENTER);
    // 1 stays the master; the others stack on the right, full height shared.
    assert!((share(&s, 1, 2) - 0.6).abs() < 0.01);
    assert_eq!(s.frame(1).h, 1038.0);
    let stack: Vec<Rect> = (2..=4).map(|i| s.frame(i)).collect();
    assert!(stack.iter().all(|r| r.x == stack[0].x && r.w == stack[0].w));
    assert!(stack.iter().all(|r| r.h < 400.0), "{stack:?}");
    assert!(stack[0].x > s.frame(1).x + s.frame(1).w);

    // Closing the master promotes the first stacked window.
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(1),
    });
    let ms = s.m.ws("C/1").unwrap();
    let first = s.m.tree.windows(ms.root)[0];
    assert_eq!(s.frame(first.0).h, 1038.0);
    let other = (2..=4).find(|i| *i != first.0).unwrap();
    assert!((share(&s, first.0, other) - 0.6).abs() < 0.01);

    // A master widened by hand keeps its width when a window opens.
    s.focus(100, first.0);
    s.cmd(Command::Resize {
        axis: pane_core::geometry::Orientation::Horizontal,
        delta: 0.1,
    });
    s.window(100, 5, "five", CENTER);
    assert!((share(&s, first.0, 5) - 0.7).abs() < 0.01);

    // Kept across a restart.
    let state: pane_core::state::State =
        serde_json::from_str(&serde_json::to_string(&s.m.to_state()).unwrap()).unwrap();
    let m2 = Model::from_state(Config::default(), &state, s.now);
    assert!(matches!(
        m2.ws("C/1").unwrap().layout,
        Some(pane_core::workspace::LayoutMode::MasterStack { .. })
    ));

    // An explicit size wins; `manual` stops re-shaping.
    layout(&mut s, "layout master-stack 50");
    assert!((share(&s, first.0, 5) - 0.5).abs() < 0.01);
    layout(&mut s, "layout manual");
    assert!(s.m.ws("C/1").unwrap().layout.is_none());
    s.focus(100, first.0);
    s.window(100, 6, "six", CENTER);
    assert!(
        s.frame(6).h > 1000.0,
        "inserted next to the master, unshaped"
    );
}

#[test]
fn columns_layout_spreads_tiles_and_keeps_n_columns() {
    let mut s = Sim::new();
    s.launch(100, "a");
    for i in 1..=5 {
        s.window(100, i, "w", CENTER);
    }
    s.focus(100, 1);
    layout(&mut s, "layout columns 3");
    let xs: std::collections::BTreeSet<i64> =
        (1..=5).map(|i| s.frame(i).x.round() as i64).collect();
    assert_eq!(xs.len(), 3, "three columns");
    let w: Vec<f64> = (1..=5).map(|i| s.frame(i).w).collect();
    assert!(
        w.iter().all(|x| (x - w[0]).abs() <= 1.0),
        "equal widths {w:?}"
    );
    // 2 + 2 + 1: the last column is a single full-height tile.
    assert_eq!(s.frame(5).h, 1038.0);
    assert!(s.frame(1).h < 600.0);
    s.window(100, 6, "w", CENTER);
    let xs: std::collections::BTreeSet<i64> =
        (1..=6).map(|i| s.frame(i).x.round() as i64).collect();
    assert_eq!(xs.len(), 3, "still three columns");
    assert!((1..=6).all(|i| s.frame(i).h < 600.0), "2 + 2 + 2");
    // Fewer windows than columns: one column each.
    for i in 3..=6 {
        s.ev(Event::WindowDestroyed {
            pid: 100,
            wid: WindowId(i),
        });
    }
    assert_eq!(s.frame(1).h, 1038.0);
    assert!((s.frame(1).w - s.frame(2).w).abs() <= 1.0);

    // Applying a profile replaces the preset with the profile's shape.
    let p = pane_core::profile::Profile {
        name: "p".into(),
        version: 1,
        when: None,
        displays: vec![pane_core::profile::ProfileDisplay {
            display: "center".into(),
            workspace: "1".into(),
            tree: slot("a"),
            visible: true,
        }],
    };
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(p),
    });
    assert!(s.m.ws("C/1").unwrap().layout.is_none());
}

#[test]
fn layout_commands_are_in_the_palette() {
    let mut s = Sim::new();
    let items = search(&mut s, ">layout");
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert!(titles.contains(&"Layout: master and stack"), "{titles:?}");
    assert!(titles.contains(&"Layout: 3 columns"), "{titles:?}");
    let items = search(&mut s, ">balance");
    assert_eq!(items[0].command, Command::Balance);
}

fn with_rules(toml: &str) -> Sim {
    Sim::with(Model::new(Config::parse(toml).unwrap(), 1_000_000))
}

#[test]
fn rule_puts_terminals_in_a_narrow_right_column() {
    let mut s =
        with_rules("[[rule]]\napp = \"com.apple.Terminal\"\nposition = \"right\"\nweight = 0.25\n");
    s.launch(100, "com.apple.TextEdit");
    s.window(100, 1, "a", CENTER);
    s.window(100, 2, "b", CENTER);
    s.focus(100, 2);
    // b is stacked under a, so the root is a column of two.
    s.cmd(Command::ToggleOrientation);
    assert_eq!(s.frame(1).w, 1908.0);
    s.launch(200, "com.apple.Terminal");
    s.window(200, 10, "term", CENTER);
    let area = Rect::new(6.0, 36.0, 1908.0, 1038.0);
    let t = s.frame(10);
    assert!((t.max_x() - area.max_x()).abs() <= 1.0, "at the right edge");
    assert_eq!(t.h, area.h, "full height");
    let share = (t.w + 3.0) / (area.w + 6.0);
    assert!(
        (share - 0.25).abs() < 0.01,
        "a quarter of the width: {share}"
    );
    // The other windows stay stacked to its left.
    assert_eq!(s.frame(1).x, area.x);
    assert!(s.frame(2).y > s.frame(1).y);
    assert!(s.frame(1).max_x() < t.x);
}

#[test]
fn rule_splits_the_focused_tile_and_sets_its_layout() {
    let mut s = with_rules(
        "[[rule]]\napp = \"com.apple.Terminal\"\norientation = \"vertical\"\nweight = 0.3\n\
         [[rule]]\napp = \"org.mozilla.firefox\"\nlayout = \"accordion\"\n",
    );
    s.launch(100, "com.apple.TextEdit");
    s.window(100, 1, "a", CENTER);
    s.window(100, 2, "b", CENTER);
    s.focus(100, 2);
    s.launch(200, "com.apple.Terminal");
    s.window(200, 10, "term", CENTER);
    // Stacked under b, taking 30% of b's column.
    assert_eq!(s.frame(10).x, s.frame(2).x);
    assert_eq!(s.frame(10).w, s.frame(2).w);
    assert!(s.frame(10).y > s.frame(2).y);
    let share = s.frame(10).h / (s.frame(10).h + s.frame(2).h);
    assert!((share - 0.3).abs() < 0.01, "{share}");
    assert_eq!(s.frame(1).h, 1038.0, "a keeps its full height");

    // A layout rule changes the split the window lands in.
    let ws = s.m.windows[&WindowId(1)].workspace.clone();
    s.focus(100, 1);
    s.launch(300, "org.mozilla.firefox");
    s.window(300, 20, "web", CENTER);
    let n = s.m.window_node(WindowId(20)).unwrap();
    let parent = s.m.tree.parent(n).unwrap();
    assert_eq!(parent, s.m.ws(&ws).unwrap().root);
    assert_eq!(
        s.m.tree.container(parent).unwrap().layout,
        pane_core::tree::ContainerLayout::Accordion
    );
}

#[test]
fn rule_layout_applies_only_the_first_time() {
    let cfg = "[[rule]]\napp = \"com.apple.Terminal\"\nposition = \"left\"\nweight = 0.2\n";
    let mut s = with_rules(cfg);
    s.launch(100, "com.apple.TextEdit");
    s.window(100, 1, "a", CENTER);
    s.launch(200, "com.apple.Terminal");
    s.window(200, 10, "term", CENTER);
    assert_eq!(s.frame(10).x, 6.0);
    assert!(s.frame(10).w < s.frame(1).w);
    // The user moves it; a relaunch brings it back where the user left it.
    s.focus(200, 10);
    s.cmd(Command::Move {
        dir: Direction::Right,
    });
    assert!(s.frame(10).x > s.frame(1).x);
    s.ev(Event::AppTerminated { pid: 200 });
    s.launch(201, "com.apple.Terminal");
    s.window(201, 11, "term", CENTER);
    assert!(
        s.frame(11).x > s.frame(1).x,
        "remembered, not the rule's edge"
    );
}

// ---------------------------------------------------------------- undo

fn frames(s: &Sim, wids: &[u32]) -> Vec<Rect> {
    wids.iter().map(|w| s.frame(*w)).collect()
}

fn undo(s: &mut Sim) -> Reply {
    s.cmd(Command::Undo)
}

fn three_tiles() -> Sim {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(100, "a");
    for i in 1..=3 {
        s.window(100, i, "w", CENTER);
    }
    s.focus(100, 3);
    s
}

#[test]
fn undo_reverts_moves_resizes_and_toggles_one_at_a_time() {
    let mut s = three_tiles();
    assert!(matches!(undo(&mut s), Reply::Error { .. }), "nothing yet");
    let start = frames(&s, &[1, 2, 3]);

    s.cmd(Command::Move {
        dir: Direction::Left,
    });
    let moved = frames(&s, &[1, 2, 3]);
    assert_ne!(moved, start);
    s.cmd(Command::Resize {
        axis: pane_core::Orientation::Horizontal,
        delta: 0.1,
    });
    let resized = frames(&s, &[1, 2, 3]);
    assert_ne!(resized, moved);
    s.cmd(Command::ToggleOrientation);
    assert_ne!(frames(&s, &[1, 2, 3]), resized);
    // Focus changes alone are not layout changes.
    s.cmd(Command::Focus { dir: Direction::Up });
    assert_eq!(s.m.undo.len(), 3);

    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(frames(&s, &[1, 2, 3]), resized);
    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(frames(&s, &[1, 2, 3]), moved);
    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(frames(&s, &[1, 2, 3]), start);
    assert_eq!(s.m.focused_window(), Some(WindowId(3)));
    assert!(matches!(undo(&mut s), Reply::Error { .. }));
}

#[test]
fn non_finite_resize_is_refused() {
    assert!(Command::parse("resize width nan").is_err());
    assert!(Command::parse("resize height inf").is_err());
    let mut s = three_tiles();
    let start = frames(&s, &[1, 2, 3]);
    let r = s.cmd(Command::Resize {
        axis: pane_core::Orientation::Horizontal,
        delta: f64::NAN,
    });
    assert!(matches!(r, Reply::Error { .. }));
    assert_eq!(frames(&s, &[1, 2, 3]), start);
}

#[test]
fn undo_keeps_the_zoom_of_workspaces_it_does_not_change() {
    let mut s = three_tiles();
    s.cmd(Command::ToggleFullscreen);
    assert_eq!(s.m.ws("C/1").unwrap().fullscreen, Some(WindowId(3)));
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(100, 4, "w", CENTER);
    s.window(100, 5, "w", CENTER);
    s.focus(100, 5);
    s.cmd(Command::Move {
        dir: Direction::Left,
    });
    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(s.m.ws("C/1").unwrap().fullscreen, Some(WindowId(3)));
}

#[test]
fn undo_of_a_profile_apply_brings_back_the_layout_preset() {
    let mut s = three_tiles();
    layout(&mut s, "layout columns 3");
    let p = pane_core::profile::Profile {
        name: "work".into(),
        version: 1,
        when: None,
        displays: vec![pane_core::profile::ProfileDisplay {
            display: "center".into(),
            workspace: "1".into(),
            tree: two_up("com.apple.mail", "a"),
            visible: true,
        }],
    };
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(p),
    });
    assert_eq!(s.m.ws("C/1").unwrap().layout, None);
    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(
        s.m.ws("C/1").unwrap().layout,
        Some(pane_core::workspace::LayoutMode::Columns { n: 3 })
    );
}

#[test]
fn undo_brings_a_window_back_from_another_workspace() {
    let mut s = three_tiles();
    let start = frames(&s, &[1, 2, 3]);
    let home = s.m.windows[&WindowId(3)].workspace.clone();
    s.cmd(Command::MoveToWorkspace { name: "7".into() });
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "C/7");
    assert!(!s.on_screen(3));

    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(s.m.windows[&WindowId(3)].workspace, home);
    assert!(s.on_screen(3));
    assert_eq!(frames(&s, &[1, 2, 3]), start);
    assert_eq!(s.m.focused_window(), Some(WindowId(3)));
    assert!(s.m.ws("C/7").is_none(), "the emptied workspace is dropped");

    // Across a display edge too.
    s.cmd(Command::Move {
        dir: Direction::Right,
    });
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "R/1");
    undo(&mut s);
    assert_eq!(s.m.windows[&WindowId(3)].workspace, home);
    assert_eq!(frames(&s, &[1, 2, 3]), start);
    assert_eq!(s.m.focus_display.as_deref(), Some("C"));
}

#[test]
fn undo_reverts_a_gesture_swap_and_an_edge_drag() {
    let mut s = two_tiles();
    s.tick(2000);
    let (a, b) = (s.frame(1), s.frame(2));
    s.gesture(1, a.center(), b.center());
    assert_eq!((s.frame(1), s.frame(2)), (b, a));
    undo(&mut s);
    assert_eq!((s.frame(1), s.frame(2)), (a, b));

    drag_to(&mut s, 1, Rect::new(a.x, a.y, a.w + 100.0, a.h));
    assert!((s.frame(1).w - (a.w + 100.0)).abs() <= 2.0);
    undo(&mut s);
    assert_eq!((s.frame(1), s.frame(2)), (a, b));
}

#[test]
fn undo_reverts_toggle_float() {
    let mut s = three_tiles();
    let start = frames(&s, &[1, 2, 3]);
    s.cmd(Command::ToggleFloat);
    assert_eq!(s.m.windows[&WindowId(3)].mode, WindowMode::Floating);
    undo(&mut s);
    assert_eq!(s.m.windows[&WindowId(3)].mode, WindowMode::Tiled);
    assert_eq!(frames(&s, &[1, 2, 3]), start);
}

#[test]
fn undo_reverts_a_profile_apply() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(200, "com.apple.Safari");
    s.window(200, 2, "Web", CENTER);
    let full = s.frame(2);
    let p = pane_core::profile::Profile {
        name: "work".into(),
        version: 1,
        when: None,
        displays: vec![pane_core::profile::ProfileDisplay {
            display: "center".into(),
            workspace: "3".into(),
            tree: two_up("com.apple.mail", "com.apple.Safari"),
            visible: true,
        }],
    };
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(p),
    });
    assert_eq!(s.m.active_profile.as_deref(), Some("work"));
    assert_eq!(s.m.visible["C"], "C/3");
    assert!(s.frame(2).w < 1000.0);

    assert_eq!(undo(&mut s), Reply::ok());
    assert_eq!(s.m.active_profile, None);
    assert_eq!(s.m.visible["C"], "C/1");
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "C/1");
    assert_eq!(s.m.windows[&WindowId(2)].slot, None);
    assert_eq!(s.frame(2), full);
    // Mail arriving later no longer has a slot to fill: it just tiles.
    s.launch(300, "com.apple.mail");
    s.window(300, 3, "Inbox", CENTER);
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "C/1");
}

#[test]
fn undo_keeps_windows_opened_since_and_skips_closed_ones() {
    let mut s = three_tiles();
    s.cmd(Command::Move {
        dir: Direction::Left,
    });
    s.window(100, 4, "new", CENTER);
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(1),
    });
    undo(&mut s);
    let ws = s.m.windows[&WindowId(2)].workspace.clone();
    let root = s.m.ws(&ws).unwrap().root;
    assert_eq!(
        s.m.tree.windows(root),
        vec![WindowId(2), WindowId(3), WindowId(4)],
        "old order, the new window kept at the end, the closed one gone"
    );
    for w in [2, 3, 4] {
        assert!(s.on_screen(w));
    }

    // A layout whose windows have all closed changes nothing: skipped.
    let mut s = three_tiles();
    s.cmd(Command::MoveToWorkspace { name: "7".into() });
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(3),
    });
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(1),
    });
    s.ev(Event::WindowDestroyed {
        pid: 100,
        wid: WindowId(2),
    });
    s.tick(11 * 60 * 1000);
    assert!(matches!(undo(&mut s), Reply::Error { .. }));
}

#[test]
fn undo_keeps_the_last_twenty_layouts() {
    let mut s = three_tiles();
    let mut seen = vec![frames(&s, &[1, 2, 3])];
    for i in 0..25 {
        let delta = if i % 2 == 0 { 0.02 } else { -0.01 };
        s.cmd(Command::Resize {
            axis: pane_core::Orientation::Horizontal,
            delta,
        });
        seen.push(frames(&s, &[1, 2, 3]));
    }
    assert_eq!(s.m.undo.len(), pane_core::undo::UNDO_DEPTH);
    for i in 0..20 {
        assert_eq!(undo(&mut s), Reply::ok());
        assert_eq!(frames(&s, &[1, 2, 3]), seen[24 - i]);
    }
    assert!(matches!(undo(&mut s), Reply::Error { .. }));
}

#[test]
fn undo_is_bound_to_mod_z() {
    let c = Config::default();
    let base = c.modifier().unwrap();
    assert!(c
        .default_bindings(&base)
        .iter()
        .any(|(chord, cmd)| cmd == "undo" && chord.ends_with("-z")));
    assert_eq!(Command::parse("undo"), Ok(Command::Undo));
}

#[test]
fn reopened_settings_window_opens_where_you_are() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(200, "com.example.app");
    s.window(200, 2, "Main", CENTER);
    // Settings opened on workspace 1, then closed.
    s.window(200, 3, "Settings", CENTER);
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "C/1");
    s.ev(Event::WindowDestroyed {
        pid: 200,
        wid: WindowId(3),
    });
    // Later, from workspace 2, Settings is opened again (new window id,
    // same title as the remembered one on hidden workspace 1).
    s.tick(2000);
    s.cmd(Command::Workspace { name: "2".into() });
    s.tick(2000);
    s.window(200, 4, "Settings", CENTER);
    assert_eq!(s.m.windows[&WindowId(4)].workspace, "C/2");
    assert!(s.on_screen(4), "the new window is visible");
}

#[test]
fn new_window_of_a_running_app_is_never_parked() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(300, "net.imput.helium");
    s.window(300, 5, "Profile 1", CENTER);
    s.ev(Event::WindowDestroyed {
        pid: 300,
        wid: WindowId(5),
    });
    // The app keeps running without windows; much later a window for
    // another profile opens while you work on workspace 3.
    s.tick(60_000);
    s.cmd(Command::Workspace { name: "3".into() });
    s.tick(2000);
    s.window(300, 6, "Profile 2", CENTER);
    assert_eq!(s.m.windows[&WindowId(6)].workspace, "C/3");
    assert!(s.on_screen(6));
}

#[test]
fn hidden_profile_slot_does_not_take_windows_of_a_running_app() {
    let mut s = Sim::new();
    s.tick(2000);
    s.launch(200, "com.apple.Safari");
    s.window(200, 2, "Web", CENTER);
    let p = pane_core::profile::Profile {
        name: "work".into(),
        version: 1,
        when: None,
        displays: vec![
            pane_core::profile::ProfileDisplay {
                display: "center".into(),
                workspace: "1".into(),
                tree: slot("com.apple.Safari"),
                visible: true,
            },
            pane_core::profile::ProfileDisplay {
                display: "center".into(),
                workspace: "2".into(),
                tree: slot("com.apple.Safari"),
                visible: false,
            },
        ],
    };
    s.cmd(Command::ProfileApply {
        name: None,
        profile: Some(p),
    });
    assert_eq!(s.m.windows[&WindowId(2)].workspace, "C/1");
    s.tick(31_000);
    // A second Safari window: the free slot is on hidden workspace 2.
    s.window(200, 3, "Settings", CENTER);
    assert_eq!(s.m.windows[&WindowId(3)].workspace, "C/1");
    assert!(s.on_screen(3));
}

// ------------------------------------------------------ trackpad swipes

#[test]
fn trackpad_swipes_cycle_the_display_under_the_pointer() {
    let mut s = Sim::new();
    s.launch(100, "a");
    s.window(100, 1, "c1", CENTER);
    s.cmd(Command::Workspace { name: "2".into() });
    s.window(100, 2, "c2", CENTER);
    assert_eq!(s.m.visible["C"], "C/2");
    // Focus is on the left display; the pointer is over the centre one.
    s.cmd(Command::FocusDisplay { which: Cycle::Prev });
    assert_eq!(s.m.focus_display.as_deref(), Some("L"));
    let swipe = |s: &mut Sim, swipe: Swipe| {
        s.ev(Event::Command {
            cmd: Command::Swipe {
                swipe,
                at: pt(900.0, 500.0),
            },
            token: None,
        })
    };
    // Off by default: nothing happens.
    swipe(&mut s, Swipe::Left);
    assert_eq!(s.m.visible["C"], "C/2");
    s.m.config.settings.trackpad_swipes = true;
    swipe(&mut s, Swipe::Left);
    assert_eq!(s.m.visible["C"], "C/1");
    assert_eq!(s.m.visible["L"], "L/1");
    swipe(&mut s, Swipe::Right);
    assert_eq!(s.m.visible["C"], "C/2");
    let fx = swipe(&mut s, Swipe::Up);
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::UiRequest(UiRequest::Palette))));
    assert_eq!(s.m.visible["C"], "C/2");
}
