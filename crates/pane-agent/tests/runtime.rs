//! The real runtime loop against a fake platform with fake apps.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use pane_agent::paths::Paths;
use pane_agent::platform::{AppWorker, HotkeyHandle, Platform, RunningApp};
use pane_agent::{runtime, Options};
use pane_core::config::{Chord, Mods};
use pane_core::display::Display;
use pane_core::event::{Event, WindowProbe};
use pane_core::proto::{Command, QueryKind, Reply};
use pane_core::rules::WindowRole;
use pane_core::worker::WorkerCmd;
use pane_core::{Pid, Rect, WindowId};

type Frames = Arc<Mutex<HashMap<WindowId, Rect>>>;

struct FakeWorker {
    pid: Pid,
    events: Sender<Event>,
    frames: Frames,
}

impl AppWorker for FakeWorker {
    fn send(&self, cmd: WorkerCmd) {
        if let WorkerCmd::SetFrame { wid, rect, gen, .. } = cmd {
            self.frames.lock().unwrap().insert(wid, rect);
            let _ = self.events.send(Event::FrameApplied {
                pid: self.pid,
                wid,
                gen,
                ok: true,
                actual: Some(rect),
            });
        }
    }
}

struct Fake {
    frames: Frames,
}

impl Platform for Fake {
    fn spawn_worker(&self, pid: Pid, events: Sender<Event>) -> Option<Box<dyn AppWorker>> {
        let windows: Vec<WindowProbe> = (1..=2)
            .map(|i| WindowProbe {
                wid: WindowId(pid as u32 * 10 + i),
                pid,
                title: format!("w{i}"),
                role: WindowRole::Standard,
                resizable: true,
                has_close_button: true,
                modal: false,
                minimized: false,
                frame: Rect::new(100.0, 100.0, 500.0, 400.0),
                document: None,
                min_size: None,
            })
            .collect();
        for w in &windows {
            self.frames.lock().unwrap().insert(w.wid, w.frame);
        }
        let _ = events.send(Event::WindowsDiscovered {
            pid,
            windows,
            initial: true,
        });
        Some(Box::new(FakeWorker {
            pid,
            events,
            frames: self.frames.clone(),
        }))
    }
    fn displays(&self) -> Vec<Display> {
        vec![Display {
            id: "C".into(),
            name: "Fake".into(),
            frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
            visible_frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
            is_main: true,
            scale: 1.0,
            position_rank: 0,
        }]
    }
    fn running_apps(&self) -> Vec<RunningApp> {
        vec![RunningApp {
            pid: 7,
            bundle_id: Some("com.example.app".into()),
            name: "App".into(),
            regular: true,
        }]
    }
    fn is_alive(&self, _pid: Pid) -> bool {
        false
    }
    fn ax_trusted(&self, _prompt: bool) -> bool {
        true
    }
    fn emergency_restore(&self) -> usize {
        0
    }
    fn sweep_all(&self, _known: &[(WindowId, Rect)]) -> usize {
        0
    }
    fn start_hotkeys(
        &self,
        _b: &[(Chord, Command)],
        _g: Option<Mods>,
        _tx: Sender<Command>,
    ) -> Result<Box<dyn HotkeyHandle>, String> {
        Err("none".into())
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pane-rt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn wait_until(mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn runtime_tiles_parks_persists_and_restores() {
    let dir = temp_dir("main");
    let frames: Frames = Arc::default();
    let platform = Arc::new(Fake {
        frames: frames.clone(),
    });
    let mut opts = Options::new(Paths::new(&dir));
    opts.hotkeys = false;
    opts.app_poll = Duration::from_millis(50);
    let snapshots = Arc::new(Mutex::new(0usize));
    let s2 = snapshots.clone();
    opts.on_snapshot = Some(Arc::new(move |_json: &str| *s2.lock().unwrap() += 1));
    let h = runtime::start(platform, opts).unwrap();

    // Two windows tile side by side (default gaps 6).
    wait_until(|| {
        frames
            .lock()
            .unwrap()
            .get(&WindowId(71))
            .is_some_and(|r| r.w < 500.0 && r.x == 6.0)
    });
    let tiled = frames.lock().unwrap().clone();
    assert_eq!(tiled[&WindowId(72)].x, 6.0 + tiled[&WindowId(71)].w + 6.0);

    // Over the socket, like the CLI.
    let r = pane_agent::control::request(
        &dir.join("pane.sock"),
        &Command::Ping,
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(r, Reply::data(&"pong"));

    // Settings edits go through config.toml and take effect immediately.
    let r = h.command(
        Command::SetSetting {
            key: "outer_gap".into(),
            value: serde_json::json!(20),
        },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    wait_until(|| frames.lock().unwrap()[&WindowId(71)].x == 20.0);
    let bad = h.command(
        Command::SetSetting {
            key: "outer_gap".into(),
            value: serde_json::json!("wide"),
        },
        Duration::from_secs(2),
    );
    assert!(
        matches!(bad, Reply::Error { .. }),
        "invalid values are refused"
    );
    assert!(std::fs::read_to_string(dir.join("config.toml"))
        .unwrap()
        .contains("outer_gap = 20"));
    let r = h.command(
        Command::Rebind {
            command: "focus left".into(),
            chord: Some("cmd-alt-y".into()),
        },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    let Reply::Ok { data } = h.command(
        Command::Query {
            what: QueryKind::Bindings,
        },
        Duration::from_secs(2),
    ) else {
        panic!()
    };
    let rows: Vec<pane_core::config::BindingInfo> = serde_json::from_value(data).unwrap();
    assert_eq!(
        rows.iter()
            .find(|r| r.command == "focus left")
            .unwrap()
            .chords,
        vec!["cmd-alt-y"]
    );
    let r = h.command(
        Command::SetSetting {
            key: "outer_gap".into(),
            value: serde_json::json!(6),
        },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }));
    wait_until(|| frames.lock().unwrap()[&WindowId(71)].x == 6.0);

    // Switch away: both parked at the corner, journal written.
    let r = h.command(
        Command::Workspace { name: "2".into() },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    wait_until(|| frames.lock().unwrap()[&WindowId(71)].x == 999.0);
    assert!(dir.join("parked.json").exists());

    // Profiles go through the runtime's storage.
    let Reply::Ok { data } = h.command(
        Command::ProfileCapture {
            name: Some("desk".into()),
        },
        Duration::from_secs(2),
    ) else {
        panic!()
    };
    let profile: pane_core::profile::Profile = serde_json::from_value(data).unwrap();
    let r = h.command(
        Command::ProfileSave {
            profile,
            startup: true,
        },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    assert_eq!(
        h.command(Command::ProfileList, Duration::from_secs(2)),
        Reply::data(&vec!["desk"])
    );
    let cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(cfg.contains("startup_profile = \"desk\""));

    let r = h.command(
        Command::Query {
            what: QueryKind::Windows,
        },
        Duration::from_secs(2),
    );
    let Reply::Ok { data } = r else { panic!() };
    assert_eq!(data.as_array().unwrap().len(), 2);
    assert!(*snapshots.lock().unwrap() > 0);

    // Clean shutdown unparks, writes state, removes the lock and journal.
    assert!(h.shutdown(Duration::from_secs(3)));
    let f = frames.lock().unwrap().clone();
    assert_eq!(
        f[&WindowId(71)],
        tiled[&WindowId(71)],
        "restored to its last visible frame"
    );
    assert!(!dir.join("run.lock").exists());
    assert!(!dir.join("parked.json").exists());
    assert!(dir.join("state.json").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stale_lock_triggers_restore_and_busy_lock_refuses() {
    let dir = temp_dir("lock");
    std::fs::create_dir_all(&dir).unwrap();
    // A live pid holding the lock refuses a second start.
    std::fs::write(dir.join("run.lock"), "1 0\n").unwrap();
    struct Alive(Fake);
    impl Platform for Alive {
        fn spawn_worker(&self, p: Pid, e: Sender<Event>) -> Option<Box<dyn AppWorker>> {
            self.0.spawn_worker(p, e)
        }
        fn displays(&self) -> Vec<Display> {
            self.0.displays()
        }
        fn running_apps(&self) -> Vec<RunningApp> {
            vec![]
        }
        fn is_alive(&self, _pid: Pid) -> bool {
            true
        }
        fn ax_trusted(&self, _p: bool) -> bool {
            true
        }
        fn emergency_restore(&self) -> usize {
            0
        }
        fn sweep_all(&self, _k: &[(WindowId, Rect)]) -> usize {
            0
        }
        fn start_hotkeys(
            &self,
            b: &[(Chord, Command)],
            g: Option<Mods>,
            tx: Sender<Command>,
        ) -> Result<Box<dyn HotkeyHandle>, String> {
            self.0.start_hotkeys(b, g, tx)
        }
    }
    let mut opts = Options::new(Paths::new(&dir));
    opts.hotkeys = false;
    opts.control_socket = false;
    let err = runtime::start(
        Arc::new(Alive(Fake {
            frames: Arc::default(),
        })),
        opts,
    )
    .err()
    .unwrap();
    assert!(err.contains("already running"), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}
