//! `run.lock`: detects a second instance and a previous run that died.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub enum LockOutcome {
    /// No previous run, or it exited cleanly.
    Clean,
    /// A previous run died without cleaning up.
    Stale { pid: i32 },
    /// Another instance is running.
    Busy { pid: i32 },
}

/// The lock as an outsider (`pane doctor`) sees it, without taking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum LockState {
    /// No lock: Pane is not running and last exited cleanly.
    Absent,
    /// A live process holds it.
    Held { pid: i32 },
    /// Its process is gone: the last run did not exit cleanly.
    Stale { pid: i32 },
}

/// The pid written in the lock (0 if unreadable), or `None` without a lock.
fn read_pid(path: &Path) -> Option<i32> {
    let t = std::fs::read_to_string(path).ok()?;
    Some(
        t.split_whitespace()
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0),
    )
}

pub fn inspect(path: &Path, is_alive: impl Fn(i32) -> bool) -> LockState {
    match read_pid(path) {
        None => LockState::Absent,
        Some(pid) if pid > 0 && is_alive(pid) => LockState::Held { pid },
        Some(pid) => LockState::Stale { pid },
    }
}

pub fn acquire(path: &Path, is_alive: impl Fn(i32) -> bool) -> std::io::Result<LockOutcome> {
    let me = std::process::id() as i32;
    let outcome = match read_pid(path) {
        Some(pid) if pid > 0 && pid != me && is_alive(pid) => {
            return Ok(LockOutcome::Busy { pid });
        }
        Some(pid) => LockOutcome::Stale { pid },
        None => LockOutcome::Clean,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    crate::persist::write_atomic(path, format!("{me} {now}\n").as_bytes())?;
    Ok(outcome)
}

pub fn release(path: &Path) {
    let _ = std::fs::remove_file(path);
}
