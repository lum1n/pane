//! What the runtime needs from the operating system.

use crossbeam_channel::Sender;
use pane_core::config::Chord;
use pane_core::display::Display;
use pane_core::event::Event;
use pane_core::proto::Command;
use pane_core::worker::WorkerCmd;
use pane_core::{Pid, Rect, WindowId};

#[derive(Clone, Debug, PartialEq)]
pub struct RunningApp {
    pub pid: Pid,
    pub bundle_id: Option<String>,
    pub name: String,
    pub regular: bool,
}

pub trait AppWorker: Send {
    fn send(&self, cmd: WorkerCmd);
}

pub trait HotkeyHandle: Send {
    fn update(&self, bindings: &[(Chord, Command)]);
}

pub trait Platform: Send + Sync + 'static {
    /// Start a worker for `pid`; it reports through `events`.
    fn spawn_worker(&self, pid: Pid, events: Sender<Event>) -> Option<Box<dyn AppWorker>>;
    /// Connected displays. Call on the main thread where it matters.
    fn displays(&self) -> Vec<Display>;
    fn running_apps(&self) -> Vec<RunningApp>;
    fn is_alive(&self, pid: Pid) -> bool;
    fn ax_trusted(&self, prompt: bool) -> bool;
    /// Restore every parked window from the AX mirror (panic path).
    fn emergency_restore(&self) -> usize;
    /// Pull windows of every app off park corners.
    fn sweep_all(&self, known: &[(WindowId, Rect)]) -> usize;
    fn start_hotkeys(
        &self,
        bindings: &[(Chord, Command)],
        tx: Sender<Command>,
    ) -> Result<Box<dyn HotkeyHandle>, String>;
}

/// The platform of this build.
pub fn native() -> Box<dyn Platform> {
    #[cfg(target_os = "macos")]
    {
        Box::new(mac::MacPlatform)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NullPlatform)
    }
}

/// No windows, one fake display. Lets the runtime run (and be tested) on
/// Linux.
pub struct NullPlatform;

impl Platform for NullPlatform {
    fn spawn_worker(&self, _pid: Pid, _events: Sender<Event>) -> Option<Box<dyn AppWorker>> {
        None
    }
    fn displays(&self) -> Vec<Display> {
        vec![Display {
            id: "00000000-0000-0000-0000-000000000000".into(),
            name: "Null".into(),
            frame: Rect::new(0.0, 0.0, 1920.0, 1080.0),
            visible_frame: Rect::new(0.0, 25.0, 1920.0, 1055.0),
            is_main: true,
            scale: 1.0,
            position_rank: 0,
        }]
    }
    fn running_apps(&self) -> Vec<RunningApp> {
        vec![]
    }
    fn is_alive(&self, pid: Pid) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
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
        _tx: Sender<Command>,
    ) -> Result<Box<dyn HotkeyHandle>, String> {
        Err("no hotkeys on this platform".into())
    }
}

#[cfg(target_os = "macos")]
pub mod mac {
    use super::*;
    use pane_macos as m;

    pub struct MacPlatform;

    struct W(m::worker::Worker);
    impl AppWorker for W {
        fn send(&self, cmd: WorkerCmd) {
            self.0.send(cmd)
        }
    }

    struct H(m::hotkeys::Hotkeys);
    impl HotkeyHandle for H {
        fn update(&self, bindings: &[(Chord, Command)]) {
            self.0.update(bindings)
        }
    }

    impl Platform for MacPlatform {
        fn spawn_worker(&self, pid: Pid, events: Sender<Event>) -> Option<Box<dyn AppWorker>> {
            m::worker::spawn(pid, events).map(|w| Box::new(W(w)) as Box<dyn AppWorker>)
        }
        fn displays(&self) -> Vec<Display> {
            m::display::displays()
        }
        fn running_apps(&self) -> Vec<RunningApp> {
            m::apps::running_apps()
                .into_iter()
                .map(|a| RunningApp {
                    pid: a.pid,
                    bundle_id: a.bundle_id,
                    name: a.name,
                    regular: a.regular,
                })
                .collect()
        }
        fn is_alive(&self, pid: Pid) -> bool {
            m::apps::is_alive(pid)
        }
        fn ax_trusted(&self, prompt: bool) -> bool {
            m::ax::is_trusted(prompt)
        }
        fn emergency_restore(&self) -> usize {
            m::recovery::emergency_restore()
        }
        fn sweep_all(&self, known: &[(WindowId, Rect)]) -> usize {
            m::recovery::sweep_all(known)
        }
        fn start_hotkeys(
            &self,
            b: &[(Chord, Command)],
            tx: Sender<Command>,
        ) -> Result<Box<dyn HotkeyHandle>, String> {
            m::hotkeys::Hotkeys::start(b, tx).map(|h| Box::new(H(h)) as Box<dyn HotkeyHandle>)
        }
    }
}
