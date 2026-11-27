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
    desk: Arc<Mutex<Vec<Display>>>,
}

impl Fake {
    fn new(frames: Frames) -> Fake {
        Fake {
            frames,
            desk: Arc::new(Mutex::new(vec![screen("C", 0.0, true)])),
        }
    }
}

fn screen(id: &str, x: f64, main: bool) -> Display {
    Display {
        id: id.into(),
        name: format!("Fake {id}"),
        frame: Rect::new(x, 0.0, 1000.0, 800.0),
        visible_frame: Rect::new(x, 0.0, 1000.0, 800.0),
        is_main: main,
        scale: 1.0,
        position_rank: 0,
    }
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
        self.desk.lock().unwrap().clone()
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
    let platform = Arc::new(Fake::new(frames.clone()));
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

    // The palette's search sees the saved profiles.
    let Reply::Ok { data } = h.command(
        Command::Search {
            query: "desk".into(),
            profiles: vec![],
        },
        Duration::from_secs(2),
    ) else {
        panic!()
    };
    assert_eq!(data[0]["kind"], "profile", "{data}");
    assert_eq!(data[0]["command"]["name"], "desk");

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
    let err = runtime::start(Arc::new(Alive(Fake::new(Arc::default()))), opts)
        .err()
        .unwrap();
    assert!(err.contains("already running"), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn profiles_follow_the_displays() {
    let dir = temp_dir("auto");
    let paths = Paths::new(&dir);
    std::fs::create_dir_all(paths.profiles()).unwrap();
    let profile = |name: &str, when: &[&str]| {
        let text = format!(
            "name = \"{name}\"\n[when]\ndisplays = [{}]\n[[display]]\ndisplay = \"uuid:C\"\nworkspace = \"1\"\ntree = {{ type = \"container\" }}\n",
            when.iter()
                .map(|w| format!("\"{w}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::fs::write(paths.profile(name), text).unwrap();
    };
    profile("solo", &["uuid:C"]);
    profile("duo", &["uuid:C", "uuid:D"]);

    let start = |fake: Fake| {
        let mut opts = Options::new(paths.clone());
        opts.hotkeys = false;
        opts.control_socket = false;
        opts.app_poll = Duration::from_millis(50);
        // Longer than the 300 ms display debounce, so a dropout is seen.
        opts.auto_profile_settle = Duration::from_millis(700);
        runtime::start(Arc::new(fake), opts).unwrap()
    };
    let snap = |h: &runtime::Handle| {
        let Reply::Ok { data } = h.command(
            Command::Query {
                what: QueryKind::Snapshot,
            },
            Duration::from_secs(2),
        ) else {
            panic!()
        };
        serde_json::from_value::<pane_core::snapshot::Snapshot>(data).unwrap()
    };
    let active = |h: &runtime::Handle| snap(h).active_profile;

    let fake = Fake::new(Arc::default());
    let desk = fake.desk.clone();
    let h = start(fake);
    // At startup: the profile for the one display.
    wait_until(|| active(&h).as_deref() == Some("solo"));
    assert_eq!(snap(&h).desk_profiles, vec!["solo"]);

    // A second display arrives: its profile follows once things settle.
    desk.lock().unwrap().push(screen("D", 1000.0, false));
    h.screens_changed();
    wait_until(|| active(&h).as_deref() == Some("duo"));

    // A profile chosen by hand stays on the same desk, through display
    // events that don't change the set.
    let r = h.command(
        Command::ProfileApply {
            name: Some("solo".into()),
            profile: None,
        },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    h.screens_changed();
    std::thread::sleep(Duration::from_millis(1300));
    assert_eq!(active(&h).as_deref(), Some("solo"));

    // ...and across a restart on the same desk.
    assert!(h.shutdown(Duration::from_secs(3)));
    let fake = Fake::new(Arc::default());
    *fake.desk.lock().unwrap() = desk.lock().unwrap().clone();
    let desk = fake.desk.clone();
    let h = start(fake);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(active(&h).as_deref(), Some("solo"));

    // A display that drops out for less than the settle time changes
    // nothing (without the settle, choosing for the lone display and then
    // for the pair again would end on "duo").
    desk.lock().unwrap().pop();
    h.screens_changed();
    std::thread::sleep(Duration::from_millis(450));
    desk.lock().unwrap().push(screen("D", 1000.0, false));
    h.screens_changed();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(active(&h).as_deref(), Some("solo"));

    // `profile-when off` stops a profile from following its displays;
    // `here` binds it to the displays connected now.
    let r = h.command(
        Command::parse("profile-when off duo").unwrap(),
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    assert!(snap(&h).desk_profiles.is_empty());
    desk.lock().unwrap().pop();
    h.screens_changed();
    wait_until(|| snap(&h).desk_profiles == vec!["solo"]);
    let r = h.command(
        Command::parse("profile-when here duo").unwrap(),
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    assert_eq!(snap(&h).desk_profiles, vec!["duo", "solo"]);
    let saved = pane_agent::persist::read_profile(&paths.profile("duo")).unwrap();
    assert_eq!(saved.when.unwrap().displays, vec!["uuid:C"]);
    assert!(matches!(
        h.command(
            Command::parse("profile-when here nope").unwrap(),
            Duration::from_secs(2)
        ),
        Reply::Error { .. }
    ));

    // Re-capturing a profile keeps its [when].
    let Reply::Ok { data } = h.command(
        Command::ProfileCapture {
            name: Some("duo".into()),
        },
        Duration::from_secs(2),
    ) else {
        panic!()
    };
    let p: pane_core::profile::Profile = serde_json::from_value(data).unwrap();
    assert_eq!(p.when.unwrap().displays, vec!["uuid:C"]);

    assert!(h.shutdown(Duration::from_secs(3)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn subscribe_streams_events() {
    let dir = temp_dir("subscribe");
    let mut opts = Options::new(Paths::new(&dir));
    opts.hotkeys = false;
    opts.app_poll = Duration::from_millis(50);
    // No on_snapshot sink, like headless pane-agentd.
    let h = runtime::start(Arc::new(Fake::new(Arc::default())), opts).unwrap();
    wait_until(|| {
        pane_agent::control::request(
            &dir.join("pane.sock"),
            &Command::Ping,
            Duration::from_secs(1),
        )
        .is_ok()
    });

    let lines = pane_agent::control::subscribe(&dir.join("pane.sock"), vec![]).unwrap();
    let (tx, rx) = crossbeam_channel::unbounded::<serde_json::Value>();
    let reader = std::thread::spawn(move || {
        for line in lines {
            let v = serde_json::from_str(&line.unwrap()).unwrap();
            if tx.send(v).is_err() {
                break; // dropping `lines` closes the connection
            }
        }
    });
    let next_until = |pred: &dyn Fn(&serde_json::Value) -> bool| {
        let t = Instant::now();
        loop {
            let left = Duration::from_secs(5).saturating_sub(t.elapsed());
            let v = rx.recv_timeout(left).expect("timed out waiting for event");
            if pred(&v) {
                return v;
            }
        }
    };

    // The current state first, one event of each default kind.
    let mut seen = Vec::new();
    while seen.len() < 5 {
        let v = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        seen.push(v["event"].as_str().unwrap().to_string());
    }
    assert_eq!(
        seen,
        [
            "displays",
            "workspace-shown",
            "workspaces",
            "focus",
            "profile"
        ]
    );

    let r = h.command(
        Command::Workspace { name: "2".into() },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    let v = next_until(&|v| v["event"] == "workspace-shown");
    assert_eq!(v["workspace"], "2");
    assert_eq!(v["display"], "C");
    assert_eq!(v["display_name"], "Fake C");

    // A client that leaves is dropped; the core carries on.
    drop(rx);
    let r = h.command(
        Command::Workspace { name: "1".into() },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Ok { .. }), "{r:?}");
    reader.join().unwrap();
    let r = h.command(Command::Ping, Duration::from_secs(2));
    assert_eq!(r, Reply::data(&"pong"));

    // Not through the in-process command path.
    let r = h.command(
        Command::Subscribe { events: vec![] },
        Duration::from_secs(2),
    );
    assert!(matches!(r, Reply::Error { .. }), "{r:?}");
    h.shutdown(Duration::from_secs(2));
}

#[test]
fn profile_drift_and_update() {
    let dir = temp_dir("drift");
    let frames: Frames = Arc::default();
    let mut opts = Options::new(Paths::new(&dir));
    opts.hotkeys = false;
    opts.control_socket = false;
    opts.app_poll = Duration::from_millis(50);
    let h = runtime::start(Arc::new(Fake::new(frames.clone())), opts).unwrap();
    let ok = |r: Reply| {
        let Reply::Ok { data } = r else {
            panic!("{r:?}")
        };
        data
    };
    let snap = |h: &runtime::Handle| {
        let data = ok(h.command(
            Command::Query {
                what: QueryKind::Snapshot,
            },
            Duration::from_secs(2),
        ));
        serde_json::from_value::<pane_core::snapshot::Snapshot>(data).unwrap()
    };
    // Two windows side by side.
    wait_until(|| {
        frames
            .lock()
            .unwrap()
            .get(&WindowId(72))
            .is_some_and(|r| r.x > 400.0)
    });
    assert!(matches!(
        h.command(Command::ProfileDiff { name: None }, Duration::from_secs(2)),
        Reply::Error { .. }
    ));

    let profile = ok(h.command(
        Command::ProfileCapture {
            name: Some("desk".into()),
        },
        Duration::from_secs(2),
    ));
    let profile: pane_core::profile::Profile = serde_json::from_value(profile).unwrap();
    ok(h.command(
        Command::ProfileSave {
            profile,
            startup: false,
        },
        Duration::from_secs(2),
    ));
    ok(h.command(
        Command::ProfileApply {
            name: Some("desk".into()),
            profile: None,
        },
        Duration::from_secs(2),
    ));
    let s = snap(&h);
    assert_eq!(s.active_profile.as_deref(), Some("desk"));
    assert!(!s.profile_drift);

    // Stack the two: the desk no longer matches the profile.
    ok(h.command(Command::ToggleOrientation, Duration::from_secs(2)));
    assert!(snap(&h).profile_drift);
    let d: pane_core::drift::ProfileDiff = serde_json::from_value(ok(
        h.command(Command::ProfileDiff { name: None }, Duration::from_secs(2))
    ))
    .unwrap();
    assert!(d.drifted);
    assert_eq!(d.name, "desk");
    assert_eq!(d.entries.len(), 1);
    assert_eq!(d.entries[0].change, pane_core::drift::EntryChange::Changed);
    assert!(d.entries[0].arrangement, "{d:?}");

    // Update the profile from the desk: saved, and no drift any more.
    ok(h.command(
        Command::ProfileUpdate { name: None },
        Duration::from_secs(2),
    ));
    assert!(!snap(&h).profile_drift);
    let text = std::fs::read_to_string(Paths::new(&dir).profile("desk")).unwrap();
    assert!(text.contains("vertical"), "{text}");
    let d: pane_core::drift::ProfileDiff = serde_json::from_value(ok(h.command(
        Command::ProfileDiff {
            name: Some("desk".into()),
        },
        Duration::from_secs(2),
    )))
    .unwrap();
    assert!(!d.drifted);

    assert!(h.shutdown(Duration::from_secs(3)));
    let _ = std::fs::remove_dir_all(dir);
}
