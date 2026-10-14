//! Client side of the session service: connect (starting the service when
//! it is not running), then spawn or reattach a shell.

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;

use crate::ipc::{self, Event, Frame, Request};
use crate::session::ServiceTarget;

/// Connect to the running service, or start it (`<this exe>
/// --session-daemon`, its own process group so it outlives the window).
pub fn connect() -> std::io::Result<UnixStream> {
    let path = ipc::socket_path().ok_or_else(|| std::io::Error::other("no data dir"))?;
    if let Ok(s) = UnixStream::connect(&path) {
        return handshake(s);
    }
    let exe = std::env::current_exe()?;
    {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(exe)
            .arg("--session-daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()?;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(&path) {
            Ok(s) => return handshake(s),
            Err(e) if Instant::now() > deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(30)),
        }
    }
}

fn handshake(mut s: UnixStream) -> std::io::Result<UnixStream> {
    // keystrokes go to this socket: only to a service run by this user
    if ipc::peer_uid(&s) != Some(unsafe { libc::getuid() }) {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "session service socket belongs to another user",
        ));
    }
    ipc::send_json(
        &mut s,
        &Request::Hello {
            version: ipc::PROTOCOL_VERSION,
            pid: std::process::id(),
        },
    )?;
    match next_event(&mut s)? {
        Event::Hello { version, .. } if version == ipc::PROTOCOL_VERSION => Ok(s),
        Event::Hello { version, .. } => Err(std::io::Error::other(format!(
            "session service speaks protocol {version}, this build {}",
            ipc::PROTOCOL_VERSION
        ))),
        other => Err(std::io::Error::other(format!("unexpected {other:?}"))),
    }
}

/// Read until the next JSON event (output frames are not expected here).
pub fn next_event(s: &mut UnixStream) -> std::io::Result<Event> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let r = loop {
        match ipc::read_frame(s)? {
            Some(Frame::Json(body)) => {
                break serde_json::from_slice(&body).map_err(std::io::Error::other);
            }
            Some(_) => continue,
            None => break Err(std::io::Error::from(ErrorKind::UnexpectedEof)),
        }
    };
    s.set_read_timeout(None)?;
    r
}

/// Spawn or reattach; the returned stream next delivers the replay, then
/// `ReplayDone`, then live output. Returns the service's session id.
pub fn open(
    target: ServiceTarget,
    shell: Option<&str>,
    cwd: Option<&std::path::Path>,
    env: Vec<(String, String)>,
    ws: WindowSize,
) -> std::io::Result<(UnixStream, u64)> {
    let mut s = connect()?;
    let session = match target {
        ServiceTarget::Spawn => {
            ipc::send_json(
                &mut s,
                &Request::Spawn {
                    shell: shell.map(str::to_string),
                    cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
                    env,
                    cols: ws.num_cols,
                    rows: ws.num_lines,
                    cell_w: ws.cell_width,
                    cell_h: ws.cell_height,
                },
            )?;
            match next_event(&mut s)? {
                Event::Spawned { session } => session,
                Event::Error { message } => return Err(std::io::Error::other(message)),
                other => return Err(std::io::Error::other(format!("unexpected {other:?}"))),
            }
        }
        ServiceTarget::Attach(id) => id,
    };
    ipc::send_json(
        &mut s,
        &Request::Attach {
            session,
            from_seq: 0,
        },
    )?;
    match next_event(&mut s)? {
        Event::Attached { .. } => {}
        Event::Error { message } => {
            return Err(std::io::Error::new(ErrorKind::NotFound, message));
        }
        other => return Err(std::io::Error::other(format!("unexpected {other:?}"))),
    }
    if matches!(target, ServiceTarget::Attach(_)) {
        // the window size may have changed while detached
        ipc::send_json(
            &mut s,
            &Request::Resize {
                session,
                cols: ws.num_cols,
                rows: ws.num_lines,
                cell_w: ws.cell_width,
                cell_h: ws.cell_height,
            },
        )?;
    }
    Ok((s, session))
}

/// Reattach to `session` on a fresh connection after the service dropped
/// this one (it does so to a client that falls too far behind). Streams the
/// output after byte `from_seq`, then `ReplayDone`, then live output; true
/// when output after `from_seq` was no longer retained (the replay starts
/// later). Never starts the service: if it is gone, so is the shell.
pub fn resume(session: u64, from_seq: u64) -> std::io::Result<(UnixStream, bool)> {
    let path = ipc::socket_path().ok_or_else(|| std::io::Error::other("no data dir"))?;
    let mut s = handshake(UnixStream::connect(path)?)?;
    ipc::send_json(&mut s, &Request::Attach { session, from_seq })?;
    match next_event(&mut s)? {
        Event::Attached { truncated, .. } => Ok((s, truncated)),
        Event::Error { message } => Err(std::io::Error::new(ErrorKind::NotFound, message)),
        other => Err(std::io::Error::other(format!("unexpected {other:?}"))),
    }
}

/// Sessions the service is running (empty when it is not running; never
/// starts it).
pub fn list() -> Vec<ipc::SessionInfo> {
    let Some(path) = ipc::socket_path() else {
        return Vec::new();
    };
    let Ok(s) = UnixStream::connect(path) else {
        return Vec::new();
    };
    let Ok(mut s) = handshake(s) else {
        return Vec::new();
    };
    if ipc::send_json(&mut s, &Request::List).is_err() {
        return Vec::new();
    }
    match next_event(&mut s) {
        Ok(Event::Sessions { sessions }) => sessions,
        _ => Vec::new(),
    }
}
