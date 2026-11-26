//! Unix socket speaking the JSON-lines protocol from `pane_core::proto`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use pane_core::proto::{Command, Reply};

use crate::runtime::Handle;

pub fn serve(path: &Path, handle: Handle) -> std::io::Result<()> {
    // A stale socket from a crashed run would make bind fail.
    if UnixStream::connect(path).is_err() {
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(path)?;
    std::thread::Builder::new()
        .name("pane-control".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let h = handle.clone();
                let _ = std::thread::Builder::new()
                    .name("pane-control-conn".into())
                    .spawn(move || client(conn, h));
            }
        })?;
    Ok(())
}

fn client(conn: UnixStream, handle: Handle) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(300)));
    let Ok(mut w) = conn.try_clone() else { return };
    for line in BufReader::new(conn).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Command>(&line) {
            Ok(cmd) => handle.command(cmd, Duration::from_secs(5)),
            Err(e) => Reply::err(format!("bad command: {e}")),
        };
        let mut out = serde_json::to_string(&reply)
            .unwrap_or_else(|_| r#"{"status":"error","message":"encode"}"#.into());
        out.push('\n');
        if w.write_all(out.as_bytes()).is_err() {
            break;
        }
    }
}

/// Client side, used by the CLI.
pub fn request(path: &Path, cmd: &Command, timeout: Duration) -> std::io::Result<Reply> {
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(timeout))?;
    let mut line = serde_json::to_string(cmd).map_err(std::io::Error::other)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut buf = String::new();
    BufReader::new(s).read_line(&mut buf)?;
    serde_json::from_str(&buf).map_err(std::io::Error::other)
}
