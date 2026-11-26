//! `run.lock`: detects a second instance and a previous run that died.

use std::path::Path;

pub enum LockOutcome {
    /// No previous run, or it exited cleanly.
    Clean,
    /// A previous run died without cleaning up.
    Stale { pid: i32 },
    /// Another instance is running.
    Busy { pid: i32 },
}

pub fn acquire(path: &Path, is_alive: impl Fn(i32) -> bool) -> std::io::Result<LockOutcome> {
    let me = std::process::id() as i32;
    let outcome = match std::fs::read_to_string(path) {
        Ok(t) => {
            let pid: i32 = t
                .split_whitespace()
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
            if pid > 0 && pid != me && is_alive(pid) {
                return Ok(LockOutcome::Busy { pid });
            }
            LockOutcome::Stale { pid }
        }
        Err(_) => LockOutcome::Clean,
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
