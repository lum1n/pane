//! The core thread and its plumbing.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{select, unbounded, Receiver, Sender};
use pane_core::config::{Config, STARTER_CONFIG};
use pane_core::display::Display;
use pane_core::effect::Effect;
use pane_core::event::Event;
use pane_core::model::Model;
use pane_core::proto::{Command, QueryKind, Reply, UiRequest};
use pane_core::reconcile::reconcile;
use pane_core::reducer::{reduce, refresh_placements};
use pane_core::snapshot::Snapshot;
use pane_core::worker::WorkerCmd;
use pane_core::Pid;

use crate::lockfile::{self, LockOutcome};
use crate::paths::Paths;
use crate::persist;
use crate::platform::{AppWorker, HotkeyHandle, Platform};

pub type Sink = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Options {
    pub paths: Paths,
    /// Receives `Snapshot` JSON whenever it changes (at most ~20 Hz).
    pub on_snapshot: Option<Sink>,
    /// Receives `UiRequest` JSON.
    pub on_request: Option<Sink>,
    pub hotkeys: bool,
    pub control_socket: bool,
    /// How often to look for launched/terminated apps.
    pub app_poll: Duration,
}

impl Options {
    pub fn new(paths: Paths) -> Options {
        Options {
            paths,
            on_snapshot: None,
            on_request: None,
            hotkeys: true,
            control_socket: true,
            app_poll: Duration::from_secs(1),
        }
    }
}

enum Msg {
    Event(Event),
    Command {
        cmd: Command,
        reply: Option<Sender<Reply>>,
    },
    Displays(Vec<Display>),
    InitialApps(Vec<Pid>),
    AxMissing,
    AxTrusted,
    Shutdown(Sender<()>),
}

/// Cheap, cloneable handle to a running core.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Msg>,
    platform: Arc<dyn Platform>,
    stopped: Arc<AtomicBool>,
}

impl Handle {
    /// Run a command and wait for its reply.
    pub fn command(&self, cmd: Command, timeout: Duration) -> Reply {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        if self
            .tx
            .send(Msg::Command {
                cmd,
                reply: Some(rtx),
            })
            .is_err()
        {
            return Reply::err("Pane is shutting down");
        }
        rrx.recv_timeout(timeout)
            .unwrap_or_else(|_| Reply::err("timed out"))
    }

    /// Fire-and-forget command (hotkeys).
    pub fn command_async(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Command { cmd, reply: None });
    }

    pub fn send_event(&self, e: Event) {
        let _ = self.tx.send(Msg::Event(e));
    }

    /// Re-read the displays. Call on the main thread (uses NSScreen).
    pub fn screens_changed(&self) {
        let d = self.platform.displays();
        let _ = self.tx.send(Msg::Displays(d));
    }

    /// Unpark everything, persist, and stop. Bounded by `timeout`.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return true;
        }
        let (dtx, drx) = crossbeam_channel::bounded(1);
        if self.tx.send(Msg::Shutdown(dtx)).is_err() {
            return true;
        }
        drx.recv_timeout(timeout + Duration::from_millis(500))
            .is_ok()
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const RECONCILE_SPACING_MS: u64 = 16;
const SNAPSHOT_SPACING_MS: u64 = 50;
const PERSIST_DEBOUNCE_MS: u64 = 500;
const DISPLAY_DEBOUNCE_MS: u64 = 300;
const INITIAL_SCAN_MS: u64 = 3000;
const SHUTDOWN_MS: u64 = 3000;

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current().name().unwrap_or("?").to_string();
            tracing::error!(%thread, "panic: {info}");
            prev(info);
        }));
    });
}

