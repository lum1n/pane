//! Unix socket speaking the JSON-lines protocol from `pane_core::proto`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use pane_core::proto::{Command, Reply};
use pane_core::stream::EventKind;

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
                no_sigpipe(&conn);
                let h = handle.clone();
                let _ = std::thread::Builder::new()
                    .name("pane-control-conn".into())
                    .spawn(move || client(conn, h));
            }
        })?;
    Ok(())
}

/// Writing to a client that went away (a Ctrl-C'd `pane subscribe`) must
/// return EPIPE, not raise SIGPIPE: inside the app, which is not a Rust
/// binary, nothing ignores the signal and it would kill Pane.
#[cfg(target_os = "macos")]
fn no_sigpipe(s: &UnixStream) {
    use std::os::fd::AsRawFd;
    let one: libc::c_int = 1;
    // SAFETY: a valid socket fd and a c_int option of the stated size.
    unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&one as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

/// Elsewhere only Rust binaries (which ignore SIGPIPE) serve the socket.
#[cfg(not(target_os = "macos"))]
fn no_sigpipe(_s: &UnixStream) {}

fn client(conn: UnixStream, handle: Handle) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(300)));
    let Ok(mut w) = conn.try_clone() else { return };
    for line in BufReader::new(conn).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Command>(&line) {
            Ok(Command::Subscribe { events }) => {
                stream(w, &handle, events);
                return;
            }
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

/// Answer `subscribe`: the ok reply, then event lines until the client goes
/// away or Pane stops (the channel closes). Nothing more is read from the
/// client, so the read timeout no longer applies.
fn stream(mut w: UnixStream, handle: &Handle, kinds: Vec<EventKind>) {
    let rx = handle.subscribe(kinds);
    if w.write_all(b"{\"status\":\"ok\"}\n").is_err() {
        return;
    }
    for mut line in rx {
        line.push('\n');
        if w.write_all(line.as_bytes()).is_err() {
            break;
        }
    }
}

/// Client side of `subscribe`: send it, check the reply, then yield each
/// event line (JSON) until Pane closes the stream.
pub fn subscribe(
    path: &Path,
    events: Vec<EventKind>,
) -> std::io::Result<impl Iterator<Item = std::io::Result<String>>> {
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut line =
        serde_json::to_string(&Command::Subscribe { events }).map_err(std::io::Error::other)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut buf = String::new();
    r.read_line(&mut buf)?;
    match serde_json::from_str::<Reply>(&buf).map_err(std::io::Error::other)? {
        Reply::Ok { .. } => {}
        Reply::Error { message } => return Err(std::io::Error::other(message)),
    }
    // Events come whenever something changes; wait as long as it takes.
    r.get_ref().set_read_timeout(None)?;
    Ok(r.lines())
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
