//! `pane`: talk to the running Pane over its socket.

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use pane_agent::paths::Paths;
use pane_core::proto::{Command, QueryKind, Reply};
use pane_core::stream::EventKind;

#[derive(Parser)]
#[command(name = "pane", version, about = "Control the Pane window manager")]
struct Cli {
    /// Support directory (default: ~/Library/Application Support/Pane).
    #[arg(long, global = true)]
    dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show a workspace: `pane ws 3`.
    #[command(alias = "workspace")]
    Ws { name: String },
    /// Move the focused window to a workspace.
    MoveTo { name: String },
    /// Print state as JSON.
    Query {
        #[arg(value_parser = ["windows", "apps", "displays", "workspaces", "tree", "snapshot", "config"])]
        what: String,
    },
    /// Pull hidden windows back on screen. Works even when Pane is not running.
    Recover {
        /// Also sweep apps Pane does not manage.
        #[arg(long)]
        all_apps: bool,
    },
    /// Reload config.toml.
    Reload,
    /// Profiles: list | apply NAME | capture NAME | show NAME | delete NAME |
    /// auto NAME (apply by itself on the displays connected now) |
    /// manual NAME (only when applied by hand) |
    /// diff [NAME] (how the desk differs from it; default: the active one) |
    /// update [NAME] (save it as the desk is now; default: the active one).
    Profile {
        action: String,
        name: Option<String>,
    },
    /// Search windows, workspaces, profiles and commands like the palette
    /// does: `pane search term`. Run an item with `pane do`.
    Search { query: Vec<String> },
    /// Any command in shortcut syntax: `pane do "focus left"`.
    Do { command: Vec<String> },
    /// Check that Pane is running.
    Ping,
    /// Diagnose: Accessibility and Input Monitoring, displays, apps whose
    /// windows Pane gave up on (and why), config errors, a stale run.lock.
    /// Works partly when Pane is not running. Exits 1 if it finds problems.
    Doctor {
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print events as JSON lines as they happen, starting with the current
    /// state: `pane subscribe workspace-shown focus`. No kinds = all but
    /// snapshot. Exits when Pane quits.
    Subscribe {
        #[arg(value_parser = ["focus", "workspace-shown", "workspaces", "displays", "profile", "snapshot"])]
        kinds: Vec<String>,
        /// Also send the whole snapshot whenever it changes.
        #[arg(long)]
        snapshot: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let paths = Paths::new(cli.dir.clone().unwrap_or_else(Paths::default_root));
    if let Cmd::Doctor { json } = cli.cmd {
        return doctor(&paths, json);
    }
    let cmd = match build(&cli.cmd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pane: {e}");
            return ExitCode::from(2);
        }
    };
    // Streams instead of one reply.
    if let Command::Subscribe { events } = cmd {
        return subscribe(&paths, events);
    }
    let cmd = match (cmd, &cli.cmd) {
        (Command::ProfileCapture { name }, _) => {
            // Capture, then save under the given name.
            match send(&paths, &Command::ProfileCapture { name: name.clone() }) {
                Ok(Reply::Ok { data }) => match serde_json::from_value(data) {
                    Ok(profile) => Command::ProfileSave {
                        profile,
                        startup: false,
                    },
                    Err(e) => {
                        eprintln!("pane: {e}");
                        return ExitCode::FAILURE;
                    }
                },
                other => return report(other, &paths, &cli.cmd),
            }
        }
        (c, _) => c,
    };
    report(send(&paths, &cmd), &paths, &cli.cmd)
}

fn build(c: &Cmd) -> Result<Command, String> {
    Ok(match c {
        Cmd::Ws { name } => Command::Workspace { name: name.clone() },
        Cmd::MoveTo { name } => Command::MoveToWorkspace { name: name.clone() },
        Cmd::Query { what } => Command::Query {
            what: match what.as_str() {
                "windows" => QueryKind::Windows,
                "apps" => QueryKind::Apps,
                "displays" => QueryKind::Displays,
                "workspaces" => QueryKind::Workspaces,
                "tree" => QueryKind::Tree,
                "snapshot" => QueryKind::Snapshot,
                _ => QueryKind::Config,
            },
        },
        Cmd::Recover { all_apps } => Command::Recover {
            all_apps: *all_apps,
        },
        Cmd::Reload => Command::Reload,
        Cmd::Profile { action, name } => {
            let need = || {
                name.clone()
                    .ok_or_else(|| format!("`profile {action}` needs a name"))
            };
            match action.as_str() {
                "list" => Command::ProfileList,
                "apply" => Command::ProfileApply {
                    name: Some(need()?),
                    profile: None,
                },
                "capture" => Command::ProfileCapture {
                    name: Some(need()?),
                },
                "delete" => Command::ProfileDelete { name: need()? },
                "show" => Command::ProfileLoad { name: need()? },
                "auto" => Command::ProfileWhen {
                    name: need()?,
                    here: true,
                },
                "manual" => Command::ProfileWhen {
                    name: need()?,
                    here: false,
                },
                "diff" => Command::ProfileDiff { name: name.clone() },
                "update" => Command::ProfileUpdate { name: name.clone() },
                other => return Err(format!("unknown profile action `{other}`")),
            }
        }
        Cmd::Search { query } => Command::Search {
            query: query.join(" "),
            profiles: vec![],
        },
        Cmd::Do { command } => Command::parse(&command.join(" "))?,
        Cmd::Ping => Command::Ping,
        Cmd::Doctor { .. } => Command::Doctor,
        Cmd::Subscribe { kinds, snapshot } => {
            let mut events: Vec<EventKind> =
                kinds.iter().filter_map(|k| EventKind::parse(k)).collect();
            if *snapshot {
                if events.is_empty() {
                    // The default set, plus the snapshot.
                    events = EventKind::ALL.to_vec();
                } else if !events.contains(&EventKind::Snapshot) {
                    events.push(EventKind::Snapshot);
                }
            }
            Command::Subscribe { events }
        }
    })
}

fn subscribe(paths: &Paths, events: Vec<EventKind>) -> ExitCode {
    let lines = match pane_agent::control::subscribe(&paths.socket(), events) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "pane: cannot subscribe at {} ({e}). Is it running?",
                paths.socket().display()
            );
            return ExitCode::FAILURE;
        }
    };
    let mut out = std::io::stdout().lock();
    for line in lines {
        let Ok(line) = line else { break };
        // Stop quietly when the reader goes away (`| head`).
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}