/// Start the runtime. Call from the main thread: the initial display scan
/// uses NSScreen.
pub fn start(platform: Arc<dyn Platform>, opts: Options) -> Result<Handle, String> {
    install_panic_hook();
    let paths = opts.paths.clone();
    paths
        .ensure()
        .map_err(|e| format!("cannot create {}: {e}", paths.root.display()))?;

    let crashed = match lockfile::acquire(&paths.lock(), |p| platform.is_alive(p))
        .map_err(|e| e.to_string())?
    {
        LockOutcome::Busy { pid } => return Err(format!("Pane is already running (pid {pid})")),
        LockOutcome::Stale { pid } => {
            tracing::warn!(pid, "previous run did not exit cleanly");
            true
        }
        LockOutcome::Clean => false,
    };
    let leftovers = if crashed {
        persist::read_parked(&paths.parked())
    } else {
        vec![]
    };

    if !paths.config().exists() {
        let _ = persist::write_atomic(&paths.config(), STARTER_CONFIG.as_bytes());
    }
    let (config, config_err) =
        match std::fs::read_to_string(paths.config()).map(|t| Config::parse(&t)) {
            Ok(Ok(c)) => (c, None),
            Ok(Err(e)) => (Config::default(), Some(e)),
            Err(_) => (Config::default(), None),
        };
    let now = now_ms();
    let model = match persist::read_state(&paths.state()) {
        Some(s) => Model::from_state(config, &s, now),
        None => Model::new(config, now),
    };

    let (tx, rx) = unbounded::<Msg>();
    let (ev_tx, ev_rx) = unbounded::<Event>();
    let stopped = Arc::new(AtomicBool::new(false));
    let handle = Handle {
        tx: tx.clone(),
        platform: platform.clone(),
        stopped: stopped.clone(),
    };

    if !leftovers.is_empty() {
        let _ = tx.send(Msg::Event(Event::RestoreParked { entries: leftovers }));
    }
    if let Some(error) = config_err {
        let _ = tx.send(Msg::Event(Event::ConfigFailed { error }));
    }
    let _ = tx.send(Msg::Displays(platform.displays()));

    let hotkey_tx = if opts.hotkeys {
        let (ctx, crx) = unbounded::<Command>();
        let h = handle.clone();
        std::thread::Builder::new()
            .name("pane-hotkey-fwd".into())
            .spawn(move || {
                for cmd in crx {
                    h.command_async(cmd);
                }
            })
            .map_err(|e| e.to_string())?;
        Some(ctx)
    } else {
        None
    };

    let mut core = Core {
        m: model,
        platform: platform.clone(),
        paths: paths.clone(),
        on_snapshot: opts.on_snapshot.clone(),
        on_request: opts.on_request.clone(),
        workers: HashMap::new(),
        ev_tx,
        pending: HashMap::new(),
        next_token: 1,
        hotkey_tx,
        hotkeys: None,
        persist_due: None,
        snapshot_due: None,
        last_snapshot_hash: 0,
        last_snapshot_at: 0,
        displays_due: None,
        first_displays: true,
        initial: None,
        profiles: persist::list_profiles(&paths.profiles()),
        trusted: false,
        last_reconcile: 0,
        reconcile_due: true,
        shutdown: None,
        stopped: stopped.clone(),
    };
    std::thread::Builder::new()
        .name("pane-core".into())
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(|| core.run(rx, ev_rx)));
            if r.is_err() {
                // The core is gone: put every parked window back, then die.
                let n = core_platform_restore(&platform);
                tracing::error!(restored = n, "core thread crashed; aborting");
                std::process::abort();
            }
        })
        .map_err(|e| e.to_string())?;

    spawn_app_poller(handle.clone(), opts.app_poll)?;
    if opts.control_socket {
        if let Err(e) = crate::control::serve(&paths.socket(), handle.clone()) {
            tracing::warn!("control socket unavailable: {e}");
        }
    }
    Ok(handle)
}

fn core_platform_restore(p: &Arc<dyn Platform>) -> usize {
    p.emergency_restore()
}

