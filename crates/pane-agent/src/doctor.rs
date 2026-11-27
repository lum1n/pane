//! `pane doctor`: one report of what usually goes wrong. The running app
//! answers [`Live`] (its own permissions, the displays it sees, apps whose
//! windows it gave up on, the config error it holds); the CLI adds what it
//! can read from disk, so most of the report works when Pane is not running.

use std::fmt::Write as _;
use std::path::Path;

use pane_core::config::Config;
use pane_core::display::Display;
use pane_core::model::Model;
use pane_core::{Pid, WindowId};
use serde::{Deserialize, Serialize};

use crate::lockfile::{self, LockState};
use crate::paths::Paths;
use crate::persist;
use crate::platform::Platform;

/// What only the running app knows. The reply to `Command::Doctor`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Live {
    pub pid: u32,
    /// Accessibility permission of the app itself.
    pub ax_trusted: bool,
    /// Input Monitoring of the app itself; `None` when it cannot be read.
    pub input_monitoring: Option<bool>,
    /// The keyboard tap is running; `None` when started without hotkeys.
    pub hotkeys: Option<bool>,
    pub displays: Vec<Display>,
    /// Apps that do not answer, or with windows Pane gave up on.
    pub apps: Vec<TroubledApp>,
    /// Why the loaded config.toml was rejected (Pane runs on defaults).
    pub config_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TroubledApp {
    pub pid: Pid,
    /// Bundle id, or the executable name.
    pub id: String,
    pub name: String,
    pub unresponsive: bool,
    pub windows: Vec<TroubledWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TroubledWindow {
    pub id: WindowId,
    pub title: String,
    pub reason: String,
}

impl Live {
    pub fn from_model(
        m: &Model,
        ax_trusted: bool,
        input_monitoring: Option<bool>,
        hotkeys: Option<bool>,
    ) -> Live {
        Live {
            pid: std::process::id(),
            ax_trusted,
            input_monitoring,
            hotkeys,
            displays: m.displays.clone(),
            apps: troubled_apps(m),
            config_error: m.config_error.as_ref().map(|e| e.render()),
        }
    }
}

/// Unresponsive apps and apps with stubborn windows (frames refused), with
/// each window's `reason`.
pub fn troubled_apps(m: &Model) -> Vec<TroubledApp> {
    m.apps
        .values()
        .filter_map(|a| {
            let windows: Vec<TroubledWindow> = m
                .windows
                .values()
                .filter(|w| w.pid == a.pid && w.stubborn)
                .map(|w| TroubledWindow {
                    id: w.id,
                    title: w.title.clone(),
                    reason: w.reason.clone(),
                })
                .collect();
            (a.unresponsive || !windows.is_empty()).then(|| TroubledApp {
                pid: a.pid,
                id: a.bundle_id.clone(),
                name: a.name.clone(),
                unresponsive: a.unresponsive,
                windows,
            })
        })
        .collect()
}

/// Whose permissions the report shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Subject {
    /// The running Pane.
    Pane,
    /// The process running `pane doctor` (macOS grants the terminal).
    ThisTerminal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Pid of the Pane that answered; `None` when it could not be reached.
    pub running: Option<u32>,
    /// Why the socket did not answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreachable: Option<String>,
    pub permissions_of: Subject,
    pub ax_trusted: bool,
    pub input_monitoring: Option<bool>,
    /// Whether hotkeys run; `None` when Pane is not running or runs
    /// without them.
    pub hotkeys: Option<bool>,
    pub displays: Vec<Display>,
    /// Unknown when Pane is not running.
    pub apps: Option<Vec<TroubledApp>>,
    pub config: ConfigCheck,
    pub lock: LockCheck,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigCheck {
    pub path: String,
    pub exists: bool,
    /// config.toml as it is on disk now.
    pub error: Option<String>,
    /// The error the running Pane holds (it may predate an edit).
    pub loaded_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LockCheck {
    pub path: String,
    #[serde(flatten)]
    pub state: LockState,
    /// Entries in parked.json: windows the next start puts back.
    pub parked: usize,
}

/// Read config.toml the way the runtime does.
pub fn check_config(path: &Path) -> ConfigCheck {
    let (exists, error) = match std::fs::read_to_string(path) {
        Ok(t) => (true, Config::parse(&t).err().map(|e| e.render())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, None),
        Err(e) => (true, Some(e.to_string())),
    };
    ConfigCheck {
        path: path.display().to_string(),
        exists,
        error,
        loaded_error: None,
    }
}

/// Build the report from the running app's answer (or why there was none)
/// and what can be read locally. `platform` is only asked when Pane is not
/// running.
pub fn gather(paths: &Paths, live: Result<Live, String>, platform: &dyn Platform) -> Report {
    let mut config = check_config(&paths.config());
    let lock = LockCheck {
        path: paths.lock().display().to_string(),
        state: lockfile::inspect(&paths.lock(), |p| platform.is_alive(p)),
        parked: persist::read_parked(&paths.parked()).len(),
    };
    match live {
        Ok(l) => {
            config.loaded_error = l.config_error;
            Report {
                running: Some(l.pid),
                unreachable: None,
                permissions_of: Subject::Pane,
                ax_trusted: l.ax_trusted,
                input_monitoring: l.input_monitoring,
                hotkeys: l.hotkeys,
                displays: l.displays,
                apps: Some(l.apps),
                config,
                lock,
            }
        }
        Err(e) => Report {
            running: None,
            unreachable: Some(e),
            permissions_of: Subject::ThisTerminal,
            ax_trusted: platform.ax_trusted(false),
            input_monitoring: platform.input_monitoring(),
            hotkeys: None,
            displays: platform.displays(),
            apps: None,
            config,
            lock,
        },
    }
}

impl Report {
    /// Things that need fixing. Offline, missing terminal permissions do not
    /// count: they say nothing about Pane's.
    pub fn problems(&self) -> usize {
        let online = self.running.is_some();
        [
            online && !self.ax_trusted,
            online && self.hotkeys == Some(false),
            self.displays.is_empty(),
            self.apps.as_ref().is_some_and(|a| !a.is_empty()),
            self.config.error.is_some(),
            self.config.loaded_error.is_some() && self.config.error.is_none(),
            matches!(self.lock.state, LockState::Stale { .. }),
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }

    pub fn render(&self) -> String {
        let mut s = String::new();
        let yes_no = |b: bool| if b { "granted" } else { "NOT granted" };
        match (self.running, &self.unreachable) {
            (Some(pid), _) => {
                let _ = writeln!(s, "Pane:             running (pid {pid})");
            }
            (None, why) => {
                let why = why.as_deref().unwrap_or("no answer");
                let _ = writeln!(s, "Pane:             not running ({why})");
            }
        }

        let who = match self.permissions_of {
            Subject::Pane => "",
            Subject::ThisTerminal => " (this terminal, not Pane)",
        };
        let _ = writeln!(s, "Accessibility:    {}{who}", yes_no(self.ax_trusted));
        let im = match self.input_monitoring {
            Some(b) => yes_no(b),
            None => "unknown",
        };
        let _ = writeln!(s, "Input Monitoring: {im}{who}");
        match self.hotkeys {
            Some(true) => {
                let _ = writeln!(s, "Hotkeys:          running");
            }
            Some(false) => {
                let _ = writeln!(
                    s,
                    "Hotkeys:          NOT running (grant Accessibility, and Input Monitoring if macOS asks, then restart Pane)"
                );
            }
            None => {}
        }
        if self.running.is_some() && !self.ax_trusted {
            let _ = writeln!(
                s,
                "  -> System Settings > Privacy & Security > Accessibility: turn Pane on"
            );
        }

        let _ = writeln!(s, "Displays:         {}", self.displays.len());
        for d in &self.displays {
            let f = d.frame;
            let main = if d.is_main { ", main" } else { "" };
            let _ = writeln!(
                s,
                "  {}. {} {}x{} at ({}, {}) @{}x{main}  [{}]",
                d.position_rank + 1,
                d.name,
                f.w,
                f.h,
                f.x,
                f.y,
                d.scale,
                d.id
            );
        }

        match &self.apps {
            None => {
                let _ = writeln!(s, "Troubled apps:    unknown (Pane is not running)");
            }
            Some(apps) if apps.is_empty() => {
                let _ = writeln!(s, "Troubled apps:    none");
            }
            Some(apps) => {
                let _ = writeln!(s, "Troubled apps:    {}", apps.len());
                for a in apps {
                    let state = if a.unresponsive {
                        " - not responding; Pane leaves its windows alone"
                    } else {
                        ""
                    };
                    let _ = writeln!(s, "  {} ({}, pid {}){state}", a.name, a.id, a.pid);
                    for w in &a.windows {
                        let _ = writeln!(s, "    window {} \"{}\": {}", w.id, w.title, w.reason);
                    }
                }
            }
        }

        let c = &self.config;
        match (&c.error, &c.loaded_error, c.exists) {
            (Some(e), _, _) => {
                let _ = writeln!(s, "Config:           ERROR in {}: {e}", c.path);
            }
            (None, Some(e), _) => {
                let _ = writeln!(
                    s,
                    "Config:           fixed on disk, but Pane still runs on defaults ({e}); run `pane reload`"
                );
            }
            (None, None, false) => {
                let _ = writeln!(s, "Config:           {} not found (defaults)", c.path);
            }
            (None, None, true) => {
                let _ = writeln!(s, "Config:           ok ({})", c.path);
            }
        }

        let l = &self.lock;
        match l.state {
            LockState::Absent => {
                let _ = writeln!(s, "run.lock:         none");
            }
            LockState::Held { pid } => {
                let _ = writeln!(s, "run.lock:         held by pid {pid}");
            }
            LockState::Stale { pid } => {
                let _ = writeln!(
                    s,
                    "run.lock:         STALE (pid {pid} is gone): the last run did not exit cleanly"
                );
                if l.parked > 0 {
                    let _ = writeln!(
                        s,
                        "  -> {} parked window(s) in parked.json; start Pane or run `pane recover` to bring them back",
                        l.parked
                    );
                }
            }
        }

        match self.problems() {
            0 => s.push_str("\nNo problems found.\n"),
            n => {
                let _ = writeln!(s, "\n{n} problem(s) found.");
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::NullPlatform;

    fn temp(tag: &str) -> Paths {
        let d = std::env::temp_dir().join(format!("pane-doctor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Paths::new(d)
    }

    #[test]
    fn offline_report_reads_config_and_stale_lock() {
        let p = temp("offline");
        std::fs::write(p.config(), "[settings]\ngaps = 999\n").unwrap();
        // A pid that cannot be alive.
        std::fs::write(p.lock(), "2147483646 0\n").unwrap();
        let r = gather(&p, Err("connection refused".into()), &NullPlatform);
        assert_eq!(r.running, None);
        assert_eq!(r.permissions_of, Subject::ThisTerminal);
        assert!(r.apps.is_none());
        assert!(r.config.error.as_deref().unwrap().contains("gaps"));
        assert_eq!(r.lock.state, LockState::Stale { pid: 2147483646 });
        assert_eq!(r.problems(), 2);
        let text = r.render();
        assert!(text.contains("not running (connection refused)"), "{text}");
        assert!(text.contains("STALE"), "{text}");
        assert!(text.contains("this terminal"), "{text}");
        let _ = std::fs::remove_dir_all(&p.root);
    }

    #[test]
    fn live_report_lists_troubled_apps_and_held_lock() {
        let p = temp("live");
        std::fs::write(p.lock(), format!("{} 0\n", std::process::id())).unwrap();
        let live = Live {
            pid: std::process::id(),
            ax_trusted: true,
            input_monitoring: Some(true),
            hotkeys: Some(true),
            displays: NullPlatform.displays(),
            apps: vec![TroubledApp {
                pid: 42,
                id: "com.example.Stubborn".into(),
                name: "Stubborn".into(),
                unresponsive: false,
                windows: vec![TroubledWindow {
                    id: WindowId(7),
                    title: "Main".into(),
                    reason: "refused to take its tile".into(),
                }],
            }],
            config_error: Some("old error".into()),
        };
        let r = gather(&p, Ok(live), &NullPlatform);
        assert_eq!(r.permissions_of, Subject::Pane);
        assert_eq!(
            r.lock.state,
            LockState::Held {
                pid: std::process::id() as i32
            }
        );
        assert!(!r.config.exists);
        // Troubled apps, and a loaded error that disk no longer has.
        assert_eq!(r.problems(), 2);
        let text = r.render();
        assert!(text.contains("refused to take its tile"), "{text}");
        assert!(text.contains("pane reload"), "{text}");
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["lock"]["state"], "held");
        assert_eq!(json["permissions_of"], "pane");
        let _ = std::fs::remove_dir_all(&p.root);
    }
}