fn doctor(paths: &Paths, json: bool) -> ExitCode {
    use pane_agent::doctor;
    let live = match pane_agent::control::request(
        &paths.socket(),
        &Command::Doctor,
        Duration::from_secs(5),
    ) {
        Ok(Reply::Ok { data }) => {
            serde_json::from_value::<doctor::Live>(data).map_err(|e| format!("bad reply: {e}"))
        }
        Ok(Reply::Error { message }) => Err(message),
        Err(e) => Err(match e.kind() {
            std::io::ErrorKind::NotFound => "no control socket".into(),
            std::io::ErrorKind::ConnectionRefused => {
                "control socket left behind, nobody listening".into()
            }
            _ => format!("control socket: {e}"),
        }),
    };
    let platform = pane_agent::platform::native();
    let report = doctor::gather(paths, live, platform.as_ref());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        print!("{}", report.render());
    }
    if report.problems() == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn send(paths: &Paths, cmd: &Command) -> std::io::Result<Reply> {
    pane_agent::control::request(&paths.socket(), cmd, Duration::from_secs(10))
}

fn report(r: std::io::Result<Reply>, paths: &Paths, c: &Cmd) -> ExitCode {
    match r {
        Ok(Reply::Ok { data }) => {
            if !data.is_null() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&data).unwrap_or_default()
                );
            }
            ExitCode::SUCCESS
        }
        Ok(Reply::Error { message }) => {
            eprintln!("pane: {message}");
            ExitCode::FAILURE
        }
        Err(e) => {
            if let Cmd::Recover { .. } = c {
                return offline_recover(paths);
            }
            eprintln!(
                "pane: cannot reach Pane at {} ({e}). Is it running?",
                paths.socket().display()
            );
            ExitCode::FAILURE
        }
    }
}

/// Pane is not running: put windows back using parked.json and a sweep of
/// every app's park corners.
#[cfg(target_os = "macos")]
fn offline_recover(paths: &Paths) -> ExitCode {
    if !pane_macos::ax::is_trusted(false) {
        eprintln!("pane: this terminal needs Accessibility permission to move windows");
        return ExitCode::FAILURE;
    }
    let known: Vec<_> = pane_agent::persist::read_parked(&paths.parked())
        .into_iter()
        .map(|e| (e.wid, e.original))
        .collect();
    let n = pane_macos::recovery::sweep_all(&known);
    let _ = pane_agent::persist::write_parked(&paths.parked(), &[]);
    println!("Pane is not running; restored {n} window(s).");
    ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn offline_recover(_paths: &Paths) -> ExitCode {
    eprintln!("pane: offline recovery needs macOS");
    ExitCode::FAILURE
}