fn spawn_app_poller(handle: Handle, every: Duration) -> Result<(), String> {
    std::thread::Builder::new()
        .name("pane-apps".into())
        .spawn(move || {
            let p = handle.platform.clone();
            if !p.ax_trusted(true) {
                let _ = handle.tx.send(Msg::AxMissing);
                while !p.ax_trusted(false) {
                    if handle.is_stopped() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
            let _ = handle.tx.send(Msg::AxTrusted);
            let mut known: HashMap<Pid, ()> = HashMap::new();
            let mut first = true;
            while !handle.is_stopped() {
                let apps = p.running_apps();
                let now: HashSet<Pid> = apps.iter().map(|a| a.pid).collect();
                for a in &apps {
                    if known.insert(a.pid, ()).is_none() {
                        handle.send_event(Event::AppLaunched {
                            pid: a.pid,
                            bundle_id: a.bundle_id.clone(),
                            name: a.name.clone(),
                            regular: a.regular,
                        });
                    }
                }
                let gone: Vec<Pid> = known
                    .keys()
                    .copied()
                    .filter(|pid| !now.contains(pid) && !p.is_alive(*pid))
                    .collect();
                for pid in gone {
                    known.remove(&pid);
                    handle.send_event(Event::AppTerminated { pid });
                }
                if first {
                    let _ = handle.tx.send(Msg::InitialApps(
                        apps.iter()
                            .filter(|a| a.bundle_id.is_some())
                            .map(|a| a.pid)
                            .collect(),
                    ));
                    first = false;
                }
                std::thread::sleep(every);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

struct Core {
    m: Model,
    platform: Arc<dyn Platform>,
    paths: Paths,
    on_snapshot: Option<Sink>,
    on_request: Option<Sink>,
    workers: HashMap<Pid, Box<dyn AppWorker>>,
    ev_tx: Sender<Event>,
    pending: HashMap<u64, Sender<Reply>>,
    next_token: u64,
    hotkey_tx: Option<Sender<Command>>,
    hotkeys: Option<Box<dyn HotkeyHandle>>,
    persist_due: Option<u64>,
    snapshot_due: Option<u64>,
    last_snapshot_hash: u64,
    last_snapshot_at: u64,
    displays_due: Option<(u64, Vec<Display>)>,
    first_displays: bool,
    /// Apps whose first enumeration we wait for before `InitialScanDone`.
    initial: Option<(HashSet<Pid>, u64)>,
    profiles: Vec<String>,
    trusted: bool,
    last_reconcile: u64,
    reconcile_due: bool,
    shutdown: Option<(Sender<()>, u64)>,
    stopped: Arc<AtomicBool>,
}

impl Core {
    fn run(&mut self, rx: Receiver<Msg>, ev_rx: Receiver<Event>) {
        loop {
            let now = now_ms();
            let wake = self.next_wake(now);
            let timeout = Duration::from_millis(wake.saturating_sub(now).min(1000));
            select! {
                recv(rx) -> msg => match msg {
                    Ok(m) => self.on_msg(m),
                    Err(_) => return,
                },
                recv(ev_rx) -> ev => if let Ok(e) = ev { self.on_event(e) },
                default(timeout) => {},
            }
            for _ in 0..512 {
                let mut any = false;
                if let Ok(m) = rx.try_recv() {
                    self.on_msg(m);
                    any = true;
                }
                if let Ok(e) = ev_rx.try_recv() {
                    self.on_event(e);
                    any = true;
                }
                if !any {
                    break;
                }
            }
            self.timers();
            self.reconcile_if_due();
            if self.finish_shutdown_if_ready() {
                return;
            }
        }
    }

    fn next_wake(&self, now: u64) -> u64 {
        let mut t = now + 1000;
        let mut at = |x: u64| t = t.min(x);
        if let Some(d) = self.m.next_deadline() {
            at(d);
        }
        if let Some(d) = self.persist_due {
            at(d);
        }
        if let Some(d) = self.snapshot_due {
            at(d);
        }
        if let Some((d, _)) = &self.displays_due {
            at(*d);
        }
        if let Some((_, d)) = &self.initial {
            at(*d);
        }
        if self.reconcile_due {
            at(self.last_reconcile + RECONCILE_SPACING_MS);
        }
        if let Some((_, d)) = &self.shutdown {
            at(*d);
        }
        t
    }

    fn reduce(&mut self, e: Event) {
        let now = now_ms();
        let m = &mut self.m;
        match catch_unwind(AssertUnwindSafe(|| reduce(m, e, now))) {
            Ok(fx) => self.dispatch(fx),
            Err(_) => {
                // Resynchronise: forget what was sent so the next reconcile
                // re-establishes every frame.
                self.m.last_sent.clear();
                self.m.layout_dirty = true;
                self.request(UiRequest::warn(
                    "Internal error; see the log. Pane recovered.",
                ));
            }
        }
        self.reconcile_due = true;
    }

    fn on_event(&mut self, e: Event) {
        if let (
            Event::WindowsDiscovered {
                pid, initial: true, ..
            },
            Some((set, _)),
        ) = (&e, &mut self.initial)
        {
            set.remove(pid);
        }
        self.reduce(e);
    }

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Event(e) => self.on_event(e),
            Msg::Command { cmd, reply } => self.command(cmd, reply),
            Msg::Displays(d) => {
                if self.first_displays {
                    self.first_displays = false;
                    self.reduce(Event::DisplaysChanged { displays: d });
                } else {
                    self.displays_due = Some((now_ms() + DISPLAY_DEBOUNCE_MS, d));
                }
            }
            Msg::InitialApps(pids) => {
                let set: HashSet<Pid> = pids
                    .into_iter()
                    .filter(|p| self.workers.contains_key(p))
                    .collect();
                self.initial = Some((set, now_ms() + INITIAL_SCAN_MS));
            }
            Msg::AxMissing => {
                self.trusted = false;
                self.request(UiRequest::AxPermissionMissing);
                self.snapshot_due = Some(now_ms());
            }
            Msg::AxTrusted => {
                self.trusted = true;
                self.start_hotkeys();
                self.snapshot_due = Some(now_ms());
            }
            Msg::Shutdown(done) => {
                tracing::info!("shutting down: restoring parked windows");
                self.reduce(Event::Shutdown);
                self.last_reconcile = 0;
                self.reconcile_if_due();
                self.shutdown = Some((done, now_ms() + SHUTDOWN_MS));
            }
        }
    }

    fn start_hotkeys(&mut self) {
        if self.hotkeys.is_some() {
            return;
        }
        let Some(tx) = self.hotkey_tx.clone() else {
            return;
        };
        let bindings = self.m.config.bindings().unwrap_or_default();
        match self.platform.start_hotkeys(&bindings, tx) {
            Ok(h) => self.hotkeys = Some(h),
            Err(e) => {
                tracing::warn!("hotkeys: {e}");
                self.request(UiRequest::warn(format!("Hotkeys unavailable: {e}")));
            }
        }
    }

    fn timers(&mut self) {
        let now = now_ms();
        if self.m.next_deadline().is_some_and(|d| d <= now) {
            self.reduce(Event::Tick);
        }
        if self.displays_due.as_ref().is_some_and(|(d, _)| *d <= now) {
            let (_, displays) = self.displays_due.take().unwrap();
            tracing::info!(count = displays.len(), "displays changed");
            self.reduce(Event::DisplaysChanged { displays });
        }
        if let Some((set, deadline)) = &self.initial {
            if set.is_empty() || *deadline <= now {
                self.initial = None;
                self.reduce(Event::InitialScanDone);
                if let Some(name) = self.m.config.settings.startup_profile.clone() {
                    self.command(
                        Command::ProfileApply {
                            name: Some(name),
                            profile: None,
                        },
                        None,
                    );
                }
            }
        }
        if self.persist_due.is_some_and(|d| d <= now) {
            self.persist_due = None;
            self.persist();
        }
        if self.snapshot_due.is_some_and(|d| d <= now) {
            self.snapshot_due = None;
            self.emit_snapshot();
        }
    }

    fn reconcile_if_due(&mut self) {
        let now = now_ms();
        if !self.reconcile_due || now < self.last_reconcile + RECONCILE_SPACING_MS {
            return;
        }
        self.reconcile_due = false;
        self.last_reconcile = now;
        self.m.now_ms = now;
        let m = &mut self.m;
        match catch_unwind(AssertUnwindSafe(|| reconcile(m))) {
            Ok(fx) => self.dispatch(fx),
            Err(_) => {
                self.m.last_sent.clear();
                self.m.layout_dirty = true;
            }
        }
    }

    fn dispatch(&mut self, fx: Vec<Effect>) {
        for f in fx {
            match f {
                Effect::Observe { pid } => {
                    if !self.workers.contains_key(&pid) {
                        match self.platform.spawn_worker(pid, self.ev_tx.clone()) {
                            Some(w) => {
                                self.workers.insert(pid, w);
                            }
                            None => tracing::debug!(pid, "no worker for app"),
                        }
                    }
                }
                Effect::Unobserve { pid } => {
                    if let Some(w) = self.workers.remove(&pid) {
                        w.send(WorkerCmd::Quit);
                    }
                }
                Effect::SetFrame {
                    pid,
                    wid,
                    rect,
                    gen,
                    double_set,
                    restore_to,
                } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::SetFrame {
                            wid,
                            rect,
                            gen,
                            double_set,
                            restore_to,
                        });
                    }
                }
                Effect::Raise { pid, wid } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::Raise { wid });
                    }
                }
                Effect::Focus { pid, wid } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::Focus { wid });
                    }
                }
                Effect::WriteParked { entries } => {
                    if let Err(e) = persist::write_parked(&self.paths.parked(), &entries) {
                        tracing::error!("writing parked.json: {e}");
                    }
                }
                Effect::PersistState => {
                    self.persist_due
                        .get_or_insert(now_ms() + PERSIST_DEBOUNCE_MS);
                }
                Effect::Snapshot => {
                    let at = (self.last_snapshot_at + SNAPSHOT_SPACING_MS).max(now_ms());
                    self.snapshot_due = Some(self.snapshot_due.map_or(at, |d| d.min(at)));
                }
                Effect::UiRequest(r) => self.request(r),
                Effect::Reply { token, reply } => {
                    if let Some(tx) = self.pending.remove(&token) {
                        let _ = tx.send(reply);
                    }
                }
                Effect::RecoverSweep { all_apps } => {
                    let displays: Vec<_> = self
                        .m
                        .displays
                        .iter()
                        .map(|d| (d.frame, d.visible_frame))
                        .collect();
                    let known: Vec<_> = self
                        .m
                        .parked
                        .values()
                        .map(|p| (p.wid, p.original))
                        .collect();
                    for w in self.workers.values() {
                        w.send(WorkerCmd::Recover {
                            displays: displays.clone(),
                            known: known.clone(),
                        });
                    }
                    if all_apps {
                        let p = self.platform.clone();
                        std::thread::spawn(move || {
                            let n = p.sweep_all(&known);
                            tracing::info!(n, "swept all apps");
                        });
                    }
                }
            }
        }
    }

    fn request(&self, r: UiRequest) {
        match (&self.on_request, serde_json::to_string(&r)) {
            (Some(sink), Ok(json)) => sink(&json),
            (None, _) => match &r {
                UiRequest::Notice { message, .. } => tracing::info!("notice: {message}"),
                other => tracing::info!("ui request: {other:?}"),
            },
            _ => {}
        }
    }

    fn snapshot(&self) -> Snapshot {
        let mut s = pane_core::snapshot::build(&self.m);
        s.profiles = self.profiles.clone();
        s.ax_trusted = self.trusted;
        s
    }

    fn emit_snapshot(&mut self) {
        self.last_snapshot_at = now_ms();
        let Some(sink) = &self.on_snapshot else {
            return;
        };
        let Ok(json) = serde_json::to_string(&self.snapshot()) else {
            return;
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        json.hash(&mut h);
        let hash = h.finish();
        if hash != self.last_snapshot_hash {
            self.last_snapshot_hash = hash;
            sink(&json);
        }
    }

    fn persist(&mut self) {
        refresh_placements(&mut self.m);
        if let Err(e) = persist::write_state(&self.paths.state(), &self.m.to_state()) {
            tracing::error!("writing state.json: {e}");
        }
    }

    fn load_config(&mut self) -> Reply {
        match std::fs::read_to_string(self.paths.config()) {
            Ok(text) => match Config::parse(&text) {
                Ok(c) => {
                    let bindings = c.bindings().unwrap_or_default();
                    self.reduce(Event::ConfigLoaded {
                        config: Box::new(c),
                    });
                    if let Some(h) = &self.hotkeys {
                        h.update(&bindings);
                    }
                    Reply::ok()
                }
                Err(e) => {
                    let msg = e.render();
                    self.reduce(Event::ConfigFailed { error: e });
                    Reply::err(msg)
                }
            },
            Err(e) => Reply::err(format!("{}: {e}", self.paths.config().display())),
        }
    }

    fn command(&mut self, cmd: Command, reply: Option<Sender<Reply>>) {
        let local = match cmd {
            Command::Reload => Some(self.load_config()),
            Command::ProfileList => Some(Reply::data(&self.profiles)),
            Command::ProfileLoad { ref name } => {
                Some(match persist::read_profile(&self.paths.profile(name)) {
                    Ok(p) => Reply::data(&p),
                    Err(e) => Reply::err(e),
                })
            }
            Command::ProfileSave {
                ref profile,
                startup,
            } => {
                let r = persist::write_profile(&self.paths.profile(&profile.name), profile);
                self.profiles = persist::list_profiles(&self.paths.profiles());
                self.snapshot_due = Some(now_ms());
                Some(match r {
                    Ok(()) if startup => {
                        let text = std::fs::read_to_string(self.paths.config()).unwrap_or_default();
                        let edited = persist::set_startup_profile(&text, Some(&profile.name));
                        match persist::write_atomic(&self.paths.config(), edited.as_bytes()) {
                            Ok(()) => self.load_config(),
                            Err(e) => Reply::err(e.to_string()),
                        }
                    }
                    Ok(()) => Reply::ok(),
                    Err(e) => Reply::err(e),
                })
            }
            Command::ProfileDelete { ref name } => {
                let r = std::fs::remove_file(self.paths.profile(name));
                self.profiles = persist::list_profiles(&self.paths.profiles());
                self.snapshot_due = Some(now_ms());
                Some(
                    r.map(|_| Reply::ok())
                        .unwrap_or_else(|e| Reply::err(e.to_string())),
                )
            }
            Command::ProfileApply {
                name: Some(ref name),
                profile: None,
            } => match persist::read_profile(&self.paths.profile(name)) {
                Ok(p) => {
                    self.forward(
                        Command::ProfileApply {
                            name: None,
                            profile: Some(p),
                        },
                        reply,
                    );
                    return;
                }
                Err(e) => Some(Reply::err(e)),
            },
            Command::Query {
                what: QueryKind::Snapshot,
            } => Some(Reply::data(&self.snapshot())),
            _ => None,
        };
        match local {
            Some(r) => match reply {
                Some(tx) => {
                    let _ = tx.send(r);
                }
                None => {
                    if let Reply::Error { message } = r {
                        self.request(UiRequest::warn(message));
                    }
                }
            },
            None => self.forward(cmd, reply),
        }
    }

    fn forward(&mut self, cmd: Command, reply: Option<Sender<Reply>>) {
        let token = reply.map(|tx| {
            let t = self.next_token;
            self.next_token += 1;
            self.pending.insert(t, tx);
            t
        });
        self.reduce(Event::Command { cmd, token });
    }

    fn finish_shutdown_if_ready(&mut self) -> bool {
        let Some((_, deadline)) = &self.shutdown else {
            return false;
        };
        let outstanding = self.m.outstanding_sets();
        if outstanding > 0 && now_ms() < *deadline {
            return false;
        }
        if outstanding > 0 {
            tracing::warn!(
                outstanding,
                "shutdown deadline reached with frames unconfirmed"
            );
        }
        let (done, _) = self.shutdown.take().unwrap();
        self.persist();
        let _ = persist::write_parked(&self.paths.parked(), &[]);
        for (_, w) in self.workers.drain() {
            w.send(WorkerCmd::Quit);
        }
        let _ = std::fs::remove_file(self.paths.socket());
        lockfile::release(&self.paths.lock());
        self.stopped.store(true, Ordering::Release);
        let _ = done.send(());
        tracing::info!("clean exit");
        true
    }
}
